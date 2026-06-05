// ambre-trame/src/ui/grid.rs
// ============================================================================
// Trame virtual-scroll grid state -- UI Thread side.
//
// Tracks the active cell, the scroll position (top-left of the visible area),
// and a local cache of received DisplayCells. The cache is keyed by (row, col)
// and grows as the Logic thread responds to ScrollViewport requests.
//
// Arrow key presses call `move_cursor()`, which returns a `TrameCommand` to
// send to the Logic thread whenever the visible range changes.
// ============================================================================

use std::collections::HashMap;

use crate::{TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

// Cell layout.
pub const ROW_HEIGHT: f64 = 20.0;
pub const COL_WIDTH: f64 = 80.0;

// Fixed-size header strips (column labels top, row numbers left).
pub const HEADER_HEIGHT: f64 = 24.0;
pub const HEADER_WIDTH: f64 = 50.0;

// Pre-fetch padding around the visible area.
const PADDING_ROWS: u32 = 20;
const PADDING_COLS: u32 = 5;

/// A cached cell ready to render: text plus its pinned formatting runs.
#[derive(Clone, Default)]
pub struct CellData {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// Outcome of a cursor move: the two commands the view forwards to Logic.
pub struct CursorMove {
    /// Always present -- the new active cell, so formatting toggles retarget.
    pub active: TrameCommand,
    /// Present only when the visible range changed and new cells must be fetched.
    pub scroll: Option<TrameCommand>,
}

// -----------------------------------------------------------------------------
// TrameUiState
// -----------------------------------------------------------------------------

/// All mutable UI state for the Trame grid on the UI Thread.
pub struct TrameUiState {
    /// Top-left visible row (0-based).
    pub top_row: u32,
    /// Top-left visible column (0-based).
    pub left_col: u32,

    /// Currently selected cell.
    pub active_row: u32,
    pub active_col: u32,

    /// Number of full rows visible in the grid area (recomputed on resize).
    pub viewport_rows: u32,
    /// Number of full columns visible in the grid area.
    pub viewport_cols: u32,

    /// Canvas physical size (CSS pixels × devicePixelRatio, set by ResizeObserver).
    pub canvas_width: f64,
    pub canvas_height: f64,

    /// Cache of received cells: (row, col) -> display data (text + formatting).
    ///
    /// A cell absent from the cache is shown as "pending" (grey background).
    /// Only cells within the last requested range + padding should be queried;
    /// we never evict entries because the grid is read-only for now.
    pub cell_cache: HashMap<(u32, u32), CellData>,

    /// Version of the last accepted `ViewportData` response (anti-flicker).
    pub accepted_version: u64,

    /// Monotonically increasing version counter for outgoing `ScrollViewport` commands.
    next_version: u64,
}

impl TrameUiState {
    pub fn new(canvas_width: f64, canvas_height: f64) -> Self {
        let mut s = Self {
            top_row: 0,
            left_col: 0,
            active_row: 0,
            active_col: 0,
            viewport_rows: 0,
            viewport_cols: 0,
            canvas_width,
            canvas_height,
            cell_cache: HashMap::new(),
            accepted_version: 0,
            next_version: 1,
        };
        s.recompute_viewport_dims();
        s
    }

    // -------------------------------------------------------------------------
    // Resize
    // -------------------------------------------------------------------------

    /// Update canvas dimensions and recompute the visible range.
    #[allow(dead_code)]
    pub fn resize(&mut self, width: f64, height: f64) {
        self.canvas_width = width;
        self.canvas_height = height;
        self.recompute_viewport_dims();
    }

    fn recompute_viewport_dims(&mut self) {
        let grid_h = (self.canvas_height - HEADER_HEIGHT).max(0.0);
        let grid_w = (self.canvas_width - HEADER_WIDTH).max(0.0);
        // ceil() already accounts for the partially-visible row/col at the edge.
        // No +1 needed: viewport_rows is the count of rows that start within the
        // grid area, whether fully or partially visible. scroll_to_include and
        // cell_pixel_rect must agree on this count, or the active cell can
        // appear "in viewport" to the scroll logic but off-canvas to the renderer.
        self.viewport_rows = (grid_h / ROW_HEIGHT).ceil() as u32;
        self.viewport_cols = (grid_w / COL_WIDTH).ceil() as u32;
    }

    // -------------------------------------------------------------------------
    // Cursor movement
    // -------------------------------------------------------------------------

    /// Move the active cell by `(dr, dc)`.
    ///
    /// Scrolls the viewport if the new position falls outside the visible area.
    /// Returns `None` if the cursor did not move (clamped at an edge); otherwise
    /// a [`CursorMove`] carrying the new active cell (always, so the Logic thread
    /// can retarget formatting) plus an optional `ScrollViewport` to fetch newly
    /// exposed cells.
    pub fn move_cursor(&mut self, dr: i32, dc: i32) -> Option<CursorMove> {
        let new_row = (self.active_row as i64 + dr as i64)
            .clamp(0, (TOTAL_ROWS - 1) as i64) as u32;
        let new_col = (self.active_col as i64 + dc as i64)
            .clamp(0, (TOTAL_COLS - 1) as i64) as u32;

        if new_row == self.active_row && new_col == self.active_col {
            return None;
        }
        self.active_row = new_row;
        self.active_col = new_col;

        // Scroll viewport so the active cell stays visible. When the active cell
        // is still within the cached range the highlight moves without a Logic
        // round-trip, but we still report the new active cell (formatting target).
        let scroll = self
            .scroll_to_include(new_row, new_col)
            .then(|| self.scroll_viewport_cmd());
        Some(CursorMove {
            active: TrameCommand::SetActiveCell { row: new_row, col: new_col },
            scroll,
        })
    }

    /// Adjust top_row / left_col so that (row, col) is visible.
    /// Returns true if the scroll position changed.
    fn scroll_to_include(&mut self, row: u32, col: u32) -> bool {
        let mut changed = false;
        if row < self.top_row {
            self.top_row = row;
            changed = true;
        } else if row >= self.top_row + self.viewport_rows {
            self.top_row = row - self.viewport_rows + 1;
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

    // -------------------------------------------------------------------------
    // IPC
    // -------------------------------------------------------------------------

    /// Build a `ScrollViewport` command for the current visible area plus padding.
    ///
    /// Evicts cached cells that fall outside the new requested range so that
    /// cells scrolled into view from an un-fetched area appear grey until the
    /// Logic thread responds. Cells already within the new range stay white,
    /// avoiding flicker on slow continuous scrolling.
    pub fn scroll_viewport_cmd(&mut self) -> TrameCommand {
        let row_start = self.top_row.saturating_sub(PADDING_ROWS);
        let row_end = (self.top_row + self.viewport_rows + PADDING_ROWS)
            .min(TOTAL_ROWS.saturating_sub(1));
        let col_start = self.left_col.saturating_sub(PADDING_COLS);
        let col_end = (self.left_col + self.viewport_cols + PADDING_COLS)
            .min(TOTAL_COLS.saturating_sub(1));

        // Evict cells that are no longer in the requested window.
        self.cell_cache.retain(|&(row, col), _| {
            row >= row_start && row <= row_end && col >= col_start && col <= col_end
        });

        let version = self.next_version;
        self.next_version += 1;

        TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end }
    }

    /// Merge a `ViewportData` batch into the local cache.
    ///
    /// Older responses (stale version) are silently discarded.
    pub fn on_viewport_data(&mut self, update: TrameEvent) {
        let TrameEvent::ViewportData { version, cells, .. } = update else { return };
        if version < self.accepted_version {
            return;
        }
        self.accepted_version = version;
        for cell in cells {
            self.cell_cache.insert(
                (cell.row, cell.col),
                CellData { text: cell.text, bold: cell.bold, italic: cell.italic },
            );
        }
    }

    /// Apply a single-cell formatting change (from `TrameEvent::CellFormat`).
    ///
    /// Pins the run to the cell regardless of where the cursor moves next. A
    /// no-op if the cell is not cached (it will arrive formatted on its next
    /// `ViewportData`).
    pub fn set_cell_format(&mut self, row: u32, col: u32, bold: bool, italic: bool) {
        if let Some(cell) = self.cell_cache.get_mut(&(row, col)) {
            cell.bold = bold;
            cell.italic = italic;
        }
    }

    // -------------------------------------------------------------------------
    // Coordinate helpers
    // -------------------------------------------------------------------------

    /// Screen pixel rect (x, y, w, h) of cell (row, col) relative to the
    /// top-left corner of the canvas, or `None` if the cell is outside the
    /// visible area.
    pub fn cell_pixel_rect(&self, row: u32, col: u32) -> Option<(f64, f64, f64, f64)> {
        // Signed arithmetic to handle the cell being left/above the viewport.
        let col_offset = col as i64 - self.left_col as i64;
        let row_offset = row as i64 - self.top_row as i64;

        let x = HEADER_WIDTH + col_offset as f64 * COL_WIDTH;
        let y = HEADER_HEIGHT + row_offset as f64 * ROW_HEIGHT;

        if x + COL_WIDTH <= HEADER_WIDTH || x >= self.canvas_width {
            return None;
        }
        if y + ROW_HEIGHT <= HEADER_HEIGHT || y >= self.canvas_height {
            return None;
        }
        Some((x, y, COL_WIDTH, ROW_HEIGHT))
    }
}
