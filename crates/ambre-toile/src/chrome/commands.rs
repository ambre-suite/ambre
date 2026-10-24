// ambre-toile/src/chrome/commands.rs
// ============================================================================
// Chrome half of Toile's command surface: the inbound `UiEvent` mirroring that
// keeps the Undo/Redo affordances honest (`UNDO-006`). The registry itself is
// `crate::ui::commands`, host-compiled so a test reaches a label without a
// browser. This module projects the Logic thread's `UndoState` into the shared
// `CommandStates` store the menu bar reads.
// ============================================================================

use ambre_bus::app::toile::ToileEvent;
use ambre_bus::{AppEvent, UiEvent};
use ambre_chrome::CommandStates;
use leptos::prelude::*;

use crate::ui::menu_bar::{REDO, UNDO};

pub use crate::ui::commands::COMMANDS;

/// The Send toggle signals the Toile chrome subscribes to, resolved once from
/// the launcher's `CommandStates` store. Plain `RwSignal`s, so this is `Copy`
/// and `Send` and may live inside the `<Router>` subtree. The `!Send` store
/// itself, by contrast, is kept outside the reactive tree by the launcher.
#[derive(Clone, Copy)]
pub struct Toggles {
    /// Undo/Redo availability (Loro `UndoManager` stacks), driving the matching
    /// menu rows' `disabled` state. Fed by [`ToileEvent::UndoState`] via
    /// [`mirror_event`].
    pub undo_enabled: RwSignal<bool>,
    pub redo_enabled: RwSignal<bool>,
}

impl Toggles {
    /// Resolve the history-enabled signals from a store built over
    /// [`COMMANDS`]. A command missing from the store reads as disabled.
    pub fn from_store(store: &CommandStates) -> Self {
        let enabled = |id| {
            store
                .enabled_signal(id)
                .unwrap_or_else(|| RwSignal::new(false))
        };
        Self {
            undo_enabled: enabled(UNDO),
            redo_enabled: enabled(REDO),
        }
    }
}

/// Mirror the authoritative Undo/Redo availability from an inbound `UiEvent`
/// into the command store, so the menu entries' `disabled` state reflects the
/// Logic thread. A no-op for every event that is not a Toile `UndoState`; the
/// launcher calls this for every event it relays, naming no Toile variant.
pub fn mirror_event(store: &CommandStates, ev: &UiEvent) {
    if let UiEvent::Viewport {
        event: AppEvent::Toile(ToileEvent::UndoState { can_undo, can_redo }),
        ..
    } = ev
    {
        store.set_enabled(UNDO, ambre_chrome::Availability::enabled_if(*can_undo));
        store.set_enabled(REDO, ambre_chrome::Availability::enabled_if(*can_redo));
    }
}

/// Seed the command store for a freshly mounted Toile board. Undo and Redo
/// start disabled, an untouched board having an empty history, until the first
/// [`ToileEvent::UndoState`] arrives. Called by the launcher right after it
/// builds the store.
pub fn init_command_states(store: &CommandStates) {
    store.set_enabled(UNDO, ambre_chrome::Availability::Disabled);
    store.set_enabled(REDO, ambre_chrome::Availability::Disabled);
}
