//! Trackball Mini v3.0 mode bridge for root RMK.

use embassy_time::{Duration, Instant, Timer};
use rmk::channel::send_hid_report;
use rmk::event::{publish_event_async, ActionEvent, KeyboardEvent, PointingProcessorEvent};
use rmk::hid::Report;
use rmk::input_device::pointing::{CursorConfig, PointingMode, ScrollConfig, SniperConfig};
use rmk::macros::processor;
use rmk::types::action::Action;
use rmk::types::keycode::{HidKeyCode, KeyCode};
use usbd_hid::descriptor::MouseReport;

const TRACKBALL_DEVICE_ID: u8 = 0;
const COMBO_WINDOW_MS: u32 = 100;
const COMBO_TAP_MS: u32 = 250;
const DOUBLE_TAP_MS: u32 = 400;
const SCROLL_DIVISOR_DEFAULT: u8 = 5;
const SCROLL_DIVISOR_MIN: u8 = 1;
const SCROLL_DIVISOR_MAX: u8 = 32;
const SNIPER_DIVISOR_DEFAULT: u8 = 4;
const SNIPER_DIVISOR_MIN: u8 = 1;
const SNIPER_DIVISOR_MAX: u8 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cursor,
    Scroll,
    Sniper,
}

#[processor(subscribe = [ActionEvent])]
pub struct TrackballModeProcessor {
    mode: Mode,
    adjust: bool,
    mb1_held: bool,
    mb2_held: bool,
    mb1_press_time: u32,
    mb2_press_time: u32,
    combo_active: bool,
    combo_start_time: u32,
    last_combo_tap_time: u32,
    scroll_divisor: u8,
    sniper_divisor: u8,
}

impl TrackballModeProcessor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Cursor,
            adjust: false,
            mb1_held: false,
            mb2_held: false,
            mb1_press_time: 0,
            mb2_press_time: 0,
            combo_active: false,
            combo_start_time: 0,
            last_combo_tap_time: 0,
            scroll_divisor: SCROLL_DIVISOR_DEFAULT,
            sniper_divisor: SNIPER_DIVISOR_DEFAULT,
        }
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        let pressed = event.keyboard_event.pressed;
        let now = now_ms();

        if is_hid(event.action, HidKeyCode::MouseBtn1) {
            self.on_mb1(pressed, now).await;
        } else if matches!(event.action, Action::User(14)) {
            self.on_user14(pressed, now).await;
        } else if let Action::User(id) = event.action {
            self.on_adjust_user(id).await;
        }
    }

    async fn on_mb1(&mut self, pressed: bool, now: u32) {
        self.mb1_held = pressed;
        if pressed {
            self.mb1_press_time = now;
            if self.mb2_held && now.wrapping_sub(self.mb2_press_time) < COMBO_WINDOW_MS {
                self.start_combo(now).await;
            }
        } else if self.combo_active {
            self.finish_combo(now).await;
        } else if self.adjust {
            let held = now.wrapping_sub(self.mb1_press_time);
            if held < COMBO_TAP_MS {
                send_virtual_key(1, 0).await;
            }
        }
    }

    async fn on_user14(&mut self, pressed: bool, now: u32) {
        self.mb2_held = pressed;
        if pressed {
            self.mb2_press_time = now;
            if self.mb1_held && now.wrapping_sub(self.mb1_press_time) < COMBO_WINDOW_MS {
                self.start_combo(now).await;
            } else if !self.adjust {
                self.set_mode(Mode::Sniper).await;
            }
        } else if self.combo_active {
            self.finish_combo(now).await;
        } else if self.adjust {
            let held = now.wrapping_sub(self.mb2_press_time);
            if held < COMBO_TAP_MS {
                send_virtual_key(1, 1).await;
            }
        } else {
            let held = now.wrapping_sub(self.mb2_press_time);
            self.set_mode(Mode::Cursor).await;
            if held < COMBO_TAP_MS {
                send_mouse_click(0b0000_0010).await;
            }
        }
    }

    async fn on_adjust_user(&mut self, id: u8) {
        match id {
            10 => {
                self.scroll_divisor = self.scroll_divisor.saturating_add(1).min(SCROLL_DIVISOR_MAX);
                self.republish_mode_if(Mode::Scroll).await;
            }
            11 => {
                self.scroll_divisor = self.scroll_divisor.saturating_sub(1).max(SCROLL_DIVISOR_MIN);
                self.republish_mode_if(Mode::Scroll).await;
            }
            12 => {
                self.sniper_divisor = self.sniper_divisor.saturating_add(1).min(SNIPER_DIVISOR_MAX);
                self.republish_mode_if(Mode::Sniper).await;
            }
            13 => {
                self.sniper_divisor = self.sniper_divisor.saturating_sub(1).max(SNIPER_DIVISOR_MIN);
                self.republish_mode_if(Mode::Sniper).await;
            }
            _ => {}
        }
    }

    async fn start_combo(&mut self, now: u32) {
        if self.adjust {
            self.adjust = false;
            self.set_mode(Mode::Cursor).await;
            return;
        }
        self.combo_active = true;
        self.combo_start_time = now;
        self.set_mode(Mode::Scroll).await;
    }

    async fn finish_combo(&mut self, now: u32) {
        let held = now.wrapping_sub(self.combo_start_time);
        self.combo_active = false;
        if held < COMBO_TAP_MS {
            if now.wrapping_sub(self.last_combo_tap_time) < DOUBLE_TAP_MS {
                self.adjust = true;
                self.last_combo_tap_time = 0;
                self.set_mode(Mode::Cursor).await;
            } else {
                send_mouse_click(0b0000_0100).await;
                self.last_combo_tap_time = now;
            }
        } else {
            self.last_combo_tap_time = 0;
        }

        if !self.adjust && self.mb2_held {
            self.set_mode(Mode::Sniper).await;
        } else {
            self.set_mode(Mode::Cursor).await;
        }
    }

    async fn republish_mode_if(&mut self, mode: Mode) {
        if self.mode == mode {
            self.publish_mode().await;
        }
    }

    async fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.publish_mode().await;
    }

    async fn publish_mode(&self) {
        publish_event_async(PointingProcessorEvent {
            device_id: TRACKBALL_DEVICE_ID,
            mode: self.pointing_mode(),
        })
        .await;
    }

    fn pointing_mode(&self) -> PointingMode {
        match self.mode {
            Mode::Cursor => PointingMode::Cursor(CursorConfig::default()),
            Mode::Scroll => PointingMode::Scroll(ScrollConfig {
                divisor_x: self.scroll_divisor,
                divisor_y: self.scroll_divisor,
                ..Default::default()
            }),
            Mode::Sniper => PointingMode::Sniper(SniperConfig {
                divisor: self.sniper_divisor,
                ..Default::default()
            }),
        }
    }
}

fn is_hid(action: Action, hid: HidKeyCode) -> bool {
    matches!(action, Action::Key(KeyCode::Hid(key)) if key == hid)
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

async fn send_virtual_key(row: u8, col: u8) {
    publish_event_async(KeyboardEvent::key(row, col, true)).await;
    Timer::after(Duration::from_millis(50)).await;
    publish_event_async(KeyboardEvent::key(row, col, false)).await;
}

async fn send_mouse_click(buttons: u8) {
    send_hid_report(Report::MouseReport(MouseReport {
        buttons,
        x: 0,
        y: 0,
        wheel: 0,
        pan: 0,
    }))
    .await;
    Timer::after(Duration::from_millis(10)).await;
    send_hid_report(Report::MouseReport(MouseReport {
        buttons: 0,
        x: 0,
        y: 0,
        wheel: 0,
        pan: 0,
    }))
    .await;
}
