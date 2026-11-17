// Shared helpers here; tests split into groups reached via `use super::*`.
mod editor_parity;
mod group1;
mod group2;
mod kinetic;
mod session;
mod viewport;

use super::{AxisPos, CellData, CellEditor, HeaderHit, SelKind, Selection, TrameUiState};
use ambre_core::index::Pos;

use crate::test_support::{at, rect, span};
use crate::{CellPos, CellRect, Col, MergeSpan, Row};
use crate::{DisplayCell, TrameEvent};

/// A pointer at logical `(x, y)`, as the canvas hands it over.
fn pointer(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::View>::new(x as f32),
        ambre_core::Px::<ambre_core::View>::new(y as f32),
    )
}

/// A logical length along one axis, as the canvas hands it over.
fn view_px(v: f64) -> ambre_core::Px<ambre_core::View> {
    ambre_core::Px::<ambre_core::View>::new(v as f32)
}

/// Seed a non-empty cell in the viewport cache (the rest of `CellData`
/// defaulted), so the current-region helpers see it as data.
fn put(state: &mut TrameUiState, row: u32, col: u32, text: &str) {
    state.cell_cache.insert(
        CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)),
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
        is_merged: crate::message::Merge::Merged,
        merge_span: span,
        ..Default::default()
    };
    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_core::Seq::ZERO,
        row_start: Pos::<Row>::new(0),
        col_start: Pos::<Col>::new(0),
        col_widths: vec![],
        row_heights: vec![],
        frozen: crate::message::FrozenView::default(),
        cells: vec![
            cell(Pos::<Row>::new(1), Pos::<Col>::new(1), Some(span(2, 2))), // master
            cell(Pos::<Row>::new(1), Pos::<Col>::new(2), None),
            cell(Pos::<Row>::new(2), Pos::<Col>::new(1), None),
            cell(Pos::<Row>::new(2), Pos::<Col>::new(2), None),
        ],
    });
}
