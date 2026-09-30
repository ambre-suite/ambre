use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rustis::commands::ScriptingCommands;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::AppError;
use crate::state::AppState;

/// Atomic sliding-window rate-limit script (M6).
///
/// Replaces the fixed-window `INCR`/`EXPIRE` pair. That pair allowed a 2x burst
/// across the window boundary, and could leave a TTL-less key if the process
/// crashed between the two commands. One sorted set per limiter key holds the
/// timestamps of recent requests. Each call prunes entries older than the window,
/// records the current request, counts the survivors, and refreshes the key TTL.
/// It does all that atomically, so there is no boundary burst and no partial
/// state.
///
/// - `KEYS[1]` = `rate:{prefix}:{key}` (sorted set: member -> request time ms)
/// - `ARGV[1]` = now (ms)
/// - `ARGV[2]` = window (ms)
/// - `ARGV[3]` = unique member for this request
///
/// Returns the number of requests in the current sliding window (this one
/// included).
const SLIDING_WINDOW_LUA: &str = r#"
local now    = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
redis.call('ZREMRANGEBYSCORE', KEYS[1], 0, now - window)
redis.call('ZADD', KEYS[1], now, ARGV[3])
local count = redis.call('ZCARD', KEYS[1])
redis.call('PEXPIRE', KEYS[1], window)
return count
"#;

/// Process-local monotonic counter, appended to each sorted-set member. Two
/// requests landing in the same millisecond (even across the same key) stay
/// distinct entries, instead of one clobbering the other.
static REQUEST_NONCE: AtomicU64 = AtomicU64::new(0);

/// Random per-instance id, mixed into each sorted-set member so two pods cannot
/// collide on the same member. The process id is NOT enough. Containers usually
/// run the server as PID 1, and each process's `REQUEST_NONCE` restarts at 0.
/// Two pods handling their first request in the same millisecond would then
/// produce an identical member. One `ZADD` would clobber the other,
/// undercounting the shared window. A fresh random id per boot removes that
/// overlap.
static INSTANCE_ID: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().simple().to_string());

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

    /// Whether a Redis outage should fail *closed* (reject) rather than
    /// fail-open (allow). The auth tier gates brute-force-sensitive endpoints,
    /// so a limiter blackout there must not become an open door (M6). The API
    /// and WS tiers stay fail-open so a transient Redis blip does not take the
    /// whole product offline.
    fn fail_closed(&self) -> bool {
        matches!(self, RateLimitCategory::Auth)
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
    // The limiter runs inside the router nested under `/api`, which has already
    // stripped that prefix from `req.uri()`. The classifier matches the path the
    // client sent, so it reads the original URI the nesting preserved (`SRV-093`).
    let path = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| req.uri().path(), |original| original.0.path())
        .to_string();
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
    // Only consult `X-Forwarded-For` when the operator has vouched for a proxy
    // that overwrites it. Otherwise it is attacker-controlled and must be
    // ignored, so the socket peer address is authoritative.
    let x_forwarded_for = if state.config.trust_proxy_headers {
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    } else {
        None
    };
    let remote_ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|a| a.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let key = rate_limit_key(&state, &auth_header, x_forwarded_for.as_deref(), &remote_ip);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let window_ms = window_secs.saturating_mul(1000);
    // No window bucket in the key: the sorted set is the sliding window.
    let redis_key = format!("rate:{}:{}", category.prefix(), key);

    let (allowed, remaining) =
        match check_sliding_window(&state, &redis_key, now_ms, window_ms, max_req).await {
            Ok((allowed, rem)) => (allowed, rem),
            Err(e) if category.fail_closed() => {
                tracing::error!("Rate limiter Redis error on auth tier: {e:?}, failing closed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [("Retry-After", window_secs.to_string())],
                    axum::Json(serde_json::json!({
                        "error": "RATE_LIMITER_UNAVAILABLE",
                        "message": "Service temporarily unavailable, please retry",
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::warn!("Rate limiter Redis error: {e:?}, allowing request");
                return next.run(req).await;
            }
        };

    let reset = (now_ms + window_ms) / 1000;

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

/// Records the current request in the sliding window for `key` and returns
/// whether it is allowed along with the remaining quota.
///
/// Runs `SLIDING_WINDOW_LUA` atomically: prune-add-count-expire in one round
/// trip, so there is no boundary burst and no TTL-less key window.
async fn check_sliding_window(
    state: &AppState,
    key: &str,
    now_ms: u64,
    window_ms: u64,
    max_requests: u64,
) -> Result<(bool, u64), AppError> {
    let nonce = REQUEST_NONCE.fetch_add(1, Ordering::Relaxed);
    // Unique per (instance, request), so same-millisecond requests are distinct
    // sorted-set members even across pods (see `INSTANCE_ID`).
    let member = format!("{now_ms}-{}-{nonce}", *INSTANCE_ID);

    let count: i64 = state
        .redis
        .eval(
            SLIDING_WINDOW_LUA,
            [key],
            [now_ms.to_string(), window_ms.to_string(), member],
        )
        .await
        .map_err(|e| AppError::Redis(format!("rate limit sliding-window eval: {e}")))?;

    let count_u64 = count.max(0) as u64;
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
        assert_eq!(client_ip_str(None, "192.168.1.1"), "192.168.1.1");
    }

    #[test]
    fn client_ip_ignores_empty_forwarded() {
        assert_eq!(client_ip_str(Some(""), "remote"), "remote");
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
        let category =
            RateLimitCategory::classify("/api/documents/abc123/snapshot", &axum::http::Method::GET);
        assert!(matches!(category, RateLimitCategory::Api));
    }

    #[test]
    fn only_auth_tier_fails_closed() {
        // The auth tier gates brute-force-sensitive endpoints, so a limiter
        // outage there must reject; API/WS stay fail-open (M6).
        assert!(RateLimitCategory::Auth.fail_closed());
        assert!(!RateLimitCategory::Api.fail_closed());
        assert!(!RateLimitCategory::WebSocket.fail_closed());
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
        let category =
            RateLimitCategory::classify("/api/organizations/abc/members", &axum::http::Method::GET);
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
