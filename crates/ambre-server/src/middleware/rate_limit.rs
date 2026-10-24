use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rustis::commands::ScriptingCommands;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::AppError;
use crate::state::{AppState, ProxyTrust};

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
static INSTANCE_ID: std::sync::LazyLock<InstanceId> =
    std::sync::LazyLock::new(|| InstanceId(uuid::Uuid::new_v4()));

/// The random id of one server boot. It prints as 32 hex digits, the form a
/// sorted-set member carries.
struct InstanceId(uuid::Uuid);

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.simple().fmt(f)
    }
}

/// The bucket a request counts against: its caller when it signs in, its
/// peer address otherwise (`SRV-019`, `SRV-094`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RateBucket {
    /// The user the access token names.
    User(ambre_core::UserId),
    /// The address the request comes from, as the socket or the proxy states
    /// it; `unknown` when neither does.
    Peer(String),
}

impl std::fmt::Display for RateBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(user) => write!(f, "user:{user}"),
            Self::Peer(peer) => write!(f, "ip:{peer}"),
        }
    }
}

/// The bucket a request lands in.
///
/// Prefers the authenticated user id (verified statelessly from the
/// `Authorization: Bearer <jwt>` access token), falling back to the client IP
/// address (`SRV-019`, `SRV-094`). No Redis lookup: the JWT carries the user id
/// in its claims. `X-Forwarded-For` is read only when `trust_proxy` says a
/// proxy overwrites it. Otherwise it is the caller's to forge, so the socket
/// peer decides.
fn request_bucket(jwt: &crate::jwt::JwtKeys, trust_proxy: ProxyTrust, req: &Request) -> RateBucket {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    if let Some(token) = header("authorization").and_then(|h| h.strip_prefix("Bearer "))
        && let Ok(claims) = jwt.verify(&ambre_protocol::ws::AccessToken::new(
            token.trim().to_owned(),
        ))
    {
        return RateBucket::User(claims.sub);
    }
    let x_forwarded_for =
        header("x-forwarded-for").filter(|_| trust_proxy == ProxyTrust::BehindProxy);
    let remote_ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map_or_else(|| "unknown".to_owned(), |a| a.0.ip().to_string());
    RateBucket::Peer(client_ip_str(x_forwarded_for, &remote_ip))
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
    fn limits(&self) -> (u64, ambre_core::Seconds) {
        use ambre_core::Seconds;
        match self {
            RateLimitCategory::Auth => (10, Seconds(60)),
            RateLimitCategory::Api => (300, Seconds(60)),
            RateLimitCategory::WebSocket => (10, Seconds(60)),
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
    let (max_req, window) = category.limits();

    let bucket = request_bucket(&state.jwt, state.config.trust_proxy_headers, &req);

    let now = ambre_core::UnixMillis(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
    );
    let window_span = ambre_core::Millis::from_duration(window.to_duration());
    let verdict =
        match check_sliding_window(&state, &category, &bucket, now, window_span, max_req).await {
            Ok(verdict) => verdict,
            Err(e) if category.fail_closed() => {
                tracing::error!("Rate limiter Redis error on auth tier: {e:?}, failing closed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [("Retry-After", window.0.to_string())],
                    crate::error::ErrorBody::json(
                        ambre_core::RefusalCode::RateLimiterUnavailable,
                        "Service temporarily unavailable, please retry",
                    ),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::warn!("Rate limiter Redis error: {e:?}, allowing request");
                return next.run(req).await;
            }
        };

    let reset = now.plus(window_span).to_unix_seconds().0;

    let Verdict::Allowed { remaining } = verdict else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("X-RateLimit-Limit", max_req.to_string()),
                ("X-RateLimit-Remaining", "0".to_string()),
                ("X-RateLimit-Reset", reset.to_string()),
                ("Retry-After", window.0.to_string()),
            ],
            crate::error::ErrorBody::json(
                ambre_core::RefusalCode::RateLimited,
                "Too many requests, please try again later",
            ),
        )
            .into_response();
    };

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

/// What the sliding window says of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The request fits, with `remaining` more allowed in the window.
    Allowed { remaining: u64 },
    /// The window is full.
    Refused,
}

/// Records the current request in the sliding window of `bucket` under
/// `category`, and says whether it is allowed, with the quota left.
///
/// Runs `SLIDING_WINDOW_LUA` atomically: prune-add-count-expire in one round
/// trip, so there is no boundary burst and no TTL-less key window.
async fn check_sliding_window(
    state: &AppState,
    category: &RateLimitCategory,
    bucket: &RateBucket,
    now: ambre_core::UnixMillis,
    window: ambre_core::Millis,
    max_requests: u64,
) -> Result<Verdict, AppError> {
    // No window bucket in the key: the sorted set is the sliding window.
    let key = format!("rate:{}:{bucket}", category.prefix());
    let nonce = REQUEST_NONCE.fetch_add(1, Ordering::Relaxed);
    // Unique per (instance, request), so same-millisecond requests are distinct
    // sorted-set members even across pods (see `INSTANCE_ID`).
    let member = format!("{}-{}-{nonce}", now.0, *INSTANCE_ID);

    let count: i64 = state
        .redis
        .eval(
            SLIDING_WINDOW_LUA,
            [key.as_str()],
            [now.0.to_string(), window.0.to_string(), member],
        )
        .await
        .map_err(|e| AppError::Redis(format!("rate limit sliding-window eval: {e}")))?;

    let count_u64 = count.max(0) as u64;
    Ok(if count_u64 <= max_requests {
        Verdict::Allowed {
            remaining: max_requests.saturating_sub(count_u64),
        }
    } else {
        Verdict::Refused
    })
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

    // -- request_bucket ----------------------------------------------------

    /// A request from `peer`, with the headers given.
    fn request_from(peer: [u8; 4], headers: &[(&str, &str)]) -> Request {
        let mut builder = axum::http::Request::get("/api/documents");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut req = builder.body(axum::body::Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                peer, 0,
            ))));
        req
    }

    /// A bearer header for a fresh caller, under `jwt`.
    fn bearer_for(jwt: &crate::jwt::JwtKeys) -> (ambre_core::UserId, String) {
        let user = ambre_core::UserId::new();
        let now = crate::s3::unix_now();
        let claims = crate::jwt::Claims {
            sub: user,
            spk: None,
            iat: now,
            exp: now.plus(ambre_core::Seconds(900)),
        };
        (
            user,
            format!("Bearer {}", jwt.mint(&claims).unwrap().expose()),
        )
    }

    /// `SRV-019`: an authenticated request is bucketed by its caller, from
    /// whatever address it comes. A token that does not verify is no caller.
    #[test]
    fn an_authenticated_request_is_bucketed_by_its_caller() {
        let jwt = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([5u8; 32]));
        let (user, bearer) = bearer_for(&jwt);
        for peer in [[10, 0, 0, 1], [10, 0, 0, 2]] {
            let req = request_from(peer, &[("authorization", &bearer)]);
            assert_eq!(
                request_bucket(&jwt, ProxyTrust::Direct, &req),
                RateBucket::User(user)
            );
        }
        let forged = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([6u8; 32]));
        let req = request_from([10, 0, 0, 1], &[("authorization", &bearer)]);
        assert_eq!(
            request_bucket(&forged, ProxyTrust::Direct, &req),
            RateBucket::Peer("10.0.0.1".to_owned())
        );
    }

    /// `SRV-094`: a request with no caller is bucketed by its socket peer. A
    /// forwarded address counts only where the deployment declares a proxy.
    #[test]
    fn an_anonymous_request_is_bucketed_by_its_peer() {
        let jwt = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([5u8; 32]));
        let req = request_from([10, 0, 0, 1], &[("x-forwarded-for", "203.0.113.9")]);
        assert_eq!(
            request_bucket(&jwt, ProxyTrust::Direct, &req),
            RateBucket::Peer("10.0.0.1".to_owned())
        );
        assert_eq!(
            request_bucket(&jwt, ProxyTrust::BehindProxy, &req),
            RateBucket::Peer("203.0.113.9".to_owned())
        );
    }

    /// A bucket names its store key by kind, so a user id and a peer address
    /// that spell the same text never share a window.
    #[test]
    fn a_bucket_names_its_kind_in_the_key() {
        let user = ambre_core::UserId::new();
        assert_eq!(RateBucket::User(user).to_string(), format!("user:{user}"));
        assert_eq!(
            RateBucket::Peer(user.to_string()).to_string(),
            format!("ip:{user}")
        );
    }

    /// `SRV-095`: two callers never share a bucket, whether they sign in or
    /// not. Two anonymous peers that forge one forwarded address stay apart
    /// when no proxy is declared.
    #[test]
    fn two_callers_never_share_a_bucket() {
        let jwt = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([5u8; 32]));
        let (_, alice) = bearer_for(&jwt);
        let (_, bob) = bearer_for(&jwt);
        let signed = [
            request_from([10, 0, 0, 1], &[("authorization", &alice)]),
            request_from([10, 0, 0, 1], &[("authorization", &bob)]),
        ];
        let forged = ("x-forwarded-for", "203.0.113.9");
        let anonymous = [
            request_from([10, 0, 0, 1], &[forged]),
            request_from([10, 0, 0, 2], &[forged]),
        ];
        for [first, second] in [signed, anonymous] {
            assert_ne!(
                request_bucket(&jwt, ProxyTrust::Direct, &first),
                request_bucket(&jwt, ProxyTrust::Direct, &second)
            );
        }
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
        assert_eq!(category.limits(), (10, ambre_core::Seconds(60)));
    }

    #[test]
    fn api_limits() {
        let category = RateLimitCategory::Api;
        assert_eq!(category.limits(), (300, ambre_core::Seconds(60)));
    }

    #[test]
    fn ws_limits() {
        let category = RateLimitCategory::WebSocket;
        assert_eq!(category.limits(), (10, ambre_core::Seconds(60)));
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
