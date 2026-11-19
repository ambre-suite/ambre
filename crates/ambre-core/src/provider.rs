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
    /// Every provider, for a check that must read them all.
    pub const ALL: [Self; 1] = [Self::Google];

    /// The host the provider's authorization endpoint lives on. The sign-in
    /// follows an address the relay names only on one of these hosts. So a
    /// relay cannot show a page of its choice where the user expects a
    /// password prompt (`CHROME-139`).
    pub const fn authorization_host(self) -> &'static str {
        match self {
            Self::Google => "accounts.google.com",
        }
    }

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

    /// `CHROME-139`: each provider names the one host of its authorization
    /// endpoint, the host its discovery document publishes.
    #[test]
    fn each_provider_pins_the_host_of_its_authorization_endpoint() {
        assert_eq!(
            OidcProvider::ALL.map(OidcProvider::authorization_host),
            ["accounts.google.com"]
        );
    }
}
