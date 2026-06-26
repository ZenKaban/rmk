#![no_main]
#![no_std]

mod deep_sleep;
mod layer_led;
mod touchpad;
mod trackball;
mod vial_settings;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    add_interrupt!(
        TWISPI1 => ::embassy_nrf::twim::InterruptHandler<::embassy_nrf::peripherals::TWISPI1>;
    );

    use crate::deep_sleep::deep_sleep_task;
    use crate::layer_led::layer_led_task;
    use crate::touchpad::{
        auto_layer_idle_loop, touchpad_task, Iqs5xxTouchpad, K04PointingProcessor,
    };
    use crate::trackball::{new_trackball_from_pins, trackball_task};
    use crate::vial_settings::{encoder_interval_ms, encoder_module_enabled, ModuleSide};

    #[overwritten(entry)]
    fn custom_entry() {
        use rmk::input_device::{InputDevice, Runnable};

        let mut layer_led_config = ::embassy_nrf::pwm::Config::default();
        layer_led_config.prescaler = ::embassy_nrf::pwm::Prescaler::Div1;
        layer_led_config.max_duty = 20;
        layer_led_config.sequence_load = ::embassy_nrf::pwm::SequenceLoad::Common;
        let layer_led =
            ::embassy_nrf::pwm::SequencePwm::new_1ch(p.PWM0, p.P0_30, layer_led_config).unwrap();
        spawner.spawn(layer_led_task(layer_led)).unwrap();
        spawner.spawn(deep_sleep_task()).unwrap();
        spawner.spawn(settings_sync_task()).unwrap();

        let trackball = new_trackball_from_pins(p.P0_01, p.P0_00, p.P0_05, p.P1_09);
        spawner
            .spawn(trackball_task(trackball, ModuleSide::Left))
            .unwrap();

        static TOUCH_TWIM_TX_BUF: ::static_cell::StaticCell<[u8; 4]> =
            ::static_cell::StaticCell::new();
        let mut touch_i2c_config = ::embassy_nrf::twim::Config::default();
        touch_i2c_config.frequency = ::embassy_nrf::twim::Frequency::K400;
        touch_i2c_config.sda_pullup = true;
        touch_i2c_config.scl_pullup = true;
        let touch_i2c = ::embassy_nrf::twim::Twim::new(
            p.TWISPI1,
            Irqs,
            p.P0_24,
            p.P0_13,
            touch_i2c_config,
            TOUCH_TWIM_TX_BUF.init([0; 4]),
        );
        let touchpad = Iqs5xxTouchpad::new(ModuleSide::Left, touch_i2c);
        spawner.spawn(touchpad_task(touchpad)).unwrap();

        let mut pointing_processor = K04PointingProcessor::new(&keymap);
        let auto_layer_task = auto_layer_idle_loop(&keymap);

        let encoder_task = async {
            loop {
                if !encoder_module_enabled(ModuleSide::Left) {
                    ::embassy_time::Timer::after_millis(250).await;
                    continue;
                }
                // k04-v0.0.18: keep real swapped A/B pins, but do not let a noisy encoder
                // flood KEY_EVENT_CHANNEL and starve matrix/layer processing.
                let event = encoder_0.read_event().await;
                if let ::rmk::event::Event::Key(key_event) = event {
                    if encoder_module_enabled(ModuleSide::Left) {
                        ::rmk::channel::KEY_EVENT_CHANNEL.send(key_event).await;
                    }
                    ::embassy_time::Timer::after_millis(encoder_interval_ms(ModuleSide::Left))
                        .await;
                }
            }
        };

        ::rmk::embassy_futures::join::join(
            ::rmk::run_devices!((adc_device, matrix) => ::rmk::channel::EVENT_CHANNEL),
            ::rmk::embassy_futures::join::join(
                encoder_task,
                ::rmk::embassy_futures::join::join(
                    auto_layer_task,
                    ::rmk::embassy_futures::join::join(
                        keyboard.run(),
                        ::rmk::embassy_futures::join::join(
                            ::rmk::run_processor_chain!(
                                ::rmk::channel::EVENT_CHANNEL => [battery_processor, pointing_processor],
                            ),
                            ::rmk::embassy_futures::join::join(
                                ::rmk::run_rmk(&keymap, driver, &stack, &mut storage, rmk_config),
                                ::rmk::embassy_futures::join::join(
                                    ::rmk::split::central::run_peripheral_manager::<5, 6, 5, 0, _>(
                                        0,
                                        &peripheral_addrs,
                                        &stack,
                                    ),
                                    ::rmk::split::ble::central::scan_peripherals(
                                        &stack,
                                        &peripheral_addrs,
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}

#[embassy_executor::task]
async fn settings_sync_task() {
    loop {
        vial_settings::publish_settings_snapshot();
        embassy_time::Timer::after_secs(3).await;
    }
}
