//! Sealed and signed bytes as the relay's JSON spells them: URL-safe base64.
//!
//! A member row carries a wrapped document key beside a sealed name. An
//! account carries a wrapped master key beside its salt, and a change
//! carries a signed log entry. All of them travel as the same base64 text,
//! so a bare `String` lets one stand for another. One type per role keeps
//! them apart.
//!
//! Each is `#[serde(transparent)]` and checks nothing on parse. The reader
//! that decodes the bytes keeps its own refusal, so the type changes no byte
//! on the wire and no answer of the relay. The base64 codec stays with the
//! crates that seal and open, which already depend on it.

use serde::{Deserialize, Serialize};

macro_rules! define_encoded {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The base64 text, as the relay exchanges it.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Whether the text is empty: a field the sender left out.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
    };
}

define_encoded! {
    /// A document key wrapped to one member: `nonce || ciphertext`.
    WrappedDocKey
}

define_encoded! {
    /// A document or channel name sealed under its document key.
    SealedName
}

define_encoded! {
    /// An account's master seed wrapped under the key its passphrase derives.
    WrappedMasterKey
}

define_encoded! {
    /// The salt the passphrase key of an account is derived with.
    MasterKeySalt
}

define_encoded! {
    /// One share of an organization's recovery secret, sealed to its holder.
    SealedShare
}

define_encoded! {
    /// A signed entry of a document's membership log.
    EncodedLogEntry
}

define_encoded! {
    /// A signed link from one document key to the next.
    EncodedKeyLink
}

define_encoded! {
    /// The proof a snapshot carries of the snapshot it replaces.
    EncodedProof
}

#[cfg(test)]
mod tests {
    use super::WrappedDocKey;

    #[test]
    fn an_encoded_value_serializes_as_the_bare_base64_string() {
        let key = WrappedDocKey("q83v".to_owned());
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"q83v\"");
        let parsed: WrappedDocKey = serde_json::from_str("\"q83v\"").unwrap();
        assert_eq!(parsed, key);
    }
}
