// ambre-trame/src/ui/grid/tests/group2.rs
// TrameUiState grid geometry/navigation tests. Helpers via `use super::super::*`.

use super::*;
use ambre_commands::SelectionMode;
use ambre_core::SurfacePx;

/// After a merge, the active cell may sit on the block's bottom-right
/// (covered) sub-cell -- the drag-end corner. `snap_active_to_master`
/// re-homes the selection on the master, so the editor targets the top-left
/// rather than a covered cell. A covered cell would reject the write as a
/// merge conflict.
#[test]
fn snap_active_to_master_rehomes_after_merge() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    seed_merge_2x2(&mut state);
    // Simulate the post-merge selection: active on the bottom-right covered
    // sub-cell (2,2), as left by a drag that ended there.
    state.selection = Selection {
        anchor: CellPos::new(Row::new(1), Col::new(1)),
        active: CellPos::new(Row::new(2), Col::new(2)),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert!(state.snap_active_to_master(), "covered active cell snaps");
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(1), Col::new(1)))
    );
    // Already on the master: nothing to do.
    assert!(!state.snap_active_to_master());
}

/// Autofill bug: dragging the fill handle from a merged source produces a grid
/// of merges. The selection then spans every filled block, with the active
/// corner on the last block's covered sub-cell. `snap_active_to_master` must
/// keep the full multi-block selection (only re-homing the active corner onto
/// its block master), not collapse it down to the last block.
#[test]
fn snap_keeps_multi_block_fill_selection() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    let cell = |row, col, span| DisplayCell {
        row,
        col,
        text: String::new(),
        raw: String::new(),
        is_merged: true,
        merge_span: span,
        ..Default::default()
    };
    // Three stacked 2x1 (1 col, 2 rows) merges in column 0: a source block
    // filled down into two more, as a merge-aware autofill leaves them.
    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(0),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(Row::new(0), Col::new(0), Some(span(2, 1))), // master, rows 0..=1
            cell(Row::new(1), Col::new(0), None),
            cell(Row::new(2), Col::new(0), Some(span(2, 1))), // master, rows 2..=3
            cell(Row::new(3), Col::new(0), None),
            cell(Row::new(4), Col::new(0), Some(span(2, 1))), // master, rows 4..=5
            cell(Row::new(5), Col::new(0), None),
        ],
    });
    // Post-fill selection: anchor on the source master, active on the last
    // block's covered bottom sub-cell.
    state.selection = Selection {
        anchor: CellPos::new(Row::new(0), Col::new(0)),
        active: CellPos::new(Row::new(5), Col::new(0)),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert!(
        state.snap_active_to_master(),
        "covered active corner re-homed"
    );
    // The selection still spans all three blocks (rows 0..=4 after re-homing
    // the active corner onto the last master); it did not collapse.
    assert_eq!(
        state.selection.anchor,
        CellPos::new(Row::new(0), Col::new(0))
    );
    assert_eq!(
        state.selection.active,
        CellPos::new(Row::new(4), Col::new(0))
    );
    assert_eq!(state.selection.bounds(), rect(0, 0, 4, 0));
    // expand_to_merges re-grows the last block, so the render covers row 5 too.
    assert_eq!(state.selection_rects(), vec![rect(0, 0, 5, 0)]);
}

/// Feed a single column of cells, flagging `filtered` rows as collapsed
/// (`trame-shared-filter`), then assert the row axis collapses over them. The
/// hidden set, vertical offsets, hit-testing and navigation all skip them.
#[test]
fn filtered_rows_collapse_geometry_and_nav() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    let cell = |row: u32, filtered: bool| DisplayCell {
        row: Row::new(row),
        col: Col::new(0),
        text: format!("v{row}"),
        raw: format!("v{row}"),
        bold: ambre_bus::Weight::Regular,
        italic: ambre_bus::Slant::Upright,
        is_merged: false,
        merge_span: None,
        is_filtered: filtered,
        ..Default::default()
    };
    // Rows 0..=4; rows 2 and 3 are filtered out.
    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(0),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(0, false),
            cell(1, false),
            cell(2, true),
            cell(3, true),
            cell(4, false),
        ],
    });

    assert_eq!(
        state.hidden_rows,
        vec![Row::new(2), Row::new(3)],
        "filtered rows tracked, sorted"
    );
    assert!(state.is_row_hidden(Row::new(2)) && state.is_row_hidden(Row::new(3)));

    // Row 4 draws on the 3rd visible slot (rows 0,1 above it; 2,3 collapsed).
    assert_eq!(state.visible_offset(Row::new(4)), RowDelta::new(2));
    assert!(
        state
            .cell_pixel_rect(CellPos::new(Row::new(2), Col::new(0)))
            .is_none(),
        "a hidden row is not drawn"
    );
    let r1 = state
        .cell_pixel_rect(CellPos::new(Row::new(1), Col::new(0)))
        .expect("row 1 visible");
    let r4 = state
        .cell_pixel_rect(CellPos::new(Row::new(4), Col::new(0)))
        .expect("row 4 visible");
    // Row 4 sits exactly one slot below row 1 (the two hidden rows took none).
    assert_eq!(r4.y0 - r1.y0, f64::from(super::super::ROW_HEIGHT.get()));

    // Hit-testing the second visible slot resolves to row 4, not row 2.
    let CellPos {
        row: hit_row,
        col: _,
    } = state
        .cell_at(pointer(r4.x0 + 1.0, r4.y0 + 1.0))
        .expect("hit");
    assert_eq!(hit_row, Row::new(4));

    // Arrow-down from row 1 jumps straight to row 4, skipping 2 and 3.
    state.select(CellPos::new(Row::new(1), Col::new(0)));
    state
        .move_cursor(RowDelta::new(1), ColDelta::new(0), SelectionMode::Move)
        .expect("moved");
    assert_eq!(state.active_row(), Row::new(4));
    // Arrow-up returns to row 1.
    state
        .move_cursor(RowDelta::new(-1), ColDelta::new(0), SelectionMode::Move)
        .expect("moved");
    assert_eq!(state.active_row(), Row::new(1));
}

// The CellEditor caret / selection / word / IME internals are now the shared
// `ambre_canvas::TextEditor` core. Those semantics are covered by the core's own
// tests and by `editor_parity` (shim -> intent delegation).

/// Absolute moves clamp to the grid and report the new active cell; a move
/// to the current cell is a no-op. End/Ctrl+End scan the local cache for the
/// last cell carrying content (the UI thread's only view of data).
#[test]
fn move_to_and_content_scans() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));

    // No change -> None.
    assert!(
        state
            .move_to(CellPos::new(Row::new(0), Col::new(0)), SelectionMode::Move)
            .is_none()
    );
    // Clamped to the last valid index.
    state
        .move_to(
            CellPos::new(
                crate::LAST_ROW + RowCount::new(10),
                crate::LAST_COL + ColCount::new(10),
            ),
            SelectionMode::Move,
        )
        .expect("moved");
    assert_eq!(
        (state.active_row(), state.active_col()),
        (crate::LAST_ROW, crate::LAST_COL),
    );

    // Empty cache: no content cells.
    assert_eq!(state.last_content_col(Row::new(0)), None);
    assert_eq!(state.last_content_cell(), None);

    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            DisplayCell {
                row: Row::new(2),
                col: Col::new(5),
                text: "x".into(),
                raw: "x".into(),
                bold: ambre_bus::Weight::Regular,
                italic: ambre_bus::Slant::Upright,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
            DisplayCell {
                row: Row::new(2),
                col: Col::new(1),
                text: "y".into(),
                raw: "y".into(),
                bold: ambre_bus::Weight::Regular,
                italic: ambre_bus::Slant::Upright,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
            DisplayCell {
                row: Row::new(4),
                col: Col::new(3),
                text: "".into(),
                raw: "".into(),
                bold: ambre_bus::Weight::Regular,
                italic: ambre_bus::Slant::Upright,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
        ],
    });
    // Rightmost non-empty col in row 2 is col 5; row 4's only cell is empty.
    assert_eq!(state.last_content_col(Row::new(2)), Some(Col::new(5)));
    assert_eq!(state.last_content_col(Row::new(4)), None);
    // Bottom-right non-empty cell ignores the empty (4, 3): max is (2, 5).
    assert_eq!(state.last_content_cell(), Some(at(2, 5)));
}

/// A formula cell arriving in a viewport batch keeps its computed result in
/// `text` for rendering, while preserving the formula source in `raw`. That
/// is the value `cell_seed` hands the editor, so editing a formula shows the
/// formula.
#[test]
fn viewport_data_preserves_formula_raw_for_editing() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![DisplayCell {
            row: Row::new(0),
            col: Col::new(2),
            text: "3".to_owned(),
            raw: "=SUM(A1:B1)".to_owned(),
            bold: ambre_bus::Weight::Regular,
            italic: ambre_bus::Slant::Upright,
            is_merged: false,
            merge_span: None,
            is_filtered: false,
            ..Default::default()
        }],
    });

    let cell = state.cell_cache.get(&at(0, 2)).expect("cell cached");
    assert_eq!(cell.text, "3", "renders the result");
    assert_eq!(
        cell.raw.as_str(),
        "=SUM(A1:B1)",
        "editor reopens on the formula"
    );
}

/// Optimistic commit of a formula must not display the formula source as its
/// own result. `text` stays blank, pending the authoritative ViewportData,
/// while `raw` retains the formula for re-editing. A plain literal shows
/// immediately.
#[test]
fn optimistic_commit_does_not_echo_formula_as_result() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));

    state.set_cell_text(CellPos::new(Row::new(0), Col::new(0)), "5".to_owned());
    let plain = state.cell_cache.get(&at(0, 0)).unwrap();
    assert_eq!(plain.text, "5");
    assert_eq!(plain.raw.as_str(), "5");

    state.set_cell_text(
        CellPos::new(Row::new(0), Col::new(2)),
        "=SUM(A1:B1)".to_owned(),
    );
    let formula = state.cell_cache.get(&at(0, 2)).unwrap();
    assert_eq!(formula.text, "", "result blank until recomputed");
    assert_eq!(
        formula.raw.as_str(),
        "=SUM(A1:B1)",
        "formula kept for editing"
    );
}

/// An optimistic edit marks a cell dirty; the authoritative `ViewportData`
/// for that cell clears the dirty flag. A formula error supersedes dirty and
/// is itself cleared once a valid value arrives.
#[test]
fn dirty_and_error_lifecycle() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));

    state.set_cell_text(CellPos::new(Row::new(0), Col::new(0)), "=A2".to_owned());
    assert!(state.dirty_cells.contains(&at(0, 0)), "edit marks dirty");

    state.set_formula_error(CellPos::new(Row::new(0), Col::new(0)), "#REF!".to_owned());
    assert!(!state.dirty_cells.contains(&at(0, 0)), "error clears dirty");
    assert_eq!(
        state.formula_errors.get(&at(0, 0)).map(String::as_str),
        Some("#REF!")
    );

    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![DisplayCell {
            row: Row::new(0),
            col: Col::new(0),
            text: "42".to_owned(),
            raw: "=A2".to_owned(),
            bold: ambre_bus::Weight::Regular,
            italic: ambre_bus::Slant::Upright,
            is_merged: false,
            merge_span: None,
            is_filtered: false,
            ..Default::default()
        }],
    });
    assert!(
        !state.formula_errors.contains_key(&at(0, 0)),
        "valid value clears error"
    );
    assert!(
        !state.dirty_cells.contains(&at(0, 0)),
        "valid value clears dirty"
    );
}

/// The column-header strip starts at the top of the canvas and the cell area
/// is pushed below it. A point inside the header strip is not a cell.
#[test]
fn header_strip_starts_at_top_of_canvas() {
    let state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));

    // A point inside the header strip is not a cell.
    assert_eq!(
        state.cell_at(pointer(100.0, super::super::HEADER_HEIGHT.get() - 1.0)),
        None
    );
    // The first cell starts just below the header strip.
    assert_eq!(
        state.cell_at(pointer(
            super::super::HEADER_WIDTH.get() + 1.0,
            super::super::HEADER_HEIGHT.get() + 1.0
        )),
        Some(at(0, 0)),
    );
    // (0,0)'s pixel rect sits at the cell-area top-left.
    let y = state
        .cell_pixel_rect(CellPos::new(Row::new(0), Col::new(0)))
        .expect("visible")
        .y0;
    assert_eq!(y, super::super::HEADER_HEIGHT.get());
}

/// On a very short canvas the grid area never inverts: `grid_bottom` floors
/// at the top header height.
#[test]
fn grid_bottom_floors_at_header_on_tiny_canvas() {
    let state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(10.0));
    assert_eq!(state.grid_bottom().get(), super::super::HEADER_HEIGHT.get());
}

/// A screen of narrow columns requests every column it shows
/// (`TRAME-UI-144`): the window is walked from the real widths, not sized from
/// the uniform one.
#[test]
fn scroll_requests_every_narrow_column_on_screen() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    for col in 0..200 {
        state
            .col_widths
            .insert(Col::new(col), ambre_core::DocPx::new(24.0));
    }
    for row in 0..200 {
        state
            .row_heights
            .insert(Row::new(row), ambre_core::DocPx::new(8.0));
    }
    let last_col_on_screen = state
        .col_at_x(view_px(799.0))
        .expect("a column under the right edge");
    let last_row_on_screen = (0..200u32)
        .take_while(|&r| state.row_y(Row::new(r)).get() < state.grid_bottom().get())
        .last()
        .expect("a row on screen");
    let crate::TrameCommand::ScrollViewport {
        col_end, row_end, ..
    } = state.scroll_viewport_cmd()
    else {
        panic!("expected ScrollViewport")
    };
    assert!(
        col_end >= last_col_on_screen,
        "{col_end:?} < {last_col_on_screen:?}"
    );
    assert!(
        row_end >= Row::new(last_row_on_screen),
        "{row_end:?} < {last_row_on_screen}"
    );
}

/// `BUS-017`: an answer older than the one the grid holds is dropped, so the
/// cell keeps the text of the newer answer.
#[test]
fn a_stale_viewport_answer_is_dropped_by_its_version() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    let answer = |version, text: &str| TrameEvent::ViewportData {
        version,
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![DisplayCell {
            row: Row::new(0),
            col: Col::new(0),
            text: text.to_owned(),
            ..Default::default()
        }],
    };
    state.on_viewport_data(answer(ambre_bus::ViewportVersion(5), "new"));
    state.on_viewport_data(answer(ambre_bus::ViewportVersion(3), "old"));
    let text = |state: &TrameUiState| state.cell_cache.get(&at(0, 0)).map(|c| c.text.clone());
    assert_eq!(
        text(&state).as_deref(),
        Some("new"),
        "the older answer is dropped"
    );
    state.on_viewport_data(answer(ambre_bus::ViewportVersion(5), "same"));
    assert_eq!(
        text(&state).as_deref(),
        Some("same"),
        "the same version lands"
    );
}
