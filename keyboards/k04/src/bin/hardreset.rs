#![no_std]
#![no_main]

// K:04 hard-reset - direct NVMC register writes, no embassy runtime

use core::panic::PanicInfo;
use cortex_m::peripheral::SCB;
use cortex_m_rt::entry;

// Import for interrupt vector table
use embassy_nrf as _;
use nrf_mpsl as _;

const NVMC_BASE: u32 = 0x4001_E000;
const NVMC_READY: *const u32 = (NVMC_BASE + 0x400) as *const u32;
const NVMC_CONFIG: *mut u32 = (NVMC_BASE + 0x504) as *mut u32;
const NVMC_ERASEPAGE: *mut u32 = (NVMC_BASE + 0x508) as *mut u32;

const PAGE_SIZE: u32 = 4096;
const ERASE_START: u32 = 0x60000;
const ERASE_END: u32 = 0xF4000;

fn nvmc_wait() {
    unsafe { while core::ptr::read_volatile(NVMC_READY) == 0 {} }
}

fn erase_page(addr: u32) {
    nvmc_wait();
    unsafe {
        core::ptr::write_volatile(NVMC_CONFIG, 2);
        nvmc_wait();
        core::ptr::write_volatile(NVMC_ERASEPAGE, addr);
        nvmc_wait();
        core::ptr::write_volatile(NVMC_CONFIG, 0);
        nvmc_wait();
    }
}

#[entry]
fn main() -> ! {
    let mut addr = ERASE_START;
    while addr < ERASE_END {
        erase_page(addr);
        addr += PAGE_SIZE;
    }
    SCB::sys_reset();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    SCB::sys_reset();
}
