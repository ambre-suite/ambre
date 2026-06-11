use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rustis::commands::{GenericCommands, StringCommands};
use std::sync::Arc;

use crate::error::AppError;
use crate::state::AppState;

/// Rate-limiting key derived from the request.
///
/// Prefers the authenticated user id (verified statelessly from the
/// `Authorization: Bearer <jwt>` access token), falling back to the client IP
/// address. No Redis lookup: the JWT carries the user id in its claims.
fn rate_limit_key(
    state: &AppState,
    auth_header: &str,
    x_forwarded_for: Option<&str>,
    remote_ip: &str,
) -> String {
    if let Some(token) = auth_header.strip_prefix("Bearer ")
        && let Ok(claims) = state.jwt.verify(token.trim())
    {
        return format!("user:{}", claims.sub);
    }

    let ip = client_ip_str(x_forwarded_for, remote_ip);
    format!("ip:{ip}")
}

/// Extracts the client IP address from headers or socket address.
///
/// Respects the `X-Forwarded-For` header (first entry) when behind a
/// reverse proxy, falling back to the remote address.
fn client_ip_str(x_forwarded_for: Option<&str>, remote_ip: &str) -> String {
    if let Some(fwd) = x_forwarded_for {
        for part in fwd.split(',') {
            let ip = part.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    remote_ip.to_string()
}

/// Rate-limiting categories with different limits.
enum RateLimitCategory {
    /// Authentication endpoints (login, callback, setup).
    /// Sensitive: low limit to prevent brute-force.
    Auth,
    /// Authenticated REST API endpoints.
    Api,
    /// WebSocket upgrades.
    WebSocket,
}

impl RateLimitCategory {
    /// Returns (max_requests_per_window, window_duration_secs).
    fn limits(&self) -> (u64, u64) {
        match self {
            RateLimitCategory::Auth => (10, 60),
            RateLimitCategory::Api => (300, 60),
            RateLimitCategory::WebSocket => (10, 60),
        }
    }

    fn prefix(&self) -> &'static str {
        match self {
            RateLimitCategory::Auth => "auth",
            RateLimitCategory::Api => "api",
            RateLimitCategory::WebSocket => "ws",
        }
    }

    /// Determines the rate-limit category from the request path and method.
    fn classify(path: &str, _method: &axum::http::Method) -> Self {
        if path.ends_with("/ws") {
            return RateLimitCategory::WebSocket;
        }
        if path.starts_with("/api/auth/") {
            return RateLimitCategory::Auth;
        }
        RateLimitCategory::Api
    }
}

/// Axum middleware that enforces per-user/IP rate limiting.
///
/// Uses Redis to implement a fixed-window counter for the request rate.
/// If the limit is exceeded, the request is rejected with HTTP 429
/// (Too Many Requests) and an `X-RateLimit-*` set of headers.
///
/// This is the inner function called by the tower middleware layer.
/// It receives the state directly (not via `State` extractor) so it
/// can be wrapped in a closure-based middleware.
pub async fn rate_limit_inner(state: Arc<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let category = RateLimitCategory::classify(&path, &method);
    let (max_req, window_secs) = category.limits();

    // Extract request metadata before any await.
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let x_forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let remote_ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|a| a.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let key = rate_limit_key(
        &state,
        &auth_header,
        x_forwarded_for.as_deref(),
        &remote_ip,
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let current_window = now / window_secs;
    let redis_key = format!("rate:{}:{}:{}", category.prefix(), key, current_window);

    let (allowed, remaining) =
        match check_and_increment(&state, &redis_key, max_req, window_secs * 2).await {
            Ok((true, rem)) => (true, rem),
            Ok((false, rem)) => (false, rem),
            Err(e) => {
                tracing::warn!("Rate limiter Redis error: {e:?}, allowing request");
                return next.run(req).await;
            }
        };

    let reset = (current_window + 1) * window_secs;

    if !allowed {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("X-RateLimit-Limit", max_req.to_string()),
                ("X-RateLimit-Remaining", "0".to_string()),
                ("X-RateLimit-Reset", reset.to_string()),
                ("Retry-After", window_secs.to_string()),
            ],
            axum::Json(serde_json::json!({
                "error": "RATE_LIMITED",
                "message": "Too many requests, please try again later",
            })),
        )
            .into_response();
    }

    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    if let Ok(v) = axum::http::HeaderValue::from_str(&max_req.to_string()) {
        headers.insert("X-RateLimit-Limit", v);
    }
    if let Ok(v) = axum::http::HeaderValue::from_str(&remaining.to_string()) {
        headers.insert("X-RateLimit-Remaining", v);
    }
    if let Ok(v) = axum::http::HeaderValue::from_str(&reset.to_string()) {
        headers.insert("X-RateLimit-Reset", v);
    }
    response
}

/// Increments the Redis counter for `key` and returns whether the request
/// is allowed along with the remaining quota.
async fn check_and_increment(
    state: &AppState,
    key: &str,
    max_requests: u64,
    ttl_secs: u64,
) -> Result<(bool, u64), AppError> {
    let count: i64 = state
        .redis
        .incr(key)
        .await
        .map_err(|e| AppError::Redis(format!("rate limit incr: {e}")))?;

    // Set expiry on first increment only (when count is 1).
    if count == 1 {
        let _ = state
            .redis
            .expire(key, ttl_secs, None)
            .await
            .map_err(|e| AppError::Redis(format!("rate limit expire: {e}")))?;
    }

    let count_u64 = count as u64;
    let allowed = count_u64 <= max_requests;
    let remaining = if allowed {
        max_requests.saturating_sub(count_u64)
    } else {
        0
    };

    Ok((allowed, remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- client_ip_str -----------------------------------------------------

    #[test]
    fn client_ip_from_forwarded_header() {
        assert_eq!(
            client_ip_str(Some("10.0.0.1, 10.0.0.2"), "127.0.0.1"),
            "10.0.0.1"
        );
    }

    #[test]
    fn client_ip_falls_back_to_remote() {
        assert_eq!(
            client_ip_str(None, "192.168.1.1"),
            "192.168.1.1"
        );
    }

    #[test]
    fn client_ip_ignores_empty_forwarded() {
        assert_eq!(
            client_ip_str(Some(""), "remote"),
            "remote"
        );
    }

    #[test]
    fn client_ip_trims_forwarded() {
        assert_eq!(
            client_ip_str(Some("  1.2.3.4  , 5.6.7.8"), "remote"),
            "1.2.3.4"
        );
    }

    // -- RateLimitCategory --------------------------------------------------

    #[test]
    fn classify_auth_path() {
        let category = RateLimitCategory::classify("/api/auth/login", &axum::http::Method::POST);
        assert!(matches!(category, RateLimitCategory::Auth));
    }

    #[test]
    fn classify_auth_setup() {
        let category = RateLimitCategory::classify("/api/auth/setup", &axum::http::Method::POST);
        assert!(matches!(category, RateLimitCategory::Auth));
    }

    #[test]
    fn classify_ws_endpoint() {
        let category = RateLimitCategory::classify("/api/ws", &axum::http::Method::GET);
        assert!(matches!(category, RateLimitCategory::WebSocket));
    }

    #[test]
    fn classify_api_documents() {
        let category = RateLimitCategory::classify(
            "/api/documents/abc123/snapshot",
            &axum::http::Method::GET,
        );
        assert!(matches!(category, RateLimitCategory::Api));
    }

    #[test]
    fn classify_api_media() {
        let category = RateLimitCategory::classify(
            "/api/documents/abc/media/upload-url",
            &axum::http::Method::POST,
        );
        assert!(matches!(category, RateLimitCategory::Api));
    }

    #[test]
    fn classify_api_organizations() {
        let category = RateLimitCategory::classify(
            "/api/organizations/abc/members",
            &axum::http::Method::GET,
        );
        assert!(matches!(category, RateLimitCategory::Api));
    }

    #[test]
    fn auth_limits() {
        let category = RateLimitCategory::Auth;
        assert_eq!(category.limits(), (10, 60));
    }

    #[test]
    fn api_limits() {
        let category = RateLimitCategory::Api;
        assert_eq!(category.limits(), (300, 60));
    }

    #[test]
    fn ws_limits() {
        let category = RateLimitCategory::WebSocket;
        assert_eq!(category.limits(), (10, 60));
    }

    #[test]
    fn auth_prefix() {
        assert_eq!(RateLimitCategory::Auth.prefix(), "auth");
    }

    #[test]
    fn api_prefix() {
        assert_eq!(RateLimitCategory::Api.prefix(), "api");
    }

    #[test]
    fn ws_prefix() {
        assert_eq!(RateLimitCategory::WebSocket.prefix(), "ws");
    }
}
