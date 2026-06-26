// k04-vial-settings-v0.0.76: keyboard-specific Vial settings for modules, auto layer, and LEDs.

use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use rmk::config::{VialDeviceSettings, VialDeviceSettingsData};
use rmk::{channel::send_controller_event_new, event::ControllerEvent};

pub const BALL_DPI_TABLE: [u16; 16] = [
    200, 400, 600, 800, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2400, 2600, 2800, 3000, 3200,
];

const VERSION: u8 = 8;
pub const SETTINGS_LEN: usize = 42;
pub const SETTINGS_SYNC_LEN: usize = 27;
const SETTINGS_STORAGE_LEN: usize = 32;
const SETTINGS_STORAGE_LEN_V3_LEGACY_31: usize = 31;
const SETTINGS_STORAGE_LEN_V3_LEGACY_30: usize = 30;
const SETTINGS_STORAGE_LEN_V3_LEGACY_29: usize = 29;
const TOUCH_DPI_BASE: u16 = 400;
const AUTO_LAYER_TIMEOUT_MS_TABLE: [u32; 6] = [250, 500, 750, 1000, 1250, 1500];
const ENCODER_INTERVAL_MS_TABLE: [u64; 10] = [0, 5, 10, 15, 20, 30, 40, 60, 80, 100];
const SLEEP_TIMEOUT_SECONDS_TABLE: [u64; 10] = [
    10 * 60,
    15 * 60,
    20 * 60,
    30 * 60,
    45 * 60,
    60 * 60,
    2 * 60 * 60,
    3 * 60 * 60,
    4 * 60 * 60,
    5 * 60 * 60,
];

const IDX_VERSION: usize = 0;
const IDX_LEFT_MODE: usize = 1;
const IDX_RIGHT_MODE: usize = 2;
const IDX_LEFT_BALL_AXIS: usize = 3;
const IDX_RIGHT_BALL_AXIS: usize = 4;
const IDX_LEFT_TOUCH_AXIS: usize = 5;
const IDX_RIGHT_TOUCH_AXIS: usize = 6;
const IDX_LEFT_BALL_DPI: usize = 7;
const IDX_RIGHT_BALL_DPI: usize = 8;
const IDX_LEFT_TOUCH_DPI: usize = 9;
const IDX_RIGHT_TOUCH_DPI: usize = 10;
const IDX_LEFT_SCROLL_SENS: usize = 11;
const IDX_LEFT_SNIPER_SENS: usize = 12;
const IDX_LEFT_TEXT_SENS: usize = 13;
const IDX_RIGHT_SCROLL_SENS: usize = 14;
const IDX_RIGHT_SNIPER_SENS: usize = 15;
const IDX_RIGHT_TEXT_SENS: usize = 16;
const IDX_FLAGS: usize = 17;
const IDX_AUTO_LAYER: usize = 18;
const IDX_AUTO_FLAGS: usize = 19;
const IDX_LED_BRIGHTNESS: usize = 20;
const IDX_LED_TIMEOUT_SEC: usize = 21;
const IDX_LAYER_COLORS_PACKED: usize = 22;
const IDX_BT_PROFILE_COLORS: usize = 32;
const IDX_SLEEP_TIMEOUT: usize = 37;
const IDX_AUTO_LAYER_TIMEOUT: usize = 38;
const IDX_MODULE_SELECT: usize = 39;
const IDX_LEFT_ENCODER_INTERVAL: usize = 40;
const IDX_RIGHT_ENCODER_INTERVAL: usize = 41;

const FLAG_LEFT_INVERT_SCROLL: u8 = 1 << 0;
const FLAG_RIGHT_INVERT_SCROLL: u8 = 1 << 1;
const FLAG_LEFT_INVERT_TEXT: u8 = 1 << 2;
const FLAG_RIGHT_INVERT_TEXT: u8 = 1 << 3;
const FLAG_LEFT_ACCELERATION: u8 = 1 << 4;
const FLAG_RIGHT_ACCELERATION: u8 = 1 << 5;
const FLAG_LEFT_STICKY: u8 = 1 << 6;
const FLAG_RIGHT_STICKY: u8 = 1 << 7;

const AUTO_FLAG_TOUCH_GESTURES_LEFT: u8 = 1 << 4;
const AUTO_FLAG_TOUCH_GESTURES_RIGHT: u8 = 1 << 5;
const AUTO_FLAG_TOUCH_GESTURES_MASK: u8 =
    AUTO_FLAG_TOUCH_GESTURES_LEFT | AUTO_FLAG_TOUCH_GESTURES_RIGHT;

const MODULE_SELECT_MASK: u8 = 0x0f;
const MODULE_SELECT_NONE: u8 = 0;
const MODULE_SELECT_ENCODER: u8 = 1;
const MODULE_SELECT_BALL: u8 = 2;
const MODULE_SELECT_TOUCH: u8 = 3;

const DEFAULTS: [u8; SETTINGS_LEN] = {
    let mut data = [0u8; SETTINGS_LEN];
    data[IDX_VERSION] = VERSION;
    data[IDX_LEFT_BALL_DPI] = 4;
    data[IDX_RIGHT_BALL_DPI] = 4;
    data[IDX_LEFT_TOUCH_DPI] = 3;
    data[IDX_RIGHT_TOUCH_DPI] = 3;
    data[IDX_MODULE_SELECT] = (MODULE_SELECT_TOUCH << 0) | (MODULE_SELECT_BALL << 2);
    data[IDX_LEFT_SCROLL_SENS] = 8;
    data[IDX_RIGHT_SCROLL_SENS] = 8;
    data[IDX_LEFT_SNIPER_SENS] = 4;
    data[IDX_RIGHT_SNIPER_SENS] = 4;
    data[IDX_LEFT_TEXT_SENS] = 16;
    data[IDX_RIGHT_TEXT_SENS] = 16;
    data[IDX_AUTO_LAYER] = 4;
    data[IDX_AUTO_FLAGS] = 1;
    data[IDX_LED_BRIGHTNESS] = 8;
    data[IDX_LED_TIMEOUT_SEC] = 1;
    data[IDX_SLEEP_TIMEOUT] = 3;
    data[IDX_AUTO_LAYER_TIMEOUT] = 1;
    data[IDX_LEFT_ENCODER_INTERVAL] = 4;
    data[IDX_RIGHT_ENCODER_INTERVAL] = 4;
    data = set_default_layer_color(data, 0, 0);
    data = set_default_layer_color(data, 1, 2);
    data = set_default_layer_color(data, 2, 16);
    data = set_default_layer_color(data, 3, 24);
    data = set_default_layer_color(data, 4, 6);
    data = set_default_layer_color(data, 5, 8);
    data = set_default_layer_color(data, 6, 10);
    data = set_default_layer_color(data, 7, 11);
    data = set_default_layer_color(data, 8, 15);
    data = set_default_layer_color(data, 9, 19);
    data = set_default_layer_color(data, 10, 20);
    data = set_default_layer_color(data, 11, 21);
    data = set_default_layer_color(data, 12, 22);
    data = set_default_layer_color(data, 13, 23);
    data = set_default_layer_color(data, 14, 3);
    data = set_default_layer_color(data, 15, 17);
    data[IDX_BT_PROFILE_COLORS] = 2;
    data[IDX_BT_PROFILE_COLORS + 1] = 16;
    data[IDX_BT_PROFILE_COLORS + 2] = 6;
    data[IDX_BT_PROFILE_COLORS + 3] = 8;
    data[IDX_BT_PROFILE_COLORS + 4] = 19;
    data
};

static SETTINGS: [AtomicU8; SETTINGS_LEN] = [const { AtomicU8::new(0) }; SETTINGS_LEN];
static MODE_OVERRIDE_ACTIVE: AtomicU8 = AtomicU8::new(0);
static MODE_OVERRIDE: [AtomicU8; 2] = [const { AtomicU8::new(0) }; 2];
static MODE_KEY_PREV_OVERRIDE_ACTIVE: AtomicU8 = AtomicU8::new(0);
static MODE_KEY_PREV_OVERRIDE: [AtomicU8; 2] = [const { AtomicU8::new(0) }; 2];
static ACTIVE_POINTING_MODULE: AtomicU8 = AtomicU8::new(POINTING_MODULE_NONE);
static LEFT_BALL_SELECTION_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static LEFT_TOUCH_SELECTION_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RIGHT_BALL_SELECTION_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RIGHT_TOUCH_SELECTION_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static SETTINGS_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

const POINTING_MODULE_NONE: u8 = 0xff;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModuleSide {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Ball,
    Touch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModuleSelection {
    None,
    Encoder,
    Ball,
    Touch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PointingMode {
    Normal,
    Sniper,
    Scroll,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const fn vial_device_settings() -> VialDeviceSettings<'static> {
    VialDeviceSettings {
        setting_keys: &SETTING_KEYS,
        get_setting,
        set_setting,
        serialize,
        deserialize,
    }
}

pub fn side_index(side: ModuleSide) -> u8 {
    match side {
        ModuleSide::Left => 0,
        ModuleSide::Right => 1,
    }
}

pub fn kind_index(kind: ModuleKind) -> u8 {
    match kind {
        ModuleKind::Ball => 0,
        ModuleKind::Touch => 1,
    }
}

pub fn pointing_module_enabled(side: ModuleSide, kind: ModuleKind) -> bool {
    matches!(
        (module_selection(side), kind),
        (ModuleSelection::Ball, ModuleKind::Ball) | (ModuleSelection::Touch, ModuleKind::Touch)
    )
}

pub fn encoder_module_enabled(side: ModuleSide) -> bool {
    module_selection(side) == ModuleSelection::Encoder
}

pub fn module_selection(side: ModuleSide) -> ModuleSelection {
    module_selection_from_value(module_selection_value(side))
}

pub fn pointing_module_claimed_by_other(kind: ModuleKind) -> bool {
    let active = ACTIVE_POINTING_MODULE.load(Ordering::Relaxed);
    active != POINTING_MODULE_NONE && active != kind_index(kind)
}

pub fn claim_pointing_module(kind: ModuleKind) -> bool {
    let desired = kind_index(kind);
    match ACTIVE_POINTING_MODULE.compare_exchange(
        POINTING_MODULE_NONE,
        desired,
        Ordering::Relaxed,
        Ordering::Relaxed,
    ) {
        Ok(_) => true,
        Err(active) => active == desired,
    }
}

pub fn release_pointing_module(kind: ModuleKind) {
    let desired = kind_index(kind);
    let _ = ACTIVE_POINTING_MODULE.compare_exchange(
        desired,
        POINTING_MODULE_NONE,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

pub async fn wait_pointing_module_selection_change(side: ModuleSide, kind: ModuleKind) {
    match (side, kind) {
        (ModuleSide::Left, ModuleKind::Ball) => LEFT_BALL_SELECTION_CHANGED.wait().await,
        (ModuleSide::Left, ModuleKind::Touch) => LEFT_TOUCH_SELECTION_CHANGED.wait().await,
        (ModuleSide::Right, ModuleKind::Ball) => RIGHT_BALL_SELECTION_CHANGED.wait().await,
        (ModuleSide::Right, ModuleKind::Touch) => RIGHT_TOUCH_SELECTION_CHANGED.wait().await,
    }
}

pub async fn wait_settings_change() {
    SETTINGS_CHANGED.wait().await;
}

pub fn side_from_index(value: u8) -> ModuleSide {
    if value == 0 {
        ModuleSide::Left
    } else {
        ModuleSide::Right
    }
}

pub fn kind_from_index(value: u8) -> ModuleKind {
    if value == 0 {
        ModuleKind::Ball
    } else {
        ModuleKind::Touch
    }
}

pub fn pointing_mode(side: ModuleSide) -> PointingMode {
    mode_from_value(if mode_override_active(side) {
        MODE_OVERRIDE[side_slot(side)].load(Ordering::Relaxed)
    } else {
        stored_mode_value(side)
    })
}

pub fn handle_mode_key(mode: PointingMode, pressed: bool, tapped: bool) {
    if pressed {
        store_mode_key_prev_override(ModuleSide::Left);
        store_mode_key_prev_override(ModuleSide::Right);
        set_mode_override(ModuleSide::Left, mode);
        set_mode_override(ModuleSide::Right, mode);
        return;
    }

    restore_or_latch_mode_override(ModuleSide::Left, mode, tapped);
    restore_or_latch_mode_override(ModuleSide::Right, mode, tapped);
}

pub fn handle_side_mode_key(side: ModuleSide, mode: PointingMode, pressed: bool, tapped: bool) {
    if pressed {
        store_mode_key_prev_override(side);
        set_mode_override(side, mode);
        return;
    }

    restore_or_latch_mode_override(side, mode, tapped);
}

fn mode_from_value(value: u8) -> PointingMode {
    match value.min(3) {
        1 => PointingMode::Sniper,
        2 => PointingMode::Scroll,
        3 => PointingMode::Text,
        _ => PointingMode::Normal,
    }
}

fn stored_mode_value(side: ModuleSide) -> u8 {
    byte(match side {
        ModuleSide::Left => IDX_LEFT_MODE,
        ModuleSide::Right => IDX_RIGHT_MODE,
    })
    .min(3)
}

fn side_slot(side: ModuleSide) -> usize {
    match side {
        ModuleSide::Left => 0,
        ModuleSide::Right => 1,
    }
}

fn side_mask(side: ModuleSide) -> u8 {
    1 << side_slot(side)
}

fn mode_override_active(side: ModuleSide) -> bool {
    (MODE_OVERRIDE_ACTIVE.load(Ordering::Relaxed) & side_mask(side)) != 0
}

fn set_mode_override(side: ModuleSide, mode: PointingMode) {
    let slot = side_slot(side);
    MODE_OVERRIDE[slot].store(mode_value(mode), Ordering::Relaxed);
    MODE_OVERRIDE_ACTIVE.fetch_or(side_mask(side), Ordering::Relaxed);
}

fn clear_mode_override(side: ModuleSide) {
    MODE_OVERRIDE_ACTIVE.fetch_and(!side_mask(side), Ordering::Relaxed);
}

fn store_mode_key_prev_override(side: ModuleSide) {
    let slot = side_slot(side);
    let mask = side_mask(side);
    let active = MODE_OVERRIDE_ACTIVE.load(Ordering::Relaxed);
    if (active & mask) != 0 {
        MODE_KEY_PREV_OVERRIDE_ACTIVE.fetch_or(mask, Ordering::Relaxed);
    } else {
        MODE_KEY_PREV_OVERRIDE_ACTIVE.fetch_and(!mask, Ordering::Relaxed);
    }
    MODE_KEY_PREV_OVERRIDE[slot].store(
        MODE_OVERRIDE[slot].load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

fn restore_mode_override(side: ModuleSide) {
    let slot = side_slot(side);
    let mask = side_mask(side);
    if (MODE_KEY_PREV_OVERRIDE_ACTIVE.load(Ordering::Relaxed) & mask) != 0 {
        MODE_OVERRIDE[slot].store(
            MODE_KEY_PREV_OVERRIDE[slot].load(Ordering::Relaxed).min(3),
            Ordering::Relaxed,
        );
        MODE_OVERRIDE_ACTIVE.fetch_or(mask, Ordering::Relaxed);
    } else {
        clear_mode_override(side);
    }
}

fn restore_or_latch_mode_override(side: ModuleSide, mode: PointingMode, tapped: bool) {
    restore_mode_override(side);
    if sticky_mode(side) && tapped {
        toggle_mode_override(side, mode);
    }
}

fn toggle_mode_override(side: ModuleSide, mode: PointingMode) {
    let value = mode_value(mode);
    if mode_override_active(side)
        && MODE_OVERRIDE[side_slot(side)]
            .load(Ordering::Relaxed)
            .min(3)
            == value
    {
        clear_mode_override(side);
    } else {
        set_mode_override(side, mode);
    }
}

fn sticky_mode(side: ModuleSide) -> bool {
    flag(match side {
        ModuleSide::Left => FLAG_LEFT_STICKY,
        ModuleSide::Right => FLAG_RIGHT_STICKY,
    })
}

pub fn toggle_both_pointing_mode(mode: PointingMode) {
    let value = mode_value(mode);
    let target = if byte(IDX_LEFT_MODE).min(3) == value && byte(IDX_RIGHT_MODE).min(3) == value {
        mode_value(PointingMode::Normal)
    } else {
        value
    };
    set_byte(IDX_LEFT_MODE, target);
    set_byte(IDX_RIGHT_MODE, target);
    publish_settings_snapshot();
}

pub fn orientation(side: ModuleSide, kind: ModuleKind) -> u8 {
    byte(match (side, kind) {
        (ModuleSide::Left, ModuleKind::Ball) => IDX_LEFT_BALL_AXIS,
        (ModuleSide::Right, ModuleKind::Ball) => IDX_RIGHT_BALL_AXIS,
        (ModuleSide::Left, ModuleKind::Touch) => IDX_LEFT_TOUCH_AXIS,
        (ModuleSide::Right, ModuleKind::Touch) => IDX_RIGHT_TOUCH_AXIS,
    })
    .min(3)
}

pub fn ball_cpi(side: ModuleSide) -> u16 {
    let idx = byte(match side {
        ModuleSide::Left => IDX_LEFT_BALL_DPI,
        ModuleSide::Right => IDX_RIGHT_BALL_DPI,
    }) as usize;
    BALL_DPI_TABLE[idx.min(BALL_DPI_TABLE.len() - 1)]
}

pub fn touch_dpi(side: ModuleSide) -> u16 {
    let idx = byte(match side {
        ModuleSide::Left => IDX_LEFT_TOUCH_DPI,
        ModuleSide::Right => IDX_RIGHT_TOUCH_DPI,
    });
    (u16::from(idx.min(9)) + 1) * 100
}

pub fn scale_touch_delta(value: i16, side: ModuleSide) -> i16 {
    let scaled = i32::from(value) * i32::from(touch_dpi(side)) / i32::from(TOUCH_DPI_BASE);
    scaled.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

pub fn sens(side: ModuleSide, mode: PointingMode) -> i16 {
    let idx = match (side, mode) {
        (ModuleSide::Left, PointingMode::Scroll) => IDX_LEFT_SCROLL_SENS,
        (ModuleSide::Left, PointingMode::Sniper) => IDX_LEFT_SNIPER_SENS,
        (ModuleSide::Left, PointingMode::Text) => IDX_LEFT_TEXT_SENS,
        (ModuleSide::Right, PointingMode::Scroll) => IDX_RIGHT_SCROLL_SENS,
        (ModuleSide::Right, PointingMode::Sniper) => IDX_RIGHT_SNIPER_SENS,
        (ModuleSide::Right, PointingMode::Text) => IDX_RIGHT_TEXT_SENS,
        (_, PointingMode::Normal) => return 1,
    };
    i16::from(byte(idx).max(1))
}

pub fn invert_scroll(side: ModuleSide) -> bool {
    flag(match side {
        ModuleSide::Left => FLAG_LEFT_INVERT_SCROLL,
        ModuleSide::Right => FLAG_RIGHT_INVERT_SCROLL,
    })
}

pub fn invert_text(side: ModuleSide) -> bool {
    flag(match side {
        ModuleSide::Left => FLAG_LEFT_INVERT_TEXT,
        ModuleSide::Right => FLAG_RIGHT_INVERT_TEXT,
    })
}

pub fn acceleration(side: ModuleSide) -> bool {
    flag(match side {
        ModuleSide::Left => FLAG_LEFT_ACCELERATION,
        ModuleSide::Right => FLAG_RIGHT_ACCELERATION,
    })
}

pub fn led_brightness() -> u8 {
    byte(IDX_LED_BRIGHTNESS)
}

pub fn led_timeout_sec() -> u8 {
    byte(IDX_LED_TIMEOUT_SEC)
}

pub fn sleep_timeout_index() -> u8 {
    byte(IDX_SLEEP_TIMEOUT).min(9)
}

pub fn sleep_timeout_secs() -> u64 {
    SLEEP_TIMEOUT_SECONDS_TABLE[usize::from(sleep_timeout_index())]
}

pub fn layer_color(layer: u8) -> Rgb {
    palette(layer_color_index(layer))
}

pub fn bt_profile_color(profile: u8) -> Rgb {
    palette(bt_profile_color_index(profile))
}

pub fn auto_layer() -> u8 {
    byte(IDX_AUTO_LAYER).min(15)
}

pub fn auto_layer_timeout_ms() -> u32 {
    AUTO_LAYER_TIMEOUT_MS_TABLE[byte(IDX_AUTO_LAYER_TIMEOUT).min(5) as usize]
}

pub fn encoder_interval_ms(side: ModuleSide) -> u64 {
    let idx = byte(match side {
        ModuleSide::Left => IDX_LEFT_ENCODER_INTERVAL,
        ModuleSide::Right => IDX_RIGHT_ENCODER_INTERVAL,
    });
    ENCODER_INTERVAL_MS_TABLE[usize::from(idx.min(9))]
}

pub fn auto_layer_enabled(mode: PointingMode) -> bool {
    auto_flag(match mode {
        PointingMode::Normal => 0,
        PointingMode::Sniper => 1,
        PointingMode::Scroll => 2,
        PointingMode::Text => 3,
    })
}

pub fn touch_gestures_enabled(side: ModuleSide) -> bool {
    let mask = match side {
        ModuleSide::Left => AUTO_FLAG_TOUCH_GESTURES_LEFT,
        ModuleSide::Right => AUTO_FLAG_TOUCH_GESTURES_RIGHT,
    };
    (byte(IDX_AUTO_FLAGS) & mask) != 0
}

pub fn settings_snapshot() -> [u8; SETTINGS_LEN] {
    ensure_initialized();
    let mut out = [0u8; SETTINGS_LEN];
    for idx in 0..SETTINGS_LEN {
        out[idx] = SETTINGS[idx].load(Ordering::Relaxed);
    }
    out
}

pub fn settings_sync_packet() -> [u8; SETTINGS_SYNC_LEN] {
    ensure_initialized();
    let mut out = [0u8; SETTINGS_SYNC_LEN];
    out[0] = VERSION;
    out[1] = byte(IDX_RIGHT_MODE);
    out[2] = byte(IDX_RIGHT_BALL_AXIS);
    out[3] = byte(IDX_RIGHT_TOUCH_AXIS);
    out[4] = (byte(IDX_RIGHT_BALL_DPI).min(15) & 0x0f)
        | ((module_selection_value(ModuleSide::Right) & 0x03) << 6);
    if touch_gestures_enabled(ModuleSide::Right) {
        out[4] |= 1 << 4;
    }
    out[5] = (byte(IDX_RIGHT_TOUCH_DPI).min(9) & 0x0f) | (sleep_timeout_index() << 4);
    out[6] = byte(IDX_RIGHT_SCROLL_SENS);
    out[7] = byte(IDX_RIGHT_SNIPER_SENS);
    out[8] = byte(IDX_RIGHT_TEXT_SENS);
    out[9] = byte(IDX_FLAGS);
    out[10] = byte(IDX_LED_BRIGHTNESS);
    out[11] = byte(IDX_LED_TIMEOUT_SEC);
    let mut i = 0usize;
    while i < 10 {
        out[12 + i] = byte(IDX_LAYER_COLORS_PACKED + i);
        i += 1;
    }
    i = 0;
    while i < 5 {
        out[22 + i] = byte(IDX_BT_PROFILE_COLORS + i);
        i += 1;
    }
    out
}

pub fn apply_settings_sync_packet(data: &[u8; SETTINGS_SYNC_LEN]) {
    if data[0] != VERSION {
        return;
    }
    ensure_initialized();
    let previous_sleep_timeout = sleep_timeout_index();
    SETTINGS[IDX_RIGHT_MODE].store(data[1], Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_BALL_AXIS].store(data[2], Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_TOUCH_AXIS].store(data[3], Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_BALL_DPI].store(data[4] & 0x0f, Ordering::Relaxed);
    set_touch_gestures_enabled(ModuleSide::Right, (data[4] & (1 << 4)) != 0);
    SETTINGS[IDX_RIGHT_TOUCH_DPI].store((data[5] & 0x0f).min(9), Ordering::Relaxed);
    SETTINGS[IDX_SLEEP_TIMEOUT].store((data[5] >> 4).min(9), Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_SCROLL_SENS].store(data[6], Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_SNIPER_SENS].store(data[7], Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_TEXT_SENS].store(data[8], Ordering::Relaxed);
    SETTINGS[IDX_FLAGS].store(data[9], Ordering::Relaxed);
    SETTINGS[IDX_LED_BRIGHTNESS].store(data[10], Ordering::Relaxed);
    SETTINGS[IDX_LED_TIMEOUT_SEC].store(data[11], Ordering::Relaxed);
    let mut i = 0usize;
    while i < 10 {
        SETTINGS[IDX_LAYER_COLORS_PACKED + i].store(data[12 + i], Ordering::Relaxed);
        i += 1;
    }
    i = 0;
    while i < 5 {
        SETTINGS[IDX_BT_PROFILE_COLORS + i].store(data[22 + i], Ordering::Relaxed);
        i += 1;
    }
    set_module_selection_local(ModuleSide::Right, (data[4] >> 6) & 0x03);
    if sleep_timeout_index() != previous_sleep_timeout {
        SETTINGS_CHANGED.signal(());
    }
}

pub fn publish_settings_snapshot() {
    send_controller_event_new(ControllerEvent::DeviceSettings(settings_sync_packet()));
}

fn mode_value(mode: PointingMode) -> u8 {
    match mode {
        PointingMode::Normal => 0,
        PointingMode::Sniper => 1,
        PointingMode::Scroll => 2,
        PointingMode::Text => 3,
    }
}

fn ensure_initialized() {
    if SETTINGS[IDX_VERSION].load(Ordering::Relaxed) == VERSION {
        return;
    }
    for (idx, default) in DEFAULTS.iter().enumerate() {
        SETTINGS[idx].store(*default, Ordering::Relaxed);
    }
}

fn byte(idx: usize) -> u8 {
    ensure_initialized();
    SETTINGS[idx].load(Ordering::Relaxed)
}

fn set_byte(idx: usize, value: u8) {
    ensure_initialized();
    SETTINGS[idx].store(value, Ordering::Relaxed);
}

fn flag(mask: u8) -> bool {
    (byte(IDX_FLAGS) & mask) != 0
}

fn set_flag(mask: u8, enabled: bool) {
    let mut flags = byte(IDX_FLAGS);
    if enabled {
        flags |= mask;
    } else {
        flags &= !mask;
    }
    set_byte(IDX_FLAGS, flags);
}

fn get_setting(qsid: u16, out: &mut [u8]) -> Option<usize> {
    let value = qsid_value(qsid)?;
    let width = qsid_width(qsid)?;
    if out.len() < width {
        return None;
    }
    out[..width].fill(0);
    out[0] = value;
    Some(width)
}

fn qsid_width(qsid: u16) -> Option<usize> {
    match qsid {
        120..=152 | 300..=315 | 317..=326 => Some(1),
        316 => Some(2),
        _ => None,
    }
}

fn set_setting(qsid: u16, data: &[u8]) -> bool {
    let value = match data.first() {
        Some(value) => *value,
        None => return false,
    };
    match qsid {
        120 => set_byte(IDX_LEFT_BALL_DPI, value.min(15)),
        121 => set_byte(IDX_RIGHT_BALL_DPI, value.min(15)),
        122 => set_byte(IDX_LEFT_TOUCH_DPI, value.min(9)),
        123 => set_byte(IDX_RIGHT_TOUCH_DPI, value.min(9)),
        124 => set_byte(IDX_LEFT_SNIPER_SENS, value.max(1)),
        125 => set_byte(IDX_LEFT_SCROLL_SENS, value.max(1)),
        126 => set_byte(IDX_LEFT_TEXT_SENS, value.max(1)),
        127 => set_byte(IDX_RIGHT_SNIPER_SENS, value.max(1)),
        128 => set_byte(IDX_RIGHT_SCROLL_SENS, value.max(1)),
        129 => set_byte(IDX_RIGHT_TEXT_SENS, value.max(1)),
        130 => set_byte(IDX_LEFT_BALL_AXIS, value.min(3)),
        131 => set_byte(IDX_RIGHT_BALL_AXIS, value.min(3)),
        132 => set_byte(IDX_LEFT_TOUCH_AXIS, value.min(3)),
        133 => set_byte(IDX_RIGHT_TOUCH_AXIS, value.min(3)),
        134 => set_byte(IDX_LEFT_MODE, value.min(3)),
        135 => set_byte(IDX_RIGHT_MODE, value.min(3)),
        136 => set_flag(FLAG_LEFT_INVERT_SCROLL, value != 0),
        137 => set_flag(FLAG_LEFT_ACCELERATION, value != 0),
        138 => set_flag(FLAG_RIGHT_INVERT_SCROLL, value != 0),
        139 => set_flag(FLAG_RIGHT_ACCELERATION, value != 0),
        140 => set_flag(FLAG_LEFT_STICKY, value != 0),
        141 => set_flag(FLAG_RIGHT_STICKY, value != 0),
        142 => set_auto_flag(0, value != 0),
        143 => set_byte(IDX_AUTO_LAYER, value.min(15)),
        144 => set_auto_flag(1, value != 0),
        145 => set_auto_flag(2, value != 0),
        146 => set_auto_flag(3, value != 0),
        147 => set_flag(FLAG_LEFT_INVERT_TEXT, value != 0),
        148 => set_flag(FLAG_RIGHT_INVERT_TEXT, value != 0),
        149 => set_module_selection(ModuleSide::Left, value),
        150 => set_module_selection(ModuleSide::Right, value),
        151 => set_touch_gestures_enabled(ModuleSide::Left, value != 0),
        152 => set_touch_gestures_enabled(ModuleSide::Right, value != 0),
        300..=315 => set_layer_color_index((qsid - 300) as u8, value.min(24)),
        316 => set_byte(IDX_LED_BRIGHTNESS, value),
        317 => set_byte(IDX_LED_TIMEOUT_SEC, value),
        318..=322 => set_bt_profile_color_index((qsid - 318) as u8, value.min(24)),
        323 => set_byte(IDX_SLEEP_TIMEOUT, value.min(9)),
        324 => set_byte(IDX_AUTO_LAYER_TIMEOUT, value.min(5)),
        325 => set_byte(IDX_LEFT_ENCODER_INTERVAL, value.min(9)),
        326 => set_byte(IDX_RIGHT_ENCODER_INTERVAL, value.min(9)),
        _ => return false,
    }
    publish_settings_snapshot();
    SETTINGS_CHANGED.signal(());
    true
}

fn qsid_value(qsid: u16) -> Option<u8> {
    Some(match qsid {
        120 => byte(IDX_LEFT_BALL_DPI),
        121 => byte(IDX_RIGHT_BALL_DPI),
        122 => byte(IDX_LEFT_TOUCH_DPI),
        123 => byte(IDX_RIGHT_TOUCH_DPI),
        124 => byte(IDX_LEFT_SNIPER_SENS),
        125 => byte(IDX_LEFT_SCROLL_SENS),
        126 => byte(IDX_LEFT_TEXT_SENS),
        127 => byte(IDX_RIGHT_SNIPER_SENS),
        128 => byte(IDX_RIGHT_SCROLL_SENS),
        129 => byte(IDX_RIGHT_TEXT_SENS),
        130 => byte(IDX_LEFT_BALL_AXIS),
        131 => byte(IDX_RIGHT_BALL_AXIS),
        132 => byte(IDX_LEFT_TOUCH_AXIS),
        133 => byte(IDX_RIGHT_TOUCH_AXIS),
        134 => byte(IDX_LEFT_MODE),
        135 => byte(IDX_RIGHT_MODE),
        136 => flag(FLAG_LEFT_INVERT_SCROLL) as u8,
        137 => flag(FLAG_LEFT_ACCELERATION) as u8,
        138 => flag(FLAG_RIGHT_INVERT_SCROLL) as u8,
        139 => flag(FLAG_RIGHT_ACCELERATION) as u8,
        140 => flag(FLAG_LEFT_STICKY) as u8,
        141 => flag(FLAG_RIGHT_STICKY) as u8,
        142 => auto_flag(0) as u8,
        143 => byte(IDX_AUTO_LAYER),
        144 => auto_flag(1) as u8,
        145 => auto_flag(2) as u8,
        146 => auto_flag(3) as u8,
        147 => flag(FLAG_LEFT_INVERT_TEXT) as u8,
        148 => flag(FLAG_RIGHT_INVERT_TEXT) as u8,
        149 => module_selection_value(ModuleSide::Left),
        150 => module_selection_value(ModuleSide::Right),
        151 => touch_gestures_enabled(ModuleSide::Left) as u8,
        152 => touch_gestures_enabled(ModuleSide::Right) as u8,
        300..=315 => layer_color_index((qsid - 300) as u8),
        316 => byte(IDX_LED_BRIGHTNESS),
        317 => byte(IDX_LED_TIMEOUT_SEC),
        318..=322 => bt_profile_color_index((qsid - 318) as u8),
        323 => sleep_timeout_index(),
        324 => byte(IDX_AUTO_LAYER_TIMEOUT).min(5),
        325 => byte(IDX_LEFT_ENCODER_INTERVAL).min(9),
        326 => byte(IDX_RIGHT_ENCODER_INTERVAL).min(9),
        _ => return None,
    })
}

fn serialize() -> VialDeviceSettingsData {
    ensure_initialized();
    let mut data = VialDeviceSettingsData::empty();
    data.len = SETTINGS_STORAGE_LEN as u8;
    data.data[0] = VERSION;
    data.data[1] = (byte(IDX_LEFT_MODE).min(3) & 0x03)
        | ((byte(IDX_RIGHT_MODE).min(3) & 0x03) << 2)
        | ((byte(IDX_AUTO_LAYER).min(15) & 0x0f) << 4);
    data.data[2] = (byte(IDX_LEFT_BALL_AXIS).min(3) & 0x03)
        | ((byte(IDX_RIGHT_BALL_AXIS).min(3) & 0x03) << 2)
        | ((byte(IDX_LEFT_TOUCH_AXIS).min(3) & 0x03) << 4)
        | ((byte(IDX_RIGHT_TOUCH_AXIS).min(3) & 0x03) << 6);
    data.data[3] =
        (byte(IDX_LEFT_BALL_DPI).min(15) & 0x0f) | ((byte(IDX_RIGHT_BALL_DPI).min(15) & 0x0f) << 4);
    data.data[4] =
        (byte(IDX_LEFT_TOUCH_DPI).min(9) & 0x0f) | ((byte(IDX_RIGHT_TOUCH_DPI).min(9) & 0x0f) << 4);
    data.data[5] = byte(IDX_LEFT_SCROLL_SENS);
    data.data[6] = byte(IDX_LEFT_SNIPER_SENS);
    data.data[7] = byte(IDX_LEFT_TEXT_SENS);
    data.data[8] = byte(IDX_RIGHT_SCROLL_SENS);
    data.data[9] = byte(IDX_RIGHT_SNIPER_SENS);
    data.data[10] = byte(IDX_RIGHT_TEXT_SENS);
    data.data[11] = byte(IDX_FLAGS);
    data.data[12] = byte(IDX_AUTO_FLAGS);
    data.data[13] = byte(IDX_LED_BRIGHTNESS);
    data.data[14] = byte(IDX_LED_TIMEOUT_SEC);

    let mut i = 0u8;
    while i < 16 {
        pack_color(&mut data.data, 15, i, layer_color_index(i));
        i += 1;
    }
    while i < 21 {
        pack_color(&mut data.data, 15, i, bt_profile_color_index(i - 16));
        i += 1;
    }
    data.data[29] = sleep_timeout_index() | ((byte(IDX_LEFT_ENCODER_INTERVAL).min(9) & 0x0f) << 4);
    data.data[30] = byte(IDX_AUTO_LAYER_TIMEOUT).min(5)
        | ((byte(IDX_RIGHT_ENCODER_INTERVAL).min(9) & 0x0f) << 4);
    data.data[31] = byte(IDX_MODULE_SELECT) & MODULE_SELECT_MASK;
    data
}

fn deserialize(data: &[u8]) {
    if data.len() != SETTINGS_STORAGE_LEN
        && data.len() != SETTINGS_STORAGE_LEN_V3_LEGACY_31
        && data.len() != SETTINGS_STORAGE_LEN_V3_LEGACY_30
        && data.len() != SETTINGS_STORAGE_LEN_V3_LEGACY_29
    {
        ensure_initialized();
        return;
    }
    ensure_initialized();
    let previous_left_module = module_selection_value(ModuleSide::Left);
    let previous_right_module = module_selection_value(ModuleSide::Right);
    let previous_sleep_timeout = sleep_timeout_index();
    for (idx, default) in DEFAULTS.iter().enumerate() {
        SETTINGS[idx].store(*default, Ordering::Relaxed);
    }

    let version = data[0];
    if version != VERSION
        && version != 7
        && version != 6
        && version != 5
        && version != 4
        && version != 3
    {
        ensure_initialized();
        return;
    }

    SETTINGS[IDX_LEFT_MODE].store(data[1] & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_MODE].store((data[1] >> 2) & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_AUTO_LAYER].store((data[1] >> 4) & 0x0f, Ordering::Relaxed);
    SETTINGS[IDX_LEFT_BALL_AXIS].store(data[2] & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_BALL_AXIS].store((data[2] >> 2) & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_LEFT_TOUCH_AXIS].store((data[2] >> 4) & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_TOUCH_AXIS].store((data[2] >> 6) & 0x03, Ordering::Relaxed);
    SETTINGS[IDX_LEFT_BALL_DPI].store(data[3] & 0x0f, Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_BALL_DPI].store((data[3] >> 4) & 0x0f, Ordering::Relaxed);
    SETTINGS[IDX_LEFT_TOUCH_DPI].store((data[4] & 0x0f).min(9), Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_TOUCH_DPI].store(((data[4] >> 4) & 0x0f).min(9), Ordering::Relaxed);
    SETTINGS[IDX_LEFT_SCROLL_SENS].store(data[5].max(1), Ordering::Relaxed);
    SETTINGS[IDX_LEFT_SNIPER_SENS].store(data[6].max(1), Ordering::Relaxed);
    SETTINGS[IDX_LEFT_TEXT_SENS].store(data[7].max(1), Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_SCROLL_SENS].store(data[8].max(1), Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_SNIPER_SENS].store(data[9].max(1), Ordering::Relaxed);
    SETTINGS[IDX_RIGHT_TEXT_SENS].store(data[10].max(1), Ordering::Relaxed);
    SETTINGS[IDX_FLAGS].store(data[11], Ordering::Relaxed);
    let auto_flags = if version == VERSION || version == 7 {
        data[12]
    } else {
        data[12] & !AUTO_FLAG_TOUCH_GESTURES_MASK
    };
    SETTINGS[IDX_AUTO_FLAGS].store(auto_flags, Ordering::Relaxed);
    SETTINGS[IDX_LED_BRIGHTNESS].store(data[13], Ordering::Relaxed);
    SETTINGS[IDX_LED_TIMEOUT_SEC].store(data[14], Ordering::Relaxed);

    let mut i = 0u8;
    while i < 16 {
        set_layer_color_index(i, unpack_color(data, 15, i).min(24));
        i += 1;
    }
    while i < 21 {
        set_bt_profile_color_index(i - 16, unpack_color(data, 15, i).min(24));
        i += 1;
    }
    if data.len() >= SETTINGS_STORAGE_LEN_V3_LEGACY_30 {
        SETTINGS[IDX_SLEEP_TIMEOUT].store((data[29] & 0x0f).min(9), Ordering::Relaxed);
        if version == VERSION {
            SETTINGS[IDX_LEFT_ENCODER_INTERVAL].store((data[29] >> 4).min(9), Ordering::Relaxed);
        }
    }
    if data.len() >= SETTINGS_STORAGE_LEN_V3_LEGACY_31 {
        SETTINGS[IDX_AUTO_LAYER_TIMEOUT].store((data[30] & 0x0f).min(5), Ordering::Relaxed);
        if version == VERSION {
            SETTINGS[IDX_RIGHT_ENCODER_INTERVAL].store((data[30] >> 4).min(9), Ordering::Relaxed);
        }
    }
    if data.len() == SETTINGS_STORAGE_LEN && (version == VERSION || version == 6 || version == 5) {
        SETTINGS[IDX_MODULE_SELECT].store(data[31] & MODULE_SELECT_MASK, Ordering::Relaxed);
    } else if data.len() == SETTINGS_STORAGE_LEN && version == 4 {
        SETTINGS[IDX_MODULE_SELECT].store(v4_module_flags_to_select(data[31]), Ordering::Relaxed);
    }
    if module_selection_value(ModuleSide::Left) != previous_left_module {
        signal_module_selection_changed(ModuleSide::Left);
    }
    if module_selection_value(ModuleSide::Right) != previous_right_module {
        signal_module_selection_changed(ModuleSide::Right);
    }
    if sleep_timeout_index() != previous_sleep_timeout {
        SETTINGS_CHANGED.signal(());
    }
    publish_settings_snapshot();
}

fn module_selection_value(side: ModuleSide) -> u8 {
    let shift = match side {
        ModuleSide::Left => 0,
        ModuleSide::Right => 2,
    };
    (byte(IDX_MODULE_SELECT) >> shift) & 0x03
}

fn module_selection_from_value(value: u8) -> ModuleSelection {
    match value {
        MODULE_SELECT_ENCODER => ModuleSelection::Encoder,
        MODULE_SELECT_BALL => ModuleSelection::Ball,
        MODULE_SELECT_TOUCH => ModuleSelection::Touch,
        _ => ModuleSelection::None,
    }
}

fn set_module_selection(side: ModuleSide, value: u8) {
    set_module_selection_local(side, value.min(MODULE_SELECT_TOUCH));
}

fn set_module_selection_local(side: ModuleSide, value: u8) {
    let shift = match side {
        ModuleSide::Left => 0,
        ModuleSide::Right => 2,
    };
    let previous = module_selection_value(side);
    let value = value.min(MODULE_SELECT_TOUCH);
    if previous == value {
        return;
    }
    let mut select = byte(IDX_MODULE_SELECT) & !(0x03 << shift);
    select |= (value & 0x03) << shift;
    set_byte(IDX_MODULE_SELECT, select & MODULE_SELECT_MASK);
    signal_module_selection_changed(side);
}

fn signal_module_selection_changed(side: ModuleSide) {
    ACTIVE_POINTING_MODULE.store(POINTING_MODULE_NONE, Ordering::Relaxed);
    match side {
        ModuleSide::Left => {
            LEFT_BALL_SELECTION_CHANGED.signal(());
            LEFT_TOUCH_SELECTION_CHANGED.signal(());
        }
        ModuleSide::Right => {
            RIGHT_BALL_SELECTION_CHANGED.signal(());
            RIGHT_TOUCH_SELECTION_CHANGED.signal(());
        }
    }
}

fn v4_module_flags_to_select(flags: u8) -> u8 {
    let left_ball = (flags & (1 << 0)) != 0;
    let left_touch = (flags & (1 << 1)) != 0;
    let right_ball = (flags & (1 << 2)) != 0;
    let right_touch = (flags & (1 << 3)) != 0;
    let left = if left_ball && left_touch {
        MODULE_SELECT_TOUCH
    } else if left_ball {
        MODULE_SELECT_BALL
    } else if left_touch {
        MODULE_SELECT_TOUCH
    } else {
        MODULE_SELECT_NONE
    };
    let right = if right_ball && right_touch {
        MODULE_SELECT_BALL
    } else if right_ball {
        MODULE_SELECT_BALL
    } else if right_touch {
        MODULE_SELECT_TOUCH
    } else {
        MODULE_SELECT_NONE
    };
    (left << 0) | (right << 2)
}

fn auto_flag(bit: u8) -> bool {
    (byte(IDX_AUTO_FLAGS) & (1 << bit)) != 0
}

fn set_auto_flag(bit: u8, enabled: bool) {
    let mut flags = byte(IDX_AUTO_FLAGS);
    if enabled {
        flags |= 1 << bit;
    } else {
        flags &= !(1 << bit);
    }
    set_byte(IDX_AUTO_FLAGS, flags);
}

fn set_touch_gestures_enabled(side: ModuleSide, enabled: bool) {
    let mask = match side {
        ModuleSide::Left => AUTO_FLAG_TOUCH_GESTURES_LEFT,
        ModuleSide::Right => AUTO_FLAG_TOUCH_GESTURES_RIGHT,
    };
    let mut flags = byte(IDX_AUTO_FLAGS);
    if enabled {
        flags |= mask;
    } else {
        flags &= !mask;
    }
    set_byte(IDX_AUTO_FLAGS, flags);
}

const fn set_default_layer_color(
    mut data: [u8; SETTINGS_LEN],
    layer: usize,
    value: u8,
) -> [u8; SETTINGS_LEN] {
    let bit = layer * 5;
    let byte_idx = IDX_LAYER_COLORS_PACKED + bit / 8;
    let shift = bit % 8;
    let raw = (value as u16 & 0x1f) << shift;
    data[byte_idx] |= raw as u8;
    if shift > 3 {
        data[byte_idx + 1] |= (raw >> 8) as u8;
    }
    data
}

fn layer_color_index(layer: u8) -> u8 {
    if layer >= 16 {
        return 0;
    }
    let bit = usize::from(layer) * 5;
    let byte_idx = IDX_LAYER_COLORS_PACKED + bit / 8;
    let shift = bit % 8;
    let mut raw = u16::from(byte(byte_idx)) >> shift;
    if shift > 3 {
        raw |= u16::from(byte(byte_idx + 1)) << (8 - shift);
    }
    (raw as u8) & 0x1f
}

fn set_layer_color_index(layer: u8, value: u8) {
    if layer >= 16 {
        return;
    }
    let bit = usize::from(layer) * 5;
    let byte_idx = IDX_LAYER_COLORS_PACKED + bit / 8;
    let shift = bit % 8;
    let mask = 0x1fu16 << shift;
    let raw = (u16::from(value) & 0x1f) << shift;

    let mut combined = u16::from(byte(byte_idx));
    if shift > 3 {
        combined |= u16::from(byte(byte_idx + 1)) << 8;
    }
    combined = (combined & !mask) | raw;
    set_byte(byte_idx, combined as u8);
    if shift > 3 {
        set_byte(byte_idx + 1, (combined >> 8) as u8);
    }
}

fn bt_profile_color_index(profile: u8) -> u8 {
    if profile >= 5 {
        return 0;
    }
    byte(IDX_BT_PROFILE_COLORS + usize::from(profile)).min(24)
}

fn set_bt_profile_color_index(profile: u8, value: u8) {
    if profile >= 5 {
        return;
    }
    set_byte(IDX_BT_PROFILE_COLORS + usize::from(profile), value.min(24));
}

fn pack_color(data: &mut [u8], base: usize, index: u8, value: u8) {
    let bit = usize::from(index) * 5;
    let byte_idx = base + bit / 8;
    let shift = bit % 8;
    let raw = (u16::from(value.min(24)) & 0x1f) << shift;
    data[byte_idx] |= raw as u8;
    if shift > 3 {
        data[byte_idx + 1] |= (raw >> 8) as u8;
    }
}

fn unpack_color(data: &[u8], base: usize, index: u8) -> u8 {
    let bit = usize::from(index) * 5;
    let byte_idx = base + bit / 8;
    let shift = bit % 8;
    let mut raw = u16::from(data[byte_idx]) >> shift;
    if shift > 3 {
        raw |= u16::from(data[byte_idx + 1]) << (8 - shift);
    }
    (raw as u8) & 0x1f
}

fn palette(index: u8) -> Rgb {
    match index {
        1 => Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        2 => Rgb { r: 255, g: 0, b: 0 },
        3 => Rgb {
            r: 255,
            g: 64,
            b: 0,
        },
        4 => Rgb {
            r: 218,
            g: 165,
            b: 32,
        },
        5 => Rgb {
            r: 255,
            g: 215,
            b: 0,
        },
        6 => Rgb {
            r: 255,
            g: 255,
            b: 0,
        },
        7 => Rgb {
            r: 128,
            g: 255,
            b: 0,
        },
        8 => Rgb { r: 0, g: 255, b: 0 },
        9 => Rgb { r: 0, g: 128, b: 0 },
        10 => Rgb {
            r: 0,
            g: 255,
            b: 128,
        },
        11 => Rgb {
            r: 0,
            g: 224,
            b: 192,
        },
        12 => Rgb {
            r: 0,
            g: 128,
            b: 128,
        },
        13 => Rgb {
            r: 0,
            g: 255,
            b: 255,
        },
        14 => Rgb {
            r: 0,
            g: 128,
            b: 255,
        },
        15 => Rgb {
            r: 0,
            g: 191,
            b: 255,
        },
        16 => Rgb { r: 0, g: 0, b: 255 },
        17 => Rgb {
            r: 75,
            g: 0,
            b: 130,
        },
        18 => Rgb {
            r: 128,
            g: 0,
            b: 255,
        },
        19 => Rgb {
            r: 255,
            g: 0,
            b: 255,
        },
        20 => Rgb {
            r: 255,
            g: 64,
            b: 128,
        },
        21 => Rgb {
            r: 255,
            g: 96,
            b: 80,
        },
        22 => Rgb {
            r: 255,
            g: 128,
            b: 114,
        },
        23 => Rgb {
            r: 255,
            g: 180,
            b: 120,
        },
        24 => Rgb {
            r: 255,
            g: 128,
            b: 0,
        },
        _ => Rgb { r: 0, g: 0, b: 0 },
    }
}

const SETTING_KEYS: [u16; 60] = [
    120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138,
    139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 300, 301, 302, 303, 304,
    305, 306, 307, 308, 309, 310, 311, 312, 313, 314, 315, 316, 317, 318, 319, 320, 321, 322, 323,
    324, 325, 326,
];
