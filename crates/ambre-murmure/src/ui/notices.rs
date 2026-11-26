//! What the user reads when a Murmure step fails, decided on the host.
//!
//! The chrome draws each notice as a toast, and the chrome is wasm-only. The
//! choice of the text lives here, so a native test reads it. Each text is a
//! fixed sentence. The cause stays in the log, where Logic wrote it. So no
//! identifier in a cause reaches the user.

use crate::message::{DirectMessageEvent, MurmureEvent, SpaceEvent};
use ambre_bus::UiEvent;

/// What the user reads when the root of a copy could not be reached. Replying
/// again starts a fresh seek (the Murmure threads spec).
pub const ROOT_LOST: ambre_commands::TitleWords = ambre_commands::words!(
    "This thread cannot open: its first message could not be loaded.",
    "Ce fil ne peut pas s'ouvrir\u{a0}: son premier message n'a pas pu \u{ea}tre charg\u{e9}."
);

/// What the user reads when a channel could not open. The cause stays in the
/// log, where Logic wrote it (`MURMURE-549`).
pub const CHANNEL_NOT_OPENED: ambre_commands::TitleWords = ambre_commands::words!(
    "This channel could not open. Select it again to retry.",
    "Ce canal n'a pas pu s'ouvrir. S\u{e9}lectionnez-le \u{e0} nouveau pour r\u{e9}essayer."
);

/// What the user reads when a channel could not be created, joined, deleted
/// or left. Nothing changed on the relay, and the cause stays in the log
/// (`MURMURE-580`).
pub const CHANNEL_NOT_CHANGED: ambre_commands::TitleWords = ambre_commands::words!(
    "The channel could not be changed. Try again.",
    "Le canal n'a pas pu \u{ea}tre modifi\u{e9}. R\u{e9}essayez."
);

/// What the user reads when a space could not open (`MURMURE-550`).
pub const SPACE_NOT_OPENED: ambre_commands::TitleWords = ambre_commands::words!(
    "A space could not open, so its channels to join are missing for now.",
    "Un espace n'a pas pu s'ouvrir\u{a0}: ses canaux \u{e0} rejoindre manquent pour l'instant."
);

/// What the user reads when a direct message could not open. Nothing changed
/// on the relay, and the cause stays in the log: it can name a member
/// (`MURMURE-581`).
pub const DIRECT_MESSAGE_NOT_OPENED: ambre_commands::TitleWords = ambre_commands::words!(
    "The direct message could not open. Try again.",
    "Le message direct n'a pas pu s'ouvrir. R\u{e9}essayez."
);

/// The warning `ev` raises, if any. A no-op for every other event.
#[must_use]
pub fn warning_of<S>(ev: &ambre_bus::UiEvent<S, crate::message::Murmure>) -> Option<&'static str> {
    match ev {
        UiEvent::Channel {
            event: MurmureEvent::RootLost { .. },
            ..
        } => Some(ambre_commands::shown(&ROOT_LOST)),
        UiEvent::Channel {
            event: MurmureEvent::OpenFailed,
            ..
        } => Some(ambre_commands::shown(&CHANNEL_NOT_OPENED)),
        UiEvent::Space {
            event: SpaceEvent::OpenFailed,
            ..
        } => Some(ambre_commands::shown(&SPACE_NOT_OPENED)),
        UiEvent::Space {
            event: SpaceEvent::ChannelFailed { .. },
            ..
        } => Some(ambre_commands::shown(&CHANNEL_NOT_CHANGED)),
        UiEvent::DirectMessage {
            event: DirectMessageEvent::Failed,
            ..
        } => Some(ambre_commands::shown(&DIRECT_MESSAGE_NOT_OPENED)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::id::{Channel, Id};

    /// `MURMURE-581`: a direct message that failed is told, and the cause
    /// stays out of the text: the event carries none to show. An opened one
    /// raises nothing.
    #[test]
    fn a_direct_message_that_failed_is_told_without_its_cause() {
        let failed: crate::message::UiEvent = UiEvent::DirectMessage {
            requested: Id::<Channel>::mint(),
            event: DirectMessageEvent::Failed,
        };
        let text = warning_of(&failed).expect("a warning");
        assert_eq!(text, ambre_commands::shown(&DIRECT_MESSAGE_NOT_OPENED));

        let opened: crate::message::UiEvent = UiEvent::DirectMessage {
            requested: Id::<Channel>::mint(),
            event: DirectMessageEvent::Opened {
                channel_id: Id::<Channel>::mint(),
            },
        };
        assert_eq!(warning_of(&opened), None);
    }
}
