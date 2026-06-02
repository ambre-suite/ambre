//! Trame (spreadsheet) application crate.
//!
//! Holds **everything** specific to Trame:
//!
//! - [`events`] — IPC types ([`TrameCommand`], [`TrameViewUpdate`],
//!   [`DisplayCell`]).
//! - [`TOTAL_ROWS`] / [`TOTAL_COLS`] — grid dimensions shared by the controller
//!   (bounds clamping) and the UI (virtual-scroll math).
//! - [`logic`] — [`TrameController`], the `AppController` impl driven by the
//!   Logic thread (CRDT / formula engine).
//! - [`ui`] — `TrameUiState` + the `DrawSurface` renderer driven by the UI
//!   Thread (wgpu / text layout).
//!
//! The UI half is consumed by `ambre-app` and the logic half by `ambre-logic`;
//! both link into the single WASM binary.

pub mod events;
pub mod logic;
pub mod ui;

pub use events::{DisplayCell, TrameCommand, TrameViewUpdate};
pub use logic::TrameController;

/// Total number of rows in a Trame grid (0-based indices `0..TOTAL_ROWS`).
pub const TOTAL_ROWS: u32 = 100_000;
/// Total number of columns in a Trame grid (0-based indices `0..TOTAL_COLS`).
pub const TOTAL_COLS: u32 = 1_000;
