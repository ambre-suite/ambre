//! Trame (spreadsheet) application crate.
//!
//! Holds **everything** specific to Trame:
//!
//! - [`logic`] -- [`TrameState`], the per-document model driven by the Logic
//!   thread (CRDT / formula engine), plus its command/event handlers.
//! - [`ui`] -- `TrameUiState` + the `DrawSurface` renderer driven by the UI
//!   Thread (wgpu / text layout).
//! - [`TOTAL_ROWS`] / [`TOTAL_COLS`] -- grid dimensions shared by the logic
//!   (bounds clamping) and the UI (virtual-scroll math).
//!
//! The IPC message types ([`TrameCommand`], [`TrameEvent`], [`DisplayCell`])
//! live in `ambre-bus` (the typed application data plane) and are re-exported
//! here for convenience.
//!
//! The UI half is consumed by `ambre-app` and the logic half by `ambre-logic`;
//! both link into the single WASM binary.

/// Canonical spreadsheet column-label conversions (`col_to_alpha` /
/// `alpha_to_col`), shared by the formula engine, logic, and UI/chrome.
pub mod columns;
pub mod formula;
pub mod logic;
pub mod model;
/// Shared, `DrawSurface`-generic grid rendering primitives (`GridLayout`,
/// `draw_cells_on`, `draw_grid_lines_on`, `draw_col_headers_on`, ...). Composed
/// by `ui::renderer` for the screen and by the future PDF path for export.
pub mod render;
pub mod ui;

/// Trame-specific chrome (format toolbar, selection toolbar, command wiring),
/// built on the `ambre-chrome` toolkit. Wasm-only (leptos CSR + DOM): the
/// launcher mounts it; host logic tests never compile it.
#[cfg(target_arch = "wasm32")]
pub mod chrome;

/// Test double for the Sync thread (`MockSyncHandle`), used by controller tests
/// to capture emitted `SyncCommand`s and relay CRDT deltas between peers. Built
/// only for the crate's own test runs.
#[cfg(test)]
pub mod test_support;

pub use ambre_bus::{
    BorderEdit, BorderPreset, BorderStyle, Borders, CellBorder, CellFormat, CellPos, CellRect,
    CellStyle, ClipboardCell, ClipboardPayload, Col, ColCount, ColDelta, ColSpan, ColVec,
    CondFormatKind, CondStyle, CsvDelimiter, Decimals, DecimalsStep, DefinedName, DisplayCell,
    EditSessionId, FillMode, FilterKind, FilterOp, HAlign, HeaderRow, MergeSpan, Row, RowCount,
    RowDelta, RowSpan, RowVec, SelectionKind, SheetTabPos, SortKey, SortOrder, StylePatch,
    TrameCommand, TrameEvent, VAlign, ValidationKind, ValidationOnInvalid,
};
use ambre_core::DocPx;
pub use logic::docfile::{export_ambre_plaintext, import_ambre_plaintext};
pub use logic::{
    CaseMatch, FindOptions, MatchExtent, SearchScope, SheetScope, TrameOutcome, TrameState,
};

/// The width of a column never resized, before the zoom. The Logic thread fills
/// the viewport slice with it, and the UI lays out the columns past the slice.
pub const COL_WIDTH: DocPx = DocPx::new(80.0);
/// The height of a row never resized, before the zoom.
pub const ROW_HEIGHT: DocPx = DocPx::new(20.0);
/// The narrowest a resize leaves a column. The UI preview and the stored width
/// clamp to the same value.
pub const MIN_COL_WIDTH: DocPx = DocPx::new(24.0);
/// The shortest a resize leaves a row.
pub const MIN_ROW_HEIGHT: DocPx = DocPx::new(12.0);
/// The widest a column gets, from a local resize or from a peer. The bound
/// keeps the sum of every width finite.
pub const MAX_COL_WIDTH: DocPx = DocPx::new(4096.0);
/// The tallest a row gets, from a local resize or from a peer.
pub const MAX_ROW_HEIGHT: DocPx = DocPx::new(2048.0);

/// Total number of rows in a Trame grid (0-based indices `0..TOTAL_ROWS`).
pub const TOTAL_ROWS: RowCount = RowCount::new(100_000);
/// Total number of columns in a Trame grid (0-based indices `0..TOTAL_COLS`).
pub const TOTAL_COLS: ColCount = ColCount::new(1_000);
/// The last row of a Trame grid.
pub const LAST_ROW: Row = Row::new(TOTAL_ROWS.get() - 1);
/// The last column of a Trame grid.
pub const LAST_COL: Col = Col::new(TOTAL_COLS.get() - 1);
