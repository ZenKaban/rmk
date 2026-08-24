//! Ergohaven Settings Reset — bare metal, no Embassy
//!
//! Erases the unified Ergohaven RMK settings flash range using raw NVMC
//! registers. Then resets into the Adafruit bootloader.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use cortex_m_rt::entry;

/// NVMC register addresses (nRF52840)
const NVMC_BASE: u32 = 0x4001_E000;
const NVMC_READY: *const u32 = (NVMC_BASE + 0x400) as *const u32;
const NVMC_CONFIG: *mut u32 = (NVMC_BASE + 0x504) as *mut u32;
const NVMC_ERASEPAGE: *mut u32 = (NVMC_BASE + 0x508) as *mut u32;

/// POWER GPREGRET: Adafruit nRF52 bootloader enters UF2/DFU on 0x57.
const POWER_BASE: u32 = 0x4000_0000;
const POWER_GPREGRET: *mut u32 = (POWER_BASE + 0x51C) as *mut u32;
const ADAFRUIT_DFU_MAGIC: u32 = 0x57;

/// NVMC CONFIG values
const NVMC_CONFIG_REN: u32 = 0; // Read-only
const NVMC_CONFIG_EEN: u32 = 2; // Erase enable

/// All production profiles reserve the K:04-compatible 128 KiB partition.
const STORAGE_RANGE: (u32, u32) = (0xCC000, 0xEC000);
const PAGE_SIZE: u32 = 4096;
const WORD_SIZE: u32 = 4;
const ERASED_WORD: u32 = 0xFFFF_FFFF;
const ERASE_ATTEMPTS: u8 = 3;

/// SCB AIRCR for system reset
const SCB_AIRCR: *mut u32 = 0xE000_ED0C as *mut u32;
const AIRCR_VECTKEY: u32 = 0x05FA_0000;
const AIRCR_SYSRESETREQ: u32 = 1 << 2;

/// Wait for NVMC to be ready
#[inline(never)]
fn nvmc_wait() {
    unsafe { while core::ptr::read_volatile(NVMC_READY) == 0 {} }
}

/// Erase a single flash page
#[inline(never)]
fn erase_page(addr: u32) {
    unsafe {
        // Enable erase
        nvmc_wait();
        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_EEN);
        nvmc_wait();

        // Erase page
        core::ptr::write_volatile(NVMC_ERASEPAGE, addr);
        nvmc_wait();

        // Back to read-only
        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_REN);
        nvmc_wait();
    }
}

/// Verify a single flash page is fully erased.
#[inline(never)]
fn page_is_erased(addr: u32) -> bool {
    let mut word = addr;
    while word < addr + PAGE_SIZE {
        let value = unsafe { core::ptr::read_volatile(word as *const u32) };
        if value != ERASED_WORD {
            return false;
        }
        word += WORD_SIZE;
    }
    true
}

#[inline(never)]
fn erase_page_checked(addr: u32) {
    let mut attempt = 0;
    while attempt < ERASE_ATTEMPTS {
        erase_page(addr);
        if page_is_erased(addr) {
            return;
        }
        attempt += 1;
    }

    // If erase verification failed, stop here instead of booting a half-reset keyboard.
    loop {}
}

/// System reset
fn system_reset() -> ! {
    unsafe {
        core::ptr::write_volatile(SCB_AIRCR, AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    }
    loop {}
}

/// Reset into the Adafruit UF2 bootloader.
fn bootloader_reset() -> ! {
    unsafe {
        core::ptr::write_volatile(POWER_GPREGRET, ADAFRUIT_DFU_MAGIC);
    }
    system_reset();
}

#[entry]
fn main() -> ! {
    cortex_m::interrupt::disable();

    let (start, end) = STORAGE_RANGE;
    let mut addr = start;
    while addr < end {
        erase_page_checked(addr);
        addr += PAGE_SIZE;
    }

    bootloader_reset();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    bootloader_reset();
}
