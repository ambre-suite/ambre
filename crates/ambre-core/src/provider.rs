//! The identity providers a person signs in with (`ambre-spec-session.md`
//! section 3).
//!
//! The relay keys its discovered clients by provider, and its two sign-in
//! routes name one in their path. The client names one in the same path. One
//! closed set serves both, so a name outside it never forms a relay path.

use serde::{Deserialize, Serialize};

/// An identity provider the relay can discover and sign a person in with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OidcProvider {
    /// Google accounts, discovered at `https://accounts.google.com`.
    Google,
}

impl OidcProvider {
    /// The provider a path segment or a stored word names, or `None` for a
    /// word that names no provider.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    /// The word a sign-in path and the `users` table carry, which
    /// [`OidcProvider::parse`] reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
        }
    }
}

impl std::fmt::Display for OidcProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::OidcProvider;

    /// A provider reads back from its word, and a word that could reach
    /// another relay route names no provider.
    #[test]
    fn a_provider_parses_from_its_word_alone() {
        assert_eq!(
            OidcProvider::parse(OidcProvider::Google.as_str()),
            Some(OidcProvider::Google)
        );
        for bad in ["", "Google", "../keys/master", "google/x", "dev", "gitlab"] {
            assert_eq!(OidcProvider::parse(bad), None, "{bad:?} was accepted");
        }
    }
}
