// k04-vial-settings-v0.0.61: layer RGB colors/brightness come from synced Vial settings.

use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Instant, Timer};
use rmk::channel::CONTROLLER_CHANNEL;
use rmk::event::ControllerEvent;

use crate::vial_settings::{
    apply_settings_sync_packet, bt_profile_color, layer_color, led_brightness, led_timeout_sec, Rgb,
};

const LED_COUNT: usize = 1;
const LOW_BATTERY_MAX: u8 = 20;
const LOW_BATTERY_BLINK_INTERVAL_MS: u64 = 2_000;
const LOW_BATTERY_BLINK_ON_MS: u64 = 120;
const SYSTEM_OFF_PURPLE_MS: u64 = 120;
const SPLIT_CONNECTED_PULSE_MS: u64 = 520;
const BLE_LED_ADVERTISING: u8 = 0;
const BLE_LED_RECONNECTING: u8 = 1;
const BLE_LED_PAIRING: u8 = 2;
const BLE_LED_CONNECTED: u8 = 3;
const BLE_LED_NONE: u8 = 4;

// 16 MHz PWM clock, COUNTERTOP=20 => 1.25 us WS2812 bit period.
const PWM_TOP: u16 = 20;
const PWM_POLARITY_INVERTED: u16 = 0x8000;
const PWM_T0H: u16 = PWM_POLARITY_INVERTED | 6; // ~0.375 us high
const PWM_T1H: u16 = PWM_POLARITY_INVERTED | 13; // ~0.812 us high
const RESET_SLOTS: usize = 80; // 80 * 1.25 us = 100 us low reset gap
const FRAME_WORDS: usize = LED_COUNT * 24 + RESET_SLOTS;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionLedMode {
    None,
    Advertising,
    Reconnecting,
    Pairing,
    HostConnectedPulse,
    SplitMissing,
    SplitUnpaired,
    SplitConnectedPulse,
    SystemOff,
}

#[embassy_executor::task]
pub async fn layer_led_task(mut led: SequencePwm<'static>) {
    let mut sub = defmt::unwrap!(CONTROLLER_CHANNEL.subscriber());
    let mut current_layer: Option<u8> = None;
    let mut displayed_layer: Option<u8> = None;
    let mut latest_battery: Option<u8> = None;
    let mut pending_battery_display = false;
    let mut last_change = Instant::now();
    let mut last_low_battery_blink = Instant::now();
    let mut low_battery_blinking = false;
    let mut low_battery_blink_started = Instant::now();
    let mut connection_mode = ConnectionLedMode::None;
    let mut connection_mode_started = Instant::now();
    let mut last_connection_color: Option<Rgb> = None;
    let mut split_connected = true;
    let mut split_unpaired = false;

    // Send a few off frames on boot to clear any latched color from bootloader
    // or previous firmware. PWM timing is hardware-generated, not code-layout dependent.
    for _ in 0..3 {
        send_color(&mut led, Rgb { r: 0, g: 0, b: 0 }).await;
    }

    loop {
        if let Some(event) = sub.try_next_message_pure() {
            if let Some(state) = event.ble_led_state_code() {
                match state {
                    BLE_LED_ADVERTISING => {
                        let mode = if split_unpaired {
                            ConnectionLedMode::SplitUnpaired
                        } else if !split_connected {
                            ConnectionLedMode::SplitMissing
                        } else {
                            ConnectionLedMode::Advertising
                        };
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            mode,
                        );
                    }
                    BLE_LED_RECONNECTING => {
                        let mode = if split_unpaired {
                            ConnectionLedMode::SplitUnpaired
                        } else if !split_connected {
                            ConnectionLedMode::SplitMissing
                        } else {
                            ConnectionLedMode::Reconnecting
                        };
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            mode,
                        );
                    }
                    BLE_LED_PAIRING => {
                        let mode = if split_unpaired {
                            ConnectionLedMode::SplitUnpaired
                        } else if !split_connected {
                            ConnectionLedMode::SplitMissing
                        } else {
                            ConnectionLedMode::Pairing
                        };
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            mode,
                        );
                    }
                    BLE_LED_CONNECTED => {
                        let mode = if split_connected {
                            ConnectionLedMode::HostConnectedPulse
                        } else if split_unpaired {
                            ConnectionLedMode::SplitUnpaired
                        } else {
                            ConnectionLedMode::SplitMissing
                        };
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            mode,
                        );
                    }
                    BLE_LED_NONE => {
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            ConnectionLedMode::None,
                        );
                        restore_display(&mut led, displayed_layer, latest_battery).await;
                    }
                    _ => {}
                }
                continue;
            }

            match event {
                ControllerEvent::Key(_, _) => continue,
                ControllerEvent::Layer(layer) => {
                    let layer_changed = current_layer != Some(layer);
                    current_layer = Some(layer);

                    if matches!(displayed_layer, Some(current) if is_temporary(current)) {
                        // Keep BT profile indication visible, but still let the timeout below run.
                    } else if layer_changed {
                        send_color(&mut led, color_for_layer(layer)).await;
                        displayed_layer = Some(layer);
                        last_change = Instant::now();
                        continue;
                    }
                }
                ControllerEvent::BleProfile(profile) => {
                    send_color(&mut led, color_for_bt_profile(profile)).await;
                    displayed_layer = Some(temporary_bt_profile(profile));
                    last_change = Instant::now();
                    continue;
                }
                ControllerEvent::Battery(level) => {
                    latest_battery = Some(level);
                    if level > LOW_BATTERY_MAX {
                        low_battery_blinking = false;
                    }
                    if pending_battery_display {
                        pending_battery_display = false;
                        send_color(&mut led, color_for_battery(level)).await;
                        displayed_layer = Some(TEMPORARY_BATTERY);
                        last_change = Instant::now();
                    }
                    continue;
                }
                ControllerEvent::BatteryLevelRequest => {
                    if let Some(level) = latest_battery {
                        send_color(&mut led, color_for_battery(level)).await;
                        displayed_layer = Some(TEMPORARY_BATTERY);
                        last_change = Instant::now();
                    } else {
                        pending_battery_display = true;
                    }
                    continue;
                }
                ControllerEvent::DeviceSettings(settings) => {
                    apply_settings_sync_packet(&settings);
                    last_connection_color = None;
                    if let Some(layer) = displayed_layer {
                        if is_temporary_bt(layer) {
                            send_color(&mut led, color_for_bt_profile(layer & 0x7f)).await;
                        } else if is_temporary_battery(layer) {
                            if let Some(level) = latest_battery {
                                send_color(&mut led, color_for_battery(level)).await;
                            }
                        } else {
                            send_color(&mut led, color_for_layer(layer)).await;
                        }
                    }
                    continue;
                }
                ControllerEvent::SplitPeripheral(_, connected)
                | ControllerEvent::SplitCentral(connected) => {
                    split_connected = connected;
                    split_unpaired = false;
                    if !connected {
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            ConnectionLedMode::SplitMissing,
                        );
                    } else {
                        set_connection_mode(
                            &mut connection_mode,
                            &mut connection_mode_started,
                            &mut last_connection_color,
                            ConnectionLedMode::SplitConnectedPulse,
                        );
                    }
                    continue;
                }
                ControllerEvent::SplitPeripheralUnpaired(_)
                | ControllerEvent::SplitCentralUnpaired => {
                    split_connected = false;
                    split_unpaired = true;
                    set_connection_mode(
                        &mut connection_mode,
                        &mut connection_mode_started,
                        &mut last_connection_color,
                        ConnectionLedMode::SplitUnpaired,
                    );
                    continue;
                }
                ControllerEvent::SystemOff => {
                    set_connection_mode(
                        &mut connection_mode,
                        &mut connection_mode_started,
                        &mut last_connection_color,
                        ConnectionLedMode::SystemOff,
                    );
                    continue;
                }
                _ => {}
            }
        }

        if let Some(color) = connection_led_color(connection_mode, connection_mode_started) {
            if last_connection_color != Some(color) {
                send_color(&mut led, color).await;
                last_connection_color = Some(color);
            }
            Timer::after(Duration::from_millis(20)).await;
            continue;
        } else if last_connection_color.is_some() {
            restore_display(&mut led, displayed_layer, latest_battery).await;
            last_connection_color = None;
            if matches!(
                connection_mode,
                ConnectionLedMode::HostConnectedPulse | ConnectionLedMode::SplitConnectedPulse
            ) {
                connection_mode = ConnectionLedMode::None;
            }
        }

        let timeout_ms = u64::from(led_timeout_sec()) * 1_000;
        if timeout_ms != 0
            && displayed_layer.is_some()
            && Instant::now().duration_since(last_change).as_millis() >= timeout_ms
        {
            send_color(&mut led, Rgb { r: 0, g: 0, b: 0 }).await;
            displayed_layer = None;
        }

        if latest_battery.is_some_and(|level| level <= LOW_BATTERY_MAX) {
            let now = Instant::now();
            if low_battery_blinking {
                if now.duration_since(low_battery_blink_started).as_millis()
                    >= LOW_BATTERY_BLINK_ON_MS
                {
                    restore_display(&mut led, displayed_layer, latest_battery).await;
                    low_battery_blinking = false;
                    last_low_battery_blink = now;
                }
            } else if now.duration_since(last_low_battery_blink).as_millis()
                >= LOW_BATTERY_BLINK_INTERVAL_MS
            {
                send_color(&mut led, color_for_low_battery_blink()).await;
                low_battery_blinking = true;
                low_battery_blink_started = now;
            }
        }

        Timer::after(Duration::from_millis(20)).await;
    }
}

fn set_connection_mode(
    mode: &mut ConnectionLedMode,
    mode_started: &mut Instant,
    last_color: &mut Option<Rgb>,
    next: ConnectionLedMode,
) {
    if *mode != next {
        *mode = next;
        *mode_started = Instant::now();
        *last_color = None;
    }
}

fn connection_led_color(mode: ConnectionLedMode, started: Instant) -> Option<Rgb> {
    let elapsed_ms = Instant::now().duration_since(started).as_millis() as u64;
    let color = match mode {
        ConnectionLedMode::None => return None,
        ConnectionLedMode::Advertising => blink_color(color_blue(), elapsed_ms, 1_000, 500),
        ConnectionLedMode::Reconnecting => blink_color(color_blue(), elapsed_ms, 250, 125),
        ConnectionLedMode::Pairing => pairing_blink_color(elapsed_ms),
        ConnectionLedMode::HostConnectedPulse => return host_connected_pulse_color(elapsed_ms),
        ConnectionLedMode::SplitMissing => blink_color(color_yellow(), elapsed_ms, 1_500, 150),
        ConnectionLedMode::SplitUnpaired => blink_color(color_orange(), elapsed_ms, 900, 180),
        ConnectionLedMode::SplitConnectedPulse => return split_connected_pulse_color(elapsed_ms),
        ConnectionLedMode::SystemOff => {
            if elapsed_ms < SYSTEM_OFF_PURPLE_MS {
                color_purple()
            } else {
                color_off()
            }
        }
    };
    Some(color)
}

fn host_connected_pulse_color(elapsed_ms: u64) -> Option<Rgb> {
    if elapsed_ms >= SPLIT_CONNECTED_PULSE_MS {
        return None;
    }
    Some(match elapsed_ms {
        0..=100 | 180..=280 => color_green(),
        _ => color_off(),
    })
}

fn split_connected_pulse_color(elapsed_ms: u64) -> Option<Rgb> {
    if elapsed_ms >= SPLIT_CONNECTED_PULSE_MS {
        return None;
    }
    Some(match elapsed_ms {
        0..=100 | 180..=280 => color_yellow(),
        _ => color_off(),
    })
}

fn blink_color(color: Rgb, elapsed_ms: u64, period_ms: u64, on_ms: u64) -> Rgb {
    if elapsed_ms % period_ms < on_ms {
        color
    } else {
        color_off()
    }
}

fn pairing_blink_color(elapsed_ms: u64) -> Rgb {
    match elapsed_ms % 1_200 {
        0..=120 | 240..=360 => color_cyan(),
        _ => color_off(),
    }
}

fn color_blue() -> Rgb {
    scale_color(Rgb { r: 0, g: 0, b: 255 })
}

fn color_green() -> Rgb {
    scale_color(Rgb { r: 0, g: 255, b: 0 })
}

fn color_cyan() -> Rgb {
    scale_color(Rgb {
        r: 0,
        g: 180,
        b: 255,
    })
}

fn color_yellow() -> Rgb {
    scale_color(Rgb {
        r: 255,
        g: 180,
        b: 0,
    })
}

fn color_orange() -> Rgb {
    scale_color(Rgb {
        r: 255,
        g: 70,
        b: 0,
    })
}

fn color_purple() -> Rgb {
    scale_color(Rgb {
        r: 160,
        g: 0,
        b: 255,
    })
}

fn color_off() -> Rgb {
    Rgb { r: 0, g: 0, b: 0 }
}

fn temporary_bt_profile(profile: u8) -> u8 {
    0x80 | profile.min(4)
}

const TEMPORARY_BATTERY: u8 = 0x90;

fn is_temporary(value: u8) -> bool {
    is_temporary_bt(value) || is_temporary_battery(value)
}

fn is_temporary_bt(value: u8) -> bool {
    (0x80..=0x84).contains(&value)
}

fn is_temporary_battery(value: u8) -> bool {
    value == TEMPORARY_BATTERY
}

fn color_for_layer(layer: u8) -> Rgb {
    let color = layer_color(layer);
    scale_color(color)
}

fn color_for_bt_profile(profile: u8) -> Rgb {
    let color = bt_profile_color(profile);
    scale_color(color)
}

fn color_for_battery(level: u8) -> Rgb {
    let color = match level {
        0..=20 => Rgb { r: 255, g: 0, b: 0 },
        21..=40 => Rgb {
            r: 255,
            g: 80,
            b: 0,
        },
        41..=74 => Rgb {
            r: 255,
            g: 220,
            b: 0,
        },
        _ => Rgb { r: 0, g: 255, b: 0 },
    };
    scale_color(color)
}

fn color_for_low_battery_blink() -> Rgb {
    scale_color(Rgb { r: 255, g: 0, b: 0 })
}

async fn restore_display(
    led: &mut SequencePwm<'static>,
    displayed_layer: Option<u8>,
    latest_battery: Option<u8>,
) {
    let color = displayed_color(displayed_layer, latest_battery);
    send_color(led, color).await;
}

fn displayed_color(displayed_layer: Option<u8>, latest_battery: Option<u8>) -> Rgb {
    match displayed_layer {
        Some(layer) if is_temporary_bt(layer) => color_for_bt_profile(layer & 0x7f),
        Some(layer) if is_temporary_battery(layer) => latest_battery
            .map(color_for_battery)
            .unwrap_or(Rgb { r: 0, g: 0, b: 0 }),
        Some(layer) => color_for_layer(layer),
        None => Rgb { r: 0, g: 0, b: 0 },
    }
}

fn scale_color(color: Rgb) -> Rgb {
    let brightness = u16::from(led_brightness());
    Rgb {
        r: scale(color.r, brightness),
        g: scale(color.g, brightness),
        b: scale(color.b, brightness),
    }
}

fn scale(value: u8, brightness: u16) -> u8 {
    ((u16::from(value) * brightness) / 255).min(255) as u8
}

async fn send_color(led: &mut SequencePwm<'static>, color: Rgb) {
    let mut words = [0u16; FRAME_WORDS];
    let mut i = 0usize;

    // K04 module LED uses GRB byte order.
    for byte in [color.g, color.r, color.b] {
        for bit in (0..8).rev() {
            words[i] = if (byte & (1 << bit)) != 0 {
                PWM_T1H
            } else {
                PWM_T0H
            };
            i += 1;
        }
    }
    // Remaining RESET_SLOTS words stay 0 => low reset/latch gap.

    {
        let sequencer = SingleSequencer::new(led, &words, SequenceConfig::default());
        let _ = sequencer.start(SingleSequenceMode::Times(1));
        // Wait longer than the frame duration before reusing the stack buffer.
        Timer::after(Duration::from_micros(200)).await;
        sequencer.stop();
    }
}
