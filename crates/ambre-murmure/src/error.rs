//! Why the engine refused a document, a delta or a command.

use ambre_core::schema::SchemaError;
use ambre_core::{ChannelId, DocumentId, SpaceId};
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
    #[error("the header of document {document_id} has no readable `{key}`")]
    HeaderMalformed {
        document_id: DocumentId,
        key: &'static str,
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

    /// The document is not a final segment the `meta` document lists.
    #[error("document {0} is not an older segment of this channel")]
    NotOlder(DocumentId),

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
