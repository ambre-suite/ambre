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
    CondFormatKind, CondStyle, DisplayCell, FillMode, FilterKind, FilterOp, GridRange, HAlign,
    HeaderRow, MergeSpan, Row, RowCount, RowDelta, RowSpan, RowVec, SelectionKind, SortKey,
    SortOrder, StylePatch, TrameCommand, TrameEvent, VAlign, ValidationKind, ValidationOnInvalid,
};
pub use logic::docfile::{export_ambre_plaintext, import_ambre_plaintext};
pub use logic::{
    CaseMatch, FindOptions, MatchExtent, SearchScope, SheetScope, TrameOutcome, TrameState,
};

/// Total number of rows in a Trame grid (0-based indices `0..TOTAL_ROWS`).
pub const TOTAL_ROWS: RowCount = RowCount::new(100_000);
/// Total number of columns in a Trame grid (0-based indices `0..TOTAL_COLS`).
pub const TOTAL_COLS: ColCount = ColCount::new(1_000);
/// The last row of a Trame grid.
pub const LAST_ROW: Row = Row::new(TOTAL_ROWS.get() - 1);
/// The last column of a Trame grid.
pub const LAST_COL: Col = Col::new(TOTAL_COLS.get() - 1);
