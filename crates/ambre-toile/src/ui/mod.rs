// ambre-toile/src/ui/mod.rs
// ============================================================================
// Toile UI thread (toile-ui). The `ui` module of the `ambre-toile` app crate:
// it consumes the culled `DisplayViewport` produced by the Logic thread, paints
// it through the shared `WgpuSurface` (from `ambre-canvas`), and turns pointer /
// keyboard input into `ToileCommand`s. It owns no CRDT state and never touches
// `LoroDoc` -- the same seam as Trame's `ui` module.
//
// Phase 0 (core standalone): the camera (`viewport`), the adaptive grid, exact
// hit-testing (`hit`), the Select tool state machine (`tool`), the content walk
// (`render`), and the `ToileView` that drives them each frame (`view`).
// ============================================================================

pub mod edit;
pub mod find;
pub mod floatbar;
pub mod grid;
pub mod hit;
pub mod menu;
pub mod menu_bar;
pub mod palette;
pub mod presence;
pub mod present;
pub mod presenter;
pub mod reactions;
pub mod render;
pub mod status;
pub mod style;
pub mod tool;
pub mod view;
pub mod viewport;

pub use tool::Tool;
pub use view::ToileView;
pub use viewport::ViewportTransform;
