//! Serde adapters for identifier serialization.
//!
//! By default, identifiers serialize as arrays of bytes (transparent). These
//! adapters serialize identifiers as hyphenated UUID strings for JSON APIs, and
//! a key identifier as its hex form.
//!
//! Each adapter module serves a bare field through `serialize`/`deserialize`,
//! an `Option` field through its `option` submodule and a `Vec` field through
//! its `vec` submodule:
//!
//! ```ignore
//! #[serde(with = "ambre_core::serde_id::user_id::option")]
//! user_id: Option<UserId>,
//! ```

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::id::*;
use crate::key_id::KeyId;

/// An identifier with a text form on the JSON wire.
trait TextId: Sized {
    fn to_text(&self) -> String;
    fn from_text(text: &str) -> Result<Self, String>;
}

macro_rules! uuid_text_id {
    ($($id_type:ty),* $(,)?) => {
        $(impl TextId for $id_type {
            fn to_text(&self) -> String {
                self.to_string()
            }

            fn from_text(text: &str) -> Result<Self, String> {
                Self::parse_str(text).map_err(|error| error.to_string())
            }
        })*
    };
}

uuid_text_id!(
    DocumentId, UserId, ChannelId, SpaceId, MetaId, SegmentId, CategoryId, MeetingId, SnapshotId,
    OrgId,
);

impl TextId for KeyId {
    fn to_text(&self) -> String {
        self.to_hex()
    }

    fn from_text(text: &str) -> Result<Self, String> {
        Self::parse_hex(text).map_err(|error| error.to_string())
    }
}

fn write<I: TextId, S: Serializer>(id: &I, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_text())
}

fn read<'de, I: TextId, D: Deserializer<'de>>(deserializer: D) -> Result<I, D::Error> {
    let text = String::deserialize(deserializer)?;
    I::from_text(&text).map_err(D::Error::custom)
}

fn write_option<I: TextId, S: Serializer>(
    id: &Option<I>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    id.as_ref().map(TextId::to_text).serialize(serializer)
}

fn read_option<'de, I: TextId, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<I>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|text| I::from_text(&text).map_err(D::Error::custom))
        .transpose()
}

fn write_vec<I: TextId, S: Serializer>(ids: &[I], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(ids.iter().map(TextId::to_text))
}

fn read_vec<'de, I: TextId, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<I>, D::Error> {
    texts_to_ids(Vec::<String>::deserialize(deserializer)?)
}

fn write_option_vec<I: TextId, S: Serializer>(
    ids: &Option<Vec<I>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    ids.as_ref()
        .map(|ids| ids.iter().map(TextId::to_text).collect::<Vec<_>>())
        .serialize(serializer)
}

fn read_option_vec<'de, I: TextId, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<I>>, D::Error> {
    Option::<Vec<String>>::deserialize(deserializer)?
        .map(texts_to_ids)
        .transpose()
}

fn texts_to_ids<I: TextId, E: serde::de::Error>(texts: Vec<String>) -> Result<Vec<I>, E> {
    texts
        .iter()
        .map(|text| I::from_text(text).map_err(E::custom))
        .collect()
}

macro_rules! define_serde_adapter {
    ($name:ident, $id_type:ty) => {
        pub mod $name {
            use super::*;

            /// Writes the identifier as its text form.
            ///
            /// # Errors
            ///
            /// Whatever `serializer` refuses.
            pub fn serialize<S: Serializer>(
                id: &$id_type,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                write(id, serializer)
            }

            /// Reads the identifier from its text form.
            ///
            /// # Errors
            ///
            /// A value that is not a string, or a string that is not the identifier.
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$id_type, D::Error> {
                read(deserializer)
            }

            /// The adapter of an optional identifier: `null` when absent.
            pub mod option {
                use super::super::*;

                /// Writes the identifier as its text form, or `null`.
                ///
                /// # Errors
                ///
                /// Whatever `serializer` refuses.
                pub fn serialize<S: Serializer>(
                    id: &Option<$id_type>,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    write_option(id, serializer)
                }

                /// Reads the identifier from its text form, or `null`.
                ///
                /// # Errors
                ///
                /// A value that is neither `null` nor the identifier's text.
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Option<$id_type>, D::Error> {
                    read_option(deserializer)
                }
            }

            /// The adapter of a list of identifiers.
            pub mod vec {
                use super::super::*;

                /// Writes each identifier as its text form.
                ///
                /// # Errors
                ///
                /// Whatever `serializer` refuses.
                pub fn serialize<S: Serializer>(
                    ids: &[$id_type],
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    write_vec(ids, serializer)
                }

                /// Reads each identifier from its text form.
                ///
                /// # Errors
                ///
                /// A value that is not a list, or an entry that is not the identifier.
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Vec<$id_type>, D::Error> {
                    read_vec(deserializer)
                }
            }

            /// The adapter of an optional list of identifiers: `null` when absent.
            pub mod option_vec {
                use super::super::*;

                /// Writes each identifier as its text form, or `null`.
                ///
                /// # Errors
                ///
                /// Whatever `serializer` refuses.
                pub fn serialize<S: Serializer>(
                    ids: &Option<Vec<$id_type>>,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    write_option_vec(ids, serializer)
                }

                /// Reads each identifier from its text form, or `null`.
                ///
                /// # Errors
                ///
                /// A value that is neither `null` nor a list of the identifier's text.
                pub fn deserialize<'de, D: Deserializer<'de>>(
                    deserializer: D,
                ) -> Result<Option<Vec<$id_type>>, D::Error> {
                    read_option_vec(deserializer)
                }
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
define_serde_adapter!(org_id, OrgId);
define_serde_adapter!(key_id, KeyId);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use crate::id::{DocumentId, OrgId, UserId};
    use crate::key_id::KeyId;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Body {
        #[serde(with = "super::document_id")]
        document_id: DocumentId,
        #[serde(with = "super::org_id::option")]
        org_id: Option<OrgId>,
        #[serde(with = "super::user_id::vec")]
        members: Vec<UserId>,
        #[serde(with = "super::key_id")]
        key_id: KeyId,
        #[serde(default, with = "super::document_id::option_vec")]
        wanted: Option<Vec<DocumentId>>,
    }

    fn body() -> Body {
        Body {
            document_id: DocumentId::from_bytes([0x11; 16]),
            org_id: Some(OrgId::from_bytes([0x22; 16])),
            members: vec![UserId::from_bytes([0x33; 16])],
            key_id: KeyId::from_array([0xab; 16]),
            wanted: Some(vec![DocumentId::from_bytes([0x44; 16])]),
        }
    }

    /// Holds `SRV-179`: each identifier reaches the JSON wire as its text,
    /// the hyphenated string of a UUID and the hex of a key identifier.
    #[test]
    fn an_identifier_reaches_json_as_its_text() {
        let json = serde_json::to_value(body()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "document_id": "11111111-1111-1111-1111-111111111111",
                "org_id": "22222222-2222-2222-2222-222222222222",
                "members": ["33333333-3333-3333-3333-333333333333"],
                "key_id": "abababababababababababababababab",
                "wanted": ["44444444-4444-4444-4444-444444444444"],
            })
        );
        let back: Body = serde_json::from_value(json).unwrap();
        assert_eq!(back, body());
    }

    /// Holds `SRV-179`: an absent optional identifier reads `null` both ways.
    #[test]
    fn an_absent_identifier_reads_null() {
        let mut absent = body();
        absent.org_id = None;
        absent.wanted = None;
        let mut json = serde_json::to_value(&absent).unwrap();
        assert_eq!(json["org_id"], serde_json::Value::Null);
        assert_eq!(json["wanted"], serde_json::Value::Null);
        assert_eq!(
            serde_json::from_value::<Body>(json.clone()).unwrap(),
            absent
        );
        // A list left out of the body reads as absent too.
        json.as_object_mut().unwrap().remove("wanted");
        assert_eq!(serde_json::from_value::<Body>(json).unwrap(), absent);
    }

    /// Holds `SRV-179`: a text that is not the identifier is refused at decode.
    /// So is it in a bare field, an optional one and the two kinds of list.
    #[test]
    fn a_malformed_identifier_is_refused() {
        let good = serde_json::to_value(body()).unwrap();
        for (field, bad) in [
            ("document_id", serde_json::json!("not-a-uuid")),
            ("document_id", serde_json::json!(vec![0x11; 16])),
            ("org_id", serde_json::json!("22")),
            ("members", serde_json::json!(["33333333", "x"])),
            ("key_id", serde_json::json!("abab")),
            ("key_id", serde_json::json!("zz".repeat(16))),
            ("wanted", serde_json::json!(["not-a-uuid"])),
        ] {
            let mut json = good.clone();
            json[field] = bad;
            assert!(
                serde_json::from_value::<Body>(json).is_err(),
                "{field} decoded a malformed identifier"
            );
        }
    }
}
