// ambre-prose/src/chrome/notify.rs
// ============================================================================
// Prose's interop notifications (`features/ambre-spec-prose-import-export.md
// section 4`). The launcher relays every event and names no Prose variant, so
// the mapping from event to toast lives here, as Trame's and Toile's do.
// ============================================================================

use ambre_bus::app::prose::{ProseEvent, ProseRefusal, RefusedAction};
use ambre_bus::{AppEvent, UiEvent};
use ambre_chrome::Toasts;

use crate::interop::report_message;
use crate::view::annotate::LostReply;

/// Surface what a conversion lost, and a file that produced nothing, as a
/// toast (`PROSE-IO-053`). A reply refused for its thread is told too
/// (`PROSE-UI-232`). A no-op for every other event.
pub fn notify_event(toasts: &Toasts, ev: &UiEvent) {
    let UiEvent::Viewport {
        event: AppEvent::Prose(event),
        ..
    } = ev
    else {
        return;
    };
    match event {
        ProseEvent::ImportCompleted { report } => match report_message(report) {
            Some(lost) => toasts.warning(format!("Imported. {lost}")),
            None => toasts.info("Imported"),
        },
        ProseEvent::ExportReady {
            filename, report, ..
        } => match report_message(report) {
            Some(lost) => toasts.warning(format!("Exported {filename}. {lost}")),
            None => toasts.info(format!("Exported {filename}")),
        },
        ProseEvent::Error(error) if matches!(error.reason, ProseRefusal::Interop(_)) => {
            toasts.warning(format!("Import failed: {}", error.reason));
        }
        // A reply sent as a peer deleted its thread (`PROSE-UI-232`).
        // The name is `ProseCommand::name` of `ReplyToComment`.
        ProseEvent::Error(error)
            if matches!(error.reason, ProseRefusal::NoSuchComment(_))
                && error.refused == RefusedAction::Command("replying to the comment") =>
        {
            toasts.warning(LostReply::Deleted.notice());
        }
        _ => {}
    }
}
