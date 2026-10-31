//! Serde adapters for identifier serialization.
//!
//! By default, an identifier serializes as its bytes (transparent). The
//! [`text`] adapter writes an identifier as its hyphenated UUID string for a
//! JSON API, and a key identifier as its hex form.
//!
//! One generic adapter serves every identifier: the type of the field fixes
//! which one. Each field shape has its own module:
//!
//! | Field | Module |
//! | :--- | :--- |
//! | bare | `text` |
//! | `Option` | `text::option` |
//! | `Vec` | `text::vec` |
//! | `Option<Vec>` | `text::option_vec` |
//!
//! ```ignore
//! #[serde(with = "ambre_core::serde_id::text::option")]
//! user_id: Option<Id<User>>,
//! ```

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::FixedBytes;
use crate::id::{Entity, Id};
use crate::key_id::KeyId;
use crate::uuid::Uuid;

mod sealed {
    /// Closes [`super::TextId`]: the identifiers with a text form are listed in
    /// this module.
    pub trait Sealed {}
}

/// An identifier with a text form on the JSON wire. Sealed: a `Uuid`
/// identifier and a key identifier.
pub trait TextId: sealed::Sealed + Sized {
    /// The text form of the identifier.
    fn to_text(&self) -> String;

    /// Parses the identifier from its text form.
    ///
    /// # Errors
    ///
    /// The reason, when `text` is not the text form of an identifier.
    fn from_text(text: &str) -> Result<Self, String>;
}

impl<E: Entity<Repr = Uuid>> sealed::Sealed for Id<E> {}
impl<E: Entity<Repr = Uuid>> TextId for Id<E> {
    fn to_text(&self) -> String {
        self.to_string()
    }

    fn from_text(text: &str) -> Result<Self, String> {
        Self::parse_str(text).map_err(|error| error.to_string())
    }
}

impl sealed::Sealed for FixedBytes<KeyId> {}
impl TextId for FixedBytes<KeyId> {
    fn to_text(&self) -> String {
        self.to_hex()
    }

    fn from_text(text: &str) -> Result<Self, String> {
        Self::from_hex(text).map_err(|error| format!("not a key identifier: {error}"))
    }
}

fn texts_to_ids<I: TextId, E: serde::de::Error>(texts: Vec<String>) -> Result<Vec<I>, E> {
    texts
        .iter()
        .map(|text| I::from_text(text).map_err(E::custom))
        .collect()
}

/// The text adapter of an identifier.
pub mod text {
    use super::*;

    /// Writes the identifier as its text form.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<I: TextId, S: Serializer>(id: &I, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&id.to_text())
    }

    /// Reads the identifier from its text form.
    ///
    /// # Errors
    ///
    /// A value that is not a string, or a string that is not the identifier.
    pub fn deserialize<'de, I: TextId, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<I, D::Error> {
        let text = String::deserialize(deserializer)?;
        I::from_text(&text).map_err(D::Error::custom)
    }

    /// The adapter of an optional identifier: `null` when absent.
    pub mod option {
        use super::super::*;

        /// Writes the identifier as its text form, or `null`.
        ///
        /// # Errors
        ///
        /// Whatever `serializer` refuses.
        pub fn serialize<I: TextId, S: Serializer>(
            id: &Option<I>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            id.as_ref().map(TextId::to_text).serialize(serializer)
        }

        /// Reads the identifier from its text form, or `null`.
        ///
        /// # Errors
        ///
        /// A value that is neither `null` nor the identifier's text.
        pub fn deserialize<'de, I: TextId, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<I>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .map(|text| I::from_text(&text).map_err(D::Error::custom))
                .transpose()
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
        pub fn serialize<I: TextId, S: Serializer>(
            ids: &[I],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(ids.iter().map(TextId::to_text))
        }

        /// Reads each identifier from its text form.
        ///
        /// # Errors
        ///
        /// A value that is not a list, or an entry that is not the identifier.
        pub fn deserialize<'de, I: TextId, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<I>, D::Error> {
            texts_to_ids(Vec::<String>::deserialize(deserializer)?)
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
        pub fn serialize<I: TextId, S: Serializer>(
            ids: &Option<Vec<I>>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            ids.as_ref()
                .map(|ids| ids.iter().map(TextId::to_text).collect::<Vec<_>>())
                .serialize(serializer)
        }

        /// Reads each identifier from its text form, or `null`.
        ///
        /// # Errors
        ///
        /// A value that is neither `null` nor a list of the identifier's text.
        pub fn deserialize<'de, I: TextId, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Vec<I>>, D::Error> {
            Option::<Vec<String>>::deserialize(deserializer)?
                .map(texts_to_ids)
                .transpose()
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use crate::FixedBytes;
    use crate::id::{Document, Id, Org, User};
    use crate::key_id::KeyId;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Body {
        #[serde(with = "super::text")]
        document_id: Id<Document>,
        #[serde(with = "super::text::option")]
        org_id: Option<Id<Org>>,
        #[serde(with = "super::text::vec")]
        members: Vec<Id<User>>,
        #[serde(with = "super::text")]
        key_id: FixedBytes<KeyId>,
        #[serde(default, with = "super::text::option_vec")]
        wanted: Option<Vec<Id<Document>>>,
    }

    fn body() -> Body {
        Body {
            document_id: Id::from_bytes([0x11; 16]),
            org_id: Some(Id::from_bytes([0x22; 16])),
            members: vec![Id::from_bytes([0x33; 16])],
            key_id: FixedBytes::from_array([0xab; 16]),
            wanted: Some(vec![Id::from_bytes([0x44; 16])]),
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
