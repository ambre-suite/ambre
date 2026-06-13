// ambre-trame/src/model/mod.rs
// ============================================================================
// Trame document model layer (Logic thread side).
//
// This module owns the *stable-identity* layer that sits between the formula
// engine (which addresses cells positionally, by grid `(row, col)`) and the
// CRDT (which keys cells by stable `ListItemId` pairs so that inserting a row
// is a single Loro operation -- see `ambre-spec-trame-logic.md` §2).
//
// `SheetIndex` is the bidirectional position <-> `ListItemId` cache; it is the
// only place that translates between the two address spaces.
// ============================================================================

mod sheet_index;

pub use sheet_index::{Lid, SheetIndex, cell_key, parse_cell_key};
