#![no_main]
#![no_std]

mod battery_nrf;

use rmk::macros::rmk_peripheral;

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    #[register_processor(event)]
    fn battery() -> crate::battery_nrf::K04Battery {
        crate::battery_nrf::K04Battery::new(p.SAADC, p.P0_31)
    }
}
