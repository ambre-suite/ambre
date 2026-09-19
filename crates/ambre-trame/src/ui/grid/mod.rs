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

use ambre_canvas::{TextEditor, TextEditorStyle};
use ambre_draw::Color;

use crate::render::BODY_SIZE;
use crate::{GridRange, SelectionKind, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

// Cell layout.

// The `impl TrameUiState` block is split across `state`/`geometry`; both reach
// the types, consts and helpers here via `use super::*`.
mod geometry;
mod state;

pub const ROW_HEIGHT: f64 = 20.0;

pub const COL_WIDTH: f64 = 80.0;

pub const HEADER_HEIGHT: f64 = 24.0;

pub const HEADER_WIDTH: f64 = 50.0;

/// Lower clamp on a column/row resize (physical px); mirrors the Logic-side
/// `MIN_COL_WIDTH` / `MIN_ROW_HEIGHT` so the UI preview and the persisted value
/// agree.
pub const MIN_COL_WIDTH: f64 = 24.0;

pub const MIN_ROW_HEIGHT: f64 = 12.0;

/// Half-width (physical px) of the grab band straddling a header boundary that
/// arms a resize drag (`trame-resize`). A press within +/- this of a column's right
/// edge (resp. a row's bottom edge) resizes instead of selecting.
pub const RESIZE_BAND: f64 = 4.0;

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
    /// Rich manual formatting (`trame-cell-styles`): underline run, alignments,
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
    /// The cell's row is hidden by the active shared filter (`trame-shared-filter`):
    /// the row axis collapses over it. Drives [`TrameUiState::hidden_rows`].
    pub is_filtered: bool,
}

impl CellData {
    /// Build the renderer's [`CellContent`](crate::render::CellContent) view of
    /// this cached cell, with `text` supplied by the caller. A merge master draws
    /// empty body text in the single-cell pass, its real text in the merge pass.
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

/// In-progress edit of a single cell. A thin Trame wrapper over the shared
/// `ambre_canvas::TextEditor` core (parley `PlainEditor` + font-free `EditIntent`
/// queue), the same core Toile's element editor drives. All editing actions are
/// recorded as font-free intents and applied at paint time (`draw_edit`), where a
/// `FontContext` is live. A commit therefore reads the buffer only after that
/// flush.
///
/// The wrapper only binds the core to a `(row, col)`. Every editing operation and
/// buffer accessor reaches the core through `Deref`/`DerefMut`. The named intent
/// constructors -- `insert`, `backspace`, the caret moves -- live on the core
/// now. Pointer hits push point intents the core resolves against its own layout
/// (no renderer-side byte-offset resolution).
pub struct CellEditor {
    pub row: u32,
    pub col: u32,
    /// The shared editor core (drawn by `renderer::draw_edit`).
    pub(crate) core: TextEditor,
}

impl CellEditor {
    /// Open an editor on `(row, col)` seeded with `text`, caret at the end. The
    /// pinned style is a placeholder; `draw_edit` re-pins it from the cell's
    /// formatting each paint before flushing.
    pub fn new(row: u32, col: u32, text: String) -> Self {
        let style = TextEditorStyle {
            weight: 400,
            italic: false,
            family: "Sans".to_owned(),
            base_size: BODY_SIZE,
            color: Color::from_rgba8(0, 0, 0, 255),
            underline: false,
            strikethrough: false,
        };
        Self {
            row,
            col,
            core: TextEditor::new(&text, style),
        }
    }

    /// The currently selected substring, or empty when nothing is selected.
    /// Font-free (safe inside a clipboard `copy` / `cut` event). Trame wants a
    /// plain `String`; the core returns an `Option`.
    pub fn selected_text(&self) -> String {
        self.core.selected_text().unwrap_or_default()
    }
}

// Every editing operation and buffer accessor lives on the shared core; the
// wrapper only binds it to a `(row, col)`. Deref exposes `insert`, `backspace`,
// the caret/selection moves, `text`, `set_text` and `push` directly on a
// `CellEditor`. The inherent `selected_text` above shadows the core's, for the
// Trame `String` signature.
impl std::ops::Deref for CellEditor {
    type Target = TextEditor;
    fn deref(&self) -> &TextEditor {
        &self.core
    }
}

impl std::ops::DerefMut for CellEditor {
    fn deref_mut(&mut self) -> &mut TextEditor {
        &mut self.core
    }
}

/// A normalized cell rectangle `(r1, c1, r2, c2)` with `r1 <= r2` and
/// `c1 <= c2`. Used for the committed disjoint rectangles of a multi-range
/// selection and as the return type of [`Selection::bounds`].
pub type CellRect = (u32, u32, u32, u32);

/// The grid selection (`trame-selection`). A single-cell selection has `anchor ==
/// active` and no extra ranges. A contiguous range keeps `anchor` fixed and
/// moves `active` (drag, Shift+click, Shift+arrows). Disjoint ranges
/// (Ctrl+click) push the live rectangle into `extra` and start a fresh one.
///
/// `active` is the cell the cursor, the inline editor and the formatting
/// commands target; `anchor` is the opposite corner of the live rectangle.
/// Selection is local view state -- never synced (`trame-selection`).
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
    /// (`trame-selection`). For `Cols`/`Rows` the perpendicular coordinate of
    /// `anchor`/`active` is ignored (the band runs the full axis); the Logic
    /// thread resolves the band to the used-range rectangle. UI-only view state.
    pub kind: SelKind,
}

/// Shape of the live selection (`trame-selection`). `Cells` is the ordinary
/// rectangle; `Cols`/`Rows` are virtual whole-axis bands selected from a header.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelKind {
    #[default]
    Cells,
    Cols,
    Rows,
}

impl From<SelKind> for SelectionKind {
    fn from(kind: SelKind) -> Self {
        match kind {
            SelKind::Cells => SelectionKind::Cells,
            SelKind::Cols => SelectionKind::Cols,
            SelKind::Rows => SelectionKind::Rows,
        }
    }
}

/// Which surface a right-click landed on, choosing the context-menu layout
/// (`trame-context-menu`). Each header scope drops the cell-only entries (clipboard /
/// merge). It keeps only the structural + sort/filter ops relevant to the axis the
/// click already selected. A column header offers column insert/delete, a row header
/// row insert/delete, and the corner the whole-sheet ops. Lives here (always
/// compiled) so the host-tested input handler can set it; the chrome layer re-exports
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CtxScope {
    /// Right-click on a grid cell: the full menu.
    #[default]
    Cell,
    /// Right-click on a column header: column-scoped structural + sort/filter.
    Column,
    /// Right-click on a row header: row-scoped structural + sort/filter.
    Row,
    /// Right-click on the top-left corner: whole-sheet ops only.
    Corner,
}

/// A resolved pointer hit inside a header strip (`trame-headers`). The body
/// zones select a whole column/row; the border zones (a +/- [`RESIZE_BAND`] band
/// around a boundary) arm a resize. The insert zones host the inline "+"; the
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

/// Live guide line shown while dragging a header boundary (`trame-resize`). A
/// vertical guide at physical x for a column resize, a horizontal guide at y
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

    /// A whole-axis band selection from a header (`trame-selection`): the band
    /// spans columns `anchor.1..=active.1` (`SelKind::Cols`) or rows
    /// `anchor.0..=active.0` (`SelKind::Rows`). The perpendicular coordinate is
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

    /// Extend a whole-axis band to column/row `idx`, keeping the anchor
    /// (`trame-selection`). The band kind is the caller's; a `Cells` selection
    /// has no band to extend and is left untouched.
    pub fn extend_axis(&mut self, kind: SelKind, idx: u32) {
        match kind {
            SelKind::Cols => self.active = (0, idx),
            SelKind::Rows => self.active = (idx, 0),
            SelKind::Cells => {}
        }
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

    /// The whole selection as a [`TrameCommand::SetActiveCell`] report
    /// (`trame-selection`): the live rectangle's two corners, every committed
    /// disjoint rectangle, and the band kind. A whole-axis band carries no
    /// extent across the other axis. The Logic thread resolves that extent from
    /// the used range, never from the UI cache.
    pub fn report(&self) -> TrameCommand {
        let (row, col) = self.active;
        let (anchor_row, anchor_col) = self.anchor;
        let extra = self
            .extra
            .iter()
            .map(|&(r1, c1, r2, c2)| GridRange {
                row: r1,
                col: c1,
                row_end: r2,
                col_end: c2,
            })
            .collect();
        TrameCommand::SetActiveCell {
            row,
            col,
            anchor_row,
            anchor_col,
            extra,
            kind: self.kind.into(),
        }
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

/// All mutable UI state for the Trame grid on the UI Thread.
pub struct TrameUiState {
    /// Top-left visible row (0-based).
    pub top_row: u32,
    /// Top-left visible column (0-based).
    pub left_col: u32,

    /// Current selection. Single-cell for now (`anchor == active`); the active
    /// cell drives the cursor, inline editor and formatting commands (`trame-selection`).
    pub selection: Selection,

    /// Number of full rows visible in the grid area (recomputed on resize).
    pub viewport_rows: u32,
    /// Number of full columns visible in the grid area.
    pub viewport_cols: u32,

    /// Canvas physical size (CSS pixels x devicePixelRatio, set by ResizeObserver).
    pub canvas_width: f64,
    pub canvas_height: f64,

    /// Application zoom factor for the grid (1.0 = 100%). Scales cells, the
    /// row/col header strips and the cell font together (`trame-status-bar`).
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

    /// Cells whose formula evaluation failed, keyed to the error string (for example
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
    /// its header (`trame-shared-filter`). `None` when no filter is active. Tracked from
    /// `TrameEvent::FilterApplied` / `FilterCleared`; UI-only derived state.
    pub filtered_col: Option<u32>,

    /// Absolute grid rows hidden by the active shared filter, sorted ascending
    /// (`trame-shared-filter`). Derived from the `is_filtered` flag of cached cells and
    /// rebuilt whenever the cache changes. The row axis collapses over these (see
    /// [`visible_offset`](Self::visible_offset)); fed to the renderer's
    /// `GridLayout`. UI-only derived state.
    pub hidden_rows: Vec<u32>,

    /// Per-column widths (physical px) keyed by absolute grid column, fed by the
    /// Logic thread's `col_widths` viewport slice (`trame-resize`). A column
    /// with no entry uses the uniform [`COL_WIDTH`]. Drives every pixel mapping
    /// (the renderer's `GridLayout` and the hit-testing helpers below). UI-only
    /// derived state, mirrors the CRDT `col_widths_{sheet}` map.
    pub col_widths: HashMap<u32, f32>,
    /// Per-row heights (physical px) keyed by absolute grid row. Row analogue of
    /// [`col_widths`](Self::col_widths); absence uses [`ROW_HEIGHT`].
    pub row_heights: HashMap<u32, f32>,

    /// The range last copied/cut, drawn with animated marching-ants until the
    /// next copy or an Escape (`trame-clipboard`). `None` when nothing is on the
    /// clipboard. Stored as the normalized rectangle `(r1, c1, r2, c2)` in
    /// absolute grid coordinates. UI-only view state.
    pub clipboard_anchor: Option<CellRect>,

    /// While the fill handle is being dragged, the destination rectangle the
    /// release will fill (`trame-autofill`), normalized and always containing the
    /// source selection. `None` when no fill drag is in progress. Drawn as a
    /// dashed preview outline. UI-only view state.
    pub fill_preview: Option<CellRect>,

    /// While a header boundary is being dragged (`trame-resize`), the live
    /// guide line position. `Col(x)` for a column resize (a vertical guide at
    /// physical x), `Row(y)` for a row resize (a horizontal guide at y). `None`
    /// when no resize is in progress. UI-only view state.
    pub resize_preview: Option<ResizePreview>,

    /// The header boundary the pointer is hovering over the inline "+" zone of
    /// (`trame-headers`), drawn as a small "+" badge. `Some(ColInsert(i))` /
    /// `Some(RowInsert(i))` while hovered, `None` otherwise. UI-only view state.
    pub insert_hint: Option<HeaderHit>,

    /// The column/row header body the pointer is hovering over, tinted with
    /// `header_hover` for chrome-consistent hover feedback (`trame-sheets`).
    /// Only ever `Some(ColBody(_))` / `Some(RowBody(_))`; `None` otherwise.
    /// UI-only view state.
    pub hover_header: Option<HeaderHit>,

    /// While a selected column/row header is being dragged to a new position
    /// (Excel-style reorder), the target insertion slot. `(Cols, i)` means the
    /// grabbed column would land left of column `i`, `(Rows, i)` above row `i`.
    /// Drawn as a full-height/width guide line at that boundary. `None` when no
    /// reorder drag is in progress. UI-only view state.
    pub reorder_preview: Option<(SelKind, u32)>,

    /// Cells matched by the active Find query (`trame-find-replace`), drawn with a
    /// translucent highlight. Absolute grid `(row, col)` pairs from the last
    /// `TrameEvent::FindResult`; empty when the panel is closed or there is no
    /// match. UI-only view state, never synced.
    pub find_matches: Vec<(u32, u32)>,
}

/// Side of the square autofill handle drawn at the selection's bottom-right
/// corner, in device pixels (`trame-autofill`).
pub const FILL_HANDLE_SIZE: f64 = 7.0;

/// Side of the square Excel-style autofilter button drawn at the right edge of
/// each current-region header cell, in device pixels at 100% zoom
/// (`trame-shared-filter`). Scaled by the grid zoom in [`filter_button_rect`].
pub const FILTER_BUTTON_SIZE: f64 = 13.0;

#[cfg(test)]
mod tests;
