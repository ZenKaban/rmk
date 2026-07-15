use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Timer};
use rmk::event::LayerChangeEvent;
use rmk::macros::processor;

use crate::module_settings::{self, Rgb};

const LED_COUNT: usize = 1;
const PWM_POLARITY_INVERTED: u16 = 0x8000;
const PWM_T0H: u16 = PWM_POLARITY_INVERTED | 6;
const PWM_T1H: u16 = PWM_POLARITY_INVERTED | 13;
const RESET_SLOTS: usize = 80;
const FRAME_WORDS: usize = LED_COUNT * 24 + RESET_SLOTS;

#[processor(subscribe = [LayerChangeEvent])]
pub struct LayerLed {
    led: SequencePwm<'static>,
    current_layer: Option<u8>,
}

impl LayerLed {
    pub fn new(led: SequencePwm<'static>) -> Self {
        Self {
            led,
            current_layer: None,
        }
    }

    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        let layer = event.0;
        if self.current_layer == Some(layer) {
            return;
        }

        self.current_layer = Some(layer);
        send_color(&mut self.led, color_for_layer(layer)).await;
    }
}

fn color_for_layer(layer: u8) -> Rgb {
    scale_color(module_settings::layer_color(layer))
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
