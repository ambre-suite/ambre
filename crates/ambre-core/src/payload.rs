//! Bytes that cross the threads, one type per role.
//!
//! A delta, a snapshot, an ephemeral sample, a media file and a font file all
//! cross the threads as bytes. A bare `Vec<u8>` lets a snapshot pass where a
//! delta is expected. One type per role keeps them apart up to the seal, where
//! Sync encrypts the ones it sends. They may hold document plaintext, so
//! `Debug` prints the length alone, as the encoded roles do. Serde sees the
//! bare bytes: the envelope that carries one is the same as the envelope that
//! carried a `Vec<u8>`. Two roles are sealed rather than plaintext: the row of
//! the local store, [`SealedRowBytes`], and the `.ambre` file sealed under a
//! passphrase, [`SealedContainer`].

use serde::{Deserialize, Serialize};

macro_rules! define_payload {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Vec<u8>);

        impl $name {
            /// Wraps the bytes.
            #[must_use]
            pub fn new(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }

            /// The bytes.
            #[must_use]
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// The bytes, owned.
            #[must_use]
            pub fn into_bytes(self) -> Vec<u8> {
                self.0
            }

            /// How many bytes it holds.
            #[must_use]
            pub fn len(&self) -> usize {
                self.0.len()
            }

            /// Whether there are no bytes.
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl From<Vec<u8>> for $name {
            fn from(bytes: Vec<u8>) -> Self {
                Self(bytes)
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl<const N: usize> PartialEq<[u8; N]> for $name {
            fn eq(&self, other: &[u8; N]) -> bool {
                self.0 == other
            }
        }

        impl<const N: usize> PartialEq<&[u8; N]> for $name {
            fn eq(&self, other: &&[u8; N]) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<Vec<u8>> for $name {
            fn eq(&self, other: &Vec<u8>) -> bool {
                &self.0 == other
            }
        }

        impl PartialEq<[u8]> for $name {
            fn eq(&self, other: &[u8]) -> bool {
                self.0 == other
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "({} bytes)"), self.0.len())
            }
        }
    };
}

define_payload! {
    /// One CRDT change as the engine exports it, before Sync seals it or
    /// after Sync opens it.
    DeltaBytes
}

define_payload! {
    /// A full CRDT state as the engine exports it, before Sync seals it or
    /// after Sync opens it.
    SnapshotBytes
}

define_payload! {
    /// One peer's presence as the engine encodes it, kept by nothing after
    /// delivery.
    EphemeralBytes
}

define_payload! {
    /// A media file as the user placed it, before Sync seals it or after Sync
    /// opens it.
    MediaBytes
}

define_payload! {
    /// A decoded image: non-premultiplied RGBA8 samples, row-major, four bytes
    /// a pixel. Decoded on the Logic thread, painted on the UI thread.
    RgbaPixels
}

define_payload! {
    /// One row of the local store, sealed by the Sync worker under the store
    /// key (`RESID-025`). The Main thread moves it between the worker and the
    /// browser's database and cannot open it. A plaintext snapshot or delta is
    /// never one of these, so the store cannot receive one.
    SealedRowBytes
}

define_payload! {
    /// A whole plaintext `.ambre` container, header included, as the picker
    /// read it or as a sealed file opened to (`ambre-spec-docfile.md`).
    PlaintextContainer
}

define_payload! {
    /// A whole file the picker read, before [`crate::classify`] names its
    /// lineage. Its version byte says whether it goes on as a
    /// [`PlaintextContainer`] or as a [`SealedContainer`].
    PickedContainer
}

define_payload! {
    /// A whole passphrase-sealed `.ambre` container, header included, as it
    /// sits on the user's disk (`DOCFILE-010`). Only the Sync worker opens it,
    /// to a [`PlaintextContainer`]; a store row is a [`SealedRowBytes`]
    /// instead, under another key.
    SealedContainer
}

define_payload! {
    /// A document's previous key, sealed under the key that replaced it and
    /// signed by the rotator: the body of a key link. The relay stores it and
    /// opens none. Transparent to serde, so a link's postcard bytes are those
    /// of the plain byte string it replaced.
    SealedPreviousKey
}

define_payload! {
    /// One font file, as the Main thread fetched it for the Logic thread to
    /// shape on.
    FontBytes
}

define_payload! {
    /// An application's own clipboard half: the encoded selection it copied,
    /// which only the same application decodes on a paste. Another program
    /// reads the text and HTML halves beside it.
    ClipboardBytes
}

/// The four bytes that open an encoding of Ambre's own, so bytes another
/// encoding wrote under the same type never decode as this one. Transparent
/// to serde, so the bytes are those of the bare array it replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FormatMagic([u8; 4]);

impl FormatMagic {
    /// The magic of an encoding, spelled as its four bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }
}

/// What a CRDT document imports: one delta, or a whole snapshot. The engine
/// imports both alike. The holder knows which it is, and a local store, a
/// ledger or a test may treat the two apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoroPayload {
    /// One change.
    Delta(DeltaBytes),
    /// A full state.
    Snapshot(SnapshotBytes),
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
    Delta(&'a DeltaBytes),
    /// A full state.
    Snapshot(&'a SnapshotBytes),
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

impl<'a> From<&'a DeltaBytes> for LoroPayloadRef<'a> {
    fn from(delta: &'a DeltaBytes) -> Self {
        Self::Delta(delta)
    }
}

impl<'a> From<&'a SnapshotBytes> for LoroPayloadRef<'a> {
    fn from(snapshot: &'a SnapshotBytes) -> Self {
        Self::Snapshot(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::{DeltaBytes, EphemeralBytes, LoroPayload, RgbaPixels, SnapshotBytes};

    #[test]
    fn debug_prints_the_length_alone() {
        assert_eq!(
            format!("{:?}", DeltaBytes::new(b"secret".to_vec())),
            "DeltaBytes(6 bytes)"
        );
        assert_eq!(
            format!("{:?}", SnapshotBytes::new(vec![1, 2])),
            "SnapshotBytes(2 bytes)"
        );
        assert_eq!(
            format!("{:?}", EphemeralBytes::default()),
            "EphemeralBytes(0 bytes)"
        );
        assert_eq!(
            format!("{:?}", RgbaPixels::new(vec![9, 8, 7, 255])),
            "RgbaPixels(4 bytes)"
        );
    }

    #[test]
    fn a_payload_keeps_its_kind_and_its_bytes() {
        let delta = LoroPayload::Delta(DeltaBytes::new(vec![1, 2]));
        assert_eq!(delta.as_bytes(), [1, 2]);
        assert_eq!(delta.borrowed().to_payload(), delta);
        let snapshot = LoroPayload::Snapshot(SnapshotBytes::new(vec![1, 2]));
        assert_ne!(snapshot, delta);
    }

    #[test]
    fn serde_sees_the_bare_bytes() {
        let bytes = vec![0xab, 0xcd];
        assert_eq!(
            serde_json::to_string(&SnapshotBytes::new(bytes.clone())).unwrap(),
            serde_json::to_string(&bytes).unwrap()
        );
    }
}
