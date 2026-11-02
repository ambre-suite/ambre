//! Shared test helper: build a `SheetIndex` with synthetic materialized rows and
//! columns. The formula-engine unit tests can then bind positional references to
//! `Uid`s without a live `LoroDoc`. Rows use peer 1, columns peer 2; the counter
//! is the grid position, so `row_id(p)`/`col_id(p)` are stable and distinct.

use crate::model::{SheetIndex, Uid};

/// A `SheetIndex` with `rows` materialized rows and `cols` materialized columns.
pub fn index_with(rows: usize, cols: usize) -> SheetIndex {
    let mut idx = SheetIndex::default();
    for i in 0..rows {
        idx.push_row(Uid {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::IdCounter::new(i as u32),
        });
    }
    for i in 0..cols {
        idx.push_col(Uid {
            peer: ambre_core::PeerId::new(2),
            counter: ambre_core::IdCounter::new(i as u32),
        });
    }
    idx
}

/// A single-sheet `SheetIndices` map (sheet 0) sized `rows` x `cols`.
pub fn indices_with(rows: usize, cols: usize) -> super::SheetIndices {
    let mut m = super::SheetIndices::new();
    m.insert(super::ACTIVE_SHEET, index_with(rows, cols));
    m
}

/// `text` as the CRDT would store it.
pub fn stored(text: &str) -> crate::formula::text::CellText<super::text::StoredForm> {
    crate::formula::text::CellText::<super::text::StoredForm>::new(text.to_owned())
}

/// The function name `text`, which must be an identifier.
pub fn function(text: &str) -> super::types::FunctionName {
    super::types::FunctionName::parse(text).expect("an identifier")
}
