//! The values that name and measure a media blob.
//!
//! Each that travels reads and writes the bare string or integer the relay
//! exchanges, and changes no byte on the wire.
//! [`MediaAddress`] never travels; its hex spelling does.

use serde::{Deserialize, Serialize};

/// The content address of a media blob, which the relay stores it under
/// (`ambre-spec-media.md`). A distinct type, so an element id or a URL never
/// takes its place.
///
/// An address is the lowercase hex spelling of a [`MediaAddress`]: sixty-four
/// characters. [`ContentHash::parse`] and [`MediaAddress::content_hash`] are
/// the only ways in. Serde reads through the same check, so a peer, the relay
/// or the store never hands over another text. The serde form is the bare
/// string, as before the check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct ContentHash(String);

/// A text that spells no content address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a content hash is sixty-four lowercase hex characters")]
pub struct ContentHashError;

impl ContentHash {
    /// The address `word` spells, or `None` for anything else.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::try_from(word.to_owned()).ok()
    }

    /// The address, as the relay names it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The width of a [`MediaAddress`], in bytes.
pub const MEDIA_ADDRESS_LEN: usize = 32;

/// The content address of a media blob as bytes: the keyed hash of its
/// plaintext under the document key. [`ContentHash`] is its hex spelling, the
/// name the relay stores the blob under. A distinct type, so a commitment tag
/// or a digest of the same width never takes its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaAddress([u8; MEDIA_ADDRESS_LEN]);

impl MediaAddress {
    /// Wraps the bytes a keyed hash produced.
    #[must_use]
    pub const fn from_array(bytes: [u8; MEDIA_ADDRESS_LEN]) -> Self {
        Self(bytes)
    }

    /// The bytes, for the additional data a blob is sealed under.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; MEDIA_ADDRESS_LEN] {
        &self.0
    }

    /// The hex spelling the relay names the blob by.
    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        ContentHash(hex::encode(self.0))
    }

    /// Reads an address back from its hex spelling. A [`ContentHash`] holds
    /// sixty-four lowercase hex characters by construction, so every hash
    /// spells exactly one address.
    #[must_use]
    pub fn of_hash(hash: &ContentHash) -> Self {
        let mut bytes = [0u8; MEDIA_ADDRESS_LEN];
        // The decode fails only on a wrong length or a non-hex byte, and the
        // check of `ContentHash` refuses both before a hash exists.
        let decoded = hex::decode_to_slice(&hash.0, &mut bytes);
        debug_assert!(decoded.is_ok(), "a ContentHash spells an address");
        Self(bytes)
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

/// The one check of a hash, which serde reads through too: sixty-four
/// lowercase hex characters, the spelling of a [`MediaAddress`]. The text is
/// kept, never copied.
impl TryFrom<String> for ContentHash {
    type Error = ContentHashError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let is_address = text.len() == 2 * MEDIA_ADDRESS_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if is_address {
            Ok(Self(text))
        } else {
            Err(ContentHashError)
        }
    }
}

/// The bare string, as `#[serde(transparent)]` wrote it.
impl Serialize for ContentHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
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

/// A media type a client rules on by name. It is a picture the client decodes
/// or opens, or the plain stream of bytes a file falls back to. Every other
/// type is only a [`MediaType`] the client carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownMediaType {
    Png,
    Jpeg,
    Gif,
    Webp,
    Avif,
    /// The type of a file the browser names no type for.
    OctetStream,
}

impl KnownMediaType {
    /// Every known type.
    pub const ALL: [Self; 6] = [
        Self::Png,
        Self::Jpeg,
        Self::Gif,
        Self::Webp,
        Self::Avif,
        Self::OctetStream,
    ];

    /// The type, as a browser spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::OctetStream => "application/octet-stream",
        }
    }

    /// The known type `media_type` names, blanks around it dropped and case
    /// folded. `None` for any other type.
    #[must_use]
    pub fn of(media_type: &MediaType) -> Option<Self> {
        let named = media_type.as_str().trim();
        Self::ALL
            .into_iter()
            .find(|known| named.eq_ignore_ascii_case(known.as_str()))
    }

    /// The type as a [`MediaType`] carries it.
    #[must_use]
    pub fn media_type(self) -> MediaType {
        MediaType::from(self.as_str())
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
    use super::{ByteSize, ContentHash, MediaAddress};

    #[test]
    fn a_media_value_serializes_as_its_bare_inner_value() {
        assert_eq!(serde_json::to_string(&ByteSize(7)).unwrap(), "7");
        assert_eq!(ByteSize::from_i64(-1), ByteSize(0));
        assert_eq!(ByteSize(u64::MAX).to_i64_saturating(), i64::MAX);
        let hex = "ab".repeat(32);
        let hash = ContentHash::parse(&hex).unwrap();
        assert_eq!(
            serde_json::to_string(&hash).unwrap(),
            serde_json::to_string(&hex).unwrap()
        );
        assert_eq!(
            postcard::to_allocvec(&hash).unwrap(),
            postcard::to_allocvec(&hex).unwrap()
        );
    }

    #[test]
    fn an_address_reads_back_from_its_hex_spelling() {
        let address = MediaAddress::from_array([0xab; 32]);
        let hash = address.content_hash();
        assert_eq!(hash.as_str(), "ab".repeat(32));
        assert_eq!(MediaAddress::of_hash(&hash), address);
    }

    /// The constructor and serde refuse an uppercase spelling and a wrong
    /// length alike, so no reader takes a text that is no address.
    #[test]
    fn a_hash_takes_sixty_four_lowercase_hex_characters_alone() {
        let hex = "0a".repeat(32);
        assert_eq!(
            ContentHash::parse(&hex).map(|hash| hash.as_str().to_owned()),
            Some(hex.clone())
        );
        let refused = [
            hex.to_uppercase(),
            hex[1..].to_owned(),
            format!("{hex}0"),
            "g".repeat(64),
            String::new(),
        ];
        for text in refused {
            assert_eq!(ContentHash::parse(&text), None, "{text}");
            let json = serde_json::to_string(&text).unwrap();
            assert!(
                serde_json::from_str::<ContentHash>(&json).is_err(),
                "{text}"
            );
            let bytes = postcard::to_allocvec(&text).unwrap();
            assert!(
                postcard::from_bytes::<ContentHash>(&bytes).is_err(),
                "{text}"
            );
        }
        let json = serde_json::to_string(&hex).unwrap();
        assert_eq!(
            serde_json::from_str::<ContentHash>(&json).unwrap().as_str(),
            hex
        );
    }
}
