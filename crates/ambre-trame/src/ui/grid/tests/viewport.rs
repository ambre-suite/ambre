//! The viewport's state machine: the request window, the eviction, the
//! version guard and the metric adoption, driven without a view.

use super::super::{COL_WIDTH, PADDING_COLS, PADDING_ROWS, ROW_HEIGHT};
use super::*;
use crate::TrameCommand;

/// A cell at `(row, col)`, hidden by the filter when `filtered`.
fn cell(row: u32, col: u32, filtered: bool) -> DisplayCell {
    DisplayCell {
        row,
        col,
        text: format!("{row}"),
        is_filtered: filtered,
        ..Default::default()
    }
}

/// An answer carrying these cells, with no size of its own.
fn answer(version: u64, cells: Vec<DisplayCell>) -> TrameEvent {
    TrameEvent::ViewportData {
        version,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells,
    }
}

/// The window one request names, as `(row_start, row_end, col_start, col_end)`.
fn window(cmd: TrameCommand) -> (u32, u32, u32, u32) {
    match cmd {
        TrameCommand::ScrollViewport {
            row_start,
            row_end,
            col_start,
            col_end,
            ..
        } => (row_start, row_end, col_start, col_end),
        other => panic!("not a viewport request: {other:?}"),
    }
}

/// `TRAME-UI-004`: the request is the screen plus a bounded padding. A
/// hidden row far above the screen stretches nothing.
#[test]
fn a_scroll_requests_the_screen_and_a_bounded_padding() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.on_viewport_data(answer(0, (0..300).map(|row| cell(row, 0, true)).collect()));
    assert!(state.scroll_by(1_000, 0));
    let (row_start, row_end, col_start, col_end) = window(state.scroll_viewport_cmd());
    let pad = state.viewport_rows.max(PADDING_ROWS);
    // A screen holds one partial row past `viewport_rows`.
    let rows_on_screen = state.viewport_rows + 1;
    assert_eq!(row_start, state.top_row - pad);
    assert!(
        row_end - row_start <= 2 * pad + rows_on_screen,
        "{row_start}..={row_end} for a screen of {rows_on_screen} rows"
    );
    assert!(
        row_end >= state.top_row + state.viewport_rows,
        "the screen is covered"
    );
    let cols_on_screen = state.viewport_cols + 1;
    assert_eq!(col_start, 0);
    assert!(col_end - col_start <= 2 * PADDING_COLS + cols_on_screen);
}

/// `TRAME-UI-024`: after a request the cache holds no cell outside its window.
#[test]
fn a_scroll_evicts_every_cell_outside_the_window() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.on_viewport_data(answer(0, vec![cell(0, 0, false), cell(0, 200, false)]));
    state.on_viewport_data(answer(1, vec![cell(5_000, 0, false)]));
    assert!(state.scroll_by(5_000, 0));
    let (row_start, row_end, col_start, col_end) = window(state.scroll_viewport_cmd());
    assert!(
        state.cell_cache.contains_key(&(5_000, 0)),
        "the screen stays"
    );
    for &(row, col) in state.cell_cache.keys() {
        assert!(
            (row_start..=row_end).contains(&row) && (col_start..=col_end).contains(&col),
            "({row}, {col}) outside {row_start}..={row_end} x {col_start}..={col_end}"
        );
    }
}

/// `TRAME-UI-145`: a scroll moves the origin and nothing the user selected.
#[test]
fn a_scroll_keeps_the_active_cell_and_the_selection() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.selection = Selection {
        anchor: (2, 1),
        active: (4, 3),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    let before = state.selection.clone();
    assert!(state.scroll_by(500, 40));
    state.scroll_viewport_cmd();
    assert!(state.scroll_by(-200, -10));
    assert_eq!(state.selection, before);
    assert_eq!((state.active_row(), state.active_col()), (4, 3));
}

/// `TRAME-UI-148`: a size an answer reports at the default reads as the
/// default, not as the size the axis held before.
#[test]
fn a_default_size_in_an_answer_clears_the_stored_one() {
    let mut state = TrameUiState::new(800.0, 600.0);
    let sized = |version, width: f64, height: f64| TrameEvent::ViewportData {
        version,
        row_start: 3,
        col_start: 2,
        col_widths: vec![width as f32],
        row_heights: vec![height as f32],
        cells: vec![],
    };
    state.on_viewport_data(sized(0, 200.0, 45.0));
    assert_eq!(state.col_width_of(2), 200.0);
    assert_eq!(state.row_height_of(3), 45.0);
    state.on_viewport_data(sized(1, COL_WIDTH, ROW_HEIGHT));
    assert_eq!(state.col_width_of(2), COL_WIDTH);
    assert_eq!(state.row_height_of(3), ROW_HEIGHT);
    assert!(!state.col_widths.contains_key(&2) && !state.row_heights.contains_key(&3));
}

/// `TRAME-UI-149`: an answer writes the caches and leaves the origin where
/// the user put it.
#[test]
fn an_answer_does_not_move_the_scroll_origin() {
    let mut state = TrameUiState::new(800.0, 600.0);
    assert!(state.scroll_by(120, 7));
    let origin = (state.top_row, state.left_col);
    state.on_viewport_data(answer(
        0,
        (0..400).map(|row| cell(row, 0, row % 2 == 0)).collect(),
    ));
    assert_eq!((state.top_row, state.left_col), origin);
}
