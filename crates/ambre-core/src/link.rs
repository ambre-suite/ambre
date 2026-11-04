//! The target of a link, as a document stores it.

use crate::Redacted;
use serde::{Deserialize, Serialize};

/// The target of a link as a document stores it: a web or a mail address,
/// or any other text a peer wrote. Nothing checks a target when a peer
/// writes it, so it is untrusted. Each application checks it by its own
/// rule before the host opens it. A distinct type keeps it apart from the
/// text the link covers. A peer typed the target, so `Debug` prints its
/// length alone, and a log carries none of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkUrl(Redacted<String>);

impl LinkUrl {
    /// The target spelled as `text`.
    pub fn new(text: String) -> Self {
        Self(Redacted::new(text))
    }

    /// The target's text, for the check that rules on it and the call that
    /// opens it.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The size of the target, in UTF-8 bytes, for the cap a writer holds
    /// it to.
    #[must_use]
    pub fn size(&self) -> crate::Count<crate::Octets> {
        crate::Count::of(self.0.as_str().len())
    }
}

impl From<&str> for LinkUrl {
    fn from(text: &str) -> Self {
        Self(Redacted::new(text.to_owned()))
    }
}

impl From<String> for LinkUrl {
    fn from(text: String) -> Self {
        Self(Redacted::new(text))
    }
}

impl std::fmt::Display for LinkUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target's `Debug` form holds none of its text, so a log carries none.
    #[test]
    fn a_target_prints_its_length_and_none_of_its_text() {
        let url = LinkUrl::from("https://a.example");
        assert_eq!(format!("{url:?}"), "LinkUrl(17 chars)");
    }
}
