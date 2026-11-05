//! Per-application data planes carried by the IPC envelopes, for the
//! applications whose vocabulary has not moved into their own crate yet
//! (`BUS-032`). Trame's lives in `ambre-trame`; `grid` and `find` stay here,
//! shared by more than one application.
//!
//! One module per document type, each holding **that app's whole typed data
//! plane together**, regardless of direction. A module holds its command enum,
//! its event enum, and the shared value / display types. `AppCommand` /
//! `AppEvent` (the closed sums in [`crate::logic`] / [`crate::ui`]) are thin
//! dispatch enums that reference these modules.
//!
//! Rationale (`ambre-spec-message-bus.md`): the four transport **envelopes**
//! (`LogicCommand` / `UiEvent` / `SyncCommand` / `LogicEvent`) are still split
//! by direction. Each is bound to one queue hop. But the per-app **content
//! plane** is grouped by app. A document type's command and event share its
//! whole value-type vocabulary (identity, styles, geometry). Co-locating them
//! keeps one app in one place, and stops the event side reaching across into the
//! command side for shared types. The `Command` / `Event` suffix on the type
//! names still carries the direction.

pub mod find;
pub mod grid;
pub mod murmure;
pub mod prose;
pub mod toile;
