use axum_extra::extract::cookie::{Cookie, CookieJar};

use crate::error::AppError;
use crate::redis_ops;
use crate::state::AppState;

/// Authenticated session extracted from the `session_id` cookie.
///
/// Contains the user identity, their Ed25519 public key (cached in Redis
/// so the server can verify signatures without hitting Postgres), and the
/// opaque session id.
#[derive(Debug, Clone)]
pub struct AuthSession {
    /// User the session belongs to.
    pub user_id: ambre_core::UserId,
    /// Cached Ed25519 public key -- avoids a Postgres lookup per WS message.
    #[allow(dead_code)]
    pub signing_pubkey: ambre_core::PublicKey,
    /// Opaque session identifier stored in the `session_id` cookie.
    pub session_id: String,
}

/// Resolves a session from the `session_id` cookie using Redis.
///
/// Returns `AuthSession` on success, or an `AppError` if the cookie is
/// missing, the session expired, or the Redis lookup failed.
pub async fn authenticate(
    state: &AppState,
    jar: &CookieJar,
) -> Result<AuthSession, AppError> {
    let session_id = jar
        .get("session_id")
        .map(|c| c.value().to_string())
        .ok_or(AppError::Unauthorized)?;

    let session = redis_ops::session_get(&state.redis, &session_id)
        .await?
        .ok_or(AppError::SessionExpired)?;

    Ok(AuthSession {
        user_id: session.user_id,
        signing_pubkey: session.signing_pubkey,
        session_id,
    })
}

/// Builds a `Set-Cookie` header that stores the session id.
///
/// The cookie is `HttpOnly`, `Secure`, `SameSite=Strict`, and lives for
/// the duration configured in `Config::session_ttl_secs`.
pub fn create_session_cookie(session_id: &str) -> Cookie<'static> {
    Cookie::build(("session_id", session_id.to_string()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(86400))
        .build()
}

/// Builds an expired `Set-Cookie` header that clears the session.
pub fn delete_session_cookie() -> Cookie<'static> {
    Cookie::build(("session_id", ""))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(0))
        .build()
}
