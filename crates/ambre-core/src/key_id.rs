//! The identifier of a document key (`SYNC-115`).
//!
//! Every sealed payload names the key that sealed it (`SYNC-125`), so a reader
//! knows which key opens it without trying each one. The identifier is public:
//! the relay sees it, and `ZK-007` admits it.

use crate::fixed_bytes::FixedRole;

/// Bytes of a key identifier.
pub const KEY_ID_LEN: usize = 16;

/// The role of the identifier of one document key: `FixedBytes<KeyId>`.
/// `ambre-crypto` derives it from the key itself, so a false identifier cannot
/// name a real key. Its form in a JSON body is lowercase hex.
pub enum KeyId {}

impl FixedRole for KeyId {
    const NAME: &'static str = "KeyId";
    type Array = [u8; KEY_ID_LEN];
}
