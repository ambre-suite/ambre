//! The session tokens and the sign-in state, as the relay reads them.

use ambre_core::{AccessToken, Credential, OidcState, RefreshToken, Secret};
use jsonwebtoken::{DecodingKey, TokenData, Validation};
use serde::de::DeserializeOwned;

/// Decodes and checks an access token: its signature under `key`, and the
/// claims `validation` asks for. Named in `src/jwt.rs` alone, which holds
/// the key (`ARCH-049`).
///
/// # Errors
///
/// The error of `jsonwebtoken` when the token does not verify.
pub fn decode_access<C: DeserializeOwned>(
    token: &Credential<AccessToken>,
    key: &DecodingKey,
    validation: &Validation,
) -> Result<TokenData<C>, jsonwebtoken::errors::Error> {
    jsonwebtoken::decode::<C>(token.secret().expose(), key, validation)
}

/// Lends the store key of a refresh-token record, `refresh:{token}`, to
/// `send`, the one store call that takes it. Named in `src/redis_ops.rs`
/// alone (`ARCH-049`). The key is a copy of the token, so it is wiped once
/// `send` returns. The store client keeps its own copy, unwiped.
pub fn refresh_key<R>(token: &Credential<RefreshToken>, send: impl FnOnce(&str) -> R) -> R {
    let key = Secret::new(format!("refresh:{}", token.secret().expose()));
    send(key.expose())
}

/// Lends the store key of a sign-in's state, `oidc:state:{state}`, to
/// `send`, as [`refresh_key`] lends its key.
pub fn oidc_state_key<R>(state: &Credential<OidcState>, send: impl FnOnce(&str) -> R) -> R {
    let key = Secret::new(format!("oidc:state:{}", state.secret().expose()));
    send(key.expose())
}
