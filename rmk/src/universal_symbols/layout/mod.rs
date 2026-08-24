mod en;
mod ru;

use rmk_types::keycode::HidKeyCode;
use rmk_types::modifier::ModifierCombination;

use super::{HostLayout, Platform, RussianLetter, Symbol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stroke {
    pub(crate) keycode: HidKeyCode,
    pub(crate) modifiers: ModifierCombination,
}

impl Stroke {
    pub(crate) const fn plain(keycode: HidKeyCode) -> Self {
        Self {
            keycode,
            modifiers: ModifierCombination::new(),
        }
    }

    pub(crate) const fn shifted(keycode: HidKeyCode) -> Self {
        Self {
            keycode,
            modifiers: ModifierCombination::LSHIFT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedStroke {
    pub(crate) stroke: Stroke,
    pub(crate) temporary_english: bool,
}

impl ResolvedStroke {
    const fn current(stroke: Stroke) -> Self {
        Self {
            stroke,
            temporary_english: false,
        }
    }

    const fn english(stroke: Stroke) -> Self {
        Self {
            stroke,
            temporary_english: true,
        }
    }
}

pub(crate) fn resolve(layout: HostLayout, platform: Platform, symbol: Symbol) -> ResolvedStroke {
    match layout {
        HostLayout::English => ResolvedStroke::current(en::stroke(symbol)),
        HostLayout::Russian => ru::stroke(platform, symbol),
    }
}

pub(crate) const fn resolve_russian_letter(layout: HostLayout, letter: RussianLetter) -> Option<HidKeyCode> {
    match layout {
        HostLayout::English => None,
        HostLayout::Russian => Some(ru::letter_keycode(letter)),
    }
}
