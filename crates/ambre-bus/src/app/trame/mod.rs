//! Trame (spreadsheet) data plane: the whole typed content vocabulary of a Trame
//! document, both directions.
//!
//! - `command` -- the `TrameCommand` family (UI -> Logic), carried by
//!   [`AppCommand::Trame`](crate::logic::AppCommand).
//! - `event` -- the `TrameEvent` family + `DisplayCell` (Logic -> UI), carried by
//!   [`AppEvent::Trame`](crate::ui::AppEvent).
//! - `grid` -- the row, column and cell coordinates both directions carry.
//! - `calendar` -- the spreadsheet calendar, the serials a date travels as.
//! - `style` / `clipboard` / `features` / `find` -- the shared value types both
//!   directions reference (cell styles, filters, clipboard payloads, deferred
//!   feature payloads, find options).
//!
//! Re-globbed so every `crate::app::trame::*` path (and the crate-root
//! re-export) resolves flat, and so a submodule's `super::CellStyle` reaches its
//! sibling value types.

mod calendar;
mod clipboard;
mod command;
mod event;
mod features;
mod find;
mod style;

pub use super::grid::*;
pub use calendar::*;
pub use clipboard::*;
pub use command::*;
pub use event::*;
pub use features::*;
pub use find::*;
pub use style::*;
