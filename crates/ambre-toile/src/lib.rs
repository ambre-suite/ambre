//! Whiteboard & presentations application for Ambre (Toile).
//!
//! Holds the Toile **Logic-thread** engine: [`ToileState`], the per-document
//! model that owns the single-board `LoroDoc`, the `rstar` spatial index, and
//! the element / connector / stroke lifecycle, and derives a culled
//! [`DisplayViewport`] for the UI thread. See `specs/3-Toile/`.
//!
//! Every logic-side module lives under [`logic`]; Phase 0 (core standalone)
//! lives here, while the UI renderer, presentation mode and interop paths
//! arrive in later crates / slices (Epic `E4`).

pub mod logic;

pub use logic::command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, End, InitContent,
    SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome, ZTarget,
};
pub use logic::index::{ElementBounds, SpatialIndex, Viewport};
pub use logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, FontSpec, NodeId, PathType,
    Shadow, ShapeKind, StrokePoint,
};
pub use logic::state::ToileState;
