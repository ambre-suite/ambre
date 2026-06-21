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
    /// Cached Ed25519 public key -- avoids a Postgres lookup per WS message.
    #[allow(dead_code)]
    pub signing_pubkey: ambre_core::PublicKey,
}

/// Optional authenticated identity.
///
/// Missing `Authorization` is accepted as anonymous access. A present but
/// invalid bearer token is still rejected, so clients cannot silently downgrade
/// an expired or forged token to anonymous access.
#[derive(Debug, Clone)]
pub struct OptionalAuthSession(pub Option<AuthSession>);

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

/// Verify an access token and resolve the identity it carries.
pub fn authenticate_token(state: &AppState, token: &str) -> Result<AuthSession, AppError> {
    let claims = state
        .jwt
        .verify(token)
        .map_err(|_| AppError::Unauthorized)?;
    let user_id = ambre_core::Uuid::parse_str(&claims.sub)
        .map(ambre_core::UserId::from_uuid)
        .map_err(|_| AppError::Unauthorized)?;
    let signing_pubkey =
        ambre_core::PublicKey::from_hex(&claims.spk).map_err(|_| AppError::Unauthorized)?;
    Ok(AuthSession {
        user_id,
        signing_pubkey,
    })
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

impl FromRequestParts<Arc<AppState>> for OptionalAuthSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        match bearer_token(parts) {
            Some(token) => authenticate_token(state, &token).map(Some).map(Self),
            None => Ok(Self(None)),
        }
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
pub async fn issue_session(
    state: &AppState,
    user_id: ambre_core::UserId,
    signing_pubkey: ambre_core::PublicKey,
) -> Result<TokenPair, AppError> {
    let now = unix_now();
    let access_ttl = state.config.access_ttl_secs;
    let claims = Claims {
        sub: user_id.to_string(),
        spk: signing_pubkey.to_hex(),
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
