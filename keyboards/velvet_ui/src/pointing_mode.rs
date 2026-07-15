//! Velvet UI pointing mode bridge for the root RMK event model.

use rmk::event::{publish_event_async, ActionEvent, LayerChangeEvent, PointingProcessorEvent};
use rmk::input_device::pointing::{CaretConfig, CursorConfig, PointingMode, ScrollConfig, SniperConfig};
use rmk::macros::processor;
use rmk::types::action::Action;
use rmk::types::keycode::HidKeyCode;

const TRACKBALL_DEVICE_ID: u8 = 0;
const LAYER_SCROLL: u8 = 5;
const LAYER_SNIPER: u8 = 6;
const USER_SNIPER: u8 = 10;
const USER_SCROLL: u8 = 11;
const USER_TEXT: u8 = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cursor,
    Sniper,
    Scroll,
    Text,
}

#[processor(subscribe = [ActionEvent, LayerChangeEvent])]
pub struct VelvetUiPointingMode {
    layer_mode: Mode,
    key_mode: Option<Mode>,
    published: Mode,
}

impl VelvetUiPointingMode {
    pub fn new() -> Self {
        Self {
            layer_mode: Mode::Cursor,
            key_mode: None,
            published: Mode::Cursor,
        }
    }

    async fn on_layer_change_event(&mut self, LayerChangeEvent(layer): LayerChangeEvent) {
        self.layer_mode = match layer {
            LAYER_SCROLL => Mode::Scroll,
            LAYER_SNIPER => Mode::Sniper,
            _ => Mode::Cursor,
        };
        self.publish_current(false).await;
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        let Action::User(id) = event.action else {
            return;
        };
        let mode = match id {
            USER_SNIPER => Some(Mode::Sniper),
            USER_SCROLL => Some(Mode::Scroll),
            USER_TEXT => Some(Mode::Text),
            _ => None,
        };
        if let Some(mode) = mode {
            self.key_mode = event.keyboard_event.pressed.then_some(mode);
            self.publish_current(false).await;
        }
    }

    async fn publish_current(&mut self, force: bool) {
        let mode = self.key_mode.unwrap_or(self.layer_mode);
        if !force && self.published == mode {
            return;
        }
        self.published = mode;
        publish_event_async(PointingProcessorEvent {
            device_id: TRACKBALL_DEVICE_ID,
            mode: pointing_mode(mode),
        })
        .await;
    }
}

fn pointing_mode(mode: Mode) -> PointingMode {
    match mode {
        Mode::Cursor => PointingMode::Cursor(CursorConfig::default()),
        Mode::Sniper => PointingMode::Sniper(SniperConfig {
            divisor: 4,
            ..Default::default()
        }),
        Mode::Scroll => PointingMode::Scroll(ScrollConfig {
            divisor_x: 8,
            divisor_y: 8,
            ..Default::default()
        }),
        Mode::Text => PointingMode::Caret(CaretConfig {
            threshold: 16,
            keycode_up: HidKeyCode::Up,
            keycode_down: HidKeyCode::Down,
            keycode_left: HidKeyCode::Left,
            keycode_right: HidKeyCode::Right,
            ..Default::default()
        }),
    }
}
