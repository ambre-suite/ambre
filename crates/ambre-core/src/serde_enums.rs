//! Serde adapters for enum serialization as lowercase strings.
//!
//! These adapters serialize enums as their lowercase string representations for
//! JSON APIs. For example, `DocType::Prose` serializes as "prose" and deserializes
//! from "prose", "Prose", or "PROSE" (case-insensitive).

use serde::{Deserialize, Deserializer, Serializer};

use crate::{ChannelDocumentKind, DocType, Role};

// DocType adapter: "prose", "trame", "toile", "murmure"
pub mod doc_type {
    use super::*;

    pub fn serialize<S>(doc_type: &DocType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let lowercase = doc_type.name().to_lowercase();
        serializer.serialize_str(&lowercase)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DocType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "prose" => Ok(DocType::Prose),
            "trame" => Ok(DocType::Trame),
            "toile" => Ok(DocType::Toile),
            "murmure" => Ok(DocType::Murmure),
            _ => Err(serde::de::Error::custom(format!("unknown doc_type: {}", s))),
        }
    }
}

// Role adapter: "owner", "editor", "viewer", "admin", "member"
pub mod role {
    use super::*;

    pub fn serialize<S>(role: &Role, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(format!("{}", role).to_lowercase().as_str())
    }

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

    pub fn serialize<S>(kind: &ChannelDocumentKind, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(kind.as_str())
    }

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
