// ambre-commands/src/keybinding.rs
// ============================================================================
// Keyboard chords (KeyBinding) and their pure resolution to a CommandId.
//
// A KeyBinding is a normalized, hashable chord: a set of modifiers plus a
// logical key. DOM `KeyboardEvent.key` strings are translated into `Key` at
// the chrome boundary (ambre-chrome). This module stays free of web-sys, so it
// builds and tests on the host.
// ============================================================================

/// Modifier set of a chord: a set of three named members, `PRIMARY`, `ALT`
/// and `SHIFT`, joined with [`Modifiers::with`]. It is `Copy + Hash` and
/// built in `const` command tables, and no constructor takes a bool by
/// position.
///
/// One accelerator modifier, `PRIMARY`, carries both the control key and the
/// command key. The two fold into it where a browser event becomes a chord
/// (`ambre-chrome`). That is the only place they are still apart. A table
/// therefore cannot name one machine's accelerator, so one table serves every
/// platform (`ambre-spec-commands.md`, `UI-THREAD-028`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self(0);
    /// The primary accelerator: Ctrl on Windows and Linux, Cmd on macOS.
    pub const PRIMARY: Self = Self(1);
    pub const ALT: Self = Self(1 << 1);
    pub const SHIFT: Self = Self(1 << 2);
    pub const PRIMARY_SHIFT: Self = Self::PRIMARY.with(Self::SHIFT);
    pub const PRIMARY_ALT: Self = Self::PRIMARY.with(Self::ALT);
    pub const ALT_SHIFT: Self = Self::ALT.with(Self::SHIFT);

    /// The union of two sets.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The set with each member of `other` removed.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every member of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the primary accelerator is held.
    #[must_use]
    pub const fn primary(self) -> bool {
        self.contains(Self::PRIMARY)
    }

    /// Whether Alt is held.
    #[must_use]
    pub const fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    /// Whether Shift is held.
    #[must_use]
    pub const fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    /// The set with `member` added when `key` is held, unchanged otherwise.
    const fn with_if(self, member: Self, key: KeyState) -> Self {
        match key {
            KeyState::Held => self.with(member),
            KeyState::Released => self,
        }
    }

    /// The set with the primary accelerator added when `key` is held.
    #[must_use]
    pub const fn primary_if(self, key: KeyState) -> Self {
        self.with_if(Self::PRIMARY, key)
    }

    /// The set with Alt added when `key` is held.
    #[must_use]
    pub const fn alt_if(self, key: KeyState) -> Self {
        self.with_if(Self::ALT, key)
    }

    /// The set with Shift added when `key` is held.
    #[must_use]
    pub const fn shift_if(self, key: KeyState) -> Self {
        self.with_if(Self::SHIFT, key)
    }

    /// The set with the primary accelerator added when a browser event
    /// reports the control key held. With [`Modifiers::meta_if`], it folds a
    /// browser event's modifier bits into a chord's set. The control key and
    /// the command key both answer the primary accelerator, so a table
    /// declares one chord and both machines resolve it (`UI-THREAD-028`).
    #[must_use]
    pub const fn ctrl_if(self, key: KeyState) -> Self {
        self.primary_if(key)
    }

    /// The set with the primary accelerator added when a browser event
    /// reports the command key held, the fold of [`Modifiers::ctrl_if`].
    #[must_use]
    pub const fn meta_if(self, key: KeyState) -> Self {
        self.primary_if(key)
    }
}

ambre_core::define_flag! {
    /// Whether one modifier key is down, as a browser event reports it. A
    /// caller names the state, so a builder of [`Modifiers`] never reads a
    /// bare `true` (`ARCH-040`).
    pub enum KeyState {
        /// The key is up.
        Released,
        /// The key is down.
        Held,
    }
}

/// The printable character of a [`Key::Char`]. An ASCII letter is lowercased
/// at construction, so 'B' and 'b' chord identically. The shift state, if
/// any, lives in [`Modifiers`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyChar(char);

/// One constant per ASCII letter, so a match arm names a letter key.
macro_rules! key_letters {
    ($($name:ident $c:literal),* $(,)?) => {
        impl KeyChar {
            $(
                #[doc = concat!("The letter key `", $c, "`.")]
                pub const $name: Self = Self($c);
            )*
        }
    };
}

key_letters! {
    A 'a', B 'b', C 'c', D 'd', E 'e', F 'f', G 'g', H 'h', I 'i', J 'j', K 'k', L 'l', M 'm',
    N 'n', O 'o', P 'p', Q 'q', R 'r', S 's', T 't', U 'u', V 'v', W 'w', X 'x', Y 'y', Z 'z',
}

impl KeyChar {
    /// The key of the character `c`, lowercased for an ASCII letter.
    #[must_use]
    pub const fn new(c: char) -> Self {
        Self(c.to_ascii_lowercase())
    }

    /// The character, lowercased for an ASCII letter.
    #[must_use]
    pub const fn get(self) -> char {
        self.0
    }
}

/// A function key, F1 to F24: the keys a DOM `KeyboardEvent.key` names
/// "F1" to "F24". Its discriminant is its number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum FunctionKey {
    F1 = 1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
}

impl FunctionKey {
    /// Every function key, in the order of its number.
    pub const ALL: [Self; 24] = [
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
        Self::F13,
        Self::F14,
        Self::F15,
        Self::F16,
        Self::F17,
        Self::F18,
        Self::F19,
        Self::F20,
        Self::F21,
        Self::F22,
        Self::F23,
        Self::F24,
    ];

    /// The key a DOM name spells after its "F": "1" to "24".
    fn from_dom_number(digits: &str) -> Option<Self> {
        let n: usize = digits.parse().ok()?;
        n.checked_sub(1).and_then(|at| Self::ALL.get(at)).copied()
    }
}

/// The name a key cap prints: "F10".
impl core::fmt::Display for FunctionKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "F{}", *self as u8)
    }
}

/// Logical key of a chord. Letters are normalized to a lowercase `Char` so a
/// chord is shift-state independent except through `Modifiers::SHIFT`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// A printable character, lowercased for ASCII letters at construction.
    Char(KeyChar),
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
    F(FunctionKey),
    /// The key that opens a context menu, beside the right Alt.
    ContextMenu,
}

impl Key {
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
                if let Some(digits) = other.strip_prefix('F').filter(|d| d.parse::<u8>().is_ok()) {
                    return FunctionKey::from_dom_number(digits).map(Self::F);
                }
                // Single printable character (letters arrive as "a"/"A").
                let mut chars = other.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None; // multi-char named key we don't model
                }
                Self::Char(KeyChar::new(c))
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

    /// `Primary + <key>` convenience for the common accelerator shape.
    pub const fn primary(key: KeyChar) -> Self {
        Self {
            mods: Modifiers::PRIMARY,
            key: Key::Char(key),
        }
    }

    /// `Primary + Shift + <key>` convenience.
    pub const fn primary_shift(key: KeyChar) -> Self {
        Self {
            mods: Modifiers::PRIMARY_SHIFT,
            key: Key::Char(key),
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
        if m.primary() {
            f.write_str("Ctrl+")?;
        }
        if m.alt() {
            f.write_str("Alt+")?;
        }
        if m.shift() {
            f.write_str("Shift+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{}", c.get().to_ascii_uppercase()),
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
            Key::F(n) => n.fmt(f),
            Key::ContextMenu => f.write_str("Menu"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_key_lowercases_ascii_letters() {
        assert_eq!(KeyChar::new('B'), KeyChar::B);
        assert_eq!(KeyChar::new('b'), KeyChar::B);
        assert_eq!(KeyChar::new('/').get(), '/');
    }

    #[test]
    fn chords_are_shift_state_independent_in_the_key() {
        // The only difference between these two is the modifier set, never the
        // case of the key, so they hash and compare distinctly but cleanly.
        assert_eq!(
            KeyBinding::primary(KeyChar::new('B')),
            KeyBinding::primary(KeyChar::B)
        );
        assert_ne!(
            KeyBinding::primary(KeyChar::B),
            KeyBinding::primary_shift(KeyChar::B)
        );
    }

    #[test]
    fn the_control_key_and_the_command_key_fold_into_one_chord() {
        // The same chord on a Windows machine and on a Mac: the table declares
        // it once and both events resolve it (`UI-THREAD-028`).
        let control = Modifiers::NONE
            .ctrl_if(KeyState::Held)
            .meta_if(KeyState::Released);
        let command = Modifiers::NONE
            .ctrl_if(KeyState::Released)
            .meta_if(KeyState::Held);
        assert_eq!(control, command);
        assert_eq!(control, Modifiers::PRIMARY);
        // Alt and Shift stay themselves, and a bare key stays bare.
        assert_eq!(
            Modifiers::NONE
                .ctrl_if(KeyState::Held)
                .shift_if(KeyState::Held),
            Modifiers::PRIMARY_SHIFT
        );
        assert_eq!(
            Modifiers::NONE
                .ctrl_if(KeyState::Released)
                .alt_if(KeyState::Released)
                .shift_if(KeyState::Released)
                .meta_if(KeyState::Released),
            Modifiers::NONE
        );
    }

    /// A set holds each member it was built with, and no other.
    #[test]
    fn a_modifier_set_holds_its_named_members() {
        let mods = Modifiers::PRIMARY.with(Modifiers::SHIFT);
        assert!(mods.primary() && mods.shift() && !mods.alt());
        assert!(mods.contains(Modifiers::PRIMARY_SHIFT));
        assert!(!mods.contains(Modifiers::PRIMARY_ALT));
        assert_eq!(Modifiers::ALT_SHIFT, Modifiers::SHIFT.with(Modifiers::ALT));
        assert_eq!(
            Modifiers::PRIMARY_ALT.without(Modifiers::ALT),
            Modifiers::PRIMARY
        );
        assert_eq!(Modifiers::default(), Modifiers::NONE);
    }

    #[test]
    fn display_uses_fixed_modifier_order() {
        assert_eq!(
            KeyBinding::primary_shift(KeyChar::B).to_string(),
            "Ctrl+Shift+B"
        );
        assert_eq!(KeyBinding::primary(KeyChar::S).to_string(), "Ctrl+S");
        assert_eq!(KeyBinding::shift(Key::Insert).to_string(), "Shift+Insert");
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::Escape).to_string(),
            "Esc"
        );
        assert_eq!(
            KeyBinding::new(Modifiers::NONE, Key::F(FunctionKey::F5)).to_string(),
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
        assert_eq!(Key::from_dom("F1"), Some(Key::F(FunctionKey::F1)));
        assert_eq!(Key::from_dom("F24"), Some(Key::F(FunctionKey::F24)));
    }

    #[test]
    fn function_keys_out_of_range_rejected() {
        assert_eq!(Key::from_dom("F0"), None);
        assert_eq!(Key::from_dom("F25"), None);
    }

    #[test]
    fn single_printable_char_maps() {
        assert_eq!(Key::from_dom("a"), Some(Key::Char(KeyChar::A)));
        assert_eq!(Key::from_dom("A"), Some(Key::Char(KeyChar::A)));
        assert_eq!(Key::from_dom("="), Some(Key::Char(KeyChar::new('='))));
    }

    #[test]
    fn unmodelled_keys_rejected() {
        // Empty string (dead key / IME), multi-char named keys we don't model.
        assert_eq!(Key::from_dom(""), None);
        assert_eq!(Key::from_dom("MediaPlayPause"), None);
        assert_eq!(Key::from_dom("Unidentified"), None);
    }
}
