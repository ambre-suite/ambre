//! Why the engine refused a document, a delta or a command.

use ambre_core::schema::SchemaError;
use ambre_core::{ChannelId, DocumentId, MessageId, SegmentId, SpaceId};
use thiserror::Error;

use crate::delta::Refusal;

/// Every refusal of the engine. None of them changes a document the channel
/// already holds: each check runs before the engine keeps what it read.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MurmureError {
    /// The bytes or the version refused the read (`MURMURE-019`).
    #[error(transparent)]
    Schema(#[from] SchemaError),

    /// The header names another channel (`MURMURE-027`).
    #[error("document {document_id} names channel {found}, not {expected}")]
    ForeignChannel {
        document_id: DocumentId,
        expected: ChannelId,
        found: ChannelId,
    },

    /// The header of a space document names another space.
    #[error("document {document_id} names space {found}")]
    ForeignSpace {
        document_id: DocumentId,
        found: SpaceId,
    },

    /// Only a public channel enters the catalogue (`MURMURE-014`).
    #[error("channel {0} is not public, so the catalogue cannot list it")]
    NotPublic(ChannelId),

    /// Only a direct message takes a name from any member (`MURMURE-054`).
    #[error("channel {0} is not a direct message, so no member renames it")]
    NotDirect(ChannelId),

    /// The catalogue lists no channel of that identifier.
    #[error("the catalogue lists no channel {0}")]
    NotListed(ChannelId),

    /// The header names another kind of document.
    #[error("document {document_id} is a {found}, where a {expected} was expected")]
    WrongKind {
        document_id: DocumentId,
        expected: &'static str,
        found: String,
    },

    /// A header key is absent or holds a value this build cannot read.
    #[error("the header of document {document_id} has no readable `{}`", .key.as_str())]
    HeaderMalformed {
        document_id: DocumentId,
        key: crate::schema::HeaderKey,
    },

    /// The `meta` document lists no active segment, so no message can go
    /// anywhere.
    #[error("the meta document of channel {0} lists no active segment")]
    NoActiveSegment(ChannelId),

    /// The `meta` document lists a live segment whose bytes the caller did not
    /// supply (`MURMURE-022`).
    #[error("segment {0} is live but its bytes were not supplied")]
    SegmentMissing(DocumentId),

    /// The channel holds no document of that identifier (`MURMURE-024`).
    #[error("document {0} is not held by this channel")]
    NotHeld(DocumentId),

    /// A close names a segment that is no longer the active one.
    #[error("segment {0} is not the active segment")]
    NotActive(DocumentId),

    /// A close while another segment of the channel is still closing, which
    /// would list two closing segments (`MURMURE-010`).
    #[error("a segment of channel {0} is still closing")]
    StillClosing(ChannelId),

    /// The document is not a final segment the `meta` document lists.
    #[error("document {0} is not an older segment of this channel")]
    NotOlder(DocumentId),

    /// A final segment changes through its amendments alone, never in place.
    #[error("segment {0} is final")]
    Final(DocumentId),

    /// A change of a locked segment before the channel holds its amendments
    /// document, which the change waits for.
    #[error("segment {0} has no amendments document held")]
    NoAmendments(SegmentId),

    /// The header of an amendments document names another segment than the
    /// one it was opened for (`MURMURE-013`).
    #[error("amendments document {0} names another segment")]
    ForeignSegment(DocumentId),

    /// The header of a thread document names another root, or another
    /// segment, than the one it was opened for (`MURMURE-009`).
    #[error("thread document {0} names another root")]
    ForeignRoot(DocumentId),

    /// A reply before the channel holds the thread document of its root,
    /// which the reply waits for.
    #[error("message {0} has no thread document held")]
    NoThread(MessageId),

    /// The segment holds no message of that identifier.
    #[error("no message {0} in the segment")]
    NoMessage(MessageId),

    /// An edit or a delete of a message another user wrote (`MURMURE-085`).
    #[error("message {0} has another author")]
    NotAuthor(MessageId),

    /// An edit of a message its author deleted (`MURMURE-087`).
    #[error("message {0} is deleted")]
    Deleted(MessageId),

    /// A reaction that is not one emoji (`MURMURE-092`).
    #[error("a reaction that is not one emoji")]
    NotEmoji,

    /// A mention whose range is empty or past the body (`MURMURE-100`).
    #[error("a mention outside the body of its message")]
    BadMention,

    /// A style whose range is empty or past the body (`MURMURE-561`). Also a
    /// link to an address that is not `http` or `https`, or two links over
    /// one char.
    #[error("a style outside the body of its message, or a link refused")]
    BadStyle,

    /// A send named an attachment with a blank name. Nothing is written.
    #[error("an attachment with no name")]
    BadAttachment,

    /// The authorship check refused a delta, which changed nothing
    /// (`MURMURE-030`).
    #[error("the authorship check refused a delta: {0}")]
    Refused(#[from] Refusal),

    /// The Sync worker has not issued the PeerID of the document yet, and an
    /// operation under any other never travels (`AUTHOR-029`).
    #[error("document {0} has no session identity yet")]
    NoSession(DocumentId),

    /// Loro refused a write or an export. Carries its message, because the
    /// Loro error types are not `Eq`.
    #[error("the CRDT engine refused the operation: {0}")]
    Crdt(String),
}

impl From<loro::LoroError> for MurmureError {
    fn from(e: loro::LoroError) -> Self {
        Self::Crdt(e.to_string())
    }
}

impl From<loro::LoroEncodeError> for MurmureError {
    fn from(e: loro::LoroEncodeError) -> Self {
        Self::Crdt(e.to_string())
    }
}
