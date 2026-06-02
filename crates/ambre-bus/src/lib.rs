//! IPC message types for inter‑thread communication in Ambre.
//!
//! Four thread envelopes, one per channel direction:
//! - `LogicCommand` (UI → Logic) and `UiEvent` (Logic → UI) — see [`ui`].
//! - `SyncCommand` (Logic → Sync) — see [`command`].
//! - `LogicEvent` (Sync → Logic) — see [`event`].
//!
//! Plus the typed application data plane carried inside the envelopes, split by
//! direction: command types (`AppCommand`, `TrameCommand`) live next to
//! `LogicCommand` in [`logic`]; event types (`AppEvent`, `TrameEvent`,
//! `DisplayCell`) live next to `UiEvent` in [`ui`]. These travel over `flume`
//! channels in shared memory; nothing here is serialized (postcard lives only
//! on the network path, in `ambre-protocol`).

pub mod command;
pub mod event;
pub mod logic;
pub mod ui;

// Re‑export the main types for convenient use.
pub use command::SyncCommand;
pub use event::{ChannelSummary, DocumentSummary, LogicEvent, MemberInfo, UnreadCount, UserInfo};
pub use logic::{AppCommand, LogicCommand, TrameCommand};
pub use ui::{AppEvent, DisplayCell, TrameEvent, UiEvent};
// Re‑export core types so that users don't need to depend on ambre‑core directly.
pub use ambre_core::{
    ChannelId, DocType, DocumentId, MeetingId, PublicKey, Role, SnapshotId, UserId, Uuid,
};

/// Fixed document_id used for end-to-end IPC smoke-testing before the full
/// auth and document-open flow is wired.
///
/// Both the UI Thread and the Logic thread reference this constant so the
/// pre-seeded test document state and the keydown event listener stay in sync.
/// Remove once real document management is operational.
pub const IPC_TEST_DOC_ID: DocumentId = DocumentId::from_bytes([
    0x01, 0x96, 0x00, 0x00, 0x00, 0x00, 0x70, 0x00,
    0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
]);
