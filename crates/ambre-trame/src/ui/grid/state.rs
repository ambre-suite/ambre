// ambre-trame/src/ui/grid/state.rs
// TrameUiState: construction, zoom, selection, cursor movement, scroll and viewport-data ingestion.

use super::*;
use ambre_commands::SelectionMode;

impl TrameUiState {
    /// Discrete zoom levels cycled by the status-bar zoom control.
    pub const ZOOM_PRESETS: [ambre_core::ZoomFactor; 6] = [
        ambre_core::ZoomFactor::positive(0.5),
        ambre_core::ZoomFactor::positive(0.75),
        ambre_core::ZoomFactor::ONE,
        ambre_core::ZoomFactor::positive(1.25),
        ambre_core::ZoomFactor::positive(1.5),
        ambre_core::ZoomFactor::positive(2.0),
    ];

    pub fn new(canvas_width: SurfacePx, canvas_height: SurfacePx) -> Self {
        let mut s = Self {
            top_row: Row::ZERO,
            left_col: Col::ZERO,
            selection: Selection::single(CellPos::ORIGIN),
            viewport_rows: RowCount::ZERO,
            viewport_cols: ColCount::ZERO,
            canvas_width,
            canvas_height,
            zoom: ambre_core::ZoomFactor::ONE,
            cell_cache: HashMap::new(),
            dirty_cells: HashSet::new(),
            formula_errors: HashSet::new(),
            accepted_version: ambre_core::Generation::ZERO,
            active_sheet: ambre_bus::SheetId::default(),
            next_version: ambre_core::Generation::ZERO.next(),
            filtered_col: None,
            hidden_rows: Vec::new(),
            col_widths: HashMap::new(),
            row_heights: HashMap::new(),
            resize_preview: None,
            insert_hint: None,
            hover_header: None,
            reorder_preview: None,
            clipboard_anchor: None,
            fill_preview: None,
            find_matches: Vec::new(),
        };
        s.recompute_viewport_dims();
        s
    }

    /// Update canvas dimensions and recompute the visible range.
    #[allow(dead_code)]
    pub fn resize(&mut self, width: SurfacePx, height: SurfacePx) {
        self.canvas_width = width;
        self.canvas_height = height;
        self.recompute_viewport_dims();
    }

    /// Bottom edge (logical px) of the scrollable grid area. The sheet-tab strip
    /// moved to DOM chrome (`SheetTabs`), so the grid now fills the full canvas
    /// height. The DOM strip occupies its own shell row outside this surface.
    pub fn grid_bottom(&self) -> SurfacePx {
        self.canvas_height.max(self.header_h())
    }

    fn recompute_viewport_dims(&mut self) {
        let grid_h = (self.grid_bottom() - self.header_h()).max(SurfacePx::ZERO);
        let grid_w = (self.canvas_width - self.header_w()).max(SurfacePx::ZERO);
        let (row_h, col_w) = (self.scaled(ROW_HEIGHT), self.scaled(COL_WIDTH));
        // ceil() already accounts for the partially-visible row/col at the edge.
        // No +1 needed: viewport_rows is the count of rows that start within the
        // grid area, whether fully or partially visible. scroll_to_include and
        // cell_pixel_rect must agree on this count, or the active cell can
        // appear "in viewport" to the scroll logic but off-canvas to the renderer.
        self.viewport_rows = RowCount::new((grid_h / row_h).ceil() as u32);
        self.viewport_cols = ColCount::new((grid_w / col_w).ceil() as u32);
    }

    /// Set the grid zoom factor (clamped to the preset range), keeping the
    /// top-left scroll origin pinned. Recomputes the visible range. Returns
    /// `true` when the zoom actually changed, so the caller can repaint and
    /// re-request the (now differently sized) viewport.
    pub fn set_zoom(&mut self, zoom: ambre_core::ZoomFactor) -> bool {
        let [lo, .., hi] = Self::ZOOM_PRESETS;
        let z = zoom.clamped(lo, hi);
        if (z.get() - self.zoom.get()).abs() < f64::EPSILON {
            return false;
        }
        self.zoom = z;
        self.recompute_viewport_dims();
        true
    }

    /// The active cell row (the selection's active corner).
    pub fn active_row(&self) -> Row {
        self.selection.active.row
    }

    /// The active cell column (the selection's active corner).
    pub fn active_col(&self) -> Col {
        self.selection.active.col
    }

    /// The active cell (the selection's active corner).
    pub fn active_cell(&self) -> CellPos {
        self.selection.active
    }

    /// Collapse the selection to a single cell (`trame-selection`). Used on a
    /// plain pointer hit; no clamping -- callers pass valid grid coords.
    ///
    /// A hit inside a merged block snaps to the block's master, so the merge is
    /// always selected as a whole. Its sub-cells are never individually selectable
    /// (`trame-cell-merges`).
    pub fn select(&mut self, at: CellPos) {
        self.selection = Selection::single(self.snap_to_master(at));
    }

    /// Replace the live selection with `rect`, collapsing the active cell onto
    /// its top-left corner. Used when the Logic thread dictates a selection the
    /// canvas cannot derive locally, such as a
    /// [`SelectRange`](ambre_bus::TrameEvent::SelectRange) after an unmerge.
    /// The freed cells must stay selected though the span-defining merge is
    /// gone.
    pub fn select_rect(&mut self, rect: CellRect) {
        self.selection = Selection {
            anchor: rect.bottom_right,
            active: rect.top_left,
            extra: Vec::new(),
            kind: SelKind::Cells,
        };
    }

    /// Resolve `at` to its merge master when inside a merged block, else
    /// return it unchanged. The single chokepoint that keeps the active cell on a
    /// master rather than a covered sub-cell.
    fn snap_to_master(&self, at: CellPos) -> CellPos {
        self.merge_master_at(at).map_or(at, |block| block.top_left)
    }

    /// If the active cell now sits on a *covered* sub-cell of a merge, collapse
    /// the selection onto the block master and return `true`. Covered sub-cells
    /// are never individually selectable (`trame-cell-merges`), so this is a safe
    /// correction. Called after a viewport batch so a freshly-created merge
    /// re-homes the cursor on its master (top-left). The editor and range ops
    /// then target the master, not a covered cell that would reject the write as
    /// a merge conflict. A no-op when the active cell is a master or unmerged.
    pub fn snap_active_to_master(&mut self) -> bool {
        let at = self.selection.active;
        match self.merge_master_at(at) {
            Some(block) if block.top_left != at => {
                // Collapse onto the master only when this one block covers the whole
                // selection (for example drag-selected cells that just became a
                // single merge). A selection spanning more than this block must keep
                // its full extent. An autofill that produced a grid of merges, whose
                // active corner landed on the last block's covered sub-cell, is such
                // a case. We only re-home the active corner onto that block's master
                // so it stops sitting on a covered sub-cell. The bounds then
                // re-expand through `expand_to_merges`, so the rendered selection
                // still covers every filled block.
                let bounds = self.selection.bounds();
                let covers_selection = self.selection.extra.is_empty()
                    && block.contains(bounds.top_left)
                    && block.contains(bounds.bottom_right);
                if covers_selection {
                    self.selection = Selection::single(block.top_left);
                } else {
                    self.selection.active = block.top_left;
                }
                true
            }
            _ => false,
        }
    }

    /// Extend the live rectangle's active corner to `at`, keeping the anchor
    /// (Shift+click / drag). The selection's other corner is unchanged.
    pub fn select_extend(&mut self, at: CellPos) {
        self.selection.extend_to(at);
    }

    /// Commit the live rectangle and start a fresh single-cell range at `at`
    /// (Ctrl+click: disjoint selection).
    pub fn select_disjoint(&mut self, at: CellPos) {
        self.selection.add_disjoint(at);
    }

    /// Move the active cell by `dr` rows and `dc` columns.
    ///
    /// With `extend` (Shift held) the selection's active corner moves and the
    /// anchor stays, growing a range; without it the selection collapses onto
    /// the new cell. Scrolls the viewport if the new position falls outside the
    /// visible area. Returns `None` if the cursor did not move (clamped at an
    /// edge). Otherwise it returns a [`CursorMove`] carrying the new active
    /// cell, plus an optional `ScrollViewport` to fetch newly exposed cells.
    /// The active cell always travels, so the Logic thread can retarget
    /// formatting.
    pub fn move_cursor(
        &mut self,
        dr: RowDelta,
        dc: ColDelta,
        extend: SelectionMode,
    ) -> Option<CursorMove> {
        use std::cmp::Ordering;
        let at = self.active_cell();
        // Stepping off a merge master must clear its whole span, else the
        // candidate lands on a covered sub-cell and snaps straight back (cursor
        // trapped). Advance by the span in the travel direction.
        let (mut row, mut col) = (at.row + dr, at.col + dc);
        if let Some(block) = self.merge_master_at(at) {
            match dc.cmp(&ColDelta::ZERO) {
                Ordering::Greater => col = block.bottom_right.col.next(),
                Ordering::Less => col = block.top_left.col - ColCount::ONE,
                Ordering::Equal => {}
            }
            match dr.cmp(&RowDelta::ZERO) {
                Ordering::Greater => row = block.bottom_right.row.next(),
                Ordering::Less => row = block.top_left.row - RowCount::ONE,
                Ordering::Equal => {}
            }
        }
        let (row, col) = (row.min(LAST_ROW), col.min(LAST_COL));
        // A vertical step never lands on a collapsed (filtered) row: skip the run
        // of hidden rows in the travel direction.
        let row = match dr.cmp(&RowDelta::ZERO) {
            Ordering::Greater => self.next_visible_row(row),
            Ordering::Less => self.prev_visible_row(row),
            Ordering::Equal => row,
        };
        self.move_to(CellPos::new(row, col), extend)
    }

    /// Move the active cell to absolute `at`, clamped to the grid.
    ///
    /// Backs the jump-style navigation keys (Home / End / Ctrl+Home / Ctrl+End,
    /// Page Up / Page Down compute their target row first). With `extend` the
    /// range grows to the target (Shift+End, Shift+Ctrl+End...) instead of
    /// collapsing. Same contract as [`move_cursor`](Self::move_cursor): `None`
    /// if the cell did not change, otherwise a [`CursorMove`] with the new
    /// active cell plus an optional `ScrollViewport`.
    pub fn move_to(&mut self, at: CellPos, extend: SelectionMode) -> Option<CursorMove> {
        // Landing on a covered sub-cell snaps to its master, but only when
        // collapsing. An extension keeps its raw corner so the live rectangle
        // can grow cell by cell.
        let clamped = CellPos::new(at.row.min(LAST_ROW), at.col.min(LAST_COL));
        let target = match extend {
            SelectionMode::Extend => clamped,
            SelectionMode::Move => self.snap_to_master(clamped),
        };
        // A jump target (Home/End/PageDown/click-through) may fall on a collapsed
        // row: snap to the nearest visible one so the caret stays on shown data.
        let row = {
            let fwd = self.next_visible_row(target.row);
            if self.is_row_hidden(fwd) {
                self.prev_visible_row(target.row)
            } else {
                fwd
            }
        };
        let target = CellPos::new(row, target.col);

        if target == self.active_cell() {
            return None;
        }
        match extend {
            // Grow the live rectangle: move the active corner, keep the anchor
            // (and any committed disjoint rectangles).
            SelectionMode::Extend => self.selection.extend_to(target),
            // Plain navigation collapses the selection onto the new cell.
            SelectionMode::Move => self.selection = Selection::single(target),
        }

        // Scroll viewport so the active cell stays visible. When the active cell
        // is still within the cached range, the highlight moves without a Logic
        // round-trip. We still report the new active cell (formatting target).
        let scroll = self
            .scroll_to_include(target)
            .then(|| self.scroll_viewport_cmd());
        Some(CursorMove {
            active: self.selection.report(),
            scroll,
        })
    }

    /// Rightmost cached non-empty column in `row`, or `None` if the row has no
    /// cached content. Backs the End key: the UI thread has no used-range model.
    /// The local cell cache is the only view of content available
    /// (`trame-grid-viewport`), making this a best-effort jump to the end of the
    /// visible data.
    pub fn last_content_col(&self, row: Row) -> Option<Col> {
        self.cell_cache
            .iter()
            .filter(|(at, d)| at.row == row && !d.text.is_empty())
            .map(|(at, _)| at.col)
            .max()
    }

    /// Bottom-right cached non-empty cell, or `None` if the cache
    /// holds no content. Backs Ctrl+End (same cache-only caveat as
    /// [`last_content_col`](Self::last_content_col)).
    pub fn last_content_cell(&self) -> Option<CellPos> {
        self.cell_cache
            .iter()
            .filter(|(_, d)| !d.text.is_empty())
            .map(|(&pos, _)| pos)
            .max()
    }

    /// Adjust top_row / left_col so that `at` is visible.
    /// Returns true if the scroll position changed.
    fn scroll_to_include(&mut self, at: CellPos) -> bool {
        let mut changed = false;
        if at.row < self.top_row {
            self.top_row = at.row;
            changed = true;
        } else if self.visible_offset(at.row) >= RowDelta::from_count(self.viewport_rows) {
            // `row` sits past the last visible slot: pull the top down so `row`
            // lands on the bottom slot, counting in collapsed (visible) rows.
            let back = self.viewport_rows.saturating_sub(RowCount::ONE);
            self.top_row = self.nth_visible_row_back(at.row, back);
            changed = true;
        }
        if at.col < self.left_col {
            self.left_col = at.col;
            changed = true;
        } else if at.col >= self.left_col + self.viewport_cols {
            self.left_col = (at.col - self.viewport_cols).next();
            changed = true;
        }
        changed
    }

    /// Pan the viewport by `dr` rows and `dc` columns *without* moving the
    /// active cell (wheel / touchpad scroll). Clamps the top-left within the
    /// grid. Returns `true` if the scroll position actually changed.
    ///
    /// The whole cells come from the accumulator of model steps alone, so a
    /// distance the grid computed itself does not move it (`KINETIC-001`).
    pub(crate) fn scroll_by(&mut self, dr: RowDelta, dc: ColDelta) -> bool {
        let new_top = (self.top_row + dr).min(LAST_ROW);
        let new_left = (self.left_col + dc).min(LAST_COL);
        // Keep the top edge on a visible row so the collapsed-row slot math stays
        // consistent (`top_row` is slot 0).
        let new_top = self.next_visible_row(new_top);
        if new_top == self.top_row && new_left == self.left_col {
            return false;
        }
        self.top_row = new_top;
        self.left_col = new_left;
        true
    }

    /// How many columns from [`left_col`](Self::left_col) start inside the
    /// canvas, walking their real widths. A screen of narrow columns holds more
    /// than the uniform `viewport_cols` counts (`TRAME-UI-144`).
    fn visible_col_span(&self) -> ColCount {
        let right = self.canvas_width;
        let mut x = self.header_w();
        let mut col = self.left_col;
        while x < right && col < TOTAL_COLS.as_end() {
            x += self.col_width_of(col);
            col = col.next();
        }
        col.count_from(self.left_col)
    }

    /// How many rows from [`top_row`](Self::top_row) start inside the grid area,
    /// walking their real heights (`TRAME-UI-144`). A row a filter hides counts,
    /// and takes no height.
    fn visible_row_span(&self) -> RowCount {
        let bottom = self.grid_bottom();
        let mut y = self.header_h();
        let mut row = self.top_row;
        while y < bottom && row < TOTAL_ROWS.as_end() {
            if !self.is_row_hidden(row) {
                y += self.row_height_of(row);
            }
            row = row.next();
        }
        row.count_from(self.top_row)
    }

    /// Build a `ScrollViewport` command for the current visible area plus padding.
    ///
    /// Evicts cached cells that fall outside the new requested range. Cells
    /// scrolled into view from an un-fetched area then appear grey until the Logic
    /// thread responds. Cells already within the new range stay white, avoiding
    /// flicker on slow continuous scrolling.
    pub fn scroll_viewport_cmd(&mut self) -> TrameCommand {
        // Prefetch one full screen of rows above and below the viewport (with a
        // PADDING_ROWS floor for tiny windows). A Page Up/Down jump moves the
        // viewport by exactly viewport_rows. A smaller fixed margin would land
        // beyond the cache and flash the entering rows grey until the Logic thread
        // responds. Sizing the margin to the viewport keeps a page jump's
        // destination within already-fetched data, eliminating that flash.
        let row_pad = self.viewport_rows.max(PADDING_ROWS);
        let row_start = self.top_row.saturating_sub(row_pad);
        // Extend the fetched window by the hidden rows between the top edge and
        // the bottom padding. After the row axis collapses there are then still
        // enough *visible* rows to fill the screen. A hidden row outside that
        // span takes no slot, so it stretches nothing (`TRAME-UI-004`).
        let reach = self.top_row + (self.viewport_rows + row_pad);
        let hidden_in_window = RowCount::from_len(
            self.hidden_rows
                .iter()
                .filter(|&&row| row >= self.top_row && row < reach)
                .count(),
        );
        // Every cell the user can see is requested, whatever the sizes on
        // screen (`TRAME-UI-144`). The span is walked from the real metrics,
        // never less than the uniform estimate.
        let rows_shown = self
            .visible_row_span()
            .max(self.viewport_rows + hidden_in_window);
        let row_end = (self.top_row + (rows_shown + row_pad)).min(LAST_ROW);
        let col_start = self.left_col.saturating_sub(PADDING_COLS);
        let cols_shown = self.visible_col_span().max(self.viewport_cols);
        let col_end = (self.left_col + (cols_shown + PADDING_COLS)).min(LAST_COL);

        // Evict cells that are no longer in the requested window.
        let window = CellRect::spanning(
            CellPos::new(row_start, col_start),
            CellPos::new(row_end, col_end),
        );
        self.cell_cache.retain(|&at, _| window.contains(at));
        // The evicted cells may have carried filter flags; resync the hidden set.
        self.rebuild_hidden_rows();

        let version = self.next_version;
        self.next_version = self.next_version.next();

        TrameCommand::ScrollViewport {
            version,
            row_start,
            row_end,
            col_start,
            col_end,
        }
    }

    /// Merge a `ViewportData` batch into the local cache.
    ///
    /// Older responses (stale version) are silently dropped.
    pub fn on_viewport_data(&mut self, update: TrameEvent) {
        let TrameEvent::ViewportData {
            version,
            row_start,
            col_start,
            cells,
            col_widths,
            row_heights,
        } = update
        else {
            return;
        };
        if version < self.accepted_version {
            return;
        }
        self.accepted_version = version;
        // Adopt the per-axis metrics for the covered window (absolute keys). Only
        // a non-default entry is stored. A column/row reset to the default drops
        // out of the map, so it falls back to the uniform metric
        // (`trame-resize`).
        for (col, w) in ColSpan::from_count(col_start, ColCount::from_len(col_widths.len()))
            .into_iter()
            .zip(&col_widths)
        {
            if *w != COL_WIDTH {
                self.col_widths.insert(col, *w);
            } else {
                self.col_widths.remove(&col);
            }
        }
        for (row, h) in RowSpan::from_count(row_start, RowCount::from_len(row_heights.len()))
            .into_iter()
            .zip(&row_heights)
        {
            if *h != ROW_HEIGHT {
                self.row_heights.insert(row, *h);
            } else {
                self.row_heights.remove(&row);
            }
        }
        for cell in cells {
            // Authoritative value supersedes the optimistic one: the cell is no
            // longer dirty, and any prior error is resolved unless re-reported.
            let at = CellPos::new(cell.row, cell.col);
            self.dirty_cells.remove(&at);
            self.formula_errors.remove(&at);
            self.cell_cache.insert(
                at,
                CellData {
                    text: cell.text,
                    raw: DisplayFormula::new(cell.raw),
                    bold: cell.bold,
                    italic: cell.italic,
                    underline: cell.underline,
                    strikethrough: cell.strikethrough,
                    h_align: cell.h_align,
                    numeric: cell.numeric,
                    v_align: cell.v_align,
                    wrap: cell.wrap,
                    fg: cell.fg,
                    bg: cell.bg,
                    font: cell.font,
                    size: cell.size,
                    borders: cell.borders,
                    merge_span: cell.merge_span,
                    is_filtered: cell.is_filtered,
                },
            );
        }
        self.rebuild_hidden_rows();
    }

    /// Recompute [`Self::hidden_rows`] (sorted, deduped) from the `is_filtered`
    /// flag of the cached cells. Called whenever the cache changes (a viewport
    /// batch arrived, or rows were evicted on scroll). The flag is row-level, so
    /// any cached cell of a row is authoritative for it.
    fn rebuild_hidden_rows(&mut self) {
        let mut rows: Vec<Row> = self
            .cell_cache
            .iter()
            .filter(|(_, c)| c.is_filtered == ambre_bus::RowFilter::Filtered)
            .map(|(at, _)| at.row)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        self.hidden_rows = rows;
    }

    /// True if `row` is collapsed by the active shared filter.
    pub fn is_row_hidden(&self, row: Row) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Signed count of *visible* row slots between [`top_row`](Self::top_row) and
    /// `row`. That is the vertical offset (in `ROW_HEIGHT` units) where `row` is
    /// drawn once filtered rows are collapsed out. Mirrors
    /// `GridLayout::visible_offset`.
    pub fn visible_offset(&self, row: Row) -> RowDelta {
        let (lo, hi) = (row.min(self.top_row), row.max(self.top_row));
        let hidden = self
            .hidden_rows
            .iter()
            .filter(|&&r| r >= lo && r < hi)
            .count();
        let slots =
            RowDelta::from_count(hi.count_from(lo).saturating_sub(RowCount::from_len(hidden)));
        if row >= self.top_row {
            slots
        } else {
            slots.reversed()
        }
    }

    /// The absolute grid row drawn at visible slot `slot` from the top of the
    /// viewport (skipping hidden rows). Saturates at the last grid row.
    pub fn row_at_slot(&self, slot: RowCount) -> Row {
        let mut row = self.top_row;
        let mut seen = RowCount::ZERO;
        while row < TOTAL_ROWS.as_end() {
            if !self.is_row_hidden(row) {
                if seen == slot {
                    return row;
                }
                seen += RowCount::ONE;
            }
            row = row.next();
        }
        LAST_ROW
    }

    /// The next visible row at or after `row` (skips a run of hidden rows). Used
    /// by vertical navigation so the caret never lands on a collapsed row.
    pub fn next_visible_row(&self, row: Row) -> Row {
        let mut r = row;
        while r < LAST_ROW && self.is_row_hidden(r) {
            r = r.next();
        }
        r
    }

    /// The previous visible row at or before `row` (skips a run of hidden rows).
    pub fn prev_visible_row(&self, row: Row) -> Row {
        let mut r = row;
        while self.is_row_hidden(r)
            && let Some(prev) = r.prev()
        {
            r = prev;
        }
        r
    }

    /// Walk back from `from` until `n` visible rows have been passed, returning
    /// the resulting row (saturating at 0). Used to place a row on the bottom
    /// viewport slot when scrolling it into view under an active filter.
    fn nth_visible_row_back(&self, from: Row, n: RowCount) -> Row {
        let mut r = from;
        let mut counted = RowCount::ZERO;
        while counted < n
            && let Some(prev) = r.prev()
        {
            r = prev;
            if !self.is_row_hidden(r) {
                counted += RowCount::ONE;
            }
        }
        r
    }
}
