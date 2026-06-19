//! Trame (spreadsheet) application crate.
//!
//! Holds **everything** specific to Trame:
//!
//! - [`logic`] — [`TrameState`], the per-document model driven by the Logic
//!   thread (CRDT / formula engine), plus its command/event handlers.
//! - [`ui`] — `TrameUiState` + the `DrawSurface` renderer driven by the UI
//!   Thread (wgpu / text layout).
//! - [`TOTAL_ROWS`] / [`TOTAL_COLS`] — grid dimensions shared by the logic
//!   (bounds clamping) and the UI (virtual-scroll math).
//!
//! The IPC message types ([`TrameCommand`], [`TrameEvent`], [`DisplayCell`])
//! live in `ambre-bus` (the typed application data plane) and are re-exported
//! here for convenience.
//!
//! The UI half is consumed by `ambre-app` and the logic half by `ambre-logic`;
//! both link into the single WASM binary.

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
    BorderEdit, BorderStyle, BorderPreset, Borders, CellBorder, CellStyle, CondFormatKind, CondStyle,
    DisplayCell, FilterKind, FilterOp,
    GridRange, HAlign, CellFormat, SortKey, StylePatch, TrameCommand, TrameEvent, VAlign,
    ValidationKind, ValidationOnInvalid,
};
pub use logic::{TrameOutcome, TrameState};

/// Total number of rows in a Trame grid (0-based indices `0..TOTAL_ROWS`).
pub const TOTAL_ROWS: u32 = 100_000;
/// Total number of columns in a Trame grid (0-based indices `0..TOTAL_COLS`).
pub const TOTAL_COLS: u32 = 1_000;
