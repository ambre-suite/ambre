// ambre-trame/src/logic/tests.rs
// Unit tests for `TrameState` (moved verbatim out of logic.rs).

use super::{TrameOutcome, TrameState, col_to_alpha, default_sheet};
use crate::model::Uid;
use crate::{CellPos, Col, ColCount, Row, RowCount};
use crate::{HeaderRow, SortKey, SortOrder, TrameCommand, TrameEvent};
use ambre_bus::{FilterKind, FilterOp, SheetId};

use crate::test_support::{at, span};

/// `rect` as `(r1, c1, r2, c2)`.
fn rect_tuple(rect: crate::CellRect) -> (u32, u32, u32, u32) {
    let (a, b) = (rect.top_left, rect.bottom_right);
    (a.row.get(), a.col.get(), b.row.get(), b.col.get())
}

// The suite is split into groups; shared helpers and imports live here and
// reach each group via `use super::*`.
mod group1;
mod group10;
mod group11;
mod group12;
mod group13;
mod group2;
mod group3;
mod group4;
mod group5;
mod group6;
mod group7;
mod group8;
mod group9;

/// A bold patch over `bold: Some(true)`.
fn bold_patch(on: bool) -> crate::StylePatch {
    crate::StylePatch {
        bold: Some(on),
        ..Default::default()
    }
}

/// Read the text at a single grid cell through a viewport request.
fn read_cell(state: &mut TrameState, row: u32, col: u32) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(999),
        row_start: Row::new(row),
        row_end: Row::new(row),
        col_start: Col::new(col),
        col_end: Col::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].text.clone()
}

/// Read the formula-bar `raw` text at a single grid cell through a viewport
/// request (the reconstructed display form, not the stored canonical form).
fn read_raw(state: &mut TrameState, row: u32, col: u32) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(999),
        row_start: Row::new(row),
        row_end: Row::new(row),
        col_start: Col::new(col),
        col_end: Col::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].raw.clone()
}

/// Read the full DisplayCell at a single grid position.
fn read_display(state: &mut TrameState, row: u32, col: u32) -> crate::DisplayCell {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(5000),
        row_start: Row::new(row),
        row_end: Row::new(row),
        col_start: Col::new(col),
        col_end: Col::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].clone()
}

/// The id of the first non-default sheet in the current tab order.
fn second_sheet_id(state: &TrameState) -> SheetId {
    state
        .sheet_order_ids()
        .into_iter()
        .find(|id| *id != super::default_sheet())
        .expect("a second sheet")
}

/// Read column `col` over rows `[0..=row_end]` as display strings.
fn read_col(state: &mut TrameState, col: u32, row_end: u32, version: u64) -> Vec<String> {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(version),
        row_start: Row::new(0),
        row_end: Row::new(row_end),
        col_start: Col::new(0),
        col_end: Col::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    (0..=row_end)
        .map(|r| {
            cells
                .iter()
                .find(|c| c.row == Row::new(r) && c.col == Col::new(col))
                .map(|c| c.text.clone())
                .unwrap_or_default()
        })
        .collect()
}

/// Whether grid `row` is hidden by the active filter (read through a viewport).
fn is_filtered(state: &mut TrameState, row: u32, col: u32) -> bool {
    read_display(state, row, col).is_filtered
}

/// Seed column A (col 0) with one value per row, top to bottom.
fn seed_col_a(state: &mut TrameState, values: &[&str]) {
    for (r, v) in values.iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: Row::new(r as u32),
            col: Col::new(0),
            text: (*v).to_owned(),
        });
    }
}

/// Seed a few cells on the active sheet for the find/replace tests.
fn seed_find(state: &mut TrameState) {
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(0),
        col: Col::new(0),
        text: "apple".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(1),
        col: Col::new(0),
        text: "Apple pie".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(2),
        col: Col::new(0),
        text: "banana".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Row::new(3),
        col: Col::new(0),
        text: "APPLE".to_owned(),
    });
}

fn find_result(out: &TrameOutcome) -> (Vec<(u32, u32)>, Option<usize>) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::FindResult { matches, active } => Some((
                matches
                    .iter()
                    .map(|at| (at.row.get(), at.col.get()))
                    .collect(),
                *active,
            )),
            _ => None,
        })
        .expect("a FindResult")
}

fn replaced_count(out: &TrameOutcome) -> u32 {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ReplaceDone { replaced } => Some(*replaced),
            _ => None,
        })
        .expect("a ReplaceDone")
}

/// Pull the `ImportComplete` event out of an outcome.
fn import_result(out: &TrameOutcome) -> (SheetId, u32, u32) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ImportComplete {
                sheet_id,
                rows,
                cols,
            } => Some((sheet_id.clone(), rows.get(), cols.get())),
            _ => None,
        })
        .expect("expected ImportComplete")
}

/// Pull the exported file bytes out of an outcome.
fn export_bytes(out: &TrameOutcome) -> Vec<u8> {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ExportComplete { data, .. } => Some(data.clone()),
            _ => None,
        })
        .expect("expected ExportComplete")
}

/// Pull the per-axis metric slices of a single-column-window viewport.
fn viewport_metrics(
    state: &mut TrameState,
    r0: u32,
    r1: u32,
    c0: u32,
    c1: u32,
) -> (Vec<f32>, Vec<f32>) {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_bus::ViewportVersion(99),
        row_start: Row::new(r0),
        row_end: Row::new(r1),
        col_start: Col::new(c0),
        col_end: Col::new(c1),
    });
    let TrameEvent::ViewportData {
        col_widths,
        row_heights,
        ..
    } = evts.display.into_iter().next().expect("ViewportData")
    else {
        panic!("expected ViewportData");
    };
    (col_widths, row_heights)
}
