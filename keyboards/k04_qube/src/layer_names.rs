use core::str;
use core::sync::atomic::{AtomicU8, Ordering};

use rmk::config::{VialDeviceSettings, VialDeviceSettingsData};

pub const LAYER_NAME_COUNT: usize = 16;
pub const LAYER_NAME_MAX: usize = 12;
const LAYER_NAME_QSID_BASE: u16 = 200;
const STORAGE_RECORD_LEN: usize = LAYER_NAME_MAX + 1;

pub type LayerNameString = heapless::String<LAYER_NAME_MAX>;

const SETTING_KEYS: [u16; LAYER_NAME_COUNT] = [
    200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215,
];

static LAYER_NAME_LEN: [AtomicU8; LAYER_NAME_COUNT] =
    [const { AtomicU8::new(0) }; LAYER_NAME_COUNT];
static LAYER_NAME_BYTES: [AtomicU8; LAYER_NAME_COUNT * LAYER_NAME_MAX] =
    [const { AtomicU8::new(0) }; LAYER_NAME_COUNT * LAYER_NAME_MAX];
static LAYER_NAMES_VERSION: AtomicU8 = AtomicU8::new(0);

pub const fn vial_device_settings() -> VialDeviceSettings<'static> {
    VialDeviceSettings {
        setting_keys: &SETTING_KEYS,
        get_setting,
        set_setting,
        serialize,
        deserialize,
    }
}

pub fn version() -> u8 {
    LAYER_NAMES_VERSION.load(Ordering::Relaxed)
}

pub fn copy_layer_name(layer: u8, out: &mut LayerNameString) -> bool {
    let index = layer as usize;
    if index >= LAYER_NAME_COUNT {
        return false;
    }

    let len = LAYER_NAME_LEN[index].load(Ordering::Acquire) as usize;
    if len == 0 || len > LAYER_NAME_MAX {
        return false;
    }

    let mut bytes = [0u8; LAYER_NAME_MAX];
    let base = index * LAYER_NAME_MAX;
    for (offset, byte) in bytes.iter_mut().take(len).enumerate() {
        *byte = LAYER_NAME_BYTES[base + offset].load(Ordering::Relaxed);
    }

    let Ok(name) = str::from_utf8(&bytes[..len]) else {
        return false;
    };
    out.clear();
    out.push_str(name).is_ok()
}

fn get_setting(qsid: u16, out: &mut [u8]) -> Option<usize> {
    let index = layer_index(qsid)?;
    let len = LAYER_NAME_LEN[index].load(Ordering::Acquire) as usize;
    let copy_len = len.min(LAYER_NAME_MAX).min(out.len().saturating_sub(1));
    let base = index * LAYER_NAME_MAX;
    for (offset, byte) in out.iter_mut().take(copy_len).enumerate() {
        *byte = LAYER_NAME_BYTES[base + offset].load(Ordering::Relaxed);
    }
    if out.len() > copy_len {
        out[copy_len] = 0;
        Some(copy_len + 1)
    } else {
        Some(copy_len)
    }
}

fn set_setting(qsid: u16, value: &[u8]) -> bool {
    let Some(index) = layer_index(qsid) else {
        return false;
    };
    let end = value
        .iter()
        .position(|&byte| byte == 0 || byte == 0xFF)
        .unwrap_or(value.len());
    let Ok(text) = str::from_utf8(&value[..end]) else {
        return false;
    };
    store_layer_name(index, text);
    true
}

fn serialize() -> VialDeviceSettingsData {
    let mut data = VialDeviceSettingsData::empty();
    let mut pos = 0usize;
    for index in 0..LAYER_NAME_COUNT {
        if pos + STORAGE_RECORD_LEN > data.data.len() {
            break;
        }
        let len = LAYER_NAME_LEN[index]
            .load(Ordering::Acquire)
            .min(LAYER_NAME_MAX as u8) as usize;
        data.data[pos] = len as u8;
        let base = index * LAYER_NAME_MAX;
        for offset in 0..LAYER_NAME_MAX {
            data.data[pos + 1 + offset] = if offset < len {
                LAYER_NAME_BYTES[base + offset].load(Ordering::Relaxed)
            } else {
                0
            };
        }
        pos += STORAGE_RECORD_LEN;
    }
    data.len = pos as u8;
    data
}

fn deserialize(bytes: &[u8]) {
    let mut pos = 0usize;
    for index in 0..LAYER_NAME_COUNT {
        if pos + STORAGE_RECORD_LEN > bytes.len() {
            break;
        }
        let len = bytes[pos].min(LAYER_NAME_MAX as u8) as usize;
        store_raw_layer_name(index, &bytes[pos + 1..pos + 1 + len]);
        pos += STORAGE_RECORD_LEN;
    }
    LAYER_NAMES_VERSION.fetch_add(1, Ordering::Relaxed);
}

fn store_layer_name(index: usize, text: &str) {
    let mut sanitized = LayerNameString::new();
    let mut chars = text.trim().chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' && chars.peek() == Some(&'%') {
            let _ = chars.next();
        }
        if sanitized.push(ch).is_err() {
            break;
        }
    }
    store_raw_layer_name(index, sanitized.as_bytes());
    LAYER_NAMES_VERSION.fetch_add(1, Ordering::Relaxed);
}

fn store_raw_layer_name(index: usize, bytes: &[u8]) {
    if index >= LAYER_NAME_COUNT {
        return;
    }
    let len = bytes.len().min(LAYER_NAME_MAX);
    LAYER_NAME_LEN[index].store(0, Ordering::Release);
    let base = index * LAYER_NAME_MAX;
    for offset in 0..LAYER_NAME_MAX {
        let byte = bytes.get(offset).copied().unwrap_or(0);
        LAYER_NAME_BYTES[base + offset].store(byte, Ordering::Relaxed);
    }
    LAYER_NAME_LEN[index].store(len as u8, Ordering::Release);
}

fn layer_index(qsid: u16) -> Option<usize> {
    let offset = qsid.checked_sub(LAYER_NAME_QSID_BASE)?;
    (offset < LAYER_NAME_COUNT as u16).then_some(offset as usize)
}
