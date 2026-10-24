// Shared helpers here; tests split into groups reached via `use super::*`.
mod editor_parity;
mod group1;
mod group2;
mod kinetic;
mod session;
mod viewport;

use super::{AxisPos, CellData, CellEditor, HeaderHit, SelKind, Selection, TrameUiState};

use crate::test_support::{at, rect, span};
use crate::{CellPos, CellRect, Col, ColCount, ColDelta, MergeSpan, Row, RowCount, RowDelta};
use crate::{DisplayCell, TrameEvent};

/// A pointer at logical `(x, y)`, as the canvas hands it over.
fn pointer(x: f64, y: f64) -> ambre_core::ViewPoint {
    ambre_core::ViewPoint::new(
        ambre_core::ViewPx::new(x as f32),
        ambre_core::ViewPx::new(y as f32),
    )
}

/// A logical length along one axis, as the canvas hands it over.
fn view_px(v: f64) -> ambre_core::ViewPx {
    ambre_core::ViewPx::new(v as f32)
}

/// Seed a non-empty cell in the viewport cache (the rest of `CellData`
/// defaulted), so the current-region helpers see it as data.
fn put(state: &mut TrameUiState, row: u32, col: u32, text: &str) {
    state.cell_cache.insert(
        CellPos::new(Row::new(row), Col::new(col)),
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
        version: ambre_bus::ViewportVersion(0),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(Row::new(1), Col::new(1), Some(span(2, 2))), // master
            cell(Row::new(1), Col::new(2), None),
            cell(Row::new(2), Col::new(1), None),
            cell(Row::new(2), Col::new(2), None),
        ],
    });
}
