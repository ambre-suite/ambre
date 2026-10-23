//! The target of a link, as a document stores it.

use serde::{Deserialize, Serialize};

/// The target of a link as a document stores it: a web or a mail address,
/// or any other text a peer wrote. Nothing checks a target when a peer
/// writes it, so it is untrusted. Each application checks it by its own
/// rule before the host opens it. A distinct type keeps it apart from the
/// text the link covers. `Debug` prints the bare text, as the `String` it
/// replaced did, so the readings that quote a target keep their words.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkUrl(String);

impl LinkUrl {
    /// The target spelled as `text`.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The target's text, for the check that rules on it and the call that
    /// opens it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LinkUrl {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

impl From<String> for LinkUrl {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Display for LinkUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for LinkUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
