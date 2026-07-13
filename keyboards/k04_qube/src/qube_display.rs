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
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_9X15};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{
    Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle,
};
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
const SAFE_X: i32 = 16;
const SAFE_W: u32 = SCREEN_W as u32 - (SAFE_X as u32 * 2);
const PANEL_RADIUS: u32 = 12;
const CHIP_RADIUS: u32 = 8;
const BAR_RADIUS: u32 = 5;
const LAYER_TITLE_W: i32 = 12;
const LAYER_TITLE_H: i32 = 24;
const LAYER_TITLE_GAP: i32 = 1;
const LAYER_TITLE_STROKE: u32 = 2;

const COL_BG: Rgb565 = Rgb565::new(1, 3, 5);
const COL_FG: Rgb565 = Rgb565::new(30, 61, 30);
const COL_MUTED: Rgb565 = Rgb565::new(13, 28, 18);
const COL_DIM: Rgb565 = Rgb565::new(7, 16, 14);
const COL_ACCENT: Rgb565 = Rgb565::new(4, 45, 31);
const COL_ACCENT_DIM: Rgb565 = Rgb565::new(2, 24, 22);
const COL_AMBER: Rgb565 = Rgb565::new(31, 43, 5);
const COL_YELLOW: Rgb565 = Rgb565::new(31, 50, 0);
const COL_RED: Rgb565 = Rgb565::new(31, 5, 5);
const COL_BAR_BG: Rgb565 = Rgb565::new(3, 9, 10);
const COL_BAR_FG: Rgb565 = Rgb565::new(4, 47, 30);
const COL_PANEL: Rgb565 = Rgb565::new(3, 8, 10);
const COL_PANEL_HI: Rgb565 = Rgb565::new(5, 13, 16);
const COL_BORDER: Rgb565 = Rgb565::new(7, 17, 18);

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
// Fixed vertical zones (280x240) so nothing overlaps:
//   14..44   compact header
//   52..130  layer panel
//   138..165 modifier chips
//   174..224 battery cards

pub struct QubeStatusRenderer;

impl DisplayRenderer<Rgb565> for QubeStatusRenderer {
    fn render<D: DrawTarget<Color = Rgb565>>(&mut self, ctx: &RenderContext, display: &mut D) {
        let _ = display.clear(COL_BG);

        let small = MonoTextStyle::new(&FONT_6X10, COL_MUTED);
        let small_dim = MonoTextStyle::new(&FONT_6X10, COL_DIM);
        let body = MonoTextStyle::new(&FONT_9X15, COL_FG);
        let body_muted = MonoTextStyle::new(&FONT_9X15, COL_MUTED);
        let body_accent = MonoTextStyle::new(&FONT_9X15, COL_ACCENT);
        let body_amber = MonoTextStyle::new(&FONT_9X15, COL_AMBER);
        let top = TextStyleBuilder::new().baseline(Baseline::Top).build();
        let tc = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        let tr = TextStyleBuilder::new()
            .alignment(Alignment::Right)
            .baseline(Baseline::Top)
            .build();

        let left = ctx.peripherals_connected.first().copied().unwrap_or(false);
        let right = ctx.peripherals_connected.get(1).copied().unwrap_or(false);
        let lp = battery_reading(ctx.peripheral_batteries.first().map(|b| b.0));
        let rp = battery_reading(ctx.peripheral_batteries.get(1).map(|b| b.0));
        let name = layer_name(ctx.layer);
        let ble_on = matches!(
            ctx.ble_status.state,
            BleState::Connected | BleState::Advertising
        );
        let ble_ok = ctx.ble_status.state == BleState::Connected;

        // Header.
        draw_panel(display, SAFE_X, 14, SAFE_W, 30, COL_PANEL, COL_BORDER);
        draw_round_fill(display, SAFE_X + 10, 23, 4, 12, 2, COL_ACCENT);
        let _ = Text::with_text_style("QUBE", Point::new(SAFE_X + 20, 22), body_accent, top)
            .draw(display);

        let mut s: heapless::String<16> = heapless::String::new();
        let _ = write!(&mut s, "{}", ctx.wpm);
        let _ = Text::with_text_style("WPM", Point::new(94, 20), small_dim, top).draw(display);
        let _ = Text::with_text_style(&s, Point::new(131, 20), body, top).draw(display);

        let _ = Text::with_text_style(
            "USB",
            Point::new(184, 22),
            if ble_on { body_muted } else { body_accent },
            top,
        )
        .draw(display);
        s.clear();
        let _ = write!(&mut s, "BLE{}", ctx.ble_status.profile.saturating_add(1));
        let _ = Text::with_text_style(
            &s,
            Point::new(252, 22),
            if ble_ok {
                body_accent
            } else if ble_on {
                body_amber
            } else {
                body_muted
            },
            tr,
        )
        .draw(display);

        // Layer panel.
        draw_panel(display, SAFE_X, 52, SAFE_W, 78, COL_PANEL_HI, COL_BORDER);
        draw_round_fill(display, SAFE_X + 18, 60, SAFE_W - 36, 3, 2, COL_ACCENT);
        s.clear();
        let _ = write!(&mut s, "LAYER {}", ctx.layer);
        let _ =
            Text::with_text_style(&s, Point::new(SCREEN_W as i32 / 2, 69), small, tc).draw(display);
        draw_vector_text_centered(display, name, SCREEN_W as i32 / 2, 101, COL_FG);

        // Modifier chips.
        draw_panel(display, SAFE_X, 138, SAFE_W, 27, COL_PANEL, COL_BORDER);
        draw_chip(display, 24, 144, 42, "CAPS", ctx.caps_lock);
        draw_chip(
            display,
            72,
            144,
            42,
            "CTRL",
            ctx.modifiers.left_ctrl() || ctx.modifiers.right_ctrl(),
        );
        draw_chip(
            display,
            120,
            144,
            46,
            "SHIFT",
            ctx.modifiers.left_shift() || ctx.modifiers.right_shift(),
        );
        draw_chip(
            display,
            172,
            144,
            38,
            "ALT",
            ctx.modifiers.left_alt() || ctx.modifiers.right_alt(),
        );
        draw_chip(
            display,
            216,
            144,
            38,
            "GUI",
            ctx.modifiers.left_gui() || ctx.modifiers.right_gui(),
        );

        // Battery cards.
        draw_bat(display, SAFE_X, 174, 118, lp, left, "LEFT");
        draw_bat(display, 146, 174, 118, rp, right, "RIGHT");
    }
}

fn draw_panel<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    fill: Rgb565,
    stroke: Rgb565,
) {
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
    let style = PrimitiveStyleBuilder::new()
        .fill_color(fill)
        .stroke_color(stroke)
        .stroke_width(1)
        .build();
    let _ = RoundedRectangle::with_equal_corners(rect, Size::new(PANEL_RADIUS, PANEL_RADIUS))
        .into_styled(style)
        .draw(display);
}

fn draw_round_fill<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    fill: Rgb565,
) {
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
    let _ = RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
        .into_styled(PrimitiveStyle::with_fill(fill))
        .draw(display);
}

fn draw_chip<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: u32,
    label: &str,
    active: bool,
) {
    let fill = if active { COL_ACCENT_DIM } else { COL_BG };
    let stroke = if active { COL_ACCENT } else { COL_BORDER };
    let text = if active {
        MonoTextStyle::new(&FONT_6X10, COL_FG)
    } else {
        MonoTextStyle::new(&FONT_6X10, COL_DIM)
    };
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, 16));
    let style = PrimitiveStyleBuilder::new()
        .fill_color(fill)
        .stroke_color(stroke)
        .stroke_width(1)
        .build();
    let _ = RoundedRectangle::with_equal_corners(rect, Size::new(CHIP_RADIUS, CHIP_RADIUS))
        .into_styled(style)
        .draw(display);
    let tc = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    let _ =
        Text::with_text_style(label, Point::new(x + w as i32 / 2, y + 3), text, tc).draw(display);
}

fn draw_vector_text_centered<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    text: &str,
    center_x: i32,
    center_y: i32,
    color: Rgb565,
) {
    let chars = text.chars().count() as i32;
    if chars == 0 {
        return;
    }
    let total_w = chars * LAYER_TITLE_W + (chars - 1) * LAYER_TITLE_GAP;
    let mut x = center_x - total_w / 2;
    let y = center_y - LAYER_TITLE_H / 2;
    for ch in text.chars() {
        draw_vector_glyph(display, ch, x, y, color);
        x += LAYER_TITLE_W + LAYER_TITLE_GAP;
    }
}

fn draw_vector_glyph<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    ch: char,
    x: i32,
    y: i32,
    color: Rgb565,
) {
    match ch {
        'A' => {
            seg(display, x, y, 1, 14, 3, 0, color);
            seg(display, x, y, 9, 14, 7, 0, color);
            seg(display, x, y, 3, 0, 7, 0, color);
            seg(display, x, y, 2, 7, 8, 7, color);
        }
        'B' => {
            seg(display, x, y, 1, 0, 1, 14, color);
            seg(display, x, y, 1, 0, 7, 0, color);
            seg(display, x, y, 7, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 10, 5, color);
            seg(display, x, y, 10, 5, 8, 7, color);
            seg(display, x, y, 1, 7, 8, 7, color);
            seg(display, x, y, 8, 7, 10, 9, color);
            seg(display, x, y, 10, 9, 10, 12, color);
            seg(display, x, y, 10, 12, 7, 14, color);
            seg(display, x, y, 1, 14, 7, 14, color);
        }
        'D' => {
            seg(display, x, y, 1, 0, 1, 14, color);
            seg(display, x, y, 1, 0, 7, 0, color);
            seg(display, x, y, 7, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 10, 11, color);
            seg(display, x, y, 10, 11, 7, 14, color);
            seg(display, x, y, 1, 14, 7, 14, color);
        }
        'E' => {
            seg(display, x, y, 1, 0, 1, 14, color);
            seg(display, x, y, 1, 0, 10, 0, color);
            seg(display, x, y, 1, 7, 8, 7, color);
            seg(display, x, y, 1, 14, 10, 14, color);
        }
        'F' => {
            seg(display, x, y, 1, 0, 1, 14, color);
            seg(display, x, y, 1, 0, 10, 0, color);
            seg(display, x, y, 1, 7, 8, 7, color);
        }
        'G' => {
            seg(display, x, y, 10, 1, 3, 1, color);
            seg(display, x, y, 3, 1, 1, 4, color);
            seg(display, x, y, 1, 4, 1, 11, color);
            seg(display, x, y, 1, 11, 3, 14, color);
            seg(display, x, y, 3, 14, 10, 14, color);
            seg(display, x, y, 10, 14, 10, 8, color);
            seg(display, x, y, 10, 8, 6, 8, color);
        }
        'J' => {
            seg(display, x, y, 2, 0, 10, 0, color);
            seg(display, x, y, 8, 0, 8, 11, color);
            seg(display, x, y, 8, 11, 6, 14, color);
            seg(display, x, y, 6, 14, 2, 14, color);
            seg(display, x, y, 2, 14, 1, 11, color);
        }
        'L' => {
            seg(display, x, y, 1, 0, 1, 14, color);
            seg(display, x, y, 1, 14, 10, 14, color);
        }
        'M' => {
            seg(display, x, y, 1, 14, 1, 0, color);
            seg(display, x, y, 1, 0, 5, 7, color);
            seg(display, x, y, 5, 7, 9, 0, color);
            seg(display, x, y, 9, 0, 9, 14, color);
        }
        'N' => {
            seg(display, x, y, 1, 14, 1, 0, color);
            seg(display, x, y, 1, 0, 10, 14, color);
            seg(display, x, y, 10, 14, 10, 0, color);
        }
        'P' => {
            seg(display, x, y, 1, 14, 1, 0, color);
            seg(display, x, y, 1, 0, 8, 0, color);
            seg(display, x, y, 8, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 10, 6, color);
            seg(display, x, y, 10, 6, 8, 8, color);
            seg(display, x, y, 1, 8, 8, 8, color);
        }
        'R' => {
            seg(display, x, y, 1, 14, 1, 0, color);
            seg(display, x, y, 1, 0, 8, 0, color);
            seg(display, x, y, 8, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 10, 6, color);
            seg(display, x, y, 10, 6, 8, 8, color);
            seg(display, x, y, 1, 8, 8, 8, color);
            seg(display, x, y, 6, 8, 10, 14, color);
        }
        'S' => {
            seg(display, x, y, 10, 1, 3, 1, color);
            seg(display, x, y, 3, 1, 1, 4, color);
            seg(display, x, y, 1, 4, 3, 7, color);
            seg(display, x, y, 3, 7, 8, 7, color);
            seg(display, x, y, 8, 7, 10, 10, color);
            seg(display, x, y, 10, 10, 8, 13, color);
            seg(display, x, y, 8, 13, 1, 13, color);
        }
        'T' => {
            seg(display, x, y, 1, 0, 10, 0, color);
            seg(display, x, y, 5, 0, 5, 14, color);
        }
        'U' => {
            seg(display, x, y, 1, 0, 1, 11, color);
            seg(display, x, y, 1, 11, 4, 14, color);
            seg(display, x, y, 4, 14, 8, 14, color);
            seg(display, x, y, 8, 14, 10, 11, color);
            seg(display, x, y, 10, 11, 10, 0, color);
        }
        'V' => {
            seg(display, x, y, 1, 0, 5, 14, color);
            seg(display, x, y, 5, 14, 10, 0, color);
        }
        'Y' => {
            seg(display, x, y, 1, 0, 5, 7, color);
            seg(display, x, y, 10, 0, 5, 7, color);
            seg(display, x, y, 5, 7, 5, 14, color);
        }
        '0' => {
            seg(display, x, y, 3, 0, 8, 0, color);
            seg(display, x, y, 8, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 10, 11, color);
            seg(display, x, y, 10, 11, 8, 14, color);
            seg(display, x, y, 8, 14, 3, 14, color);
            seg(display, x, y, 3, 14, 1, 11, color);
            seg(display, x, y, 1, 11, 1, 3, color);
            seg(display, x, y, 1, 3, 3, 0, color);
        }
        '1' => {
            seg(display, x, y, 5, 0, 5, 14, color);
            seg(display, x, y, 3, 3, 5, 0, color);
            seg(display, x, y, 2, 14, 8, 14, color);
        }
        '2' => {
            seg(display, x, y, 2, 1, 8, 1, color);
            seg(display, x, y, 8, 1, 10, 4, color);
            seg(display, x, y, 10, 4, 1, 14, color);
            seg(display, x, y, 1, 14, 10, 14, color);
        }
        '3' => {
            seg(display, x, y, 2, 1, 8, 1, color);
            seg(display, x, y, 8, 1, 10, 4, color);
            seg(display, x, y, 6, 7, 10, 7, color);
            seg(display, x, y, 10, 7, 10, 11, color);
            seg(display, x, y, 10, 11, 8, 14, color);
            seg(display, x, y, 8, 14, 2, 14, color);
        }
        '4' => {
            seg(display, x, y, 9, 0, 9, 14, color);
            seg(display, x, y, 1, 0, 1, 7, color);
            seg(display, x, y, 1, 7, 10, 7, color);
        }
        '5' => {
            seg(display, x, y, 10, 1, 2, 1, color);
            seg(display, x, y, 2, 1, 2, 7, color);
            seg(display, x, y, 2, 7, 8, 7, color);
            seg(display, x, y, 8, 7, 10, 10, color);
            seg(display, x, y, 10, 10, 8, 14, color);
            seg(display, x, y, 8, 14, 1, 14, color);
        }
        '6' => {
            seg(display, x, y, 9, 1, 3, 1, color);
            seg(display, x, y, 3, 1, 1, 5, color);
            seg(display, x, y, 1, 5, 1, 11, color);
            seg(display, x, y, 1, 11, 3, 14, color);
            seg(display, x, y, 3, 14, 8, 14, color);
            seg(display, x, y, 8, 14, 10, 11, color);
            seg(display, x, y, 10, 11, 8, 8, color);
            seg(display, x, y, 8, 8, 1, 8, color);
        }
        '7' => {
            seg(display, x, y, 1, 1, 10, 1, color);
            seg(display, x, y, 10, 1, 4, 14, color);
        }
        '8' => {
            seg(display, x, y, 3, 0, 8, 0, color);
            seg(display, x, y, 8, 0, 10, 3, color);
            seg(display, x, y, 10, 3, 8, 7, color);
            seg(display, x, y, 8, 7, 10, 11, color);
            seg(display, x, y, 10, 11, 8, 14, color);
            seg(display, x, y, 8, 14, 3, 14, color);
            seg(display, x, y, 3, 14, 1, 11, color);
            seg(display, x, y, 1, 11, 3, 7, color);
            seg(display, x, y, 3, 7, 1, 3, color);
            seg(display, x, y, 1, 3, 3, 0, color);
            seg(display, x, y, 3, 7, 8, 7, color);
        }
        '9' => {
            seg(display, x, y, 8, 14, 10, 10, color);
            seg(display, x, y, 10, 10, 10, 3, color);
            seg(display, x, y, 10, 3, 8, 0, color);
            seg(display, x, y, 8, 0, 3, 0, color);
            seg(display, x, y, 3, 0, 1, 3, color);
            seg(display, x, y, 1, 3, 3, 7, color);
            seg(display, x, y, 3, 7, 10, 7, color);
        }
        '?' => {
            seg(display, x, y, 2, 2, 5, 0, color);
            seg(display, x, y, 5, 0, 9, 2, color);
            seg(display, x, y, 9, 2, 9, 5, color);
            seg(display, x, y, 9, 5, 5, 8, color);
            dot(display, x, y, 5, 13, color);
        }
        _ => {}
    }
}

fn seg<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb565,
) {
    let p0 = Point::new(x + x0 * LAYER_TITLE_W / 10, y + y0 * LAYER_TITLE_H / 14);
    let p1 = Point::new(x + x1 * LAYER_TITLE_W / 10, y + y1 * LAYER_TITLE_H / 14);
    let _ = Line::new(p0, p1)
        .into_styled(PrimitiveStyle::with_stroke(color, LAYER_TITLE_STROKE))
        .draw(display);
}

fn dot<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    color: Rgb565,
) {
    let p = Point::new(
        x + cx * LAYER_TITLE_W / 10 - 1,
        y + cy * LAYER_TITLE_H / 14 - 1,
    );
    let _ = Rectangle::new(Point::new(p.x, p.y), Size::new(3, 3))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display);
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

    draw_panel(display, x, y, w as u32, 50, COL_PANEL, COL_BORDER);

    let title = MonoTextStyle::new(&FONT_6X10, COL_MUTED);
    let percent = MonoTextStyle::new(&FONT_9X15, col);
    let top = TextStyleBuilder::new().baseline(Baseline::Top).build();
    let tr = TextStyleBuilder::new()
        .alignment(Alignment::Right)
        .baseline(Baseline::Top)
        .build();
    let _ = Text::with_text_style(side, Point::new(x + 10, y + 8), title, top).draw(display);
    let _ = Text::with_text_style(&label, Point::new(x + w - 12, y + 5), percent, tr).draw(display);

    let bx = x + 10;
    let by = y + 31;
    let bw = w - 28;
    let bh = 9u32;
    let bar = RoundedRectangle::with_equal_corners(
        Rectangle::new(Point::new(bx, by), Size::new(bw as u32, bh)),
        Size::new(BAR_RADIUS, BAR_RADIUS),
    );
    let bar_style = PrimitiveStyleBuilder::new()
        .fill_color(COL_BAR_BG)
        .stroke_color(COL_BORDER)
        .stroke_width(1)
        .build();
    let _ = bar.into_styled(bar_style).draw(display);
    draw_round_fill(display, bx + bw + 2, by + 3, 4, 3, 2, COL_BORDER);
    if let Some(pct) = fill_pct {
        if pct > 0 {
            let inner = (bw - 4).max(1) as u32;
            let fw = (inner * pct as u32 / 100).max(2);
            let fc = if pct < 10 {
                COL_RED
            } else if pct < 25 {
                COL_YELLOW
            } else {
                COL_BAR_FG
            };
            draw_round_fill(display, bx + 2, by + 2, fw, bh - 4, 3, fc);
        }
    }
}

fn layer_name(layer: u8) -> &'static str {
    match layer {
        0 => "BASE",
        1 => "NAV",
        2 => "SYM",
        3 => "ADJ",
        4 => "PTR",
        5 => "L5",
        6 => "L6",
        7 => "L7",
        8 => "L8",
        9 => "L9",
        10 => "L10",
        11 => "L11",
        12 => "L12",
        13 => "L13",
        14 => "L14",
        15 => "L15",
        _ => "?",
    }
}
