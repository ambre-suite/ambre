// ambre-trame/src/ui/grid/tests/group2.rs
// TrameUiState grid geometry/navigation tests. Helpers via `use super::super::*`.

use super::*;

/// After a merge, the active cell may sit on the block's bottom-right
/// (covered) sub-cell -- the drag-end corner. `snap_active_to_master`
/// re-homes the selection on the master so the editor targets the top-left,
/// not a covered cell (which would reject the write as a merge conflict).
#[test]
fn snap_active_to_master_rehomes_after_merge() {
    let mut state = TrameUiState::new(800.0, 600.0);
    seed_merge_2x2(&mut state);
    // Simulate the post-merge selection: active on the bottom-right covered
    // sub-cell (2,2), as left by a drag that ended there.
    state.selection = Selection {
        anchor: (1, 1),
        active: (2, 2),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert!(state.snap_active_to_master(), "covered active cell snaps");
    assert_eq!(state.selection, Selection::single(1, 1));
    // Already on the master: nothing to do.
    assert!(!state.snap_active_to_master());
}

/// Autofill bug: dragging the fill handle from a merged source produces a
/// grid of merges, leaving the selection spanning every filled block with the
/// active corner on the last block's covered sub-cell. `snap_active_to_master`
/// must keep the full multi-block selection (only re-homing the active corner
/// onto its block master), not collapse it down to the last block.
#[test]
fn snap_keeps_multi_block_fill_selection() {
    let mut state = TrameUiState::new(800.0, 600.0);
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
        version: 0,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(0, 0, Some((1, 2))), // master, rows 0..=1
            cell(1, 0, None),
            cell(2, 0, Some((1, 2))), // master, rows 2..=3
            cell(3, 0, None),
            cell(4, 0, Some((1, 2))), // master, rows 4..=5
            cell(5, 0, None),
        ],
    });
    // Post-fill selection: anchor on the source master, active on the last
    // block's covered bottom sub-cell.
    state.selection = Selection {
        anchor: (0, 0),
        active: (5, 0),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert!(
        state.snap_active_to_master(),
        "covered active corner re-homed"
    );
    // The selection still spans all three blocks (rows 0..=4 after re-homing
    // the active corner onto the last master); it did not collapse.
    assert_eq!(state.selection.anchor, (0, 0));
    assert_eq!(state.selection.active, (4, 0));
    assert_eq!(state.selection.bounds(), (0, 0, 4, 0));
    // expand_to_merges re-grows the last block, so the render covers row 5 too.
    assert_eq!(state.selection_rects(), vec![(0, 0, 5, 0)]);
}

/// Feed a single column of cells, flagging `filtered` rows as collapsed
/// (`-ui.md` sec.5.D), then assert the row axis collapses over them: the
/// hidden set, vertical offsets, hit-testing, and navigation all skip them.
#[test]
fn filtered_rows_collapse_geometry_and_nav() {
    let mut state = TrameUiState::new(800.0, 600.0);
    let cell = |row: u32, filtered: bool| DisplayCell {
        row,
        col: 0,
        text: format!("v{row}"),
        raw: format!("v{row}"),
        bold: false,
        italic: false,
        is_merged: false,
        merge_span: None,
        is_filtered: filtered,
        ..Default::default()
    };
    // Rows 0..=4; rows 2 and 3 are filtered out.
    state.on_viewport_data(TrameEvent::ViewportData {
        version: 0,
        row_start: 0,
        col_start: 0,
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
        vec![2, 3],
        "filtered rows tracked, sorted"
    );
    assert!(state.is_row_hidden(2) && state.is_row_hidden(3));

    // Row 4 draws on the 3rd visible slot (rows 0,1 above it; 2,3 collapsed).
    assert_eq!(state.visible_offset(4), 2);
    assert!(
        state.cell_pixel_rect(2, 0).is_none(),
        "a hidden row is not drawn"
    );
    let r1 = state.cell_pixel_rect(1, 0).expect("row 1 visible");
    let r4 = state.cell_pixel_rect(4, 0).expect("row 4 visible");
    // Row 4 sits exactly one slot below row 1 (the two hidden rows took none).
    assert_eq!(r4.1 - r1.1, super::super::ROW_HEIGHT);

    // Hit-testing the second visible slot resolves to row 4, not row 2.
    let (hit_row, _) = state.cell_at(r4.0 + 1.0, r4.1 + 1.0).expect("hit");
    assert_eq!(hit_row, 4);

    // Arrow-down from row 1 jumps straight to row 4, skipping 2 and 3.
    state.select(1, 0);
    state.move_cursor(1, 0, false).expect("moved");
    assert_eq!(state.active_row(), 4);
    // Arrow-up returns to row 1.
    state.move_cursor(-1, 0, false).expect("moved");
    assert_eq!(state.active_row(), 1);
}

/// Shift+arrows grow the selection from a seeded mark; a bare arrow collapses
/// it to the matching edge; typing and Delete replace the selected range.
#[test]
fn cell_editor_text_selection() {
    let mut ed = CellEditor::new(0, 0, "hello world".to_owned());
    assert_eq!(ed.caret, 11);
    assert_eq!(ed.selection(), None);

    // Shift+Left twice selects the last two chars ("ld").
    ed.extend_left();
    ed.extend_left();
    assert_eq!(ed.selection(), Some((9, 11)));
    assert_eq!(ed.selected_text(), "ld");

    // A bare Left collapses to the selection's left edge (no extra move).
    ed.move_left();
    assert_eq!(ed.caret, 9);
    assert_eq!(ed.selection(), None);

    // Home..Shift? Extend to start, then replace the run by typing.
    ed.extend_home();
    assert_eq!(ed.selected_text(), "hello wor");
    ed.insert("HI ");
    assert_eq!(ed.buffer, "HI ld");
    assert_eq!(ed.caret, 3);

    // Delete with a selection removes it; without one, the next char.
    ed.select_all();
    assert!(ed.delete_forward());
    assert_eq!(ed.buffer, "");
    assert!(!ed.delete_forward());
}

/// Double-click word selection spans the alphanumeric run under the hit and
/// Ctrl+A grabs the whole buffer.
#[test]
fn cell_editor_word_and_select_all() {
    let mut ed = CellEditor::new(0, 0, "foo bar baz".to_owned());
    ed.select_word(5); // inside "bar"
    assert_eq!(ed.selected_text(), "bar");

    ed.select_all();
    assert_eq!(ed.selection(), Some((0, 11)));
    ed.backspace();
    assert_eq!(ed.buffer, "");
}

/// Absolute moves clamp to the grid and report the new active cell; a move
/// to the current cell is a no-op. End/Ctrl+End scan the local cache for the
/// last cell carrying content (the UI thread's only view of data).
#[test]
fn move_to_and_content_scans() {
    let mut state = TrameUiState::new(800.0, 600.0);

    // No change -> None.
    assert!(state.move_to(0, 0, false).is_none());
    // Clamped to the last valid index.
    state
        .move_to(crate::TOTAL_ROWS + 10, crate::TOTAL_COLS + 10, false)
        .expect("moved");
    assert_eq!(
        (state.active_row(), state.active_col()),
        (crate::TOTAL_ROWS - 1, crate::TOTAL_COLS - 1),
    );

    // Empty cache: no content cells.
    assert_eq!(state.last_content_col(0), None);
    assert_eq!(state.last_content_cell(), None);

    state.on_viewport_data(TrameEvent::ViewportData {
        version: 1,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            DisplayCell {
                row: 2,
                col: 5,
                text: "x".into(),
                raw: "x".into(),
                bold: false,
                italic: false,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
            DisplayCell {
                row: 2,
                col: 1,
                text: "y".into(),
                raw: "y".into(),
                bold: false,
                italic: false,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
            DisplayCell {
                row: 4,
                col: 3,
                text: "".into(),
                raw: "".into(),
                bold: false,
                italic: false,
                is_merged: false,
                merge_span: None,
                is_filtered: false,
                ..Default::default()
            },
        ],
    });
    // Rightmost non-empty col in row 2 is col 5; row 4's only cell is empty.
    assert_eq!(state.last_content_col(2), Some(5));
    assert_eq!(state.last_content_col(4), None);
    // Bottom-right non-empty cell ignores the empty (4, 3): max is (2, 5).
    assert_eq!(state.last_content_cell(), Some((2, 5)));
}

/// A formula cell arriving in a viewport batch keeps its computed result in
/// `text` for rendering while preserving the formula source in `raw` -- the
/// value `cell_seed` hands the editor so editing a formula shows the formula.
#[test]
fn viewport_data_preserves_formula_raw_for_editing() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.on_viewport_data(TrameEvent::ViewportData {
        version: 1,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![DisplayCell {
            row: 0,
            col: 2,
            text: "3".to_owned(),
            raw: "=SUM(A1:B1)".to_owned(),
            bold: false,
            italic: false,
            is_merged: false,
            merge_span: None,
            is_filtered: false,
            ..Default::default()
        }],
    });

    let cell = state.cell_cache.get(&(0, 2)).expect("cell cached");
    assert_eq!(cell.text, "3", "renders the result");
    assert_eq!(cell.raw, "=SUM(A1:B1)", "editor reopens on the formula");
}

/// Optimistic commit of a formula must not display the formula source as its
/// own result: `text` stays blank (pending the authoritative ViewportData)
/// while `raw` retains the formula for re-editing. A plain literal shows
/// immediately.
#[test]
fn optimistic_commit_does_not_echo_formula_as_result() {
    let mut state = TrameUiState::new(800.0, 600.0);

    state.set_cell_text(0, 0, "5".to_owned());
    let plain = state.cell_cache.get(&(0, 0)).unwrap();
    assert_eq!(plain.text, "5");
    assert_eq!(plain.raw, "5");

    state.set_cell_text(0, 2, "=SUM(A1:B1)".to_owned());
    let formula = state.cell_cache.get(&(0, 2)).unwrap();
    assert_eq!(formula.text, "", "result blank until recomputed");
    assert_eq!(formula.raw, "=SUM(A1:B1)", "formula kept for editing");
}

/// An optimistic edit marks a cell dirty; the authoritative `ViewportData`
/// for that cell clears the dirty flag. A formula error supersedes dirty and
/// is itself cleared once a valid value arrives.
#[test]
fn dirty_and_error_lifecycle() {
    let mut state = TrameUiState::new(800.0, 600.0);

    state.set_cell_text(0, 0, "=A2".to_owned());
    assert!(state.dirty_cells.contains(&(0, 0)), "edit marks dirty");

    state.set_formula_error(0, 0, "#REF!".to_owned());
    assert!(!state.dirty_cells.contains(&(0, 0)), "error clears dirty");
    assert_eq!(
        state.formula_errors.get(&(0, 0)).map(String::as_str),
        Some("#REF!")
    );

    state.on_viewport_data(TrameEvent::ViewportData {
        version: 1,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![DisplayCell {
            row: 0,
            col: 0,
            text: "42".to_owned(),
            raw: "=A2".to_owned(),
            bold: false,
            italic: false,
            is_merged: false,
            merge_span: None,
            is_filtered: false,
            ..Default::default()
        }],
    });
    assert!(
        !state.formula_errors.contains_key(&(0, 0)),
        "valid value clears error"
    );
    assert!(
        !state.dirty_cells.contains(&(0, 0)),
        "valid value clears dirty"
    );
}

/// Insertion advances the caret; backspace and caret motion respect UTF-8
/// boundaries (no byte-splitting a multi-byte char).
#[test]
fn editor_insert_backspace_caret_motion() {
    let mut e = CellEditor::new(0, 0, String::new());
    e.insert("ab");
    assert_eq!(e.buffer, "ab");
    assert_eq!(e.caret, 2);

    e.move_left();
    e.insert("X"); // between a and b
    assert_eq!(e.buffer, "aXb");
    assert_eq!(e.caret, 2);

    // Multi-byte: 'e' acute is 2 bytes; backspace removes the whole char.
    let mut e = CellEditor::new(0, 0, "caf\u{e9}".to_owned());
    assert_eq!(e.caret, 5);
    e.backspace();
    assert_eq!(e.buffer, "caf");
    assert_eq!(e.caret, 3);
}

/// The column-header strip starts at the top of the canvas and the cell area
/// is pushed below it. A point inside the header strip is not a cell.
#[test]
fn header_strip_starts_at_top_of_canvas() {
    let state = TrameUiState::new(800.0, 600.0);

    // A point inside the header strip is not a cell.
    assert_eq!(
        state.cell_at(100.0, super::super::HEADER_HEIGHT - 1.0),
        None
    );
    // The first cell starts just below the header strip.
    assert_eq!(
        state.cell_at(
            super::super::HEADER_WIDTH + 1.0,
            super::super::HEADER_HEIGHT + 1.0
        ),
        Some((0, 0)),
    );
    // (0,0)'s pixel rect sits at the cell-area top-left.
    let (_, y, _, _) = state.cell_pixel_rect(0, 0).expect("visible");
    assert_eq!(y, super::super::HEADER_HEIGHT);
}

/// On a very short canvas the grid area never inverts: `grid_bottom` floors
/// at the top header height.
#[test]
fn grid_bottom_floors_at_header_on_tiny_canvas() {
    let state = TrameUiState::new(800.0, 10.0);
    assert_eq!(state.grid_bottom(), super::super::HEADER_HEIGHT);
}

/// The display string folds the IME preview in at the caret; `before_caret`
/// covers prefix + preview (where the caret and IME popup sit).
#[test]
fn editor_preview_display() {
    let mut e = CellEditor::new(0, 0, "ac".to_owned());
    e.caret = 1; // between a and c
    e.preview = "b".to_owned();
    assert_eq!(e.display(), "abc");
    assert_eq!(e.before_caret(), "ab");
}
