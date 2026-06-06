//! IPC message types for inter‑thread communication in Ambre.
//!
//! Four thread envelopes, one per channel direction:
//! - `LogicCommand` (UI → Logic) and `UiEvent` (Logic → UI) — see [`ui`].
//! - `SyncCommand` (Logic → Sync) — see [`command`].
//! - `LogicEvent` (Sync → Logic) — see [`event`].
//!
//! Plus the typed data planes carried inside the envelopes:
//! - **Content** (`AppCommand`/`TrameCommand` next to `LogicCommand` in
//!   [`logic`]; `AppEvent`/`TrameEvent`/`DisplayCell` next to `UiEvent` in
//!   [`ui`]) — document edits and viewport data, routed by `DocumentId`.
//! - **Service** (`RestCommand`/`RestEvent` + `RequestId` in [`rest`]) — the
//!   correlatable request/response family carried by `…::Rest { id, … }` on all
//!   four envelopes, defined once and forwarded verbatim.
//!
//! These travel over `flume` channels in shared memory; nothing here is
//! serialized (postcard lives only on the network path, in `ambre-protocol`).

pub mod command;
pub mod event;
pub mod logic;
pub mod rest;
pub mod sync_warning;
pub mod ui;

// Re‑export the main types for convenient use.
pub use command::SyncCommand;
pub use event::LogicEvent;
pub use logic::{AppCommand, LogicCommand, TrameCommand};
pub use rest::{
    ChannelSummary, DocumentSummary, MemberInfo, RequestId, RestCommand, RestEvent, UnreadCount,
    UserInfo,
};
pub use ui::{AppEvent, DisplayCell, TrameEvent, UiEvent};
// Re‑export core types so that users don't need to depend on ambre‑core directly.
pub use ambre_core::{
    ChannelId, DocType, DocumentId, MeetingId, PublicKey, Role, SnapshotId, UserId, Uuid,
};
