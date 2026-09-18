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
// State flows one way: trigger -> CommandId -> LogicCommand -> mutation ->
// UiEvent -> dispatcher -> these signals. The chrome never writes its own
// toggle state optimistically.
// ============================================================================

use std::collections::HashMap;
use std::rc::Rc;

use ambre_commands::{Key, KeyBinding, Modifiers};
use leptos::prelude::*;

/// Reactive store of command toggle state, keyed by `CommandId`.
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
    toggled: Rc<HashMap<ambre_commands::CommandId, RwSignal<bool>>>,
    /// Per-command enabled state (for example Undo/Redo availability), keyed like
    /// `toggled`. Initially `true`: a command is actionable until the Logic
    /// thread reports otherwise (`TrameEvent::UndoState`, ...).
    enabled: Rc<HashMap<ambre_commands::CommandId, RwSignal<bool>>>,
}

impl CommandStates {
    /// Create a store with a toggle signal (initially `false`) and an enabled
    /// signal (initially `true`) for each id. Must run inside a Leptos reactive
    /// owner (for example the mount closure).
    ///
    /// Takes identities rather than a slice so an app passes them straight from
    /// its command table (`ambre-spec-commands.md`, UI-THREAD-019) without
    /// building a second list beside it.
    pub fn new(ids: impl IntoIterator<Item = ambre_commands::CommandId>) -> Self {
        let ids: Vec<_> = ids.into_iter().collect();
        let toggled = ids.iter().map(|&id| (id, RwSignal::new(false))).collect();
        let enabled = ids.iter().map(|&id| (id, RwSignal::new(true))).collect();
        Self {
            toggled: Rc::new(toggled),
            enabled: Rc::new(enabled),
        }
    }

    /// The reactive toggle signal for `id`, for a chrome view to subscribe to
    /// (for example a button's `aria-pressed`). `None` if the id was not registered.
    pub fn toggled_signal(&self, id: ambre_commands::CommandId) -> Option<RwSignal<bool>> {
        self.toggled.get(&id).copied()
    }

    /// Push the authoritative toggle state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_toggled(&self, id: ambre_commands::CommandId, value: bool) {
        if let Some(sig) = self.toggled.get(&id) {
            sig.set(value);
        }
    }

    /// The reactive enabled signal for `id`, for a chrome view to subscribe to
    /// (for example a button's `disabled` / `aria-disabled`). `None` if the id was not
    /// registered.
    pub fn enabled_signal(&self, id: ambre_commands::CommandId) -> Option<RwSignal<bool>> {
        self.enabled.get(&id).copied()
    }

    /// Push the authoritative enabled state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_enabled(&self, id: ambre_commands::CommandId, value: bool) {
        if let Some(sig) = self.enabled.get(&id) {
            sig.set(value);
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
    let mods = Modifiers::from_event(ev.ctrl_key(), ev.alt_key(), ev.shift_key(), ev.meta_key());
    Some(KeyBinding::new(mods, key_from_dom(&ev.key())?))
}

/// Map a DOM `KeyboardEvent.key` string to a logical [`Key`].
fn key_from_dom(s: &str) -> Option<Key> {
    Some(match s {
        "Enter" => Key::Enter,
        "Escape" | "Esc" => Key::Escape,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" | "Del" => Key::Delete,
        " " | "Spacebar" => Key::Space,
        "Insert" => Key::Insert,
        "ArrowUp" => Key::ArrowUp,
        "ArrowDown" => Key::ArrowDown,
        "ArrowLeft" => Key::ArrowLeft,
        "ArrowRight" => Key::ArrowRight,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        other => {
            // Function keys "F1".."F24".
            if let Some(n) = other.strip_prefix('F').and_then(|d| d.parse::<u8>().ok()) {
                if (1..=24).contains(&n) {
                    return Some(Key::F(n));
                }
                return None;
            }
            // Single printable character (letters arrive as "a"/"A").
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-char named key we don't model
            }
            Key::char(c)
        }
    })
}

// -- Tests (host tier) -------------------------------------------------------
// `key_from_dom` is pure (`&str -> Option<Key>`), the host-testable seam of the
// keybinding manager: `keybinding_from_event` is reduced to wrapping it with the
// modifier bits read off a `web_sys::KeyboardEvent` (covered in the wasm tier).
// ambre-chrome compiles on host (leptos CSR + web-sys types), so this runs under
// `cargo test -p ambre-chrome` -- see ambre-spec-testing.md.
#[cfg(test)]
mod tests {
    use super::key_from_dom;
    use ambre_commands::Key;

    #[test]
    fn named_keys_map() {
        assert_eq!(key_from_dom("Enter"), Some(Key::Enter));
        assert_eq!(key_from_dom("Escape"), Some(Key::Escape));
        assert_eq!(key_from_dom("Esc"), Some(Key::Escape));
        assert_eq!(key_from_dom("Del"), Some(Key::Delete));
        assert_eq!(key_from_dom("Delete"), Some(Key::Delete));
        assert_eq!(key_from_dom(" "), Some(Key::Space));
        assert_eq!(key_from_dom("Spacebar"), Some(Key::Space));
        assert_eq!(key_from_dom("Insert"), Some(Key::Insert));
        assert_eq!(key_from_dom("ArrowUp"), Some(Key::ArrowUp));
        assert_eq!(key_from_dom("PageDown"), Some(Key::PageDown));
    }

    #[test]
    fn function_keys_in_range() {
        assert_eq!(key_from_dom("F1"), Some(Key::F(1)));
        assert_eq!(key_from_dom("F24"), Some(Key::F(24)));
    }

    #[test]
    fn function_keys_out_of_range_rejected() {
        assert_eq!(key_from_dom("F0"), None);
        assert_eq!(key_from_dom("F25"), None);
    }

    #[test]
    fn single_printable_char_maps() {
        assert_eq!(key_from_dom("a"), Some(Key::char('a')));
        assert_eq!(key_from_dom("A"), Some(Key::char('A')));
        assert_eq!(key_from_dom("="), Some(Key::char('=')));
    }

    #[test]
    fn unmodelled_keys_rejected() {
        // Empty string (dead key / IME), multi-char named keys we don't model.
        assert_eq!(key_from_dom(""), None);
        assert_eq!(key_from_dom("MediaPlayPause"), None);
        assert_eq!(key_from_dom("Unidentified"), None);
    }
}
