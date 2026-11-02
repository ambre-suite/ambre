//! An email address, as an account or an invitation names it.

use crate::Redacted;
use serde::{Deserialize, Serialize};

/// An email address, as an account or an invitation names it. A distinct
/// type, so a display name never takes its place. It checks no syntax: the
/// text is whatever the identity provider or the inviter gave. The address is
/// personal data, so `Debug` prints its length alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(Redacted<String>);

impl Email {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: String) -> Self {
        Self(Redacted::new(value))
    }

    /// The address text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for Email {
    fn from(text: &str) -> Self {
        Self(Redacted::new(text.to_owned()))
    }
}

impl From<String> for Email {
    fn from(text: String) -> Self {
        Self(Redacted::new(text))
    }
}

impl std::fmt::Display for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address is personal data: its `Debug` form prints its length alone.
    #[test]
    fn an_address_prints_its_length_and_none_of_its_text() {
        let printed = format!("{:?}", Email::from("ada@example.org"));
        assert_eq!(printed, "Email(15 chars)");
    }
}
