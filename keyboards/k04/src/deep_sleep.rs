use embassy_time::Timer;
use rmk::embassy_futures::select::{select3, Either3};

const ROW_WAKE_PINS: &[(u8, usize)] = &[(1, 13), (0, 28), (0, 3), (1, 10), (1, 11)];
const COL_OUTPUT_PINS: &[(u8, usize)] = &[(1, 6), (1, 4), (1, 2), (1, 0), (0, 22), (0, 20)];
const WAKE_SETTLE_MS: u64 = 5;
const ACTIVE_WAKE_RETRY_MS: u64 = 500;
const SYSTEM_OFF_LED_SETTLE_MS: u64 = 300;

#[embassy_executor::task]
pub async fn deep_sleep_task() {
    loop {
        let timeout = crate::vial_settings::sleep_timeout_secs();
        match select3(
            Timer::after_secs(timeout),
            rmk::channel::ACTIVITY_SIGNAL.wait(),
            crate::vial_settings::wait_settings_change(),
        )
        .await
        {
            Either3::First(_) => {
                if prepare_matrix_wake().await {
                    rmk::channel::send_controller_event_new(rmk::event::ControllerEvent::SystemOff);
                    Timer::after_millis(SYSTEM_OFF_LED_SETTLE_MS).await;
                    enter_system_off();
                } else {
                    // A held key or noisy wake line would instantly wake SYSTEM OFF.
                    // Treat it as activity and retry after the line settles.
                    rmk::channel::signal_activity();
                    Timer::after_millis(ACTIVE_WAKE_RETRY_MS).await;
                }
            }
            Either3::Second(_) | Either3::Third(_) => {}
        }
    }
}

async fn prepare_matrix_wake() -> bool {
    clear_gpio_latches();
    configure_matrix_wake();
    Timer::after_millis(WAKE_SETTLE_MS).await;

    if any_wake_input_active() {
        return false;
    }

    clear_gpio_latches();
    true
}

fn enter_system_off() -> ! {
    embassy_nrf::power::set_system_off();
    loop {
        cortex_m::asm::wfi();
    }
}

fn configure_matrix_wake() {
    for &(port, pin) in COL_OUTPUT_PINS {
        configure_output_high(port, pin);
    }
    for &(port, pin) in ROW_WAKE_PINS {
        configure_input_pulldown_sense_high(port, pin);
    }
}

fn configure_output_high(port: u8, pin: usize) {
    use embassy_nrf::pac::gpio::vals::{Dir, Drive, Input, Pull, Sense};

    gpio_port(port).outset().write(|w| w.set_pin(pin, true));
    gpio_port(port).pin_cnf(pin).write(|w| {
        w.set_dir(Dir::OUTPUT);
        w.set_input(Input::DISCONNECT);
        w.set_pull(Pull::DISABLED);
        w.set_drive(Drive::S0S1);
        w.set_sense(Sense::DISABLED);
    });
}

fn configure_input_pulldown_sense_high(port: u8, pin: usize) {
    use embassy_nrf::pac::gpio::vals::{Dir, Drive, Input, Pull, Sense};

    gpio_port(port).pin_cnf(pin).write(|w| {
        w.set_dir(Dir::INPUT);
        w.set_input(Input::CONNECT);
        w.set_pull(Pull::PULLDOWN);
        w.set_drive(Drive::S0S1);
        w.set_sense(Sense::HIGH);
    });
}

fn any_wake_input_active() -> bool {
    ROW_WAKE_PINS
        .iter()
        .any(|&(port, pin)| gpio_port(port).in_().read().pin(pin))
}

fn clear_gpio_latches() {
    embassy_nrf::pac::P0.latch().write(|w| w.0 = 0xffff_ffff);
    embassy_nrf::pac::P1.latch().write(|w| w.0 = 0xffff_ffff);
}

fn gpio_port(port: u8) -> embassy_nrf::pac::gpio::Gpio {
    match port {
        0 => embassy_nrf::pac::P0,
        1 => embassy_nrf::pac::P1,
        _ => unreachable!(),
    }
}
