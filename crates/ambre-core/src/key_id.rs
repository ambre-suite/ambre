//! The identifier of a document key (`SYNC-115`).
//!
//! Every sealed payload names the key that sealed it (`SYNC-125`), so a reader
//! knows which key opens it without trying each one. The identifier is public:
//! the relay sees it, and `ZK-007` admits it.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Bytes of a key identifier.
pub const KEY_ID_LEN: usize = 16;

/// The identifier of one document key. `ambre-crypto` derives it from the key
/// itself, so a false identifier cannot name a real key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyId([u8; KEY_ID_LEN]);

impl KeyId {
    /// Wraps the bytes of an identifier.
    pub const fn from_array(bytes: [u8; KEY_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// The bytes of the identifier.
    pub const fn as_array(&self) -> &[u8; KEY_ID_LEN] {
        &self.0
    }

    /// The identifier as lowercase hex, its form in a JSON body.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Reads an identifier from its hex form.
    ///
    /// # Errors
    ///
    /// [`KeyIdError`] when `text` is not hex, or not [`KEY_ID_LEN`] bytes long.
    pub fn parse_hex(text: &str) -> Result<Self, KeyIdError> {
        hex::decode(text)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .map(Self)
            .ok_or(KeyIdError)
    }
}

crate::envelope::fixed_bytes!(KeyId, KEY_ID_LEN);

/// A text that is not the hex form of a key identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a key identifier: {KEY_ID_LEN} bytes in hex")]
pub struct KeyIdError;

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({})", hex::encode(self.0))
    }
}
