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
/// the given `ttl_secs`.
pub fn create_session_cookie(session_id: &str, ttl_secs: u64) -> Cookie<'static> {
    Cookie::build(("session_id", session_id.to_string()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(ttl_secs as i64))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_session_cookie_http_only() {
        let c = create_session_cookie("abc123", 86400);
        assert_eq!(c.http_only(), Some(true));
    }

    #[test]
    fn create_session_cookie_secure() {
        let c = create_session_cookie("abc123", 86400);
        assert!(c.secure().unwrap_or(false));
    }

    #[test]
    fn create_session_cookie_same_site_strict() {
        let c = create_session_cookie("abc123", 86400);
        assert_eq!(c.same_site(), Some(axum_extra::extract::cookie::SameSite::Strict));
    }

    #[test]
    fn create_session_cookie_path_root() {
        let c = create_session_cookie("abc123", 86400);
        assert_eq!(c.path(), Some("/"));
    }

    #[test]
    fn create_session_cookie_contains_session_id() {
        let c = create_session_cookie("test-session-42", 86400);
        assert_eq!(c.value(), "test-session-42");
    }

    #[test]
    fn create_session_cookie_respects_ttl() {
        let c = create_session_cookie("abc123", 3600);
        assert_eq!(c.max_age(), Some(time::Duration::seconds(3600)));
    }

    #[test]
    fn delete_session_cookie_has_zero_max_age() {
        let c = delete_session_cookie();
        assert_eq!(c.max_age(), Some(time::Duration::seconds(0)));
    }

    #[test]
    fn delete_session_cookie_has_empty_value() {
        let c = delete_session_cookie();
        assert_eq!(c.value(), "");
    }

    #[test]
    fn delete_session_cookie_http_only() {
        let c = delete_session_cookie();
        assert_eq!(c.http_only(), Some(true));
    }

    #[test]
    fn delete_session_cookie_secure() {
        let c = delete_session_cookie();
        assert!(c.secure().unwrap_or(false));
    }
}
