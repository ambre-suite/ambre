use ambre_core::Count;
use ambre_core::id::{Id, RelayInstance, User};
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rustis::commands::ScriptingCommands;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::AppError;
use crate::state::{AppState, ProxyTrust};
use ambre_core::{Instant, Ms, Sec, TimeSpan, UnixMs};

/// Atomic sliding-window rate-limit script (M6).
///
/// Replaces the fixed-window `INCR`/`EXPIRE` pair. That pair allowed a 2x burst
/// across the window boundary, and could leave a TTL-less key if the process
/// crashed between the two commands. One sorted set per limiter key holds the
/// timestamps of recent requests. Each call prunes entries older than the window
/// and counts the survivors. It records the current request and refreshes the
/// key TTL only when the request fits. It does all that atomically, so there is
/// no boundary burst and no partial state.
///
/// A refused request records nothing (`SRV-286`). A flood past the limit
/// therefore never grows the set. A caller that keeps retrying regains its
/// quota once its allowed requests leave the window.
///
/// - `KEYS[1]` = `rate:{prefix}:user:{id}` or `rate:{prefix}:ip:{addr}`
///   (sorted set: member -> request time ms)
/// - `ARGV[1]` = now (ms)
/// - `ARGV[2]` = window (ms)
/// - `ARGV[3]` = unique member for this request
/// - `ARGV[4]` = the most requests the window allows
///
/// Returns the number of requests in the current sliding window with this one
/// counted, whether or not it was recorded. A number above `ARGV[4]` is a
/// refusal.
const SLIDING_WINDOW_LUA: &str = r#"
local now    = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local max    = tonumber(ARGV[4])
redis.call('ZREMRANGEBYSCORE', KEYS[1], 0, now - window)
local count = redis.call('ZCARD', KEYS[1])
if count >= max then
  return count + 1
end
redis.call('ZADD', KEYS[1], now, ARGV[3])
redis.call('PEXPIRE', KEYS[1], window)
return count + 1
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
///
/// The id is a v4 UUID, all random bits: a v7 one spends 48 of its bits on
/// the boot time.
static INSTANCE_ID: std::sync::LazyLock<Id<RelayInstance>> =
    std::sync::LazyLock::new(|| Id::from_bytes(uuid::Uuid::new_v4().into_bytes()));

/// The instance id as a sorted-set member carries it: 32 hex digits, no
/// hyphen. The `Display` of an `Id` writes the hyphenated form instead.
fn member_form(instance: &Id<RelayInstance>) -> uuid::fmt::Simple {
    uuid::Uuid::from_bytes(*instance.as_bytes()).simple()
}

/// The bucket a request counts against: its caller when it signs in, its
/// peer address otherwise (`SRV-019`, `SRV-094`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RateBucket {
    /// The user the access token names.
    User(Id<User>),
    /// The address the request comes from, as the socket or the proxy states
    /// it.
    Peer(Peer),
}

/// The bucket's segment of its store key. The user or the address is the hash
/// tag of the key, so the windows of many callers spread over a cluster
/// (`SRV-252`).
impl std::fmt::Display for RateBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(user) => write!(f, "user:{{{user}}}"),
            Self::Peer(peer) => write!(f, "ip:{{{peer}}}"),
        }
    }
}

/// The peer a request without a caller counts against.
///
/// A parsed address in its canonical form. One address therefore has one
/// spelling in the store key, and two spellings of it never get two windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Peer {
    /// The address the socket or a trusted proxy states.
    Addr(std::net::IpAddr),
    /// No address: the socket gave none, or the proxy wrote one that does not
    /// parse. Every such request shares one window, which fails toward the
    /// stricter limit.
    Unknown,
}

impl Peer {
    /// The peer `addr` names. An IPv4-mapped IPv6 address becomes its IPv4
    /// address, so one client never holds a window under each form.
    fn of(addr: std::net::IpAddr) -> Self {
        Self::Addr(addr.to_canonical())
    }

    /// The peer an `X-Forwarded-For` entry names: an address, or an address
    /// and a port as a socket address spells it. The port is dropped, so a
    /// proxy that appends it opens no window per source port.
    fn forwarded(entry: &str) -> Self {
        entry
            .parse::<std::net::IpAddr>()
            .or_else(|_| entry.parse::<std::net::SocketAddr>().map(|a| a.ip()))
            .map_or(Self::Unknown, Self::of)
    }
}

impl std::fmt::Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Addr(addr) => addr.fmt(f),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// The bucket a request lands in.
///
/// Prefers the authenticated user id (verified statelessly from the
/// `Authorization: Bearer <jwt>` access token), falling back to the client IP
/// address (`SRV-019`, `SRV-094`). No Redis lookup: the JWT carries the user id
/// in its claims. `X-Forwarded-For` is read only when `trust_proxy` declares
/// the proxies in front of the relay. Otherwise it is the caller's to forge,
/// so the socket peer decides.
fn request_bucket(jwt: &crate::jwt::JwtKeys, trust_proxy: ProxyTrust, req: &Request) -> RateBucket {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    if let Some(token) = header("authorization").and_then(|h| h.strip_prefix("Bearer "))
        && let Ok(claims) = jwt.verify(&ambre_protocol::ws::Credential::new(
            token.trim().to_owned(),
        ))
    {
        return RateBucket::User(claims.sub);
    }
    RateBucket::Peer(request_peer(trust_proxy, req))
}

/// The peer a request comes from, whatever token it carries.
fn request_peer(trust_proxy: ProxyTrust, req: &Request) -> Peer {
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    let remote = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map_or(Peer::Unknown, |a| Peer::of(a.0.ip()));
    client_peer(trust_proxy, forwarded, remote)
}

/// The bucket a request of `category` counts against.
///
/// The sign-in tier counts by peer alone. Each account a caller sets up
/// brings a token. A bucket per token would grow the rate of new accounts
/// from one address with their count (`SRV-304`). The other tiers
/// count a signed-in caller by its account (`SRV-019`).
fn bucket_for(
    category: &RateLimitCategory,
    jwt: &crate::jwt::JwtKeys,
    trust_proxy: ProxyTrust,
    req: &Request,
) -> RateBucket {
    match category {
        RateLimitCategory::Auth => RateBucket::Peer(request_peer(trust_proxy, req)),
        RateLimitCategory::Api | RateLimitCategory::WebSocket => {
            request_bucket(jwt, trust_proxy, req)
        }
    }
}

/// The peer of a request: the `X-Forwarded-For` entry `hops` places from the
/// right when `trust` declares `hops` proxies, the socket peer `remote`
/// otherwise (`SRV-285`).
///
/// Each proxy appends the address of its own peer, so the entries a caller
/// writes sit left of the ones the proxies wrote. The leftmost entry is
/// therefore the caller's choice, and a fresh one per request would open a
/// fresh window.
///
/// An entry that parses neither as an address nor as a socket address is
/// [`Peer::Unknown`], and so is a header shorter than the chain. It is never
/// the socket peer: behind a proxy, the socket peer is the proxy itself.
fn client_peer(trust: ProxyTrust, x_forwarded_for: Option<&str>, remote: Peer) -> Peer {
    let ProxyTrust::BehindProxy { hops } = trust else {
        return remote;
    };
    let Some(header) = x_forwarded_for else {
        return remote;
    };
    let mut entries = header
        .rsplit(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .peekable();
    if entries.peek().is_none() {
        return remote;
    }
    let skipped = usize::try_from(hops.get())
        .unwrap_or(usize::MAX)
        .saturating_sub(1);
    entries.nth(skipped).map_or(Peer::Unknown, Peer::forwarded)
}

/// A number of requests in one sliding window: a limit, a count or what
/// is left of the limit.
pub enum Requests {}

impl ambre_core::NarrowCount for Requests {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("RequestCount");
}

/// The limit of one category: at most `max_requests` in each `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RateLimit {
    /// The most requests one bucket makes in a window.
    max_requests: Count<Requests>,
    /// The span of the sliding window.
    window: TimeSpan<Sec>,
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
    /// The limit of the category.
    fn limits(&self) -> RateLimit {
        let max_requests = Count::<Requests>::new(match self {
            RateLimitCategory::Auth | RateLimitCategory::WebSocket => 10,
            RateLimitCategory::Api => 300,
        });
        RateLimit {
            max_requests,
            window: TimeSpan::<Sec>::new(60),
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
    ///
    /// The provider configuration needs no session and proves nothing, so it
    /// counts in the API tier. In the sign-in tier, one client behind a shared
    /// address would spend the budget of everyone behind it (`SRV-287`).
    fn classify(path: &str, _method: &axum::http::Method) -> Self {
        if path.ends_with("/ws") {
            return RateLimitCategory::WebSocket;
        }
        if path == "/api/auth/config" {
            return RateLimitCategory::Api;
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
    let RateLimit {
        max_requests: max_req,
        window,
    } = category.limits();

    let bucket = bucket_for(
        &category,
        &state.jwt,
        state.config.trust_proxy_headers,
        &req,
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(Instant::EPOCH, |d| {
            Instant::<UnixMs>::after_epoch(TimeSpan::from_duration(d))
        });
    let window_span = window.to_floor::<ambre_core::Ms>();
    let verdict =
        match check_sliding_window(&state, &category, &bucket, now, window_span, max_req).await {
            Ok(verdict) => verdict,
            Err(e) if category.fail_closed() => {
                tracing::error!("Rate limiter Redis error on auth tier: {e:?}, failing closed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [("Retry-After", window.get().to_string())],
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

    let reset = now.plus(window_span).to_unix_seconds().get();

    let Verdict::Allowed { remaining } = verdict else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("X-RateLimit-Limit", max_req.to_string()),
                ("X-RateLimit-Remaining", "0".to_string()),
                ("X-RateLimit-Reset", reset.to_string()),
                ("Retry-After", window.get().to_string()),
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
    Allowed { remaining: Count<Requests> },
    /// The window is full.
    Refused,
}

/// Says whether the current request fits the sliding window of `bucket` under
/// `category`, with the quota left, and records it only when it fits.
///
/// Runs `SLIDING_WINDOW_LUA` atomically: prune-count-add-expire in one round
/// trip, so there is no boundary burst and no TTL-less key window.
async fn check_sliding_window(
    state: &AppState,
    category: &RateLimitCategory,
    bucket: &RateBucket,
    now: Instant<UnixMs>,
    window: TimeSpan<Ms>,
    max_requests: Count<Requests>,
) -> Result<Verdict, AppError> {
    // No window bucket in the key: the sorted set is the sliding window.
    let key = format!("rate:{}:{bucket}", category.prefix());
    let nonce = REQUEST_NONCE.fetch_add(1, Ordering::Relaxed);
    // Unique per (instance, request), so same-millisecond requests are distinct
    // sorted-set members even across pods (see `INSTANCE_ID`).
    let member = format!("{}-{}-{nonce}", now.get(), member_form(&INSTANCE_ID));

    let count: i64 = state
        .redis
        .eval(
            SLIDING_WINDOW_LUA,
            [key.as_str()],
            [
                now.get().to_string(),
                window.get().to_string(),
                member,
                max_requests.get().to_string(),
            ],
        )
        .await
        .map_err(|e| AppError::Redis(format!("rate limit sliding-window eval: {e}")))?;

    let count = Count::<Requests>::new(u32::try_from(count.max(0)).unwrap_or(u32::MAX));
    Ok(if count <= max_requests {
        Verdict::Allowed {
            remaining: max_requests.minus(count),
        }
    } else {
        Verdict::Refused
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Sec, TimeSpan};

    // -- INSTANCE_ID -------------------------------------------------------

    #[test]
    fn the_instance_id_enters_a_member_as_32_hex_digits_of_a_v4_uuid() {
        let form = member_form(&INSTANCE_ID).to_string();
        assert_eq!(form.len(), 32);
        assert!(
            form.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(form, INSTANCE_ID.to_hyphenated().replace('-', ""));
        assert_eq!(
            uuid::Uuid::from_bytes(*INSTANCE_ID.as_bytes()).get_version_num(),
            4
        );
    }

    // -- client_peer -------------------------------------------------------

    /// A parsed address, for a test.
    fn addr(text: &str) -> Peer {
        Peer::Addr(text.parse().unwrap())
    }

    /// A chain of `hops` trusted proxies.
    fn proxies(hops: u32) -> ProxyTrust {
        ProxyTrust::BehindProxy {
            hops: Count::new(hops),
        }
    }

    /// The peer behind one trusted proxy.
    fn behind_one(x_forwarded_for: Option<&str>, remote: Peer) -> Peer {
        client_peer(proxies(1), x_forwarded_for, remote)
    }

    #[test]
    fn client_ip_from_forwarded_header() {
        assert_eq!(
            behind_one(Some("10.0.0.1, 10.0.0.2"), addr("127.0.0.1")),
            addr("10.0.0.2")
        );
    }

    #[test]
    fn client_ip_falls_back_to_remote() {
        assert_eq!(behind_one(None, addr("192.168.1.1")), addr("192.168.1.1"));
        assert_eq!(behind_one(None, Peer::Unknown), Peer::Unknown);
        assert_eq!(
            client_peer(ProxyTrust::Direct, Some("1.2.3.4"), addr("10.0.0.7")),
            addr("10.0.0.7")
        );
    }

    #[test]
    fn client_ip_ignores_empty_forwarded() {
        assert_eq!(behind_one(Some(""), addr("10.0.0.7")), addr("10.0.0.7"));
        assert_eq!(behind_one(Some(" , "), addr("10.0.0.7")), addr("10.0.0.7"));
    }

    #[test]
    fn client_ip_trims_forwarded() {
        assert_eq!(
            behind_one(Some("  1.2.3.4  , 5.6.7.8 "), addr("10.0.0.7")),
            addr("5.6.7.8")
        );
    }

    /// `SRV-285`: behind `n` proxies, the client is the entry `n` places
    /// from the right. Whatever the caller writes left of it changes no
    /// bucket, and a header shorter than the chain is the unknown window.
    #[test]
    fn the_client_is_read_from_the_right_of_the_forwarded_chain() {
        for forged in ["", "6.6.6.6, ", "6.6.6.6, 7.7.7.7, ", "not-an-ip, "] {
            let one = format!("{forged}1.2.3.4");
            assert_eq!(behind_one(Some(&one), addr("10.0.0.7")), addr("1.2.3.4"));
            let two = format!("{forged}1.2.3.4, 10.0.0.5");
            assert_eq!(
                client_peer(proxies(2), Some(&two), addr("10.0.0.7")),
                addr("1.2.3.4")
            );
        }
        assert_eq!(
            client_peer(proxies(3), Some("1.2.3.4, 10.0.0.5"), addr("10.0.0.7")),
            Peer::Unknown
        );
    }

    /// `SRV-285`: the setting names the number of proxies, `true` names one,
    /// and anything else is no proxy.
    #[test]
    fn the_proxy_setting_names_the_length_of_the_chain() {
        assert_eq!(ProxyTrust::from_setting(Some("true")), proxies(1));
        assert_eq!(ProxyTrust::from_setting(Some("1")), proxies(1));
        assert_eq!(ProxyTrust::from_setting(Some(" 2 ")), proxies(2));
        for off in [None, Some("0"), Some("false"), Some("-1"), Some("")] {
            assert_eq!(ProxyTrust::from_setting(off), ProxyTrust::Direct);
        }
    }

    /// A forwarded entry that is no address shares the unknown window. It
    /// never falls back to the socket peer, which is the proxy.
    #[test]
    fn a_forwarded_entry_that_is_no_address_is_unknown() {
        assert_eq!(
            behind_one(Some("1.2.3.4, not-an-ip"), addr("10.0.0.7")),
            Peer::Unknown
        );
    }

    /// The store key of an address in its canonical spelling is the address
    /// as `IpAddr` prints it, between braces after `ip:`, as its hash tag.
    /// A missing address keeps `ip:{unknown}`. Any other spelling changes key
    /// on purpose, onto the canonical one: two spellings of one IPv6 address
    /// share one key.
    #[test]
    fn the_store_key_of_a_real_address_is_unchanged() {
        for (text, key) in [
            ("10.0.0.1", "ip:{10.0.0.1}"),
            ("203.0.113.9", "ip:{203.0.113.9}"),
            ("2001:db8::1", "ip:{2001:db8::1}"),
            ("::1", "ip:{::1}"),
        ] {
            assert_eq!(RateBucket::Peer(addr(text)).to_string(), key);
            assert_eq!(
                RateBucket::Peer(behind_one(Some(text), Peer::Unknown)).to_string(),
                key
            );
        }
        assert_eq!(RateBucket::Peer(Peer::Unknown).to_string(), "ip:{unknown}");
        assert_eq!(
            behind_one(Some("2001:DB8:0::1"), Peer::Unknown),
            addr("2001:db8::1")
        );
    }

    /// `SRV-094`: a forwarded entry that carries a port counts as its
    /// address alone. A proxy that appends the port opens no window per port.
    #[test]
    fn a_forwarded_entry_with_a_port_counts_as_its_address() {
        for (entry, peer) in [
            ("1.2.3.4:5678", "1.2.3.4"),
            ("[::1]:443", "::1"),
            ("1.2.3.4, [2001:db8::1]:8080", "2001:db8::1"),
        ] {
            assert_eq!(behind_one(Some(entry), addr("10.0.0.7")), addr(peer));
        }
        assert_eq!(
            behind_one(Some("1.2.3.4:port"), addr("10.0.0.7")),
            Peer::Unknown
        );
    }

    /// `SRV-094`: an IPv4 address spelled as IPv4-mapped IPv6 shares the
    /// window of the IPv4 address, from a proxy and from the socket alike. One
    /// client therefore cannot hold two windows.
    #[test]
    fn an_ipv4_mapped_address_shares_the_ipv4_window() {
        for entry in ["::ffff:1.2.3.4", "[::ffff:1.2.3.4]:80"] {
            let peer = behind_one(Some(entry), Peer::Unknown);
            assert_eq!(peer, addr("1.2.3.4"));
            assert_eq!(RateBucket::Peer(peer).to_string(), "ip:{1.2.3.4}");
        }
        let jwt = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([5u8; 32]));
        let mut req = request_from([10, 0, 0, 1], &[]);
        let mapped: std::net::SocketAddr = "[::ffff:1.2.3.4]:443".parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(mapped));
        assert_eq!(
            request_bucket(&jwt, ProxyTrust::Direct, &req),
            RateBucket::Peer(addr("1.2.3.4"))
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
    fn bearer_for(jwt: &crate::jwt::JwtKeys) -> (Id<User>, String) {
        let user = Id::<User>::mint();
        let now = crate::s3::unix_now();
        let claims = crate::jwt::Claims {
            sub: user,
            spk: None,
            iat: now,
            exp: now.plus(TimeSpan::<Sec>::new(900)),
        };
        (
            user,
            format!("Bearer {}", jwt.mint(&claims).unwrap().secret().expose()),
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
            RateBucket::Peer(addr("10.0.0.1"))
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
            RateBucket::Peer(addr("10.0.0.1"))
        );
        assert_eq!(
            request_bucket(&jwt, proxies(1), &req),
            RateBucket::Peer(addr("203.0.113.9"))
        );
    }

    /// `SRV-304`, audit `P55`: the sign-in tier counts a request by its peer,
    /// whatever token it carries, so each account set up brings no fresh
    /// bucket. The other tiers count the signed-in caller.
    #[test]
    fn the_sign_in_tier_counts_by_peer_whatever_the_token() {
        let jwt = crate::jwt::JwtKeys::from_seed(&crate::jwt::JwtSeed::from_array([5u8; 32]));
        let (user, bearer) = bearer_for(&jwt);
        let req = request_from([10, 0, 0, 1], &[("authorization", &bearer)]);
        assert_eq!(
            bucket_for(&RateLimitCategory::Auth, &jwt, ProxyTrust::Direct, &req),
            RateBucket::Peer(addr("10.0.0.1"))
        );
        for category in [RateLimitCategory::Api, RateLimitCategory::WebSocket] {
            assert_eq!(
                bucket_for(&category, &jwt, ProxyTrust::Direct, &req),
                RateBucket::User(user)
            );
        }
    }

    /// A bucket names its store key by kind, so a user id and a peer address
    /// never share a window.
    #[test]
    fn a_bucket_names_its_kind_in_the_key() {
        let user = Id::<User>::mint();
        assert_eq!(
            RateBucket::User(user).to_string(),
            format!("user:{{{user}}}")
        );
        assert_eq!(
            RateBucket::Peer(addr("10.0.0.1")).to_string(),
            "ip:{10.0.0.1}"
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

    // -- SLIDING_WINDOW_LUA -------------------------------------------------

    /// `SRV-286`: a refused request adds nothing to its bucket. Past the
    /// limit, the script answers a count above it and the set keeps the
    /// allowed requests alone, however many refusals follow.
    #[tokio::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs redis; run with --features db")]
    async fn a_refused_request_adds_nothing_to_its_bucket() {
        use rustis::commands::{GenericCommands, SortedSetCommands};
        let redis = crate::test_helpers::connect_test_redis().await.unwrap();
        let key = format!("rate:test:ip:{{{}}}", uuid::Uuid::new_v4());
        let now: u64 = 1_700_000_000_000;
        let mut answers = Vec::new();
        for nonce in 0..8u32 {
            let count: i64 = redis
                .eval(
                    SLIDING_WINDOW_LUA,
                    [key.as_str()],
                    [
                        now.to_string(),
                        "60000".to_owned(),
                        format!("member-{nonce}"),
                        "3".to_owned(),
                    ],
                )
                .await
                .unwrap();
            answers.push(count);
        }
        assert_eq!(answers, vec![1, 2, 3, 4, 4, 4, 4, 4]);
        let recorded: usize = redis.zcard(&key).await.unwrap();
        assert_eq!(recorded, 3);
        let _: usize = redis.del(&key).await.unwrap();
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

    /// `SRV-287`: the provider configuration counts in the API tier, so it
    /// spends nothing of the sign-in budget of a shared address.
    #[test]
    fn the_provider_configuration_counts_in_the_api_tier() {
        let category = RateLimitCategory::classify("/api/auth/config", &axum::http::Method::GET);
        assert!(matches!(category, RateLimitCategory::Api));
        let category = RateLimitCategory::classify("/api/auth/refresh", &axum::http::Method::POST);
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
        assert_eq!(
            category.limits(),
            RateLimit {
                max_requests: Count::<Requests>::new(10),
                window: TimeSpan::<Sec>::new(60),
            }
        );
    }

    #[test]
    fn api_limits() {
        let category = RateLimitCategory::Api;
        assert_eq!(
            category.limits(),
            RateLimit {
                max_requests: Count::<Requests>::new(300),
                window: TimeSpan::<Sec>::new(60),
            }
        );
    }

    #[test]
    fn ws_limits() {
        let category = RateLimitCategory::WebSocket;
        assert_eq!(
            category.limits(),
            RateLimit {
                max_requests: Count::<Requests>::new(10),
                window: TimeSpan::<Sec>::new(60),
            }
        );
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
