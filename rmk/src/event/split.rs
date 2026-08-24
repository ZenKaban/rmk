//! Split keyboard events

use rmk_macro::event;

use super::battery::BatteryStatusEvent;

/// Peripheral connected state changed event
#[event(channel_size = crate::PERIPHERAL_CONNECTED_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_CONNECTED_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_CONNECTED_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralConnectedEvent {
    pub id: usize,
    pub connected: bool,
}

/// Connected to central state changed event
#[event(channel_size = crate::CENTRAL_CONNECTED_EVENT_CHANNEL_SIZE, pubs = crate::CENTRAL_CONNECTED_EVENT_PUB_SIZE, subs = crate::CENTRAL_CONNECTED_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CentralConnectedEvent {
    pub connected: bool,
}

/// Current split-link acquisition state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SplitConnectionState {
    /// The central/peripheral is actively looking for its split peer.
    Searching,
    /// Every required split link is established.
    Connected,
    /// The configured split search window elapsed.
    Idle,
}

/// Split-link acquisition state changed event.
#[event(channel_size = crate::SPLIT_CONNECTION_STATE_EVENT_CHANNEL_SIZE, pubs = crate::SPLIT_CONNECTION_STATE_EVENT_PUB_SIZE, subs = crate::SPLIT_CONNECTION_STATE_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SplitConnectionStateEvent(pub SplitConnectionState);

impl_payload_wrapper!(SplitConnectionStateEvent, SplitConnectionState);

/// Peripheral battery status changed event
#[event(channel_size = crate::PERIPHERAL_BATTERY_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_BATTERY_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_BATTERY_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralBatteryEvent {
    pub id: usize,
    pub state: BatteryStatusEvent,
}

/// Runtime settings packet synced from split central to peripherals.
#[event(channel_size = crate::PERIPHERAL_SETTINGS_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_SETTINGS_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_SETTINGS_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralSettingsEvent(pub [u8; 27]);

/// Ask the keyboard to re-publish its current settings snapshot.
///
/// Unlike [`PeripheralSettingsEvent`], this never crosses the split link: the
/// central raises it locally whenever a peripheral link comes up, because a
/// peripheral that rebooted on its own starts from hardcoded defaults and the
/// central would otherwise stay silent until the next settings edit.
///
/// Only keyboards that own a settings snapshot subscribe to it, so `subs`
/// defaults to 0 and publishing compiles away unless a board opts in.
#[event(channel_size = crate::PERIPHERAL_SETTINGS_REFRESH_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_SETTINGS_REFRESH_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_SETTINGS_REFRESH_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralSettingsRefreshEvent;

/// Request a peripheral battery refresh.
#[cfg(feature = "_ble")]
#[event(channel_size = crate::PERIPHERAL_BATTERY_REFRESH_EVENT_CHANNEL_SIZE, pubs = crate::PERIPHERAL_BATTERY_REFRESH_EVENT_PUB_SIZE, subs = crate::PERIPHERAL_BATTERY_REFRESH_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PeripheralBatteryRefreshEvent;

/// Clear BLE peer information event
#[cfg(feature = "_ble")]
#[event(channel_size = crate::CLEAR_PEER_EVENT_CHANNEL_SIZE, pubs = crate::CLEAR_PEER_EVENT_PUB_SIZE, subs = crate::CLEAR_PEER_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ClearPeerEvent;
