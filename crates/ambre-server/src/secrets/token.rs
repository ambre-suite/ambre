//! The session tokens and the sign-in state, as the relay reads them.

use ambre_core::{AccessToken, Credential, OidcState, RefreshToken};
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

/// Lends the store key of a refresh-token record, `refresh:{SHA-256 of the
/// token}`, to `send`, the one store call that takes it. Named in
/// `src/redis_ops.rs` alone (`ARCH-049`).
pub fn refresh_key<R>(token: &Credential<RefreshToken>, send: impl FnOnce(&str) -> R) -> R {
    // The hash, not the token: a copy of the store holds no token that signs
    // anyone in (`SRV-267`, audit `N10`).
    use sha2::Digest as _;
    let digest = hex::encode(sha2::Sha256::digest(token.secret().expose().as_bytes()));
    send(&format!("refresh:{digest}"))
}

/// Lends the store key of a sign-in's state, `oidc:state:{SHA-256 of the
/// state}`, to `send`, as [`refresh_key`] lends its key.
pub fn oidc_state_key<R>(state: &Credential<OidcState>, send: impl FnOnce(&str) -> R) -> R {
    use sha2::Digest as _;
    let digest = hex::encode(sha2::Sha256::digest(state.secret().expose().as_bytes()));
    send(&format!("oidc:state:{digest}"))
}
