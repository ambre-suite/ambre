// ambre-trame/src/ui/mod.rs
// ============================================================================
// Trame UI layer -- UI Thread side.
//
// Behind the `ui` feature so that crates wiring only the logic side
// (ambre-logic) don't pull the wgpu / text-layout dependency tree.
//
//   grid     -- virtual-scroll navigation state (TrameUiState).
//   renderer -- DrawSurface rendering of the grid (draw_cells / draw_overlay).
//
// The shared IPC types live in `ambre-bus` (re-exported from the crate root);
// the grid dimensions live in the crate root (`crate::TOTAL_ROWS` / `TOTAL_COLS`).
// ============================================================================

pub mod grid;
pub mod renderer;
pub mod view;

pub use grid::TrameUiState;
pub use view::TrameView;
