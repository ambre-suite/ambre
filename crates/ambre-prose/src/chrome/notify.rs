// ambre-prose/src/chrome/notify.rs
// ============================================================================
// Prose's interop notifications (`features/ambre-spec-prose-import-export.md
// section 4`). The launcher relays every event and names no Prose variant, so
// the mapping from event to toast lives here, as Trame's and Toile's do.
// ============================================================================

use crate::message::{ProseCommand, ProseError, ProseEvent, ProseRefusal, RefusedAction};
use ambre_bus::{Embed, UiEvent};
use ambre_chrome::Toasts;

use crate::view::annotate::LostReply;

/// Surface what a conversion lost, and a file that produced nothing, as a
/// toast (`PROSE-IO-053`). A reply refused for its thread is told too
/// (`PROSE-UI-232`). A no-op for every other event.
pub fn notify_event<S: Embed<ProseEvent>, P: ambre_bus::ChannelPlane>(
    toasts: &Toasts,
    ev: &UiEvent<S, P>,
) {
    let Some(event) = ev.viewport_event::<ProseEvent>() else {
        return;
    };
    match event {
        // The shared sentence (`INTEROP-007`, `INTEROP-011`).
        ProseEvent::Interop(interop) => {
            if let Some(message) = interop.message() {
                if interop.is_warning() {
                    toasts.warning(message);
                } else {
                    toasts.info(message);
                }
            }
        }
        // A reply sent as a peer deleted its thread (`PROSE-UI-232`).
        ProseEvent::Error(error) if is_refused_reply(error) => {
            toasts.warning(LostReply::Deleted.notice());
        }
        _ => {}
    }
}

/// Whether a refusal turned down a reply because its thread is gone. The
/// engine puts the name of the reply command on that refusal.
fn is_refused_reply(error: &ProseError) -> bool {
    matches!(error.reason, ProseRefusal::NoSuchComment(_))
        && error.refused == RefusedAction::Command(ProseCommand::REPLY_TO_COMMENT_NAME)
}
