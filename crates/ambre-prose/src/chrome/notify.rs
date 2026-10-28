// ambre-prose/src/chrome/notify.rs
// ============================================================================
// Prose's interop notifications (`features/ambre-spec-prose-import-export.md
// section 4`). The launcher relays every event and names no Prose variant, so
// the mapping from event to toast lives here, as Trame's and Toile's do.
// ============================================================================

use ambre_bus::app::prose::{ProseCommand, ProseError, ProseEvent, ProseRefusal, RefusedAction};
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
        ProseEvent::Error(error) if is_refused_reply(error) => {
            toasts.warning(LostReply::Deleted.notice());
        }
        _ => {}
    }
}

/// Whether a refusal turned down a reply because its thread is gone. The
/// command is rebuilt only to read its name, which is the one the engine put
/// on the refusal.
fn is_refused_reply(error: &ProseError) -> bool {
    let ProseRefusal::NoSuchComment(comment_id) = error.reason else {
        return false;
    };
    let reply = ProseCommand::ReplyToComment {
        comment_id,
        text: String::new(),
        now: ambre_core::Timestamp(0),
    };
    error.refused == RefusedAction::Command(reply.name())
}
