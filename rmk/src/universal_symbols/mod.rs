//! Firmware-native punctuation that follows the active EN/RU host layout.
//!
//! IDs live in the upper half of [`Action::User`](rmk_types::action::Action::User)
//! so they cannot collide with Vial's 64 keyboard-specific user keycodes. New
//! layouts belong in their own file under `layout/`.

mod layout;

pub const USER_TOGGLE: u8 = 0x80;
pub const USER_SYNC: u8 = 0x81;
pub const USER_SET_ENGLISH: u8 = 0x82;
pub const USER_SET_RUSSIAN: u8 = 0x83;
pub const USER_TOGGLE_MACOS: u8 = 0x84;
pub const USER_SYMBOL_START: u8 = 0x90;
pub const USER_SYMBOL_END: u8 = 0xB0;
pub const USER_RUSSIAN_LETTER_START: u8 = 0xB0;
pub const USER_RUSSIAN_LETTER_END: u8 = 0xB4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HostLayout {
    #[default]
    English,
    Russian,
}

impl HostLayout {
    const fn toggled(self) -> Self {
        match self {
            Self::English => Self::Russian,
            Self::Russian => Self::English,
        }
    }

    const fn from_host_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::English),
            1 => Some(Self::Russian),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Platform {
    #[default]
    Pc,
    Mac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Symbol {
    Dot,
    Comma,
    Semicolon,
    Colon,
    Exclamation,
    Question,
    Slash,
    Grave,
    Tilde,
    Apostrophe,
    Quote,
    LeftParenthesis,
    RightParenthesis,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    LessThan,
    GreaterThan,
    Minus,
    Plus,
    Asterisk,
    Equal,
    Hash,
    At,
    Dollar,
    Percent,
    Caret,
    Ampersand,
    Pipe,
    Backslash,
    Underscore,
}

impl Symbol {
    pub(crate) const fn from_user_id(id: u8) -> Option<Self> {
        if id < USER_SYMBOL_START || id >= USER_SYMBOL_END {
            return None;
        }
        Some(match id - USER_SYMBOL_START {
            0 => Self::Dot,
            1 => Self::Comma,
            2 => Self::Semicolon,
            3 => Self::Colon,
            4 => Self::Exclamation,
            5 => Self::Question,
            6 => Self::Slash,
            7 => Self::Grave,
            8 => Self::Tilde,
            9 => Self::Apostrophe,
            10 => Self::Quote,
            11 => Self::LeftParenthesis,
            12 => Self::RightParenthesis,
            13 => Self::LeftBracket,
            14 => Self::RightBracket,
            15 => Self::LeftBrace,
            16 => Self::RightBrace,
            17 => Self::LessThan,
            18 => Self::GreaterThan,
            19 => Self::Minus,
            20 => Self::Plus,
            21 => Self::Asterisk,
            22 => Self::Equal,
            23 => Self::Hash,
            24 => Self::At,
            25 => Self::Dollar,
            26 => Self::Percent,
            27 => Self::Caret,
            28 => Self::Ampersand,
            29 => Self::Pipe,
            30 => Self::Backslash,
            31 => Self::Underscore,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RussianLetter {
    Kha,
    Be,
    Yu,
    HardSign,
}

impl RussianLetter {
    pub(crate) const fn from_user_id(id: u8) -> Option<Self> {
        if id < USER_RUSSIAN_LETTER_START || id >= USER_RUSSIAN_LETTER_END {
            return None;
        }
        Some(match id - USER_RUSSIAN_LETTER_START {
            0 => Self::Kha,
            1 => Self::Be,
            2 => Self::Yu,
            3 => Self::HardSign,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    None,
    SwitchLayout,
    Type(layout::ResolvedStroke),
    TypeRussianLetter(rmk_types::keycode::HidKeyCode),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    layout: HostLayout,
    platform: Platform,
    last_host_layout: Option<HostLayout>,
}

impl State {
    pub(crate) fn handle(&mut self, user_id: u8, host_layout: Option<u8>) -> Option<Command> {
        self.sync_from_host(host_layout);

        let command = match user_id {
            USER_TOGGLE => {
                self.layout = self.layout.toggled();
                Command::SwitchLayout
            }
            USER_SYNC => {
                self.layout = self.layout.toggled();
                Command::None
            }
            USER_SET_ENGLISH => self.set_layout(HostLayout::English),
            USER_SET_RUSSIAN => self.set_layout(HostLayout::Russian),
            USER_TOGGLE_MACOS => {
                self.platform = match self.platform {
                    Platform::Pc => Platform::Mac,
                    Platform::Mac => Platform::Pc,
                };
                Command::None
            }
            id => {
                if let Some(letter) = RussianLetter::from_user_id(id) {
                    match layout::resolve_russian_letter(self.layout, letter) {
                        Some(keycode) => Command::TypeRussianLetter(keycode),
                        None => Command::None,
                    }
                } else {
                    Command::Type(layout::resolve(self.layout, self.platform, Symbol::from_user_id(id)?))
                }
            }
        };
        Some(command)
    }

    pub(crate) const fn platform(&self) -> Platform {
        self.platform
    }

    fn set_layout(&mut self, layout: HostLayout) -> Command {
        if self.layout == layout {
            Command::None
        } else {
            self.layout = layout;
            Command::SwitchLayout
        }
    }

    fn sync_from_host(&mut self, code: Option<u8>) {
        let Some(layout) = code.and_then(HostLayout::from_host_code) else {
            return;
        };
        if self.last_host_layout != Some(layout) {
            self.layout = layout;
            self.last_host_layout = Some(layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::keycode::HidKeyCode;
    use rmk_types::modifier::ModifierCombination;

    use super::*;

    #[test]
    fn stable_symbol_id_range_covers_all_punctuation() {
        for id in USER_SYMBOL_START..USER_SYMBOL_END {
            assert!(Symbol::from_user_id(id).is_some(), "missing user id {id:#04x}");
        }
        assert!(Symbol::from_user_id(USER_SYMBOL_START - 1).is_none());
        assert!(Symbol::from_user_id(USER_SYMBOL_END).is_none());
    }

    #[test]
    fn stable_russian_letter_ids_map_only_in_russian_layout() {
        let expected = [
            HidKeyCode::LeftBracket,
            HidKeyCode::Comma,
            HidKeyCode::Dot,
            HidKeyCode::RightBracket,
        ];
        let mut state = State::default();

        for id in USER_RUSSIAN_LETTER_START..USER_RUSSIAN_LETTER_END {
            assert_eq!(state.handle(id, None), Some(Command::None));
        }
        assert!(RussianLetter::from_user_id(USER_RUSSIAN_LETTER_START - 1).is_none());
        assert!(RussianLetter::from_user_id(USER_RUSSIAN_LETTER_END).is_none());

        assert_eq!(state.handle(USER_SYNC, None), Some(Command::None));
        for (id, keycode) in (USER_RUSSIAN_LETTER_START..USER_RUSSIAN_LETTER_END).zip(expected) {
            assert_eq!(state.handle(id, None), Some(Command::TypeRussianLetter(keycode)));
        }
    }

    #[test]
    fn russian_pc_uses_native_period_and_temporarily_switches_for_brace() {
        let mut state = State::default();
        assert_eq!(state.handle(USER_SYNC, None), Some(Command::None));
        let Command::Type(dot) = state.handle(USER_SYMBOL_START, None).unwrap() else {
            panic!("expected dot stroke");
        };
        assert_eq!(dot.stroke.keycode, HidKeyCode::Slash);
        assert_eq!(dot.stroke.modifiers, ModifierCombination::new());
        assert!(!dot.temporary_english);

        let Command::Type(brace) = state.handle(USER_SYMBOL_START + 15, None).unwrap() else {
            panic!("expected brace stroke");
        };
        assert_eq!(brace.stroke.keycode, HidKeyCode::LeftBracket);
        assert_eq!(brace.stroke.modifiers, ModifierCombination::LSHIFT);
        assert!(brace.temporary_english);
    }

    #[test]
    fn host_layout_sync_only_applies_when_host_value_changes() {
        let mut state = State::default();
        assert_eq!(
            state.handle(USER_SYMBOL_START, Some(0)).unwrap(),
            Command::Type(layout::resolve(HostLayout::English, Platform::Pc, Symbol::Dot))
        );
        assert_eq!(state.handle(USER_SYNC, Some(0)), Some(Command::None));

        let Command::Type(dot) = state.handle(USER_SYMBOL_START, Some(0)).unwrap() else {
            panic!("expected dot stroke");
        };
        assert_eq!(dot.stroke.keycode, HidKeyCode::Slash);

        let Command::Type(dot) = state.handle(USER_SYMBOL_START, Some(1)).unwrap() else {
            panic!("expected dot stroke");
        };
        assert_eq!(dot.stroke.keycode, HidKeyCode::Slash);
        assert!(!dot.temporary_english);
    }

    #[test]
    fn mac_mode_uses_mac_russian_punctuation() {
        let mut state = State::default();
        state.handle(USER_SYNC, None);
        state.handle(USER_TOGGLE_MACOS, None);
        let Command::Type(dot) = state.handle(USER_SYMBOL_START, None).unwrap() else {
            panic!("expected dot stroke");
        };
        assert_eq!(dot.stroke.keycode, HidKeyCode::Kc7);
        assert_eq!(dot.stroke.modifiers, ModifierCombination::LSHIFT);
    }
}
