use super::oidc::authorization_code;
use super::token::{oidc_state_key, refresh_key};
use super::{OidcSecret, OidcSecretSinks, SeedText};
use ambre_core::Confidential;

#[test]
fn a_store_key_keeps_the_pattern_it_had_before_the_wrapper() {
    let token = ambre_core::Credential::<ambre_core::RefreshToken>::new("r-1".to_owned());
    assert_eq!(refresh_key(&token, str::to_owned), "refresh:r-1");
    let state = ambre_core::Credential::<ambre_core::OidcState>::new("s-1".to_owned());
    assert_eq!(oidc_state_key(&state, str::to_owned), "oidc:state:s-1");
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
