//! The secrets of the OIDC exchange, as the provider crate takes them.

use ambre_core::{Confidential, ConfidentialKind};
use openidconnect::{AuthorizationCode, ClientSecret, PkceCodeVerifier};

/// The role of a secret of the OIDC exchange: the client secret the
/// provider issued, or a PKCE verifier. `Confidential<OidcSecret>` holds its
/// text in a shared `Secret`, so a clone copies the pointer and the text is
/// wiped when the last owner drops it. `Debug` prints no byte of it, so a
/// logged `Config` or sign-in state carries none. The role has no serde
/// form: `ambre_core::WireConfidential` is sealed to the roles of the core.
pub enum OidcSecret {}

impl ConfidentialKind for OidcSecret {
    type Inner = String;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("OidcSecret");
}

/// The sinks of an OIDC secret (`ARCH-049`). The relay's secrets module is
/// the one place the relay reads a secret (`ARCH-048`). An inherent impl
/// must sit in the crate of the type, so the sinks are a trait here.
pub trait OidcSecretSinks {
    /// The client secret, as the provider client presents it. Named in
    /// `src/oidc.rs` alone, which builds the one client (`ARCH-049`).
    fn to_client_secret(&self) -> ClientSecret;

    /// The PKCE verifier, as the code exchange presents it. Named in
    /// `src/oidc.rs` alone, which makes the one exchange (`ARCH-049`).
    fn to_pkce_verifier(&self) -> PkceCodeVerifier;

    /// Lends the text to `store`, the one call that writes it beside the
    /// sign-in state until the callback takes it back. Named in
    /// `src/redis_ops.rs` alone (`ARCH-049`). No copy is kept here.
    fn present_stored<R>(&self, store: impl FnOnce(&str) -> R) -> R;
}

impl OidcSecretSinks for Confidential<OidcSecret> {
    fn to_client_secret(&self) -> ClientSecret {
        ClientSecret::new(self.secret().expose().clone())
    }

    fn to_pkce_verifier(&self) -> PkceCodeVerifier {
        PkceCodeVerifier::new(self.secret().expose().clone())
    }

    fn present_stored<R>(&self, store: impl FnOnce(&str) -> R) -> R {
        store(self.secret().expose())
    }
}

/// The authorization code, as the code exchange presents it. Named in
/// `src/oidc.rs` alone, which makes the one exchange (`ARCH-049`).
pub fn authorization_code(
    code: &ambre_core::Credential<ambre_core::OidcCode>,
) -> AuthorizationCode {
    AuthorizationCode::new(code.secret().expose().clone())
}
