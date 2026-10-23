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

impl From<&str> for MediaType {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for MediaType {
    fn from(text: String) -> Self {
        Self(text)
    }
}

/// A size in bytes.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ByteSize(pub u64);

impl ByteSize {
    /// The size of a buffer `len` bytes long, saturating at `u64::MAX`.
    #[must_use]
    pub fn of_len(len: usize) -> Self {
        Self(u64::try_from(len).unwrap_or(u64::MAX))
    }

    /// The size as a buffer length, saturating at `usize::MAX`.
    #[must_use]
    pub fn as_len(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }

    /// The two sizes together, saturating at `u64::MAX` bytes.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// The bytes past `other`, or zero when `other` is larger.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// The size as a Postgres `BIGINT` stores it, saturating at `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }

    /// A size a Postgres `BIGINT` stored. A negative value, which no
    /// `octet_length` returns, reads as zero.
    pub fn from_i64(stored: i64) -> Self {
        Self(u64::try_from(stored).unwrap_or(0))
    }
}

impl std::iter::Sum for ByteSize {
    fn sum<I: Iterator<Item = Self>>(sizes: I) -> Self {
        sizes.fold(Self(0), Self::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteSize, ContentHash};

    #[test]
    fn a_media_value_serializes_as_its_bare_inner_value() {
        assert_eq!(serde_json::to_string(&ByteSize(7)).unwrap(), "7");
        assert_eq!(ByteSize::from_i64(-1), ByteSize(0));
        assert_eq!(ByteSize(u64::MAX).to_i64_saturating(), i64::MAX);
        assert_eq!(
            serde_json::to_string(&ContentHash("ab".to_owned())).unwrap(),
            "\"ab\""
        );
    }
}
