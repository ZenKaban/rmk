use core::sync::atomic::{AtomicU8, Ordering};

const UNKNOWN: u8 = u8::MAX;

static HOST_HOUR: AtomicU8 = AtomicU8::new(UNKNOWN);
static HOST_MINUTE: AtomicU8 = AtomicU8::new(UNKNOWN);
static HOST_LAYOUT: AtomicU8 = AtomicU8::new(UNKNOWN);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostData {
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub layout: Option<u8>,
}

pub fn update_time(hour: u8, minute: u8) {
    if hour < 24 && minute < 60 {
        HOST_HOUR.store(hour, Ordering::Relaxed);
        HOST_MINUTE.store(minute, Ordering::Relaxed);
    }
}

pub fn update_layout(layout: u8) {
    HOST_LAYOUT.store(layout, Ordering::Relaxed);
}

pub fn snapshot() -> HostData {
    HostData {
        hour: known(HOST_HOUR.load(Ordering::Relaxed)),
        minute: known(HOST_MINUTE.load(Ordering::Relaxed)),
        layout: known(HOST_LAYOUT.load(Ordering::Relaxed)),
    }
}

fn known(value: u8) -> Option<u8> {
    (value != UNKNOWN).then_some(value)
}
