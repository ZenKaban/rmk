//! Trackball Royale layer/mode bridge for root RMK.

use rmk::event::{publish_event_async, ActionEvent, LayerChangeEvent, PointingProcessorEvent};
use rmk::input_device::pointing::{CursorConfig, PointingMode, ScrollConfig, SniperConfig};
use rmk::macros::processor;
use rmk::types::action::Action;

const TRACKBALL_DEVICE_ID: u8 = 0;
const LAYER_SCROLL: u8 = 1;
const LAYER_SNIPER: u8 = 2;
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

#[processor(subscribe = [ActionEvent, LayerChangeEvent])]
pub struct TrackballModeProcessor {
    mode: Mode,
    scroll_divisor: u8,
    sniper_divisor: u8,
}

impl TrackballModeProcessor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Cursor,
            scroll_divisor: SCROLL_DIVISOR_DEFAULT,
            sniper_divisor: SNIPER_DIVISOR_DEFAULT,
        }
    }

    async fn on_layer_change_event(&mut self, LayerChangeEvent(layer): LayerChangeEvent) {
        let mode = match layer {
            LAYER_SCROLL => Mode::Scroll,
            LAYER_SNIPER => Mode::Sniper,
            _ => Mode::Cursor,
        };
        self.set_mode(mode).await;
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        let Action::User(id) = event.action else {
            return;
        };
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
