use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use ambre_protocol::ws::AccessToken;

use crate::error::AppError;
use crate::jwt::Claims;
use crate::redis_ops::{self, RefreshData};
use crate::state::AppState;

/// Authenticated identity, recovered from a verified access-token (JWT).
///
/// No Redis or Postgres lookup happens to build this: the claims carry the
/// `user_id` and the Ed25519 `signing_pubkey` directly (see [`crate::jwt`]).
#[derive(Debug, Clone)]
pub struct AuthSession {
    /// User the token belongs to.
    pub user_id: ambre_core::UserId,
    /// Cached Ed25519 public key. It avoids a Postgres lookup per WS message,
    /// and binds every authored `Update` to this session identity (see the WS
    /// `handle_update` author-key check).
    pub signing_pubkey: ambre_core::PublicKey,
}

/// Extracts the bearer token from the `Authorization` header, if present.
pub fn bearer_token(parts: &Parts) -> Option<AccessToken> {
    parts
        .headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| AccessToken::new(s.trim().to_string()))
}

/// A verified access token, whether or not its account has registered keys.
///
/// Only the key setup routes take it: reading and storing the master
/// material, the caller's own profile and the sign-out. Every other route
/// takes [`AuthSession`], which refuses a session without a signing key
/// (`SRV-027`).
#[derive(Debug, Clone)]
pub struct AccountSession {
    /// User the token belongs to.
    pub user_id: ambre_core::UserId,
    /// `None` until the account registers its keys.
    pub signing_pubkey: Option<ambre_core::PublicKey>,
}

/// Verify an access token and resolve the account it names.
pub fn authenticate_account(
    state: &AppState,
    token: &AccessToken,
) -> Result<AccountSession, AppError> {
    let claims = state
        .jwt
        .verify(token)
        .map_err(|_| AppError::Unauthorized)?;
    let user_id = claims.sub;
    let signing_pubkey = claims
        .spk
        .as_ref()
        .map(|spk| ambre_core::PublicKey::from_hex(spk.as_str()))
        .transpose()
        .map_err(|_| AppError::Unauthorized)?;
    Ok(AccountSession {
        user_id,
        signing_pubkey,
    })
}

/// Verify an access token and resolve the identity it carries. A session
/// without a signing key is refused with [`AppError::SetupRequired`].
pub fn authenticate_token(state: &AppState, token: &AccessToken) -> Result<AuthSession, AppError> {
    authenticate_account(state, token)?.try_into()
}

impl TryFrom<AccountSession> for AuthSession {
    type Error = AppError;

    fn try_from(account: AccountSession) -> Result<Self, AppError> {
        Ok(AuthSession {
            user_id: account.user_id,
            signing_pubkey: account.signing_pubkey.ok_or(AppError::SetupRequired)?,
        })
    }
}

/// Axum extractor: pull `Authorization: Bearer <jwt>` and verify it. Rejects
/// with 401 when the header is missing or the token is invalid/expired.
impl FromRequestParts<Arc<AppState>> for AuthSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AppError::Unauthorized)?;
        authenticate_token(state, &token)
    }
}

/// Axum extractor for the key setup routes: like [`AuthSession`], and also
/// accepts a session whose account has no keys yet.
impl FromRequestParts<Arc<AppState>> for AccountSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AppError::Unauthorized)?;
        authenticate_account(state, &token)
    }
}

/// A freshly minted access + refresh pair, returned to the client in the login
/// and refresh response bodies (never as cookies).
pub struct TokenPair {
    pub access_token: ambre_protocol::ws::AccessToken,
    pub refresh_token: ambre_protocol::ws::RefreshToken,
    /// Access-token lifetime in seconds, so the client can schedule a refresh.
    pub access_expires_in: ambre_core::Seconds,
}

/// Mint an access JWT and create a revocable refresh token in Redis for the
/// given identity. Called by every login path (dev / OIDC / setup) and by the
/// refresh endpoint.
///
/// `signing_pubkey` is `None` for an account that has not registered its keys.
/// The session then carries no key, and only the setup routes accept it. A
/// placeholder key is never minted in its place (`SRV-027`).
pub async fn issue_session(
    state: &AppState,
    user_id: ambre_core::UserId,
    signing_pubkey: Option<ambre_core::PublicKey>,
) -> Result<TokenPair, AppError> {
    let now = crate::s3::unix_now();
    let access_ttl = access_lifetime(state.config.access_ttl_secs);
    let claims = Claims {
        sub: user_id,
        spk: signing_pubkey
            .as_ref()
            .map(|key| ambre_core::SigningKeyHex(key.to_hex())),
        iat: now,
        exp: now.plus(access_ttl),
    };
    let access_token = state
        .jwt
        .mint(&claims)
        .map_err(|e| AppError::Internal(format!("jwt mint: {e}")))?;

    let refresh_token = ambre_protocol::ws::RefreshToken::new(uuid::Uuid::new_v4().to_string());
    redis_ops::refresh_create(
        &state.redis,
        &refresh_token,
        &RefreshData {
            user_id,
            signing_pubkey,
        },
        state.config.refresh_ttl_secs,
    )
    .await?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        access_expires_in: access_ttl,
    })
}

/// The longest an access token may live, whatever the deployment configures
/// (`SRV-025`). A token is not revocable before its expiry, so this bounds the
/// window a leaked one stays usable.
pub const ACCESS_TOKEN_MAX_TTL: ambre_core::Seconds = ambre_core::Seconds(15 * 60);

/// The lifetime a minted token carries: the configured one, capped.
fn access_lifetime(configured: ambre_core::Seconds) -> ambre_core::Seconds {
    configured.min(ACCESS_TOKEN_MAX_TTL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SRV-090`: the token check reads no store. Both stores are closed
    /// before it runs, so any read would fail, and the session still resolves.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_token_check_reads_no_store(pool: sqlx::PgPool) {
        let state = crate::test_helpers::test_app_state(pool.clone()).await;
        let user = crate::test_helpers::create_test_user(&pool, "local@ambre.app").await;
        let key = ambre_core::PublicKey::from_array([3u8; 32]);
        let pair = issue_session(&state, user, Some(key)).await.unwrap();
        pool.close().await;
        state.redis.clone().close().await.unwrap();

        let (mut parts, ()) = axum::http::Request::get("/api/documents")
            .header(
                AUTHORIZATION,
                format!("Bearer {}", pair.access_token.expose()),
            )
            .body(())
            .unwrap()
            .into_parts();
        let session = AuthSession::from_request_parts(&mut parts, &state)
            .await
            .unwrap();
        assert_eq!(session.user_id, user);
        assert!(crate::db::get_user(&state.db, &user).await.is_err());
    }
}
