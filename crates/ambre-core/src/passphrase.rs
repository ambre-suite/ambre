//! The text a person types to derive a key: the account passphrase, or the
//! passphrase of one sealed `.ambre` file.
//!
//! A passphrase travels from the prompt that reads it to the worker that
//! derives a key from it, and no further. Along that path it is a type of its
//! own, not a `String` or a `&[u8]` beside a salt of the same shape. Its
//! `Debug` prints no character of it. Its bytes are wiped when the last copy
//! drops, including a copy a relay merely binds and drops.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// A passphrase, as typed. Wiped on drop, and redacted in `Debug`.
///
/// Two values compare equal when their text is the same, which is the check a
/// sealing prompt makes against its confirmation field. The comparison is not
/// constant-time: both sides are the user's own typing, never a stored secret.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    /// Wraps the typed text.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The UTF-8 bytes, for the one derivation that reads them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Whether nothing was typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for Passphrase {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

impl Serialize for Passphrase {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Passphrase {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::Passphrase;

    #[test]
    fn a_passphrase_prints_no_character_and_travels_as_its_text() {
        let typed = Passphrase::new("correct horse".to_owned());
        assert_eq!(format!("{typed:?}"), "Passphrase(<redacted>)");
        let wire = serde_json::to_string(&typed).unwrap();
        assert_eq!(wire, "\"correct horse\"");
        let back: Passphrase = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, typed);
        assert_eq!(back.as_bytes(), b"correct horse");
    }
}
