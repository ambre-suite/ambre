//! Core value types of the Toile CRDT model (`ambre-spec-toile-schema.md`).
//!
//! These pure-data types (identity, element kinds, the single per-element style
//! register, freehand strokes, connector anchoring) live in `ambre-bus` so the
//! `ambre-logic` dispatcher can carry them inside `AppCommand::Toile` /
//! `AppEvent::Toile` without a dependency cycle. This module re-exports them so
//! the engine keeps its `crate::logic::model::...` paths (mirrors how `ambre-trame`
//! consumes its value types from `ambre-bus`).

pub use ambre_bus::app::toile::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, FontSpec, FrameId, NodeId,
    PathType, PlaybackState, Shadow, ShapeKind, StrokePoint, TargetId, ThreadId,
};
