//! The activity summary an application may add to an update
//! (`ambre-spec-residency.md section 3`).
//!
//! It tells a client what an update holds with no document loaded: the kind of
//! change, and who it names. It travels on
//! [`SyncCommand::SendUpdate`](crate::SyncCommand::SendUpdate), so it crosses
//! the postcard hop and derives serde.

use serde::{Deserialize, Serialize};

use crate::UserId;

/// What one update holds, readable with no Loro document.
///
/// A member can lie in a summary. The lie costs one false badge, and the
/// document corrects it on open (`RESID-011`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySummary {
    /// The kind of change the update carries.
    pub kind: ActivityKind,
    /// The users the update names.
    pub mentions: Vec<UserId>,
    /// Whether the update names every member, an `@channel`.
    pub mentions_all: bool,
    /// An application key, for example a thread root. Opaque here.
    pub anchor: Option<ActivityAnchor>,
}

/// The kinds of change a summary distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityKind {
    /// New content, a message for example.
    Content,
    /// A reaction on existing content.
    Reaction,
    /// An edit of existing content.
    Edit,
    /// Anything else, which moves no counter.
    Other,
}

/// An application key, opaque to the foundation and typed by the application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityAnchor(pub Vec<u8>);
