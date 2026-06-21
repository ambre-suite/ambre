// ambre-commands/src/keybinding.rs
// ============================================================================
// Keyboard chords (KeyBinding) and their pure resolution to a CommandId.
//
// A KeyBinding is a normalized, hashable chord: a set of modifiers plus a
// logical key. DOM `KeyboardEvent.key` strings are translated into `Key` at
// the chrome boundary (ambre-chrome); this module stays free of web-sys so it
// builds and tests on the host.
// ============================================================================

/// Modifier set of a chord. A plain struct of bools so it is `Copy + Hash` and
/// constructible in `const` command tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Command key on macOS / Windows key. Kept distinct from `ctrl` so the
    /// chrome can map either to the primary accelerator per platform.
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
        meta: false,
    };
    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
        meta: false,
    };
    pub const SHIFT: Self = Self {
        ctrl: false,
        alt: false,
        shift: true,
        meta: false,
    };
    pub const CTRL_SHIFT: Self = Self {
        ctrl: true,
        alt: false,
        shift: true,
        meta: false,
    };
    pub const CTRL_ALT: Self = Self {
        ctrl: true,
        alt: true,
        shift: false,
        meta: false,
    };
}

impl Default for Modifiers {
    fn default() -> Self {
        Self::NONE
    }
}

/// Logical key of a chord. Letters are normalized to a lowercase `Char` so a
/// chord is shift-state independent except through `Modifiers::shift`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// A printable character, lowercased for ASCII letters at construction.
    Char(char),
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    /// Function key F1..=F24.
    F(u8),
}

impl Key {
    /// Build a character key, lowercasing ASCII letters so 'B' and 'b' chord
    /// identically (the shift state, if any, lives in `Modifiers`).
    pub const fn char(c: char) -> Self {
        Self::Char(c.to_ascii_lowercase())
    }
}

/// A fully-specified keyboard chord, e.g. Ctrl+Shift+B.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyBinding {
    pub mods: Modifiers,
    pub key: Key,
}

impl KeyBinding {
    pub const fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }

    /// `Ctrl + <char>` convenience for the common accelerator shape.
    pub const fn ctrl(c: char) -> Self {
        Self {
            mods: Modifiers::CTRL,
            key: Key::char(c),
        }
    }

    /// `Ctrl + Shift + <char>` convenience.
    pub const fn ctrl_shift(c: char) -> Self {
        Self {
            mods: Modifiers::CTRL_SHIFT,
            key: Key::char(c),
        }
    }
}

/// Human-readable rendering, e.g. "Ctrl+Shift+B". Order is fixed
/// (Ctrl, Alt, Shift, Meta) so equal chords format identically. This is the
/// canonical form; locale-specific symbol substitution (macOS glyphs) is a
/// chrome concern layered on top.
impl core::fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = self.mods;
        if m.ctrl {
            f.write_str("Ctrl+")?;
        }
        if m.alt {
            f.write_str("Alt+")?;
        }
        if m.shift {
            f.write_str("Shift+")?;
        }
        if m.meta {
            f.write_str("Meta+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Enter => f.write_str("Enter"),
            Key::Escape => f.write_str("Esc"),
            Key::Tab => f.write_str("Tab"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Delete => f.write_str("Del"),
            Key::Space => f.write_str("Space"),
            Key::ArrowUp => f.write_str("Up"),
            Key::ArrowDown => f.write_str("Down"),
            Key::ArrowLeft => f.write_str("Left"),
            Key::ArrowRight => f.write_str("Right"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::F(n) => write!(f, "F{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_key_lowercases_ascii_letters() {
        assert_eq!(Key::char('B'), Key::Char('b'));
        assert_eq!(Key::char('b'), Key::Char('b'));
        // Non-letters are untouched.
        assert_eq!(Key::char('/'), Key::Char('/'));
    }

    #[test]
    fn chords_are_shift_state_independent_in_the_key() {
        // The only difference between these two is the modifier set, never the
        // case of the key, so they hash and compare distinctly but cleanly.
        assert_eq!(KeyBinding::ctrl('B'), KeyBinding::ctrl('b'));
        assert_ne!(KeyBinding::ctrl('b'), KeyBinding::ctrl_shift('b'));
    }

    #[test]
    fn display_uses_fixed_modifier_order() {
        assert_eq!(KeyBinding::ctrl_shift('b').to_string(), "Ctrl+Shift+B");
        assert_eq!(KeyBinding::ctrl('s').to_string(), "Ctrl+S");
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::Escape).to_string(),
            "Esc"
        );
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::F(5)).to_string(),
            "F5"
        );
    }
}
