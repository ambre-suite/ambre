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
            peer: 1,
            counter: i as u32,
        });
    }
    for i in 0..cols {
        idx.push_col(Uid {
            peer: 2,
            counter: i as u32,
        });
    }
    idx
}

/// A single-sheet `SheetIndices` map (sheet 0) sized `rows` x `cols`.
pub fn indices_with(rows: usize, cols: usize) -> super::SheetIndices {
    let mut m = super::SheetIndices::new();
    m.insert(0, index_with(rows, cols));
    m
}
