#![no_main]
#![no_std]

mod battery_nrf;
mod pointing_mode;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    #[register_processor(event)]
    fn battery() -> crate::battery_nrf::SplitBattery {
        crate::battery_nrf::SplitBattery::new(p.SAADC, p.P0_31)
    }

    #[register_processor(event)]
    fn pointing_mode() -> crate::pointing_mode::VelvetUiPointingMode {
        crate::pointing_mode::VelvetUiPointingMode::new()
    }

    #[register_processor(poll)]
    fn ergohaven_user_keys() -> ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys {
        ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys::new()
    }
}
