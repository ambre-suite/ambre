use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

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
pub fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
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
pub fn authenticate_account(state: &AppState, token: &str) -> Result<AccountSession, AppError> {
    let claims = state
        .jwt
        .verify(token)
        .map_err(|_| AppError::Unauthorized)?;
    let user_id = ambre_core::Uuid::parse_str(&claims.sub)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|_| AppError::Unauthorized)?;
    let signing_pubkey = claims
        .spk
        .as_deref()
        .map(ambre_core::PublicKey::from_hex)
        .transpose()
        .map_err(|_| AppError::Unauthorized)?;
    Ok(AccountSession {
        user_id,
        signing_pubkey,
    })
}

/// Verify an access token and resolve the identity it carries. A session
/// without a signing key is refused with [`AppError::SetupRequired`].
pub fn authenticate_token(state: &AppState, token: &str) -> Result<AuthSession, AppError> {
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
    pub access_token: String,
    pub refresh_token: String,
    /// Access-token lifetime in seconds, so the client can schedule a refresh.
    pub access_expires_in: u64,
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
    let now = unix_now();
    let access_ttl = state.config.access_ttl_secs;
    let claims = Claims {
        sub: user_id.to_string(),
        spk: signing_pubkey.as_ref().map(ambre_core::PublicKey::to_hex),
        iat: now,
        exp: now + access_ttl,
    };
    let access_token = state
        .jwt
        .mint(&claims)
        .map_err(|e| AppError::Internal(format!("jwt mint: {e}")))?;

    let refresh_token = uuid::Uuid::new_v4().to_string();
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

/// Current Unix time in seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
