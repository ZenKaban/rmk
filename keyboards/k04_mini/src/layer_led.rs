use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Timer};
use rmk::event::{
    CentralConnectedEvent, ConnectionStatusChangeEvent, LayerChangeEvent, PeripheralConnectedEvent,
    PeripheralSettingsEvent,
};
use rmk::macros::processor;
use rmk::types::ble::BleState;

use crate::module_settings::{self, Rgb};

const LED_COUNT: usize = 1;
const PWM_POLARITY_INVERTED: u16 = 0x8000;
const PWM_T0H: u16 = PWM_POLARITY_INVERTED | 6;
const PWM_T1H: u16 = PWM_POLARITY_INVERTED | 13;
const RESET_SLOTS: usize = 80;
const FRAME_WORDS: usize = LED_COUNT * 24 + RESET_SLOTS;

#[processor(subscribe = [LayerChangeEvent, ConnectionStatusChangeEvent, PeripheralConnectedEvent, CentralConnectedEvent, PeripheralSettingsEvent])]
pub struct LayerLed {
    led: SequencePwm<'static>,
    current_layer: Option<u8>,
    current_color: Option<Rgb>,
    ble_profile: u8,
    ble_state: BleState,
    split_connected: bool,
}

impl LayerLed {
    pub fn new(led: SequencePwm<'static>) -> Self {
        Self {
            led,
            current_layer: None,
            current_color: None,
            ble_profile: 0,
            ble_state: BleState::Inactive,
            split_connected: true,
        }
    }

    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        let layer = event.0;
        if self.current_layer == Some(layer) {
            return;
        }

        self.current_layer = Some(layer);
        self.render().await;
    }

    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        self.ble_profile = event.0.ble.profile;
        self.ble_state = event.0.ble.state;
        self.render().await;
    }

    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        self.split_connected = event.connected;
        self.render().await;
    }

    async fn on_central_connected_event(&mut self, event: CentralConnectedEvent) {
        self.split_connected = event.connected;
        self.render().await;
    }

    async fn on_peripheral_settings_event(&mut self, _event: PeripheralSettingsEvent) {
        self.current_color = None;
        self.render().await;
    }

    async fn render(&mut self) {
        let color = if !self.split_connected {
            color_split_missing()
        } else {
            match self.ble_state {
                BleState::Advertising => color_for_bt_profile(self.ble_profile),
                BleState::Connected | BleState::Inactive => {
                    self.current_layer.map(color_for_layer).unwrap_or_else(color_off)
                }
            }
        };

        if self.current_color == Some(color) {
            return;
        }
        self.current_color = Some(color);
        send_color(&mut self.led, color).await;
    }
}

fn color_for_layer(layer: u8) -> Rgb {
    scale_color(module_settings::layer_color(layer))
}

fn color_for_bt_profile(profile: u8) -> Rgb {
    let color = match profile {
        0 => Rgb { r: 255, g: 0, b: 0 },
        1 => Rgb { r: 0, g: 0, b: 255 },
        2 => Rgb { r: 255, g: 255, b: 0 },
        3 => Rgb { r: 0, g: 255, b: 0 },
        4 => Rgb { r: 255, g: 0, b: 255 },
        _ => Rgb { r: 255, g: 255, b: 255 },
    };
    scale_color(color)
}

fn color_split_missing() -> Rgb {
    scale_color(Rgb { r: 255, g: 128, b: 0 })
}

fn color_off() -> Rgb {
    Rgb { r: 0, g: 0, b: 0 }
}

fn scale_color(color: Rgb) -> Rgb {
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

fn scale(value: u8) -> u8 {
    ((u16::from(value) * u16::from(module_settings::led_brightness())) / 255).min(255) as u8
}

async fn send_color(led: &mut SequencePwm<'static>, color: Rgb) {
    let mut words = [0u16; FRAME_WORDS];
    let mut i = 0usize;

    for byte in [color.g, color.r, color.b] {
        for bit in (0..8).rev() {
            words[i] = if (byte & (1 << bit)) != 0 { PWM_T1H } else { PWM_T0H };
            i += 1;
        }
    }

    let sequencer = SingleSequencer::new(led, &words, SequenceConfig::default());
    let _ = sequencer.start(SingleSequenceMode::Times(1));
    Timer::after(Duration::from_micros(200)).await;
    sequencer.stop();
}
