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

use ambre_commands::{Key, KeyBinding, KeyState, Modifiers};
use leptos::prelude::*;

pub use ambre_core::{Availability, Toggle};

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
    toggled: Rc<HashMap<ambre_commands::CommandId, RwSignal<Toggle>>>,
    /// Per-command availability (for example Undo/Redo), keyed like
    /// `toggled`. Initially `Enabled`: a command is actionable until the Logic
    /// thread reports otherwise (`TrameEvent::UndoState`, ...).
    enabled: Rc<HashMap<ambre_commands::CommandId, RwSignal<Availability>>>,
}

impl CommandStates {
    /// Create a store with a toggle signal (initially `Off`) and an
    /// availability signal (initially `Enabled`) for each id. Must run inside a Leptos reactive
    /// owner (for example the mount closure).
    ///
    /// Takes identities rather than a slice so an app passes them straight from
    /// its command table (`ambre-spec-commands.md`, UI-THREAD-019) without
    /// building a second list beside it.
    pub fn new(ids: impl IntoIterator<Item = ambre_commands::CommandId>) -> Self {
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
    pub fn toggled_signal(&self, id: ambre_commands::CommandId) -> Option<RwSignal<Toggle>> {
        self.toggled.get(&id).copied()
    }

    /// Push the authoritative toggle state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_toggled(&self, id: ambre_commands::CommandId, toggle: Toggle) {
        if let Some(sig) = self.toggled.get(&id) {
            sig.set(toggle);
        }
    }

    /// The reactive enabled signal for `id`, for a chrome view to subscribe to
    /// (for example a button's `disabled` / `aria-disabled`). `None` if the id was not
    /// registered.
    pub fn enabled_signal(&self, id: ambre_commands::CommandId) -> Option<RwSignal<Availability>> {
        self.enabled.get(&id).copied()
    }

    /// Push the authoritative enabled state for `id`. Called by the dispatcher
    /// from the inbound `UiEvent` stream -- the single writer.
    pub fn set_enabled(&self, id: ambre_commands::CommandId, availability: Availability) {
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
    Some(KeyBinding::new(modifiers_of(ev), Key::from_dom(&ev.key())?))
}

/// The modifier set of a DOM keyboard event, the control and the command keys
/// folded into the primary accelerator (`UI-THREAD-028`).
pub(crate) fn modifiers_of(ev: &web_sys::KeyboardEvent) -> Modifiers {
    Modifiers::NONE
        .ctrl_if(KeyState::from_bool(ev.ctrl_key()))
        .alt_if(KeyState::from_bool(ev.alt_key()))
        .shift_if(KeyState::from_bool(ev.shift_key()))
        .meta_if(KeyState::from_bool(ev.meta_key()))
}
