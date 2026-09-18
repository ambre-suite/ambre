//! Whiteboard & presentations application for Ambre (Toile).
//!
//! Holds the Toile **Logic-thread** engine: [`ToileState`], the per-document
//! model that owns the single-board `LoroDoc` and the `rstar` spatial index. It
//! runs the element / connector / stroke lifecycle and derives a culled
//! [`DisplayViewport`] for the UI thread. See `specs/3-Toile/`.
//!
//! Every logic-side module lives under [`logic`]. Phase 0 (core standalone)
//! lives here, while the UI renderer, presentation mode and interop paths
//! arrive in later crates / slices (Epic `E4`).

pub mod logic;
pub mod ui;

/// Toile-specific chrome (the Find panel and, later, the tool palette / context
/// menu / status bar), built on the `ambre-chrome` toolkit. Wasm-only (leptos
/// CSR + DOM), like `ambre-trame::chrome` -- scoped to wasm32 so host logic /
/// render tests never build it.
#[cfg(target_arch = "wasm32")]
pub mod chrome;

pub use logic::command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, End, InitContent,
    SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome, ToileRequest, ZTarget,
};
pub use logic::index::{ElementBounds, SpatialIndex, Viewport};
pub use logic::interop::docfile::{export_ambre_plaintext, import_ambre_plaintext};
pub use logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, FontSpec, NodeId, PathType,
    Shadow, ShapeKind, StrokePoint,
};
pub use logic::state::ToileState;
