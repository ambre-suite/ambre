// ambre-trame/src/ui/grid/tests/group1.rs
// TrameUiState grid geometry/navigation tests. Helpers via `use super::super::*`.

use super::*;
use ambre_commands::SelectionMode;
use ambre_core::SurfacePx;

/// `current_region` returns `None` on an isolated empty active cell and the
/// bounding rectangle of an L-shaped contiguous block otherwise (`trame-shared-filter`).
#[test]
fn current_region_bounds_the_contiguous_block() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    // Isolated empty active cell -> no block to filter.
    assert_eq!(state.current_region(), None);

    // L-shaped block around the active cell (0,0): (0,0),(0,1),(1,0).
    put(&mut state, 0, 0, "a");
    put(&mut state, 0, 1, "b");
    put(&mut state, 1, 0, "c");
    // The region is the bounding rect of the contiguous block.
    assert_eq!(state.current_region(), Some(rect(0, 0, 1, 1)));

    // A far-away non-contiguous cell is excluded.
    put(&mut state, 10, 10, "z");
    assert_eq!(state.current_region(), Some(rect(0, 0, 1, 1)));
}

/// `filter_button_at` hit-tests the autofilter button drawn at the right edge
/// of each current-region header cell, returning `(band_row, col)` (`trame-shared-filter`).
#[test]
fn filter_button_hit_test_matches_drawn_rect() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    put(&mut state, 0, 0, "a");
    put(&mut state, 0, 1, "b");
    // (r1, c1, r2, c2): a one-row, two-column block.
    assert_eq!(state.current_region(), Some(rect(0, 0, 0, 1)));

    // The button for column 1 sits at the right edge of cell (0, 1). Hitting
    // its centre resolves to that column on the region's header row.
    let button = state
        .filter_button_rect(CellPos::new(Row::new(0), Col::new(1)))
        .expect("button visible");
    let centre = button.center();
    assert_eq!(
        state.filter_button_at(pointer(centre.x, centre.y)),
        Some(at(0, 1))
    );
    // A point well clear of any button misses.
    assert_eq!(
        state.filter_button_at(pointer(button.x0 + 400.0, centre.y)),
        None
    );
}

/// The autofilter button scales with the grid zoom (it used to keep a fixed
/// device size, shrinking visually as the zoomed cells grew around it).
#[test]
fn filter_button_scales_with_zoom() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    put(&mut state, 0, 0, "a");
    let w100 = state
        .filter_button_rect(CellPos::new(Row::new(0), Col::new(0)))
        .expect("button visible")
        .width();

    state.set_zoom(ambre_core::ZoomFactor::positive(2.0));
    let w200 = state
        .filter_button_rect(CellPos::new(Row::new(0), Col::new(0)))
        .expect("button visible")
        .width();
    // At 2x the cell is taller than the un-clamped button, so the side is the
    // zoom-scaled base: strictly larger than at 100%.
    assert!(w200 > w100, "button must grow with zoom: {w100} -> {w200}");
}

/// A pointer hit collapses the selection to a single cell (`anchor ==
/// active`); keyboard navigation moves the single-cell selection and never
/// leaves a stale anchor behind.
#[test]
fn single_cell_selection_tracks_active() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(0), Col::new(0)))
    );

    state.select(CellPos::new(Row::new(5), Col::new(3)));
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(5), Col::new(3)))
    );
    assert_eq!(
        (state.active_row(), state.active_col()),
        (Row::new(5), Col::new(3))
    );

    // Arrow navigation keeps the selection single-cell on the new active cell.
    state
        .move_cursor(RowDelta::new(1), ColDelta::new(0), SelectionMode::Move)
        .expect("cursor moved");
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(6), Col::new(3)))
    );
    assert_eq!(state.selection.anchor, state.selection.active);
}

/// `col_at_x` / `row_at_y` resolve the axis band under a pointer through the
/// variable metrics (drives a whole-axis header drag, `trame-selection`).
#[test]
fn axis_pointer_resolution_follows_variable_metrics() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    state
        .col_widths
        .insert(Col::new(0), ambre_core::DocPx::new(160.0)); // col 0 widened
    // x just past the wide col 0 lands in col 1.
    assert_eq!(
        state.col_at_x(view_px(super::super::HEADER_WIDTH.get() + 160.0 + 5.0)),
        Some(Col::new(1))
    );
    assert_eq!(
        state.col_at_x(view_px(super::super::HEADER_WIDTH.get() + 10.0)),
        Some(Col::new(0))
    );
    assert_eq!(state.col_at_x(view_px(0.0)), None); // over the row-header strip

    state
        .row_heights
        .insert(Row::new(0), ambre_core::DocPx::new(50.0));
    assert_eq!(
        state.row_at_y(view_px(super::super::HEADER_HEIGHT.get() + 50.0 + 5.0)),
        Some(Row::new(1))
    );
    assert_eq!(
        state.row_at_y(view_px(super::super::HEADER_HEIGHT.get() + 10.0)),
        Some(Row::new(0))
    );
    assert_eq!(state.row_at_y(view_px(0.0)), None);
}

/// The insert "+" zone sits in the top/left sliver of a boundary band, the
/// resize grip in the rest (`trame-resize`, `trame-headers`).
#[test]
fn header_boundary_splits_insert_and_resize() {
    let state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    let boundary = super::super::HEADER_WIDTH.get() + f64::from(super::super::COL_WIDTH.get()); // col 0's right edge
    // Lower half -> resize col 0.
    assert_eq!(
        state.header_hit(pointer(boundary, super::super::HEADER_HEIGHT.get() * 0.8)),
        Some(HeaderHit::ColBorder(Col::new(0)))
    );
    // Upper half -> insert left of col 1 (at this boundary).
    assert_eq!(
        state.header_hit(pointer(boundary, 2.0)),
        Some(HeaderHit::ColInsert(Col::new(1)))
    );
}

/// `header_hit` resolves the three zones of each header strip plus the
/// corner, with the resize band winning over the body (`trame-headers`).
#[test]
fn header_hit_resolves_zones() {
    let state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    // Top-left corner.
    assert_eq!(
        state.header_hit(pointer(10.0, 10.0)),
        Some(HeaderHit::Corner)
    );
    // Column 0 body: header strip, x mid-column 0 ([50, 130)).
    assert_eq!(
        state.header_hit(pointer(super::super::HEADER_WIDTH.get() + 30.0, 5.0)),
        Some(HeaderHit::ColBody(Col::new(0)))
    );
    // Column 0 right border (~130), lower half of the band, arms a resize.
    assert_eq!(
        state.header_hit(pointer(
            super::super::HEADER_WIDTH.get() + f64::from(super::super::COL_WIDTH.get()),
            super::super::HEADER_HEIGHT.get() * 0.8
        )),
        Some(HeaderHit::ColBorder(Col::new(0)))
    );
    // Row 1 body (rows start at HEADER_HEIGHT, each ROW_HEIGHT tall).
    assert_eq!(
        state.header_hit(pointer(
            10.0,
            super::super::HEADER_HEIGHT.get() + f64::from(super::super::ROW_HEIGHT.get()) + 5.0
        )),
        Some(HeaderHit::RowBody(Row::new(1)))
    );
    // Row 0 bottom border, right half of the band, resizes row 0.
    assert_eq!(
        state.header_hit(pointer(
            super::super::HEADER_WIDTH.get() * 0.8,
            super::super::HEADER_HEIGHT.get() + f64::from(super::super::ROW_HEIGHT.get())
        )),
        Some(HeaderHit::RowBorder(Row::new(0)))
    );
    // A point in the grid body is not a header hit.
    assert_eq!(
        state.header_hit(pointer(
            super::super::HEADER_WIDTH.get() + 10.0,
            super::super::HEADER_HEIGHT.get() + 10.0
        )),
        None
    );
}

/// `Selection::axis` builds a whole-column / whole-row band whose perpendicular
/// coordinate is irrelevant (`trame-selection`).
#[test]
fn axis_selection_spans_the_band() {
    let cols = Selection::axis(AxisPos::Col(Col::new(2)), AxisPos::Col(Col::new(4)));
    assert_eq!(cols.kind, SelKind::Cols);
    let band = cols.bounds();
    assert_eq!(
        (band.top_left.col, band.bottom_right.col),
        (Col::new(2), Col::new(4))
    );

    let rows = Selection::axis(AxisPos::Row(Row::new(5)), AxisPos::Row(Row::new(3)));
    assert_eq!(rows.kind, SelKind::Rows);
    let band = rows.bounds();
    assert_eq!(
        (band.top_left.row, band.bottom_right.row),
        (Row::new(3), Row::new(5))
    );
}

/// `Selection::report` carries a whole-axis band as its kind, with no extent
/// across the other axis. The Logic thread resolves that extent from the used
/// range (`TRAME-UI-033`).
#[test]
fn axis_report_carries_the_band_not_the_cache() {
    let cols = Selection::axis(AxisPos::Col(Col::new(2)), AxisPos::Col(Col::new(4))).report();
    match cols {
        crate::TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            extra,
            kind,
        } => {
            assert_eq!(
                (row, col),
                (Row::new(0), Col::new(4)),
                "active is the clicked column"
            );
            assert_eq!(
                (anchor_row, anchor_col),
                (Row::new(0), Col::new(2)),
                "anchor is the band start"
            );
            assert!(extra.is_empty(), "a band commits no disjoint rectangles");
            assert_eq!(kind, crate::SelectionKind::Cols);
        }
        other => panic!("expected SetActiveCell, got {other:?}"),
    }

    let rows = Selection::axis(AxisPos::Row(Row::new(5)), AxisPos::Row(Row::new(3))).report();
    match rows {
        crate::TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            kind,
            ..
        } => {
            assert_eq!((row, col), (Row::new(3), Col::new(0)));
            assert_eq!((anchor_row, anchor_col), (Row::new(5), Col::new(0)));
            assert_eq!(kind, crate::SelectionKind::Rows);
        }
        other => panic!("expected SetActiveCell, got {other:?}"),
    }
}

/// `Selection::extend_axis` keeps the anchor and moves the band's active edge,
/// so a single-column band swept to the right becomes a multi-cell selection.
/// That is the state the status band reads after a header drag, and the trigger
/// the aggregates refresh must not miss (`TRAME-UI-109`).
#[test]
fn extending_a_header_band_makes_the_selection_multi() {
    let mut cols = Selection::axis(AxisPos::Col(Col::new(0)), AxisPos::Col(Col::new(0)));
    assert!(
        !cols.is_multi(),
        "a single column is not a multi-cell selection"
    );
    cols.extend_axis(AxisPos::Col(Col::new(3)));
    assert_eq!(cols.live_rect(), rect(0, 0, 0, 3));
    assert!(
        cols.is_multi(),
        "sweeping columns A to D is a multi-cell band"
    );

    let mut rows = Selection::axis(AxisPos::Row(Row::new(5)), AxisPos::Row(Row::new(5)));
    assert!(
        !rows.is_multi(),
        "a single row is not a multi-cell selection"
    );
    rows.extend_axis(AxisPos::Row(Row::new(2)));
    assert_eq!(rows.live_rect(), rect(2, 0, 5, 0));
    assert!(rows.is_multi(), "sweeping rows is a multi-cell band");
}

/// A resized column / row shifts the pixel mapping: `cell_at` and
/// `cell_pixel_rect` must agree on the variable geometry (`trame-resize`).
#[test]
fn variable_metrics_round_trip_pixel_and_hit_test() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    // Col 0 widened to 160, row 0 heightened to 40.
    state
        .col_widths
        .insert(Col::new(0), ambre_core::DocPx::new(160.0));
    state
        .row_heights
        .insert(Row::new(0), ambre_core::DocPx::new(40.0));

    assert_eq!(state.col_width_of(Col::new(0)).get(), 160.0);
    assert_eq!(
        state.col_width_of(Col::new(1)).get(),
        f64::from(super::super::COL_WIDTH.get())
    );
    assert_eq!(state.row_height_of(Row::new(0)).get(), 40.0);

    // Cell (1,1) sits after the wide col 0 and below the tall row 0.
    let cell = state
        .cell_pixel_rect(CellPos::new(Row::new(1), Col::new(1)))
        .expect("visible");
    assert_eq!(cell.x0, super::super::HEADER_WIDTH.get() + 160.0);
    assert_eq!(cell.y0, super::super::HEADER_HEIGHT.get() + 40.0);
    assert_eq!(
        (cell.width(), cell.height()),
        (
            f64::from(super::super::COL_WIDTH.get()),
            f64::from(super::super::ROW_HEIGHT.get())
        )
    );

    // Hit-testing the interior of that cell returns it.
    assert_eq!(state.cell_at(pointer(cell.x0 + 5.0, cell.y0 + 5.0)), Some(at(1, 1)));
    // A point inside the tall row 0 / wide col 0 maps to (0, 0).
    assert_eq!(
        state.cell_at(pointer(
            super::super::HEADER_WIDTH.get() + 100.0,
            super::super::HEADER_HEIGHT.get() + 30.0
        )),
        Some(at(0, 0))
    );
}

/// Grid zoom scales every metric (header strips, cell widths/heights) so the
/// pixel mapping and hit-testing stay self-consistent (`trame-status-bar`).
#[test]
fn zoom_scales_metrics_and_round_trips_hit_test() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    assert!(state.set_zoom(ambre_core::ZoomFactor::positive(2.0)));
    assert_eq!(state.zoom.get(), 2.0);

    // Header strips and the default cell metrics double.
    assert_eq!(
        state.header_w().get(),
        super::super::HEADER_WIDTH.get() * 2.0
    );
    assert_eq!(
        state.header_h().get(),
        super::super::HEADER_HEIGHT.get() * 2.0
    );
    assert_eq!(
        state.col_width_of(Col::new(0)).get(),
        f64::from(super::super::COL_WIDTH.get()) * 2.0
    );
    assert_eq!(
        state.row_height_of(Row::new(0)).get(),
        f64::from(super::super::ROW_HEIGHT.get()) * 2.0
    );

    // Column 1's left edge is the (doubled) header plus one (doubled) column.
    assert_eq!(
        state.col_x(Col::new(1)).get(),
        super::super::HEADER_WIDTH.get() * 2.0 + f64::from(super::super::COL_WIDTH.get()) * 2.0
    );

    // A point inside cell (2, 2) at 200% maps back to it.
    let x = state.header_w().get() + f64::from(super::super::COL_WIDTH.get()) * 2.0 * 2.0 + 1.0;
    let y = state.header_h().get() + f64::from(super::super::ROW_HEIGHT.get()) * 2.0 * 2.0 + 1.0;
    assert_eq!(state.cell_at(pointer(x, y)), Some(at(2, 2)));

    // The preset clamp rejects out-of-range factors.
    state.set_zoom(ambre_core::ZoomFactor::positive(99.0));
    assert_eq!(state.zoom, *TrameUiState::ZOOM_PRESETS.last().unwrap());
}

/// A 2x2 merge master at (1,1) covering (1,1)..=(2,2). Selecting any covered
/// sub-cell snaps the active cell to the master. Arrow navigation then steps
/// across the whole block rather than landing inside it.
#[test]
fn merge_block_selects_as_a_whole_and_nav_steps_over_it() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
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

    // Clicking the bottom-right covered sub-cell selects the master.
    state.select(CellPos::new(Row::new(2), Col::new(2)));
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(1), Col::new(1)))
    );

    // Arrow Right from the master clears the 2-wide span -> column 3.
    state
        .move_cursor(RowDelta::new(0), ColDelta::new(1), SelectionMode::Move)
        .expect("moved past merge");
    assert_eq!(
        (state.active_row(), state.active_col()),
        (Row::new(1), Col::new(3))
    );

    // Arrow Left back into the block snaps onto the master, not a sub-cell.
    state
        .move_cursor(RowDelta::new(0), ColDelta::new(-1), SelectionMode::Move)
        .expect("moved back");
    assert_eq!(
        (state.active_row(), state.active_col()),
        (Row::new(1), Col::new(1))
    );
}

/// Shift+arrows / drag extend the live rectangle (anchor fixed, active
/// moves); a plain move collapses it; Ctrl+click commits a disjoint range.
#[test]
fn selection_extend_disjoint_and_membership() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    state.select(CellPos::new(Row::new(2), Col::new(2)));
    assert!(!state.selection.is_multi(), "lone single cell is not multi");
    assert_eq!(state.selection.bounds(), rect(2, 2, 2, 2));

    // Extend to (4, 5): the live rectangle spans (2,2)..=(4,5).
    state.select_extend(CellPos::new(Row::new(4), Col::new(5)));
    assert!(state.selection.is_multi());
    assert_eq!(state.selection.bounds(), rect(2, 2, 4, 5));
    assert!(
        state
            .selection
            .contains(CellPos::new(Row::new(3), Col::new(3)))
    );
    assert!(
        !state
            .selection
            .contains(CellPos::new(Row::new(5), Col::new(5)))
    );

    // Extending backwards past the anchor renormalizes the bounds.
    state.select_extend(CellPos::new(Row::new(0), Col::new(0)));
    assert_eq!(state.selection.bounds(), rect(0, 0, 2, 2));

    // Ctrl+click starts a disjoint single-cell range; the old rect persists.
    state.select_disjoint(CellPos::new(Row::new(10), Col::new(10)));
    assert_eq!(
        state.selection.active(),
        CellPos::new(Row::new(10), Col::new(10))
    );
    assert!(
        state
            .selection
            .contains(CellPos::new(Row::new(1), Col::new(1))),
        "first rect still selected"
    );
    assert!(
        state
            .selection
            .contains(CellPos::new(Row::new(10), Col::new(10))),
        "new cell selected"
    );
    assert_eq!(state.selection.rects().count(), 2);

    // A plain move collapses everything back to a single cell.
    state
        .move_to(CellPos::new(Row::new(7), Col::new(7)), SelectionMode::Move)
        .expect("moved");
    assert_eq!(
        state.selection,
        Selection::single(CellPos::new(Row::new(7), Col::new(7)))
    );
    assert!(!state.selection.is_multi());
}

/// `selected_cells` enumerates every covered cell across rectangles and
/// deduplicates overlaps (Ctrl+Enter fill target).
#[test]
fn selected_cells_enumerates_and_dedups() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    // A 2x2 block (0,0)..=(1,1).
    state.select(CellPos::new(Row::new(0), Col::new(0)));
    state.select_extend(CellPos::new(Row::new(1), Col::new(1)));
    // A disjoint block (1,1)..=(1,2) that overlaps the first at (1,1).
    state.select_disjoint(CellPos::new(Row::new(1), Col::new(1)));
    state.select_extend(CellPos::new(Row::new(1), Col::new(2)));

    let cells = state.selection.selected_cells();
    assert_eq!(
        cells,
        vec![at(0, 0), at(0, 1), at(1, 0), at(1, 1), at(1, 2)]
    );
}

/// `move_cursor`/`move_to` with `extend` grow the range while keeping the
/// anchor put (Shift+arrows, Shift+End).
#[test]
fn move_with_extend_keeps_anchor() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    state.select(CellPos::new(Row::new(0), Col::new(0)));
    state
        .move_cursor(RowDelta::new(2), ColDelta::new(0), SelectionMode::Extend)
        .expect("moved");
    state
        .move_cursor(RowDelta::new(0), ColDelta::new(3), SelectionMode::Extend)
        .expect("moved");
    assert_eq!(
        state.selection.anchor,
        CellPos::new(Row::new(0), Col::new(0))
    );
    assert_eq!(
        state.selection.active(),
        CellPos::new(Row::new(2), Col::new(3))
    );
    assert_eq!(state.selection.bounds(), rect(0, 0, 2, 3));
}

/// Reported bug: two vertical merges A1:A2 and B1:B2 selected by their masters
/// only (row 0). Before the unmerge the cache holds the merges, so the
/// rendered selection expands to cover all four cells. After the unmerge the
/// Logic thread sends a `SelectRange`, applied via `select_rect`. The freed
/// cells then stay selected once the re-fetch clears the merge spans.
#[test]
fn select_rect_keeps_unmerged_cells_selected() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    let cell = |row, col, span: Option<MergeSpan>| DisplayCell {
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
        version: ambre_bus::ViewportVersion(0),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(Row::new(0), Col::new(0), Some(span(2, 1))),
            cell(Row::new(1), Col::new(0), None),
            cell(Row::new(0), Col::new(1), Some(span(2, 1))),
            cell(Row::new(1), Col::new(1), None),
        ],
    });
    // Selection spans the two masters only (A1:B1, row 0). With the merges in
    // the cache the rendered rect already covers all four cells.
    state.selection = Selection {
        anchor: CellPos::new(Row::new(0), Col::new(0)),
        active: CellPos::new(Row::new(0), Col::new(1)),
        extra: Vec::new(),
        kind: SelKind::Cells,
    };
    assert_eq!(state.selection_rects(), vec![rect(0, 0, 1, 1)]);

    // Logic's SelectRange after the unmerge: adopt the freed area.
    state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(0), Col::new(0)),
        CellPos::new(Row::new(1), Col::new(1)),
    ));
    // The re-fetch clears the merge spans (now plain cells).
    state.on_viewport_data(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(1),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            cell(Row::new(0), Col::new(0), None),
            cell(Row::new(1), Col::new(0), None),
            cell(Row::new(0), Col::new(1), None),
            cell(Row::new(1), Col::new(1), None),
        ],
    });
    // All four freed cells remain selected (the bug left only row 0).
    assert_eq!(state.selection_rects(), vec![rect(0, 0, 1, 1)]);
    assert!(!state.snap_active_to_master(), "active master, no collapse");
}

/// Bug 2: selecting a merged block draws the autofill handle at the
/// bottom-right of the *whole block*, not of the master cell. With the
/// selection collapsed on the master (1,1), the handle must sit on the
/// extent corner (2,2).
#[test]
fn fill_handle_sits_on_merge_extent_corner() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    seed_merge_2x2(&mut state);
    state.select(CellPos::new(Row::new(2), Col::new(2))); // snaps to master (1,1)
    assert_eq!(state.selection.bounds(), rect(1, 1, 1, 1));

    let handle = state.fill_handle_rect().expect("handle visible");
    let extent = state
        .cell_pixel_rect(CellPos::new(Row::new(2), Col::new(2)))
        .expect("extent corner visible");
    let s = super::super::FILL_HANDLE_SIZE;
    assert_eq!(
        handle.x0,
        extent.x1 - s / 2.0,
        "handle x on extent"
    );
    assert_eq!(
        handle.y0,
        extent.y1 - s / 2.0,
        "handle y on extent"
    );
}

/// Bug 3: a drag-selection that touches part of a merge selects the whole
/// block at once. The drawn/operative rectangles snap out to full merges.
#[test]
fn drag_selection_expands_to_whole_merge() {
    let mut state = TrameUiState::new(SurfacePx::new(800.0), SurfacePx::new(600.0));
    seed_merge_2x2(&mut state);
    // Drag from (0,0) to (1,1): the raw live rect (0,0)..=(1,1) cuts the
    // merge in half; the expanded rect must cover the whole (1,1)..=(2,2).
    state.select(CellPos::new(Row::new(0), Col::new(0)));
    state.select_extend(CellPos::new(Row::new(1), Col::new(1)));
    let rects = state.selection_rects();
    assert_eq!(
        rects,
        vec![rect(0, 0, 2, 2)],
        "rect grows to contain the merge"
    );
}
