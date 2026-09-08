// ambre-toile/src/chrome/notify.rs
// ============================================================================
// Toile's user-facing notifications (toile-import-export). The launcher relays
// every event and names no Toile variant itself, so the mapping from event to
// toast lives here, exactly as Trame's does.
//
// The wording is pure and host-tested in `ui::interop`; this is the collage that
// puts it on screen.
// ============================================================================

use ambre_bus::app::toile::ToileEvent;
use ambre_bus::{AppEvent, UiEvent};
use ambre_chrome::Toasts;

use crate::ui::interop::import_message;

/// Surface a Toile `UiEvent` as a transient toast when it carries something the
/// user must see: what an import brought in and what it dropped
/// (`TOILE-IO-022`), why a file was refused, and that an export produced a file.
/// A no-op for every other event.
pub fn notify_event(toasts: &Toasts, ev: &UiEvent) {
    let UiEvent::Viewport {
        event: AppEvent::Toile(te),
        ..
    } = ev
    else {
        return;
    };
    match te {
        // The import landed. A lossy one says so here and nowhere else: the
        // diagnostic log is not a surface a user reads (`TOILE-IO-022`).
        ToileEvent::ImportReport { imported, skipped } => {
            let message = import_message(*imported, skipped);
            if skipped.is_empty() {
                toasts.info(message);
            } else {
                toasts.warning(message);
            }
        }
        // Nothing was applied, and the board looks exactly as it did: without a
        // word the user cannot tell the drop from a miss.
        ToileEvent::ImportFailed(reason) => {
            toasts.warning(format!("Import failed: {reason}"));
        }
        // The download starts canvas-side on this same event; the toast confirms
        // which file left.
        ToileEvent::ExportComplete { filename, .. } => {
            toasts.info(format!("Exported {filename}"));
        }
        _ => {}
    }
}
