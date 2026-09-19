// ambre-trame/src/ui/grid/tests/group1.rs
// TrameUiState grid geometry/navigation tests. Helpers via `use super::super::*`.

use super::*;

/// `current_region` returns `None` on an isolated empty active cell and the
/// bounding rectangle of an L-shaped contiguous block otherwise (`trame-shared-filter`).
#[test]
fn current_region_bounds_the_contiguous_block() {
    let mut state = TrameUiState::new(800.0, 600.0);
    // Isolated empty active cell -> no block to filter.
    assert_eq!(state.current_region(), None);

    // L-shaped block around the active cell (0,0): (0,0),(0,1),(1,0).
    put(&mut state, 0, 0, "a");
    put(&mut state, 0, 1, "b");
    put(&mut state, 1, 0, "c");
    // The region is the bounding rect of the contiguous block.
    assert_eq!(state.current_region(), Some((0, 0, 1, 1)));

    // A far-away non-contiguous cell is excluded.
    put(&mut state, 10, 10, "z");
    assert_eq!(state.current_region(), Some((0, 0, 1, 1)));
}

/// `filter_button_at` hit-tests the autofilter button drawn at the right edge
/// of each current-region header cell, returning `(band_row, col)` (`trame-shared-filter`).
#[test]
fn filter_button_hit_test_matches_drawn_rect() {
    let mut state = TrameUiState::new(800.0, 600.0);
    put(&mut state, 0, 0, "a");
    put(&mut state, 0, 1, "b");
    // (r1, c1, r2, c2): a one-row, two-column block.
    assert_eq!(state.current_region(), Some((0, 0, 0, 1)));

    // The button for column 1 sits at the right edge of cell (0, 1). Hitting
    // its centre resolves to that column on the region's header row.
    let (bx, by, w, h) = state.filter_button_rect(0, 1).expect("button visible");
    assert_eq!(
        state.filter_button_at(bx + w / 2.0, by + h / 2.0),
        Some((0, 1))
    );
    // A point well clear of any button misses.
    assert_eq!(state.filter_button_at(bx + 400.0, by + h / 2.0), None);
}

/// The autofilter button scales with the grid zoom (it used to keep a fixed
/// device size, shrinking visually as the zoomed cells grew around it).
#[test]
fn filter_button_scales_with_zoom() {
    let mut state = TrameUiState::new(800.0, 600.0);
    put(&mut state, 0, 0, "a");
    let (.., w100, _) = state.filter_button_rect(0, 0).expect("button visible");

    state.set_zoom(2.0);
    let (.., w200, _) = state.filter_button_rect(0, 0).expect("button visible");
    // At 2x the cell is taller than the un-clamped button, so the side is the
    // zoom-scaled base: strictly larger than at 100%.
    assert!(w200 > w100, "button must grow with zoom: {w100} -> {w200}");
}

/// A pointer hit collapses the selection to a single cell (`anchor ==
/// active`); keyboard navigation moves the single-cell selection and never
/// leaves a stale anchor behind.
#[test]
fn single_cell_selection_tracks_active() {
    let mut state = TrameUiState::new(800.0, 600.0);
    assert_eq!(state.selection, Selection::single(0, 0));

    state.select(5, 3);
    assert_eq!(state.selection, Selection::single(5, 3));
    assert_eq!((state.active_row(), state.active_col()), (5, 3));

    // Arrow navigation keeps the selection single-cell on the new active cell.
    state.move_cursor(1, 0, false).expect("cursor moved");
    assert_eq!(state.selection, Selection::single(6, 3));
    assert_eq!(state.selection.anchor, state.selection.active);
}

/// `col_at_x` / `row_at_y` resolve the axis band under a pointer through the
/// variable metrics (drives a whole-axis header drag, `trame-selection`).
#[test]
fn axis_pointer_resolution_follows_variable_metrics() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.col_widths.insert(0, 160.0); // col 0 widened
    // x just past the wide col 0 lands in col 1.
    assert_eq!(
        state.col_at_x(super::super::HEADER_WIDTH + 160.0 + 5.0),
        Some(1)
    );
    assert_eq!(state.col_at_x(super::super::HEADER_WIDTH + 10.0), Some(0));
    assert_eq!(state.col_at_x(0.0), None); // over the row-header strip

    state.row_heights.insert(0, 50.0);
    assert_eq!(
        state.row_at_y(super::super::HEADER_HEIGHT + 50.0 + 5.0),
        Some(1)
    );
    assert_eq!(state.row_at_y(super::super::HEADER_HEIGHT + 10.0), Some(0));
    assert_eq!(state.row_at_y(0.0), None);
}

/// The insert "+" zone sits in the top/left sliver of a boundary band, the
/// resize grip in the rest (`trame-resize`, `trame-headers`).
#[test]
fn header_boundary_splits_insert_and_resize() {
    let state = TrameUiState::new(800.0, 600.0);
    let boundary = super::super::HEADER_WIDTH + super::super::COL_WIDTH; // col 0's right edge
    // Lower half -> resize col 0.
    assert_eq!(
        state.header_hit(boundary, super::super::HEADER_HEIGHT * 0.8),
        Some(HeaderHit::ColBorder(0))
    );
    // Upper half -> insert left of col 1 (at this boundary).
    assert_eq!(
        state.header_hit(boundary, 2.0),
        Some(HeaderHit::ColInsert(1))
    );
}

/// `header_hit` resolves the three zones of each header strip plus the
/// corner, with the resize band winning over the body (`trame-headers`).
#[test]
fn header_hit_resolves_zones() {
    let state = TrameUiState::new(800.0, 600.0);
    // Top-left corner.
    assert_eq!(state.header_hit(10.0, 10.0), Some(HeaderHit::Corner));
    // Column 0 body: header strip, x mid-column 0 ([50, 130)).
    assert_eq!(
        state.header_hit(super::super::HEADER_WIDTH + 30.0, 5.0),
        Some(HeaderHit::ColBody(0))
    );
    // Column 0 right border (~130), lower half of the band, arms a resize.
    assert_eq!(
        state.header_hit(
            super::super::HEADER_WIDTH + super::super::COL_WIDTH,
            super::super::HEADER_HEIGHT * 0.8
        ),
        Some(HeaderHit::ColBorder(0))
    );
    // Row 1 body (rows start at HEADER_HEIGHT, each ROW_HEIGHT tall).
    assert_eq!(
        state.header_hit(
            10.0,
            super::super::HEADER_HEIGHT + super::super::ROW_HEIGHT + 5.0
        ),
        Some(HeaderHit::RowBody(1))
    );
    // Row 0 bottom border, right half of the band, resizes row 0.
    assert_eq!(
        state.header_hit(
            super::super::HEADER_WIDTH * 0.8,
            super::super::HEADER_HEIGHT + super::super::ROW_HEIGHT
        ),
        Some(HeaderHit::RowBorder(0))
    );
    // A point in the grid body is not a header hit.
    assert_eq!(
        state.header_hit(
            super::super::HEADER_WIDTH + 10.0,
            super::super::HEADER_HEIGHT + 10.0
        ),
        None
    );
}

/// `Selection::axis` builds a whole-column / whole-row band whose perpendicular
/// coordinate is irrelevant (`trame-selection`).
#[test]
fn axis_selection_spans_the_band() {
    let cols = Selection::axis(SelKind::Cols, 2, 4);
    assert_eq!(cols.kind, SelKind::Cols);
    let (_, c1, _, c2) = cols.bounds();
    assert_eq!((c1, c2), (2, 4));

    let rows = Selection::axis(SelKind::Rows, 5, 3);
    assert_eq!(rows.kind, SelKind::Rows);
    let (r1, _, r2, _) = rows.bounds();
    assert_eq!((r1, r2), (3, 5));
}

/// `Selection::report` carries a whole-axis band as its kind, with no extent
/// across the other axis. The Logic thread resolves that extent from the used
/// range (`TRAME-UI-033`).
#[test]
fn axis_report_carries_the_band_not_the_cache() {
    let cols = Selection::axis(SelKind::Cols, 2, 4).report();
    match cols {
        crate::TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            extra,
            kind,
        } => {
            assert_eq!((row, col), (0, 4), "active is the clicked column");
            assert_eq!((anchor_row, anchor_col), (0, 2), "anchor is the band start");
            assert!(extra.is_empty(), "a band commits no disjoint rectangles");
            assert_eq!(kind, crate::SelectionKind::Cols);
        }
        other => panic!("expected SetActiveCell, got {other:?}"),
    }

    let rows = Selection::axis(SelKind::Rows, 5, 3).report();
    match rows {
        crate::TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            kind,
            ..
        } => {
            assert_eq!((row, col), (3, 0));
            assert_eq!((anchor_row, anchor_col), (5, 0));
            assert_eq!(kind, crate::SelectionKind::Rows);
        }
        other => panic!("expected SetActiveCell, got {other:?}"),
    }
}

/// A resized column / row shifts the pixel mapping: `cell_at` and
/// `cell_pixel_rect` must agree on the variable geometry (`trame-resize`).
#[test]
fn variable_metrics_round_trip_pixel_and_hit_test() {
    let mut state = TrameUiState::new(800.0, 600.0);
    // Col 0 widened to 160, row 0 heightened to 40.
    state.col_widths.insert(0, 160.0);
    state.row_heights.insert(0, 40.0);

    assert_eq!(state.col_width_of(0), 160.0);
    assert_eq!(state.col_width_of(1), super::super::COL_WIDTH);
    assert_eq!(state.row_height_of(0), 40.0);

    // Cell (1,1) sits after the wide col 0 and below the tall row 0.
    let (x, y, w, h) = state.cell_pixel_rect(1, 1).expect("visible");
    assert_eq!(x, super::super::HEADER_WIDTH + 160.0);
    assert_eq!(y, super::super::HEADER_HEIGHT + 40.0);
    assert_eq!((w, h), (super::super::COL_WIDTH, super::super::ROW_HEIGHT));

    // Hit-testing the interior of that cell returns it.
    assert_eq!(state.cell_at(x + 5.0, y + 5.0), Some((1, 1)));
    // A point inside the tall row 0 / wide col 0 maps to (0, 0).
    assert_eq!(
        state.cell_at(
            super::super::HEADER_WIDTH + 100.0,
            super::super::HEADER_HEIGHT + 30.0
        ),
        Some((0, 0))
    );
}

/// Grid zoom scales every metric (header strips, cell widths/heights) so the
/// pixel mapping and hit-testing stay self-consistent (`trame-status-bar`).
#[test]
fn zoom_scales_metrics_and_round_trips_hit_test() {
    let mut state = TrameUiState::new(800.0, 600.0);
    assert!(state.set_zoom(2.0));
    assert_eq!(state.zoom, 2.0);

    // Header strips and the default cell metrics double.
    assert_eq!(state.header_w(), super::super::HEADER_WIDTH * 2.0);
    assert_eq!(state.header_h(), super::super::HEADER_HEIGHT * 2.0);
    assert_eq!(state.col_width_of(0), super::super::COL_WIDTH * 2.0);
    assert_eq!(state.row_height_of(0), super::super::ROW_HEIGHT * 2.0);

    // Column 1's left edge is the (doubled) header plus one (doubled) column.
    assert_eq!(
        state.col_x(1),
        super::super::HEADER_WIDTH * 2.0 + super::super::COL_WIDTH * 2.0
    );

    // A point inside cell (2, 2) at 200% maps back to it.
    let x = state.header_w() + super::super::COL_WIDTH * 2.0 * 2.0 + 1.0;
    let y = state.header_h() + super::super::ROW_HEIGHT * 2.0 * 2.0 + 1.0;
    assert_eq!(state.cell_at(x, y), Some((2, 2)));

    // The preset clamp rejects out-of-range factors.
    state.set_zoom(99.0);
    assert_eq!(state.zoom, *TrameUiState::ZOOM_PRESETS.last().unwrap());
}

/// A 2x2 merge master at (1,1) covering (1,1)..=(2,2). Selecting any covered
/// sub-cell snaps the active cell to the master. Arrow navigation then steps
/// across the whole block rather than landing inside it.
#[test]
fn merge_block_selects_as_a_whole_and_nav_steps_over_it() {
    let mut state = TrameUiState::new(800.0, 600.0);
    let cell = |row, col, span| DisplayCell {
        row,
        col,
        text: String::new(),
        raw: String::new(),
        bold: false,
        italic: false,
        is_merged: true,
        merge_span: span,
        is_filtered: false,
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

    // Clicking the bottom-right covered sub-cell selects the master.
    state.select(2, 2);
    assert_eq!(state.selection, Selection::single(1, 1));

    // Arrow Right from the master clears the 2-wide span -> column 3.
    state.move_cursor(0, 1, false).expect("moved past merge");
    assert_eq!((state.active_row(), state.active_col()), (1, 3));

    // Arrow Left back into the block snaps onto the master, not a sub-cell.
    state.move_cursor(0, -1, false).expect("moved back");
    assert_eq!((state.active_row(), state.active_col()), (1, 1));
}

/// Shift+arrows / drag extend the live rectangle (anchor fixed, active
/// moves); a plain move collapses it; Ctrl+click commits a disjoint range.
#[test]
fn selection_extend_disjoint_and_membership() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.select(2, 2);
    assert!(!state.selection.is_multi(), "lone single cell is not multi");
    assert_eq!(state.selection.bounds(), (2, 2, 2, 2));

    // Extend to (4, 5): the live rectangle spans (2,2)..=(4,5).
    state.select_extend(4, 5);
    assert!(state.selection.is_multi());
    assert_eq!(state.selection.bounds(), (2, 2, 4, 5));
    assert!(state.selection.contains(3, 3));
    assert!(!state.selection.contains(5, 5));

    // Extending backwards past the anchor renormalizes the bounds.
    state.select_extend(0, 0);
    assert_eq!(state.selection.bounds(), (0, 0, 2, 2));

    // Ctrl+click starts a disjoint single-cell range; the old rect persists.
    state.select_disjoint(10, 10);
    assert_eq!(state.selection.active(), (10, 10));
    assert!(state.selection.contains(1, 1), "first rect still selected");
    assert!(state.selection.contains(10, 10), "new cell selected");
    assert_eq!(state.selection.rects().count(), 2);

    // A plain move collapses everything back to a single cell.
    state.move_to(7, 7, false).expect("moved");
    assert_eq!(state.selection, Selection::single(7, 7));
    assert!(!state.selection.is_multi());
}

/// `selected_cells` enumerates every covered cell across rectangles and
/// deduplicates overlaps (Ctrl+Enter fill target).
#[test]
fn selected_cells_enumerates_and_dedups() {
    let mut state = TrameUiState::new(800.0, 600.0);
    // A 2x2 block (0,0)..=(1,1).
    state.select(0, 0);
    state.select_extend(1, 1);
    // A disjoint block (1,1)..=(1,2) that overlaps the first at (1,1).
    state.select_disjoint(1, 1);
    state.select_extend(1, 2);

    let cells = state.selection.selected_cells();
    assert_eq!(cells, vec![(0, 0), (0, 1), (1, 0), (1, 1), (1, 2)]);
}

/// `move_cursor`/`move_to` with `extend` grow the range while keeping the
/// anchor put (Shift+arrows, Shift+End).
#[test]
fn move_with_extend_keeps_anchor() {
    let mut state = TrameUiState::new(800.0, 600.0);
    state.select(0, 0);
    state.move_cursor(2, 0, true).expect("moved");
    state.move_cursor(0, 3, true).expect("moved");
    assert_eq!(state.selection.anchor, (0, 0));
    assert_eq!(state.selection.active(), (2, 3));
    assert_eq!(state.selection.bounds(), (0, 0, 2, 3));
}

/// Reported bug: two vertical merges A1:A2 and B1:B2 selected by their masters
/// only (row 0). Before the unmerge the cache holds the merges, so the
/// rendered selection expands to cover all four cells. After the unmerge the
/// Logic thread sends a `SelectRange`, applied via `select_rect`. The freed
/// cells then stay selected once the re-fetch clears the merge spans.
#[test]
fn select_rect_keeps_unmerged_cells_selected() {
    let mut state = TrameUiState::new(800.0, 600.0);
    let cell = |row, col, span: Option<(u32, u32)>| DisplayCell {
        row,
        col,
        text: String::new(),
        raw: String::new(),
        is_merged: span.is_some(),
        merge_span: span,
        ..Default::default()
    };
    // A1:A2 (master (0,0), 1 col x 2 rows) and B1:B2 (master (0,1)).
    state.on_viewport_data(TrameEvent::ViewportData {
        version: 0,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(0, 0, Some((1, 2))),
            cell(1, 0, None),
            cell(0, 1, Some((1, 2))),
            cell(1, 1, None),
        ],
    });
    // Selection spans the two masters only (A1:B1, row 0). With the merges in
    // the cache the rendered rect already covers all four cells.
    state.selection = Selection {
        anchor: (0, 0),
        active: (0, 1),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert_eq!(state.selection_rects(), vec![(0, 0, 1, 1)]);

    // Logic's SelectRange after the unmerge: adopt the freed area.
    state.select_rect(0, 0, 1, 1);
    // The re-fetch clears the merge spans (now plain cells).
    state.on_viewport_data(TrameEvent::ViewportData {
        version: 1,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(0, 0, None),
            cell(1, 0, None),
            cell(0, 1, None),
            cell(1, 1, None),
        ],
    });
    // All four freed cells remain selected (the bug left only row 0).
    assert_eq!(state.selection_rects(), vec![(0, 0, 1, 1)]);
    assert!(!state.snap_active_to_master(), "active master, no collapse");
}

/// Bug 2: selecting a merged block draws the autofill handle at the
/// bottom-right of the *whole block*, not of the master cell. With the
/// selection collapsed on the master (1,1), the handle must sit on the
/// extent corner (2,2).
#[test]
fn fill_handle_sits_on_merge_extent_corner() {
    let mut state = TrameUiState::new(800.0, 600.0);
    seed_merge_2x2(&mut state);
    state.select(2, 2); // snaps to master (1,1)
    assert_eq!(state.selection.bounds(), (1, 1, 1, 1));

    let handle = state.fill_handle_rect().expect("handle visible");
    let extent = state.cell_pixel_rect(2, 2).expect("extent corner visible");
    let s = super::super::FILL_HANDLE_SIZE;
    assert_eq!(
        handle.0,
        extent.0 + extent.2 - s / 2.0,
        "handle x on extent"
    );
    assert_eq!(
        handle.1,
        extent.1 + extent.3 - s / 2.0,
        "handle y on extent"
    );
}

/// Bug 3: a drag-selection that touches part of a merge selects the whole
/// block at once. The drawn/operative rectangles snap out to full merges.
#[test]
fn drag_selection_expands_to_whole_merge() {
    let mut state = TrameUiState::new(800.0, 600.0);
    seed_merge_2x2(&mut state);
    // Drag from (0,0) to (1,1): the raw live rect (0,0)..=(1,1) cuts the
    // merge in half; the expanded rect must cover the whole (1,1)..=(2,2).
    state.select(0, 0);
    state.select_extend(1, 1);
    let rects = state.selection_rects();
    assert_eq!(rects, vec![(0, 0, 2, 2)], "rect grows to contain the merge");
}
