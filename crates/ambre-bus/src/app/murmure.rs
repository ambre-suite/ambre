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

use crate::{CategoryId, ChannelDocumentKind, ChannelId, DocumentId, MessageId, SegmentId, UserId};

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
    LoadOlder { segment: SegmentId },
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
        segment: SegmentId,
        messages: Vec<MessageView>,
    },
    /// The channel could not open: the relay refused it, or one of its
    /// documents failed the checks. Nothing of it is held.
    OpenFailed { message: String },
}

/// A channel key sealed under the key of its space (`MURMURE-157`).
///
/// The Logic thread holds it and never opens it: the space document lives
/// there, and no key material may reach it (`SYNC-039`). Only the Sync
/// worker, which holds the space key, seals and unseals it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedChannelKey(Vec<u8>);

impl SealedChannelKey {
    /// Wrap the sealed bytes the Sync worker produced.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The sealed bytes, as the catalogue stores them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SealedChannelKey {
    // The length alone: a log line needs no ciphertext.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealedChannelKey({} bytes)", self.0.len())
    }
}

/// The topic of a channel: a line of text under its name.
///
/// Never empty: a channel with no topic holds no topic at all, so the
/// catalogue stores an absent key rather than an empty string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Topic(String);

impl Topic {
    /// The topic `text` states, or `None` for an empty text.
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        (!text.is_empty()).then_some(Self(text))
    }

    /// The text of the topic.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Topic {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text).ok_or("a topic is never empty")
    }
}

impl From<Topic> for String {
    fn from(topic: Topic) -> Self {
        topic.0
    }
}

/// A category of a space, as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryView {
    pub id: CategoryId,
    pub name: String,
    /// The rank among the categories: lower first, ties by identifier.
    pub position: u32,
}

/// One public channel of the catalogue, as the view draws it.
///
/// No key: the view has no use for one, and the sealed key stays in the space
/// document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub channel_id: ChannelId,
    pub name: String,
    pub topic: Option<Topic>,
    /// Absent for a channel outside every category.
    pub category: Option<CategoryId>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: u32,
}

/// A public channel the view asks a space to create and list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicChannelDraft {
    /// Minted by the view, so it can follow the channel before it exists.
    pub channel_id: ChannelId,
    pub name: String,
    pub topic: Option<Topic>,
    /// Absent for a channel outside every category.
    pub category: Option<CategoryId>,
    /// The rank in the catalogue: lower first, ties by identifier.
    pub position: u32,
}

/// A command for an open space, UI thread -> Logic thread.
///
/// Carried by [`LogicCommand::Space`](crate::LogicCommand::Space). A space is
/// one document, and its catalogue lists its public channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceCommand {
    /// Add a category to the catalogue.
    AddCategory {
        category_id: CategoryId,
        name: String,
        /// The rank among the categories: lower first, ties by identifier.
        position: u32,
    },
    /// Create a public channel, then list it in the catalogue with its key
    /// sealed under the space key (`MURMURE-037`).
    CreatePublicChannel {
        draft: PublicChannelDraft,
        /// The instant of the creation, stamped by the view.
        now: Timestamp,
    },
    /// Join a public channel of the catalogue, with no other member online
    /// (`MURMURE-039`).
    JoinChannel { channel_id: ChannelId },
}

/// An event from an open space, Logic thread -> UI thread.
///
/// Carried by [`UiEvent::Space`](crate::UiEvent::Space).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceEvent {
    /// The space and its catalogue, whole, after an open or any change.
    Catalogue {
        /// Absent when the space has no name.
        name: Option<String>,
        categories: Vec<CategoryView>,
        channels: Vec<CatalogueEntry>,
    },
    /// The relay holds the new channel, and the catalogue lists it.
    ChannelCreated { channel_id: ChannelId },
    /// The user is now a member of the channel, which the view may open.
    Joined { channel_id: ChannelId },
    /// A creation or a join failed. Nothing changed on the relay.
    ChannelFailed {
        channel_id: ChannelId,
        message: String,
    },
    /// The space could not open. Nothing of it is held.
    OpenFailed { message: String },
}

#[cfg(test)]
mod tests {
    use super::Topic;

    /// An empty topic is no topic: it cannot be built, nor decoded from the
    /// wire, so the catalogue never stores an empty string.
    #[test]
    fn a_topic_is_never_empty() {
        assert_eq!(Topic::new(""), None);
        let figures = Topic::new("the weekly figures").expect("a topic");
        assert_eq!(figures.as_str(), "the weekly figures");

        let bytes = postcard::to_allocvec(&figures).expect("encode");
        assert_eq!(postcard::from_bytes::<Topic>(&bytes), Ok(figures));
        let empty = postcard::to_allocvec("").expect("encode");
        assert!(postcard::from_bytes::<Topic>(&empty).is_err());
    }
}
