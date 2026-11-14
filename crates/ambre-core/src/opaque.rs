//! Text that crosses a thread, a wire or a store, and that a log never
//! prints. An ICE candidate, a session description, a draft row and a stored
//! mark value are such texts.
//!
//! Every such text is one generic type, [`Opaque<K>`], over a marker `K`
//! that names its kind. The code that carries the text reads none of it, so
//! no rule checks it at the door. The marker still keeps two kinds apart: an
//! ICE candidate never stands in for a session description, though both are
//! a browser's text.
//!
//! ```compile_fail,E0308
//! use ambre_core::{Opaque, OpaqueKind, OpaqueLength};
//! enum Candidates {}
//! impl OpaqueKind for Candidates {
//!     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("IceCandidate");
//!     const LENGTH: OpaqueLength = OpaqueLength::Bytes;
//! }
//! enum Descriptions {}
//! impl OpaqueKind for Descriptions {
//!     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("SessionDescription");
//!     const LENGTH: OpaqueLength = OpaqueLength::Bytes;
//! }
//! let _: Opaque<Descriptions> = Opaque::<Candidates>::new("candidate:1");
//! ```
//!
//! A plain [`crate::Redacted`] does not do: it names no kind, so two kinds
//! mix in it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Confidential, Plaintext, RedactedChars, Secret};

/// The unit in which `Debug` prints the length of an opaque text.
///
/// Each kind keeps the spelling its hand-written type printed, so a log line
/// and a test read the same after the fold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpaqueLength {
    /// `NAME(N bytes)`: the length in UTF-8 bytes.
    Bytes,
    /// `NAME(N chars)`: the length in code points.
    Chars,
}

/// The marker of one kind of opaque text.
///
/// The marker is an uninhabited type: it names what the text is and holds
/// no value. The trait is open, because each kind lives in the crate that
/// carries it.
pub trait OpaqueKind: 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: crate::StaticKey<crate::DebugNames>;

    /// The unit `Debug` prints the length in.
    const LENGTH: OpaqueLength;
}

/// An opaque text that crosses a wire as the bare string it is.
///
/// A capability that each kind opts in to. A kind without it has no serde
/// form, so it never reaches a frame by mistake. A draft row leaves the page
/// sealed, through [`SealedAtRest`] alone.
///
/// ```compile_fail,E0277
/// use ambre_core::{Opaque, OpaqueKind, OpaqueLength};
/// enum Rows {}
/// impl OpaqueKind for Rows {
///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("DraftRow");
///     const LENGTH: OpaqueLength = OpaqueLength::Bytes;
/// }
/// let _ = serde_json::to_string(&Opaque::<Rows>::new("typed"));
/// ```
pub trait WireOpaque: OpaqueKind {}

/// An opaque text that a store keeps sealed: it leaves as a plaintext wiped
/// on drop, and comes back from the bytes a seal opens.
///
/// A capability that each kind opts in to. A text no store seals has no way
/// out as bytes.
pub trait SealedAtRest: OpaqueKind {}

/// A text of the kind `K` that the code carries and reads none of.
///
/// The field is private. [`Opaque::new`] is the one door, and it is public:
/// no kind checks its text, so a closed door would guard nothing. The text
/// leaves through [`Opaque::as_str`], at the one site that reads it. `Debug`
/// prints the kind and the length alone.
pub struct Opaque<K: OpaqueKind> {
    text: String,
    kind: PhantomData<fn() -> K>,
}

impl<K: OpaqueKind> Opaque<K> {
    /// Wraps `text`. The constructor checks nothing.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: PhantomData,
        }
    }

    /// The text, for the one site that reads it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl<K: SealedAtRest> Opaque<K> {
    /// The text as the plaintext a store seals, wiped as it drops. The bytes
    /// move, so no copy of the text is left behind.
    #[must_use]
    pub fn into_plaintext(self) -> Confidential<Plaintext> {
        Confidential::<Plaintext>::new(self.text.into_bytes())
    }

    /// The text an opened seal holds, `None` for bytes that are not UTF-8.
    /// Refused bytes are a plaintext too, so they are wiped as they drop.
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        String::from_utf8(bytes)
            .map_err(|refused| Secret::wipe(refused.into_bytes()))
            .ok()
            .map(Self::new)
    }
}

impl<K: OpaqueKind> Clone for Opaque<K> {
    fn clone(&self) -> Self {
        Self::new(self.text.clone())
    }
}

impl<K: OpaqueKind> PartialEq for Opaque<K> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<K: OpaqueKind> Eq for Opaque<K> {}

/// Hashes the text, so an opaque text keys a map. A map reads no text: it
/// compares texts, as `Eq` does. No bound lands on the marker.
impl<K: OpaqueKind> Hash for Opaque<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

/// The empty text.
impl<K: OpaqueKind> Default for Opaque<K> {
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// `NAME(N bytes)` or `NAME(N chars)`, as the hand-written type printed it.
/// A log carries none of the text.
impl<K: OpaqueKind> fmt::Debug for Opaque<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match K::LENGTH {
            OpaqueLength::Bytes => write!(f, "{}({} bytes)", K::NAME.as_str(), self.text.len()),
            OpaqueLength::Chars => f
                .debug_tuple(K::NAME.as_str())
                .field(&RedactedChars::of(&self.text))
                .finish(),
        }
    }
}

/// The bare string, so a frame keeps the bytes of the type it replaced.
impl<K: WireOpaque> Serialize for Opaque<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.text.serialize(serializer)
    }
}

/// The bare string. Any text decodes: no kind checks it.
impl<'de, K: WireOpaque> Deserialize<'de> for Opaque<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::{Opaque, OpaqueKind, OpaqueLength, SealedAtRest, WireOpaque};

    enum Candidates {}
    impl OpaqueKind for Candidates {
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("IceCandidate");
        const LENGTH: OpaqueLength = OpaqueLength::Bytes;
    }
    impl WireOpaque for Candidates {}

    enum Values {}
    impl OpaqueKind for Values {
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("StoredMarkValue");
        const LENGTH: OpaqueLength = OpaqueLength::Chars;
    }

    enum Rows {}
    impl OpaqueKind for Rows {
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("DraftRow");
        const LENGTH: OpaqueLength = OpaqueLength::Bytes;
    }
    impl SealedAtRest for Rows {}

    #[test]
    fn debug_prints_the_kind_and_the_length_in_its_unit() {
        let text = "h\u{e9}llo";
        assert_eq!(
            format!("{:?}", Opaque::<Candidates>::new(text)),
            "IceCandidate(6 bytes)"
        );
        assert_eq!(
            format!("{:?}", Opaque::<Values>::new(text)),
            "StoredMarkValue(5 chars)"
        );
    }

    /// The serde form is the bare string, in a binary frame and in JSON, so
    /// the bytes of the hand-written type stay.
    #[test]
    fn a_wire_text_round_trips_as_the_bare_string() {
        let text = "candidate:1 1 udp";
        let opaque = Opaque::<Candidates>::new(text);
        let frame = postcard::to_allocvec(&opaque).unwrap();
        assert_eq!(frame, postcard::to_allocvec(&text.to_owned()).unwrap());
        assert_eq!(
            postcard::from_bytes::<Opaque<Candidates>>(&frame).unwrap(),
            opaque
        );
        let json = serde_json::to_string(&opaque).unwrap();
        assert_eq!(json, "\"candidate:1 1 udp\"");
        assert_eq!(
            serde_json::from_str::<Opaque<Candidates>>(&json).unwrap(),
            opaque
        );
    }

    #[test]
    fn a_sealed_text_comes_back_from_its_bytes_and_refuses_other_bytes() {
        let row = Opaque::<Rows>::new("{\"text\":\"hi\"}");
        let bytes = row.as_str().as_bytes().to_vec();
        assert_eq!(Opaque::<Rows>::from_bytes(bytes), Some(row.clone()));
        assert_eq!(Opaque::<Rows>::from_bytes(vec![0xff, 0xfe]), None);
        let plaintext = row.into_plaintext();
        plaintext.with_plaintext(|bytes| assert_eq!(bytes, b"{\"text\":\"hi\"}"));
    }

    /// Two equal texts hash alike, so an opaque text keys a map, and a
    /// marker that implements no `Hash` still gets it.
    #[test]
    fn an_opaque_text_keys_a_map_by_its_text() {
        let mut seen = std::collections::HashMap::new();
        seen.insert(Opaque::<Candidates>::new("candidate:1"), 1);
        seen.insert(Opaque::<Candidates>::new("candidate:2"), 2);
        assert_eq!(
            seen.get(&Opaque::<Candidates>::new("candidate:1")),
            Some(&1)
        );
        assert_eq!(seen.get(&Opaque::<Candidates>::new("candidate:3")), None);
        assert_eq!(
            seen.insert(Opaque::<Candidates>::new("candidate:2"), 4),
            Some(2)
        );
    }

    #[test]
    fn the_default_is_the_empty_text() {
        assert_eq!(Opaque::<Values>::default().as_str(), "");
    }
}
