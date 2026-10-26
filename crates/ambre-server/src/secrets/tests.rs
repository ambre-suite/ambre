use super::{OidcSecret, authorization_code, oidc_state_key, refresh_key};

#[test]
fn a_store_key_keeps_the_pattern_it_had_before_the_wrapper() {
    let token = ambre_core::RefreshToken::new("r-1".to_owned());
    assert_eq!(refresh_key(&token), "refresh:r-1");
    let state = ambre_core::OidcState::new("s-1".to_owned());
    assert_eq!(oidc_state_key(&state), "oidc:state:s-1");
}

#[test]
fn an_oidc_secret_prints_nothing_and_reaches_the_provider_whole() {
    let secret = OidcSecret::new("v3rifier".to_owned());
    assert_eq!(format!("{secret:?}"), "OidcSecret(Secret([REDACTED]))");
    assert_eq!(secret.pkce_verifier().secret(), "v3rifier");
    assert_eq!(secret.client_secret().secret(), "v3rifier");
    assert_eq!(secret.stored_text(), "v3rifier");
    let code = ambre_core::OidcCode::new("c0de".to_owned());
    assert_eq!(authorization_code(&code).secret(), "c0de");
}
