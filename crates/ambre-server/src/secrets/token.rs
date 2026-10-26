//! The session tokens and the sign-in state, as the relay reads them.

use ambre_core::{AccessToken, OidcState, RefreshToken};
use jsonwebtoken::{DecodingKey, TokenData, Validation};
use serde::de::DeserializeOwned;

/// Decodes and checks an access token: its signature under `key`, and the
/// claims `validation` asks for.
///
/// # Errors
///
/// The error of `jsonwebtoken` when the token does not verify.
pub fn decode_access<C: DeserializeOwned>(
    token: &AccessToken,
    key: &DecodingKey,
    validation: &Validation,
) -> Result<TokenData<C>, jsonwebtoken::errors::Error> {
    jsonwebtoken::decode::<C>(token.secret().expose(), key, validation)
}

/// The store key of a refresh-token record: `refresh:{token}`.
pub fn refresh_key(token: &RefreshToken) -> String {
    format!("refresh:{}", token.secret().expose())
}

/// The store key of a sign-in's state: `oidc:state:{state}`.
pub fn oidc_state_key(state: &OidcState) -> String {
    format!("oidc:state:{}", state.secret().expose())
}
