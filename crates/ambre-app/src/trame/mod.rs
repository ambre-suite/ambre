// ambre-app/src/trame/mod.rs
// ============================================================================
// Trame UI module -- UI Thread side.
//
// Owns the grid navigation state and the wgpu/DrawSurface rendering code.
// Heavy logic (CRDT, formula engine) lives in ambre-logic-worker/src/trame/.
// Shared IPC types (TrameCommand, TrameViewUpdate, DisplayCell) are in ambre-trame.
// ============================================================================

pub mod grid;
pub mod renderer;

pub use grid::TrameUiState;
