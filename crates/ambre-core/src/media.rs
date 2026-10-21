//! The values that name and measure a media blob.
//!
//! Each is `#[serde(transparent)]`: it reads and writes the bare string or
//! integer the relay exchanges, so the type changes no byte on the wire.

use serde::{Deserialize, Serialize};

/// The content address of a media blob, which the relay stores it under
/// (`ambre-spec-media.md`). A distinct type, so an element id or a URL never
/// takes its place.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// The address, as the relay names it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::borrow::Borrow<str> for ContentHash {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ContentHash {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for ContentHash {
    fn from(text: String) -> Self {
        Self(text)
    }
}

/// The media type of a file, as the browser named it, such as `image/png`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaType(pub String);

impl MediaType {
    /// The type, as the browser spelled it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A size in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ByteSize(pub u64);

#[cfg(test)]
mod tests {
    use super::{ByteSize, ContentHash};

    #[test]
    fn a_media_value_serializes_as_its_bare_inner_value() {
        assert_eq!(serde_json::to_string(&ByteSize(7)).unwrap(), "7");
        assert_eq!(
            serde_json::to_string(&ContentHash("ab".to_owned())).unwrap(),
            "\"ab\""
        );
    }
}
