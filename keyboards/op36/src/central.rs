#![no_main]
#![no_std]

mod battery_nrf;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    #[register_processor(event)]
    fn battery() -> crate::battery_nrf::Op36Battery {
        crate::battery_nrf::Op36Battery::new(p.SAADC, p.P0_31)
    }

    #[register_processor(poll)]
    fn ergohaven_user_keys() -> ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys {
        ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys::new()
    }
}
