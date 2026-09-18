// ambre-trame/src/ui/grid/state.rs
// TrameUiState: construction, zoom, selection, cursor movement, scroll and viewport-data ingestion.

use super::*;

impl TrameUiState {
    /// Discrete zoom levels cycled by the status-bar zoom control.
    pub const ZOOM_PRESETS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

    pub fn new(canvas_width: f64, canvas_height: f64) -> Self {
        let mut s = Self {
            top_row: 0,
            left_col: 0,
            selection: Selection::single(0, 0),
            viewport_rows: 0,
            viewport_cols: 0,
            canvas_width,
            canvas_height,
            zoom: 1.0,
            cell_cache: HashMap::new(),
            dirty_cells: HashSet::new(),
            formula_errors: HashMap::new(),
            accepted_version: 0,
            active_sheet: String::new(),
            next_version: 1,
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
    pub fn resize(&mut self, width: f64, height: f64) {
        self.canvas_width = width;
        self.canvas_height = height;
        self.recompute_viewport_dims();
    }

    /// Bottom edge (physical px) of the scrollable grid area. The sheet-tab strip
    /// moved to DOM chrome (`SheetTabs`), so the grid now fills the full canvas
    /// height. The DOM strip occupies its own shell row outside this surface.
    pub fn grid_bottom(&self) -> f64 {
        self.canvas_height.max(self.header_h())
    }

    fn recompute_viewport_dims(&mut self) {
        let grid_h = (self.grid_bottom() - self.header_h()).max(0.0);
        let grid_w = (self.canvas_width - self.header_w()).max(0.0);
        // ceil() already accounts for the partially-visible row/col at the edge.
        // No +1 needed: viewport_rows is the count of rows that start within the
        // grid area, whether fully or partially visible. scroll_to_include and
        // cell_pixel_rect must agree on this count, or the active cell can
        // appear "in viewport" to the scroll logic but off-canvas to the renderer.
        self.viewport_rows = (grid_h / (ROW_HEIGHT * self.zoom)).ceil() as u32;
        self.viewport_cols = (grid_w / (COL_WIDTH * self.zoom)).ceil() as u32;
    }

    /// Set the grid zoom factor (clamped to the preset range), keeping the
    /// top-left scroll origin pinned. Recomputes the visible range. Returns
    /// `true` when the zoom actually changed, so the caller can repaint and
    /// re-request the (now differently sized) viewport.
    pub fn set_zoom(&mut self, zoom: f64) -> bool {
        let z = zoom.clamp(
            Self::ZOOM_PRESETS[0],
            Self::ZOOM_PRESETS[Self::ZOOM_PRESETS.len() - 1],
        );
        if (z - self.zoom).abs() < f64::EPSILON {
            return false;
        }
        self.zoom = z;
        self.recompute_viewport_dims();
        true
    }

    /// The active cell row (the selection's active corner).
    pub fn active_row(&self) -> u32 {
        self.selection.active.0
    }

    /// The active cell column (the selection's active corner).
    pub fn active_col(&self) -> u32 {
        self.selection.active.1
    }

    /// Collapse the selection to a single cell (`trame-selection`). Used on a
    /// plain pointer hit; no clamping -- callers pass valid grid coords.
    ///
    /// A hit inside a merged block snaps to the block's master, so the merge is
    /// always selected as a whole. Its sub-cells are never individually selectable
    /// (`trame-cell-merges`).
    pub fn select(&mut self, row: u32, col: u32) {
        let (row, col) = self.snap_to_master(row, col);
        self.selection = Selection::single(row, col);
    }

    /// Replace the live selection with the rectangle `(r1,c1)`-`(r2,c2)` (corners
    /// already ordered), collapsing the active cell onto the top-left corner. Used
    /// when the Logic thread dictates a selection the canvas cannot derive
    /// locally, such as a [`SelectRange`](ambre_bus::TrameEvent::SelectRange)
    /// after an unmerge. The freed cells must stay selected though the
    /// span-defining merge is gone.
    pub fn select_rect(&mut self, r1: u32, c1: u32, r2: u32, c2: u32) {
        self.selection = Selection {
            anchor: (r2, c2),
            active: (r1, c1),
            extra: Vec::new(),
            kind: SelKind::Cells,
        };
    }

    /// Resolve `(row, col)` to its merge master when inside a merged block, else
    /// return it unchanged. The single chokepoint that keeps the active cell on a
    /// master rather than a covered sub-cell.
    fn snap_to_master(&self, row: u32, col: u32) -> (u32, u32) {
        self.merge_master_at(row, col)
            .map_or((row, col), |(mr, mc, _, _)| (mr, mc))
    }

    /// If the active cell now sits on a *covered* sub-cell of a merge, collapse
    /// the selection onto the block master and return `true`. Covered sub-cells
    /// are never individually selectable (`trame-cell-merges`), so this is a safe
    /// correction. Called after a viewport batch so a freshly-created merge
    /// re-homes the cursor on its master (top-left). The editor and range ops
    /// then target the master, not a covered cell that would reject the write as
    /// a merge conflict. A no-op when the active cell is a master or unmerged.
    pub fn snap_active_to_master(&mut self) -> bool {
        let (ar, ac) = self.selection.active;
        match self.merge_master_at(ar, ac) {
            Some((mr, mc, cs, rs)) if (mr, mc) != (ar, ac) => {
                // Collapse onto the master only when this one block covers the whole
                // selection (for example drag-selected cells that just became a
                // single merge). A selection spanning more than this block must keep
                // its full extent. An autofill that produced a grid of merges, whose
                // active corner landed on the last block's covered sub-cell, is such
                // a case. We only re-home the active corner onto that block's master
                // so it stops sitting on a covered sub-cell. The bounds then
                // re-expand through `expand_to_merges`, so the rendered selection
                // still covers every filled block.
                let (sr1, sc1, sr2, sc2) = self.selection.bounds();
                let covers_selection = self.selection.extra.is_empty()
                    && sr1 >= mr
                    && sc1 >= mc
                    && sr2 < mr + rs
                    && sc2 < mc + cs;
                if covers_selection {
                    self.selection = Selection::single(mr, mc);
                } else {
                    self.selection.active = (mr, mc);
                }
                true
            }
            _ => false,
        }
    }

    /// Extend the live rectangle's active corner to `(row, col)`, keeping the
    /// anchor (Shift+click / drag). The selection's other corner is unchanged.
    pub fn select_extend(&mut self, row: u32, col: u32) {
        self.selection.extend_to(row, col);
    }

    /// Commit the live rectangle and start a fresh single-cell range at
    /// `(row, col)` (Ctrl+click: disjoint selection).
    pub fn select_disjoint(&mut self, row: u32, col: u32) {
        self.selection.add_disjoint(row, col);
    }

    /// Move the active cell by `(dr, dc)`.
    ///
    /// With `extend` (Shift held) the selection's active corner moves and the
    /// anchor stays, growing a range; without it the selection collapses onto
    /// the new cell. Scrolls the viewport if the new position falls outside the
    /// visible area. Returns `None` if the cursor did not move (clamped at an
    /// edge). Otherwise it returns a [`CursorMove`] carrying the new active
    /// cell, plus an optional `ScrollViewport` to fetch newly exposed cells.
    /// The active cell always travels, so the Logic thread can retarget
    /// formatting.
    pub fn move_cursor(&mut self, dr: i32, dc: i32, extend: bool) -> Option<CursorMove> {
        let (ar, ac) = (self.active_row(), self.active_col());
        // Stepping off a merge master must clear its whole span, else the
        // candidate lands on a covered sub-cell and snaps straight back (cursor
        // trapped). Advance by the span in the travel direction.
        let (mut tr, mut tc) = (ar as i64 + dr as i64, ac as i64 + dc as i64);
        if let Some((mr, mc, cs, rs)) = self.merge_master_at(ar, ac) {
            if dc > 0 {
                tc = mc as i64 + cs as i64;
            } else if dc < 0 {
                tc = mc as i64 - 1;
            }
            if dr > 0 {
                tr = mr as i64 + rs as i64;
            } else if dr < 0 {
                tr = mr as i64 - 1;
            }
        }
        let mut new_row = tr.clamp(0, (TOTAL_ROWS - 1) as i64) as u32;
        let new_col = tc.clamp(0, (TOTAL_COLS - 1) as i64) as u32;
        // A vertical step never lands on a collapsed (filtered) row: skip the run
        // of hidden rows in the travel direction.
        new_row = match dr.cmp(&0) {
            std::cmp::Ordering::Greater => self.next_visible_row(new_row),
            std::cmp::Ordering::Less => self.prev_visible_row(new_row),
            std::cmp::Ordering::Equal => new_row,
        };
        self.move_to(new_row, new_col, extend)
    }

    /// Move the active cell to absolute `(row, col)`, clamped to the grid.
    ///
    /// Backs the jump-style navigation keys (Home / End / Ctrl+Home / Ctrl+End,
    /// Page Up / Page Down compute their target row first). With `extend` the
    /// range grows to the target (Shift+End, Shift+Ctrl+End...) instead of
    /// collapsing. Same contract as [`move_cursor`](Self::move_cursor): `None`
    /// if the cell did not change, otherwise a [`CursorMove`] with the new
    /// active cell plus an optional `ScrollViewport`.
    pub fn move_to(&mut self, row: u32, col: u32, extend: bool) -> Option<CursorMove> {
        // Landing on a covered sub-cell snaps to its master, but only when
        // collapsing. An extension keeps its raw corner so the live rectangle
        // can grow cell by cell.
        let (new_row, new_col) = if extend {
            (row.min(TOTAL_ROWS - 1), col.min(TOTAL_COLS - 1))
        } else {
            self.snap_to_master(row.min(TOTAL_ROWS - 1), col.min(TOTAL_COLS - 1))
        };
        // A jump target (Home/End/PageDown/click-through) may fall on a collapsed
        // row: snap to the nearest visible one so the caret stays on shown data.
        let new_row = {
            let fwd = self.next_visible_row(new_row);
            if self.is_row_hidden(fwd) {
                self.prev_visible_row(new_row)
            } else {
                fwd
            }
        };

        if new_row == self.active_row() && new_col == self.active_col() {
            return None;
        }
        if extend {
            // Grow the live rectangle: move the active corner, keep the anchor
            // (and any committed disjoint rectangles).
            self.selection.extend_to(new_row, new_col);
        } else {
            // Plain navigation collapses the selection onto the new cell.
            self.selection = Selection::single(new_row, new_col);
        }

        // Scroll viewport so the active cell stays visible. When the active cell
        // is still within the cached range, the highlight moves without a Logic
        // round-trip. We still report the new active cell (formatting target).
        let scroll = self
            .scroll_to_include(new_row, new_col)
            .then(|| self.scroll_viewport_cmd());
        let (anchor_row, anchor_col) = self.selection.anchor;
        Some(CursorMove {
            active: TrameCommand::SetActiveCell {
                row: new_row,
                col: new_col,
                anchor_row,
                anchor_col,
            },
            scroll,
        })
    }

    /// Rightmost cached non-empty column in `row`, or `None` if the row has no
    /// cached content. Backs the End key: the UI thread has no used-range model.
    /// The local cell cache is the only view of content available
    /// (`trame-grid-viewport`), making this a best-effort jump to the end of the
    /// visible data.
    pub fn last_content_col(&self, row: u32) -> Option<u32> {
        self.cell_cache
            .iter()
            .filter(|((r, _), d)| *r == row && !d.text.is_empty())
            .map(|((_, c), _)| *c)
            .max()
    }

    /// Bottom-right cached non-empty cell `(row, col)`, or `None` if the cache
    /// holds no content. Backs Ctrl+End (same cache-only caveat as
    /// [`last_content_col`](Self::last_content_col)).
    pub fn last_content_cell(&self) -> Option<(u32, u32)> {
        self.cell_cache
            .iter()
            .filter(|(_, d)| !d.text.is_empty())
            .map(|(&pos, _)| pos)
            .max()
    }

    /// Adjust top_row / left_col so that (row, col) is visible.
    /// Returns true if the scroll position changed.
    fn scroll_to_include(&mut self, row: u32, col: u32) -> bool {
        let mut changed = false;
        if row < self.top_row {
            self.top_row = row;
            changed = true;
        } else if self.visible_offset(row) >= self.viewport_rows as i64 {
            // `row` sits past the last visible slot: pull the top down so `row`
            // lands on the bottom slot, counting in collapsed (visible) rows.
            self.top_row = self.nth_visible_row_back(row, self.viewport_rows.saturating_sub(1));
            changed = true;
        }
        if col < self.left_col {
            self.left_col = col;
            changed = true;
        } else if col >= self.left_col + self.viewport_cols {
            self.left_col = col - self.viewport_cols + 1;
            changed = true;
        }
        changed
    }

    /// Pan the viewport by `(dr, dc)` rows/cols *without* moving the active
    /// cell (wheel / touchpad scroll). Clamps the top-left within the grid.
    /// Returns `true` if the scroll position actually changed.
    pub fn scroll_by(&mut self, dr: i32, dc: i32) -> bool {
        let new_top = (self.top_row as i64 + dr as i64).clamp(0, (TOTAL_ROWS - 1) as i64) as u32;
        let new_left = (self.left_col as i64 + dc as i64).clamp(0, (TOTAL_COLS - 1) as i64) as u32;
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
        // Extend the fetched window by the number of currently-hidden rows in it.
        // After the row axis collapses there are then still enough *visible* rows
        // to fill the screen; a filtered window would otherwise run short.
        let hidden_in_window = self.hidden_rows.len() as u32;
        let row_end = (self.top_row + self.viewport_rows + row_pad + hidden_in_window)
            .min(TOTAL_ROWS.saturating_sub(1));
        let col_start = self.left_col.saturating_sub(PADDING_COLS);
        let col_end =
            (self.left_col + self.viewport_cols + PADDING_COLS).min(TOTAL_COLS.saturating_sub(1));

        // Evict cells that are no longer in the requested window.
        self.cell_cache.retain(|&(row, col), _| {
            row >= row_start && row <= row_end && col >= col_start && col <= col_end
        });
        // The evicted cells may have carried filter flags; resync the hidden set.
        self.rebuild_hidden_rows();

        let version = self.next_version;
        self.next_version += 1;

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
        for (i, w) in col_widths.iter().enumerate() {
            let col = col_start + i as u32;
            if (*w as f64 - COL_WIDTH).abs() > f64::EPSILON {
                self.col_widths.insert(col, *w);
            } else {
                self.col_widths.remove(&col);
            }
        }
        for (i, h) in row_heights.iter().enumerate() {
            let row = row_start + i as u32;
            if (*h as f64 - ROW_HEIGHT).abs() > f64::EPSILON {
                self.row_heights.insert(row, *h);
            } else {
                self.row_heights.remove(&row);
            }
        }
        for cell in cells {
            // Authoritative value supersedes the optimistic one: the cell is no
            // longer dirty, and any prior error is resolved unless re-reported.
            self.dirty_cells.remove(&(cell.row, cell.col));
            self.formula_errors.remove(&(cell.row, cell.col));
            self.cell_cache.insert(
                (cell.row, cell.col),
                CellData {
                    text: cell.text,
                    raw: cell.raw,
                    bold: cell.bold,
                    italic: cell.italic,
                    underline: cell.underline,
                    strikethrough: cell.strikethrough,
                    h_align: cell.h_align,
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
        let mut rows: Vec<u32> = self
            .cell_cache
            .iter()
            .filter(|(_, c)| c.is_filtered)
            .map(|(&(row, _), _)| row)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        self.hidden_rows = rows;
    }

    /// True if `row` is collapsed by the active shared filter.
    pub fn is_row_hidden(&self, row: u32) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Signed count of *visible* row slots between [`top_row`](Self::top_row) and
    /// `row`. That is the vertical offset (in `ROW_HEIGHT` units) where `row` is
    /// drawn once filtered rows are collapsed out. Mirrors
    /// `GridLayout::visible_offset`.
    pub fn visible_offset(&self, row: u32) -> i64 {
        if self.hidden_rows.is_empty() {
            return row as i64 - self.top_row as i64;
        }
        if row >= self.top_row {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= self.top_row && r < row)
                .count() as i64;
            (row - self.top_row) as i64 - hidden
        } else {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= row && r < self.top_row)
                .count() as i64;
            -((self.top_row - row) as i64 - hidden)
        }
    }

    /// The absolute grid row drawn at visible slot `slot` from the top of the
    /// viewport (skipping hidden rows). Saturates at the last grid row.
    pub fn row_at_slot(&self, slot: u32) -> u32 {
        let mut row = self.top_row;
        let mut seen = 0u32;
        while row < TOTAL_ROWS {
            if !self.is_row_hidden(row) {
                if seen == slot {
                    return row;
                }
                seen += 1;
            }
            row += 1;
        }
        TOTAL_ROWS.saturating_sub(1)
    }

    /// The next visible row at or after `row` (skips a run of hidden rows). Used
    /// by vertical navigation so the caret never lands on a collapsed row.
    pub fn next_visible_row(&self, row: u32) -> u32 {
        let mut r = row;
        while r < TOTAL_ROWS.saturating_sub(1) && self.is_row_hidden(r) {
            r += 1;
        }
        r
    }

    /// The previous visible row at or before `row` (skips a run of hidden rows).
    pub fn prev_visible_row(&self, row: u32) -> u32 {
        let mut r = row;
        while r > 0 && self.is_row_hidden(r) {
            r -= 1;
        }
        r
    }

    /// Walk back from `from` until `n` visible rows have been passed, returning
    /// the resulting row (saturating at 0). Used to place a row on the bottom
    /// viewport slot when scrolling it into view under an active filter.
    fn nth_visible_row_back(&self, from: u32, n: u32) -> u32 {
        let mut r = from;
        let mut counted = 0u32;
        while r > 0 && counted < n {
            r -= 1;
            if !self.is_row_hidden(r) {
                counted += 1;
            }
        }
        r
    }
}
