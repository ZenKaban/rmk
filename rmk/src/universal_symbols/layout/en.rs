use rmk_types::keycode::HidKeyCode;

use super::Stroke;
use crate::universal_symbols::Symbol;

pub(super) const fn stroke(symbol: Symbol) -> Stroke {
    match symbol {
        Symbol::Dot => Stroke::plain(HidKeyCode::Dot),
        Symbol::Comma => Stroke::plain(HidKeyCode::Comma),
        Symbol::Semicolon => Stroke::plain(HidKeyCode::Semicolon),
        Symbol::Colon => Stroke::shifted(HidKeyCode::Semicolon),
        Symbol::Exclamation => Stroke::shifted(HidKeyCode::Kc1),
        Symbol::Question => Stroke::shifted(HidKeyCode::Slash),
        Symbol::Slash => Stroke::plain(HidKeyCode::Slash),
        Symbol::Grave => Stroke::plain(HidKeyCode::Grave),
        Symbol::Tilde => Stroke::shifted(HidKeyCode::Grave),
        Symbol::Apostrophe => Stroke::plain(HidKeyCode::Quote),
        Symbol::Quote => Stroke::shifted(HidKeyCode::Quote),
        Symbol::LeftParenthesis => Stroke::shifted(HidKeyCode::Kc9),
        Symbol::RightParenthesis => Stroke::shifted(HidKeyCode::Kc0),
        Symbol::LeftBracket => Stroke::plain(HidKeyCode::LeftBracket),
        Symbol::RightBracket => Stroke::plain(HidKeyCode::RightBracket),
        Symbol::LeftBrace => Stroke::shifted(HidKeyCode::LeftBracket),
        Symbol::RightBrace => Stroke::shifted(HidKeyCode::RightBracket),
        Symbol::LessThan => Stroke::shifted(HidKeyCode::Comma),
        Symbol::GreaterThan => Stroke::shifted(HidKeyCode::Dot),
        Symbol::Minus => Stroke::plain(HidKeyCode::Minus),
        Symbol::Plus => Stroke::shifted(HidKeyCode::Equal),
        Symbol::Asterisk => Stroke::shifted(HidKeyCode::Kc8),
        Symbol::Equal => Stroke::plain(HidKeyCode::Equal),
        Symbol::Hash => Stroke::shifted(HidKeyCode::Kc3),
        Symbol::At => Stroke::shifted(HidKeyCode::Kc2),
        Symbol::Dollar => Stroke::shifted(HidKeyCode::Kc4),
        Symbol::Percent => Stroke::shifted(HidKeyCode::Kc5),
        Symbol::Caret => Stroke::shifted(HidKeyCode::Kc6),
        Symbol::Ampersand => Stroke::shifted(HidKeyCode::Kc7),
        Symbol::Pipe => Stroke::shifted(HidKeyCode::Backslash),
        Symbol::Backslash => Stroke::plain(HidKeyCode::Backslash),
        Symbol::Underscore => Stroke::shifted(HidKeyCode::Minus),
    }
}
