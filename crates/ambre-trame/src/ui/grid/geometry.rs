// ambre-trame/src/ui/grid/geometry.rs
// TrameUiState: pixel/axis geometry, hit-testing, merge and fill-handle rectangles.

use super::*;
use ambre_draw::HitTest;
use ambre_draw::kinetic::EdgeSpeed;

/// The cell `(row, col)` under a canvas point (`UI-THREAD-001`): the grid's
/// geometry answers, through [`TrameUiState::cell_at`].
impl HitTest for TrameUiState {
    type Target = (u32, u32);

    fn hit_test(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        self.cell_at(x, y)
    }
}

impl TrameUiState {
    /// Record a formula evaluation failure for `(row, col)` (from
    /// `TrameEvent::FormulaError`). The cell renders with a corner error marker
    /// until a valid `ViewportData` value clears it.
    pub fn set_formula_error(&mut self, row: u32, col: u32, error: String) {
        self.dirty_cells.remove(&(row, col));
        self.formula_errors.insert((row, col), error);
    }

    /// Optimistically set a cell's text after a local edit commit, preserving
    /// its formatting. Keeps the local view in sync without a Logic round-trip
    /// (the CRDT echo would only re-confirm what we already show).
    pub fn set_cell_text(&mut self, row: u32, col: u32, text: String) {
        let entry = self.cell_cache.entry((row, col)).or_default();
        // `text` here is the raw buffer. For a formula we cannot evaluate it on
        // the UI thread, so the displayed result stays blank until the next
        // ViewportData carries the computed value. Never echo the formula
        // source as its own result. For a plain literal, raw == display.
        let is_formula = text.starts_with('=');
        entry.raw = text.clone();
        // A literal the engine would read as a number aligns as one until the
        // pack confirms it.
        entry.numeric = !is_formula && text.parse::<f64>().is_ok_and(f64::is_finite);
        entry.text = if is_formula { String::new() } else { text };
        // The shown value is now optimistic: a formula awaits recompute, and a
        // literal awaits the CRDT echo. Either way the prior error is stale.
        self.formula_errors.remove(&(row, col));
        self.dirty_cells.insert((row, col));
    }

    /// Mint the next monotonic command version (shared with the viewport
    /// request). Used by commands that carry a `version` for echo ordering, for example
    /// [`TrameCommand::ResizeCol`].
    pub fn bump_version(&mut self) -> ambre_bus::ViewportVersion {
        let v = self.next_version;
        self.next_version = self.next_version.next();
        v
    }

    /// Width (logical px) of column `col`: its resized entry, else [`COL_WIDTH`].
    pub fn col_width_of(&self, col: u32) -> f64 {
        self.col_widths
            .get(&col)
            .map(|&w| w as f64)
            .unwrap_or(COL_WIDTH)
            * self.zoom
    }

    /// Height (logical px) of row `row`: its resized entry, else [`ROW_HEIGHT`].
    pub fn row_height_of(&self, row: u32) -> f64 {
        self.row_heights
            .get(&row)
            .map(|&h| h as f64)
            .unwrap_or(ROW_HEIGHT)
            * self.zoom
    }

    /// Width (logical px) of the fixed row-number strip on the left, scaled by
    /// the current [`zoom`](Self::zoom). The header strips zoom with the grid.
    pub fn header_w(&self) -> f64 {
        HEADER_WIDTH * self.zoom
    }

    /// Height (logical px) of the fixed column-letter strip on top, scaled by
    /// the current [`zoom`](Self::zoom).
    pub fn header_h(&self) -> f64 {
        HEADER_HEIGHT * self.zoom
    }

    /// X of column `col`'s left edge relative to the canvas, summing the variable
    /// widths from [`left_col`](Self::left_col). Columns left of the scroll origin
    /// yield a value smaller than [`HEADER_WIDTH`] (callers clip).
    pub fn col_x(&self, col: u32) -> f64 {
        let mut x = self.header_w();
        if col >= self.left_col {
            for c in self.left_col..col {
                x += self.col_width_of(c);
            }
        } else {
            for c in col..self.left_col {
                x -= self.col_width_of(c);
            }
        }
        x
    }

    /// Y of row `row`'s top edge relative to the canvas, summing the *visible*
    /// rows' variable heights from [`top_row`](Self::top_row) (collapsing hidden
    /// rows, `trame-shared-filter`). Mirrors `GridLayout::row_top`.
    pub fn row_y(&self, row: u32) -> f64 {
        let cells_top = self.header_h();
        let mut y = cells_top;
        if row >= self.top_row {
            for r in self.top_row..row {
                if !self.is_row_hidden(r) {
                    y += self.row_height_of(r);
                }
            }
        } else {
            for r in row..self.top_row {
                if !self.is_row_hidden(r) {
                    y -= self.row_height_of(r);
                }
            }
        }
        y
    }

    /// Absolute grid column whose band contains logical x `px` (>= the row
    /// header strip), walking the variable widths. Used by a whole-axis drag,
    /// where the pointer may be below the header strip. `None` left of the grid.
    pub fn col_at_x(&self, px: f64) -> Option<u32> {
        if px < self.header_w() {
            return None;
        }
        let mut col = self.left_col;
        let mut cx = self.header_w();
        while col < TOTAL_COLS {
            let w = self.col_width_of(col);
            if px < cx + w {
                return Some(col);
            }
            cx += w;
            col += 1;
        }
        None
    }

    /// Insertion slot for a column reorder drop at logical x `px`: the column
    /// index the grabbed column would land *before*. It snaps to the nearer
    /// vertical boundary: the left edge of the column under the cursor, or the
    /// next one past its midpoint. That last option keeps the final position
    /// reachable. `None` left of the grid. The returned index may equal the
    /// column count (drop at the end).
    pub fn col_drop_slot(&self, px: f64) -> Option<u32> {
        let col = self.col_at_x(px)?;
        let mid = self.col_x(col) + self.col_width_of(col) / 2.0;
        Some(if px >= mid { col + 1 } else { col })
    }

    /// Insertion slot for a row reorder drop at logical y `py`: the row index
    /// the grabbed row would land *above*. It snaps to the nearer horizontal
    /// boundary. Row analogue of [`col_drop_slot`](Self::col_drop_slot).
    pub fn row_drop_slot(&self, py: f64) -> Option<u32> {
        let row = self.row_at_y(py)?;
        let mid = self.row_y(row) + self.row_height_of(row) / 2.0;
        Some(if py >= mid { row + 1 } else { row })
    }

    /// Absolute *visible* grid row whose band contains logical y `py` (>= the
    /// column header strip), walking the variable heights and skipping hidden
    /// rows. `None` above the grid.
    pub fn row_at_y(&self, py: f64) -> Option<u32> {
        if py < self.header_h() {
            return None;
        }
        let mut row = self.top_row;
        let mut cy = self.header_h();
        while row < TOTAL_ROWS {
            if !self.is_row_hidden(row) {
                let h = self.row_height_of(row);
                if py < cy + h {
                    return Some(row);
                }
                cy += h;
            }
            row += 1;
        }
        None
    }

    /// The cell a held drag points at: the cell under `(x, y)`, or the
    /// nearest edge cell when the pointer has left the cell area. A drag past
    /// an edge keeps reaching the cells the scroll brings in.
    pub fn drag_cell_at(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let x = x.clamp(
            self.header_w(),
            (self.canvas_width - 1.0).max(self.header_w()),
        );
        let y = y.clamp(
            self.header_h(),
            (self.grid_bottom() - 1.0).max(self.header_h()),
        );
        self.cell_at(x, y)
    }

    /// The speed a drag held at `(x, y)` scrolls the grid at, in logical
    /// pixels per millisecond on each axis (`KINETIC-007`). Each edge is the
    /// cell area's, past the header strips; zero away from them.
    pub fn edge_scroll_speed(&self, x: f64, y: f64) -> (EdgeSpeed, EdgeSpeed) {
        let (left, top) = (self.header_w(), self.header_h());
        (
            EdgeSpeed::at((x - left) as f32, (self.canvas_width - left) as f32),
            EdgeSpeed::at((y - top) as f32, (self.grid_bottom() - top) as f32),
        )
    }

    /// Cell `(row, col)` at logical pixel `(x, y)`, or `None` if the point is
    /// in a header strip or past the grid bounds. Inverse of `cell_pixel_rect`.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let cells_top = self.header_h();
        if x < self.header_w() || y < cells_top {
            return None;
        }
        if x >= self.canvas_width || y >= self.grid_bottom() {
            return None;
        }
        // Walk the variable column widths from the scroll origin.
        let mut col = self.left_col;
        let mut cx = self.header_w();
        loop {
            let w = self.col_width_of(col);
            if x < cx + w {
                break;
            }
            cx += w;
            col += 1;
            if col >= TOTAL_COLS {
                return None;
            }
        }
        // Walk the *visible* rows' variable heights from the scroll origin.
        let mut row = self.top_row;
        let mut cy = cells_top;
        loop {
            if !self.is_row_hidden(row) {
                let h = self.row_height_of(row);
                if y < cy + h {
                    break;
                }
                cy += h;
            }
            row += 1;
            if row >= TOTAL_ROWS {
                return None;
            }
        }
        Some((row, col))
    }

    /// Resolve a pointer at logical `(x, y)` to a header zone
    /// (`trame-headers`), or `None` when the point is over the grid body (not a
    /// header strip). The resize bands (a +/- [`RESIZE_BAND`] zone around a
    /// boundary) take priority over the body so a grab on the edge resizes rather
    /// than selects. Variable-metric aware (walks the per-axis widths/heights).
    pub fn header_hit(&self, x: f64, y: f64) -> Option<HeaderHit> {
        let in_col_strip = (0.0..self.header_h()).contains(&y);
        let in_row_strip = (0.0..self.header_w()).contains(&x);
        // Top-left corner -> select-all.
        if in_col_strip && in_row_strip {
            return Some(HeaderHit::Corner);
        }
        // Column header strip (top): resolve the column under x, with its right
        // border arming a resize.
        if in_col_strip && x >= self.header_w() && x < self.canvas_width {
            let mut col = self.left_col;
            let mut cx = self.header_w();
            while col < TOTAL_COLS {
                let right = cx + self.col_width_of(col);
                // Boundary band: the top sliver hosts the inline "+" insert
                // (insert left of the next column); the rest arms a resize.
                if (x - right).abs() <= RESIZE_BAND {
                    return Some(if y < self.header_h() * 0.5 {
                        HeaderHit::ColInsert(col + 1)
                    } else {
                        HeaderHit::ColBorder(col)
                    });
                }
                if x < right {
                    return Some(HeaderHit::ColBody(col));
                }
                cx = right;
                col += 1;
            }
            return None;
        }
        // Row header strip (left): resolve the visible row under y, with its
        // bottom border arming a resize.
        if in_row_strip && y >= self.header_h() && y < self.grid_bottom() {
            let mut row = self.top_row;
            let mut cy = self.header_h();
            while row < TOTAL_ROWS {
                if self.is_row_hidden(row) {
                    row += 1;
                    continue;
                }
                let bottom = cy + self.row_height_of(row);
                if (y - bottom).abs() <= RESIZE_BAND {
                    return Some(if x < self.header_w() * 0.5 {
                        HeaderHit::RowInsert(row + 1)
                    } else {
                        HeaderHit::RowBorder(row)
                    });
                }
                if y < bottom {
                    return Some(HeaderHit::RowBody(row));
                }
                cy = bottom;
                row += 1;
            }
            return None;
        }
        None
    }

    /// If `(row, col)` falls inside a merged block, return its master corner and
    /// span `(master_row, master_col, col_span, row_span)`.
    ///
    /// A covered cell carries no back-reference to its master, so this scans the
    /// cached merge masters. The cache is bounded to the viewport + padding, so
    /// the scan is cheap. The master itself resolves to its own block.
    pub fn merge_master_at(&self, row: u32, col: u32) -> Option<(u32, u32, u32, u32)> {
        self.cell_cache.iter().find_map(|(&(mr, mc), c)| {
            let (col_span, row_span) = c.merge_span?;
            (row >= mr && row < mr + row_span && col >= mc && col < mc + col_span)
                .then_some((mr, mc, col_span, row_span))
        })
    }

    /// Screen pixel rect (x, y, w, h) of the merged block the active cell belongs
    /// to. `None` when the active cell is not merged (or scrolled out). Lets the
    /// overlay draw the active-cell border around the whole merge.
    pub fn active_merge_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let (mr, mc, cs, rs) = self.merge_master_at(self.active_row(), self.active_col())?;
        self.range_pixel_rect(mr, mc, mr + rs - 1, mc + cs - 1)
    }

    /// Screen pixel rect (x, y, w, h) of cell (row, col) relative to the
    /// top-left corner of the canvas. `None` if the cell is outside the
    /// visible area.
    pub fn cell_pixel_rect(&self, row: u32, col: u32) -> Option<(f64, f64, f64, f64)> {
        // A collapsed (filtered) row is not drawn at all.
        if self.is_row_hidden(row) {
            return None;
        }
        // Variable-metric edges; the helpers handle a cell left/above the
        // viewport (negative offset) and collapse over hidden rows.
        let cells_top = self.header_h();
        let x = self.col_x(col);
        let y = self.row_y(row);
        let cw = self.col_width_of(col);
        let rh = self.row_height_of(row);

        if x + cw <= self.header_w() || x >= self.canvas_width {
            return None;
        }
        if y + rh <= cells_top || y >= self.grid_bottom() {
            return None;
        }
        Some((x, y, cw, rh))
    }

    /// Screen pixel rect (x, y, w, h) covering the cell range `(r1, c1)..=(r2,
    /// c2)`, clipped to the scrollable grid area. The clip never reaches under a
    /// header strip or the tab strip. Returns `None` if the range is entirely
    /// outside the visible area. Used to fill / outline a multi-cell selection.
    pub fn range_pixel_rect(
        &self,
        r1: u32,
        c1: u32,
        r2: u32,
        c2: u32,
    ) -> Option<(f64, f64, f64, f64)> {
        let cells_top = self.header_h();
        // Unclipped edges (signed: the range may start left/above the viewport).
        // Row edges are in collapsed (visible-row) space. The top is the first
        // row's slot; the bottom is one slot past the last *visible* row of the
        // range. A range straddling hidden rows therefore shrinks to its visible
        // height.
        let x_left = self.col_x(c1);
        let x_right = self.col_x(c2) + self.col_width_of(c2);
        let y_top = self.row_y(r1);
        // Bottom edge: just below r2 if r2 is visible, else its own top (its
        // successors have already collapsed up onto it).
        let y_bot = self.row_y(r2)
            + if self.is_row_hidden(r2) {
                0.0
            } else {
                self.row_height_of(r2)
            };

        // Clip to the visible cell area.
        let cx1 = x_left.max(self.header_w());
        let cx2 = x_right.min(self.canvas_width);
        let cy1 = y_top.max(cells_top);
        let cy2 = y_bot.min(self.grid_bottom());
        if cx2 <= cx1 || cy2 <= cy1 {
            return None;
        }
        Some((cx1, cy1, cx2 - cx1, cy2 - cy1))
    }

    /// Grow `rect` so it fully contains every merged block it intersects. A merge
    /// is one logical cell (`trame-cell-merges`), so a selection touching part of
    /// a block must cover the whole block. That holds for drawing: the drag reads
    /// as selecting the merge at once, not cell by cell. It holds for the
    /// autofill handle too, which is anchored on the block's extent corner.
    /// Iterates to a fixpoint because expanding onto one merge can bring a
    /// neighbouring one into range. Scans the cached merge masters (bounded to
    /// the viewport, so it is cheap).
    pub fn expand_to_merges(&self, rect: CellRect) -> CellRect {
        let (mut r1, mut c1, mut r2, mut c2) = rect;
        loop {
            let mut grew = false;
            for (&(mr, mc), data) in &self.cell_cache {
                let Some((cs, rs)) = data.merge_span else {
                    continue;
                };
                let (br, bc) = (mr + rs - 1, mc + cs - 1);
                // Block (mr,mc)..=(br,bc) intersects the current rectangle?
                if mr <= r2 && br >= r1 && mc <= c2 && bc >= c1 {
                    let (nr1, nc1, nr2, nc2) = (r1.min(mr), c1.min(mc), r2.max(br), c2.max(bc));
                    if (nr1, nc1, nr2, nc2) != (r1, c1, r2, c2) {
                        (r1, c1, r2, c2) = (nr1, nc1, nr2, nc2);
                        grew = true;
                    }
                }
            }
            if !grew {
                return (r1, c1, r2, c2);
            }
        }
    }

    /// The fill granularity `(rows, cols)` of an autofill source `bounds`
    /// (`trame-autofill`). When the source is tiled by equal merged blocks (one
    /// block or a uniform grid of identically sized merges), returns the span. The
    /// drag then snaps block by block and the release re-merges each tile (Excel
    /// behaviour). Otherwise `(1, 1)` keeps the plain per-cell fill. A source that
    /// mixes merges of different sizes or a merge with extra cells is not
    /// block-aligned and falls back to `(1, 1)`.
    pub fn fill_source_block(&self, bounds: CellRect) -> (u32, u32) {
        let (r1, c1, r2, c2) = self.expand_to_merges(bounds);
        let Some(&(cs, rs)) = self
            .cell_cache
            .get(&(r1, c1))
            .and_then(|d| d.merge_span.as_ref())
        else {
            return (1, 1);
        };
        let (bh, bw) = (rs, cs);
        if !(r2 - r1 + 1).is_multiple_of(bh) || !(c2 - c1 + 1).is_multiple_of(bw) {
            return (1, 1);
        }
        let mut top = r1;
        while top <= r2 {
            let mut left = c1;
            while left <= c2 {
                match self
                    .cell_cache
                    .get(&(top, left))
                    .and_then(|d| d.merge_span.as_ref())
                {
                    Some(&(s_cs, s_rs)) if s_rs == bh && s_cs == bw => {}
                    _ => return (1, 1),
                }
                left += bw;
            }
            top += bh;
        }
        (bh, bw)
    }

    /// The selection rectangles to draw/operate on, each expanded to whole
    /// merges (see [`expand_to_merges`](Self::expand_to_merges)). Mirrors
    /// [`Selection::rects`] but merge-aware.
    pub fn selection_rects(&self) -> Vec<CellRect> {
        self.selection
            .rects()
            .map(|r| self.expand_to_merges(r))
            .collect()
    }

    /// Screen pixel rect (x, y, w, h) of the autofill handle: a small square
    /// centred on the bottom-right corner of the selection (`trame-autofill`).
    /// When the selection's corner falls in a merge, the handle anchors on the
    /// block's extent corner so it hugs the whole block. `None` when that corner
    /// cell is scrolled out of view.
    pub fn fill_handle_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let (_, _, r2, c2) = self.expand_to_merges(self.selection.bounds());
        let (x, y, w, h) = self.cell_pixel_rect(r2, c2)?;
        let s = FILL_HANDLE_SIZE;
        Some((x + w - s / 2.0, y + h - s / 2.0, s, s))
    }

    /// True if `(x, y)` (device px) hits the fill handle, with a small tolerance
    /// margin so the ~7px square is easy to grab.
    pub fn fill_handle_at(&self, x: f64, y: f64) -> bool {
        self.fill_handle_rect().is_some_and(|(hx, hy, w, h)| {
            let m = 2.0;
            x >= hx - m && x <= hx + w + m && y >= hy - m && y <= hy + h + m
        })
    }

    /// Excel-style "current region" around the active cell (`trame-shared-filter`):
    /// the contiguous rectangle of non-empty cells. It grows one line at a time
    /// until the bordering row/column is fully empty. Resolved from the viewport
    /// `cell_cache` (cells beyond the cache read as empty -- acceptable for the
    /// always-on autofilter affordance, which only draws on visible cells). Returns
    /// absolute `(r1, c1, r2, c2)`; `None` when the active cell is an isolated
    /// empty cell (no block to filter).
    pub fn current_region(&self) -> Option<(u32, u32, u32, u32)> {
        let non_empty = |r: u32, c: u32| {
            self.cell_cache
                .get(&(r, c))
                .is_some_and(|d| !d.text.is_empty())
        };
        let (ar, ac) = (self.active_row(), self.active_col());
        let (mut r1, mut r2, mut c1, mut c2) = (ar, ar, ac, ac);
        loop {
            let mut grew = false;
            if r1 > 0 && (c1..=c2).any(|c| non_empty(r1 - 1, c)) {
                r1 -= 1;
                grew = true;
            }
            if (c1..=c2).any(|c| non_empty(r2 + 1, c)) {
                r2 += 1;
                grew = true;
            }
            if c1 > 0 && (r1..=r2).any(|r| non_empty(r, c1 - 1)) {
                c1 -= 1;
                grew = true;
            }
            if (r1..=r2).any(|r| non_empty(r, c2 + 1)) {
                c2 += 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        if r1 == r2 && c1 == c2 && !non_empty(ar, ac) {
            return None;
        }
        Some((r1, c1, r2, c2))
    }

    /// Screen pixel rect (x, y, w, h) of the autofilter button for column `col` on
    /// the current region's header row `band_row` (`trame-shared-filter`). It is a
    /// small square hugging the right edge of cell `(band_row, col)`. `None` when
    /// that cell is hidden or scrolled out of view.
    pub fn filter_button_rect(&self, band_row: u32, col: u32) -> Option<(f64, f64, f64, f64)> {
        let (x, y, w, h) = self.cell_pixel_rect(band_row, col)?;
        // Scale the button with the grid zoom so it tracks the cell (which is
        // already zoom-scaled via `cell_pixel_rect`); clamp to the cell height.
        let s = (FILTER_BUTTON_SIZE * self.zoom).min(h - 2.0).max(0.0);
        let bx = x + w - s - 1.0;
        let by = y + (h - s) / 2.0;
        Some((bx, by, s, s))
    }

    /// Hit-test the autofilter button row: returns `(band_row, col)` of the
    /// button under `(x, y)` (device px), with a small tolerance margin. `None`
    /// when no button is hit (or no current region exists).
    pub fn filter_button_at(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let (r1, c1, _r2, c2) = self.current_region()?;
        (c1..=c2).find_map(|col| {
            self.filter_button_rect(r1, col).and_then(|(bx, by, w, h)| {
                let m = 2.0;
                (x >= bx - m && x <= bx + w + m && y >= by - m && y <= by + h + m)
                    .then_some((r1, col))
            })
        })
    }
}
