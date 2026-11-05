//! What the user reads when a Murmure step fails, decided on the host.
//!
//! The chrome draws each notice as a toast, and the chrome is wasm-only. The
//! choice of the text lives here, so a native test reads it. Each text is a
//! fixed sentence. The cause stays in the log, where Logic wrote it. So no
//! identifier in a cause reaches the user.

use ambre_bus::{DirectMessageEvent, MurmureEvent, SpaceEvent, UiEvent};

/// What the user reads when the root of a copy could not be reached. Replying
/// again starts a fresh seek (the Murmure threads spec).
pub const ROOT_LOST: &str = "This thread cannot open: its first message could not be loaded.";

/// What the user reads when a channel could not open. The cause stays in the
/// log, where Logic wrote it (`MURMURE-549`).
pub const CHANNEL_NOT_OPENED: &str = "This channel could not open. Select it again to retry.";

/// What the user reads when a channel could not be created, joined, deleted
/// or left. Nothing changed on the relay, and the cause stays in the log
/// (`MURMURE-580`).
pub const CHANNEL_NOT_CHANGED: &str = "The channel could not be changed. Try again.";

/// What the user reads when a space could not open (`MURMURE-550`).
pub const SPACE_NOT_OPENED: &str =
    "A space could not open, so its channels to join are missing for now.";

/// What the user reads when a direct message could not open. Nothing changed
/// on the relay, and the cause stays in the log: it can name a member
/// (`MURMURE-581`).
pub const DIRECT_MESSAGE_NOT_OPENED: &str = "The direct message could not open. Try again.";

/// The warning `ev` raises, if any. A no-op for every other event.
#[must_use]
pub fn warning_of<S>(ev: &UiEvent<S>) -> Option<&'static str> {
    match ev {
        UiEvent::Channel {
            event: MurmureEvent::RootLost { .. },
            ..
        } => Some(ROOT_LOST),
        UiEvent::Channel {
            event: MurmureEvent::OpenFailed { .. },
            ..
        } => Some(CHANNEL_NOT_OPENED),
        UiEvent::Space {
            event: SpaceEvent::OpenFailed { .. },
            ..
        } => Some(SPACE_NOT_OPENED),
        UiEvent::Space {
            event: SpaceEvent::ChannelFailed { .. },
            ..
        } => Some(CHANNEL_NOT_CHANGED),
        UiEvent::DirectMessage {
            event: DirectMessageEvent::Failed { .. },
            ..
        } => Some(DIRECT_MESSAGE_NOT_OPENED),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::id::{Channel, Id, User};

    /// `MURMURE-581`: a direct message that failed is told, and the cause
    /// stays out of the text, the member it names included. An opened one
    /// raises nothing.
    #[test]
    fn a_direct_message_that_failed_is_told_without_its_cause() {
        let member = Id::<User>::mint();
        let failed: UiEvent = UiEvent::DirectMessage {
            requested: Id::<Channel>::mint(),
            event: DirectMessageEvent::Failed {
                message: format!("the exchange key of {member} is not the one pinned"),
            },
        };
        let text = warning_of(&failed).expect("a warning");
        assert_eq!(text, DIRECT_MESSAGE_NOT_OPENED);
        assert!(!text.contains(&member.to_string()));
        assert!(!text.contains("pinned"));

        let opened: UiEvent = UiEvent::DirectMessage {
            requested: Id::<Channel>::mint(),
            event: DirectMessageEvent::Opened {
                channel_id: Id::<Channel>::mint(),
            },
        };
        assert_eq!(warning_of(&opened), None);
    }
}
