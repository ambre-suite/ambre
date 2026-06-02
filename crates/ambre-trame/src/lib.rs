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

pub mod logic;
pub mod ui;

pub use ambre_bus::{DisplayCell, TrameCommand, TrameEvent};
pub use logic::TrameState;

/// Total number of rows in a Trame grid (0-based indices `0..TOTAL_ROWS`).
pub const TOTAL_ROWS: u32 = 100_000;
/// Total number of columns in a Trame grid (0-based indices `0..TOTAL_COLS`).
pub const TOTAL_COLS: u32 = 1_000;
