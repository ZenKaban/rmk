use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};
#[cfg(feature = "controller")]
use {rmk_types::action::KeyAction, rmk_types::led_indicator::LedIndicator, rmk_types::modifier::ModifierCombination};

use crate::input_device::rotary_encoder::Direction;

/// Raw events from input devices and keyboards
///
/// This should be as close to the raw output of the devices as possible.
/// The input processors receives it, processes it,
/// and then converts it to the final keyboard/mouse report.
#[non_exhaustive]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Event {
    /// Keyboard event
    Key(KeyboardEvent),
    /// Multi-touch touchpad
    Touchpad(TouchpadEvent),
    /// Joystick, suppose we have x,y,z axes for this joystick
    Joystick([AxisEvent; 3]),
    /// An AxisEvent in a stream of events. The receiver should keep receiving events until it receives [`Event::Eos`] event.
    AxisEventStream(AxisEvent),
    /// Battery percentage event
    Battery(u16),
    /// Charging state changed event, true means charging, false means not charging
    ChargingState(bool),
    /// End of the event sequence
    ///
    /// This is used with [`Event::AxisEventStream`] to indicate the end of the event sequence.
    Eos,
    /// Custom event
    Custom([u8; 16]),
}

/// `KeyboardEvent` is the event whose `KeyAction` is stored in the keymap.
///
/// `KeyboardEvent` is different from events from pointing devices,
/// events from pointing devices are processed directly by the corresponding processors,
/// while `KeyboardEvent` is processed by the keyboard with the keymap.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyboardEvent {
    pub(crate) pressed: bool,
    pub(crate) pos: KeyboardEventPos,
}

impl KeyboardEvent {
    pub fn key(row: u8, col: u8, pressed: bool) -> Self {
        Self {
            pressed,
            pos: KeyboardEventPos::Key(KeyPos { row, col }),
        }
    }

    pub fn rotary_encoder(id: u8, direction: Direction, pressed: bool) -> Self {
        Self {
            pressed,
            pos: KeyboardEventPos::RotaryEncoder(RotaryEncoderPos { id, direction }),
        }
    }

    pub fn pressed(&self) -> bool {
        self.pressed
    }

    pub fn rotary_encoder_id(&self) -> Option<u8> {
        match self.pos {
            KeyboardEventPos::RotaryEncoder(pos) => Some(pos.id),
            _ => None,
        }
    }
}

/// The position of the keyboard event.
///
/// The position can be either a key (row, col), or a rotary encoder (id, direction)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardEventPos {
    Key(KeyPos),
    RotaryEncoder(RotaryEncoderPos),
}

impl KeyboardEventPos {
    pub(crate) fn key_pos(col: u8, row: u8) -> Self {
        Self::Key(KeyPos { row, col })
    }

    pub(crate) fn rotary_encoder_pos(id: u8, direction: Direction) -> Self {
        Self::RotaryEncoder(RotaryEncoderPos { id, direction })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyPos {
    pub row: u8,
    pub col: u8,
}

/// Event for rotary encoder
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RotaryEncoderPos {
    /// The id of the rotary encoder
    pub id: u8,
    /// The direction of the rotary encoder
    pub direction: Direction,
}

/// Event for multi-touch touchpad
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchpadEvent {
    /// Finger slot
    pub finger: u8,
    /// X, Y, Z axes for touchpad
    pub axis: [AxisEvent; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AxisEvent {
    /// The axis event value type, relative or absolute
    pub typ: AxisValType,
    /// The axis name
    pub axis: Axis,
    /// Value of the axis event
    pub value: i16,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AxisValType {
    /// The axis value is relative
    Rel,
    /// The axis value is absolute
    Abs,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Axis {
    X,
    Y,
    Z,
    H,
    V,
    // .. More is allowed
}

/// Event for controllers
#[cfg(feature = "controller")]
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControllerEvent {
    /// Key event and action
    Key(KeyboardEvent, KeyAction),
    /// Battery percent changed
    Battery(u8),
    /// Raw battery ADC sample changed
    BatteryAdc(u16),
    /// Charging state changed, true means charging, false means not charging
    ChargingState(bool),
    /// Request a one-shot battery level indication on keyboard LEDs
    BatteryLevelRequest,
    /// Request a one-shot battery level indication on a split peripheral
    #[cfg(feature = "split")]
    SplitBatteryLevelRequest(usize),
    /// Layer changed
    Layer(u8),
    /// Device settings blob changed
    DeviceSettings([u8; 27]),
    /// Keyboard idle sleep timeout index changed
    SleepTimeout(u8),
    /// Modifier changed
    Modifier(ModifierCombination),
    /// Typing speed
    Wpm(u16),
    /// Usb or Ble connection
    ConnectionType(u8),
    /// Split peripheral connection
    SplitPeripheral(usize, bool),
    /// Split peripheral has no saved peer and is looking for a new pair
    SplitPeripheralUnpaired(usize),
    /// Split central connection
    SplitCentral(bool),
    /// Split central has no saved peer and is looking for a new pair
    SplitCentralUnpaired,
    /// Lock state led indicator
    KeyboardIndicator(LedIndicator),
    /// Sleep state changed
    Sleep(bool),
    /// Keyboard is about to enter system off
    SystemOff,
    /// Ble state changed
    #[cfg(feature = "_ble")]
    BleState(u8, crate::ble::BleState),
    /// Ble profile changed
    #[cfg(feature = "_ble")]
    BleProfile(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    ClearPeer,
}

#[cfg(feature = "controller")]
impl ControllerEvent {
    #[cfg(feature = "_ble")]
    pub fn ble_led_state_code(&self) -> Option<u8> {
        match self {
            ControllerEvent::BleState(_, crate::ble::BleState::Advertising) => Some(0),
            ControllerEvent::BleState(_, crate::ble::BleState::Reconnecting) => Some(1),
            ControllerEvent::BleState(_, crate::ble::BleState::Pairing) => Some(2),
            ControllerEvent::BleState(_, crate::ble::BleState::Connected) => Some(3),
            ControllerEvent::BleState(_, crate::ble::BleState::None) => Some(4),
            _ => None,
        }
    }
}
