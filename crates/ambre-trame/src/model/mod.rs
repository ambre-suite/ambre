// ambre-trame/src/model/mod.rs
// ============================================================================
// Trame document model layer (Logic thread side).
//
// This module owns the *stable-identity* layer that sits between the formula
// engine (which addresses cells positionally, by grid `(row, col)`) and the
// CRDT (which keys cells by our own move-invariant `Uid` pairs so that inserting
// a row is a single Loro op and a reorder never orphans a cell -- see
// `ambre-spec-trame-logic.md` §2.A).
//
// `SheetIndex` is the bidirectional position <-> `Uid` cache; it is the only
// place that translates between the two address spaces.
// ============================================================================

mod sheet_index;

pub use sheet_index::{SheetIndex, Uid, cell_key, parse_cell_key};
