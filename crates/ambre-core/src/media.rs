//! The values that name and measure a media blob.
//!
//! Each that travels reads and writes the bare string or integer the relay
//! exchanges, and changes no byte on the wire.
//! [`MediaAddress`] never travels; its hex spelling does.

use serde::{Deserialize, Serialize};

use crate::count::{CountColumn, CountKind, IndexedAs};
use crate::fixed_bytes::{FixedBytes, FixedRole};
use crate::text_offset::Byte;
use crate::{Displayed, Rule, Validated};

/// The alternative text of a picture: what a reader without the picture
/// reads in its place. `Validated<AltText>`.
pub enum AltText {}

impl crate::Free for AltText {}

impl crate::Displayed for AltText {}

/// The caption under a picture, empty when it has none.
/// `Validated<Caption>`.
pub enum Caption {}

impl crate::Free for Caption {}

impl crate::Displayed for Caption {}

/// The rule of the content address of a media blob, which the relay stores
/// it under (`ambre-spec-media.md`). A `Validated<ContentHash>` holds one, a
/// distinct type, so an element id or a URL never takes its place.
///
/// An address is the lowercase hex spelling of a [`MediaAddress`]: sixty-four
/// characters. [`Validated::parse`] and [`FixedBytes::content_hash`] are the
/// only ways in. Serde reads through the same check, so a peer, the relay or
/// the store never hands over another text. The serde form is the bare
/// string, as before the check.
pub enum ContentHash {}

/// A text that spells no content address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a content hash is sixty-four lowercase hex characters")]
pub struct ContentHashError;

/// The one check of a hash, which serde reads through too: sixty-four
/// lowercase hex characters, the spelling of a [`MediaAddress`]. The text is
/// kept, never copied.
impl Rule for ContentHash {
    type Error = ContentHashError;

    const LOGGED_AS: Option<&'static str> = Some("ContentHash");

    fn admit(text: String) -> Result<String, ContentHashError> {
        let is_address = text.len() == 2 * MEDIA_ADDRESS_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if is_address {
            Ok(text)
        } else {
            Err(ContentHashError)
        }
    }
}

/// The relay names the blob by its hash, so a route and a store key write it.
impl Displayed for ContentHash {}

/// The width of a [`MediaAddress`], in bytes.
pub const MEDIA_ADDRESS_LEN: usize = 32;

/// The role of the content address of a media blob as bytes:
/// `FixedBytes<MediaAddress>`. It is the keyed hash of its plaintext under the
/// document key. [`ContentHash`] is its hex spelling, the name the relay
/// stores the blob under. A distinct role, so a commitment tag or a digest of
/// the same width never takes its place.
pub enum MediaAddress {}

impl FixedRole for MediaAddress {
    const NAME: &'static str = "MediaAddress";
    type Array = [u8; MEDIA_ADDRESS_LEN];
}

impl FixedBytes<MediaAddress> {
    /// The hex spelling the relay names the blob by.
    #[must_use]
    pub fn content_hash(&self) -> Validated<ContentHash> {
        Validated::admitted(self.to_hex())
    }

    /// Reads an address back from its hex spelling. A [`ContentHash`] holds
    /// sixty-four lowercase hex characters by construction, so every hash
    /// spells exactly one address.
    #[must_use]
    pub fn of_hash(hash: &Validated<ContentHash>) -> Self {
        let mut bytes = [0u8; MEDIA_ADDRESS_LEN];
        // The decode fails only on a wrong length or a non-hex byte, and the
        // check of `ContentHash` refuses both before a hash exists.
        let decoded = hex::decode_to_slice(hash.as_str(), &mut bytes);
        debug_assert!(decoded.is_ok(), "a ContentHash spells an address");
        Self::from_array(bytes)
    }
}

/// The media type of a file, as the browser named it, such as `image/png`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaType(String);

impl MediaType {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

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

/// The marker of a size in bytes: `Count<Octets>`, the size of a file, a
/// blob or a buffer. Named apart from the [`crate::text_offset::Byte`] axis,
/// which a size in bytes reads its length on.
///
/// A file may pass 4 GiB, so the count is 64 bits wide, while an index
/// length is 32: [`crate::Count::saturating_len`] holds the gap. Serde writes the
/// bare `u64` the hand-written `ByteSize` wrote, and `Debug` keeps its name.
pub enum Octets {}

impl CountKind for Octets {
    type Repr = u64;
    const NAME: &'static str = "ByteSize";
}

/// The relay keeps a size in a Postgres `BIGINT`.
impl CountColumn for Octets {}

/// A buffer is reserved and sliced by its size.
impl IndexedAs for Octets {
    type Axis = Byte;
}

#[cfg(test)]
mod tests {
    use super::{ContentHash, ContentHashError, FixedBytes, MediaAddress, Octets};
    use crate::Count;
    use crate::Validated;
    use crate::index::Len;
    use crate::span::StoredAsI64;
    use crate::text_offset::Byte;

    /// The hand-written `ByteSize` a `Count<Octets>` replaced: a transparent
    /// `u64`, as it derived its serde.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct ByteSize(u64);

    /// A size writes the bytes `ByteSize` wrote, in postcard and in JSON,
    /// reads them back, and prints the old name.
    #[test]
    fn a_size_writes_the_bytes_of_the_type_it_replaced() {
        for n in [0, 7, 300, u64::from(u32::MAX) + 1, u64::MAX] {
            let size = Count::<Octets>::new(n);
            let old = ByteSize(n);
            let bytes = postcard::to_allocvec(&size).unwrap();
            assert_eq!(bytes, postcard::to_allocvec(&old).unwrap());
            assert_eq!(postcard::from_bytes::<Count<Octets>>(&bytes).unwrap(), size);
            let json = serde_json::to_string(&size).unwrap();
            assert_eq!(json, serde_json::to_string(&old).unwrap());
            assert_eq!(serde_json::from_str::<Count<Octets>>(&json).unwrap(), size);
        }
        assert_eq!(format!("{:?}", Count::<Octets>::new(4)), "ByteSize(4)");
        assert_eq!(Count::<Octets>::from_i64(-1), Count::<Octets>::ZERO);
        assert_eq!(Count::<Octets>::new(u64::MAX).to_i64_saturating(), i64::MAX);
    }

    /// A size reads as a length up to `u32::MAX`, and holds there past it:
    /// it never truncates to the low 32 bits.
    #[test]
    fn a_size_past_an_index_length_saturates_at_its_top() {
        let len = |n: u64| Count::<Octets>::new(n).saturating_len();
        assert_eq!(len(7), Len::<Byte>::new(7));
        let top = u64::from(u32::MAX);
        assert_eq!(len(top), Len::<Byte>::new(u32::MAX));
        assert_eq!(len(top + 1), Len::<Byte>::new(u32::MAX));
        assert_eq!(len(top + 8), Len::<Byte>::new(u32::MAX));
        assert_eq!(len(u64::MAX), Len::<Byte>::new(u32::MAX));
    }

    #[test]
    fn a_media_value_serializes_as_its_bare_inner_value() {
        let hex = "ab".repeat(32);
        let hash = Validated::<ContentHash>::parse(hex.clone()).unwrap();
        assert_eq!(format!("{hash:?}"), format!("ContentHash({hex:?})"));
        assert_eq!(hash.to_string(), hex);
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
        let address = FixedBytes::<MediaAddress>::from_array([0xab; 32]);
        let hash = address.content_hash();
        assert_eq!(hash.as_str(), "ab".repeat(32));
        assert_eq!(FixedBytes::<MediaAddress>::of_hash(&hash), address);
    }

    /// The constructor and serde refuse an uppercase spelling and a wrong
    /// length alike, so no reader takes a text that is no address.
    #[test]
    fn a_hash_takes_sixty_four_lowercase_hex_characters_alone() {
        let hex = "0a".repeat(32);
        assert_eq!(
            Validated::<ContentHash>::parse(hex.clone()).map(Validated::into_string),
            Ok(hex.clone())
        );
        let refused = [
            hex.to_uppercase(),
            hex[1..].to_owned(),
            format!("{hex}0"),
            "g".repeat(64),
            String::new(),
        ];
        for text in refused {
            assert_eq!(
                Validated::<ContentHash>::parse(text.clone()),
                Err(ContentHashError),
                "{text}"
            );
            let json = serde_json::to_string(&text).unwrap();
            assert!(
                serde_json::from_str::<Validated<ContentHash>>(&json).is_err(),
                "{text}"
            );
            let bytes = postcard::to_allocvec(&text).unwrap();
            assert!(
                postcard::from_bytes::<Validated<ContentHash>>(&bytes).is_err(),
                "{text}"
            );
        }
        let json = serde_json::to_string(&hex).unwrap();
        assert_eq!(
            serde_json::from_str::<Validated<ContentHash>>(&json)
                .unwrap()
                .as_str(),
            hex
        );
    }
}
