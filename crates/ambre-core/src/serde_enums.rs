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

use crate::{ChannelDocumentKind, DocType, Role};

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

// DocType adapter: "prose", "trame", "toile", "murmure"
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
        const WORDS: &[&str] = &["prose", "trame", "toile", "murmure"];
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
