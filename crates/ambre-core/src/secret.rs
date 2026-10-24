//! Secret text that crosses a thread: the recovery phrase of a new account.
//!
//! The phrase travels from the Sync worker that derives it to the screen that
//! shows it once. On that path it is a type of its own, wiped when the last
//! copy drops, and its `Debug` prints only its length. It lives here, not in
//! the bus, so a crate that cannot depend on the bus holds it too.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use zeroize::Zeroizing;

/// Secret UTF-8 text, wiped on drop and redacted in `Debug`.
#[derive(Clone, Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Wraps an owned string as a zeroizing secret.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The text, for the one screen or call that reads it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The length of the text, in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretString {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString([redacted; {} bytes])", self.0.len())
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::SecretString;

    #[test]
    fn a_secret_string_prints_only_its_length() {
        let secret = SecretString::new("abandon ability".to_owned());
        let printed = format!("{secret:?}");
        assert_eq!(printed, "SecretString([redacted; 15 bytes])");
        assert!(!printed.contains("abandon"));
        assert_eq!(secret.expose(), "abandon ability");
    }

    #[test]
    fn a_secret_string_serialises_as_its_text() {
        let secret = SecretString::new("words".to_owned());
        let bytes = postcard::to_allocvec(&secret).unwrap();
        assert_eq!(bytes, postcard::to_allocvec("words").unwrap());
        let back: SecretString = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back.expose(), "words");
    }
}
