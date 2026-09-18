// ambre-toile/src/ui/mod.rs
// ============================================================================
// Toile UI thread (toile-board-rendering). This is the `ui` module of the
// `ambre-toile` app crate. It consumes the culled `DisplayViewport` produced by
// the Logic thread and paints it through the shared `WgpuSurface` (from
// `ambre-canvas`). It turns pointer / keyboard input into `ToileCommand`s. It
// owns no CRDT state and never touches `LoroDoc` -- the same seam as Trame's
// `ui` module.
//
// Phase 0 (core standalone) covers the camera (`viewport`), the adaptive grid,
// exact hit-testing (`hit`) and the Select tool state machine (`tool`). It also
// covers the content walk (`render`), and the `ToileView` that drives them each
// frame (`view`).
// ============================================================================

pub mod arrow;
pub mod commands;
pub mod comments;
pub mod deck;
pub mod edit;
pub mod embed;
pub mod find;
pub mod floatbar;
pub mod grid;
pub mod hit;
pub mod interop;
pub mod label;
pub mod link;
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
