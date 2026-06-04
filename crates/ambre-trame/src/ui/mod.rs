// ambre-trame/src/ui/mod.rs
// ============================================================================
// Trame UI layer -- UI Thread side.
//
//   grid     -- virtual-scroll navigation state (TrameUiState).
//   renderer -- DrawSurface rendering of the grid (draw_cells / draw_overlay).
//   view     -- TrameView, the `ambre_canvas::DocView` impl driven by the FrameHost.
//
// The shared IPC types live in `ambre-bus` (re-exported from the crate root);
// the grid dimensions live in the crate root (`crate::TOTAL_ROWS` / `TOTAL_COLS`).
// ============================================================================

pub mod grid;
pub mod renderer;
pub mod view;

pub use grid::TrameUiState;
pub use view::TrameView;
