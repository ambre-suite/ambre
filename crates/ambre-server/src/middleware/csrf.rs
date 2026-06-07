use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::Cookie;

/// Name of the non-HttpOnly cookie that holds the CSRF token.
pub const CSRF_COOKIE: &str = "csrf_token";

/// Name of the custom header that the client must echo back.
pub const CSRF_HEADER: &str = "X-CSRF-Token";

/// Parses a cookie value from a Cookie header string by name.
fn get_cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=')
            && key.trim() == name
        {
            return Some(value.trim());
        }
    }
    None
}

/// Paths exempt from CSRF validation.
///
/// These are routes that either are safe (GET), create a session (no
/// token yet), or use an alternative check (OIDC PKCE state, WebSocket).
fn is_csrf_exempt(path: &str, method: &Method) -> bool {
    if !matches!(
        method,
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    ) {
        return true;
    }
    // OIDC flow is protected by PKCE state, not by this cookie.
    // setup_account creates the session and the CSRF cookie.
    // The WebSocket upgrade is a GET, already exempt above.
    path.starts_with("/api/auth/")
}

/// Tower middleware implementing Double Submit Cookie CSRF protection.
///
/// On every mutating request (POST, PUT, PATCH, DELETE), compares the
/// `csrf_token` cookie against the `X-CSRF-Token` header. If they do
/// not match (or are missing), the request is rejected with 403.
///
/// # Exemptions
/// - Safe methods (GET, HEAD, OPTIONS) pass through.
/// - `/api/auth/*` paths are exempt (OIDC login/callback use PKCE,
///   setup_account creates the first cookie, logout clears it).
pub async fn csrf_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    if is_csrf_exempt(&path, &method) {
        return next.run(req).await;
    }

    let csrf_cookie = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| get_cookie_value(h, CSRF_COOKIE))
        .map(|s| s.to_string());

    let csrf_header = req
        .headers()
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    match (csrf_cookie, csrf_header) {
        (Some(cookie_val), Some(header_val)) if cookie_val == header_val => next.run(req).await,
        _ => (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "error": "CSRF_INVALID",
                "message": "CSRF token missing or does not match cookie",
            })),
        )
            .into_response(),
    }
}

/// Creates a CSRF token cookie.
///
/// The cookie is NOT HttpOnly so JavaScript can read it and echo it
/// back in the `X-CSRF-Token` header. It is `SameSite=Strict` and
/// `Secure` so it cannot be read by cross-origin scripts.
/// The max_age should match the session TTL so the token survives
/// browser restarts (otherwise a persistent session cookie with no
/// matching CSRF token breaks all POST requests).
pub fn create_csrf_cookie(token: &str, max_age_secs: u64) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE, token.to_string()))
        .path("/")
        .http_only(false)
        .secure(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(max_age_secs as i64))
        .build()
}

/// Creates an expired CSRF cookie to clear the token on logout.
pub fn delete_csrf_cookie() -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE, ""))
        .path("/")
        .http_only(false)
        .secure(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds(0))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn safe_methods_are_exempt() {
        let get = Method::GET;
        let head = Method::HEAD;
        let options = Method::OPTIONS;
        assert!(is_csrf_exempt("/api/documents", &get));
        assert!(is_csrf_exempt("/api/documents", &head));
        assert!(is_csrf_exempt("/api/documents", &options));
    }

    #[test]
    fn auth_routes_are_exempt() {
        let post = Method::POST;
        assert!(is_csrf_exempt("/api/auth/google/login", &Method::GET));
        assert!(is_csrf_exempt("/api/auth/google/callback", &post));
        assert!(is_csrf_exempt("/api/auth/setup", &post));
        assert!(is_csrf_exempt("/api/auth/logout", &post));
        assert!(is_csrf_exempt("/api/auth/me", &Method::GET));
    }

    #[test]
    fn api_routes_require_csrf() {
        let post = Method::POST;
        let delete = Method::DELETE;
        assert!(!is_csrf_exempt("/api/documents", &post));
        assert!(!is_csrf_exempt("/api/documents/abc", &delete));
        assert!(!is_csrf_exempt("/api/keys/master/rotate", &Method::PUT));
    }

    #[test]
    fn get_cookie_parses_simple_value() {
        let header = "session_id=abc123; csrf_token=xyz789";
        assert_eq!(get_cookie_value(header, "csrf_token"), Some("xyz789"));
        assert_eq!(get_cookie_value(header, "session_id"), Some("abc123"));
    }

    #[test]
    fn get_cookie_returns_none_for_missing() {
        assert_eq!(get_cookie_value("a=1; b=2", "c"), None);
    }

    #[test]
    fn get_cookie_handles_whitespace() {
        assert_eq!(get_cookie_value(" a = 1 ; b = 2 ", "b"), Some("2"));
    }

    #[test]
    fn csrf_cookie_is_not_http_only() {
        let c = create_csrf_cookie("test-token", 86400);
        assert_eq!(c.http_only(), Some(false));
    }

    #[test]
    fn csrf_cookie_is_secure() {
        let c = create_csrf_cookie("test-token", 86400);
        assert!(c.secure().unwrap_or(false));
    }

    #[test]
    fn csrf_cookie_same_site_strict() {
        let c = create_csrf_cookie("test-token", 86400);
        assert_eq!(
            c.same_site(),
            Some(axum_extra::extract::cookie::SameSite::Strict)
        );
    }

    #[test]
    fn csrf_cookie_has_max_age() {
        let c = create_csrf_cookie("test-token", 3600);
        assert_eq!(c.max_age(), Some(time::Duration::seconds(3600)));
    }

    #[test]
    fn delete_csrf_cookie_has_zero_max_age() {
        let c = delete_csrf_cookie();
        assert_eq!(c.max_age(), Some(time::Duration::seconds(0)));
    }

    #[test]
    fn delete_csrf_cookie_has_empty_value() {
        let c = delete_csrf_cookie();
        assert_eq!(c.value(), "");
    }
}
