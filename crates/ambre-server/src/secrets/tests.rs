use super::oidc::authorization_code;
use super::token::{oidc_state_key, refresh_key};
use super::{OidcSecret, OidcSecretSinks, SeedText};
use ambre_core::Confidential;

/// `SRV-267`, audit `N10`: the store key of a refresh token is the SHA-256 of
/// the token. So is the key of a sign-in state. A copy of the store holds neither.
#[test]
fn a_store_key_holds_the_hash_of_the_token_and_never_the_token() {
    let token = ambre_core::Credential::<ambre_core::RefreshToken>::new("r-1".to_owned());
    assert_eq!(
        refresh_key(&token, str::to_owned),
        "refresh:a5fa777a3dc67268caea6eea038ba14177e360dba84e1db44e738dbae137e7ef"
    );
    let state = ambre_core::Credential::<ambre_core::OidcState>::new("s-1".to_owned());
    assert_eq!(
        oidc_state_key(&state, str::to_owned),
        "oidc:state:6a840baf5d8c3ff241688aeb14546e653774cd5387faf1cb982b0fbbf1fbb810"
    );
}

#[test]
fn an_oidc_secret_prints_nothing_and_reaches_the_provider_whole() {
    let secret = Confidential::<OidcSecret>::new("v3rifier".to_owned());
    assert_eq!(format!("{secret:?}"), "OidcSecret(Secret([REDACTED]))");
    assert_eq!(secret.to_pkce_verifier().secret(), "v3rifier");
    assert_eq!(secret.to_client_secret().secret(), "v3rifier");
    assert_eq!(secret.present_stored(str::to_owned), "v3rifier");
    let code = ambre_core::Credential::<ambre_core::OidcCode>::new("c0de".to_owned());
    assert_eq!(authorization_code(&code).secret(), "c0de");
}

#[test]
fn a_seed_text_prints_nothing() {
    let text = SeedText::new("ab".repeat(32));
    assert_eq!(format!("{text:?}"), "SeedText(Secret([REDACTED]))");
}
