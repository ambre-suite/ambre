// ambre-toile/src/chrome/notify.rs
// ============================================================================
// Toile's user-facing notifications (toile-import-export). The launcher relays
// every event and names no Toile variant itself, so the mapping from event to
// toast lives here, exactly as Trame's does.
//
// The wording is pure and host-tested in `ui::interop`; this is the collage that
// puts it on screen.
// ============================================================================

use crate::message::ToileEvent;
use ambre_bus::{Embed, UiEvent};
use ambre_chrome::Toasts;

use crate::ui::interop::import_message;

/// Surface a Toile `UiEvent` as a transient toast when it carries something the
/// user must see. That is: what an import brought in and what it dropped
/// (`TOILE-IO-022`), why a file was refused, and that an export produced a file.
/// It also surfaces the two schema notices: a board this build cannot open
/// (`SCHEMA-008`) and a register it cannot decode (`SCHEMA-003`). A no-op for
/// every other event.
pub fn notify_event<S: Embed<ToileEvent>, P: ambre_bus::ChannelPlane>(
    toasts: &Toasts,
    ev: &UiEvent<S, P>,
) {
    let Some(te) = ev.viewport_event::<ToileEvent>() else {
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
        // An export that produced nothing is silent otherwise: no file lands,
        // and the board looks exactly as it did (`TOILE-IO-029`).
        ToileEvent::ExportFailed(reason) => {
            toasts.warning(format!("Export failed: {reason}"));
        }
        // The board was written by a newer client (`SCHEMA-008`). An error, not
        // a warning: nothing opened, and the remedy is not on this board.
        ToileEvent::DocumentRefused(reason) => {
            toasts.error(reason.clone());
        }
        // Part of the board could not be read and is drawn plain (`SCHEMA-003`).
        // The document is open and usable, so this is a warning. But it must be
        // said: a register that silently reads as the default style is a document
        // that looks intact and is not.
        ToileEvent::UndecodableRecords => {
            toasts.warning(
                "Some formatting in this board was written by a newer client and could not be read.",
            );
        }
        // An arrow lost an end, here or under a peer's delete (`TOILE-044`,
        // `TOILE-165`). It stops being painted, so the board cannot show it.
        // The notice is the one surface that says the document still holds it.
        ToileEvent::DanglingConnector(_) => {
            toasts.warning("An arrow lost an end and is no longer drawn");
        }
        _ => {}
    }
}
