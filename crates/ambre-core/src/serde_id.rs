//! Serde adapters for identifier serialization.
//!
//! By default, identifiers serialize as arrays of bytes (transparent). These
//! adapters serialize identifiers as hyphenated UUID strings for JSON APIs.

use serde::{Deserialize, Deserializer, Serializer};

use crate::id::*;

macro_rules! define_serde_adapter {
    ($name:ident, $id_type:ty) => {
        pub mod $name {
            use super::*;

            /// Writes the identifier as its hyphenated UUID string.
            ///
            /// # Errors
            ///
            /// Whatever `serializer` refuses.
            pub fn serialize<S>(id: &$id_type, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&id.to_string())
            }

            /// Reads the identifier from a UUID string.
            ///
            /// # Errors
            ///
            /// A value that is not a string, or a string that is not a UUID.
            pub fn deserialize<'de, D>(deserializer: D) -> Result<$id_type, D::Error>
            where
                D: Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                <$id_type>::parse_str(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_serde_adapter!(document_id, DocumentId);
define_serde_adapter!(user_id, UserId);
define_serde_adapter!(channel_id, ChannelId);
define_serde_adapter!(space_id, SpaceId);
define_serde_adapter!(meta_id, MetaId);
define_serde_adapter!(segment_id, SegmentId);
define_serde_adapter!(category_id, CategoryId);
define_serde_adapter!(meeting_id, MeetingId);
define_serde_adapter!(snapshot_id, SnapshotId);
