//! One-time raw storage migration for pre-unification Ergohaven firmware.
//!
//! Copies the legacy 128 KiB RMK partition at 0xA0000 to the unified
//! K:04-compatible partition at 0xCC000, verifies it, and returns to the
//! bootloader. The legacy source is left untouched.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use cortex_m_rt::entry;
use storage_migrate::{word_is_migration_compatible, ERASED_WORD};

const NVMC_BASE: u32 = 0x4001_E000;
const NVMC_READY: *const u32 = (NVMC_BASE + 0x400) as *const u32;
const NVMC_CONFIG: *mut u32 = (NVMC_BASE + 0x504) as *mut u32;
const NVMC_ERASEPAGE: *mut u32 = (NVMC_BASE + 0x508) as *mut u32;

const POWER_BASE: u32 = 0x4000_0000;
const POWER_GPREGRET: *mut u32 = (POWER_BASE + 0x51C) as *mut u32;
const ADAFRUIT_DFU_MAGIC: u32 = 0x57;

const NVMC_CONFIG_REN: u32 = 0;
const NVMC_CONFIG_WEN: u32 = 1;
const NVMC_CONFIG_EEN: u32 = 2;

const LEGACY_START: u32 = 0xA0000;
const UNIFIED_START: u32 = 0xCC000;
const STORAGE_SIZE: u32 = 0x20000;
const PAGE_SIZE: u32 = 4096;
const WORD_SIZE: u32 = 4;
const WRITE_ATTEMPTS: u8 = 3;

const SCB_AIRCR: *mut u32 = 0xE000_ED0C as *mut u32;
const AIRCR_VECTKEY: u32 = 0x05FA_0000;
const AIRCR_SYSRESETREQ: u32 = 1 << 2;

#[inline(never)]
fn nvmc_wait() {
    unsafe { while core::ptr::read_volatile(NVMC_READY) == 0 {} }
}

#[inline(never)]
fn erase_page(addr: u32) {
    unsafe {
        nvmc_wait();
        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_EEN);
        nvmc_wait();
        core::ptr::write_volatile(NVMC_ERASEPAGE, addr);
        nvmc_wait();
        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_REN);
        nvmc_wait();
    }
}

fn pages_equal(left: u32, right: u32) -> bool {
    let mut offset = 0;
    while offset < PAGE_SIZE {
        let left_value = unsafe { core::ptr::read_volatile((left + offset) as *const u32) };
        let right_value = unsafe { core::ptr::read_volatile((right + offset) as *const u32) };
        if left_value != right_value {
            return false;
        }
        offset += WORD_SIZE;
    }
    true
}

fn page_is_compatible(source: u32, destination: u32) -> bool {
    let mut offset = 0;
    while offset < PAGE_SIZE {
        let source_value = unsafe { core::ptr::read_volatile((source + offset) as *const u32) };
        let destination_value = unsafe { core::ptr::read_volatile((destination + offset) as *const u32) };
        if !word_is_migration_compatible(source_value, destination_value) {
            return false;
        }
        offset += WORD_SIZE;
    }
    true
}

/// A migration can start or resume only when every programmed destination word
/// matches its source. This protects an existing K:04 partition while allowing
/// an interrupted, partially written destination page to be erased and retried.
fn destination_is_safe() -> bool {
    let mut offset = 0;
    while offset < STORAGE_SIZE {
        let source = LEGACY_START + offset;
        let destination = UNIFIED_START + offset;
        if !page_is_compatible(source, destination) {
            return false;
        }
        offset += PAGE_SIZE;
    }
    true
}

#[inline(never)]
fn copy_page(source: u32, destination: u32) {
    erase_page(destination);

    unsafe {
        nvmc_wait();
        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_WEN);
        nvmc_wait();

        let mut offset = 0;
        while offset < PAGE_SIZE {
            let value = core::ptr::read_volatile((source + offset) as *const u32);
            if value != ERASED_WORD {
                core::ptr::write_volatile((destination + offset) as *mut u32, value);
                nvmc_wait();
            }
            offset += WORD_SIZE;
        }

        core::ptr::write_volatile(NVMC_CONFIG, NVMC_CONFIG_REN);
        nvmc_wait();
    }
}

fn copy_page_checked(source: u32, destination: u32) {
    let mut attempt = 0;
    while attempt < WRITE_ATTEMPTS {
        copy_page(source, destination);
        if pages_equal(source, destination) {
            return;
        }
        attempt += 1;
    }

    // Keep the source intact and stop instead of booting with a partial copy.
    loop {}
}

fn system_reset() -> ! {
    unsafe {
        core::ptr::write_volatile(SCB_AIRCR, AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    }
    loop {}
}

fn bootloader_reset() -> ! {
    unsafe {
        core::ptr::write_volatile(POWER_GPREGRET, ADAFRUIT_DFU_MAGIC);
    }
    system_reset();
}

#[entry]
fn main() -> ! {
    cortex_m::interrupt::disable();

    if destination_is_safe() {
        let mut offset = 0;
        while offset < STORAGE_SIZE {
            let source = LEGACY_START + offset;
            let destination = UNIFIED_START + offset;
            if !pages_equal(source, destination) {
                copy_page_checked(source, destination);
            }
            offset += PAGE_SIZE;
        }
    }

    bootloader_reset();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    bootloader_reset();
}
