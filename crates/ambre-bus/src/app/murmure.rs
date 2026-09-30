//! Murmure data plane: the command a view sends to a channel, the event the
//! engine answers with, and the value types both carry
//! (`features/ambre-spec-murmure-engine.md section 3`).
//!
//! Pure data, like every module of [`crate::app`]: no Loro type crosses the
//! bus (`SYNC-011`). The engine that reads and writes the documents lives in
//! `ambre-murmure`.
//!
//! A channel is a set of documents, never one document (`MURMURE-021`). So the
//! command is addressed to a [`ChannelId`](crate::ChannelId), carried by
//! [`LogicCommand::Channel`](crate::LogicCommand::Channel), and never by
//! [`LogicCommand::App`](crate::LogicCommand::App), whose address is a
//! document (`BUS-027`).

use serde::{Deserialize, Serialize};

use crate::{ChannelId, DocumentId, MessageId, UserId};

/// What a document of a channel is (`ambre-spec-murmure-schema.md`).
///
/// The relay stores it beside each document (`ZK-015`), and the listing
/// answers it, so it crosses the Sync worker's `postMessage` hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelDocumentKind {
    /// The one `meta` document: the channel's name, type and segment list.
    Meta,
    /// One time slice of the channel's messages.
    Segment,
    /// The corrections to a final segment.
    Amendments,
    /// The replies to one message.
    Thread,
}

impl ChannelDocumentKind {
    /// The kind a stored word names, or `None` for a word no build wrote.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "meta" => Some(Self::Meta),
            "segment" => Some(Self::Segment),
            "amendments" => Some(Self::Amendments),
            "thread" => Some(Self::Thread),
            _ => None,
        }
    }
}

/// One document a channel groups, as the relay lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDocument {
    pub document_id: DocumentId,
    pub kind: ChannelDocumentKind,
}

/// Whether one document of a channel holds something past the caller's
/// read cursor. The relay answers a yes or no, never a count (`SRV-110`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentActivity {
    pub channel_id: ChannelId,
    pub document_id: DocumentId,
    pub active: bool,
}

/// Whether a channel holds something new, the mark the sidebar shows before
/// any count is known (`MURMURE-119`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelActivity {
    pub channel_id: ChannelId,
    pub active: bool,
}

/// A wall-clock instant in Unix milliseconds.
///
/// Display only: a peer's clock is untrusted, so no rule orders or refuses a
/// message on it (`ambre-spec-murmure-schema.md section 2`). The view stamps
/// it, because the Logic thread reads no clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub u64);

/// What a mention names: one member, or every member of the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MentionTarget {
    /// One member, by user identifier.
    User(UserId),
    /// The channel itself, an `@channel`.
    Channel,
}

/// A message the view hands the engine to send.
///
/// The text is plain. The `mention` marks over it are written by the mentions
/// feature. Until then the mentions reach the activity summary alone
/// (`MURMURE-081`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDraft {
    /// The body, as the author typed it.
    pub text: String,
    /// Who the body names, in the order the author typed them.
    pub mentions: Vec<MentionTarget>,
}

/// A command for an open channel, UI thread -> Logic thread.
///
/// Carried by [`LogicCommand::Channel`](crate::LogicCommand::Channel). The
/// variants are the ones the engine builds today. The spec lists the others,
/// each with the Story that brings it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MurmureCommand {
    /// Append a message to the active segment, under this session's user
    /// (`MURMURE-080`).
    SendMessage {
        draft: MessageDraft,
        /// The instant the author sent it, stamped by the view.
        now: Timestamp,
    },
    /// Load the final segment listed before `segment`, because the view
    /// scrolled past the oldest message it holds (`MURMURE-023`).
    LoadOlder { segment: DocumentId },
    /// The view shows the latest message of the channel: move the read
    /// cursor to it (`MURMURE-115`). A relay record, never a document write.
    MarkRead,
}

impl MurmureCommand {
    /// Whether the command can change a document of the channel.
    ///
    /// The same question [`AppCommand::is_mutating`](crate::AppCommand::is_mutating)
    /// answers for a document. A load reads.
    pub fn is_mutating(&self) -> bool {
        match self {
            Self::SendMessage { .. } => true,
            Self::LoadOlder { .. } | Self::MarkRead => false,
        }
    }
}

/// One message as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    /// The identity the author's peer minted (`MURMURE-002`).
    pub id: MessageId,
    /// The user who wrote the message (`MURMURE-003`).
    pub author: UserId,
    /// The instant the author's peer stamped.
    pub ts: Timestamp,
    /// The body, as plain text.
    pub text: String,
}

/// An event from an open channel, Logic thread -> UI thread.
///
/// Carried by [`UiEvent::Channel`](crate::UiEvent::Channel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MurmureEvent {
    /// These messages of `segment` are new or changed. The view keys them by
    /// [`MessageId`], so a message it already holds is replaced, never
    /// duplicated. `messages` keeps the order of the segment.
    Messages {
        segment: DocumentId,
        messages: Vec<MessageView>,
    },
    /// The channel could not open: the relay refused it, or one of its
    /// documents failed the checks. Nothing of it is held.
    OpenFailed { message: String },
}
