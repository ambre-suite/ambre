//! IPC message types for inter‑thread communication in Ambre.
//!
//! Four thread envelopes, one per channel direction:
//! - `LogicCommand` (UI → Logic) and `UiEvent` (Logic → UI) — see [`ui`].
//! - `SyncCommand` (Logic → Sync) — see [`command`].
//! - `LogicEvent` (Sync → Logic) — see [`event`].
//!
//! Plus the typed data planes carried inside the envelopes:
//! - **Content** — the `AppCommand` / `AppEvent` closed sums live next to their
//!   envelopes ([`logic`] / [`ui`]), but each document type's whole data plane
//!   (its command + event + shared value / display types) is grouped **one
//!   module per app** under [`app`] (`app::trame`, `app::toile`), re-exported
//!   flat at the crate root. Document edits and viewport data, routed by
//!   `DocumentId`. (See `ambre-spec-architecture.md` "ambre-bus layout".)
//! - **Service** (`RestCommand`/`RestEvent` + `RequestId` in [`rest`]) — the
//!   correlatable request/response family carried by `…::Rest { id, … }` on all
//!   four envelopes, defined once and forwarded verbatim.
//!
//! Transport split: `LogicCommand`/`UiEvent` travel UI <-> Logic over `flume`
//! channels in shared memory (typed Rust values, no serialization). The
//! `SyncCommand`/`LogicEvent` envelope crosses a `postMessage` boundary to the
//! separate Sync worker, so it (and the `Rest*` payloads it carries) derives
//! `Serialize`/`Deserialize` and is postcard-encoded on that hop.

pub mod app;
pub mod command;
pub mod event;
pub mod logic;
pub mod rest;
pub mod sink;
pub mod sync_warning;
pub mod ui;

// Re‑export the main types for convenient use.
pub use command::SyncCommand;
pub use event::LogicEvent;
// Trame data plane (command + event + value types) lives under `app::trame`;
// re-exported flat at the crate root so downstream crates keep `ambre_bus::…`.
pub use app::trame::{
    BORDER_EDGE_ALL, BORDER_EDGE_BOTTOM, BORDER_EDGE_LEFT, BORDER_EDGE_RIGHT, BORDER_EDGE_TOP,
    BorderEdit, BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellStyle,
    ClipboardCell, ClipboardPayload, CondFormatKind, CondStyle, DisplayCell, FillMode, FilterKind,
    FilterOp, GridRange, HAlign, SortKey, StylePatch, TrameCommand, TrameEvent, VAlign,
    ValidationKind, ValidationOnInvalid,
};
pub use logic::{AppCommand, LogicCommand};
pub use rest::{
    ChannelSummary, DocumentSummary, MemberInfo, RequestId, RestCommand, RestEvent, SecretBytes,
    SecretString, UnreadCount, UserInfo,
};
pub use sink::CommandSink;
pub use ui::{AppEvent, UiEvent};
// Re‑export core types so that users don't need to depend on ambre‑core directly.
pub use ambre_core::{
    ChannelId, DocType, DocumentId, MeetingId, PublicKey, Role, SnapshotId, UserId, Uuid,
};
