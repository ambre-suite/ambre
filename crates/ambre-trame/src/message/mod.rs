//! Trame (spreadsheet) data plane: the whole typed content vocabulary of a Trame
//! document, both directions. It lives in the application's crate, beside the
//! view and the engine that use it, and the Sync worker never builds against it
//! (`BUS-032`, `ARCH-070`).
//!
//! - `command` -- the `TrameCommand` family (UI -> Logic), carried by
//!   `AppCommand::Trame` of the Logic runtime.
//! - `event` -- the `TrameEvent` family + `DisplayCell` (Logic -> UI), carried by
//!   `AppEvent::Trame` of the Logic runtime.
//! - `grid` -- the row, column and cell coordinates both directions carry,
//!   shared with Prose and re-exported from `ambre-bus`.
//! - `calendar` -- the spreadsheet calendar, the serials a date travels as.
//! - `style` / `clipboard` / `features` / `find` -- the shared value types both
//!   directions reference (cell styles, filters, clipboard payloads, deferred
//!   feature payloads, find options).
//! - `text` -- the three forms a cell's text takes: source, stored, display.
//!
//! Re-globbed so every `crate::message::*` path (and the crate-root
//! re-export) resolves flat, and so a submodule's `super::CellStyle` reaches its
//! sibling value types.

mod calendar;
mod clipboard;
mod command;
mod event;
mod features;
mod find;
mod style;
mod text;

pub use ambre_bus::app::grid::*;
pub use calendar::*;
pub use clipboard::*;
pub use command::*;
pub use event::*;
pub use features::*;
pub use find::*;
pub use style::*;
pub use text::*;
