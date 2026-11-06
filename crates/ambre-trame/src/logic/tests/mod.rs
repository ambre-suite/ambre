// ambre-trame/src/logic/tests.rs
// Unit tests for `TrameState` (moved verbatim out of logic.rs).

use super::{TrameOutcome, TrameState, col_to_alpha, default_sheet};
use crate::message::{FilterKind, FilterOp};
use crate::model::Uid;
use crate::{CellPos, Col, Row};
use crate::{HeaderRow, SortKey, SortOrder, TrameCommand, TrameEvent};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Pos;

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
mod group14;
mod group2;
mod group3;
mod group4;
mod group5;
mod group6;
mod group7;
mod group8;
mod group9;

/// A patch that sets the bold weight, and nothing else.
fn bold_patch() -> crate::StylePatch {
    crate::StylePatch {
        bold: Some(ambre_core::Weight::Bold),
        ..Default::default()
    }
}

/// Read the text at a single grid cell through a viewport request.
fn read_cell(state: &mut TrameState, row: u32, col: u32) -> String {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(999),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row),
        col_start: Pos::<Col>::new(col),
        col_end: Pos::<Col>::new(col),
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
        version: ambre_core::Seq::nth(999),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row),
        col_start: Pos::<Col>::new(col),
        col_end: Pos::<Col>::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].raw.clone()
}

/// Read the full DisplayCell at a single grid position.
fn read_display(state: &mut TrameState, row: u32, col: u32) -> crate::DisplayCell {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(5000),
        row_start: Pos::<Row>::new(row),
        row_end: Pos::<Row>::new(row),
        col_start: Pos::<Col>::new(col),
        col_end: Pos::<Col>::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    cells[0].clone()
}

/// The id of the first non-default sheet in the current tab order.
fn second_sheet_id(state: &TrameState) -> Id<Sheet> {
    state
        .sheet_order_ids()
        .into_iter()
        .find(|id| *id != super::default_sheet())
        .expect("a second sheet")
}

/// Read column `col` over rows `[0..=row_end]` as display strings.
fn read_col(state: &mut TrameState, col: u32, row_end: u32, version: u64) -> Vec<String> {
    let evts = state.handle_command(TrameCommand::ScrollViewport {
        version: ambre_core::Seq::nth(version),
        row_start: Pos::<Row>::new(0),
        row_end: Pos::<Row>::new(row_end),
        col_start: Pos::<Col>::new(0),
        col_end: Pos::<Col>::new(col),
    });
    let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
        panic!()
    };
    (0..=row_end)
        .map(|r| {
            cells
                .iter()
                .find(|c| c.row == Pos::<Row>::new(r) && c.col == Pos::<Col>::new(col))
                .map(|c| c.text.clone())
                .unwrap_or_default()
        })
        .collect()
}

/// Whether grid `row` is hidden by the active filter (read through a viewport).
fn is_filtered(state: &mut TrameState, row: u32, col: u32) -> bool {
    read_display(state, row, col).is_filtered == crate::message::RowFilter::Filtered
}

/// Seed column A (col 0) with one value per row, top to bottom.
fn seed_col_a(state: &mut TrameState, values: &[&str]) {
    for (r, v) in values.iter().enumerate() {
        state.handle_command(TrameCommand::SetCellText {
            row: Pos::<Row>::new(r as u32),
            col: Pos::<Col>::new(0),
            text: (*v).to_owned(),
        });
    }
}

/// The query a find panel sends for the typed `text`: none when it is empty.
fn find_query(
    text: String,
    options: crate::FindOptions,
) -> Option<ambre_bus::app::find::FindQuery<crate::FindOptions>> {
    ambre_bus::app::find::FindText::new(text)
        .map(|text| ambre_bus::app::find::FindQuery::new(text, options))
}

/// Seed a few cells on the active sheet for the find/replace tests.
fn seed_find(state: &mut TrameState) {
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(0),
        col: Pos::<Col>::new(0),
        text: "apple".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(1),
        col: Pos::<Col>::new(0),
        text: "Apple pie".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(2),
        col: Pos::<Col>::new(0),
        text: "banana".to_owned(),
    });
    state.handle_command(TrameCommand::SetCellText {
        row: Pos::<Row>::new(3),
        col: Pos::<Col>::new(0),
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
                active.map(Pos::<ambre_bus::app::find::Match>::to_usize),
            )),
            _ => None,
        })
        .expect("a FindResult")
}

fn replaced_count(out: &TrameOutcome) -> u32 {
    out.display
        .iter()
        .find_map(|e| match e {
            TrameEvent::ReplaceDone { replaced } => Some(replaced.get()),
            _ => None,
        })
        .expect("a ReplaceDone")
}

/// Pull the `ImportComplete` event out of an outcome.
fn import_result(out: &TrameOutcome) -> (Id<Sheet>, u32, u32) {
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
            TrameEvent::ExportComplete { data, .. } => Some(data.as_bytes().to_vec()),
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
        version: ambre_core::Seq::nth(99),
        row_start: Pos::<Row>::new(r0),
        row_end: Pos::<Row>::new(r1),
        col_start: Pos::<Col>::new(c0),
        col_end: Pos::<Col>::new(c1),
    });
    let TrameEvent::ViewportData {
        col_widths,
        row_heights,
        ..
    } = evts.display.into_iter().next().expect("ViewportData")
    else {
        panic!("expected ViewportData");
    };
    (
        col_widths.iter().map(|w| w.get()).collect(),
        row_heights.iter().map(|h| h.get()).collect(),
    )
}
