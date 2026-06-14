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

use std::collections::{HashMap, HashSet};

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
    /// Raw cell text (formula source for formula cells, equal to `text`
    /// otherwise). Seeds the in-cell editor so formulas are editable as text.
    pub raw: String,
    pub bold: bool,
    pub italic: bool,
    /// `Some((col_span, row_span))` on a merge master, `None` otherwise. Covered
    /// cells arrive with empty text and are simply rendered blank.
    pub merge_span: Option<(u32, u32)>,
}

/// In-progress edit of a single cell, driven by the transparent textarea pipe.
///
/// The `buffer` is the committed plaintext; `caret` is a byte offset into it
/// (always on a char boundary). `preview` is the live IME composition string,
/// conceptually inserted at the caret but kept separate so it can be styled and
/// discarded/replaced atomically on each composition pass.
#[derive(Clone, Default)]
pub struct CellEditor {
    pub row: u32,
    pub col: u32,
    pub buffer: String,
    pub caret: usize,
    pub preview: String,
}

impl CellEditor {
    /// Open an editor on `(row, col)` seeded with `text`, caret at the end.
    pub fn new(row: u32, col: u32, text: String) -> Self {
        let caret = text.len();
        Self { row, col, buffer: text, caret, preview: String::new() }
    }

    /// Insert committed text at the caret (latin typing, paste, IME commit).
    pub fn insert(&mut self, s: &str) {
        self.buffer.insert_str(self.caret, s);
        self.caret += s.len();
    }

    /// Delete the char before the caret. No-op at the start.
    pub fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let prev = self.buffer[..self.caret]
            .chars()
            .next_back()
            .map(char::len_utf8)
            .unwrap_or(0);
        self.buffer.drain(self.caret - prev..self.caret);
        self.caret -= prev;
    }

    /// Move the caret one char left.
    pub fn move_left(&mut self) {
        if let Some(c) = self.buffer[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
        }
    }

    /// Move the caret one char right.
    pub fn move_right(&mut self) {
        if let Some(c) = self.buffer[self.caret..].chars().next() {
            self.caret += c.len_utf8();
        }
    }

    /// The text shown before the caret (used to position caret + IME popup):
    /// committed prefix followed by the live composition preview.
    pub fn before_caret(&self) -> String {
        let mut s = self.buffer[..self.caret].to_owned();
        s.push_str(&self.preview);
        s
    }

    /// The full visual string: prefix + preview + suffix.
    pub fn display(&self) -> String {
        let mut s = self.buffer[..self.caret].to_owned();
        s.push_str(&self.preview);
        s.push_str(&self.buffer[self.caret..]);
        s
    }
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

    /// Cells edited locally whose authoritative (recomputed) value has not yet
    /// arrived from the Logic thread. Rendered with a reduced-opacity veil so
    /// the optimistic value reads as "not yet confirmed". Cleared per cell when
    /// the next `ViewportData` carries it. UI-only derived state -- never synced.
    pub dirty_cells: HashSet<(u32, u32)>,

    /// Cells whose formula evaluation failed, keyed to the error string (e.g.
    /// `#DIV/0!`). Rendered with a small corner marker. Populated by
    /// `TrameEvent::FormulaError`, cleared when a valid value arrives. UI-only.
    pub formula_errors: HashMap<(u32, u32), String>,

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
            dirty_cells: HashSet::new(),
            formula_errors: HashMap::new(),
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

    /// Pan the viewport by `(dr, dc)` rows/cols *without* moving the active
    /// cell (wheel / touchpad scroll). Clamps the top-left within the grid.
    /// Returns `true` if the scroll position actually changed.
    pub fn scroll_by(&mut self, dr: i32, dc: i32) -> bool {
        let new_top =
            (self.top_row as i64 + dr as i64).clamp(0, (TOTAL_ROWS - 1) as i64) as u32;
        let new_left =
            (self.left_col as i64 + dc as i64).clamp(0, (TOTAL_COLS - 1) as i64) as u32;
        if new_top == self.top_row && new_left == self.left_col {
            return false;
        }
        self.top_row = new_top;
        self.left_col = new_left;
        true
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
                    merge_span: cell.merge_span,
                },
            );
        }
    }

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
        // ViewportData carries the computed value -- never echo the formula
        // source as its own result. For a plain literal, raw == display.
        let is_formula = text.starts_with('=');
        entry.raw = text.clone();
        entry.text = if is_formula { String::new() } else { text };
        // The shown value is now optimistic: a formula awaits recompute, and a
        // literal awaits the CRDT echo. Either way the prior error is stale.
        self.formula_errors.remove(&(row, col));
        self.dirty_cells.insert((row, col));
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

    /// Cell `(row, col)` at physical pixel `(x, y)`, or `None` if the point is
    /// in a header strip or past the grid bounds. Inverse of `cell_pixel_rect`.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        if x < HEADER_WIDTH || y < HEADER_HEIGHT {
            return None;
        }
        if x >= self.canvas_width || y >= self.canvas_height {
            return None;
        }
        let col = self.left_col + ((x - HEADER_WIDTH) / COL_WIDTH) as u32;
        let row = self.top_row + ((y - HEADER_HEIGHT) / ROW_HEIGHT) as u32;
        if row >= TOTAL_ROWS || col >= TOTAL_COLS {
            return None;
        }
        Some((row, col))
    }

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

#[cfg(test)]
mod tests {
    use super::{CellEditor, TrameUiState};
    use crate::{DisplayCell, TrameEvent};

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
        assert_eq!(state.formula_errors.get(&(0, 0)).map(String::as_str), Some("#REF!"));

        state.on_viewport_data(TrameEvent::ViewportData {
            version: 1,
            row_start: 0,
            col_start: 0,
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
            }],
        });
        assert!(!state.formula_errors.contains_key(&(0, 0)), "valid value clears error");
        assert!(!state.dirty_cells.contains(&(0, 0)), "valid value clears dirty");
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
}
