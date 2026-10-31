//! Toile **Logic-thread** engine.
//!
//! Groups every logic-side module of the crate: the CRDT-backed [`state`]
//! machine ([`ToileState`]), the command / event surface ([`command`]), and the
//! CRDT [`model`]. It also groups the `rstar` spatial [`index`], the
//! [`connector`] geometry and [`stroke`] smoothing. The UI renderer,
//! presentation mode and interop paths arrive in later crates / slices (Epic
//! `E4`).

pub mod align;
pub mod animate;
pub mod autolayout;
pub mod boolean;
pub mod command;
pub mod connector;
pub mod index;
pub mod interop;
pub mod model;
/// Toile's persisted schema version and its migration registry
/// (`ambre-spec-schema-version.md`).
pub mod schema;
pub mod state;
pub mod stroke;
pub mod text_merge;

pub use align::align_deltas;
pub use animate::{AnimatedElement, FrameSnapshot, SnapElement, ease_in_out_cubic, smart_animate};
pub use autolayout::{Align, AutoLayout, Direction, Sizing, solve_layout};
pub use boolean::boolean_op;
pub use command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, End, InitContent,
    SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome, ZTarget,
};
pub use index::{ElementBounds, SpatialIndex, Viewport};
pub use model::{
    Anchor, ElementKind, ElementStyle, EndCaps, FontSpec, NodeId, PathType, Shadow, ShapeKind,
    StrokePoint,
};
pub use state::ToileState;

/// A board text as a `Debug` form prints it: its length in characters and
/// none of its content (`ZK-006`).
pub(crate) struct TextLen<'a>(pub(crate) &'a str);

impl std::fmt::Debug for TextLen<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} chars>", self.0.chars().count())
    }
}

/// A board document as a `Debug` form prints it: its length in bytes and
/// none of its content (`ZK-006`).
pub(crate) struct ByteLen<'a>(pub(crate) &'a str);

impl std::fmt::Debug for ByteLen<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

/// A board outline as a `Debug` form prints it: how many path elements it
/// holds and none of their points (`ZK-006`).
pub(crate) struct PathLen<'a>(pub(crate) &'a kurbo::BezPath);

impl std::fmt::Debug for PathLen<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} path elements>", self.0.elements().len())
    }
}

/// 2-peer CRDT convergence property test (DoD). Host-only.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod convergence_tests;

/// The shape of a board driven through the command surface. Host-only.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod structure_tests;
