//! The secrets of the OIDC exchange, as the provider crate takes them.

use std::sync::Arc;

use ambre_core::Secret;
use openidconnect::{AuthorizationCode, ClientSecret, PkceCodeVerifier};

/// A secret of the OIDC exchange: the client secret the provider issued, or
/// a PKCE verifier. It holds its text in a shared [`Secret`], so a clone
/// copies the pointer and the text is wiped when the last owner drops it.
/// `Debug` prints no byte of it, so a logged `Config` or sign-in state
/// carries none.
#[derive(Clone, Debug, Default)]
pub struct OidcSecret(Arc<Secret<String>>);

impl OidcSecret {
    /// Wraps the secret text.
    pub fn new(secret: String) -> Self {
        Self(Arc::new(Secret::new(secret)))
    }

    /// The client secret, as the provider client presents it.
    pub fn client_secret(&self) -> ClientSecret {
        ClientSecret::new(self.0.expose().clone())
    }

    /// The PKCE verifier, as the code exchange presents it.
    pub fn pkce_verifier(&self) -> PkceCodeVerifier {
        PkceCodeVerifier::new(self.0.expose().clone())
    }

    /// The text the store keeps beside the sign-in state, until the
    /// callback takes it back.
    pub fn stored_text(&self) -> String {
        self.0.expose().clone()
    }
}

/// The authorization code, as the code exchange presents it.
pub fn authorization_code(code: &ambre_core::OidcCode) -> AuthorizationCode {
    AuthorizationCode::new(code.secret().expose().clone())
}
