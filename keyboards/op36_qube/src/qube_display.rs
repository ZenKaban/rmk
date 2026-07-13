//! Ergohaven Qube dongle display (ST7789V over SPI).
//!
//! Full landscape **280×240** UI without a full-frame RGB565 buffer
//! (~134 KiB would OOM / kill HID on nRF52840).
//!
//! Strategy: **stripe multipass**
//! - Logical size: 280×240 (full panel after Deg90)
//! - Physical FB: 280×48×2 ≈ 27 KiB (< EasyDMA MAXCNT 65535, RAM-safe)
//! - Each redraw: for each stripe → clip-draw full UI → SPI that stripe
//!
//! Pinout (`qube.overlay`):
//! SPI3 SCK=P1.11 MOSI=P1.10 · CS=P1.13 · DC=P0.28 · RST=P0.03 · BL=P0.02

use core::fmt::Write as _;

use defmt::{info, warn};
use embassy_futures::select::{select, Either};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::peripherals::{P0_02, P0_03, P0_28, P1_10, P1_11, P1_13, SPI3};
use embassy_nrf::spim::{self, Spim};
use embassy_nrf::{interrupt, Peri};
use embassy_time::{Delay, Duration, Instant, Timer};
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10, FONT_9X15};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use lcd_async::interface::SpiInterface;
use lcd_async::models::ST7789;
use lcd_async::options::{ColorInversion, Orientation, Rotation};
use lcd_async::{Builder, Display as LcdDisplay};
use rmk::core_traits::Runnable;
use rmk::display::{DisplayRenderer, RenderContext};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ConnectionStatusChangeEvent, EventSubscriber,
    KeyboardEvent, LayerChangeEvent, LedIndicatorEvent, ModifierEvent, PeripheralBatteryEvent,
    PeripheralConnectedEvent, SleepStateEvent, SubscribableEvent, WpmUpdateEvent,
};
use rmk::processor::Processor;
use rmk_types::battery::BatteryStatus;
use rmk_types::ble::BleState;
use static_cell::StaticCell;

// --- Panel geometry ---------------------------------------------------------

pub const PANEL_NATIVE_W: usize = 240;
pub const PANEL_NATIVE_H: usize = 280;
const PANEL_ROTATION: Rotation = Rotation::Deg90;

/// Full landscape frame (after Deg90).
pub const SCREEN_W: usize = 280;
pub const SCREEN_H: usize = 240;

/// Stripe height: 280×48×2 = 26_880 B < 64 KiB EasyDMA, comfortable RAM.
const STRIPE_H: usize = 48;
const STRIPE_BYTES: usize = SCREEN_W * STRIPE_H * 2;

const BACKLIGHT_ACTIVE_HIGH: bool = true;

const COL_BG: Rgb565 = Rgb565::BLACK;
const COL_FG: Rgb565 = Rgb565::WHITE;
const COL_DIM: Rgb565 = Rgb565::new(14, 28, 14);
const COL_GREEN: Rgb565 = Rgb565::new(5, 50, 5);
const COL_YELLOW: Rgb565 = Rgb565::new(31, 50, 0);
const COL_RED: Rgb565 = Rgb565::new(31, 5, 5);
const COL_BAR_BG: Rgb565 = Rgb565::new(6, 12, 6);
const COL_BAR_FG: Rgb565 = Rgb565::new(28, 56, 28);
const COL_PANEL: Rgb565 = Rgb565::new(4, 8, 4);

type SpiDev = ExclusiveDevice<Spim<'static>, Output<'static>, NoDelay>;
type Di = SpiInterface<SpiDev, Output<'static>>;
type Panel = LcdDisplay<Di, ST7789, Output<'static>>;

// --- Stripe framebuffer (clip window into full screen) ----------------------

struct StripeLcd {
    display: Panel,
    buffer: &'static mut [u8; STRIPE_BYTES],
    /// Top of the active stripe in full-screen coordinates.
    band_y: u16,
    /// Height of the active stripe (≤ STRIPE_H), last stripe may be shorter.
    band_h: u16,
}

impl StripeLcd {
    fn set_band(&mut self, y: u16, h: u16) {
        self.band_y = y;
        self.band_h = h.min(STRIPE_H as u16);
    }

    fn clear_stripe(&mut self, color: Rgb565) {
        let c = color.into_storage().to_be_bytes();
        for pix in self.buffer.chunks_exact_mut(2) {
            pix[0] = c[0];
            pix[1] = c[1];
        }
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Rgb565) {
        if x < 0 || y < 0 {
            return;
        }
        let x = x as u32;
        let y = y as u32;
        if x >= SCREEN_W as u32 {
            return;
        }
        let by = self.band_y as u32;
        let bh = self.band_h as u32;
        if y < by || y >= by + bh {
            return;
        }
        let ly = (y - by) as usize;
        let lx = x as usize;
        let off = (ly * SCREEN_W + lx) * 2;
        if off + 1 >= self.buffer.len() {
            return;
        }
        let c = color.into_storage().to_be_bytes();
        self.buffer[off] = c[0];
        self.buffer[off + 1] = c[1];
    }

    async fn flush_band(&mut self) {
        let w = SCREEN_W as u16;
        let h = self.band_h;
        let y = self.band_y;
        // Only send used rows (last stripe may be shorter).
        let bytes = (SCREEN_W * h as usize) * 2;
        let slice = &self.buffer[..bytes];
        let _ = self.display.show_raw_data(0, y, w, h, slice).await;
    }
}

impl OriginDimensions for StripeLcd {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

impl DrawTarget for StripeLcd {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(p, col) in pixels {
            self.put_pixel(p.x, p.y, col);
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Self::Error> {
        let by = self.band_y as i32;
        let bh = self.band_h as i32;
        let band = Rectangle::new(Point::new(0, by), Size::new(SCREEN_W as u32, bh as u32));
        let isect = area.intersection(&band);
        if isect.is_zero_sized() {
            return Ok(());
        }
        // Fast path: full-width clear of the stripe
        if isect.top_left.x == 0
            && isect.size.width == SCREEN_W as u32
            && isect.top_left.y == by
            && isect.size.height == bh as u32
        {
            self.clear_stripe(color);
            return Ok(());
        }
        let x0 = isect.top_left.x;
        let y0 = isect.top_left.y;
        let x1 = x0 + isect.size.width as i32;
        let y1 = y0 + isect.size.height as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_pixel(x, y, color);
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Rgb565) -> Result<(), Self::Error> {
        // Only clear the active stripe (multipass re-renders full UI per band).
        self.clear_stripe(color);
        Ok(())
    }
}

// --- Lazy init --------------------------------------------------------------

struct PendingPins {
    spi: Peri<'static, SPI3>,
    sck: Peri<'static, P1_11>,
    mosi: Peri<'static, P1_10>,
    cs: Peri<'static, P1_13>,
    dc: Peri<'static, P0_28>,
    rst: Peri<'static, P0_03>,
}

enum LcdState {
    Pending(PendingPins),
    Active(StripeLcd),
    Failed,
}

pub struct LazyQubeLcd<I> {
    state: LcdState,
    irq: I,
}

impl<I> LazyQubeLcd<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    async fn ensure_init(&mut self) {
        let pins = match core::mem::replace(&mut self.state, LcdState::Failed) {
            LcdState::Pending(p) => p,
            LcdState::Active(d) => {
                self.state = LcdState::Active(d);
                return;
            }
            LcdState::Failed => return,
        };
        match try_init_lcd(pins, self.irq).await {
            Some(lcd) => {
                info!("ST7789 ready (full-screen stripe mode)");
                self.state = LcdState::Active(lcd);
            }
            None => {
                warn!("ST7789 init failed — HID keeps running");
                self.state = LcdState::Failed;
            }
        }
    }

    /// Full-screen redraw via stripe multipass.
    async fn present(&mut self, renderer: &mut QubeStatusRenderer, ctx: &RenderContext) {
        self.ensure_init().await;
        let LcdState::Active(lcd) = &mut self.state else {
            return;
        };

        let mut y: u16 = 0;
        while (y as usize) < SCREEN_H {
            let remaining = (SCREEN_H as u16).saturating_sub(y);
            let h = remaining.min(STRIPE_H as u16);
            lcd.set_band(y, h);
            lcd.clear_stripe(COL_BG);
            // Re-run full UI; DrawTarget keeps only this stripe's pixels.
            renderer.render(ctx, lcd);
            lcd.flush_band().await;
            y = y.saturating_add(h);
        }
    }
}

impl<I> OriginDimensions for LazyQubeLcd<I> {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

// DrawTarget on LazyQubeLcd only needed if something draws before present;
// multipass uses StripeLcd directly via present().

async fn try_init_lcd<I>(pins: PendingPins, irq: I) -> Option<StripeLcd>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    let mut spi_cfg = spim::Config::default();
    spi_cfg.frequency = spim::Frequency::M8;
    let spim = Spim::new_txonly(pins.spi, irq, pins.sck, pins.mosi, spi_cfg);

    let cs = Output::new(pins.cs, Level::High, OutputDrive::Standard);
    let dc = Output::new(pins.dc, Level::Low, OutputDrive::Standard);
    let rst = Output::new(pins.rst, Level::High, OutputDrive::Standard);

    let spi_dev = ExclusiveDevice::new_no_delay(spim, cs).ok()?;
    let di = SpiInterface::new(spi_dev, dc);

    let mut delay = Delay;
    let display = Builder::new(ST7789, di)
        .reset_pin(rst)
        .display_size(PANEL_NATIVE_W as u16, PANEL_NATIVE_H as u16)
        .display_offset(0, 20)
        .invert_colors(ColorInversion::Inverted)
        .orientation(Orientation::new().rotate(PANEL_ROTATION))
        .init(&mut delay)
        .await
        .ok()?;

    static FB: StaticCell<[u8; STRIPE_BYTES]> = StaticCell::new();
    let buffer = FB.init([0; STRIPE_BYTES]);
    Some(StripeLcd {
        display,
        buffer,
        band_y: 0,
        band_h: STRIPE_H as u16,
    })
}

// --- Dongle screen processor (own event loop + multipass present) -----------

pub struct DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    lcd: LazyQubeLcd<I>,
    renderer: QubeStatusRenderer,
    ctx: RenderContext,
    last_render: Instant,
    pending: bool,
    min_interval: Duration,
}

pub fn create_processor<I>(
    spi: Peri<'static, SPI3>,
    sck: Peri<'static, P1_11>,
    mosi: Peri<'static, P1_10>,
    cs: Peri<'static, P1_13>,
    dc: Peri<'static, P0_28>,
    rst: Peri<'static, P0_03>,
    bl: Peri<'static, P0_02>,
    irq: I,
) -> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    let level = if BACKLIGHT_ACTIVE_HIGH {
        Level::High
    } else {
        Level::Low
    };
    static BL: StaticCell<Output<'static>> = StaticCell::new();
    let _ = BL.init(Output::new(bl, level, OutputDrive::Standard));

    DongleScreen {
        lcd: LazyQubeLcd {
            state: LcdState::Pending(PendingPins {
                spi,
                sck,
                mosi,
                cs,
                dc,
                rst,
            }),
            irq,
        },
        renderer: QubeStatusRenderer,
        ctx: RenderContext::default(),
        last_render: Instant::from_ticks(0),
        pending: true,
        min_interval: Duration::from_millis(80),
    }
}

impl<I> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    async fn redraw(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_render) < self.min_interval {
            self.pending = true;
            return;
        }
        self.lcd.present(&mut self.renderer, &self.ctx).await;
        self.ctx.key_press_latch = false;
        self.pending = false;
        self.last_render = Instant::now();
    }

    fn request_redraw(&mut self) {
        self.pending = true;
    }
}

pub struct NeverEvent;
struct NeverSub;

impl EventSubscriber for NeverSub {
    type Event = NeverEvent;
    async fn next_event(&mut self) -> NeverEvent {
        core::future::pending().await
    }
}

impl<I> Runnable for DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    async fn run(&mut self) -> ! {
        self.pending = true;
        self.redraw().await;

        let mut layer_sub = LayerChangeEvent::subscriber();
        let mut wpm_sub = WpmUpdateEvent::subscriber();
        let mut led_sub = LedIndicatorEvent::subscriber();
        let mut mod_sub = ModifierEvent::subscriber();
        let mut key_sub = KeyboardEvent::subscriber();
        let mut sleep_sub = SleepStateEvent::subscriber();
        let mut bat_sub = BatteryStatusEvent::subscriber();
        let mut conn_sub = ConnectionStatusChangeEvent::subscriber();
        let mut peri_conn_sub = PeripheralConnectedEvent::subscriber();
        let mut peri_bat_sub = PeripheralBatteryEvent::subscriber();
        let mut central_sub = CentralConnectedEvent::subscriber();

        loop {
            // Wait for at least one event (or deferred redraw timer).
            if self.pending {
                let wait = self
                    .min_interval
                    .checked_sub(self.last_render.elapsed())
                    .unwrap_or(Duration::MIN);
                match select(
                    Timer::after(wait),
                    Self::next_any(
                        &mut layer_sub,
                        &mut wpm_sub,
                        &mut led_sub,
                        &mut mod_sub,
                        &mut key_sub,
                        &mut sleep_sub,
                        &mut bat_sub,
                        &mut conn_sub,
                        &mut peri_conn_sub,
                        &mut peri_bat_sub,
                        &mut central_sub,
                    ),
                )
                .await
                {
                    Either::First(_) => {}
                    Either::Second(ev) => {
                        self.apply(ev);
                    }
                }
            } else {
                let ev = Self::next_any(
                    &mut layer_sub,
                    &mut wpm_sub,
                    &mut led_sub,
                    &mut mod_sub,
                    &mut key_sub,
                    &mut sleep_sub,
                    &mut bat_sub,
                    &mut conn_sub,
                    &mut peri_conn_sub,
                    &mut peri_bat_sub,
                    &mut central_sub,
                )
                .await;
                self.apply(ev);
            }

            // Coalesce a burst of events that arrived during the previous
            // multipass present (layer MO + OSM mods, etc.) before redrawing.
            for _ in 0..16 {
                match select(
                    Timer::after(Duration::from_millis(0)),
                    Self::next_any(
                        &mut layer_sub,
                        &mut wpm_sub,
                        &mut led_sub,
                        &mut mod_sub,
                        &mut key_sub,
                        &mut sleep_sub,
                        &mut bat_sub,
                        &mut conn_sub,
                        &mut peri_conn_sub,
                        &mut peri_bat_sub,
                        &mut central_sub,
                    ),
                )
                .await
                {
                    Either::First(_) => break,
                    Either::Second(ev) => self.apply(ev),
                }
            }

            if self.pending {
                self.redraw().await;
            }
        }
    }
}

/// Unified UI event for the dongle screen loop.
enum UiEv {
    Layer(LayerChangeEvent),
    Wpm(WpmUpdateEvent),
    Led(LedIndicatorEvent),
    Mod(ModifierEvent),
    Key(KeyboardEvent),
    Sleep(SleepStateEvent),
    Bat(BatteryStatusEvent),
    Conn(ConnectionStatusChangeEvent),
    PeriConn(PeripheralConnectedEvent),
    PeriBat(PeripheralBatteryEvent),
    Central(CentralConnectedEvent),
}

impl<I> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    async fn next_any(
        layer: &mut impl EventSubscriber<Event = LayerChangeEvent>,
        wpm: &mut impl EventSubscriber<Event = WpmUpdateEvent>,
        led: &mut impl EventSubscriber<Event = LedIndicatorEvent>,
        mods: &mut impl EventSubscriber<Event = ModifierEvent>,
        key: &mut impl EventSubscriber<Event = KeyboardEvent>,
        sleep: &mut impl EventSubscriber<Event = SleepStateEvent>,
        bat: &mut impl EventSubscriber<Event = BatteryStatusEvent>,
        conn: &mut impl EventSubscriber<Event = ConnectionStatusChangeEvent>,
        peri_conn: &mut impl EventSubscriber<Event = PeripheralConnectedEvent>,
        peri_bat: &mut impl EventSubscriber<Event = PeripheralBatteryEvent>,
        central: &mut impl EventSubscriber<Event = CentralConnectedEvent>,
    ) -> UiEv {
        // Nested select — a bit verbose but no heap / macro dependency.
        // Prefer input events; depth is fine for status UI.
        use embassy_futures::select::{select, select3, Either, Either3};

        match select3(
            select3(layer.next_event(), wpm.next_event(), led.next_event()),
            select3(mods.next_event(), key.next_event(), sleep.next_event()),
            select3(
                select(bat.next_event(), conn.next_event()),
                select(peri_conn.next_event(), peri_bat.next_event()),
                central.next_event(),
            ),
        )
        .await
        {
            Either3::First(Either3::First(e)) => UiEv::Layer(e),
            Either3::First(Either3::Second(e)) => UiEv::Wpm(e),
            Either3::First(Either3::Third(e)) => UiEv::Led(e),
            Either3::Second(Either3::First(e)) => UiEv::Mod(e),
            Either3::Second(Either3::Second(e)) => UiEv::Key(e),
            Either3::Second(Either3::Third(e)) => UiEv::Sleep(e),
            Either3::Third(Either3::First(Either::First(e))) => UiEv::Bat(e),
            Either3::Third(Either3::First(Either::Second(e))) => UiEv::Conn(e),
            Either3::Third(Either3::Second(Either::First(e))) => UiEv::PeriConn(e),
            Either3::Third(Either3::Second(Either::Second(e))) => UiEv::PeriBat(e),
            Either3::Third(Either3::Third(e)) => UiEv::Central(e),
        }
    }

    fn apply(&mut self, ev: UiEv) {
        // Keyboard matrix floods KeyboardEvent; UI doesn't show individual
        // keys — skip redraw for those so multipass can keep up with layer/mod.
        let mut need_redraw = true;
        match ev {
            UiEv::Layer(e) => self.ctx.layer = e.0,
            UiEv::Wpm(e) => self.ctx.wpm = e.0,
            UiEv::Led(e) => {
                self.ctx.caps_lock = e.0.caps_lock();
                self.ctx.num_lock = e.0.num_lock();
            }
            UiEv::Mod(e) => self.ctx.modifiers = e.modifier,
            UiEv::Key(e) => {
                self.ctx.key_pressed = e.pressed;
                if e.pressed {
                    self.ctx.key_press_latch = true;
                }
                need_redraw = false;
            }
            UiEv::Sleep(e) => self.ctx.sleeping = e.0,
            UiEv::Bat(e) => self.ctx.battery = e,
            UiEv::Conn(e) => self.ctx.ble_status = e.0.ble,
            UiEv::PeriConn(e) => {
                if let Some(slot) = self.ctx.peripherals_connected.get_mut(e.id) {
                    *slot = e.connected;
                }
            }
            UiEv::PeriBat(e) => {
                if let Some(slot) = self.ctx.peripheral_batteries.get_mut(e.id) {
                    *slot = e.state;
                }
            }
            UiEv::Central(e) => self.ctx.central_connected = e.connected,
        }
        if need_redraw {
            self.request_redraw();
        }
    }
}

impl<I> Processor for DongleScreen<I>
where
    I: interrupt::typelevel::Binding<
            <SPI3 as spim::Instance>::Interrupt,
            spim::InterruptHandler<SPI3>,
        > + Copy
        + 'static,
{
    type Event = NeverEvent;
    fn subscriber() -> impl EventSubscriber<Event = NeverEvent> {
        NeverSub
    }
    async fn process(&mut self, _: NeverEvent) {}
    async fn process_loop(&mut self) -> ! {
        self.run().await
    }
}

// Silence unused DisplayDriver import path if needed — keep for future.

// --- Full-screen UI ---------------------------------------------------------
//
// Fixed vertical zones (280×240) so nothing overlaps:
//   0..56   chrome (WPM | USB/BLE) — side columns only
//  60..108  layer (centered, large)
// 112..148  modifiers (centered)
// 148..200  batteries L / R
// 208..240  footer (version · RMK)

pub struct QubeStatusRenderer;

impl DisplayRenderer<Rgb565> for QubeStatusRenderer {
    fn render<D: DrawTarget<Color = Rgb565>>(&mut self, ctx: &RenderContext, display: &mut D) {
        let _ = display.clear(COL_BG);

        let white = MonoTextStyle::new(&FONT_9X15, COL_FG);
        let dim_mid = MonoTextStyle::new(&FONT_9X15, COL_DIM);
        let green = MonoTextStyle::new(&FONT_9X15, COL_GREEN);
        let wpm_num = MonoTextStyle::new(&FONT_10X20, COL_FG);

        let left = ctx.peripherals_connected.first().copied().unwrap_or(false);
        let right = ctx.peripherals_connected.get(1).copied().unwrap_or(false);
        let lp = battery_reading(ctx.peripheral_batteries.first().map(|b| b.0));
        let rp = battery_reading(ctx.peripheral_batteries.get(1).map(|b| b.0));

        // --- Zone 0..56: side chrome (a bit lower + larger type) ---
        let _ = Rectangle::new(Point::new(0, 0), Size::new(SCREEN_W as u32, 56))
            .into_styled(PrimitiveStyle::with_fill(COL_PANEL))
            .draw(display);

        // WPM — left column; counter uses largest mono font
        let mut s: heapless::String<16> = heapless::String::new();
        let _ = write!(&mut s, "{}", ctx.wpm);
        let _ = Text::new(&s, Point::new(22, 20), wpm_num).draw(display);
        let _ = Text::new("WPM", Point::new(22, 44), white).draw(display);

        // USB / BLE — right column (FONT_9X15)
        let ble_on = matches!(
            ctx.ble_status.state,
            BleState::Connected | BleState::Advertising
        );
        let ble_ok = ctx.ble_status.state == BleState::Connected;
        // Same inset as WPM from the left (x=22).
        let right_x = SCREEN_W as i32 - 22;
        let tr = TextStyleBuilder::new()
            .alignment(Alignment::Right)
            .baseline(Baseline::Top)
            .build();
        let _ = Text::with_text_style(
            "USB",
            Point::new(right_x, 12),
            if ble_on { dim_mid } else { white },
            tr,
        )
        .draw(display);
        let mut ble_l: heapless::String<12> = heapless::String::new();
        let _ = write!(
            &mut ble_l,
            "{}BLE{}",
            if ble_on { ">" } else { "" },
            ctx.ble_status.profile.saturating_add(1)
        );
        let _ = Text::with_text_style(
            &ble_l,
            Point::new(right_x, 34),
            if ble_ok {
                green
            } else if ble_on {
                white
            } else {
                dim_mid
            },
            tr,
        )
        .draw(display);

        // --- Zone 60..100: layer (2× FONT_10X20 → 40 px tall) ---
        let name = layer_name(ctx.layer);
        draw_text_scaled_centered(
            display,
            name,
            SCREEN_W as i32 / 2,
            60,
            &FONT_10X20,
            COL_FG,
            2,
        );

        // --- Zone 104..140: modifiers (2× FONT_9X15 → 30 px; shorter alphabet) ---
        if ctx.modifiers.into_bits() != 0 || ctx.caps_lock {
            s.clear();
            if ctx.caps_lock {
                let _ = s.push_str("CAPS ");
            }
            if ctx.modifiers.left_ctrl() || ctx.modifiers.right_ctrl() {
                let _ = s.push_str("C ");
            }
            if ctx.modifiers.left_shift() || ctx.modifiers.right_shift() {
                let _ = s.push_str("S ");
            }
            if ctx.modifiers.left_alt() || ctx.modifiers.right_alt() {
                let _ = s.push_str("A ");
            }
            if ctx.modifiers.left_gui() || ctx.modifiers.right_gui() {
                let _ = s.push_str("G");
            }
            // "CAPS C S A G" at 9×15×2 ≈ 198 px — still < 280
            draw_text_scaled_centered(
                display,
                s.trim_end(),
                SCREEN_W as i32 / 2,
                108,
                &FONT_9X15,
                COL_FG,
                2,
            );
        }

        // --- Zone 148..200: batteries (two columns, no scale-2 on long labels) ---
        let _ = Text::new("BATTERY", Point::new(12, 150), dim_mid).draw(display);
        draw_bat(display, 16, 172, 118, lp, left, "L");
        draw_bat(display, 150, 172, 118, rp, right, "R");

        // --- Zone 208..240: footer ---
        let _ = Rectangle::new(Point::new(20, 210), Size::new((SCREEN_W - 40) as u32, 1))
            .into_styled(PrimitiveStyle::with_fill(COL_DIM))
            .draw(display);
        let mut ver: heapless::String<24> = heapless::String::new();
        let _ = write!(&mut ver, "v{} · RMK", env!("CARGO_PKG_VERSION"));
        let tc = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        let _ = Text::with_text_style(&ver, Point::new(SCREEN_W as i32 / 2, 218), dim_mid, tc)
            .draw(display);
    }
}

fn draw_text_scaled_centered<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    text: &str,
    center_x: i32,
    top_y: i32,
    font: &embedded_graphics::mono_font::MonoFont<'_>,
    color: Rgb565,
    scale: i32,
) {
    let w = text.chars().count() as i32 * font.character_size.width as i32 * scale;
    let top_left = Point::new(center_x - w / 2, top_y);
    if scale <= 1 {
        let style = MonoTextStyle::new(font, color);
        let _ = Text::new(text, top_left, style).draw(display);
        return;
    }
    let style = MonoTextStyle::new(font, color);
    let mut scaler = ScaledDrawTarget {
        inner: display,
        origin: top_left,
        scale,
    };
    let _ = Text::new(text, Point::zero(), style).draw(&mut scaler);
}

struct ScaledDrawTarget<'a, D> {
    inner: &'a mut D,
    origin: Point,
    scale: i32,
}

impl<D: DrawTarget<Color = Rgb565>> OriginDimensions for ScaledDrawTarget<'_, D> {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

impl<D: DrawTarget<Color = Rgb565>> DrawTarget for ScaledDrawTarget<'_, D> {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        let s = self.scale.max(1) as u32;
        for Pixel(p, col) in pixels {
            let x = self.origin.x + p.x * self.scale;
            let y = self.origin.y + p.y * self.scale;
            let _ = Rectangle::new(Point::new(x, y), Size::new(s, s))
                .into_styled(PrimitiveStyle::with_fill(col))
                .draw(self.inner);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum BatReading {
    Unknown,
    Pending,
    Pct(u8),
}

fn battery_reading(status: Option<BatteryStatus>) -> BatReading {
    match status {
        Some(BatteryStatus::Available {
            level: Some(level), ..
        }) => BatReading::Pct(level),
        Some(BatteryStatus::Available { level: None, .. }) => BatReading::Pending,
        Some(BatteryStatus::Unavailable) | None => BatReading::Unknown,
    }
}

fn draw_bat<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: i32,
    reading: BatReading,
    connected: bool,
    side: &str,
) {
    let dim = MonoTextStyle::new(&FONT_6X10, COL_DIM);
    let _ = Text::new(side, Point::new(x, y), dim).draw(display);

    let (label, col, fill_pct): (heapless::String<8>, Rgb565, Option<u8>) =
        match (connected, reading) {
            (false, _) => {
                let mut s = heapless::String::new();
                let _ = s.push_str("--");
                (s, COL_DIM, None)
            }
            (true, BatReading::Unknown) | (true, BatReading::Pending) => {
                let mut s = heapless::String::new();
                let _ = s.push_str("??");
                (s, COL_DIM, None)
            }
            (true, BatReading::Pct(p)) => {
                let mut s = heapless::String::new();
                let _ = write!(&mut s, "{}%", p);
                let c = if p < 10 {
                    COL_RED
                } else if p < 25 {
                    COL_YELLOW
                } else {
                    COL_FG
                };
                (s, c, Some(p))
            }
        };

    let style = MonoTextStyle::new(&FONT_9X15, col);
    let _ = Text::new(&label, Point::new(x + 14, y), style).draw(display);

    let bx = x;
    let by = y + 18;
    let bw = w;
    let bh = 12u32;
    let _ = Rectangle::new(Point::new(bx, by), Size::new(bw as u32, bh))
        .into_styled(PrimitiveStyle::with_fill(COL_BAR_BG))
        .draw(display);
    if let Some(pct) = fill_pct {
        if pct > 0 {
            let fw = ((bw as u32) * pct as u32 / 100).max(2);
            let fc = if pct < 10 {
                COL_RED
            } else if pct < 25 {
                COL_YELLOW
            } else {
                COL_BAR_FG
            };
            let _ = Rectangle::new(Point::new(bx, by), Size::new(fw, bh))
                .into_styled(PrimitiveStyle::with_fill(fc))
                .draw(display);
        }
    }
}

fn layer_name(layer: u8) -> &'static str {
    match layer {
        0 => "BASE",
        1 => "GAM",
        2 => "GFN",
        3 => "NAV",
        4 => "SYM",
        5 => "NUM",
        _ => "?",
    }
}
