//! Public keys as the relay's JSON spells them: lowercase hexadecimal.
//!
//! A user holds two public keys, one that verifies signatures and one that
//! wraps a document key. Both travel as the same hex text, so a bare
//! `String` lets one stand for the other. One type per role keeps them
//! apart. Each is `#[serde(transparent)]` and checks nothing on parse. The
//! reader that decodes the bytes keeps its own refusal, so the type changes
//! no byte on the wire and no answer of the relay.

use serde::{Deserialize, Serialize};

macro_rules! define_key_hex {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The hex spelling of `bytes`.
            pub fn encode(bytes: impl AsRef<[u8]>) -> Self {
                Self(hex::encode(bytes))
            }

            /// The hex text, as the relay exchanges it.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Whether the text is empty: a peer that sent no key.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
    };
}

define_key_hex! {
    /// The hex spelling of an Ed25519 public key: the one that verifies a
    /// user's signatures.
    SigningKeyHex
}

define_key_hex! {
    /// The hex spelling of an X25519 public key: the one a document key is
    /// wrapped to.
    ExchangeKeyHex
}

#[cfg(test)]
mod tests {
    use super::{ExchangeKeyHex, SigningKeyHex};

    #[test]
    fn a_key_spelling_serializes_as_the_bare_hex_string() {
        let key = SigningKeyHex::encode([0xab, 0x01]);
        assert_eq!(key.as_str(), "ab01");
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"ab01\"");
        let parsed: ExchangeKeyHex = serde_json::from_str("\"ff\"").unwrap();
        assert_eq!(parsed, ExchangeKeyHex("ff".to_owned()));
    }
}
