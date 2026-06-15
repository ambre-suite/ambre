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

// Formula bar band pinned to the top of the canvas, above the column-header
// strip (spec sec.6). It shows the active cell reference and its raw value (or
// the live edit buffer). Reserved from the top, mirroring how the sheet-tab
// strip is reserved from the bottom: the column headers begin at
// `FORMULA_BAR_HEIGHT`, the cells one header-height further down.
pub const FORMULA_BAR_HEIGHT: f64 = 24.0;
/// Width of the cell-reference ("name") box at the left of the formula bar.
pub const NAME_BOX_WIDTH: f64 = 64.0;

// Sheet-tab strip pinned to the bottom of the canvas (spec sec.13). The grid
// area stops at `canvas_height - TAB_STRIP_HEIGHT`; the strip hosts the tabs and
// the trailing "+" button.
pub const TAB_STRIP_HEIGHT: f64 = 26.0;
/// Fixed tab slot width (label is truncated to fit). Uniform width keeps tab
/// hit-testing pure and font-independent, mirroring the uniform grid metrics.
pub const TAB_WIDTH: f64 = 104.0;
/// Horizontal gap between adjacent tab slots.
pub const TAB_GAP: f64 = 2.0;
/// Width of the trailing "+" (new sheet) button.
pub const ADD_BTN_WIDTH: f64 = 26.0;

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

/// A deferred pointer hit inside the in-cell editor. Resolving a click x to a
/// byte offset needs text measurement (font metrics), which only the renderer
/// has, so the view records the raw physical pointer x plus the intent here and
/// the renderer resolves it into `caret` / `mark` on the next paint.
#[derive(Clone, Copy)]
pub struct PendingHit {
    /// Physical pointer x (same space as `cell_pixel_rect`).
    pub x: f64,
    pub kind: HitKind,
}

/// What a resolved pointer hit does to the editor selection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    /// Place the caret and collapse the selection (single click / drag start).
    Place,
    /// Extend the selection to the hit (drag move / shift-click).
    Extend,
    /// Select the word under the hit (double click).
    Word,
}

/// In-progress edit of a single cell, driven by the transparent textarea pipe.
///
/// The `buffer` is the committed plaintext; `caret` is a byte offset into it
/// (always on a char boundary). `preview` is the live IME composition string,
/// conceptually inserted at the caret but kept separate so it can be styled and
/// discarded/replaced atomically on each composition pass.
///
/// `mark` is the fixed end of a text selection (the moving end is `caret`);
/// `None` means no selection (a bare caret). The selected range is
/// `min(mark, caret)..max(mark, caret)`.
#[derive(Clone, Default)]
pub struct CellEditor {
    pub row: u32,
    pub col: u32,
    pub buffer: String,
    pub caret: usize,
    pub preview: String,
    pub mark: Option<usize>,
    /// Pending pointer hit awaiting resolution by the renderer (then cleared).
    pub pending: Option<PendingHit>,
}

impl CellEditor {
    /// Open an editor on `(row, col)` seeded with `text`, caret at the end.
    pub fn new(row: u32, col: u32, text: String) -> Self {
        let caret = text.len();
        Self { row, col, buffer: text, caret, preview: String::new(), mark: None, pending: None }
    }

    /// The selected byte range `start..end` (start <= end), or `None` when the
    /// selection is empty (no mark, or mark collapsed onto the caret).
    pub fn selection(&self) -> Option<(usize, usize)> {
        match self.mark {
            Some(m) if m != self.caret => Some((m.min(self.caret), m.max(self.caret))),
            _ => None,
        }
    }

    /// The currently selected substring, or empty when nothing is selected.
    pub fn selected_text(&self) -> &str {
        match self.selection() {
            Some((s, e)) => &self.buffer[s..e],
            None => "",
        }
    }

    /// Drop the active selection (keeps the caret where it is).
    pub fn clear_selection(&mut self) {
        self.mark = None;
    }

    /// Delete the selected range, collapsing the caret to its start. Returns
    /// `true` when something was removed.
    pub fn delete_selection(&mut self) -> bool {
        let Some((s, e)) = self.selection() else { return false };
        self.buffer.drain(s..e);
        self.caret = s;
        self.mark = None;
        true
    }

    /// Insert committed text at the caret (latin typing, paste, IME commit),
    /// replacing the selection first if there is one.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        self.buffer.insert_str(self.caret, s);
        self.caret += s.len();
    }

    /// Delete the char before the caret, or the selection when there is one.
    /// No-op at the start with no selection.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
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

    /// Delete the selection, or the char after the caret when none (Del key).
    /// Returns `true` when something was removed.
    pub fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if let Some(c) = self.buffer[self.caret..].chars().next() {
            self.buffer.drain(self.caret..self.caret + c.len_utf8());
            return true;
        }
        false
    }

    /// Byte offset one char left of the caret (or the caret at the start).
    fn prev_offset(&self) -> usize {
        self.buffer[..self.caret]
            .chars()
            .next_back()
            .map(|c| self.caret - c.len_utf8())
            .unwrap_or(self.caret)
    }

    /// Byte offset one char right of the caret (or the caret at the end).
    fn next_offset(&self) -> usize {
        self.buffer[self.caret..]
            .chars()
            .next()
            .map(|c| self.caret + c.len_utf8())
            .unwrap_or(self.caret)
    }

    /// Move the caret one char left. Collapses an existing selection to its
    /// left edge instead of moving past it (standard text-field behaviour).
    pub fn move_left(&mut self) {
        if let Some((s, _)) = self.selection() {
            self.caret = s;
        } else {
            self.caret = self.prev_offset();
        }
        self.mark = None;
    }

    /// Move the caret one char right, collapsing a selection to its right edge.
    pub fn move_right(&mut self) {
        if let Some((_, e)) = self.selection() {
            self.caret = e;
        } else {
            self.caret = self.next_offset();
        }
        self.mark = None;
    }

    /// Extend the selection one char left (Shift+Left).
    pub fn extend_left(&mut self) {
        self.mark.get_or_insert(self.caret);
        self.caret = self.prev_offset();
    }

    /// Extend the selection one char right (Shift+Right).
    pub fn extend_right(&mut self) {
        self.mark.get_or_insert(self.caret);
        self.caret = self.next_offset();
    }

    /// Move the caret to the start of the buffer (Home), dropping the selection.
    pub fn move_home(&mut self) {
        self.caret = 0;
        self.mark = None;
    }

    /// Move the caret to the end of the buffer (End), dropping the selection.
    pub fn move_end(&mut self) {
        self.caret = self.buffer.len();
        self.mark = None;
    }

    /// Extend the selection to the start of the buffer (Shift+Home).
    pub fn extend_home(&mut self) {
        self.mark.get_or_insert(self.caret);
        self.caret = 0;
    }

    /// Extend the selection to the end of the buffer (Shift+End).
    pub fn extend_end(&mut self) {
        self.mark.get_or_insert(self.caret);
        self.caret = self.buffer.len();
    }

    /// Select the whole buffer (Ctrl+A).
    pub fn select_all(&mut self) {
        self.mark = Some(0);
        self.caret = self.buffer.len();
    }

    /// Place the caret at byte offset `off` (clamped to a char boundary) and
    /// collapse the selection. Used to resolve a single-click hit.
    pub fn place_caret(&mut self, off: usize) {
        self.caret = self.clamp_boundary(off);
        self.mark = None;
    }

    /// Extend the selection to byte offset `off` (drag / shift-click hit),
    /// seeding the mark at the current caret if there is no selection yet.
    pub fn extend_to(&mut self, off: usize) {
        self.mark.get_or_insert(self.caret);
        self.caret = self.clamp_boundary(off);
    }

    /// Select the word surrounding byte offset `off` (double-click hit). Words
    /// are maximal runs of alphanumeric chars; a hit on a non-word char selects
    /// that single char.
    pub fn select_word(&mut self, off: usize) {
        let off = self.clamp_boundary(off);
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        // Anchor on the char to the right of the hit, falling back to the left.
        let here = self.buffer[off..].chars().next().or_else(|| self.buffer[..off].chars().next_back());
        let Some(here) = here else {
            self.place_caret(off);
            return;
        };
        let word = is_word(here);
        let start = self.buffer[..off]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c) == word)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(off);
        let end = self.buffer[off..]
            .char_indices()
            .take_while(|(_, c)| is_word(*c) == word)
            .last()
            .map(|(i, c)| off + i + c.len_utf8())
            .unwrap_or(off);
        self.mark = Some(start);
        self.caret = end;
    }

    /// Snap a byte offset down to the nearest char boundary within the buffer.
    fn clamp_boundary(&self, off: usize) -> usize {
        let off = off.min(self.buffer.len());
        if self.buffer.is_char_boundary(off) {
            off
        } else {
            self.buffer[..off].char_indices().next_back().map(|(i, _)| i).unwrap_or(0)
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

// -----------------------------------------------------------------------------
// Selection
// -----------------------------------------------------------------------------

/// The grid selection (spec sec.8). A single-cell selection has `anchor ==
/// active`; a multi-cell range (Phase 2) keeps `anchor` fixed and moves `active`.
///
/// `active` is the cell the cursor, the inline editor and the formatting
/// commands target; `anchor` is the opposite corner of the range. Selection is
/// local view state -- never synced (sec.16).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    /// Fixed corner of the range (where the selection started).
    pub anchor: (u32, u32),
    /// Active cell; the cursor / editor / formatting target.
    pub active: (u32, u32),
}

impl Default for Selection {
    fn default() -> Self {
        Self::single(0, 0)
    }
}

impl Selection {
    /// A selection collapsed to a single cell (`anchor == active`).
    pub fn single(row: u32, col: u32) -> Self {
        Self { anchor: (row, col), active: (row, col) }
    }

    /// Move the active corner, keeping the anchor (Phase 2 range extension).
    #[allow(dead_code)]
    pub fn extend_to(&mut self, row: u32, col: u32) {
        self.active = (row, col);
    }

    /// The active cell (row, col).
    pub fn active(&self) -> (u32, u32) {
        self.active
    }

    /// True if `col` is within the selected column span.
    #[allow(dead_code)]
    pub fn contains_col(&self, col: u32) -> bool {
        let (c1, c2) = (self.anchor.1.min(self.active.1), self.anchor.1.max(self.active.1));
        col >= c1 && col <= c2
    }

    /// True if `row` is within the selected row span.
    #[allow(dead_code)]
    pub fn contains_row(&self, row: u32) -> bool {
        let (r1, r2) = (self.anchor.0.min(self.active.0), self.anchor.0.max(self.active.0));
        row >= r1 && row <= r2
    }
}

// -----------------------------------------------------------------------------
// Sheet tabs (spec sec.13)
// -----------------------------------------------------------------------------

/// An in-progress tab rename, seeded with the sheet's current name. Reuses the
/// same caret/insert/backspace machinery as the cell editor; `sheet_id` pins the
/// target so a concurrent reorder cannot misdirect the commit.
#[derive(Clone)]
pub struct TabRename {
    pub sheet_id: String,
    pub editor: CellEditor,
}

/// An open per-tab context menu (`{ Rename, Delete }`, spec sec.13), anchored by
/// its top-left corner in physical pixels.
#[derive(Clone)]
pub struct TabMenu {
    pub sheet_id: String,
    pub x: f64,
    pub y: f64,
}

/// The two entries of the tab context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabMenuItem {
    Rename,
    Delete,
}

/// Menu geometry (physical px). Two stacked items.
pub const TAB_MENU_WIDTH: f64 = 120.0;
pub const TAB_MENU_ITEM_HEIGHT: f64 = 24.0;

impl TabMenu {
    /// Rect of menu item `i` (0 = Rename, 1 = Delete).
    pub fn item_rect(&self, i: usize) -> (f64, f64, f64, f64) {
        let y = self.y + i as f64 * TAB_MENU_ITEM_HEIGHT;
        (self.x, y, TAB_MENU_WIDTH, TAB_MENU_ITEM_HEIGHT)
    }

    /// The menu item at physical pixel `(x, y)`, or `None` if outside the menu.
    pub fn item_at(&self, x: f64, y: f64) -> Option<TabMenuItem> {
        if x < self.x || x >= self.x + TAB_MENU_WIDTH || y < self.y {
            return None;
        }
        match ((y - self.y) / TAB_MENU_ITEM_HEIGHT) as usize {
            0 => Some(TabMenuItem::Rename),
            1 => Some(TabMenuItem::Delete),
            _ => None,
        }
    }
}

/// An in-progress tab drag-reorder. Tracks the grabbed sheet and the live
/// pointer x so the renderer can show the dragged tab follow the cursor; `moved`
/// gates whether the release commits a `MoveSheet` (vs a plain activate-click).
#[derive(Clone)]
pub struct TabDrag {
    pub sheet_id: String,
    pub grab_x: f64,
    pub cur_x: f64,
    pub moved: bool,
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

    /// Current selection. Single-cell for now (`anchor == active`); the active
    /// cell drives the cursor, inline editor and formatting commands (sec.8).
    pub selection: Selection,

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

    /// The workbook's sheets in tab-strip order: `(sheet_id, display_name)`.
    /// Mirrors `TrameEvent::SheetList`; empty until the first one arrives.
    pub sheets: Vec<(String, String)>,
    /// Id of the sheet currently shown (the highlighted tab). Empty until the
    /// first `SheetList`.
    pub active_sheet: String,

    /// Monotonically increasing version counter for outgoing `ScrollViewport` commands.
    next_version: u64,
}

impl TrameUiState {
    pub fn new(canvas_width: f64, canvas_height: f64) -> Self {
        let mut s = Self {
            top_row: 0,
            left_col: 0,
            selection: Selection::single(0, 0),
            viewport_rows: 0,
            viewport_cols: 0,
            canvas_width,
            canvas_height,
            cell_cache: HashMap::new(),
            dirty_cells: HashSet::new(),
            formula_errors: HashMap::new(),
            accepted_version: 0,
            sheets: Vec::new(),
            active_sheet: String::new(),
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

    /// Bottom edge (physical px) of the scrollable grid area: the canvas height
    /// minus the sheet-tab strip pinned along the bottom (sec.13).
    pub fn grid_bottom(&self) -> f64 {
        (self.canvas_height - TAB_STRIP_HEIGHT).max(HEADER_HEIGHT)
    }

    fn recompute_viewport_dims(&mut self) {
        let grid_h = (self.grid_bottom() - FORMULA_BAR_HEIGHT - HEADER_HEIGHT).max(0.0);
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
    // Selection
    // -------------------------------------------------------------------------

    /// The active cell row (the selection's active corner).
    pub fn active_row(&self) -> u32 {
        self.selection.active.0
    }

    /// The active cell column (the selection's active corner).
    pub fn active_col(&self) -> u32 {
        self.selection.active.1
    }

    /// Collapse the selection to a single cell (single-cell selection, sec.8).
    /// Used on a pointer hit; no clamping -- callers pass valid grid coords.
    pub fn select(&mut self, row: u32, col: u32) {
        self.selection = Selection::single(row, col);
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
        let new_row = (self.active_row() as i64 + dr as i64)
            .clamp(0, (TOTAL_ROWS - 1) as i64) as u32;
        let new_col = (self.active_col() as i64 + dc as i64)
            .clamp(0, (TOTAL_COLS - 1) as i64) as u32;
        self.move_to(new_row, new_col)
    }

    /// Move the active cell to absolute `(row, col)`, clamped to the grid.
    ///
    /// Backs the jump-style navigation keys (Home / End / Ctrl+Home / Ctrl+End,
    /// Page Up / Page Down compute their target row first). Same contract as
    /// [`move_cursor`](Self::move_cursor): `None` if the cell did not change,
    /// otherwise a [`CursorMove`] with the new active cell plus an optional
    /// `ScrollViewport`.
    pub fn move_to(&mut self, row: u32, col: u32) -> Option<CursorMove> {
        let new_row = row.min(TOTAL_ROWS - 1);
        let new_col = col.min(TOTAL_COLS - 1);

        if new_row == self.active_row() && new_col == self.active_col() {
            return None;
        }
        // Keyboard navigation collapses the selection onto the new cell.
        self.selection = Selection::single(new_row, new_col);

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

    /// Rightmost cached non-empty column in `row`, or `None` if the row has no
    /// cached content. Backs the End key: the UI thread has no used-range model,
    /// so the local cell cache is the only view of content available (sec.16),
    /// making this a best-effort jump to the end of the visible data.
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
        // Prefetch one full screen of rows above and below the viewport (with a
        // PADDING_ROWS floor for tiny windows). A Page Up/Down jump moves the
        // viewport by exactly viewport_rows, so a smaller fixed margin would land
        // beyond the cache and flash the entering rows grey until the Logic thread
        // responds. Sizing the margin to the viewport keeps a page jump's
        // destination within already-fetched data, eliminating that flash.
        let row_pad = self.viewport_rows.max(PADDING_ROWS);
        let row_start = self.top_row.saturating_sub(row_pad);
        let row_end = (self.top_row + self.viewport_rows + row_pad)
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
    // Sheet tabs (spec sec.13)
    // -------------------------------------------------------------------------

    /// Replace the tab strip from a `TrameEvent::SheetList`.
    pub fn on_sheet_list(&mut self, sheets: Vec<(String, String)>, active: String) {
        self.sheets = sheets;
        self.active_sheet = active;
    }

    /// True if physical `y` falls within the bottom tab strip.
    pub fn in_tab_strip(&self, y: f64) -> bool {
        y >= self.grid_bottom() && y < self.canvas_height
    }

    /// Pixel x-range `(x0, x1)` of tab slot `index` (no horizontal tab scroll in
    /// the MVP).
    pub fn tab_slot_x(&self, index: usize) -> (f64, f64) {
        let x0 = index as f64 * (TAB_WIDTH + TAB_GAP);
        (x0, x0 + TAB_WIDTH)
    }

    /// Pixel x-range of the trailing "+" button, just past the last tab.
    pub fn add_button_x(&self) -> (f64, f64) {
        let x0 = self.sheets.len() as f64 * (TAB_WIDTH + TAB_GAP);
        (x0, x0 + ADD_BTN_WIDTH)
    }

    /// Index of the tab at physical pixel `(x, y)`, or `None` if `(x, y)` is not
    /// over a tab (header strip required; "+" button excluded).
    pub fn tab_at(&self, x: f64, y: f64) -> Option<usize> {
        if !self.in_tab_strip(y) {
            return None;
        }
        (0..self.sheets.len()).find(|&i| {
            let (x0, x1) = self.tab_slot_x(i);
            x >= x0 && x < x1
        })
    }

    /// True if `(x, y)` hits the trailing "+" button.
    pub fn add_button_at(&self, x: f64, y: f64) -> bool {
        if !self.in_tab_strip(y) {
            return false;
        }
        let (x0, x1) = self.add_button_x();
        x >= x0 && x < x1
    }

    /// The drop index a tab dragged to physical x `cur_x` would land at, clamped
    /// to the strip. Slots are uniform, so the target is the slot the tab's left
    /// edge currently overlaps.
    pub fn drop_index_at(&self, cur_x: f64) -> u32 {
        let slot = (cur_x / (TAB_WIDTH + TAB_GAP)).floor().max(0.0) as u32;
        slot.min(self.sheets.len().saturating_sub(1) as u32)
    }

    // -------------------------------------------------------------------------
    // Coordinate helpers
    // -------------------------------------------------------------------------

    /// True if physical `(x, y)` falls in the formula bar's content area (the
    /// band right of the name box). A click there opens an edit on the active
    /// cell (spec sec.6); the name box itself is reserved (future go-to).
    pub fn in_formula_bar(&self, x: f64, y: f64) -> bool {
        (0.0..FORMULA_BAR_HEIGHT).contains(&y) && x >= NAME_BOX_WIDTH && x < self.canvas_width
    }

    /// Cell `(row, col)` at physical pixel `(x, y)`, or `None` if the point is
    /// in a header strip or past the grid bounds. Inverse of `cell_pixel_rect`.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let cells_top = FORMULA_BAR_HEIGHT + HEADER_HEIGHT;
        if x < HEADER_WIDTH || y < cells_top {
            return None;
        }
        if x >= self.canvas_width || y >= self.grid_bottom() {
            return None;
        }
        let col = self.left_col + ((x - HEADER_WIDTH) / COL_WIDTH) as u32;
        let row = self.top_row + ((y - cells_top) / ROW_HEIGHT) as u32;
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

        let cells_top = FORMULA_BAR_HEIGHT + HEADER_HEIGHT;
        let x = HEADER_WIDTH + col_offset as f64 * COL_WIDTH;
        let y = cells_top + row_offset as f64 * ROW_HEIGHT;

        if x + COL_WIDTH <= HEADER_WIDTH || x >= self.canvas_width {
            return None;
        }
        if y + ROW_HEIGHT <= cells_top || y >= self.grid_bottom() {
            return None;
        }
        Some((x, y, COL_WIDTH, ROW_HEIGHT))
    }
}

#[cfg(test)]
mod tests {
    use super::{CellEditor, Selection, TabMenu, TabMenuItem, TrameUiState};
    use crate::{DisplayCell, TrameEvent};

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
        state.move_cursor(1, 0).expect("cursor moved");
        assert_eq!(state.selection, Selection::single(6, 3));
        assert_eq!(state.selection.anchor, state.selection.active);
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
        assert!(state.move_to(0, 0).is_none());
        // Clamped to the last valid index.
        state.move_to(crate::TOTAL_ROWS + 10, crate::TOTAL_COLS + 10).expect("moved");
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
            cells: vec![
                DisplayCell { row: 2, col: 5, text: "x".into(), raw: "x".into(),
                    bold: false, italic: false, is_merged: false, merge_span: None, is_filtered: false },
                DisplayCell { row: 2, col: 1, text: "y".into(), raw: "y".into(),
                    bold: false, italic: false, is_merged: false, merge_span: None, is_filtered: false },
                DisplayCell { row: 4, col: 3, text: "".into(), raw: "".into(),
                    bold: false, italic: false, is_merged: false, merge_span: None, is_filtered: false },
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

    /// The tab strip is reserved from the bottom of the canvas: the grid area
    /// stops above it, and a point inside the strip is not a grid cell.
    #[test]
    fn tab_strip_reserves_bottom_of_canvas() {
        let state = TrameUiState::new(800.0, 600.0);
        assert_eq!(state.grid_bottom(), 600.0 - super::TAB_STRIP_HEIGHT);
        // A click in the strip is not over any cell.
        assert_eq!(state.cell_at(100.0, 590.0), None);
        assert!(state.in_tab_strip(590.0));
        assert!(!state.in_tab_strip(100.0));
    }

    /// `on_sheet_list` mirrors the event, and tab/"+"/drop hit-testing maps pixel
    /// x to the right slot.
    #[test]
    fn tab_hit_testing_maps_pixels_to_slots() {
        let mut state = TrameUiState::new(800.0, 600.0);
        state.on_sheet_list(
            vec![("s0".into(), "One".into()), ("s1".into(), "Two".into())],
            "s0".into(),
        );
        assert_eq!(state.active_sheet, "s0");

        let y = state.grid_bottom() + 5.0;
        // First slot centre -> tab 0; second slot centre -> tab 1.
        let (x0, x1) = state.tab_slot_x(0);
        assert_eq!(state.tab_at((x0 + x1) / 2.0, y), Some(0));
        let (x0, _) = state.tab_slot_x(1);
        assert_eq!(state.tab_at(x0 + 1.0, y), Some(1));

        // The "+" button sits just past the last tab.
        let (ax0, _) = state.add_button_x();
        assert!(state.add_button_at(ax0 + 1.0, y));
        assert!(!state.add_button_at(ax0 + 1.0, 50.0)); // not in the strip

        // A drag to the first slot drops at index 0; far right clamps to last.
        assert_eq!(state.drop_index_at(1.0), 0);
        assert_eq!(state.drop_index_at(5000.0), 1);
    }

    /// Hit-testing rejects points outside any tab: the gap past the last tab but
    /// before "+", a y above the strip, and a far-right x are not tabs. The "+"
    /// button tracks the sheet count, and `on_sheet_list` fully replaces state.
    #[test]
    fn tab_hit_testing_rejects_non_tab_points() {
        let mut state = TrameUiState::new(800.0, 600.0);
        // No sheets yet: nothing is a tab, and "+" sits at the origin.
        let y = state.grid_bottom() + 5.0;
        assert_eq!(state.tab_at(10.0, y), None);
        assert_eq!(state.add_button_x().0, 0.0);
        assert!(state.add_button_at(2.0, y));

        state.on_sheet_list(
            vec![("s0".into(), "A".into()), ("s1".into(), "B".into()), ("s2".into(), "C".into())],
            "s1".into(),
        );
        // "+" now follows the third tab; the slot before it is tab 2, not "+".
        let (ax0, _) = state.add_button_x();
        assert_eq!(ax0, 3.0 * (super::TAB_WIDTH + super::TAB_GAP));
        assert_eq!(state.tab_at(state.tab_slot_x(2).0 + 1.0, y), Some(2));
        assert!(!state.add_button_at(state.tab_slot_x(2).0 + 1.0, y));

        // A point above the strip is never a tab, even at a tab's x.
        assert_eq!(state.tab_at(state.tab_slot_x(0).0 + 1.0, 10.0), None);
        // The inter-tab gap (the 2px between slots) is not a tab.
        assert_eq!(state.tab_at(state.tab_slot_x(0).1 + 0.5, y), None);

        // A fresh list shrinks the strip and re-points the active tab.
        state.on_sheet_list(vec![("only".into(), "Only".into())], "only".into());
        assert_eq!(state.sheets.len(), 1);
        assert_eq!(state.active_sheet, "only");
        assert_eq!(state.tab_at(state.tab_slot_x(1).0 + 1.0, y), None);
        // A single sheet: every drag clamps to index 0.
        assert_eq!(state.drop_index_at(9999.0), 0);
    }

    /// The tab context menu maps a click to Rename / Delete by row, and rejects
    /// clicks outside its box (left/right of the width, above/below the items).
    #[test]
    fn tab_menu_item_hit_testing() {
        let menu = TabMenu { sheet_id: "s0".into(), x: 100.0, y: 200.0 };
        // First row -> Rename, second -> Delete.
        assert_eq!(menu.item_at(110.0, 200.0 + 1.0), Some(TabMenuItem::Rename));
        assert_eq!(
            menu.item_at(110.0, 200.0 + super::TAB_MENU_ITEM_HEIGHT + 1.0),
            Some(TabMenuItem::Delete),
        );
        // Outside the width / above / below the two items.
        assert_eq!(menu.item_at(99.0, 210.0), None);
        assert_eq!(menu.item_at(100.0 + super::TAB_MENU_WIDTH + 1.0, 210.0), None);
        assert_eq!(menu.item_at(110.0, 199.0), None);
        assert_eq!(menu.item_at(110.0, 200.0 + 2.0 * super::TAB_MENU_ITEM_HEIGHT + 1.0), None);
    }

    /// The formula bar band reserves the top of the canvas: a point in its
    /// content area (right of the name box) is not a grid cell, and the cell
    /// area is pushed below both the band and the column-header strip.
    #[test]
    fn formula_bar_reserves_top_of_canvas() {
        let state = TrameUiState::new(800.0, 600.0);
        let bar = super::FORMULA_BAR_HEIGHT;
        let name_box = super::NAME_BOX_WIDTH;

        // A click in the bar's content area is a formula bar hit, not a cell.
        assert!(state.in_formula_bar(name_box + 10.0, bar / 2.0));
        assert_eq!(state.cell_at(name_box + 10.0, bar / 2.0), None);
        // The name box itself is reserved (not a formula bar content hit).
        assert!(!state.in_formula_bar(name_box / 2.0, bar / 2.0));
        // Below the band, but still in the header strip: not a cell either.
        assert_eq!(state.cell_at(100.0, bar + super::HEADER_HEIGHT - 1.0), None);
        // The first cell starts just below the band + header strip.
        assert_eq!(
            state.cell_at(super::HEADER_WIDTH + 1.0, bar + super::HEADER_HEIGHT + 1.0),
            Some((0, 0)),
        );
        // (0,0)'s pixel rect sits at the cell-area top-left.
        let (_, y, _, _) = state.cell_pixel_rect(0, 0).expect("visible");
        assert_eq!(y, bar + super::HEADER_HEIGHT);
    }

    /// On a very short canvas the grid area never inverts: `grid_bottom` floors
    /// at the top header height.
    #[test]
    fn grid_bottom_floors_at_header_on_tiny_canvas() {
        let state = TrameUiState::new(800.0, 10.0);
        assert_eq!(state.grid_bottom(), super::HEADER_HEIGHT);
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
