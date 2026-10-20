//! An email address, as an account or an invitation names it.

use serde::{Deserialize, Serialize};

/// An email address, as an account or an invitation names it. A distinct
/// type, so a display name never takes its place. It checks no syntax: the
/// text is whatever the identity provider or the inviter gave.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(pub String);

impl Email {
    /// The address text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Email {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for Email {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Display for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
