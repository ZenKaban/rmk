use embassy_nrf::twim::Twim;
use embassy_time::{Duration, Instant, Timer};
use rmk::event::{publish_event, Axis, AxisEvent, AxisValType, LayerChangeEvent, PointingEvent};
use rmk::macros::processor;

use crate::module_settings;

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
const TOUCH_EVENT_BLOCK_LEN: usize = 10;
const TOUCH_PROBE_INTERVAL_MS: u32 = 500;
const TOUCH_READ_FAILURE_REINIT_THRESHOLD: u8 = 4;
const TOUCH_MOTION_ACCUM_LIMIT: i32 = (i8::MAX as i32) * 2;
const TOUCH_REPORT_INTERVAL_MS: u32 = 8;
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

#[processor(subscribe = [LayerChangeEvent], poll_interval = 8)]
pub struct Touchpad {
    i2c: Twim<'static>,
    device_id: u8,
    side: u8,
    ready: bool,
    read_failures: u8,
    acc_x: i32,
    acc_y: i32,
    last_report_ms: u32,
    next_probe_ms: u32,
}

impl Touchpad {
    pub fn new(device_id: u8, i2c: Twim<'static>) -> Self {
        Self {
            i2c,
            device_id,
            side: side_for_device_id(device_id),
            ready: false,
            read_failures: 0,
            acc_x: 0,
            acc_y: 0,
            last_report_ms: 0,
            next_probe_ms: 0,
        }
    }

    async fn on_layer_change_event(&mut self, _event: LayerChangeEvent) {}

    async fn poll(&mut self) {
        if !self.ready {
            let now = now_ms();
            if self.next_probe_ms != 0 && now.wrapping_sub(self.next_probe_ms) < u32::MAX / 2 {
                return;
            }

            if !self.init().await {
                self.next_probe_ms = now.wrapping_add(TOUCH_PROBE_INTERVAL_MS);
                Timer::after(Duration::from_millis(1)).await;
                return;
            }
            self.last_report_ms = now_ms();
        }

        match self.read_motion().await {
            TouchReadResult::Motion { x, y } => {
                self.read_failures = 0;
                let x = module_settings::scale_touch_delta(x, self.side);
                let y = module_settings::scale_touch_delta(y, self.side);
                self.acc_x = clamp_motion_accum(self.acc_x.saturating_add(x as i32));
                self.acc_y = clamp_motion_accum(self.acc_y.saturating_add(y as i32));
            }
            TouchReadResult::NoMotion => {
                self.read_failures = 0;
            }
            TouchReadResult::ReadFailed => {
                self.read_failures = self.read_failures.saturating_add(1);
                if self.read_failures >= TOUCH_READ_FAILURE_REINIT_THRESHOLD {
                    self.reset();
                    Timer::after(Duration::from_millis(50)).await;
                }
                return;
            }
        }

        let now = now_ms();
        if now.wrapping_sub(self.last_report_ms) >= TOUCH_REPORT_INTERVAL_MS {
            self.send_accumulated_motion();
            self.last_report_ms = now;
        }
    }

    async fn init(&mut self) -> bool {
        let product = self.read_u16(REG_PRODUCT_NUMBER).await.unwrap_or(0);
        if product == 0 {
            return false;
        }

        let _ = self.write_u8(REG_SYSTEM_CONTROL_1, 0x80).await;
        let _ = self.end_session().await;
        Timer::after(Duration::from_millis(100)).await;

        if self.read_u16(REG_PRODUCT_NUMBER).await.unwrap_or(0) == 0 {
            return false;
        }

        let mut ok = true;
        ok &= self.write_u16(REG_REPORT_RATE_ACTIVE, REPORT_RATE_MS).await;
        ok &= self.write_u16(REG_REPORT_RATE_IDLE_TOUCH, REPORT_RATE_MS).await;
        ok &= self.write_u16(REG_REPORT_RATE_IDLE, REPORT_RATE_MS).await;
        ok &= self.write_u8(REG_IDLE_MODE_TIMEOUT, 255).await;
        ok &= self.write_u8(REG_SYSTEM_CONFIG_1, 0x46).await;
        ok &= self.write_u8(REG_BOTTOM_BETA, 5).await;
        ok &= self.write_u8(REG_STATIONARY_THRESH, 5).await;
        ok &= self
            .write_u8(REG_FILTER_SETTINGS, FILTER_IIR | FILTER_MAV | FILTER_ALP_COUNT)
            .await;
        ok &= self.write_u8(REG_XY_CONFIG_0, 0x05).await;
        ok &= self
            .write_u8(
                REG_SINGLE_FINGER_GESTURES,
                GESTURE_0_SINGLE_TAP | GESTURE_0_PRESS_AND_HOLD,
            )
            .await;
        ok &= self
            .write_u8(REG_MULTI_FINGER_GESTURES, GESTURE_1_TWO_FINGER_TAP | GESTURE_1_SCROLL)
            .await;
        ok &= self.write_u16(REG_HOLD_TIME, 0x012c).await;
        ok &= self.write_u16(REG_SCROLL_INITIAL_DISTANCE, 0x0001).await;
        ok &= self.write_u8(REG_SYSTEM_CONFIG_0, 0x60).await;
        ok &= self.end_session().await;

        if ok {
            self.ready = true;
            self.read_failures = 0;
            self.next_probe_ms = 0;
            self.acc_x = 0;
            self.acc_y = 0;
        }
        ok
    }

    async fn read_motion(&mut self) -> TouchReadResult {
        let mut data = [0u8; TOUCH_EVENT_BLOCK_LEN];
        if !self.read(REG_PREVIOUS_CYCLE_TIME, &mut data).await {
            return TouchReadResult::ReadFailed;
        }

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
            let _ = self.write_u8(REG_SYSTEM_CONTROL_0, SYSTEM_CONTROL_0_ACK_RESET).await;
            let _ = self.end_session().await;
            self.reset();
            return TouchReadResult::NoMotion;
        }

        if !self.end_session().await {
            return TouchReadResult::ReadFailed;
        }

        if number_of_fingers != 1 || (x == 0 && y == 0) {
            return TouchReadResult::NoMotion;
        }

        TouchReadResult::Motion {
            x,
            y: y.saturating_neg(),
        }
    }

    fn reset(&mut self) {
        self.ready = false;
        self.read_failures = 0;
        self.acc_x = 0;
        self.acc_y = 0;
        self.next_probe_ms = now_ms().wrapping_add(TOUCH_PROBE_INTERVAL_MS);
    }

    fn send_accumulated_motion(&mut self) {
        if self.acc_x == 0 && self.acc_y == 0 {
            return;
        }

        let report_x = self.acc_x.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        let report_y = self.acc_y.clamp(i8::MIN as i32, i8::MAX as i32) as i16;
        self.acc_x -= report_x as i32;
        self.acc_y -= report_y as i32;

        publish_event(PointingEvent {
            device_id: self.device_id,
            axes: [
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::X,
                    value: report_x,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Y,
                    value: report_y,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Z,
                    value: 0,
                },
            ],
        });
    }

    async fn read_u16(&mut self, reg: u16) -> Option<u16> {
        let mut buf = [0u8; 2];
        if self.read(reg, &mut buf).await {
            Some(u16::from_be_bytes(buf))
        } else {
            None
        }
    }

    async fn read(&mut self, reg: u16, out: &mut [u8]) -> bool {
        self.i2c.write_read(IQS5XX_ADDR, &reg.to_be_bytes(), out).await.is_ok()
    }

    async fn write_u8(&mut self, reg: u16, val: u8) -> bool {
        let r = reg.to_be_bytes();
        self.i2c.write(IQS5XX_ADDR, &[r[0], r[1], val]).await.is_ok()
    }

    async fn write_u16(&mut self, reg: u16, val: u16) -> bool {
        let r = reg.to_be_bytes();
        let v = val.to_be_bytes();
        self.i2c.write(IQS5XX_ADDR, &[r[0], r[1], v[0], v[1]]).await.is_ok()
    }

    async fn end_session(&mut self) -> bool {
        let r = REG_END_COMMS.to_be_bytes();
        self.i2c.write(IQS5XX_ADDR, &[r[0], r[1], 0]).await.is_ok()
    }
}

fn side_for_device_id(device_id: u8) -> u8 {
    match device_id {
        2 => 0,
        3 => 1,
        _ => device_id.min(1),
    }
}

enum TouchReadResult {
    Motion { x: i16, y: i16 },
    NoMotion,
    ReadFailed,
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

fn clamp_motion_accum(value: i32) -> i32 {
    value.clamp(-TOUCH_MOTION_ACCUM_LIMIT, TOUCH_MOTION_ACCUM_LIMIT)
}
