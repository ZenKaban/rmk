use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Instant, Timer};
use rmk::event::{
    BatteryStatusEvent, LayerChangeEvent, SleepStateEvent, SplitConnectionState, SplitConnectionStateEvent,
};
use rmk::macros::processor;
use rmk::types::battery::BatteryStatus;

use crate::module_settings::{self, Rgb};

const LED_COUNT: usize = 1;
const SPLIT_BLINK_PERIOD_MS: u64 = 360;
const SPLIT_BLINK_ON_MS: u64 = 180;
const CONNECTED_INDICATOR_MS: u64 = 1_000;
const CHARGED_BATTERY_MIN: u8 = 95;
const PWM_POLARITY_INVERTED: u16 = 0x8000;
const PWM_T0H: u16 = PWM_POLARITY_INVERTED | 6;
const PWM_T1H: u16 = PWM_POLARITY_INVERTED | 13;
const RESET_SLOTS: usize = 80;
const FRAME_WORDS: usize = LED_COUNT * 24 + RESET_SLOTS;

#[processor(
    subscribe = [
        LayerChangeEvent,
        SplitConnectionStateEvent,
        SleepStateEvent,
        BatteryStatusEvent
    ],
    poll_interval = 10
)]
pub struct LayerLed {
    led: SequencePwm<'static>,
    current_layer: Option<u8>,
    current_color: Option<Rgb>,
    split_state: SplitConnectionState,
    sleeping: bool,
    phase_started: Instant,
    connected_until: Option<Instant>,
    latest_battery: Option<u8>,
}

impl LayerLed {
    pub fn new(led: SequencePwm<'static>) -> Self {
        let now = Instant::now();
        Self {
            led,
            current_layer: Some(0),
            current_color: None,
            split_state: SplitConnectionState::Searching,
            sleeping: false,
            phase_started: now,
            connected_until: None,
            latest_battery: None,
        }
    }

    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.current_layer = Some(event.0);
        self.render(Instant::now()).await;
    }

    async fn on_split_connection_state_event(&mut self, event: SplitConnectionStateEvent) {
        if self.split_state != event.0 {
            let now = Instant::now();
            self.split_state = event.0;
            self.phase_started = now;
            self.connected_until = (event.0 == SplitConnectionState::Connected)
                .then(|| now + Duration::from_millis(CONNECTED_INDICATOR_MS));
        }
        self.render(Instant::now()).await;
    }

    async fn on_sleep_state_event(&mut self, event: SleepStateEvent) {
        self.sleeping = event.0;
        self.render(Instant::now()).await;
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        match event.0 {
            BatteryStatus::Available { level, .. } => {
                self.latest_battery = level;
            }
            BatteryStatus::Unavailable => {
                self.latest_battery = None;
            }
        }
        self.render(Instant::now()).await;
    }

    async fn poll(&mut self) {
        self.render(Instant::now()).await;
    }

    async fn render(&mut self, now: Instant) {
        let color = self.display_color(now);
        if self.current_color == Some(color) {
            return;
        }
        self.current_color = Some(color);
        send_color(&mut self.led, color).await;
    }

    fn display_color(&self, now: Instant) -> Rgb {
        if self.current_layer == Some(0)
            && crate::battery_nrf::usb_powered()
            && module_settings::charge_indicator_enabled()
        {
            return if self.latest_battery.is_some_and(|level| level >= CHARGED_BATTERY_MIN) {
                color_green()
            } else {
                color_yellow()
            };
        }

        if self.sleeping || self.split_state == SplitConnectionState::Idle {
            return color_off();
        }

        if self.split_state == SplitConnectionState::Searching {
            let elapsed_ms = now.duration_since(self.phase_started).as_millis();
            return if elapsed_ms % SPLIT_BLINK_PERIOD_MS < SPLIT_BLINK_ON_MS {
                color_yellow()
            } else {
                color_off()
            };
        }

        if self.connected_until.is_some_and(|until| now < until) {
            return color_green();
        }

        self.current_layer.map(color_for_layer).unwrap_or_else(color_off)
    }
}

fn color_for_layer(layer: u8) -> Rgb {
    scale_color(module_settings::layer_color(layer))
}

fn color_yellow() -> Rgb {
    scale_color(Rgb { r: 255, g: 180, b: 0 })
}

fn color_green() -> Rgb {
    scale_color(Rgb { r: 0, g: 255, b: 0 })
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
