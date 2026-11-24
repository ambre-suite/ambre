// ambre-trame/src/ui/grid/state.rs
// TrameUiState: construction, zoom, selection, cursor movement, scroll and viewport-data ingestion.

use super::*;
use ambre_commands::SelectionMode;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Delta, Len, Pos, Span};

impl TrameUiState {
    /// The factors the grid supports (`ZOOM-002`). The chip lists the stops
    /// of the one ladder inside it (`ZOOM-006`).
    pub const ZOOM_RANGE: ambre_core::ZoomRange<ambre_core::Doc> = ambre_core::ZoomRange::new(
        ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(0.5),
        ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(2.0),
    );

    pub fn new(canvas_width: Px<Surface>, canvas_height: Px<Surface>) -> Self {
        let mut s = Self {
            top_row: Pos::<Row>::ZERO,
            left_col: Pos::<Col>::ZERO,
            selection: Selection::single(CellPos::ORIGIN),
            viewport_rows: Len::<Row>::ZERO,
            viewport_cols: Len::<Col>::ZERO,
            canvas_width,
            canvas_height,
            zoom: ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::ONE,
            cell_cache: HashMap::default(),
            dirty_cells: HashSet::default(),
            formula_errors: HashSet::default(),
            accepted_version: ambre_core::Seq::ZERO,
            active_sheet: Id::<Sheet>::from_text(String::new()),
            next_version: ambre_core::Seq::ZERO.next(),
            full_request: None,
            frozen: crate::message::FrozenPane::NONE,
            filtered_col: None,
            hidden_rows: Vec::new(),
            col_widths: HashMap::default(),
            row_heights: HashMap::default(),
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
    pub fn resize(&mut self, width: Px<Surface>, height: Px<Surface>) {
        self.canvas_width = width;
        self.canvas_height = height;
        self.recompute_viewport_dims();
    }

    /// Bottom edge (logical px) of the scrollable grid area. The sheet-tab strip
    /// moved to DOM chrome (`SheetTabs`), so the grid now fills the full canvas
    /// height. The DOM strip occupies its own shell row outside this surface.
    pub fn grid_bottom(&self) -> Px<Surface> {
        self.canvas_height.max(self.header_h())
    }

    fn recompute_viewport_dims(&mut self) {
        let grid_h = (self.grid_bottom() - self.header_h()).max(Px::<Surface>::ZERO);
        let grid_w = (self.canvas_width - self.header_w()).max(Px::<Surface>::ZERO);
        let (row_h, col_w) = (self.scaled(ROW_HEIGHT), self.scaled(COL_WIDTH));
        // ceil() already accounts for the partially-visible row/col at the edge.
        // No +1 needed: viewport_rows is the count of rows that start within the
        // grid area, whether fully or partially visible. scroll_to_include and
        // cell_pixel_rect must agree on this count, or the active cell can
        // appear "in viewport" to the scroll logic but off-canvas to the renderer.
        self.viewport_rows = Len::<Row>::new((grid_h / row_h).ceil() as u32);
        self.viewport_cols = Len::<Col>::new((grid_w / col_w).ceil() as u32);
    }

    /// The grid's zoom, read through the one engine (`UI-THREAD-012`).
    pub fn zoom_engine(&self) -> ambre_core::Zoom<ambre_core::Doc> {
        ambre_core::Zoom::at(Self::ZOOM_RANGE, self.zoom)
    }

    /// Set the grid zoom factor (clamped to the grid's range), keeping the
    /// top-left scroll origin pinned. Recomputes the visible range. Returns
    /// `true` when the zoom actually changed, so the caller can repaint and
    /// re-request the (now differently sized) viewport.
    pub fn set_zoom(&mut self, zoom: ambre_core::Scale<ambre_core::Doc, ambre_core::View>) -> bool {
        let z = self.zoom_engine().to(zoom).factor();
        if (z.get() - self.zoom.get()).abs() < f64::EPSILON {
            return false;
        }
        self.zoom = z;
        self.recompute_viewport_dims();
        true
    }

    /// Zoom to `asked` about a point of the canvas (`ZOOM-003`). The factor
    /// comes from the one engine, and the anchor is the grid's own mapping.
    /// The grid scrolls by whole cells, so the cell under the point stays.
    /// The frozen band and the first row and column bound that scroll. A point over a
    /// header or past the cells keeps the top-left cell. Returns `true`
    /// when the zoom changed.
    pub fn zoom_about(
        &mut self,
        asked: ambre_core::Scale<ambre_core::Doc, ambre_core::View>,
        at: ambre_core::Point<Px<ambre_core::View>>,
    ) -> bool {
        let under = self.cell_at(at);
        if !self.set_zoom(asked) {
            return false;
        }
        let Some(cell) = under else {
            return true;
        };
        // Variable sizes and hidden rows can leave the first pan a cell short.
        // The pan repeats until the cell is back or the grid stops.
        for _ in 0..4 {
            let Some(now) = self.cell_at(at) else {
                break;
            };
            if now == cell
                || !self.show_from(
                    self.top_row + (cell.row - now.row),
                    self.left_col + (cell.col - now.col),
                )
            {
                break;
            }
        }
        true
    }

    /// Zoom so the selection shows whole, its top-left cell at the top-left
    /// of the cells, through the one engine (`ZOOM-007`). A band of whole
    /// columns or rows frames its own axis alone. The factor is clamped to
    /// the grid's range, so a range too large to show stops at the floor.
    /// The selection stays as it was (`ZOOM-010`). Returns `true` when the
    /// view moved.
    pub fn zoom_to_selection(&mut self) -> bool {
        let rect = self.selection.bounds();
        let kind = self.selection.kind;
        let zoom = self.zoom.get();
        // Past what the floor shows, the sum only proves the floor governs.
        let floor = Self::ZOOM_RANGE.floor().get();
        let cap_w = self.canvas_width.get() / floor;
        let cap_h = self.grid_bottom().get() / floor;
        let at_one = |len: Px<Doc>| f64::from(len.get());
        let mut cols = 0.0;
        for col in rect.top_left.col.through(rect.bottom_right.col) {
            cols += at_one(self.col_widths.get(&col).copied().unwrap_or(COL_WIDTH));
            if cols > cap_w {
                break;
            }
        }
        let mut rows = 0.0;
        for row in rect.top_left.row.through(rect.bottom_right.row) {
            if !self.is_row_hidden(row) {
                rows += at_one(self.row_heights.get(&row).copied().unwrap_or(ROW_HEIGHT));
            }
            if rows > cap_h {
                break;
            }
        }
        // The header strips and the frozen bands zoom with the cells, so they
        // join the content the canvas must hold.
        let chrome_w = HEADER_WIDTH.get() + self.frozen_w().get() / zoom;
        let chrome_h = HEADER_HEIGHT.get() + self.frozen_h().get() / zoom;
        // A band leaves the other axis free: a sliver there never governs.
        let (across, down) = match kind {
            SelKind::Cells => (cols + chrome_w, rows + chrome_h),
            SelKind::Cols => (cols + chrome_w, f64::from(f32::MIN_POSITIVE)),
            SelKind::Rows => (f64::from(f32::MIN_POSITIVE), rows + chrome_h),
        };
        let content =
            ambre_core::Vector::new(Px::<Doc>::new(across as f32), Px::<Doc>::new(down as f32));
        let view = ambre_core::Vector::new(
            self.canvas_width.view_length(),
            self.grid_bottom().view_length(),
        );
        let Some(framed) = self.zoom_engine().framing(content, view) else {
            return false;
        };
        let zoomed = self.set_zoom(framed.factor());
        let top = match kind {
            SelKind::Cols => self.top_row,
            SelKind::Cells | SelKind::Rows => rect.top_left.row,
        };
        let left = match kind {
            SelKind::Rows => self.left_col,
            SelKind::Cells | SelKind::Cols => rect.top_left.col,
        };
        let scrolled = self.show_from(top, left);
        zoomed || scrolled
    }

    /// The active cell row (the selection's active corner).
    pub fn active_row(&self) -> Pos<Row> {
        self.selection.active.row
    }

    /// The active cell column (the selection's active corner).
    pub fn active_col(&self) -> Pos<Col> {
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
    /// [`SelectRange`](crate::message::TrameEvent::SelectRange) after an unmerge.
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
        dr: Delta<Row>,
        dc: Delta<Col>,
        extend: SelectionMode,
    ) -> Option<CursorMove> {
        use std::cmp::Ordering;
        let at = self.active_cell();
        // Stepping off a merge master must clear its whole span, else the
        // candidate lands on a covered sub-cell and snaps straight back (cursor
        // trapped). Advance by the span in the travel direction.
        let (mut row, mut col) = (at.row + dr, at.col + dc);
        if let Some(block) = self.merge_master_at(at) {
            match dc.cmp(&Delta::<Col>::ZERO) {
                Ordering::Greater => col = block.bottom_right.col.next(),
                Ordering::Less => col = block.top_left.col - Len::<Col>::ONE,
                Ordering::Equal => {}
            }
            match dr.cmp(&Delta::<Row>::ZERO) {
                Ordering::Greater => row = block.bottom_right.row.next(),
                Ordering::Less => row = block.top_left.row - Len::<Row>::ONE,
                Ordering::Equal => {}
            }
        }
        let (row, col) = (row.min(LAST_ROW), col.min(LAST_COL));
        // A vertical step never lands on a collapsed (filtered) row: skip the run
        // of hidden rows in the travel direction.
        let row = match dr.cmp(&Delta::<Row>::ZERO) {
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
    pub fn last_content_col(&self, row: Pos<Row>) -> Option<Pos<Col>> {
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
        // A frozen line is always on screen. The scrolled pane holds the
        // slots the frozen band leaves, never fewer than one.
        let rows_shown = self
            .viewport_rows
            .saturating_sub(self.frozen.rows)
            .max(Len::<Row>::ONE);
        let cols_shown = self
            .viewport_cols
            .saturating_sub(self.frozen.cols)
            .max(Len::<Col>::ONE);
        let row_scrolls = at.row >= self.frozen.rows.as_end();
        let col_scrolls = at.col >= self.frozen.cols.as_end();
        if !row_scrolls {
            // A frozen row is on screen whatever the scroll.
        } else if at.row < self.top_row {
            self.top_row = at.row;
            changed = true;
        } else if self.visible_offset(at.row) >= Delta::<Row>::from_len(rows_shown) {
            // `row` sits past the last visible slot: pull the top down so `row`
            // lands on the bottom slot, counting in collapsed (visible) rows.
            let back = rows_shown.saturating_sub(Len::<Row>::ONE);
            self.top_row = self
                .nth_visible_row_back(at.row, back)
                .max(self.frozen.rows.as_end());
            changed = true;
        }
        if !col_scrolls {
            // A frozen column is on screen whatever the scroll.
        } else if at.col < self.left_col {
            self.left_col = at.col;
            changed = true;
        } else if at.col >= self.left_col + cols_shown {
            self.left_col = (at.col - cols_shown).next();
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
    pub(crate) fn scroll_by(&mut self, dr: Delta<Row>, dc: Delta<Col>) -> bool {
        self.show_from(self.top_row + dr, self.left_col + dc)
    }

    /// Put `top` and `left` at the top-left of the scrolled cells, as a zoom
    /// places them about its anchor. A placement, not a pan: no motion runs
    /// through it (`KINETIC-001`). Clamps the corner within the grid. Returns
    /// `true` if the scroll position actually changed.
    fn show_from(&mut self, top: Pos<Row>, left: Pos<Col>) -> bool {
        let new_top = top.min(LAST_ROW).max(self.frozen.rows.as_end());
        let new_left = left.min(LAST_COL).max(self.frozen.cols.as_end());
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
    fn visible_col_span(&self) -> Len<Col> {
        let right = self.canvas_width;
        let mut x = self.header_w() + self.frozen_w();
        let mut col = self.left_col;
        while x < right && col < TOTAL_COLS.as_end() {
            x += self.col_width_of(col);
            col = col.next();
        }
        col.len_from(self.left_col)
    }

    /// How many rows from [`top_row`](Self::top_row) start inside the grid area,
    /// walking their real heights (`TRAME-UI-144`). A row a filter hides counts,
    /// and takes no height.
    fn visible_row_span(&self) -> Len<Row> {
        let bottom = self.grid_bottom();
        let mut y = self.header_h() + self.frozen_h();
        let mut row = self.top_row;
        while y < bottom && row < TOTAL_ROWS.as_end() {
            if !self.is_row_hidden(row) {
                y += self.row_height_of(row);
            }
            row = row.next();
        }
        row.len_from(self.top_row)
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
        let hidden_in_window = Len::<Row>::of(
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
        let evicted: Vec<CellPos> = self
            .cell_cache
            .keys()
            .copied()
            .filter(|&at| !window.contains(at) && !self.in_frozen_band(window, at))
            .collect();
        for at in evicted {
            self.cell_cache.remove(&at);
        }
        // The evicted cells may have carried filter flags; resync the hidden set.
        self.rebuild_hidden_rows();

        let version = self.next_version;
        self.next_version = self.next_version.next();
        self.full_request = Some((version, window));

        TrameCommand::ScrollViewport {
            version,
            row_start,
            row_end,
            col_start,
            col_end,
        }
    }

    /// The request that re-fetches the cells of `range` the window shows, for
    /// a change the Logic thread made after it answered `served`
    /// (`TRAME-UI-184`). It evicts nothing.
    ///
    /// `None` when the window shows none of `range`, or when a request for
    /// the whole window was sent after `served`. The Logic thread answers that
    /// one after the change, so the answer already shows it.
    pub fn range_viewport_cmd(
        &mut self,
        range: CellRect,
        served: ambre_core::Seq<ambre_bus::ViewportVersion>,
    ) -> Option<TrameCommand> {
        let (asked, window) = self.full_request?;
        if asked > served {
            return None;
        }
        // A change in a frozen band the window does not hold re-fetches the
        // whole window: its answer carries the bands (`TRAME-UI-192`).
        let shown = intersection(window, range).or_else(|| {
            let band = CellRect::spanning(
                CellPos::new(Pos::<Row>::ZERO, Pos::<Col>::ZERO),
                CellPos::new(window.bottom_right.row, window.bottom_right.col),
            );
            let touched = intersection(band, range)?;
            let frozen = CellPos::new(touched.top_left.row, touched.top_left.col);
            (self.in_frozen_band(window, frozen)
                || self.in_frozen_band(window, touched.bottom_right))
            .then_some(window)
        })?;
        let version = self.next_version;
        self.next_version = self.next_version.next();
        Some(TrameCommand::ScrollViewport {
            version,
            row_start: shown.top_left.row,
            row_end: shown.bottom_right.row,
            col_start: shown.top_left.col,
            col_end: shown.bottom_right.col,
        })
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
            frozen,
        } = update
        else {
            return;
        };
        if version < self.accepted_version {
            return;
        }
        self.accepted_version = version;
        self.adopt_frozen(&frozen);
        // Adopt the per-axis metrics for the covered window (absolute keys). Only
        // a non-default entry is stored. A column/row reset to the default drops
        // out of the map, so it falls back to the uniform metric
        // (`trame-resize`).
        for (col, w) in Span::<Col>::from_len(col_start, Len::<Col>::of(col_widths.len()))
            .into_iter()
            .zip(&col_widths)
        {
            if *w != COL_WIDTH {
                self.col_widths.insert(col, *w);
            } else {
                self.col_widths.remove(&col);
            }
        }
        for (row, h) in Span::<Row>::from_len(row_start, Len::<Row>::of(row_heights.len()))
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
            self.cell_cache.insert(at, CellData::from(cell));
        }
        self.rebuild_hidden_rows();
    }

    /// Adopt the frozen pane an answer carries and the sizes of its frozen
    /// lines. Then move the scroll origin out of the frozen bands
    /// (`TRAME-UI-193`).
    fn adopt_frozen(&mut self, frozen: &crate::message::FrozenView) {
        self.frozen = frozen.pane;
        for (row, h) in
            Span::<Row>::from_len(Pos::<Row>::ZERO, Len::<Row>::of(frozen.row_heights.len()))
                .into_iter()
                .zip(&frozen.row_heights)
        {
            if *h != ROW_HEIGHT {
                self.row_heights.insert(row, *h);
            } else {
                self.row_heights.remove(&row);
            }
        }
        for (col, w) in
            Span::<Col>::from_len(Pos::<Col>::ZERO, Len::<Col>::of(frozen.col_widths.len()))
                .into_iter()
                .zip(&frozen.col_widths)
        {
            if *w != COL_WIDTH {
                self.col_widths.insert(col, *w);
            } else {
                self.col_widths.remove(&col);
            }
        }
        self.top_row = self.top_row.max(self.frozen.rows.as_end());
        self.left_col = self.left_col.max(self.frozen.cols.as_end());
    }

    /// Whether `at` lies in a frozen band of the window `window`. A band is
    /// the frozen rows over its columns, the frozen columns over its rows, or
    /// the corner of both. A scroll keeps such a cell (`TRAME-UI-192`).
    fn in_frozen_band(&self, window: CellRect, at: CellPos) -> bool {
        let row_frozen = at.row < self.frozen.rows.as_end();
        let col_frozen = at.col < self.frozen.cols.as_end();
        let rows = at.row >= window.top_left.row && at.row <= window.bottom_right.row;
        let cols = at.col >= window.top_left.col && at.col <= window.bottom_right.col;
        (row_frozen && (cols || col_frozen)) || (col_frozen && rows)
    }

    /// Recompute [`Self::hidden_rows`] (sorted, deduped) from the `is_filtered`
    /// flag of the cached cells. Called whenever the cache changes (a viewport
    /// batch arrived, or rows were evicted on scroll). The flag is row-level, so
    /// any cached cell of a row is authoritative for it.
    fn rebuild_hidden_rows(&mut self) {
        let mut rows: Vec<Pos<Row>> = self
            .cell_cache
            .iter()
            .filter(|(_, c)| c.is_filtered == crate::message::RowFilter::Filtered)
            .map(|(at, _)| at.row)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        self.hidden_rows = rows;
    }

    /// True if `row` is collapsed by the active shared filter.
    pub fn is_row_hidden(&self, row: Pos<Row>) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Signed count of *visible* row slots between [`top_row`](Self::top_row) and
    /// `row`. That is the vertical offset (in `ROW_HEIGHT` units) where `row` is
    /// drawn once filtered rows are collapsed out. Mirrors
    /// `GridLayout::visible_offset`.
    pub fn visible_offset(&self, row: Pos<Row>) -> Delta<Row> {
        let (lo, hi) = (row.min(self.top_row), row.max(self.top_row));
        let hidden = self
            .hidden_rows
            .iter()
            .filter(|&&r| r >= lo && r < hi)
            .count();
        let slots = Delta::<Row>::from_len(hi.len_from(lo).saturating_sub(Len::<Row>::of(hidden)));
        if row >= self.top_row {
            slots
        } else {
            slots.reversed()
        }
    }

    /// The absolute grid row drawn at visible slot `slot` from the top of the
    /// viewport (skipping hidden rows). Saturates at the last grid row.
    pub fn row_at_slot(&self, slot: Len<Row>) -> Pos<Row> {
        let mut row = self.top_row;
        let mut seen = Len::<Row>::ZERO;
        while row < TOTAL_ROWS.as_end() {
            if !self.is_row_hidden(row) {
                if seen == slot {
                    return row;
                }
                seen += Len::<Row>::ONE;
            }
            row = row.next();
        }
        LAST_ROW
    }

    /// The next visible row at or after `row` (skips a run of hidden rows). Used
    /// by vertical navigation so the caret never lands on a collapsed row.
    pub fn next_visible_row(&self, row: Pos<Row>) -> Pos<Row> {
        let mut r = row;
        while r < LAST_ROW && self.is_row_hidden(r) {
            r = r.next();
        }
        r
    }

    /// The previous visible row at or before `row` (skips a run of hidden rows).
    pub fn prev_visible_row(&self, row: Pos<Row>) -> Pos<Row> {
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
    fn nth_visible_row_back(&self, from: Pos<Row>, n: Len<Row>) -> Pos<Row> {
        let mut r = from;
        let mut counted = Len::<Row>::ZERO;
        while counted < n
            && let Some(prev) = r.prev()
        {
            r = prev;
            if !self.is_row_hidden(r) {
                counted += Len::<Row>::ONE;
            }
        }
        r
    }
}

/// The cells both rectangles cover, or `None` when they share none.
fn intersection(a: CellRect, b: CellRect) -> Option<CellRect> {
    let top = a.top_left.row.max(b.top_left.row);
    let left = a.top_left.col.max(b.top_left.col);
    let bottom = a.bottom_right.row.min(b.bottom_right.row);
    let right = a.bottom_right.col.min(b.bottom_right.col);
    (top <= bottom && left <= right)
        .then(|| CellRect::spanning(CellPos::new(top, left), CellPos::new(bottom, right)))
}
