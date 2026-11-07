//! The chrome of each document application, fed from the Logic thread's
//! events.
//!
//! Each application owns its event-to-chrome mapping: the command toggles,
//! the formatting toolbar, the toasts and its panels (`ARCH-018`). This
//! module hands each event to every mapping, and names no event variant.

use ambre_chrome::{CommandStates, Toasts};
use ambre_logic::app::UiEvent;
use ambre_toile::chrome as toile_chrome;
use ambre_trame::chrome as trame_chrome;

use crate::host::ProseSignals;

/// The chrome state of each document application that an event can change.
pub(crate) struct ChromeMirror {
    /// Trame's command toggles.
    pub(crate) store: CommandStates,
    /// Toile's command toggles.
    pub(crate) toile_store: CommandStates,
    /// Prose's history availability, read by its context menu.
    pub(crate) prose_signals: ProseSignals,
    /// The active cell's rich style, for the formatting toolbar.
    pub(crate) style_state: trame_chrome::StyleState,
    /// The toasts every application raises.
    pub(crate) toasts: Toasts,
    /// Trame's panels' state.
    pub(crate) trame_panels: trame_chrome::TrameRelay,
    /// Toile's panels' state.
    pub(crate) toile_panels: toile_chrome::ToileRelay,
}

impl ChromeMirror {
    /// Hand one event to the chrome mapping of each application.
    pub(crate) fn mirror(&self, ev: &UiEvent) {
        // Mirror authoritative command-toggle state into the chrome store.
        // The active app owns the event->toggle mapping; the launcher names
        // no content event variant.
        trame_chrome::mirror_event(&self.store, ev);
        // Mirror Toile's Undo/Redo availability into its own command store,
        // so the menu bar disables the two rows from the Logic thread's
        // `UndoState` alone (`UNDO-006`).
        toile_chrome::mirror_event(&self.toile_store, ev);
        // Prose's history availability, into the signals its context
        // menu reads (`UNDO-006`).
        self.prose_signals.mirror_event(ev);
        // Mirror the active cell's rich style into the formatting toolbar
        // (`trame-cell-styles`). A no-op for non-style events.
        trame_chrome::apply_style_event(&self.style_state, ev);
        // Surface Trame's user-facing notifications (rejections, validation
        // errors, sheet-lifecycle errors, sort/filter completion) as toasts.
        // The app owns the event->toast mapping; the launcher names no variant.
        trame_chrome::notify_event(&self.toasts, ev);
        // Surface Toile's interop outcomes -- the lossy-import report, a
        // refused file, a finished export -- as toasts (`toile-import-export`).
        // The app owns the event->toast mapping; the launcher names no variant.
        toile_chrome::notify_event(&self.toasts, ev);
        // Prose's conversion reports, the same way (`prose-import-export`).
        ambre_prose::chrome::notify_event(&self.toasts, ev);
        // Murmure's root of a copy not reached, the same way.
        ambre_murmure::chrome::notify_event(&self.toasts, ev);
        // Each application fills its own panels from its own events
        // (`ARCH-018`). The launcher names no variant.
        self.trame_panels.absorb(ev);
        self.toile_panels.absorb(ev);
    }
}
