//! Per-application data planes carried by the IPC envelopes.
//!
//! One module per document type, each holding **that app's whole typed data
//! plane together**, regardless of direction. A module holds its command enum,
//! its event enum, and the shared value / display types. `AppCommand` /
//! `AppEvent` (the closed sums in [`crate::logic`] / [`crate::ui`]) are thin
//! dispatch enums that reference these modules.
//!
//! Rationale (`ambre-spec-message-bus.md`): the four transport **envelopes**
//! (`LogicCommand` / `UiEvent` / `SyncCommand` / `LogicEvent`) are still split
//! by direction. Each is bound to one channel hop. But the per-app **content
//! plane** is grouped by app. A document type's command and event share its
//! whole value-type vocabulary (identity, styles, geometry). Co-locating them
//! keeps one app in one place, and stops the event side reaching across into the
//! command side for shared types. The `Command` / `Event` suffix on the type
//! names still carries the direction.

pub mod toile;
pub mod trame;
