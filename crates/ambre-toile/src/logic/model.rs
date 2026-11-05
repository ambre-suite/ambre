//! Core value types of the Toile CRDT model (`ambre-spec-toile-schema.md`).
//!
//! These pure-data types (identity, element kinds, the single per-element style
//! register, freehand strokes, connector anchoring) live in the message
//! vocabulary (`crate::message`). The Logic runtime carries them inside
//! `AppCommand::Toile` / `AppEvent::Toile`. This module re-exports them so the
//! engine keeps its `crate::logic::model::...` paths.

pub use crate::message::{
    Anchor, ElementKind, ElementStyle, EndCaps, FontSpec, LinkTarget, NodeId, PathType,
    PlaybackState, Shadow, ShapeKind, StrokePoint, UnitFraction,
};
