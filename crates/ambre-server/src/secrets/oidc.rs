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

    /// The client secret, as the provider client presents it. Named in
    /// `src/oidc.rs` alone, which builds the one client (`ARCH-049`).
    pub fn to_client_secret(&self) -> ClientSecret {
        ClientSecret::new(self.0.expose().clone())
    }

    /// The PKCE verifier, as the code exchange presents it. Named in
    /// `src/oidc.rs` alone, which makes the one exchange (`ARCH-049`).
    pub fn to_pkce_verifier(&self) -> PkceCodeVerifier {
        PkceCodeVerifier::new(self.0.expose().clone())
    }

    /// Lends the text to `store`, the one call that writes it beside the
    /// sign-in state until the callback takes it back. Named in
    /// `src/redis_ops.rs` alone (`ARCH-049`). No copy is kept here.
    pub fn present_stored<R>(&self, store: impl FnOnce(&str) -> R) -> R {
        store(self.0.expose())
    }
}

/// The authorization code, as the code exchange presents it. Named in
/// `src/oidc.rs` alone, which makes the one exchange (`ARCH-049`).
pub fn authorization_code(
    code: &ambre_core::Credential<ambre_core::OidcCode>,
) -> AuthorizationCode {
    AuthorizationCode::new(code.secret().expose().clone())
}
