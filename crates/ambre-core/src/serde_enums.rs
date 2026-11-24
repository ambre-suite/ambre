//! Serde adapters for enum serialization as lowercase strings.
//!
//! These adapters serialize enums as their lowercase string representations for
//! JSON APIs. For example, `DocType::Prose` serializes as "prose" and
//! deserializes from "prose" alone. Each closed set has one parser, which
//! refuses any other word when the body is read.
//!
//! The refusal is serde's own unknown variant error, the one a derived enum
//! raises. A reader can then tell a word this build does not know from any
//! other malformed body.

use serde::{Deserialize, Deserializer, Serializer};

use crate::{ChannelDocumentKind, DocType, DocumentScope, Role, Visibility};

/// Reads a word of a closed set, refusing one `parse` does not know as an
/// unknown variant among `words`.
fn read_word<'de, D, T>(
    deserializer: D,
    words: &'static [&'static str],
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    parse(&word).ok_or_else(|| serde::de::Error::unknown_variant(&word, words))
}

// DocType adapter: "prose", "trame", "toile", "murmure", "settings"
pub mod doc_type {
    use super::*;

    /// Writes the document type as its lowercase name.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<S>(doc_type: &DocType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(doc_type.as_str())
    }

    /// Reads the document type from its lowercase name.
    ///
    /// # Errors
    ///
    /// A value that is not a string, or a name no document type has.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<DocType, D::Error>
    where
        D: Deserializer<'de>,
    {
        const WORDS: &[&str] = &["prose", "trame", "toile", "murmure", "settings"];
        read_word(deserializer, WORDS, |word| word.parse().ok())
    }
}

// Role adapter: "owner", "editor", "viewer", "admin", "member"
pub mod role {
    use super::*;

    /// Writes the role as its lowercase name.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<S>(role: &Role, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(format!("{}", role).to_lowercase().as_str())
    }

    /// Reads the role from its name.
    ///
    /// # Errors
    ///
    /// A value that is not a string, or a name no role has.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Role, D::Error>
    where
        D: Deserializer<'de>,
    {
        const WORDS: &[&str] = &["owner", "editor", "viewer", "admin", "member"];
        read_word(deserializer, WORDS, |word| word.parse().ok())
    }
}

// ChannelDocumentKind adapter: "meta", "segment", "amendments", "thread"
pub mod channel_document_kind {
    use super::*;

    /// Writes the kind as its lowercase name.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<S>(kind: &ChannelDocumentKind, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(kind.as_str())
    }

    /// Reads the kind from its lowercase name.
    ///
    /// # Errors
    ///
    /// A value that is not a string, or a name no kind has.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<ChannelDocumentKind, D::Error>
    where
        D: Deserializer<'de>,
    {
        const WORDS: &[&str] = &["meta", "segment", "amendments", "thread"];
        read_word(deserializer, WORDS, ChannelDocumentKind::parse)
    }
}

// Visibility adapter: the JSON flag `true` for a public channel, `false` for
// a private one. The relay HTTP API keeps the flag it always carried.
pub mod visibility_flag {
    use super::*;
    use serde::Serialize;

    /// Writes the visibility as the flag of a public channel.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<S>(visibility: &Visibility, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (*visibility == Visibility::Public).serialize(serializer)
    }

    /// Reads the visibility from the flag of a public channel.
    ///
    /// # Errors
    ///
    /// A value that is not a JSON boolean.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Visibility, D::Error>
    where
        D: Deserializer<'de>,
    {
        let public = bool::deserialize(deserializer)?;
        Ok(if public {
            Visibility::Public
        } else {
            Visibility::Private
        })
    }
}

// DocumentScope adapter: the JSON flag `true` for a Murmure space. A body
// that leaves the flag out creates a document of its own.

pub mod space_flag {
    use super::*;
    use serde::Serialize;

    /// Whether a body leaves the flag out: the relay reads it as false.
    #[must_use]
    pub fn is_personal(scope: &DocumentScope) -> bool {
        *scope == DocumentScope::Personal
    }

    /// Writes the scope as the flag of a space.
    ///
    /// # Errors
    ///
    /// Whatever `serializer` refuses.
    pub fn serialize<S>(scope: &DocumentScope, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (*scope == DocumentScope::Space).serialize(serializer)
    }

    /// Reads the scope from the flag of a space.
    ///
    /// # Errors
    ///
    /// A value that is not a JSON boolean.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<DocumentScope, D::Error>
    where
        D: Deserializer<'de>,
    {
        let space = bool::deserialize(deserializer)?;
        Ok(if space {
            DocumentScope::Space
        } else {
            DocumentScope::Personal
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Words {
        #[serde(with = "doc_type")]
        _doc_type: DocType,
        #[serde(with = "role")]
        _role: Role,
        #[serde(with = "channel_document_kind")]
        _kind: ChannelDocumentKind,
    }

    #[derive(Debug, serde::Serialize, Deserialize, PartialEq)]
    struct Listed {
        #[serde(with = "visibility_flag")]
        public: Visibility,
    }

    /// The visibility travels as the JSON flag of a public channel, both ways.
    #[test]
    fn a_visibility_reads_and_writes_as_the_public_flag() {
        for (visibility, flag) in [(Visibility::Public, true), (Visibility::Private, false)] {
            let listed = Listed { public: visibility };
            let json = serde_json::to_value(&listed).unwrap();
            assert_eq!(json, serde_json::json!({ "public": flag }));
            assert_eq!(serde_json::from_value::<Listed>(json).unwrap(), listed);
        }
    }

    /// Holds `SYNC-152`: a word no member of its set carries is refused as
    /// serde's unknown variant, in each adapter, and the error names it.
    #[test]
    fn an_unknown_word_reads_as_an_unknown_variant_in_every_set() {
        let body = |doc_type: &str, role: &str, kind: &str| serde_json::json!({ "_doc_type": doc_type, "_role": role, "_kind": kind });
        assert!(serde_json::from_value::<Words>(body("toile", "editor", "thread")).is_ok());
        for (bad, word) in [
            (body("sheet", "editor", "thread"), "sheet"),
            (body("toile", "auditor", "thread"), "auditor"),
            (body("toile", "editor", "poll"), "poll"),
        ] {
            let error = serde_json::from_value::<Words>(bad)
                .unwrap_err()
                .to_string();
            assert!(
                error.starts_with(&format!("unknown variant `{word}`")),
                "{error}"
            );
        }
    }
}
