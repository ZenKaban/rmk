// k04-vial-settings-v0.0.61: IQS5xx motion applies Vial touch DPI and module auto layer.
// Scroll remains v0.0.29-style with a simple stateless divisor.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use embassy_nrf::twim::Twim;
use embassy_time::{Duration, Instant, Timer};
use rmk::descriptor::KeyboardReport;
use rmk::event::{Axis, Event};
use rmk::hid::Report;
use rmk::input_device::{InputProcessor, ProcessResult};
use rmk::keyboard::K04_MOUSE_BUTTONS;
use rmk::keymap::KeyMap;
use usbd_hid::descriptor::MouseReport;

use crate::vial_settings::{
    acceleration, auto_layer, auto_layer_enabled, auto_layer_timeout_ms, claim_pointing_module,
    encoder_interval_ms, encoder_module_enabled, handle_mode_key, handle_side_mode_key,
    invert_scroll, invert_text, kind_from_index, kind_index, orientation, pointing_mode,
    pointing_module_claimed_by_other, pointing_module_enabled, release_pointing_module,
    scale_touch_delta, sens, side_from_index, side_index, touch_gestures_enabled,
    wait_pointing_module_selection_change, ModuleKind, ModuleSide, PointingMode,
};

const IQS5XX_ADDR: u8 = 0x74;
const REG_PRODUCT_NUMBER: u16 = 0x0000;
const REG_PREVIOUS_CYCLE_TIME: u16 = 0x000c;
const REG_BOTTOM_BETA: u16 = 0x0637;
const REG_FILTER_SETTINGS: u16 = 0x0632;
const REG_STATIONARY_THRESH: u16 = 0x0672;
const REG_SYSTEM_CONTROL_0: u16 = 0x0431;
const REG_SYSTEM_CONTROL_1: u16 = 0x0432;
const REG_REPORT_RATE_ACTIVE: u16 = 0x057a;
const REG_REPORT_RATE_IDLE_TOUCH: u16 = 0x057c;
const REG_REPORT_RATE_IDLE: u16 = 0x057e;
const REG_IDLE_MODE_TIMEOUT: u16 = 0x0586;
const REG_SYSTEM_CONFIG_0: u16 = 0x058e;
const REG_SYSTEM_CONFIG_1: u16 = 0x058f;
const REG_XY_CONFIG_0: u16 = 0x0669;
const REG_SINGLE_FINGER_GESTURES: u16 = 0x06b7;
const REG_MULTI_FINGER_GESTURES: u16 = 0x06b8;
const REG_HOLD_TIME: u16 = 0x06bd;
const REG_SCROLL_INITIAL_DISTANCE: u16 = 0x06c8;
const REG_END_COMMS: u16 = 0xeeee;

const REPORT_RATE_MS: u16 = 5;
const TOUCH_POLL_INTERVAL_MS: u32 = 8;
const TOUCH_READ_FAILURE_REINIT_THRESHOLD: u8 = 4;
const CLICK_MS: u64 = 40;
const SCROLL_DIVISOR: i16 = 8;
const NORMAL_MOTION_ACCUM_LIMIT: i32 = (i8::MAX as i32) * 2;
const NORMAL_MOTION_STALE_MS: u32 = 20;
const TOUCH_PACED_REPORT_MS: u32 = 8;
const TOUCH_PACED_STALE_MS: u32 = 32;
const TOUCH_EVENT_BLOCK_LEN: usize = 10;
#[cfg(feature = "usb_debug")]
const DEBUG_MOTION_LOG_INTERVAL_MS: u32 = 50;

const GESTURE_0_SINGLE_TAP: u8 = 1 << 0;
const GESTURE_0_PRESS_AND_HOLD: u8 = 1 << 1;
const GESTURE_1_TWO_FINGER_TAP: u8 = 1 << 0;
const GESTURE_1_SCROLL: u8 = 1 << 1;
const SYSTEM_INFO_0_SHOW_RESET: u8 = 1 << 7;
const SYSTEM_INFO_1_TP_MOVEMENT: u8 = 1 << 0;
const SYSTEM_CONTROL_0_ACK_RESET: u8 = 1 << 7;
const FILTER_IIR: u8 = 1 << 0;
const FILTER_MAV: u8 = 1 << 1;
const FILTER_ALP_COUNT: u8 = 1 << 3;

const CUSTOM_MAGIC: [u8; 4] = *b"K04P";
const CUSTOM_BUTTON: u8 = 1;
const CUSTOM_SCROLL: u8 = 2;
const CUSTOM_MOTION: u8 = 3;
const CUSTOM_MODE: u8 = 4;
const BUTTON_LEFT: u8 = 1 << 0;
const BUTTON_RIGHT: u8 = 1 << 1;
const KEY_RIGHT: u8 = 0x4f;
const KEY_LEFT: u8 = 0x50;
const KEY_DOWN: u8 = 0x51;
const KEY_UP: u8 = 0x52;
const AUTO_LAYER_NONE: u8 = 0xff;

static ACTIVE_AUTO_LAYER: AtomicU8 = AtomicU8::new(AUTO_LAYER_NONE);
static LAST_AUTO_MOTION_MS: AtomicU32 = AtomicU32::new(0);
static AUTO_LAYER_HELD_KEYS: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum InitState {
    Pending,
    Ready,
}

enum TouchReadResult {
    Event(Event),
    NoEvent,
    ReadFailed,
}

pub struct Iqs5xxTouchpad {
    side: ModuleSide,
    i2c: Twim<'static>,
    state: InitState,
    read_failures: u8,
    next_poll_ms: u32,
}

impl Iqs5xxTouchpad {
    pub fn new(side: ModuleSide, i2c: Twim<'static>) -> Self {
        Self {
            side,
            i2c,
            state: InitState::Pending,
            read_failures: 0,
            next_poll_ms: 0,
        }
    }

    async fn init(&mut self) -> bool {
        // Wake / probe.
        let product = self.read_u16(REG_PRODUCT_NUMBER).await.unwrap_or(0);
        if product == 0 {
            return false;
        }
        ::defmt::info!("K04 IQS5xx product={}", product);

        // Keep the value that preserves working touchpad init on K04 hardware.
        // v0.0.27 tried bit1 reset (QMK/ZMK bitfield interpretation), but that
        // made the IQS5xx stop reporting touches on the tested left half.
        let _ = self.write_u8(REG_SYSTEM_CONTROL_1, 0x80).await;
        let _ = self.end_session().await;
        Timer::after(Duration::from_millis(100)).await;

        if self.read_u16(REG_PRODUCT_NUMBER).await.unwrap_or(0) == 0 {
            return false;
        }

        let mut ok = true;
        ok &= self.write_u16(REG_REPORT_RATE_ACTIVE, REPORT_RATE_MS).await;
        ok &= self
            .write_u16(REG_REPORT_RATE_IDLE_TOUCH, REPORT_RATE_MS)
            .await;
        ok &= self.write_u16(REG_REPORT_RATE_IDLE, REPORT_RATE_MS).await;
        ok &= self.write_u8(REG_IDLE_MODE_TIMEOUT, 255).await;
        // No RDY pin on K04 modules, so keep polling mode while enabling gesture/touch events.
        ok &= self.write_u8(REG_SYSTEM_CONFIG_1, 0x46).await;
        // Match the ZMK IQS5xx defaults: less raw jitter than zeroed filters,
        // while still keeping the touchpad responsive enough for cursor use.
        ok &= self.write_u8(REG_BOTTOM_BETA, 5).await;
        ok &= self.write_u8(REG_STATIONARY_THRESH, 5).await;
        ok &= self
            .write_u8(
                REG_FILTER_SETTINGS,
                FILTER_IIR | FILTER_MAV | FILTER_ALP_COUNT,
            )
            .await;
        // ROTATION_270 from Phenom/QMK without palm reject for touch drag testing.
        ok &= self.write_u8(REG_XY_CONFIG_0, 0x05).await;
        ok &= self.write_gesture_config().await;
        // ZMK uses setup_complete + WDT; QMK only sets REATI. This is a test to
        // make the IQS gesture engine accept the separately written config.
        ok &= self.write_u8(REG_SYSTEM_CONFIG_0, 0x60).await;
        ok &= self.end_session().await;

        if ok {
            self.state = InitState::Ready;
            self.read_failures = 0;
            self.reset_poll_timing();
            ::defmt::info!("K04 IQS5xx initialized");
        }
        ok
    }

    fn reset_poll_timing(&mut self) {
        self.next_poll_ms = 0;
    }

    async fn wait_next_poll(&mut self) {
        let now = now_ms();
        if self.next_poll_ms == 0 {
            self.next_poll_ms = now;
        }

        let wait_ms = self.next_poll_ms.wrapping_sub(now);
        if wait_ms > 1000 {
            self.next_poll_ms = now.wrapping_add(TOUCH_POLL_INTERVAL_MS);
            return;
        }

        if wait_ms != 0 {
            Timer::after(Duration::from_millis(
                wait_ms.min(TOUCH_POLL_INTERVAL_MS) as u64
            ))
            .await;
        }
        self.next_poll_ms = self.next_poll_ms.wrapping_add(TOUCH_POLL_INTERVAL_MS);
    }

    async fn read_touch_event(&mut self) -> TouchReadResult {
        let mut data = [0u8; TOUCH_EVENT_BLOCK_LEN];
        if !self.read(REG_PREVIOUS_CYCLE_TIME, &mut data).await {
            return TouchReadResult::ReadFailed;
        }

        let previous_cycle_time = data[0];
        let gesture_0 = data[1];
        let gesture_1 = data[2];
        let system_info_0 = data[3];
        let system_info_1 = data[4];
        let number_of_fingers = data[5];
        let movement_or_scroll =
            (system_info_1 & SYSTEM_INFO_1_TP_MOVEMENT) != 0 || (gesture_1 & GESTURE_1_SCROLL) != 0;
        let x = if movement_or_scroll {
            i16::from_be_bytes([data[6], data[7]])
        } else {
            0
        };
        let y = if movement_or_scroll {
            i16::from_be_bytes([data[8], data[9]])
        } else {
            0
        };

        if (system_info_0 & SYSTEM_INFO_0_SHOW_RESET) != 0 {
            let _ = self
                .write_u8(REG_SYSTEM_CONTROL_0, SYSTEM_CONTROL_0_ACK_RESET)
                .await;
            let _ = self.end_session().await;
            self.state = InitState::Pending;
            self.reset_poll_timing();
            return TouchReadResult::NoEvent;
        }

        if !self.end_session().await {
            return TouchReadResult::ReadFailed;
        }

        let gestures_enabled = touch_gestures_enabled(self.side);

        if gestures_enabled && (gesture_0 & (GESTURE_0_SINGLE_TAP | GESTURE_0_PRESS_AND_HOLD)) != 0
        {
            #[cfg(feature = "usb_debug")]
            log::info!(
                "tp click gesture side={} g0=0x{:02x} g1=0x{:02x} fingers={} raw=({}, {})",
                side_index(self.side),
                gesture_0,
                gesture_1,
                number_of_fingers,
                x,
                y
            );
            return TouchReadResult::Event(custom_button(BUTTON_LEFT));
        }

        // The previous_cycle_time guard matches the QMK workaround for duplicate two-finger clicks.
        if gestures_enabled
            && (gesture_1 & GESTURE_1_TWO_FINGER_TAP) != 0
            && previous_cycle_time != 0
        {
            #[cfg(feature = "usb_debug")]
            log::info!(
                "tp two-finger tap side={} g0=0x{:02x} g1=0x{:02x} fingers={}",
                side_index(self.side),
                gesture_0,
                gesture_1,
                number_of_fingers
            );
            return TouchReadResult::Event(custom_button(BUTTON_RIGHT));
        }

        if gestures_enabled
            && ((gesture_1 & GESTURE_1_SCROLL) != 0
                || (number_of_fingers >= 2 && (x != 0 || y != 0)))
        {
            return match self.scroll_event(x, y) {
                Some(event) => TouchReadResult::Event(event),
                None => TouchReadResult::NoEvent,
            };
        }

        if number_of_fingers != 1 {
            return TouchReadResult::NoEvent;
        }

        if x == 0 && y == 0 {
            return TouchReadResult::NoEvent;
        }
        TouchReadResult::Event(custom_motion(
            self.side,
            ModuleKind::Touch,
            x,
            y.saturating_neg(),
        ))
    }

    fn scroll_event(&mut self, x: i16, y: i16) -> Option<Event> {
        // Only one scrolling direction per report, same strategy as the ZMK driver.
        // v0.0.33: v0.0.29-style raw scroll path, only divided statelessly.
        // No accumulator and no forced ±1 events.
        if x != 0 {
            let h = x / SCROLL_DIVISOR;
            return if h != 0 {
                Some(custom_scroll(self.side, h, 0))
            } else {
                None
            };
        }

        if y != 0 {
            let v = y / SCROLL_DIVISOR;
            return if v != 0 {
                Some(custom_scroll(self.side, 0, v))
            } else {
                None
            };
        }

        None
    }

    async fn write_gesture_config(&mut self) -> bool {
        let mut ok = true;
        ok &= self
            .write_u8(
                REG_SINGLE_FINGER_GESTURES,
                GESTURE_0_SINGLE_TAP | GESTURE_0_PRESS_AND_HOLD,
            )
            .await;
        ok &= self
            .write_u8(
                REG_MULTI_FINGER_GESTURES,
                GESTURE_1_TWO_FINGER_TAP | GESTURE_1_SCROLL,
            )
            .await;
        ok &= self.write_u16(REG_HOLD_TIME, 0x012c).await;
        // Lower than QMK's default 0x32 to make entering scroll mode easier on K04.
        ok &= self.write_u16(REG_SCROLL_INITIAL_DISTANCE, 0x0001).await;
        ok
    }

    async fn read_u16(&mut self, reg: u16) -> Option<u16> {
        let mut buf = [0u8; 2];
        if self.read(reg, &mut buf).await {
            Some(u16::from_be_bytes(buf))
        } else {
            None
        }
    }

    async fn read_i16(&mut self, reg: u16) -> Option<i16> {
        self.read_u16(reg).await.map(|v| v as i16)
    }

    async fn read_u8(&mut self, reg: u16) -> Option<u8> {
        let mut buf = [0u8; 1];
        if self.read(reg, &mut buf).await {
            Some(buf[0])
        } else {
            None
        }
    }

    async fn read(&mut self, reg: u16, out: &mut [u8]) -> bool {
        self.i2c
            .write_read(IQS5XX_ADDR, &reg.to_be_bytes(), out)
            .await
            .is_ok()
    }

    async fn write_u8(&mut self, reg: u16, val: u8) -> bool {
        let bytes = [reg.to_be_bytes()[0], reg.to_be_bytes()[1], val];
        self.i2c.write(IQS5XX_ADDR, &bytes).await.is_ok()
    }

    async fn write_u16(&mut self, reg: u16, val: u16) -> bool {
        let r = reg.to_be_bytes();
        let v = val.to_be_bytes();
        let bytes = [r[0], r[1], v[0], v[1]];
        self.i2c.write(IQS5XX_ADDR, &bytes).await.is_ok()
    }

    async fn end_session(&mut self) -> bool {
        let r = REG_END_COMMS.to_be_bytes();
        self.i2c.write(IQS5XX_ADDR, &[r[0], r[1], 0]).await.is_ok()
    }
}

impl rmk::input_device::InputDevice for Iqs5xxTouchpad {
    async fn read_event(&mut self) -> Event {
        loop {
            if !pointing_module_enabled(self.side, ModuleKind::Touch) {
                release_pointing_module(ModuleKind::Touch);
                self.state = InitState::Pending;
                self.read_failures = 0;
                self.reset_poll_timing();
                wait_pointing_module_selection_change(self.side, ModuleKind::Touch).await;
                continue;
            }

            if pointing_module_claimed_by_other(ModuleKind::Touch) {
                self.state = InitState::Pending;
                self.read_failures = 0;
                self.reset_poll_timing();
                Timer::after(Duration::from_millis(50)).await;
                continue;
            }

            if self.state != InitState::Ready {
                release_pointing_module(ModuleKind::Touch);
                if !self.init().await {
                    self.read_failures = 0;
                    self.reset_poll_timing();
                    Timer::after(Duration::from_millis(500)).await;
                    continue;
                }
                if !claim_pointing_module(ModuleKind::Touch) {
                    self.state = InitState::Pending;
                    self.read_failures = 0;
                    self.reset_poll_timing();
                    Timer::after(Duration::from_millis(50)).await;
                    continue;
                }
            }

            self.wait_next_poll().await;
            match self.read_touch_event().await {
                TouchReadResult::Event(event) => {
                    self.read_failures = 0;
                    return event;
                }
                TouchReadResult::NoEvent => {
                    self.read_failures = 0;
                }
                TouchReadResult::ReadFailed => {
                    self.read_failures = self.read_failures.saturating_add(1);
                    if self.read_failures >= TOUCH_READ_FAILURE_REINIT_THRESHOLD {
                        self.state = InitState::Pending;
                        self.read_failures = 0;
                        self.reset_poll_timing();
                        Timer::after(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
}

pub struct K04PointingProcessor<
    'a,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
> {
    keymap: &'a RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>>,
    normal_accum_x: i32,
    normal_accum_y: i32,
    touch_accum_x: i32,
    touch_accum_y: i32,
    scroll_accum_h: i32,
    scroll_accum_v: i32,
    text_accum_x: i32,
    text_accum_y: i32,
    normal_last_motion_ms: u32,
    touch_last_motion_ms: u32,
    touch_last_flush_ms: u32,
    touch_last_divisor: i32,
    right_encoder_last_press_ms: u32,
    #[cfg(feature = "usb_debug")]
    debug_last_motion_ms: u32,
}

impl<'a, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    K04PointingProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    pub fn new(keymap: &'a RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>>) -> Self {
        Self {
            keymap,
            normal_accum_x: 0,
            normal_accum_y: 0,
            touch_accum_x: 0,
            touch_accum_y: 0,
            scroll_accum_h: 0,
            scroll_accum_v: 0,
            text_accum_x: 0,
            text_accum_y: 0,
            normal_last_motion_ms: 0,
            touch_last_motion_ms: 0,
            touch_last_flush_ms: 0,
            touch_last_divisor: 1,
            right_encoder_last_press_ms: 0,
            #[cfg(feature = "usb_debug")]
            debug_last_motion_ms: 0,
        }
    }

    async fn send_mouse(&self, buttons: u8, x: i16, y: i16, wheel: i16, pan: i16) {
        if buttons != 0 || x != 0 || y != 0 || wheel != 0 || pan != 0 {
            rmk::channel::signal_activity();
        }
        let report = self.mouse_report(buttons, x, y, wheel, pan);
        rmk::channel::KEYBOARD_REPORT_CHANNEL
            .send(Report::MouseReport(report))
            .await;
    }

    fn try_send_mouse(&self, buttons: u8, x: i16, y: i16, wheel: i16, pan: i16) -> bool {
        if buttons != 0 || x != 0 || y != 0 || wheel != 0 || pan != 0 {
            rmk::channel::signal_activity();
        }
        let report = self.mouse_report(buttons, x, y, wheel, pan);
        let report = Report::MouseReport(report);
        match rmk::channel::KEYBOARD_REPORT_CHANNEL.try_send(report.clone()) {
            Ok(()) => true,
            Err(_) => match rmk::channel::KEYBOARD_REPORT_CHANNEL.try_receive() {
                Ok(Report::MouseReport(_)) => rmk::channel::KEYBOARD_REPORT_CHANNEL
                    .try_send(report)
                    .is_ok(),
                Ok(old_report) => {
                    let _ = rmk::channel::KEYBOARD_REPORT_CHANNEL.try_send(old_report);
                    false
                }
                Err(_) => rmk::channel::KEYBOARD_REPORT_CHANNEL
                    .try_send(report)
                    .is_ok(),
            },
        }
    }

    fn mouse_report(&self, buttons: u8, x: i16, y: i16, wheel: i16, pan: i16) -> MouseReport {
        let held_buttons = K04_MOUSE_BUTTONS.load(Ordering::Relaxed);
        MouseReport {
            buttons: held_buttons | buttons,
            x: x.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
            y: y.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
            wheel: wheel.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
            pan: pan.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
        }
    }

    async fn send_normal_motion(&mut self, x: i16, y: i16) {
        let now = now_ms();
        if now.wrapping_sub(self.normal_last_motion_ms) > NORMAL_MOTION_STALE_MS {
            self.normal_accum_x = 0;
            self.normal_accum_y = 0;
        }
        self.normal_last_motion_ms = now;

        self.normal_accum_x =
            clamp_normal_motion_accum(self.normal_accum_x.saturating_add(x as i32));
        self.normal_accum_y =
            clamp_normal_motion_accum(self.normal_accum_y.saturating_add(y as i32));

        if self.normal_accum_x != 0 || self.normal_accum_y != 0 {
            let report_x = self.normal_accum_x.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
            let report_y = self.normal_accum_y.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
            if self.try_send_mouse(0, report_x, report_y, 0, 0) {
                self.normal_accum_x -= report_x as i32;
                self.normal_accum_y -= report_y as i32;
            } else {
                #[cfg(feature = "usb_debug")]
                log::debug!(
                    "tp mouse queue full accum=({}, {}) report=({}, {})",
                    self.normal_accum_x,
                    self.normal_accum_y,
                    report_x,
                    report_y
                );
            }
        }
    }

    async fn send_touch_motion_paced(&mut self, x: i16, y: i16) {
        self.send_touch_motion_paced_divided(x, y, 1).await;
    }

    async fn send_touch_motion_paced_divided(&mut self, x: i16, y: i16, divisor: i16) {
        let divisor = i32::from(divisor.max(1));
        let now = now_ms();
        if divisor != self.touch_last_divisor
            || now.wrapping_sub(self.touch_last_motion_ms) > TOUCH_PACED_STALE_MS
        {
            self.touch_accum_x = 0;
            self.touch_accum_y = 0;
            self.touch_last_flush_ms = now.saturating_sub(TOUCH_PACED_REPORT_MS);
            self.touch_last_divisor = divisor;
        }
        self.touch_last_motion_ms = now;

        self.touch_accum_x =
            clamp_scaled_motion_accum(self.touch_accum_x.saturating_add(x as i32), divisor);
        self.touch_accum_y =
            clamp_scaled_motion_accum(self.touch_accum_y.saturating_add(y as i32), divisor);

        let force_x = i8::MAX as u32 * divisor as u32;
        let force_y = i8::MAX as u32 * divisor as u32;
        let should_flush = now.wrapping_sub(self.touch_last_flush_ms) >= TOUCH_PACED_REPORT_MS
            || self.touch_accum_x.unsigned_abs() >= force_x
            || self.touch_accum_y.unsigned_abs() >= force_y;
        if !should_flush {
            return;
        }

        let report_x = pending_steps(self.touch_accum_x, divisor)
            .clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i16;
        let report_y = pending_steps(self.touch_accum_y, divisor)
            .clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i16;
        if report_x == 0 && report_y == 0 {
            return;
        }
        if self.try_send_mouse(0, report_x, report_y, 0, 0) {
            self.touch_last_flush_ms = now;
            self.touch_accum_x -= i32::from(report_x) * divisor;
            self.touch_accum_y -= i32::from(report_y) * divisor;
        }
    }

    fn clear_touch_motion_pacer(&mut self) {
        self.touch_accum_x = 0;
        self.touch_accum_y = 0;
        self.touch_last_motion_ms = now_ms();
        self.touch_last_flush_ms = self
            .touch_last_motion_ms
            .saturating_sub(TOUCH_PACED_REPORT_MS);
        self.touch_last_divisor = 1;
    }

    async fn send_scroll_motion(&mut self, side: ModuleSide, x: i16, y: i16) {
        let divisor = i32::from(sens(side, PointingMode::Scroll));
        let invert = if invert_scroll(side) { -1 } else { 1 };
        self.scroll_accum_h = self.scroll_accum_h.saturating_add(i32::from(x) * invert);
        self.scroll_accum_v = self.scroll_accum_v.saturating_add(i32::from(y) * invert);

        let h = pending_steps(self.scroll_accum_h, divisor);
        let v = pending_steps(self.scroll_accum_v, divisor);
        if (h != 0 || v != 0) && self.try_send_mouse(0, 0, 0, v as i16, h as i16) {
            self.scroll_accum_h -= h * divisor;
            self.scroll_accum_v -= v * divisor;
        }
    }

    async fn send_text_motion(&mut self, side: ModuleSide, x: i16, y: i16) {
        let divisor = i32::from(sens(side, PointingMode::Text));
        let invert = if invert_text(side) { -1 } else { 1 };
        self.text_accum_x = self.text_accum_x.saturating_add(i32::from(x) * invert);
        self.text_accum_y = self.text_accum_y.saturating_add(i32::from(y) * invert);

        let x_steps = drain_steps(&mut self.text_accum_x, divisor).clamp(-4, 4);
        let y_steps = drain_steps(&mut self.text_accum_y, divisor).clamp(-4, 4);

        for _ in 0..x_steps.unsigned_abs() {
            self.tap_key(if x_steps > 0 { KEY_RIGHT } else { KEY_LEFT })
                .await;
        }
        for _ in 0..y_steps.unsigned_abs() {
            self.tap_key(if y_steps > 0 { KEY_DOWN } else { KEY_UP })
                .await;
        }
    }

    async fn tap_key(&self, keycode: u8) {
        self.send_keyboard_key(keycode).await;
        Timer::after(Duration::from_millis(20)).await;
        self.send_keyboard_key(0).await;
    }

    async fn send_keyboard_key(&self, keycode: u8) {
        let mut report = KeyboardReport::default();
        if keycode != 0 {
            report.keycodes[0] = keycode;
        }
        rmk::channel::KEYBOARD_REPORT_CHANNEL
            .send(Report::KeyboardReport(report))
            .await;
    }

    async fn send_configured_motion(&mut self, side: ModuleSide, kind: ModuleKind, x: i16, y: i16) {
        let (mut x, mut y) = rotate_motion(x, y, orientation(side, kind));
        #[cfg(feature = "usb_debug")]
        let debug_rotated = (x, y);
        let held_buttons = K04_MOUSE_BUTTONS.load(Ordering::Relaxed);
        let is_touch_drag = kind == ModuleKind::Touch && held_buttons != 0;
        if !is_touch_drag {
            self.sync_auto_layer_for_motion(side);
        }
        if kind == ModuleKind::Touch {
            x = scale_touch_delta(x, side);
            y = scale_touch_delta(y, side);
        }
        if acceleration(side) && !is_touch_drag {
            x = accelerate_axis(x);
            y = accelerate_axis(y);
        }
        #[cfg(feature = "usb_debug")]
        let debug_scaled = (x, y);

        #[cfg(feature = "usb_debug")]
        if kind == ModuleKind::Touch && (x != 0 || y != 0) {
            let now = now_ms();
            if now.wrapping_sub(self.debug_last_motion_ms) >= DEBUG_MOTION_LOG_INTERVAL_MS {
                self.debug_last_motion_ms = now;
                log::debug!(
                    "tp motion side={} held=0x{:02x} rot=({}, {}) scaled=({}, {}) out=({}, {})",
                    side_index(side),
                    held_buttons,
                    debug_rotated.0,
                    debug_rotated.1,
                    debug_scaled.0,
                    debug_scaled.1,
                    x,
                    y
                );
            }
        }

        if is_touch_drag {
            self.clear_touch_motion_pacer();
            self.send_normal_motion(x, y).await;
            return;
        }

        match pointing_mode(side) {
            PointingMode::Normal if kind == ModuleKind::Touch => {
                self.send_touch_motion_paced(x, y).await
            }
            PointingMode::Normal => self.send_normal_motion(x, y).await,
            PointingMode::Sniper => {
                let divisor = sens(side, PointingMode::Sniper);
                if kind == ModuleKind::Touch {
                    self.send_touch_motion_paced_divided(x, y, divisor).await;
                } else {
                    self.send_normal_motion(x / divisor, y / divisor).await;
                }
            }
            PointingMode::Scroll => self.send_scroll_motion(side, x, y).await,
            PointingMode::Text => self.send_text_motion(side, x, y).await,
        }
    }

    async fn click(&self, buttons: u8) {
        self.send_mouse(buttons, 0, 0, 0, 0).await;
        Timer::after(Duration::from_millis(CLICK_MS)).await;
        self.send_mouse(0, 0, 0, 0, 0).await;
    }

    fn sync_auto_layer_for_motion(&self, side: ModuleSide) {
        let mode = pointing_mode(side);
        if auto_layer_enabled(mode) {
            let layer = auto_layer();
            LAST_AUTO_MOTION_MS.store(now_ms(), Ordering::Relaxed);
            let previous = ACTIVE_AUTO_LAYER.swap(layer, Ordering::Relaxed);
            if previous != layer {
                let mut keymap = self.keymap.borrow_mut();
                if previous != AUTO_LAYER_NONE {
                    keymap.deactivate_layer(previous);
                }
                if layer != 0 {
                    keymap.activate_layer(layer);
                }
            }
        } else {
            self.deactivate_auto_layer();
        }
    }

    fn deactivate_auto_layer(&self) {
        let previous = ACTIVE_AUTO_LAYER.swap(AUTO_LAYER_NONE, Ordering::Relaxed);
        AUTO_LAYER_HELD_KEYS.store(0, Ordering::Relaxed);
        if previous != AUTO_LAYER_NONE {
            self.keymap.borrow_mut().deactivate_layer(previous);
        }
    }

    fn sync_auto_layer_for_key(&self, pressed: bool) {
        if ACTIVE_AUTO_LAYER.load(Ordering::Relaxed) == AUTO_LAYER_NONE {
            return;
        }

        if pressed {
            let _ =
                AUTO_LAYER_HELD_KEYS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    Some(value.saturating_add(1))
                });
        } else {
            let _ =
                AUTO_LAYER_HELD_KEYS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    Some(value.saturating_sub(1))
                });
            LAST_AUTO_MOTION_MS.store(now_ms(), Ordering::Relaxed);
        }
    }
}

impl<'a, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    InputProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>
    for K04PointingProcessor<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn process(&mut self, event: Event) -> ProcessResult {
        match event {
            Event::Key(key_event) => {
                if key_event.rotary_encoder_id() == Some(1) {
                    if !encoder_module_enabled(ModuleSide::Right) {
                        return ProcessResult::Stop;
                    }
                    if key_event.pressed() {
                        let now = now_ms();
                        let interval = encoder_interval_ms(ModuleSide::Right) as u32;
                        if interval != 0
                            && self.right_encoder_last_press_ms != 0
                            && now.wrapping_sub(self.right_encoder_last_press_ms) < interval
                        {
                            return ProcessResult::Stop;
                        }
                        self.right_encoder_last_press_ms = now;
                    }
                }
                self.sync_auto_layer_for_key(key_event.pressed());
                ProcessResult::Continue(Event::Key(key_event))
            }
            Event::Joystick(axis_events) => {
                let mut x = 0i16;
                let mut y = 0i16;
                for axis_event in axis_events.iter() {
                    match axis_event.axis {
                        Axis::X => x = axis_event.value,
                        Axis::Y => y = axis_event.value,
                        _ => {}
                    }
                }
                self.send_normal_motion(x, y).await;
                ProcessResult::Stop
            }
            Event::Custom(data) if data[0..4] == CUSTOM_MAGIC => {
                match data[4] {
                    CUSTOM_BUTTON => self.click(data[5]).await,
                    CUSTOM_SCROLL => {
                        let side = side_from_index(data[5]);
                        let h = i16::from_be_bytes([data[6], data[7]]);
                        let v = i16::from_be_bytes([data[8], data[9]]);
                        let invert = if invert_scroll(side) { -1 } else { 1 };
                        self.send_mouse(0, 0, 0, v * invert, h * invert).await;
                    }
                    CUSTOM_MOTION => {
                        let side = side_from_index(data[5]);
                        let kind = kind_from_index(data[6]);
                        let x = i16::from_be_bytes([data[7], data[8]]);
                        let y = i16::from_be_bytes([data[9], data[10]]);
                        self.send_configured_motion(side, kind, x, y).await;
                    }
                    CUSTOM_MODE => {
                        let mode = match data[5] {
                            1 => PointingMode::Sniper,
                            2 => PointingMode::Scroll,
                            3 => PointingMode::Text,
                            _ => PointingMode::Normal,
                        };
                        match data[8] {
                            0 => handle_side_mode_key(
                                ModuleSide::Left,
                                mode,
                                data[6] != 0,
                                data[7] != 0,
                            ),
                            1 => handle_side_mode_key(
                                ModuleSide::Right,
                                mode,
                                data[6] != 0,
                                data[7] != 0,
                            ),
                            _ => handle_mode_key(mode, data[6] != 0, data[7] != 0),
                        }
                    }
                    _ => {}
                }
                ProcessResult::Stop
            }
            _ => ProcessResult::Continue(event),
        }
    }

    fn get_keymap(&self) -> &RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>> {
        self.keymap
    }
}

fn custom_button(buttons: u8) -> Event {
    let mut data = [0u8; 16];
    data[0..4].copy_from_slice(&CUSTOM_MAGIC);
    data[4] = CUSTOM_BUTTON;
    data[5] = buttons;
    Event::Custom(data)
}

fn custom_motion(side: ModuleSide, kind: ModuleKind, x: i16, y: i16) -> Event {
    let mut data = [0u8; 16];
    data[0..4].copy_from_slice(&CUSTOM_MAGIC);
    data[4] = CUSTOM_MOTION;
    data[5] = side_index(side);
    data[6] = kind_index(kind);
    data[7..9].copy_from_slice(&x.to_be_bytes());
    data[9..11].copy_from_slice(&y.to_be_bytes());
    Event::Custom(data)
}

fn clamp_normal_motion_accum(value: i32) -> i32 {
    value.clamp(-NORMAL_MOTION_ACCUM_LIMIT, NORMAL_MOTION_ACCUM_LIMIT)
}

fn clamp_scaled_motion_accum(value: i32, divisor: i32) -> i32 {
    let divisor = divisor.max(1);
    let limit = NORMAL_MOTION_ACCUM_LIMIT.saturating_mul(divisor);
    value.clamp(-limit, limit)
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

fn custom_scroll(side: ModuleSide, h: i16, v: i16) -> Event {
    let mut data = [0u8; 16];
    data[0..4].copy_from_slice(&CUSTOM_MAGIC);
    data[4] = CUSTOM_SCROLL;
    data[5] = side_index(side);
    data[6..8].copy_from_slice(&h.to_be_bytes());
    data[8..10].copy_from_slice(&v.to_be_bytes());
    Event::Custom(data)
}

fn rotate_motion(x: i16, y: i16, orientation: u8) -> (i16, i16) {
    match orientation {
        1 => (y, x.saturating_neg()),
        2 => (x.saturating_neg(), y.saturating_neg()),
        3 => (y.saturating_neg(), x),
        _ => (x, y),
    }
}

fn accelerate_axis(value: i16) -> i16 {
    if value.unsigned_abs() > 10 {
        value.saturating_mul(2)
    } else {
        value
    }
}

fn drain_steps(accum: &mut i32, divisor: i32) -> i16 {
    if divisor <= 0 {
        return 0;
    }
    let steps = pending_steps(*accum, divisor);
    *accum -= steps * divisor;
    steps as i16
}

fn pending_steps(accum: i32, divisor: i32) -> i32 {
    if divisor <= 0 {
        return 0;
    }
    (accum / divisor).clamp(i32::from(i8::MIN), i32::from(i8::MAX))
}

pub type K04Touchpad = Iqs5xxTouchpad;

pub async fn auto_layer_idle_loop<
    'a,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
>(
    keymap: &'a RefCell<KeyMap<'a, ROW, COL, NUM_LAYER, NUM_ENCODER>>,
) {
    loop {
        Timer::after(Duration::from_millis(50)).await;
        let active = ACTIVE_AUTO_LAYER.load(Ordering::Relaxed);
        if active == AUTO_LAYER_NONE {
            continue;
        }
        if AUTO_LAYER_HELD_KEYS.load(Ordering::Relaxed) != 0 {
            continue;
        }
        let elapsed = now_ms().wrapping_sub(LAST_AUTO_MOTION_MS.load(Ordering::Relaxed));
        if elapsed >= auto_layer_timeout_ms() {
            let previous = ACTIVE_AUTO_LAYER.swap(AUTO_LAYER_NONE, Ordering::Relaxed);
            if previous != AUTO_LAYER_NONE {
                keymap.borrow_mut().deactivate_layer(previous);
            }
        }
    }
}

#[embassy_executor::task]
pub async fn touchpad_task(mut touchpad: K04Touchpad) {
    use rmk::input_device::InputDevice;

    Timer::after(Duration::from_millis(200)).await;
    loop {
        let event = touchpad.read_event().await;
        if rmk::channel::EVENT_CHANNEL.try_send(event).is_err() {
            #[cfg(feature = "usb_debug")]
            log::debug!("tp event channel full, dropping one old event");
            let _ = rmk::channel::EVENT_CHANNEL.receive().await;
            let _ = rmk::channel::EVENT_CHANNEL.try_send(event);
        }
    }
}
