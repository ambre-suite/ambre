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
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({})", hex::encode(self.0))
    }
}
