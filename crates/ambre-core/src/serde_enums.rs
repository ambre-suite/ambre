//! Serde adapters for enum serialization as lowercase strings.
//!
//! These adapters serialize enums as their lowercase string representations for
//! JSON APIs. For example, `DocType::Prose` serializes as "prose" and
//! deserializes from "prose" alone. Each closed set has one parser, which
//! refuses any other word when the body is read.

use serde::{Deserialize, Deserializer, Serializer};

use crate::{ChannelDocumentKind, DocType, Role};

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
        let s = String::deserialize(deserializer)?;
        s.parse::<DocType>().map_err(serde::de::Error::custom)
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
        let s = String::deserialize(deserializer)?;
        s.parse::<Role>().map_err(serde::de::Error::custom)
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
        let s = String::deserialize(deserializer)?;
        ChannelDocumentKind::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown channel_document_kind: {}", s))
        })
    }
}
