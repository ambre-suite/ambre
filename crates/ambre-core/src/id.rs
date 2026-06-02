//! Strongly-typed identifiers.
//!
//! Each identifier is a distinct newtype over [`Uuid`], so the compiler refuses
//! to pass (say) a [`UserId`] where a [`DocumentId`] is expected — a class of
//! bug that an untyped `Uuid` everywhere cannot catch.
//!
//! The wire representation is unchanged: every newtype is `#[serde(transparent)]`,
//! so a `DocumentId` serializes byte-for-byte like the inner `Uuid` (postcard /
//! JSON compatibility is preserved across the network and IPC boundaries).

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
    /// Identifies a document (Prose, Trame, Toile, …) across the whole system.
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
    /// Identifies a Rendez-Vous meeting room.
    MeetingId
}

define_id! {
    /// Identifies a CRDT snapshot of a document.
    SnapshotId
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

    #[test]
    fn debug_carries_type_name() {
        let id = UserId::from_bytes([0; 16]);
        assert!(format!("{id:?}").starts_with("UserId("));
    }
}
