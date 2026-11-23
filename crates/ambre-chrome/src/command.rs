// ambre-chrome/src/command.rs
// ============================================================================
// Chrome side of the command model (ambre-spec-commands.md):
//
//   CommandStates          reactive store of per-command runtime state, fed
//                          ONLY by the UI-thread dispatcher (authoritative
//                          Logic-thread state). Every chrome view -- toolbar
//                          button, context menu, palette -- reads it; none owns
//                          its own copy, so the trigger is decoupled from the
//                          effect.
//   keybinding_from_event  the keybinding manager's DOM half: translate a
//                          `KeyboardEvent` into a `KeyBinding` for `Keymap`
//                          resolution.
//
// State flows one way: trigger -> StaticKey<Commands> -> LogicCommand -> mutation ->
// UiEvent -> dispatcher -> these signals. The chrome never writes its own
// toggle state optimistically.
// ============================================================================

use ambre_commands::{Key, KeyBinding, KeyState, ModifierFold, ModifierKeys};
use ambre_core::Flags;
use ambre_core::{Commands, StaticKey};
use leptos::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;

use ambre_core::Flag as _;
pub use ambre_core::{Availability, Toggle};

/// Reactive store of command toggle state, keyed by `StaticKey<Commands>`.
///
/// Cheap to clone (an `Rc` to the signal map). Toggle signals are created once
/// (at mount) and live for the app's lifetime; the dispatcher pushes
/// authoritative values via [`set_toggled`], chrome views subscribe via
/// [`toggled_signal`].
///
/// [`set_toggled`]: CommandStates::set_toggled
/// [`toggled_signal`]: CommandStates::toggled_signal
#[derive(Clone)]
pub struct CommandStates {
    toggled: Rc<HashMap<StaticKey<Commands>, RwSignal<Toggle>>>,
    /// Per-command availability (for example Undo/Redo), keyed like
    /// `toggled`. Initially `Enabled`: a command is actionable until the Logic
    /// thread reports otherwise (`TrameEvent::UndoState`, ...).
    enabled: Rc<HashMap<StaticKey<Commands>, RwSignal<Availability>>>,
}

impl CommandStates {
    /// Create a store with a toggle signal (initially `Off`) and an
    /// availability signal (initially `Enabled`) for each id. Must run inside a Leptos reactive
    /// owner (for example the mount closure).
    ///
    /// Takes identities rather than a slice so an app passes them straight from
    /// its command table (`ambre-spec-commands.md`, UI-THREAD-019) without
    /// building a second list beside it.
    pub fn new(ids: impl IntoIterator<Item = StaticKey<Commands>>) -> Self {
        let ids: Vec<_> = ids.into_iter().collect();
        let toggled = ids
            .iter()
            .map(|&id| (id, RwSignal::new(Toggle::Off)))
            .collect();
        let enabled = ids
            .iter()
            .map(|&id| (id, RwSignal::new(Availability::Enabled)))
            .collect();
        Self {
            toggled: Rc::new(toggled),
            enabled: Rc::new(enabled),
        }
    }

    /// The reactive toggle signal for `id`, for a chrome view to subscribe to
    /// (for example a button's `aria-pressed`). `None` if the id was not registered.
    pub fn toggled_signal(&self, id: StaticKey<Commands>) -> Option<RwSignal<Toggle>> {
        self.toggled.get(&id).copied()
    }

    /// Push the authoritative toggle state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_toggled(&self, id: StaticKey<Commands>, toggle: Toggle) {
        if let Some(sig) = self.toggled.get(&id) {
            sig.set(toggle);
        }
    }

    /// The reactive enabled signal for `id`, for a chrome view to subscribe to
    /// (for example a button's `disabled` / `aria-disabled`). `None` if the id was not
    /// registered.
    pub fn enabled_signal(&self, id: StaticKey<Commands>) -> Option<RwSignal<Availability>> {
        self.enabled.get(&id).copied()
    }

    /// Push the authoritative enabled state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_enabled(&self, id: StaticKey<Commands>, availability: Availability) {
        if let Some(sig) = self.enabled.get(&id) {
            sig.set(availability);
        }
    }
}

/// Translate a DOM `keydown`/`keyup` event into a [`KeyBinding`].
///
/// This is where the control key and the command key fold into the primary
/// accelerator. Past this point one chord serves every platform, and no table
/// can name one machine's accelerator (`ambre-spec-commands.md`,
/// `UI-THREAD-028`).
///
/// Returns `None` for keys not modelled by [`Key`] (dead keys, IME, unknown
/// named keys), which the caller treats as "no command chord".
pub fn keybinding_from_event(ev: &web_sys::KeyboardEvent) -> Option<KeyBinding> {
    let mods = modifiers_of(ev);
    let held =
        KeyState::from_bool(ev.shift_key() && (ev.alt_key() || ev.ctrl_key() || ev.meta_key()));
    let key = digit_of_code(&ev.code(), held).or_else(|| Key::from_dom(&ev.key()))?;
    Some(KeyBinding::new(mods, key))
}

/// The digit of the key `code` names, `Digit0` to `Digit9`, while Shift and another modifier are
/// held. Shift changes the character of a digit key on every layout, a symbol on a US keyboard
/// and the digit itself on a French one. So a chord of Shift and a digit reads the key, never
/// the character (`ambre-spec-commands.md`).
fn digit_of_code(code: &str, held: KeyState) -> Option<Key> {
    if held != KeyState::Held {
        return None;
    }
    let digit = code.strip_prefix("Digit")?;
    let mut chars = digit.chars();
    let c = chars.next().filter(char::is_ascii_digit)?;
    chars
        .next()
        .is_none()
        .then(|| Key::Char(ambre_commands::KeyChar::new(c)))
}

/// The modifier set of a DOM keyboard event, the control and the command keys
/// folded into the primary accelerator (`UI-THREAD-028`).
pub(crate) fn modifiers_of(ev: &web_sys::KeyboardEvent) -> Flags<ModifierKeys> {
    ModifierKeys::NONE
        .ctrl_if(KeyState::from_bool(ev.ctrl_key()))
        .alt_if(KeyState::from_bool(ev.alt_key()))
        .shift_if(KeyState::from_bool(ev.shift_key()))
        .meta_if(KeyState::from_bool(ev.meta_key()))
}

#[cfg(test)]
mod digit_tests {
    use super::*;

    /// A chord of Shift, another modifier and a digit reads the digit from the key: `Digit1`
    /// gives `1` whatever the layout types. With Shift alone, or with no digit key, the
    /// character decides.
    #[test]
    fn a_shifted_digit_chord_reads_the_key_and_not_the_character() {
        let one = Some(Key::Char(ambre_commands::KeyChar::new('1')));
        assert_eq!(digit_of_code("Digit1", KeyState::Held), one);
        assert_eq!(digit_of_code("Digit1", KeyState::Released), None);
        assert_eq!(digit_of_code("KeyA", KeyState::Held), None);
        assert_eq!(digit_of_code("Digit", KeyState::Held), None);
        assert_eq!(digit_of_code("Digit12", KeyState::Held), None);
    }
}
