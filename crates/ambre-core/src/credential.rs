//! The address of an identity provider's sign-in page.
//!
//! The relay answers it for a sign-in, and the page follows it. It is no
//! secret of its own: the browser keeps it whole in the address bar and in
//! the history, and the state its query carries comes back in clear in the
//! callback address. The state itself is an [`crate::OidcState`], which is a
//! secret, and the code the provider answers is an [`crate::OidcCode`]. So
//! this type holds a bare `String`. Its `Debug` still prints no byte of it,
//! so a log line keeps no sign-in state.

use serde::{Deserialize, Serialize};

/// The identity provider's authorization address the relay answers for a
/// sign-in. It is the relay's data, and a page checks it before following it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderUrl(String);

impl ProviderUrl {
    /// Wraps the address text.
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The address text, for the check and the navigation of the page.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ProviderUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderUrl(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderUrl;

    #[test]
    fn a_provider_address_prints_no_byte_and_serializes_as_its_text() {
        let url = ProviderUrl::new("https://idp.test/auth?state=s".to_owned());
        assert_eq!(format!("{url:?}"), "ProviderUrl(<redacted>)");
        assert_eq!(
            serde_json::to_string(&url).unwrap(),
            "\"https://idp.test/auth?state=s\""
        );
    }
}
