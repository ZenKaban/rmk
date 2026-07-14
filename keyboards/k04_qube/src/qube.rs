#![no_main]
#![no_std]

//! K:04 Qube dongle — USB HID central + ST7789 status screen.
//!
//! Build: `cargo make uf2-qube`

mod layer_names;
mod qube_display;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    add_interrupt! {
        SPIM3 => ::embassy_nrf::spim::InterruptHandler<::embassy_nrf::peripherals::SPI3>;
    }

    #[register_processor(event)]
    fn display_processor() -> crate::qube_display::DongleScreen<Irqs> {
        crate::qube_display::create_processor(
            p.SPI3, p.P1_11, p.P1_10, p.P1_13, p.P0_28, p.P0_03, p.P0_02, Irqs,
        )
    }
}
