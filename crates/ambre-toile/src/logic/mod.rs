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
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, FontSpec, NodeId, PathType,
    Shadow, ShapeKind, StrokePoint,
};
pub use state::ToileState;

/// 2-peer CRDT convergence property test (DoD). Host-only.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod convergence_tests;
