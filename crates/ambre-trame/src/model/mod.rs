// ambre-trame/src/model/mod.rs
// ============================================================================
// Trame document model layer (Logic thread side).
//
// This module owns the *stable-identity* layer between two address spaces. The
// formula engine addresses cells positionally, by grid `CellPos`. The CRDT
// keys them by our own move-invariant `Uid` pairs. A row insert is then a single
// Loro op, and a reorder never orphans a cell (`trame-rows-and-columns`).
//
// `SheetIndex` is the bidirectional position <-> `Uid` cache; it is the only
// place that translates between the two address spaces.
// ============================================================================

mod line;
mod sheet_index;

pub use line::Line;
pub use sheet_index::{CellKey, SheetIndex, Uid, parse_cell_key};
