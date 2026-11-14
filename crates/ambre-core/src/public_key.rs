//! The roles of a public key: 32 bytes each.
//!
//! One role per use: `FixedBytes<PublicKey>` verifies an Ed25519 signature,
//! and `FixedBytes<ExchangePublicKey>` receives an X25519 key exchange. Two
//! types, so an exchange key never stands where a signing key is read. The
//! relay's JSON spells both as lowercase hexadecimal, the readable form of
//! [`FixedBytes`](crate::FixedBytes).
//!
//! Public keys are not secret, so neither role wipes its bytes on drop.
//! Doing so would add a memset to a hot, frequently copied type, and would
//! imply a confidentiality the type does not have. Wiping is reserved for
//! the secret key types.

use crate::fixed_bytes::{DebugForm, FixedRole};

/// The role of a 32-byte Ed25519 public key: the one that verifies a user's
/// signatures. `FixedBytes<PublicKey>`.
pub enum PublicKey {}

impl FixedRole for PublicKey {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("PublicKey");
    const DEBUG: DebugForm = DebugForm::HexPrefix;
    type Array = [u8; 32];
}

/// The role of a 32-byte X25519 public key: the one a document key is
/// wrapped to. `FixedBytes<ExchangePublicKey>`.
pub enum ExchangePublicKey {}

impl FixedRole for ExchangePublicKey {
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("ExchangePublicKey");
    const DEBUG: DebugForm = DebugForm::HexPrefix;
    type Array = [u8; 32];
}

#[cfg(test)]
mod tests {
    use super::{ExchangePublicKey, PublicKey};
    use crate::FixedBytes;

    #[test]
    fn a_key_reads_its_own_width_alone() {
        let slice = [0x99; 32];
        let pk = FixedBytes::<PublicKey>::from_slice(&slice).unwrap();
        assert_eq!(pk.as_slice(), slice);
        assert!(FixedBytes::<PublicKey>::from_slice(&[0x00; 31]).is_err());
        assert!(FixedBytes::<PublicKey>::from_slice(&[0x00; 33]).is_err());
    }

    #[test]
    fn a_key_reads_its_own_hex() {
        let pk = FixedBytes::<PublicKey>::from_array([0x01; 32]);
        assert_eq!(pk.to_hex(), "01".repeat(32));
        assert_eq!(FixedBytes::from_hex(&pk.to_hex()), Ok(pk));
        let key = FixedBytes::<ExchangePublicKey>::from_array([0x0f; 32]);
        assert_eq!(FixedBytes::from_hex(&key.to_hex()), Ok(key));
        assert_eq!(format!("{key:?}"), "ExchangePublicKey(0x0f0f0f0f...)");
    }

    /// The relay's JSON spells a key as the hexadecimal of its 32 bytes, the
    /// text the hexadecimal key types held before.
    #[test]
    fn a_key_travels_as_its_hex_in_json() {
        let pk = FixedBytes::<PublicKey>::from_array([0xaa; 32]);
        let json = serde_json::to_string(&pk).unwrap();
        assert_eq!(json, format!("\"{}\"", "aa".repeat(32)));
        assert_eq!(
            serde_json::from_str::<FixedBytes<PublicKey>>(&json).unwrap(),
            pk
        );
    }
}
