#![no_main]
#![no_std]

mod deep_sleep;
mod layer_led;
mod touchpad;
mod trackball;
mod vial_settings;

use rmk::macros::rmk_peripheral;

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    add_interrupt!(
        TWISPI1 => ::embassy_nrf::twim::InterruptHandler<::embassy_nrf::peripherals::TWISPI1>;
    );

    use crate::deep_sleep::deep_sleep_task;
    use crate::layer_led::layer_led_task;
    use crate::touchpad::{touchpad_task, Iqs5xxTouchpad};
    use crate::trackball::{new_trackball_from_pins, trackball_task};
    use crate::vial_settings::ModuleSide;

    #[overwritten(chip_init)]
    fn custom_chip_init() {
        use embassy_nrf::interrupt::InterruptExt;

        let mut config = ::embassy_nrf::config::Config::default();
        config.dcdc.reg0_voltage = Some(::embassy_nrf::config::Reg0Voltage::_3V3);
        // k04-v0.0.14: rollback power/DCDC experiment; this was the last known stable LED state.
        config.dcdc.reg0 = false;
        config.dcdc.reg1 = false;
        ::defmt::info!(
            "DCDC config: reg0_voltage={}, reg0={}, reg1={}",
            "3V3",
            false,
            false
        );

        let p = ::embassy_nrf::init(config);

        let mut layer_led_config = ::embassy_nrf::pwm::Config::default();
        layer_led_config.prescaler = ::embassy_nrf::pwm::Prescaler::Div1;
        layer_led_config.max_duty = 20;
        layer_led_config.sequence_load = ::embassy_nrf::pwm::SequenceLoad::Common;
        let layer_led =
            ::embassy_nrf::pwm::SequencePwm::new_1ch(p.PWM0, p.P0_30, layer_led_config).unwrap();
        spawner.spawn(layer_led_task(layer_led)).unwrap();
        spawner.spawn(deep_sleep_task()).unwrap();

        let trackball = new_trackball_from_pins(p.P0_01, p.P0_00, p.P0_05, p.P1_09);
        spawner
            .spawn(trackball_task(trackball, ModuleSide::Right))
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
        let touchpad = Iqs5xxTouchpad::new(ModuleSide::Right, touch_i2c);
        spawner.spawn(touchpad_task(touchpad)).unwrap();

        let mpsl_p = ::nrf_sdc::mpsl::Peripherals::new(
            p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31,
        );
        let lfclk_cfg = ::nrf_sdc::mpsl::raw::mpsl_clock_lfclk_cfg_t {
            source: ::nrf_sdc::mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
            rc_ctiv: ::nrf_sdc::mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
            rc_temp_ctiv: ::nrf_sdc::mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
            accuracy_ppm: ::nrf_sdc::mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
            skip_wait_lfclk_started: ::nrf_sdc::mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED
                != 0,
        };

        static MPSL: ::static_cell::StaticCell<::nrf_sdc::mpsl::MultiprotocolServiceLayer> =
            ::static_cell::StaticCell::new();
        static SESSION_MEM: ::static_cell::StaticCell<::nrf_sdc::mpsl::SessionMem<1>> =
            ::static_cell::StaticCell::new();
        let mpsl = MPSL.init(::defmt::unwrap!(
            ::nrf_sdc::mpsl::MultiprotocolServiceLayer::with_timeslots(
                mpsl_p,
                Irqs,
                lfclk_cfg,
                SESSION_MEM.init(::nrf_sdc::mpsl::SessionMem::new()),
            )
        ));
        spawner.must_spawn(mpsl_task(&*mpsl));

        let sdc_p = ::nrf_sdc::Peripherals::new(
            p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
            p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
        );
        let mut rng = ::embassy_nrf::rng::Rng::new(p.RNG, Irqs);
        use rand_core::SeedableRng;
        let mut rng_gen = ::rand_chacha::ChaCha12Rng::from_rng(&mut rng).unwrap();
        let mut sdc_mem = ::nrf_sdc::Mem::<6144>::new();
        let sdc = ::defmt::unwrap!(build_sdc(sdc_p, &mut rng, &*mpsl, &mut sdc_mem));

        let ble_addr = {
            let ficr = ::embassy_nrf::pac::FICR;
            let high = u64::from(ficr.deviceid(1).read());
            let addr = high << 32 | u64::from(ficr.deviceid(0).read());
            let addr = addr | 0x0000_c000_0000_0000;
            addr.to_le_bytes()[..6]
                .try_into()
                .expect("Failed to read BLE address from FICR")
        };
        let mut host_resources = ::rmk::HostResources::new();
        let stack =
            ::rmk::ble::build_ble_stack(sdc, ble_addr, &mut rng_gen, &mut host_resources).await;
    }
}
