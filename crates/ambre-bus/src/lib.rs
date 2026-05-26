//! IPC message types for inter‑worker communication in Ambre.
//!
//! Defines `SyncCommand` (Logic → Sync), `LogicEvent` (Sync → Logic),
//! `UiEnvelope` (UI → Logic), `LogicEnvelope` (Logic → UI), and the
//! `AppController` / `CommandSink` traits.

pub mod channel;
pub mod command;
pub mod envelope;
pub mod event;
pub mod traits;

// Re‑export the main types for convenient use.
pub use channel::SendCommandSink;
pub use command::SyncCommand;
pub use envelope::{LogicEnvelope, UiEnvelope};
pub use event::{ChannelSummary, DocumentSummary, LogicEvent, MemberInfo, UnreadCount, UserInfo};
pub use traits::{AppController, CommandSink};
// Re‑export core types so that users don't need to depend on ambre‑core directly.
pub use ambre_core::{DocType, PublicKey, Role, Uuid};

/// Fixed document_id used for end-to-end IPC smoke-testing before the full
/// auth and document-open flow is wired.
///
/// Both the UI Thread and the Logic Worker reference this constant so the
/// pre-seeded test controller and the keydown event listener stay in sync.
/// Remove once real document management is operational.
pub const IPC_TEST_DOC_ID: Uuid = Uuid::from_bytes([
    0x01, 0x96, 0x00, 0x00, 0x00, 0x00, 0x70, 0x00,
    0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);

#[cfg(test)]
mod tests {
    use super::*;
    use postcard::{from_bytes, to_allocvec};

    #[test]
    fn test_sync_command_serialization() {
        let cmd = SyncCommand::ListDocuments;
        let bytes = to_allocvec(&cmd).unwrap();
        let decoded: SyncCommand = from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, SyncCommand::ListDocuments));
    }

    #[test]
    fn test_logic_event_serialization() {
        let event = LogicEvent::AccountSetupRequired;
        let bytes = to_allocvec(&event).unwrap();
        let decoded: LogicEvent = from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, LogicEvent::AccountSetupRequired));
    }

    #[test]
    fn test_envelope_serialization() {
        let uuid = Uuid::new_v7();
        let envelope = UiEnvelope::AppCommand { document_id: uuid, payload: vec![1, 2, 3] };
        let bytes = to_allocvec(&envelope).unwrap();
        let decoded: UiEnvelope = from_bytes(&bytes).unwrap();
        assert!(
            matches!(decoded, UiEnvelope::AppCommand { document_id, payload } if document_id == uuid && payload == [1, 2, 3])
        );
    }
}
