//! Toile **Logic-thread** engine.
//!
//! Groups every logic-side module of the crate: the CRDT-backed [`state`]
//! machine ([`ToileState`]), the command / event surface ([`command`]), the
//! CRDT [`model`], the `rstar` spatial [`index`], the [`connector`] geometry and
//! [`stroke`] smoothing. The UI renderer, presentation mode and interop paths
//! arrive in later crates / slices (Epic `E4`).

pub mod command;
pub mod connector;
pub mod index;
pub mod model;
pub mod state;
pub mod stroke;

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

/// 2-peer CRDT convergence property test (§10, DoD). Host-only.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod convergence_tests;
