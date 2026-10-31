//! Bytes that cross the threads, one generic type over a role marker.
//!
//! A delta, a snapshot, an ephemeral sample, a media file and a font file all
//! cross the threads as bytes. A bare `Vec<u8>` lets a snapshot pass where a
//! delta is expected. [`Payload`] keeps them apart up to the seal, where Sync
//! encrypts the ones it sends: `Payload<DeltaBytes>` and
//! `Payload<SnapshotBytes>` are two types. They may hold document plaintext,
//! so `Debug` prints the length alone, as the encoded roles do. Serde sees the
//! bare bytes: the envelope that carries one is the same as the envelope that
//! carried a `Vec<u8>`. Two roles are sealed rather than plaintext: the row of
//! the local store, [`SealedRowBytes`], and the `.ambre` file sealed under a
//! passphrase, [`SealedContainer`].

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The role of bytes that cross the threads: the name its debug form prints.
/// A crate downstream may declare a role of its own.
pub trait PayloadKind: 'static {
    /// The name the debug form prints, beside the length.
    const NAME: &'static str;
}

/// Bytes of the role `K`. The field is private: [`Payload::new`] and
/// `From<Vec<u8>>` are the ways in. Serde sees the bare bytes, and `Debug`
/// prints the length alone.
pub struct Payload<K: PayloadKind> {
    bytes: Vec<u8>,
    kind: PhantomData<fn() -> K>,
}

impl<K: PayloadKind> Payload<K> {
    /// Wraps the bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            kind: PhantomData,
        }
    }

    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes, owned.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// How many bytes it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether there are no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl<K: PayloadKind> From<Vec<u8>> for Payload<K> {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl<K: PayloadKind> AsRef<[u8]> for Payload<K> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl<K: PayloadKind> Clone for Payload<K> {
    fn clone(&self) -> Self {
        Self::new(self.bytes.clone())
    }
}

impl<K: PayloadKind> Default for Payload<K> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<K: PayloadKind> PartialEq for Payload<K> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<K: PayloadKind> Eq for Payload<K> {}

impl<K: PayloadKind> Hash for Payload<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl<K: PayloadKind, const N: usize> PartialEq<[u8; N]> for Payload<K> {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.bytes == other
    }
}

impl<K: PayloadKind, const N: usize> PartialEq<&[u8; N]> for Payload<K> {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.bytes == *other
    }
}

impl<K: PayloadKind> PartialEq<Vec<u8>> for Payload<K> {
    fn eq(&self, other: &Vec<u8>) -> bool {
        &self.bytes == other
    }
}

impl<K: PayloadKind> PartialEq<[u8]> for Payload<K> {
    fn eq(&self, other: &[u8]) -> bool {
        self.bytes == other
    }
}

impl<K: PayloadKind> fmt::Debug for Payload<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({} bytes)", K::NAME, self.bytes.len())
    }
}

impl<K: PayloadKind> Serialize for Payload<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.bytes.serialize(serializer)
    }
}

impl<'de, K: PayloadKind> Deserialize<'de> for Payload<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(Self::new)
    }
}

/// The role of one CRDT change as the engine exports it, before Sync seals it
/// or after Sync opens it. `Payload<DeltaBytes>`.
pub enum DeltaBytes {}

impl PayloadKind for DeltaBytes {
    const NAME: &'static str = "DeltaBytes";
}

/// The role of a full CRDT state as the engine exports it, before Sync seals
/// it or after Sync opens it. `Payload<SnapshotBytes>`.
pub enum SnapshotBytes {}

impl PayloadKind for SnapshotBytes {
    const NAME: &'static str = "SnapshotBytes";
}

impl Payload<SnapshotBytes> {
    /// The state as the plaintext a vault seals. The bytes move into the
    /// wrapper that wipes them, and no copy stays behind.
    #[must_use]
    pub fn into_secret(self) -> crate::SecretBytes {
        crate::SecretBytes::new(self.into_bytes())
    }
}

/// The role of one peer's presence as the engine encodes it, kept by nothing
/// after delivery. `Payload<EphemeralBytes>`.
pub enum EphemeralBytes {}

impl PayloadKind for EphemeralBytes {
    const NAME: &'static str = "EphemeralBytes";
}

/// The role of a media file as the user placed it, before Sync seals it or
/// after Sync opens it. `Payload<MediaBytes>`.
pub enum MediaBytes {}

impl PayloadKind for MediaBytes {
    const NAME: &'static str = "MediaBytes";
}

impl Payload<MediaBytes> {
    /// Wipes the bytes now: a media file its reader refuses and drops
    /// (`ARCH-046`).
    pub fn wipe(self) {
        crate::Secret::wipe(self.bytes);
    }
}

/// The role of a decoded image: non-premultiplied RGBA8 samples, row-major,
/// four bytes a pixel. Decoded on the Logic thread, painted on the UI thread.
/// `Payload<RgbaPixels>`.
pub enum RgbaPixels {}

impl PayloadKind for RgbaPixels {
    const NAME: &'static str = "RgbaPixels";
}

/// The role of one row of the local store, sealed by the Sync worker under
/// the store key (`RESID-025`). The Main thread moves it between the worker
/// and the browser's database and cannot open it. A plaintext snapshot or
/// delta is never one of these, so the store cannot receive one.
/// `Payload<SealedRowBytes>`.
pub enum SealedRowBytes {}

impl PayloadKind for SealedRowBytes {
    const NAME: &'static str = "SealedRowBytes";
}

/// The role of a whole plaintext `.ambre` container, header included, as the
/// picker read it or as a sealed file opened to (`ambre-spec-docfile.md`).
/// `Payload<PlaintextContainer>`.
pub enum PlaintextContainer {}

impl PayloadKind for PlaintextContainer {
    const NAME: &'static str = "PlaintextContainer";
}

/// The role of a whole file the picker read, before [`crate::classify`] names
/// its lineage. Its version byte says whether it goes on as a
/// [`PlaintextContainer`] or as a [`SealedContainer`].
/// `Payload<PickedContainer>`.
pub enum PickedContainer {}

impl PayloadKind for PickedContainer {
    const NAME: &'static str = "PickedContainer";
}

/// A picked file under the role its version byte names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickedFile {
    /// A plaintext container, or bytes [`crate::classify`] refuses.
    Plaintext(Payload<PlaintextContainer>),
    /// A passphrase-sealed container.
    Sealed(Payload<SealedContainer>),
}

impl Payload<PickedContainer> {
    /// The file under the role [`crate::classify`] names (`DOCFILE-010`).
    /// Bytes it refuses go on as plaintext, and the plaintext reader names
    /// the refusal. Each reader validates the bytes again.
    #[must_use]
    pub fn into_lineage(self) -> PickedFile {
        match crate::classify(&self) {
            Ok(crate::ContainerKind::Sealed(_)) => PickedFile::Sealed(Payload::new(self.bytes)),
            Ok(crate::ContainerKind::Plaintext(_)) | Err(_) => {
                PickedFile::Plaintext(Payload::new(self.bytes))
            }
        }
    }
}

/// The role of a whole passphrase-sealed `.ambre` container, header included,
/// as it sits on the user's disk (`DOCFILE-010`). Only the Sync worker opens
/// it, to a [`PlaintextContainer`]; a store row is a [`SealedRowBytes`]
/// instead, under another key. `Payload<SealedContainer>`.
pub enum SealedContainer {}

impl PayloadKind for SealedContainer {
    const NAME: &'static str = "SealedContainer";
}

/// The role of a document's previous key, sealed under the key that replaced
/// it and signed by the rotator: the body of a key link. The relay stores it
/// and opens none. Transparent to serde, so a link's postcard bytes are those
/// of the plain byte string it replaced. `Payload<SealedPreviousKey>`.
pub enum SealedPreviousKey {}

impl PayloadKind for SealedPreviousKey {
    const NAME: &'static str = "SealedPreviousKey";
}

/// The role of one font file, as the Main thread fetched it for the Logic
/// thread to shape on. `Payload<FontBytes>`.
pub enum FontBytes {}

impl PayloadKind for FontBytes {
    const NAME: &'static str = "FontBytes";
}

/// The role of an application's own clipboard half: the encoded selection it
/// copied, which only the same application decodes on a paste. Another
/// program reads the text and HTML halves beside it.
/// `Payload<ClipboardBytes>`.
pub enum ClipboardBytes {}

impl PayloadKind for ClipboardBytes {
    const NAME: &'static str = "ClipboardBytes";
}

/// The role of a small PNG a board stores beside a picture, painted before
/// the full picture is fetched. An import encodes it, and the board's media
/// map stores its bytes as they are. `Payload<ThumbnailPng>`.
pub enum ThumbnailPng {}

impl PayloadKind for ThumbnailPng {
    const NAME: &'static str = "ThumbnailPng";
}

/// The role of the four bytes that open an encoding of Ambre's own:
/// `FixedBytes<FormatMagic>`. Bytes another encoding wrote under the same
/// type then never decode as this one. A binary format sees the four bytes
/// with no length prefix, as the bare array it replaced.
pub enum FormatMagic {}

impl crate::FixedRole for FormatMagic {
    const NAME: &'static str = "FormatMagic";
    type Array = [u8; 4];
}

/// What a CRDT document imports: one delta, or a whole snapshot. The engine
/// imports both alike. The holder knows which it is, and a local store, a
/// ledger or a test may treat the two apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoroPayload {
    /// One change.
    Delta(Payload<DeltaBytes>),
    /// A full state.
    Snapshot(Payload<SnapshotBytes>),
}

impl LoroPayload {
    /// The payload, borrowed.
    #[must_use]
    pub fn borrowed(&self) -> LoroPayloadRef<'_> {
        match self {
            Self::Delta(delta) => LoroPayloadRef::Delta(delta),
            Self::Snapshot(snapshot) => LoroPayloadRef::Snapshot(snapshot),
        }
    }

    /// The bytes, for the engine that imports either form.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.borrowed().as_bytes()
    }
}

/// A [`LoroPayload`] borrowed from its holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoroPayloadRef<'a> {
    /// One change.
    Delta(&'a Payload<DeltaBytes>),
    /// A full state.
    Snapshot(&'a Payload<SnapshotBytes>),
}

impl<'a> LoroPayloadRef<'a> {
    /// The bytes, for the engine that imports either form.
    #[must_use]
    pub fn as_bytes(self) -> &'a [u8] {
        match self {
            Self::Delta(delta) => delta.as_bytes(),
            Self::Snapshot(snapshot) => snapshot.as_bytes(),
        }
    }

    /// The payload, owned.
    #[must_use]
    pub fn to_payload(self) -> LoroPayload {
        match self {
            Self::Delta(delta) => LoroPayload::Delta(delta.clone()),
            Self::Snapshot(snapshot) => LoroPayload::Snapshot(snapshot.clone()),
        }
    }
}

impl<'a> From<&'a Payload<DeltaBytes>> for LoroPayloadRef<'a> {
    fn from(delta: &'a Payload<DeltaBytes>) -> Self {
        Self::Delta(delta)
    }
}

impl<'a> From<&'a Payload<SnapshotBytes>> for LoroPayloadRef<'a> {
    fn from(snapshot: &'a Payload<SnapshotBytes>) -> Self {
        Self::Snapshot(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeltaBytes, EphemeralBytes, LoroPayload, MediaBytes, Payload, PlaintextContainer,
        RgbaPixels, SnapshotBytes,
    };

    #[test]
    fn debug_prints_the_length_alone() {
        assert_eq!(
            format!("{:?}", Payload::<DeltaBytes>::new(b"secret".to_vec())),
            "DeltaBytes(6 bytes)"
        );
        assert_eq!(
            format!("{:?}", Payload::<SnapshotBytes>::new(vec![1, 2])),
            "SnapshotBytes(2 bytes)"
        );
        assert_eq!(
            format!("{:?}", Payload::<EphemeralBytes>::default()),
            "EphemeralBytes(0 bytes)"
        );
        assert_eq!(
            format!("{:?}", Payload::<RgbaPixels>::new(vec![9, 8, 7, 255])),
            "RgbaPixels(4 bytes)"
        );
        assert_eq!(
            format!("{:?}", Payload::<MediaBytes>::new(b"a photo".to_vec())),
            "MediaBytes(7 bytes)"
        );
        assert_eq!(
            format!(
                "{:?}",
                Payload::<PlaintextContainer>::new(b"AMBRE".to_vec())
            ),
            "PlaintextContainer(5 bytes)"
        );
    }

    #[test]
    fn a_payload_keeps_its_kind_and_its_bytes() {
        let delta = LoroPayload::Delta(Payload::new(vec![1, 2]));
        assert_eq!(delta.as_bytes(), [1, 2]);
        assert_eq!(delta.borrowed().to_payload(), delta);
        let snapshot = LoroPayload::Snapshot(Payload::new(vec![1, 2]));
        assert_ne!(snapshot, delta);
    }

    /// Each serde form is the bare byte string, in JSON and in postcard, and
    /// it reads back.
    #[test]
    fn serde_sees_the_bare_bytes() {
        let bytes = vec![0xab, 0xcd];
        let snapshot = Payload::<SnapshotBytes>::new(bytes.clone());
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            serde_json::to_string(&bytes).unwrap()
        );
        let wire = postcard::to_allocvec(&snapshot).unwrap();
        assert_eq!(wire, postcard::to_allocvec(&bytes).unwrap());
        let back: Payload<SnapshotBytes> = postcard::from_bytes(&wire).unwrap();
        assert_eq!(back, snapshot);
    }
}
