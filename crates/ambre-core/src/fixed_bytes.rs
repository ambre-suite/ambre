//! Fixed-width byte values, one generic type over a role marker.
//!
//! A nonce, a signature, a public key, a digest, a key identifier and a salt
//! are each a fixed number of bytes. A bare array lets one stand for another
//! of the same width. [`FixedBytes`] keeps them apart: `FixedBytes<Nonce>` and
//! `FixedBytes<Signature>` are two types. The marker, an empty `enum`, states
//! the width of its role once, as its [`FixedRole::Array`].
//!
//! One serde form serves every role, written here by hand:
//!
//! * a binary format, such as `postcard`, reads and writes the bytes in order,
//!   with no length prefix: the type fixes the length (`BUS-031`);
//! * a readable format, such as JSON, spells the value as lowercase
//!   hexadecimal, and refuses any other width when it reads (`SRV-242`).
//!
//! So no role chooses its wire form by an attribute, and a new role cannot
//! pick another form by mistake.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Count, Octets};

mod sealed {
    /// Closes [`super::ByteArray`] to the byte arrays.
    pub trait Sealed {}
    impl<const N: usize> Sealed for [u8; N] {}

    /// Closes [`super::FixedRole`] to the roles of this crate. A role
    /// downstream could otherwise give any secret a serde form and a value
    /// that never wipes itself.
    pub trait SealedRole {}
    impl SealedRole for crate::PublicKey {}
    impl SealedRole for crate::ExchangePublicKey {}
    impl SealedRole for crate::Nonce {}
    impl SealedRole for crate::CommitmentTag {}
    impl SealedRole for crate::Signature {}
    impl SealedRole for crate::ChainHash {}
    impl SealedRole for crate::EntryDigest {}
    impl SealedRole for crate::SnapshotDigest {}
    impl SealedRole for crate::ObjectChecksum {}
    impl SealedRole for crate::InviterPin {}
    impl SealedRole for crate::KeyId {}
    impl SealedRole for crate::KeyIdentifierBytes {}
    impl SealedRole for crate::SlotSenderBytes {}
    impl SealedRole for crate::SessionNonce {}
    impl SealedRole for crate::FileSalt {}
    impl SealedRole for crate::FileDigest {}
    impl SealedRole for crate::ModuleDigest {}
    impl SealedRole for crate::MediaAddress {}
    impl SealedRole for crate::FormatMagic {}
    impl SealedRole for crate::MasterKeySalt {}
    impl SealedRole for crate::RoomPseudonym {}
    impl SealedRole for crate::ParticipantPseudonym {}
    impl SealedRole for crate::crdt::TextDigest {}
    impl SealedRole for crate::MemberPseudonym {}
}

/// A byte array of any width: the storage of a [`FixedBytes`]. Closed to
/// `[u8; N]`, so a role states its width and nothing else.
pub trait ByteArray:
    sealed::Sealed + Copy + Eq + Ord + Hash + AsRef<[u8]> + AsMut<[u8]> + Send + Sync + Unpin + 'static
{
    /// The width, in bytes.
    const LEN: usize;

    /// The array of zero bytes, which a reader fills. The standard library
    /// gives `Default` up to 32 bytes alone, and a signature is 64.
    fn zeroed() -> Self;
}

impl<const N: usize> ByteArray for [u8; N] {
    const LEN: usize = N;

    fn zeroed() -> Self {
        [0; N]
    }
}

/// What the debug form of a role prints. Every role here is public, so the
/// choice is one of reading comfort, never of secrecy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugForm {
    /// Every byte, in hexadecimal.
    Hex,
    /// The first four bytes, in hexadecimal: enough to tell two keys apart in
    /// a log.
    HexPrefix,
    /// The width alone.
    Length,
}

/// The role of a fixed-width value: its name and its width.
///
/// A role is an empty `enum` that names a value no code builds. The width
/// sits in [`FixedRole::Array`], since a const generic parameter cannot be
/// read from an associated constant on stable Rust. The trait is sealed: every
/// role lives in this crate, and a review reads each one.
pub trait FixedRole: sealed::SealedRole + 'static {
    /// The name the debug form and an error print.
    const NAME: crate::StaticKey<crate::DebugNames>;
    /// What the debug form prints.
    const DEBUG: DebugForm = DebugForm::Hex;
    /// The storage, `[u8; N]` for a role of `N` bytes.
    type Array: ByteArray;
}

/// A fixed-width value of the role `R`.
///
/// The field is private: [`FixedBytes::from_array`] is the one way in, and
/// a slice or a text goes through a check of its width.
pub struct FixedBytes<R: FixedRole> {
    bytes: R::Array,
    role: PhantomData<fn() -> R>,
}

/// Bytes or text that spell no value of a role.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FixedBytesError {
    /// The bytes are not as long as the role.
    #[error("invalid length: got {0} bytes")]
    InvalidLength(usize),

    /// The text is not hexadecimal.
    #[error("invalid hex string: {0}")]
    InvalidHex(#[from] hex::FromHexError),
}

impl<R: FixedRole> FixedBytes<R> {
    /// Bytes of one value where a layout frames it.
    pub const SIZE: Count<Octets> = Count::new(R::Array::LEN as u64);

    /// Wraps the bytes.
    #[must_use]
    pub const fn from_array(bytes: R::Array) -> Self {
        Self {
            bytes,
            role: PhantomData,
        }
    }

    /// The bytes, as the array of the role.
    #[must_use]
    pub const fn as_array(&self) -> &R::Array {
        &self.bytes
    }

    /// The bytes, as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Reads a value from exactly its width of bytes.
    ///
    /// # Errors
    ///
    /// [`FixedBytesError::InvalidLength`], carrying the length it received.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, FixedBytesError> {
        let mut array = R::Array::zeroed();
        copy_exact(bytes, array.as_mut())?;
        Ok(Self::from_array(array))
    }

    /// Reads one value from the front of `bytes`, and returns the bytes after
    /// it. `None` when `bytes` is shorter than [`Self::SIZE`].
    #[must_use]
    pub fn split_from(bytes: &[u8]) -> Option<(Self, &[u8])> {
        let (head, rest) = bytes.split_at_checked(R::Array::LEN)?;
        Self::from_slice(head).ok().map(|value| (value, rest))
    }

    /// Appends the value to `out`, as [`Self::split_from`] reads it back.
    pub fn push_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_slice());
    }

    /// The value as lowercase hexadecimal, its form in a readable format.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_slice())
    }

    /// Reads a value from its hexadecimal form.
    ///
    /// # Errors
    ///
    /// [`FixedBytesError::InvalidHex`] on a non-hexadecimal digit, and
    /// [`FixedBytesError::InvalidLength`] on any width but the role's.
    pub fn from_hex(text: &str) -> Result<Self, FixedBytesError> {
        Self::from_slice(&hex::decode(text)?)
    }
}

impl<R: FixedRole<Array = [u8; 32]>> FixedBytes<R> {
    /// Reads a value of 32 bytes from its 64 hexadecimal digits, in a const
    /// context too. A build names a key this way, so a name that spells no
    /// key fails the build (`ZK-062`). `None` on a non-hexadecimal digit or on
    /// any other count of digits.
    #[must_use]
    pub const fn from_hex_const(text: &str) -> Option<Self> {
        // A macro and not a helper: a const fn over a bare byte would be one
        // more primitive site for one match.
        macro_rules! nibble {
            ($digit:expr) => {
                match $digit {
                    b'0'..=b'9' => $digit - b'0',
                    b'a'..=b'f' => $digit - b'a' + 10,
                    b'A'..=b'F' => $digit - b'A' + 10,
                    _ => return None,
                }
            };
        }
        let mut digits = text.as_bytes();
        if digits.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        let mut out: &mut [u8] = &mut bytes;
        while let ([high, low, rest @ ..], [slot, tail @ ..]) = (digits, out) {
            *slot = (nibble!(*high) << 4) | nibble!(*low);
            digits = rest;
            out = tail;
        }
        Some(Self::from_array(bytes))
    }
}

impl<R: FixedRole> Clone for FixedBytes<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: FixedRole> Copy for FixedBytes<R> {}

impl<R: FixedRole> PartialEq for FixedBytes<R> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<R: FixedRole> Eq for FixedBytes<R> {}

impl<R: FixedRole> PartialOrd for FixedBytes<R> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<R: FixedRole> Ord for FixedBytes<R> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bytes.cmp(&other.bytes)
    }
}

impl<R: FixedRole> Hash for FixedBytes<R> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl<R: FixedRole> AsRef<[u8]> for FixedBytes<R> {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<R: FixedRole> fmt::Debug for FixedBytes<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.as_slice();
        match R::DEBUG {
            DebugForm::Hex => write!(f, "{}({})", R::NAME.as_str(), hex::encode(bytes)),
            DebugForm::HexPrefix => {
                let prefix = bytes.get(..4).unwrap_or(bytes);
                write!(f, "{}(0x{}...)", R::NAME.as_str(), hex::encode(prefix))
            }
            DebugForm::Length => write!(f, "{}({} bytes)", R::NAME.as_str(), bytes.len()),
        }
    }
}

impl<R: FixedRole> Serialize for FixedBytes<R> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        write_bytes(self.as_slice(), serializer)
    }
}

/// Writes `bytes` in either form: the hexadecimal text of a readable format,
/// or a tuple of bytes in a binary one. No role is named, so every role
/// shares one copy of the write.
fn write_bytes<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
        return serializer.serialize_str(&hex::encode(bytes));
    }
    let mut tuple = serializer.serialize_tuple(bytes.len())?;
    for byte in bytes {
        tuple.serialize_element(byte)?;
    }
    tuple.end()
}

/// Copies `bytes` into `slots`, which must be as long.
///
/// # Errors
///
/// [`FixedBytesError::InvalidLength`], carrying the length of `bytes`.
fn copy_exact(bytes: &[u8], slots: &mut [u8]) -> Result<(), FixedBytesError> {
    if bytes.len() != slots.len() {
        return Err(FixedBytesError::InvalidLength(bytes.len()));
    }
    slots.copy_from_slice(bytes);
    Ok(())
}

/// Reads a [`FixedBytes`] into the bytes of its array. A readable format
/// holds its hexadecimal text, and a binary one its tuple of bytes.
///
/// No role is named in the type, so every role shares one copy of each read.
/// A visitor per role cost about 3 KB of the Sync worker
/// (`ambre-spec-budgets.md` section 5).
struct Slots<'a> {
    bytes: &'a mut [u8],
    name: crate::StaticKey<crate::DebugNames>,
}

/// The value a read expects, as the errors of serde name it.
struct Width {
    name: crate::StaticKey<crate::DebugNames>,
    len: usize,
}

impl de::Expected for Width {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a {} of {} bytes", self.name.as_str(), self.len)
    }
}

impl<'de> Visitor<'de> for Slots<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        de::Expected::fmt(&self.width(), f)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<(), E> {
        read_hex(text, self.bytes, self.name)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let width = self.width();
        read_tuple(&mut seq, self.bytes, &width)
    }
}

impl Slots<'_> {
    fn width(&self) -> Width {
        Width {
            name: self.name,
            len: self.bytes.len(),
        }
    }
}

/// Fills `slots` from the hexadecimal `text` of a value of the role `name`.
fn read_hex<E: de::Error>(
    text: &str,
    slots: &mut [u8],
    name: crate::StaticKey<crate::DebugNames>,
) -> Result<(), E> {
    hex::decode(text)
        .map_err(FixedBytesError::from)
        .and_then(|bytes| copy_exact(&bytes, slots))
        .map_err(|e| E::custom(format_args!("{}: {e}", name.as_str())))
}

/// Fills `slots` from the tuple of bytes of a binary format, one element per
/// slot. `expected` names the value in the error of a short tuple. Every
/// width shares one copy of the loop, [`crate::Uuid`] included.
pub(crate) fn read_tuple<'de, A: SeqAccess<'de>>(
    seq: &mut A,
    slots: &mut [u8],
    expected: &dyn de::Expected,
) -> Result<(), A::Error> {
    for (read, slot) in slots.iter_mut().enumerate() {
        *slot = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(read, expected))?;
    }
    Ok(())
}

impl<'de, R: FixedRole> Deserialize<'de> for FixedBytes<R> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut array = R::Array::zeroed();
        let slots = Slots {
            bytes: array.as_mut(),
            name: R::NAME,
        };
        if deserializer.is_human_readable() {
            deserializer.deserialize_str(slots)?;
        } else {
            deserializer.deserialize_tuple(R::Array::LEN, slots)?;
        }
        Ok(Self::from_array(array))
    }
}

#[cfg(test)]
mod tests {
    use super::{DebugForm, FixedBytes, FixedRole};
    use crate::{Count, Octets};

    /// `ZK-062`: the const read of a build constant agrees with the runtime
    /// read, in either case. It refuses a short text and a non-hex digit.
    #[test]
    fn the_const_hex_read_agrees_with_the_runtime_read() {
        let text = "927308f53d6765503d2b64c103ab5aa47206ab39502cfe8f98442fdb543100b7";
        let runtime = FixedBytes::<crate::PublicKey>::from_hex(text).expect("hex");
        assert_eq!(
            FixedBytes::<crate::PublicKey>::from_hex_const(text),
            Some(runtime)
        );
        assert_eq!(
            FixedBytes::<crate::PublicKey>::from_hex_const(&text.to_uppercase()),
            Some(runtime)
        );
        assert_eq!(
            FixedBytes::<crate::PublicKey>::from_hex_const(&text[2..]),
            None
        );
        assert_eq!(
            FixedBytes::<crate::PublicKey>::from_hex_const(&format!("{text}00")),
            None
        );
        assert_eq!(
            FixedBytes::<crate::PublicKey>::from_hex_const(&text.replacen('9', "g", 1)),
            None
        );
    }

    /// A role wider than the 32 bytes serde derives an array for.
    enum Wide {}

    impl super::sealed::SealedRole for Wide {}

    impl FixedRole for Wide {
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Wide");
        type Array = [u8; 64];
    }

    /// A role whose debug form prints its width alone.
    enum Short {}

    impl super::sealed::SealedRole for Short {}

    impl FixedRole for Short {
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Short");
        const DEBUG: DebugForm = DebugForm::Length;
        type Array = [u8; 3];
    }

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Frame {
        head: u8,
        wide: FixedBytes<Wide>,
        tail: u8,
    }

    /// `BUS-031`: a binary format writes the bytes in order with no length
    /// prefix, at any width, and reads back the same value.
    #[test]
    fn a_binary_format_writes_the_bytes_with_no_length_prefix() {
        let mut bytes = [0u8; 64];
        bytes.iter_mut().zip(0u8..).for_each(|(b, i)| *b = i);
        let frame = Frame {
            head: 0xAA,
            wide: FixedBytes::from_array(bytes),
            tail: 0xBB,
        };
        let wire = postcard::to_allocvec(&frame).unwrap();
        let mut expected = vec![0xAA];
        expected.extend_from_slice(&bytes);
        expected.push(0xBB);
        assert_eq!(wire, expected);
        assert_eq!(postcard::from_bytes::<Frame>(&wire).unwrap(), frame);
        assert!(postcard::from_bytes::<FixedBytes<Wide>>(&wire[..63]).is_err());
    }

    /// `SRV-242`: a readable format spells the value as lowercase
    /// hexadecimal, and refuses a text that is not hexadecimal of its width.
    #[test]
    fn a_readable_format_spells_hexadecimal_and_refuses_another_width() {
        let value = FixedBytes::<Short>::from_array([0xAB, 0x01, 0xFF]);
        assert_eq!(serde_json::to_string(&value).unwrap(), "\"ab01ff\"");
        let back: FixedBytes<Short> = serde_json::from_str("\"ab01ff\"").unwrap();
        assert_eq!(back, value);
        for refused in [
            "\"ab01\"",
            "\"ab01ff00\"",
            "\"zz01ff\"",
            "\"\"",
            "[171,1,255]",
        ] {
            assert!(
                serde_json::from_str::<FixedBytes<Short>>(refused).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_value_reads_from_its_width_alone() {
        assert_eq!(FixedBytes::<Short>::SIZE, Count::<Octets>::new(3));
        assert!(FixedBytes::<Short>::from_slice(&[1, 2]).is_err());
        assert!(FixedBytes::<Short>::from_slice(&[1, 2, 3, 4]).is_err());
        let value = FixedBytes::<Short>::from_slice(&[1, 2, 3]).unwrap();
        assert_eq!(value.as_array(), &[1, 2, 3]);
        assert_eq!(FixedBytes::<Short>::from_hex(&value.to_hex()), Ok(value));
    }

    #[test]
    fn the_debug_form_follows_the_role() {
        let short = FixedBytes::<Short>::from_array([1, 2, 3]);
        assert_eq!(format!("{short:?}"), "Short(3 bytes)");
        let wide = FixedBytes::<Wide>::from_array([0x0f; 64]);
        assert_eq!(format!("{wide:?}"), format!("Wide({})", "0f".repeat(64)));
    }
}
