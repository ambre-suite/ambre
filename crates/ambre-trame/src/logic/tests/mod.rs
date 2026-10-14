// ambre-trame/src/logic/tests.rs
// Unit tests for `TrameState` (moved verbatim out of logic.rs).

use super::{TrameOutcome, TrameState, col_to_alpha};
use crate::model::Uid;
use crate::{SortKey, TrameCommand, TrameEvent};
use ambre_bus::{FilterKind, FilterOp};

const TEST_SHEET: &str = "s0";

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
        version: 999,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
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
        version: 999,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].raw.clone()
}

/// Read the full DisplayCell at a single grid position.
fn read_display(state: &mut TrameState, row: u32, col: u32) -> crate::DisplayCell {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: 5000,
        row_start: row,
        row_end: row,
        col_start: col,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].clone()
}

/// The id of the first non-default sheet in the current tab order.
fn second_sheet_id(state: &TrameState) -> String {
    state
        .sheet_order_ids()
        .into_iter()
        .find(|id| id != super::DEFAULT_SHEET)
        .expect("a second sheet")
}

/// Read column `col` over rows `[0..=row_end]` as display strings.
fn read_col(state: &mut TrameState, col: u32, row_end: u32, version: u64) -> Vec<String> {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version,
        row_start: 0,
        row_end,
        col_start: 0,
        col_end: col,
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    (0..=row_end)
        .map(|r| {
            cells
                .iter()
                .find(|c| c.row == r && c.col == col)
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
            row: r as u32,
            col: 0,
            text: (*v).to_owned(),
        });
    }
}

/// Seed a few cells on the active sheet for the find/replace tests.
fn seed_find(state: &mut TrameState) {
    state.handle_command(TrameCommand::SetCellText {
        row: 0,
        col: 0,
        text: "apple".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 1,
        col: 0,
        text: "Apple pie".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 2,
        col: 0,
        text: "banana".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: 3,
        col: 0,
        text: "APPLE".to_owned(),
    });
}

fn find_result(out: &TrameOutcome) -> (Vec<(u32, u32)>, Option<usize>) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::FindResult { matches, active } => Some((matches.clone(), *active)),
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
fn import_result(out: &TrameOutcome) -> (String, u32, u32) {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ImportComplete {
                sheet_id,
                rows,
                cols,
            } => Some((sheet_id.clone(), *rows, *cols)),
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
        version: 99,
        row_start: r0,
        row_end: r1,
        col_start: c0,
        col_end: c1,
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
