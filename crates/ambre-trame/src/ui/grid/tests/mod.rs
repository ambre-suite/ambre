// Shared helpers here; tests split into groups reached via `use super::*`.
mod editor_parity;
mod group1;
mod group2;
mod kinetic;

use super::{CellData, CellEditor, HeaderHit, SelKind, Selection, TrameUiState};

use crate::{DisplayCell, TrameEvent};

/// Seed a non-empty cell in the viewport cache (the rest of `CellData`
/// defaulted), so the current-region helpers see it as data.
fn put(state: &mut TrameUiState, row: u32, col: u32, text: &str) {
    state.cell_cache.insert(
        (row, col),
        CellData {
            text: text.to_owned(),
            ..Default::default()
        },
    );
}

/// Seed a 2x2 merge master at (1,1) covering (1,1)..=(2,2) in the cache.
fn seed_merge_2x2(state: &mut TrameUiState) {
    let cell = |row, col, span| DisplayCell {
        row,
        col,
        text: String::new(),
        raw: String::new(),
        is_merged: true,
        merge_span: span,
        ..Default::default()
    };
    state.on_viewport_data(TrameEvent::ViewportData {
        version: 0,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(1, 1, Some((2, 2))), // master
            cell(1, 2, None),
            cell(2, 1, None),
            cell(2, 2, None),
        ],
    });
}
