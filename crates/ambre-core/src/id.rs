//! Strongly-typed identifiers.
//!
//! Each identifier is a distinct newtype over [`Uuid`], so the compiler refuses to
//! pass (say) a [`UserId`] where a [`DocumentId`] is expected. That is a class of
//! bug an untyped `Uuid` everywhere cannot catch.
//!
//! The wire representation is unchanged. Every newtype is `#[serde(transparent)]`,
//! so a `DocumentId` serializes byte-for-byte like the inner `Uuid`. Postcard and
//! JSON compatibility is preserved across the network and IPC boundaries.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::UuidError;
use crate::uuid::Uuid;

/// Defines a strongly-typed identifier newtype wrapping [`Uuid`].
///
/// Generates the constructors, accessors, conversions, and `Debug`/`Display`
/// impls shared by every id type, so adding a new id is a single invocation.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new identifier backed by a fresh UUID v7.
            pub fn new() -> Self {
                Self(Uuid::new())
            }

            /// Wraps an existing [`Uuid`].
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Builds an identifier from a raw 16-byte array.
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Uuid::from_bytes(bytes))
            }

            /// Returns the underlying [`Uuid`].
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Returns the underlying 16-byte array.
            pub const fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }

            /// Parses an identifier from its string representation.
            ///
            /// Accepts both hyphenated and non-hyphenated forms.
            ///
            /// # Errors
            ///
            /// [`UuidError::InvalidString`] when `s` is not a UUID in either form.
            pub fn parse_str(s: &str) -> Result<Self, UuidError> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

define_id! {
    /// Identifies a document (Prose, Trame, Toile, ...) across the whole system.
    DocumentId
}

define_id! {
    /// Identifies a user account.
    UserId
}

define_id! {
    /// Identifies a Murmure channel.
    ChannelId
}

define_id! {
    /// Identifies a Murmure space. A space is a document, so its identifier
    /// is the identifier of that document (the Murmure spaces spec).
    SpaceId
}

impl SpaceId {
    /// The space document this space is.
    pub const fn document_id(&self) -> DocumentId {
        DocumentId::from_uuid(self.0)
    }

    /// The space a space document is.
    pub const fn from_document(document_id: DocumentId) -> Self {
        Self(*document_id.as_uuid())
    }
}

define_id! {
    /// Identifies the `meta` document of a Murmure channel. A distinct type,
    /// so a segment id never takes its place: access to a channel resolves
    /// through the member rows of this one document.
    MetaId
}

impl MetaId {
    /// The document this `meta` is.
    pub const fn document_id(&self) -> DocumentId {
        DocumentId::from_uuid(self.0)
    }

    /// The `meta` a document is, for a caller that knows its kind.
    pub const fn from_document(document_id: DocumentId) -> Self {
        Self(*document_id.as_uuid())
    }
}

define_id! {
    /// Identifies a segment of a Murmure channel: one document of its message
    /// log. A distinct type, so a segment id never takes the place of `meta`.
    SegmentId
}

impl SegmentId {
    /// The document this segment is.
    pub const fn document_id(&self) -> DocumentId {
        DocumentId::from_uuid(self.0)
    }

    /// The segment a document is, for a caller that knows its kind.
    pub const fn from_document(document_id: DocumentId) -> Self {
        Self(*document_id.as_uuid())
    }
}

define_id! {
    /// Identifies a category of a Murmure space, minted by the admin's client.
    CategoryId
}

define_id! {
    /// Identifies a Murmure message, minted by the peer that creates it
    /// (the Murmure schema). A position in a list moves with every concurrent
    /// append, so a pin or a reply names the message by this value instead.
    MessageId
}

define_id! {
    /// Identifies a Rendez-Vous meeting room.
    MeetingId
}

define_id! {
    /// Identifies a CRDT snapshot of a document.
    SnapshotId
}

impl SnapshotId {
    /// The genesis snapshot of a document: a deterministic id every client agrees
    /// on without a server round-trip, reusing the document's own bytes.
    ///
    /// A brand-new document has no uploaded snapshot, yet its first deltas must
    /// reference SOME parent. If a client invented a random base id, a peer (or
    /// the same client after a refresh) opening the document would never
    /// recognise that parent. It would quarantine those deltas forever. Anchoring
    /// the pre-first-snapshot base to `genesis(document_id)` makes the parent
    /// universally known, so the earliest deltas are always releasable. Once a
    /// real snapshot is uploaded, that snapshot's id supersedes the genesis.
    pub const fn genesis(document_id: DocumentId) -> Self {
        Self::from_bytes(*document_id.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_transparent_over_inner_uuid() {
        // A typed id must serialize exactly like its inner Uuid (wire compat).
        let uuid = Uuid::new();
        let id = DocumentId::from_uuid(uuid);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            serde_json::to_string(&uuid).unwrap(),
        );
    }

    #[test]
    fn roundtrip() {
        let id = DocumentId::new();
        let bytes = serde_json::to_vec(&id).unwrap();
        let back: DocumentId = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(id, back);
    }

    /// Holds `SYNC-026`: the parent of a document never snapshotted is the
    /// document's own bytes. A peer derives it from the id alone, so the
    /// derivation is pinned: changing it would strand every peer's first deltas.
    #[test]
    fn every_client_derives_the_same_genesis_parent() {
        let bytes = [0x5a; 16];
        let doc = DocumentId::from_bytes(bytes);
        assert_eq!(SnapshotId::genesis(doc), SnapshotId::from_bytes(bytes));
        // A second client, holding the id only through its text, agrees.
        let parsed = DocumentId::parse_str(&doc.to_string()).unwrap();
        assert_eq!(SnapshotId::genesis(parsed), SnapshotId::genesis(doc));
        assert_ne!(
            SnapshotId::genesis(DocumentId::from_bytes([0x5b; 16])),
            SnapshotId::genesis(doc)
        );
    }

    #[test]
    fn debug_carries_type_name() {
        let id = UserId::from_bytes([0; 16]);
        assert!(format!("{id:?}").starts_with("UserId("));
    }
}
