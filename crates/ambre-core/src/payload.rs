//! Plaintext CRDT bytes, one type per role.
//!
//! A delta, a snapshot and an ephemeral sample all cross the threads as
//! bytes. A bare `Vec<u8>` lets a snapshot pass where a delta is expected. One type per role keeps them apart up to the seal, where Sync
//! encrypts them. They hold document plaintext, so `Debug` prints the length
//! alone, as the encoded roles do. Serde sees the bare bytes: the envelope
//! that carries one is the same as the envelope that carried a `Vec<u8>`.

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

#[cfg(test)]
mod tests {
    use super::{DeltaBytes, EphemeralBytes, SnapshotBytes};

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
