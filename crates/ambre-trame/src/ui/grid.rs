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

// Header interaction metrics (`-ui.md` sec.4bis).
/// Lower clamp on a column/row resize (physical px); mirrors the Logic-side
/// `MIN_COL_WIDTH` / `MIN_ROW_HEIGHT` so the UI preview and the persisted value
/// agree.
pub const MIN_COL_WIDTH: f64 = 24.0;
pub const MIN_ROW_HEIGHT: f64 = 12.0;
/// Half-width (physical px) of the grab band straddling a header boundary that
/// arms a resize drag (sec.4bis.C). A press within +/- this of a column's right
/// edge (resp. a row's bottom edge) resizes instead of selecting.
pub const RESIZE_BAND: f64 = 4.0;

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
    /// Rich manual formatting (`-ui.md` sec.5bis): underline run, alignments,
    /// wrap, text/fill colours, font family/size, border preset. Pinned to the
    /// cell, fed by [`DisplayCell`](crate::DisplayCell).
    pub underline: bool,
    pub strikethrough: bool,
    pub h_align: ambre_bus::HAlign,
    pub v_align: ambre_bus::VAlign,
    pub wrap: bool,
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub font: Option<String>,
    pub size: Option<u16>,
    pub borders: ambre_bus::Borders,
    /// `Some((col_span, row_span))` on a merge master, `None` otherwise. Covered
    /// cells arrive with empty text and are simply rendered blank.
    pub merge_span: Option<(u32, u32)>,
    /// The cell's row is hidden by the active shared filter (`-ui.md` sec.5.D):
    /// the row axis collapses over it. Drives [`TrameUiState::hidden_rows`].
    pub is_filtered: bool,
}

impl CellData {
    /// Build the renderer's [`CellContent`](crate::render::CellContent) view of
    /// this cached cell, with `text` supplied by the caller (a merge master draws
    /// empty body text in the single-cell pass, its real text in the merge pass).
    pub fn content<'a>(&'a self, text: &'a str) -> crate::render::CellContent<'a> {
        crate::render::CellContent {
            text,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
            h_align: self.h_align,
            v_align: self.v_align,
            fg: self.fg,
            bg: self.bg,
            font: self.font.as_deref(),
            size: self.size,
            wrap: self.wrap,
            borders: self.borders,
        }
    }
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
        Self {
            row,
            col,
            buffer: text,
            caret,
            preview: String::new(),
            mark: None,
            pending: None,
        }
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
        let Some((s, e)) = self.selection() else {
            return false;
        };
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
        let here = self.buffer[off..]
            .chars()
            .next()
            .or_else(|| self.buffer[..off].chars().next_back());
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
            self.buffer[..off]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
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

/// A normalized cell rectangle `(r1, c1, r2, c2)` with `r1 <= r2` and
/// `c1 <= c2`. Used for the committed disjoint rectangles of a multi-range
/// selection and as the return type of [`Selection::bounds`].
pub type CellRect = (u32, u32, u32, u32);

/// The grid selection (spec sec.8). A single-cell selection has `anchor ==
/// active` and no extra ranges. A contiguous range keeps `anchor` fixed and
/// moves `active` (drag, Shift+click, Shift+arrows). Disjoint ranges
/// (Ctrl+click) push the live rectangle into `extra` and start a fresh one.
///
/// `active` is the cell the cursor, the inline editor and the formatting
/// commands target; `anchor` is the opposite corner of the live rectangle.
/// Selection is local view state -- never synced (sec.16).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Selection {
    /// Fixed corner of the live rectangle (where this range started).
    pub anchor: (u32, u32),
    /// Active cell; the cursor / editor / formatting target.
    pub active: (u32, u32),
    /// Committed disjoint rectangles (Ctrl+click), each normalized. The live
    /// rectangle (`anchor..=active`) is *not* in here; the full selection is
    /// `extra` plus that rectangle (see [`rects`](Self::rects)).
    pub extra: Vec<CellRect>,
    /// Whether the live selection spans whole cells, columns, or rows
    /// (`-ui.md` sec.4bis.A). For `Cols`/`Rows` the perpendicular coordinate of
    /// `anchor`/`active` is ignored (the band runs the full axis); the Logic
    /// thread resolves the band to the used-range rectangle. UI-only view state.
    pub kind: SelKind,
}

/// Shape of the live selection (`-ui.md` sec.4bis.A). `Cells` is the ordinary
/// rectangle; `Cols`/`Rows` are virtual whole-axis bands selected from a header.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelKind {
    #[default]
    Cells,
    Cols,
    Rows,
}

/// Which surface a right-click landed on, choosing the context-menu layout
/// (`-ui.md` sec.5.C). A header right-click drops the cell-only entries
/// (clipboard / merge) and keeps the structural / sort / filter ops scoped to
/// the clicked column or row (which the header click has already selected).
/// Lives here (always compiled) so the host-tested input handler can set it; the
/// chrome layer re-exports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CtxScope {
    /// Right-click on a grid cell: the full menu.
    #[default]
    Cell,
    /// Right-click on a column/row header: the header-scoped menu.
    Header,
}

/// A resolved pointer hit inside a header strip (`-ui.md` sec.4bis). The body
/// zones select a whole column/row; the border zones (a +/- [`RESIZE_BAND`] band
/// around a boundary) arm a resize; the insert zones host the inline "+"; the
/// top-left corner selects the whole sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeaderHit {
    /// Column header body -> select / drag-extend the whole column.
    ColBody(u32),
    /// Row header body -> select the whole row.
    RowBody(u32),
    /// Band around column `i`'s right border -> resize column `i`.
    ColBorder(u32),
    /// Band around row `i`'s bottom border -> resize row `i`.
    RowBorder(u32),
    /// Inline "+" affordance at a column boundary -> insert a column left of `i`.
    ColInsert(u32),
    /// Inline "+" affordance at a row boundary -> insert a row above `i`.
    RowInsert(u32),
    /// Top-left corner -> select the whole sheet.
    Corner,
}

/// Live guide line shown while dragging a header boundary (`-ui.md` sec.4bis.C):
/// a vertical guide at physical x for a column resize, a horizontal guide at y
/// for a row resize.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ResizePreview {
    Col(f64),
    Row(f64),
}

impl Default for Selection {
    fn default() -> Self {
        Self::single(0, 0)
    }
}

impl Selection {
    /// A selection collapsed to a single cell (`anchor == active`, no extras).
    pub fn single(row: u32, col: u32) -> Self {
        Self {
            anchor: (row, col),
            active: (row, col),
            extra: Vec::new(),
            kind: SelKind::Cells,
        }
    }

    /// A whole-axis band selection from a header (`-ui.md` sec.4bis.A): the band
    /// spans columns `anchor.1..=active.1` (`SelKind::Cols`) or rows
    /// `anchor.0..=active.0` (`SelKind::Rows`); the perpendicular coordinate is
    /// irrelevant (set to 0). `active` carries the clicked band so Shift extends
    /// and arrow keys still resolve an active cell.
    pub fn axis(kind: SelKind, anchor: u32, active: u32) -> Self {
        let (anc, act) = match kind {
            SelKind::Cols => ((0, anchor), (0, active)),
            // Rows (and the degenerate Cells, never built here) key on the row.
            _ => ((anchor, 0), (active, 0)),
        };
        Self {
            anchor: anc,
            active: act,
            extra: Vec::new(),
            kind,
        }
    }

    /// Move the active corner of the live rectangle, keeping the anchor (range
    /// extension: drag, Shift+click, Shift+arrows).
    pub fn extend_to(&mut self, row: u32, col: u32) {
        self.active = (row, col);
    }

    /// Commit the live rectangle and start a fresh single-cell one at
    /// `(row, col)` (Ctrl+click: disjoint selection).
    pub fn add_disjoint(&mut self, row: u32, col: u32) {
        self.extra.push(self.live_rect());
        self.anchor = (row, col);
        self.active = (row, col);
    }

    /// The active cell (row, col).
    pub fn active(&self) -> (u32, u32) {
        self.active
    }

    /// The live rectangle normalized to `(r1, c1, r2, c2)`.
    pub fn live_rect(&self) -> CellRect {
        (
            self.anchor.0.min(self.active.0),
            self.anchor.1.min(self.active.1),
            self.anchor.0.max(self.active.0),
            self.anchor.1.max(self.active.1),
        )
    }

    /// Bounds of the live rectangle (the target of range ops: merge, sort,
    /// style, fill...). Mirrors the spec's `sel.bounds()`.
    pub fn bounds(&self) -> CellRect {
        self.live_rect()
    }

    /// Iterator over every selected rectangle: the committed disjoint ones
    /// followed by the live rectangle.
    pub fn rects(&self) -> impl Iterator<Item = CellRect> + '_ {
        self.extra
            .iter()
            .copied()
            .chain(std::iter::once(self.live_rect()))
    }

    /// True when the selection covers more than a single cell -- either a
    /// multi-cell live rectangle or any committed disjoint range. Drives the
    /// range-fill rendering (a lone single cell shows only the active border).
    pub fn is_multi(&self) -> bool {
        !self.extra.is_empty() || self.anchor != self.active
    }

    /// Every distinct cell `(row, col)` covered by the selection, across all
    /// rectangles, deduplicated (disjoint rectangles may overlap). Backs
    /// Ctrl+Enter "fill the selection with the entered value".
    pub fn selected_cells(&self) -> Vec<(u32, u32)> {
        let mut cells: Vec<(u32, u32)> = self
            .rects()
            .flat_map(|(r1, c1, r2, c2)| {
                (r1..=r2).flat_map(move |r| (c1..=c2).map(move |c| (r, c)))
            })
            .collect();
        cells.sort_unstable();
        cells.dedup();
        cells
    }

    /// True if `(row, col)` falls inside any selected rectangle.
    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.rects()
            .any(|(r1, c1, r2, c2)| row >= r1 && row <= r2 && col >= c1 && col <= c2)
    }

    /// True if `col` is within the live rectangle's column span.
    #[allow(dead_code)]
    pub fn contains_col(&self, col: u32) -> bool {
        let (c1, c2) = (
            self.anchor.1.min(self.active.1),
            self.anchor.1.max(self.active.1),
        );
        col >= c1 && col <= c2
    }

    /// True if `row` is within the live rectangle's row span.
    #[allow(dead_code)]
    pub fn contains_row(&self, row: u32) -> bool {
        let (r1, r2) = (
            self.anchor.0.min(self.active.0),
            self.anchor.0.max(self.active.0),
        );
        row >= r1 && row <= r2
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

    /// Application zoom factor for the grid (1.0 = 100%). Scales cells, the
    /// row/col header strips and the cell font together (`-ui.md` sec.13bis).
    /// Pure local view state -- never persisted, never synced (like the scroll
    /// position). The browser's own page zoom (Ctrl+wheel / Ctrl+/-) is left
    /// untouched and zooms the whole page incl. chrome; this scales the grid only.
    pub zoom: f64,

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

    /// Id of the sheet currently shown. Mirrors `TrameEvent::SheetList`; empty
    /// until the first one arrives. Tags the view's edit commands so they target
    /// the right sheet. The tab strip itself is DOM chrome (`SheetTabs`), fed by
    /// the same event -- the canvas only needs the active id, not the full list.
    pub active_sheet: String,

    /// Monotonically increasing version counter for outgoing `ScrollViewport` commands.
    next_version: u64,

    /// Grid column carrying the active shared filter, drawn with a funnel badge on
    /// its header (`-ui.md` sec.5.D). `None` when no filter is active. Tracked from
    /// `TrameEvent::FilterApplied` / `FilterCleared`; UI-only derived state.
    pub filtered_col: Option<u32>,

    /// Absolute grid rows hidden by the active shared filter, sorted ascending
    /// (`-ui.md` sec.5.D). Derived from the `is_filtered` flag of cached cells and
    /// rebuilt whenever the cache changes. The row axis collapses over these (see
    /// [`visible_offset`](Self::visible_offset)); fed to the renderer's
    /// `GridLayout`. UI-only derived state.
    pub hidden_rows: Vec<u32>,

    /// Per-column widths (physical px) keyed by absolute grid column, fed by the
    /// Logic thread's `col_widths` viewport slice (`-ui.md` sec.4bis.C). A column
    /// with no entry uses the uniform [`COL_WIDTH`]. Drives every pixel mapping
    /// (the renderer's `GridLayout` and the hit-testing helpers below). UI-only
    /// derived state, mirrors the CRDT `col_widths_{sheet}` map.
    pub col_widths: HashMap<u32, f32>,
    /// Per-row heights (physical px) keyed by absolute grid row. Row analogue of
    /// [`col_widths`](Self::col_widths); absence uses [`ROW_HEIGHT`].
    pub row_heights: HashMap<u32, f32>,

    /// The range last copied/cut, drawn with animated marching-ants until the
    /// next copy or an Escape (`-ui.md` sec.5.E). `None` when nothing is on the
    /// clipboard. Stored as the normalized rectangle `(r1, c1, r2, c2)` in
    /// absolute grid coordinates. UI-only view state.
    pub clipboard_anchor: Option<CellRect>,

    /// While the fill handle is being dragged, the destination rectangle the
    /// release will fill (`-ui.md` sec.5.F), normalized and always containing the
    /// source selection. `None` when no fill drag is in progress. Drawn as a
    /// dashed preview outline. UI-only view state.
    pub fill_preview: Option<CellRect>,

    /// While a header boundary is being dragged (`-ui.md` sec.4bis.C), the live
    /// guide line position: `Col(x)` for a column resize (a vertical guide at
    /// physical x), `Row(y)` for a row resize (a horizontal guide at y). `None`
    /// when no resize is in progress. UI-only view state.
    pub resize_preview: Option<ResizePreview>,

    /// The header boundary the pointer is hovering over the inline "+" zone of
    /// (`-ui.md` sec.4bis.D), drawn as a small "+" badge. `Some(ColInsert(i))` /
    /// `Some(RowInsert(i))` while hovered, `None` otherwise. UI-only view state.
    pub insert_hint: Option<HeaderHit>,

    /// Cells matched by the active Find query (`-ui.md` sec.5.G), drawn with a
    /// translucent highlight. Absolute grid `(row, col)` pairs from the last
    /// `TrameEvent::FindResult`; empty when the panel is closed or there is no
    /// match. UI-only view state, never synced.
    pub find_matches: Vec<(u32, u32)>,
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
            clipboard_anchor: None,
            fill_preview: None,
            find_matches: Vec::new(),
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

    /// Bottom edge (physical px) of the scrollable grid area. The sheet-tab strip
    /// moved to DOM chrome (`SheetTabs`), so the grid now fills the full canvas
    /// height; the DOM strip occupies its own shell row outside this surface.
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

    /// Allowed grid zoom presets (50%..200%), shared with the status-bar menu.
    pub const ZOOM_PRESETS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

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
    /// Used on a plain pointer hit; no clamping -- callers pass valid grid coords.
    ///
    /// A hit inside a merged block snaps to the block's master, so the merge is
    /// always selected as a whole (its sub-cells are never individually
    /// selectable, spec sec.2).
    pub fn select(&mut self, row: u32, col: u32) {
        let (row, col) = self.snap_to_master(row, col);
        self.selection = Selection::single(row, col);
    }

    /// Replace the live selection with the rectangle `(r1,c1)`-`(r2,c2)` (corners
    /// already ordered), collapsing the active cell onto the top-left corner. Used
    /// when the Logic thread dictates a selection the canvas cannot derive locally
    /// (a [`SelectRange`](ambre_bus::TrameEvent::SelectRange) after an unmerge):
    /// the freed cells must stay selected though the span-defining merge is gone.
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
    /// are never individually selectable (spec sec.2), so this is always a safe
    /// correction. Called after a viewport batch so a freshly-created merge
    /// re-homes the cursor on its master (top-left): the editor and range ops
    /// then target the master, not a covered cell that would reject the write as
    /// a merge conflict. A no-op when the active cell is a master or unmerged.
    pub fn snap_active_to_master(&mut self) -> bool {
        let (ar, ac) = self.selection.active;
        match self.merge_master_at(ar, ac) {
            Some((mr, mc, cs, rs)) if (mr, mc) != (ar, ac) => {
                // Collapse onto the master only when this one block covers the
                // whole selection (e.g. drag-selected cells that just became a
                // single merge). A selection spanning more than this block -- such
                // as an autofill that produced a grid of merges, whose active
                // corner landed on the last block's covered sub-cell -- must keep
                // its full extent; we only re-home the active corner onto that
                // block's master so it stops sitting on a covered sub-cell. The
                // bounds then re-expand through `expand_to_merges`, so the rendered
                // selection still covers every filled block.
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

    // -------------------------------------------------------------------------
    // Cursor movement
    // -------------------------------------------------------------------------

    /// Move the active cell by `(dr, dc)`.
    ///
    /// With `extend` (Shift held) the selection's active corner moves and the
    /// anchor stays, growing a range; without it the selection collapses onto
    /// the new cell. Scrolls the viewport if the new position falls outside the
    /// visible area. Returns `None` if the cursor did not move (clamped at an
    /// edge); otherwise a [`CursorMove`] carrying the new active cell (always,
    /// so the Logic thread can retarget formatting) plus an optional
    /// `ScrollViewport` to fetch newly exposed cells.
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
        // Landing on a covered sub-cell snaps to its master (only when
        // collapsing: an extension keeps its raw corner so the live rectangle
        // can grow cell by cell).
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
        // is still within the cached range the highlight moves without a Logic
        // round-trip, but we still report the new active cell (formatting target).
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
        // Extend the fetched window by the number of currently-hidden rows in it,
        // so that after the row axis collapses there are still enough *visible*
        // rows to fill the screen (a filtered window would otherwise run short).
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
    /// Older responses (stale version) are silently discarded.
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
        // a non-default entry is stored; a column/row reset to the default drops
        // out of the map so it falls back to the uniform metric (sec.4bis.C).
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
    /// `row` -- the vertical offset (in `ROW_HEIGHT` units) where `row` is drawn
    /// once filtered rows are collapsed out. Mirrors `GridLayout::visible_offset`.
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

    // -------------------------------------------------------------------------
    // Clipboard (spec sec.5.E)
    // -------------------------------------------------------------------------

    /// Serialise the live selection rectangle into a [`ClipboardPayload`] from
    /// the viewport cache (`-ui.md` sec.5.E). Only the cells currently cached
    /// contribute -- the UI holds no document data beyond the viewport (sec.16),
    /// so a copy of cells scrolled far out of view is best-effort. Blank cells
    /// are omitted. Number format / decimals are not carried (the viewport does
    /// not surface them); pasted cells fall back to `General`.
    pub fn build_clipboard_payload(&self) -> ambre_bus::ClipboardPayload {
        let (r1, c1, r2, c2) = self.selection.bounds();
        let mut cells = Vec::new();
        for row in r1..=r2 {
            for col in c1..=c2 {
                let Some(data) = self.cell_cache.get(&(row, col)) else {
                    continue;
                };
                if data.raw.is_empty() {
                    continue;
                }
                cells.push(ambre_bus::ClipboardCell {
                    dr: row - r1,
                    dc: col - c1,
                    raw: data.raw.clone(),
                    bold: data.bold,
                    italic: data.italic,
                    style: ambre_bus::CellStyle {
                        underline: data.underline,
                        strikethrough: data.strikethrough,
                        h_align: data.h_align,
                        v_align: data.v_align,
                        wrap: data.wrap,
                        fg: data.fg,
                        bg: data.bg,
                        font: data.font.clone(),
                        size: data.size,
                        borders: data.borders,
                        ..Default::default()
                    },
                });
            }
        }
        ambre_bus::ClipboardPayload {
            src_row: r1,
            src_col: c1,
            rows: r2 - r1 + 1,
            cols: c2 - c1 + 1,
            cells,
        }
    }

    // -------------------------------------------------------------------------
    // Coordinate helpers
    // -------------------------------------------------------------------------

    /// Mint the next monotonic command version (shared with the viewport
    /// request). Used by commands that carry a `version` for echo ordering, e.g.
    /// [`TrameCommand::ResizeCol`].
    pub fn bump_version(&mut self) -> u64 {
        let v = self.next_version;
        self.next_version += 1;
        v
    }

    /// Width (physical px) of column `col`: its resized entry, else [`COL_WIDTH`].
    pub fn col_width_of(&self, col: u32) -> f64 {
        self.col_widths
            .get(&col)
            .map(|&w| w as f64)
            .unwrap_or(COL_WIDTH)
            * self.zoom
    }
    /// Height (physical px) of row `row`: its resized entry, else [`ROW_HEIGHT`].
    pub fn row_height_of(&self, row: u32) -> f64 {
        self.row_heights
            .get(&row)
            .map(|&h| h as f64)
            .unwrap_or(ROW_HEIGHT)
            * self.zoom
    }

    /// Width (physical px) of the fixed row-number strip on the left, scaled by
    /// the current [`zoom`](Self::zoom). The header strips zoom with the grid.
    pub fn header_w(&self) -> f64 {
        HEADER_WIDTH * self.zoom
    }
    /// Height (physical px) of the fixed column-letter strip on top, scaled by
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
    /// rows, sec.5.D). Mirrors `GridLayout::row_top`.
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

    /// Absolute grid column whose band contains physical x `px` (>= the row
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

    /// Absolute *visible* grid row whose band contains physical y `py` (>= the
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

    /// Cell `(row, col)` at physical pixel `(x, y)`, or `None` if the point is
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

    /// Resolve a pointer at physical `(x, y)` to a header zone (`-ui.md`
    /// sec.4bis), or `None` when the point is over the scrollable grid body (not a
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
    /// cached merge masters (the cache is bounded to the viewport + padding, so
    /// the scan is cheap). The master itself resolves to its own block.
    pub fn merge_master_at(&self, row: u32, col: u32) -> Option<(u32, u32, u32, u32)> {
        self.cell_cache.iter().find_map(|(&(mr, mc), c)| {
            let (col_span, row_span) = c.merge_span?;
            (row >= mr && row < mr + row_span && col >= mc && col < mc + col_span)
                .then_some((mr, mc, col_span, row_span))
        })
    }

    /// Screen pixel rect (x, y, w, h) of the merged block the active cell belongs
    /// to, or `None` when the active cell is not merged (or scrolled out). Lets
    /// the overlay draw the active-cell border around the whole merge.
    pub fn active_merge_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let (mr, mc, cs, rs) = self.merge_master_at(self.active_row(), self.active_col())?;
        self.range_pixel_rect(mr, mc, mr + rs - 1, mc + cs - 1)
    }

    /// Screen pixel rect (x, y, w, h) of cell (row, col) relative to the
    /// top-left corner of the canvas, or `None` if the cell is outside the
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

    /// Screen pixel rect (x, y, w, h) covering the cell range
    /// `(r1, c1)..=(r2, c2)`, clipped to the scrollable grid area (never under a
    /// header strip or the tab strip). Returns `None` if the range is entirely
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
        // Row edges are in collapsed (visible-row) space: the top is the first
        // row's slot, the bottom is one slot past the last *visible* row of the
        // range (so a range straddling hidden rows shrinks to its visible height).
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

    /// Grow `rect` so it fully contains every merged block it intersects. A
    /// merge is one logical cell (spec sec.2): a selection that touches any part
    /// of a block must cover the whole block -- both for drawing (the drag reads
    /// as selecting the merge at once, not cell by cell) and for the autofill
    /// handle (anchored on the block's extent corner). Iterates to a fixpoint
    /// because expanding onto one merge can bring a neighbouring one into range.
    /// Scans the cached merge masters (bounded to the viewport, so it is cheap).
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
    /// (sec.5.F). When the source is tiled by equal merged blocks (one block or a
    /// uniform grid of identically sized merges), returns the block span so the
    /// drag snaps block by block and the release re-merges each tile (Excel
    /// behaviour); otherwise `(1, 1)` keeps the plain per-cell fill. A source that
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
    /// centred on the bottom-right corner of the selection (`-ui.md` sec.5.F).
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

    /// Excel-style "current region" around the active cell (`-ui.md` sec.5.D):
    /// the contiguous rectangle of non-empty cells, grown one line at a time
    /// until the bordering row/column is fully empty. Resolved from the viewport
    /// `cell_cache` (cells beyond the cache read as empty -- acceptable for the
    /// always-on autofilter affordance, which only draws on visible cells).
    /// Returns absolute `(r1, c1, r2, c2)`; `None` when the active cell is an
    /// isolated empty cell (no block to filter).
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

    /// Screen pixel rect (x, y, w, h) of the autofilter button for column `col`
    /// on the current region's header row `band_row` (`-ui.md` sec.5.D): a small
    /// square hugging the right edge of cell `(band_row, col)`. `None` when that
    /// cell is hidden or scrolled out of view.
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

/// Side of the square autofill handle drawn at the selection's bottom-right
/// corner, in device pixels (`-ui.md` sec.5.F).
pub const FILL_HANDLE_SIZE: f64 = 7.0;

/// Side of the square Excel-style autofilter button drawn at the right edge of
/// each current-region header cell, in device pixels at 100% zoom (`-ui.md`
/// sec.5.D). Scaled by the grid zoom in [`filter_button_rect`].
pub const FILTER_BUTTON_SIZE: f64 = 13.0;

#[cfg(test)]
mod tests {
    use super::{CellData, CellEditor, HeaderHit, SelKind, Selection, TrameUiState};
    use crate::{DisplayCell, TrameEvent};

    /// Seed a non-empty cell in the viewport cache (the rest of `CellData`
    /// defaulted), so the current-region helpers see it as data.
    fn put(state: &mut TrameUiState, row: u32, col: u32, text: &str) {
        state.cell_cache.insert(
            (row, col),
            CellData {
                text: text.to_owned(),
                ..Default::default()
            },
        );
    }

    /// `current_region` returns `None` on an isolated empty active cell and the
    /// bounding rectangle of an L-shaped contiguous block otherwise (sec.5.D).
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
    /// of each current-region header cell, returning `(band_row, col)` (sec.5.D).
    #[test]
    fn filter_button_hit_test_matches_drawn_rect() {
        let mut state = TrameUiState::new(800.0, 600.0);
        put(&mut state, 0, 0, "a");
        put(&mut state, 0, 1, "b");
        // (r1, c1, r2, c2): a one-row, two-column block.
        assert_eq!(state.current_region(), Some((0, 0, 0, 1)));

        // The button for column 1 sits at the right edge of cell (0, 1); hitting
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
    /// variable metrics (drives a whole-axis header drag, sec.4bis.A).
    #[test]
    fn axis_pointer_resolution_follows_variable_metrics() {
        let mut state = TrameUiState::new(800.0, 600.0);
        state.col_widths.insert(0, 160.0); // col 0 widened
        // x just past the wide col 0 lands in col 1.
        assert_eq!(state.col_at_x(super::HEADER_WIDTH + 160.0 + 5.0), Some(1));
        assert_eq!(state.col_at_x(super::HEADER_WIDTH + 10.0), Some(0));
        assert_eq!(state.col_at_x(0.0), None); // over the row-header strip

        state.row_heights.insert(0, 50.0);
        assert_eq!(state.row_at_y(super::HEADER_HEIGHT + 50.0 + 5.0), Some(1));
        assert_eq!(state.row_at_y(super::HEADER_HEIGHT + 10.0), Some(0));
        assert_eq!(state.row_at_y(0.0), None);
    }

    /// The insert "+" zone sits in the top/left sliver of a boundary band, the
    /// resize grip in the rest (sec.4bis.C/D).
    #[test]
    fn header_boundary_splits_insert_and_resize() {
        let state = TrameUiState::new(800.0, 600.0);
        let boundary = super::HEADER_WIDTH + super::COL_WIDTH; // col 0's right edge
        // Lower half -> resize col 0.
        assert_eq!(
            state.header_hit(boundary, super::HEADER_HEIGHT * 0.8),
            Some(HeaderHit::ColBorder(0))
        );
        // Upper half -> insert left of col 1 (at this boundary).
        assert_eq!(
            state.header_hit(boundary, 2.0),
            Some(HeaderHit::ColInsert(1))
        );
    }

    /// `header_hit` resolves the three zones of each header strip plus the
    /// corner, with the resize band winning over the body (sec.4bis).
    #[test]
    fn header_hit_resolves_zones() {
        let state = TrameUiState::new(800.0, 600.0);
        // Top-left corner.
        assert_eq!(state.header_hit(10.0, 10.0), Some(HeaderHit::Corner));
        // Column 0 body: header strip, x mid-column 0 ([50, 130)).
        assert_eq!(
            state.header_hit(super::HEADER_WIDTH + 30.0, 5.0),
            Some(HeaderHit::ColBody(0))
        );
        // Column 0 right border (~130), lower half of the band, arms a resize.
        assert_eq!(
            state.header_hit(
                super::HEADER_WIDTH + super::COL_WIDTH,
                super::HEADER_HEIGHT * 0.8
            ),
            Some(HeaderHit::ColBorder(0))
        );
        // Row 1 body (rows start at HEADER_HEIGHT, each ROW_HEIGHT tall).
        assert_eq!(
            state.header_hit(10.0, super::HEADER_HEIGHT + super::ROW_HEIGHT + 5.0),
            Some(HeaderHit::RowBody(1))
        );
        // Row 0 bottom border, right half of the band, resizes row 0.
        assert_eq!(
            state.header_hit(
                super::HEADER_WIDTH * 0.8,
                super::HEADER_HEIGHT + super::ROW_HEIGHT
            ),
            Some(HeaderHit::RowBorder(0))
        );
        // A point in the grid body is not a header hit.
        assert_eq!(
            state.header_hit(super::HEADER_WIDTH + 10.0, super::HEADER_HEIGHT + 10.0),
            None
        );
    }

    /// `Selection::axis` builds a whole-column / whole-row band whose perpendicular
    /// coordinate is irrelevant (sec.4bis.A).
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

    /// A resized column / row shifts the pixel mapping: `cell_at` and
    /// `cell_pixel_rect` must agree on the variable geometry (sec.4bis.C).
    #[test]
    fn variable_metrics_round_trip_pixel_and_hit_test() {
        let mut state = TrameUiState::new(800.0, 600.0);
        // Col 0 widened to 160, row 0 heightened to 40.
        state.col_widths.insert(0, 160.0);
        state.row_heights.insert(0, 40.0);

        assert_eq!(state.col_width_of(0), 160.0);
        assert_eq!(state.col_width_of(1), super::COL_WIDTH);
        assert_eq!(state.row_height_of(0), 40.0);

        // Cell (1,1) sits after the wide col 0 and below the tall row 0.
        let (x, y, w, h) = state.cell_pixel_rect(1, 1).expect("visible");
        assert_eq!(x, super::HEADER_WIDTH + 160.0);
        assert_eq!(y, super::HEADER_HEIGHT + 40.0);
        assert_eq!((w, h), (super::COL_WIDTH, super::ROW_HEIGHT));

        // Hit-testing the interior of that cell returns it.
        assert_eq!(state.cell_at(x + 5.0, y + 5.0), Some((1, 1)));
        // A point inside the tall row 0 / wide col 0 maps to (0, 0).
        assert_eq!(
            state.cell_at(super::HEADER_WIDTH + 100.0, super::HEADER_HEIGHT + 30.0),
            Some((0, 0))
        );
    }

    /// Grid zoom scales every metric (header strips, cell widths/heights) so the
    /// pixel mapping and hit-testing stay self-consistent (`-ui.md` sec.13bis).
    #[test]
    fn zoom_scales_metrics_and_round_trips_hit_test() {
        let mut state = TrameUiState::new(800.0, 600.0);
        assert!(state.set_zoom(2.0));
        assert_eq!(state.zoom, 2.0);

        // Header strips and the default cell metrics double.
        assert_eq!(state.header_w(), super::HEADER_WIDTH * 2.0);
        assert_eq!(state.header_h(), super::HEADER_HEIGHT * 2.0);
        assert_eq!(state.col_width_of(0), super::COL_WIDTH * 2.0);
        assert_eq!(state.row_height_of(0), super::ROW_HEIGHT * 2.0);

        // Column 1's left edge is the (doubled) header plus one (doubled) column.
        assert_eq!(
            state.col_x(1),
            super::HEADER_WIDTH * 2.0 + super::COL_WIDTH * 2.0
        );

        // A point inside cell (2, 2) at 200% maps back to it.
        let x = state.header_w() + super::COL_WIDTH * 2.0 * 2.0 + 1.0;
        let y = state.header_h() + super::ROW_HEIGHT * 2.0 * 2.0 + 1.0;
        assert_eq!(state.cell_at(x, y), Some((2, 2)));

        // The preset clamp rejects out-of-range factors.
        state.set_zoom(99.0);
        assert_eq!(state.zoom, *TrameUiState::ZOOM_PRESETS.last().unwrap());
    }

    /// A 2x2 merge master at (1,1) covering (1,1)..=(2,2): selecting any covered
    /// sub-cell snaps the active cell to the master, and arrow navigation steps
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

    /// Reported bug: two vertical merges A1:A2 and B1:B2 selected by their
    /// masters only (row 0). Before the unmerge the cache holds the merges, so the
    /// rendered selection expands to cover all four cells; after the unmerge the
    /// Logic thread sends a `SelectRange` (applied via `select_rect`) so the freed
    /// cells stay selected once the re-fetch clears the merge spans.
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

    /// Seed a 2x2 merge master at (1,1) covering (1,1)..=(2,2) in the cache.
    fn seed_merge_2x2(state: &mut TrameUiState) {
        let cell = |row, col, span| DisplayCell {
            row,
            col,
            text: String::new(),
            raw: String::new(),
            is_merged: true,
            merge_span: span,
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
        let s = super::FILL_HANDLE_SIZE;
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
        assert_eq!(r4.1 - r1.1, super::ROW_HEIGHT);

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
        assert_eq!(state.cell_at(100.0, super::HEADER_HEIGHT - 1.0), None);
        // The first cell starts just below the header strip.
        assert_eq!(
            state.cell_at(super::HEADER_WIDTH + 1.0, super::HEADER_HEIGHT + 1.0),
            Some((0, 0)),
        );
        // (0,0)'s pixel rect sits at the cell-area top-left.
        let (_, y, _, _) = state.cell_pixel_rect(0, 0).expect("visible");
        assert_eq!(y, super::HEADER_HEIGHT);
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
