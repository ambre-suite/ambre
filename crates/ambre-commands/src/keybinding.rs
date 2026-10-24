// ambre-commands/src/keybinding.rs
// ============================================================================
// Keyboard chords (KeyBinding) and their pure resolution to a CommandId.
//
// A KeyBinding is a normalized, hashable chord: a set of modifiers plus a
// logical key. DOM `KeyboardEvent.key` strings are translated into `Key` at
// the chrome boundary (ambre-chrome). This module stays free of web-sys, so it
// builds and tests on the host.
// ============================================================================

/// Modifier set of a chord. A plain struct of bools so it is `Copy + Hash` and
/// constructible in `const` command tables.
///
/// One accelerator modifier, `primary`, carries both the control key and the
/// command key. The two fold into it where a browser event becomes a chord
/// (`ambre-chrome`). That is the only place they are still apart. A table
/// therefore cannot name one machine's accelerator, so one table serves every
/// platform (`ambre-spec-commands.md`, `UI-THREAD-028`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Modifiers {
    /// The primary accelerator: Ctrl on Windows and Linux, Cmd on macOS.
    pub primary: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        primary: false,
        alt: false,
        shift: false,
    };
    pub const PRIMARY: Self = Self {
        primary: true,
        alt: false,
        shift: false,
    };
    pub const ALT: Self = Self {
        primary: false,
        alt: true,
        shift: false,
    };
    pub const SHIFT: Self = Self {
        primary: false,
        alt: false,
        shift: true,
    };
    pub const PRIMARY_SHIFT: Self = Self {
        primary: true,
        alt: false,
        shift: true,
    };
    pub const PRIMARY_ALT: Self = Self {
        primary: true,
        alt: true,
        shift: false,
    };

    /// Fold a browser event's modifier bits into a chord's modifier set. The
    /// control key and the command key both answer the primary accelerator, so
    /// a table declares one chord and both machines resolve it
    /// (`UI-THREAD-028`).
    ///
    /// The pure seam of the chrome's event translation, so the fold is host
    /// testable without a `KeyboardEvent`.
    pub const fn from_event(ctrl: bool, alt: bool, shift: bool, meta: bool) -> Self {
        Self {
            primary: ctrl || meta,
            alt,
            shift,
        }
    }
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
    /// Carries the legacy clipboard chords: Shift+Insert pastes and
    /// Primary+Insert copies (`ambre-spec-commands.md`, `UI-THREAD-011`).
    Insert,
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
    /// The key that opens a context menu, beside the right Alt.
    ContextMenu,
}

impl Key {
    /// Build a character key, lowercasing ASCII letters so 'B' and 'b' chord
    /// identically (the shift state, if any, lives in `Modifiers`).
    pub const fn char(c: char) -> Self {
        Self::Char(c.to_ascii_lowercase())
    }

    /// The key a DOM `KeyboardEvent.key` names, `None` for a key the set does
    /// not model: a dead key, an IME composition, an unknown named key. The
    /// caller treats it as no chord.
    pub fn from_dom(name: &str) -> Option<Self> {
        Some(match name {
            "Enter" => Self::Enter,
            "Escape" | "Esc" => Self::Escape,
            "Tab" => Self::Tab,
            "Backspace" => Self::Backspace,
            "Delete" | "Del" => Self::Delete,
            " " | "Spacebar" => Self::Space,
            "Insert" => Self::Insert,
            "ArrowUp" => Self::ArrowUp,
            "ArrowDown" => Self::ArrowDown,
            "ArrowLeft" => Self::ArrowLeft,
            "ArrowRight" => Self::ArrowRight,
            "Home" => Self::Home,
            "End" => Self::End,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            "ContextMenu" => Self::ContextMenu,
            other => {
                // Function keys "F1".."F24".
                if let Some(n) = other.strip_prefix('F').and_then(|d| d.parse::<u8>().ok()) {
                    if (1..=24).contains(&n) {
                        return Some(Self::F(n));
                    }
                    return None;
                }
                // Single printable character (letters arrive as "a"/"A").
                let mut chars = other.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None; // multi-char named key we don't model
                }
                Self::char(c)
            }
        })
    }
}

/// A fully-specified keyboard chord, for example Ctrl+Shift+B.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyBinding {
    pub mods: Modifiers,
    pub key: Key,
}

impl KeyBinding {
    pub const fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }

    /// `Primary + <char>` convenience for the common accelerator shape.
    pub const fn primary(c: char) -> Self {
        Self {
            mods: Modifiers::PRIMARY,
            key: Key::char(c),
        }
    }

    /// `Primary + Shift + <char>` convenience.
    pub const fn primary_shift(c: char) -> Self {
        Self {
            mods: Modifiers::PRIMARY_SHIFT,
            key: Key::char(c),
        }
    }

    /// `Shift + <key>` convenience, for the legacy clipboard chords.
    pub const fn shift(key: Key) -> Self {
        Self {
            mods: Modifiers::SHIFT,
            key,
        }
    }
}

/// Human-readable rendering, for example "Ctrl+Shift+B". Order is fixed
/// (Primary, Alt, Shift) so equal chords format identically.
///
/// The primary accelerator is written "Ctrl", which the fold makes true on
/// every platform. A machine whose accelerator is the command key answers the
/// control key as well. Writing the macOS glyph instead is a chrome concern
/// layered on top, and no surface asks for it today.
impl core::fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = self.mods;
        if m.primary {
            f.write_str("Ctrl+")?;
        }
        if m.alt {
            f.write_str("Alt+")?;
        }
        if m.shift {
            f.write_str("Shift+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Enter => f.write_str("Enter"),
            Key::Escape => f.write_str("Esc"),
            Key::Tab => f.write_str("Tab"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Delete => f.write_str("Del"),
            Key::Space => f.write_str("Space"),
            Key::Insert => f.write_str("Insert"),
            Key::ArrowUp => f.write_str("Up"),
            Key::ArrowDown => f.write_str("Down"),
            Key::ArrowLeft => f.write_str("Left"),
            Key::ArrowRight => f.write_str("Right"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::F(n) => write!(f, "F{n}"),
            Key::ContextMenu => f.write_str("Menu"),
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
        assert_eq!(KeyBinding::primary('B'), KeyBinding::primary('b'));
        assert_ne!(KeyBinding::primary('b'), KeyBinding::primary_shift('b'));
    }

    #[test]
    fn the_control_key_and_the_command_key_fold_into_one_chord() {
        // The same chord on a Windows machine and on a Mac: the table declares
        // it once and both events resolve it (`UI-THREAD-028`).
        let control = Modifiers::from_event(true, false, false, false);
        let command = Modifiers::from_event(false, false, false, true);
        assert_eq!(control, command);
        assert_eq!(control, Modifiers::PRIMARY);
        // Alt and Shift stay themselves, and a bare key stays bare.
        assert_eq!(
            Modifiers::from_event(true, false, true, false),
            Modifiers::PRIMARY_SHIFT
        );
        assert_eq!(
            Modifiers::from_event(false, false, false, false),
            Modifiers::NONE
        );
    }

    #[test]
    fn display_uses_fixed_modifier_order() {
        assert_eq!(KeyBinding::primary_shift('b').to_string(), "Ctrl+Shift+B");
        assert_eq!(KeyBinding::primary('s').to_string(), "Ctrl+S");
        assert_eq!(KeyBinding::shift(Key::Insert).to_string(), "Shift+Insert");
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::Escape).to_string(),
            "Esc"
        );
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::F(5)).to_string(),
            "F5"
        );
    }

    #[test]
    fn named_keys_map() {
        assert_eq!(Key::from_dom("Enter"), Some(Key::Enter));
        assert_eq!(Key::from_dom("Escape"), Some(Key::Escape));
        assert_eq!(Key::from_dom("Esc"), Some(Key::Escape));
        assert_eq!(Key::from_dom("Del"), Some(Key::Delete));
        assert_eq!(Key::from_dom("Delete"), Some(Key::Delete));
        assert_eq!(Key::from_dom(" "), Some(Key::Space));
        assert_eq!(Key::from_dom("Spacebar"), Some(Key::Space));
        assert_eq!(Key::from_dom("Insert"), Some(Key::Insert));
        assert_eq!(Key::from_dom("ArrowUp"), Some(Key::ArrowUp));
        assert_eq!(Key::from_dom("PageDown"), Some(Key::PageDown));
    }

    #[test]
    fn function_keys_in_range() {
        assert_eq!(Key::from_dom("F1"), Some(Key::F(1)));
        assert_eq!(Key::from_dom("F24"), Some(Key::F(24)));
    }

    #[test]
    fn function_keys_out_of_range_rejected() {
        assert_eq!(Key::from_dom("F0"), None);
        assert_eq!(Key::from_dom("F25"), None);
    }

    #[test]
    fn single_printable_char_maps() {
        assert_eq!(Key::from_dom("a"), Some(Key::char('a')));
        assert_eq!(Key::from_dom("A"), Some(Key::char('A')));
        assert_eq!(Key::from_dom("="), Some(Key::char('=')));
    }

    #[test]
    fn unmodelled_keys_rejected() {
        // Empty string (dead key / IME), multi-char named keys we don't model.
        assert_eq!(Key::from_dom(""), None);
        assert_eq!(Key::from_dom("MediaPlayPause"), None);
        assert_eq!(Key::from_dom("Unidentified"), None);
    }
}
