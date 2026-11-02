use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;
use std::sync::Arc;

use crate::oidc::OidcClients;
use ambre_core::{Bounded, Bounds, Defaulted, Refusal, Sec, TimeSpan};

/// Application state shared across all Axum handlers via `State<Arc<Self>>`.
///
/// Holds handles to the three external services the Blind Relay depends on.
/// PostgreSQL for metadata, Redis for sessions and real-time state,
/// S3 for encrypted blobs (snapshots, deltas, media). It also holds the OIDC
/// clients for authentication.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: rustis::client::Client,
    /// Process-wide PubSub fan-out hub. ONE Redis subscriber per topic,
    /// fanned out in-process to every WS connection watching it. Redis PubSub
    /// subscriptions are connection-global, so a single shared subscriber is
    /// the only model that supports several connections on the same topic.
    pub pubsub: crate::ws::PubSubHub,
    pub s3: S3Client,
    pub s3_bucket: crate::s3::StoreText<crate::s3::BucketName>,
    pub config: Arc<Config>,
    /// Server Ed25519 keys for minting/verifying access JWTs. Derived once at
    /// startup from `JWT_SIGNING_SEED` (shared across instances).
    pub jwt: Arc<crate::jwt::JwtKeys>,
    /// Per-provider OIDC clients ("google" -> `OidcProviderState`).
    /// Used by `GET /auth/{provider}/login` and `POST /auth/{provider}/callback`.
    pub oidc_clients: OidcClients,
}

/// Operational knobs read from environment variables at startup.
///
/// Includes URL prefixes, S3 path conventions, TTLs for sessions
/// and presigned URLs, and OIDC provider configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Access-token (JWT) lifetime in seconds (default 15 min). Short: it is not
    /// revocable before expiry, so the window of a leaked token is bounded.
    pub access_ttl: TimeSpan<Sec>,
    /// Refresh-token lifetime in seconds (default 30 days). Stored in Redis and
    /// revocable (logout / rotation).
    pub refresh_ttl: TimeSpan<Sec>,
    /// Presigned S3 URL lifetime in seconds (default 15 min).
    pub presigned_url_ttl: TimeSpan<Sec>,
    /// Maximum size in bytes of a single encrypted media blob (default 100 MiB).
    /// The declared size is validated against this cap, and then bound into the
    /// presigned PUT URL (`Content-Length`). A member cannot upload an
    /// arbitrarily large object to the bucket.
    pub media_max_size: ambre_core::ByteSize,
    /// Maximum size in bytes of a single encrypted snapshot blob (default 50 MiB).
    /// Bound into the presigned PUT URL exactly like `media_max_size`.
    pub snapshot_max_size: ambre_core::ByteSize,
    /// Maximum size in bytes of a single encrypted delta (`Update`) ciphertext
    /// carried inline over the WebSocket (default 4 MiB). A CRDT delta is small;
    /// a large blob belongs on the media path. Bounding this stops an authorized
    /// writer from pushing oversized deltas. Such deltas are persisted durably
    /// (S3 + Redis pending) and fanned out to every subscriber. That is the same
    /// storage/cost abuse the media and snapshot caps close, on the hot write
    /// path.
    pub delta_max_size: ambre_core::ByteSize,
    /// Prefix for S3 object keys (for example "dev-ambre").
    pub s3_env_prefix: crate::s3::StoreText<crate::s3::EnvPrefix>,
    /// The registration with the `google` provider, `None` when provider
    /// sign-in is off.
    pub oidc_google: Option<crate::oidc::OidcRegistration>,
    /// Dev-only switch: when on, the `POST /api/auth/dev/login` endpoint is
    /// enabled, letting `?user=<name>` authenticate as an on-the-fly user
    /// (deterministic keys, no OIDC). MUST stay off in production.
    pub dev_mode: DevMode,
    /// Whether to trust the `X-Forwarded-For` header for the client IP used in
    /// rate limiting. MUST stay `Direct` unless the server runs behind a reverse
    /// proxy that OVERWRITES `X-Forwarded-For` with the real client address
    /// (for example nginx `proxy_set_header X-Forwarded-For $remote_addr`). When
    /// `Direct`, the header is ignored and the socket peer address is used, so a client
    /// cannot spoof its rate-limit bucket by forging the header.
    pub trust_proxy_headers: ProxyTrust,
    /// Sustained inbound-message rate per WebSocket connection, in messages per
    /// second (default 60). The WS *upgrade* is rate-limited. But once a socket
    /// is open, each `Update` drives a Postgres INSERT + Redis RPUSH + S3
    /// pending write + PubSub fan-out. Without a per-connection bound, an
    /// authorized `editor` flooding updates is a storage/cost amplification.
    /// Enforced by an in-memory token bucket, so it costs no Redis round-trip.
    pub ws_msg_rate_per_sec: Bounded<RatePerSec>,
    /// Burst allowance for the per-connection message bucket (default 120). Lets a
    /// client send a short batch without being throttled, such as the initial
    /// subscribes plus a rapid edit burst. The sustained rate stays capped.
    pub ws_msg_burst: Bounded<Burst>,
    /// Maximum size in bytes of an `Ephemeral` (presence/cursor) ciphertext
    /// carried over the WebSocket (default 64 KiB). A presence blob is tiny: a
    /// cursor position and a display name. This bounds the fire-and-forget
    /// broadcast + Redis presence-hash write, mirroring `delta_max_size` for the
    /// durable `Update` path.
    pub ephemeral_max_size: ambre_core::ByteSize,
}

/// Whether the server runs in development mode, read from `DEV_MODE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DevMode {
    /// Production: no dev login, no wildcard origin, no ephemeral signing key.
    #[default]
    Off,
    /// Local development only.
    On,
}

/// Who writes the `X-Forwarded-For` header a request carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProxyTrust {
    /// The socket peer is the client: the header is the caller's to forge.
    #[default]
    Direct,
    /// A reverse proxy overwrites the header with the real client address.
    BehindProxy,
}

/// Why a per-connection message limit read at startup is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LimitRefusal {
    /// A limit of zero would refuse every message.
    #[error("a message limit must be at least 1")]
    Zero,
    /// The value does not fit the 32 bits the bucket counts in.
    #[error("a message limit must be at most {max}, not {0}", max = u32::MAX)]
    TooLarge(u64),
}

mod sealed {
    /// Closes [`super::MessageLimit`]: [`super::limit_from_env`] reads a
    /// refusal below the bound as zero, which holds for a bound of 1 alone.
    pub trait Sealed {}
}

/// A per-connection message limit: a count from 1 to `u32::MAX`, which the
/// token bucket counts in. A value is a `Bounded<K>` of one of the two
/// markers below. Each marker states the value a deployment that sets
/// nothing runs with, through [`Defaulted`]. Sealed: each marker has
/// `MIN = 1` and `MAX = u32::MAX`, which [`limit_from_env`] relies on.
pub trait MessageLimit: sealed::Sealed + Bounds<Repr = u32> + Defaulted {}

/// Reads the limit `K` from the 64-bit value its environment variable parsed
/// to. The conversion is checked: an oversized value is refused at startup,
/// never truncated (`SRV-237`).
///
/// # Errors
///
/// [`LimitRefusal::Zero`] on zero, and [`LimitRefusal::TooLarge`] past
/// `u32::MAX`.
pub fn limit_from_env<K: MessageLimit>(value: u64) -> Result<Bounded<K>, LimitRefusal> {
    let count = u32::try_from(value).map_err(|_| LimitRefusal::TooLarge(value))?;
    // Every limit runs from 1 to `u32::MAX`, so a count that fits 32 bits
    // and is refused is zero.
    Bounded::new(count).ok_or(LimitRefusal::Zero)
}

/// The sustained inbound-message rate of one WebSocket, in messages per
/// second.
pub enum RatePerSec {}

impl Bounds for RatePerSec {
    type Repr = u32;
    const MIN: u32 = 1;
    const MAX: u32 = u32::MAX;
    const NAME: &'static str = "RatePerSec";
    const REFUSAL: Refusal = Refusal::new("a message rate is at least 1, not ", "");
}

impl Defaulted for RatePerSec {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::of::<60>();
}

impl sealed::Sealed for RatePerSec {}
impl MessageLimit for RatePerSec {}

/// The burst allowance of one WebSocket: the messages it may send at once.
pub enum Burst {}

impl Bounds for Burst {
    type Repr = u32;
    const MIN: u32 = 1;
    const MAX: u32 = u32::MAX;
    const NAME: &'static str = "Burst";
    const REFUSAL: Refusal = Refusal::new("a message burst is at least 1, not ", "");
}

impl Defaulted for Burst {
    const DEFAULT: Bounded<Self> = Bounded::<Self>::of::<120>();
}

impl sealed::Sealed for Burst {}
impl MessageLimit for Burst {}

impl Default for Config {
    fn default() -> Self {
        Self {
            access_ttl: TimeSpan::<Sec>::new(900),
            refresh_ttl: TimeSpan::<Sec>::new(30 * 24 * 3600),
            presigned_url_ttl: TimeSpan::<Sec>::new(900),
            media_max_size: ambre_core::ByteSize::new(100 * 1024 * 1024),
            snapshot_max_size: ambre_core::ByteSize::new(50 * 1024 * 1024),
            delta_max_size: ambre_core::ByteSize::new(4 * 1024 * 1024),
            s3_env_prefix: crate::s3::StoreText::<crate::s3::EnvPrefix>::new(
                "dev-ambre".to_owned(),
            ),
            oidc_google: None,
            dev_mode: DevMode::Off,
            trust_proxy_headers: ProxyTrust::Direct,
            ws_msg_rate_per_sec: Bounded::default(),
            ws_msg_burst: Bounded::default(),
            ephemeral_max_size: ambre_core::ByteSize::new(64 * 1024),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Bounded, Burst, LimitRefusal, RatePerSec, limit_from_env};

    #[test]
    fn a_message_limit_past_u32_is_refused_not_truncated() {
        // `u32::MAX + 61` used to truncate to 60 through `as u32`.
        let past = u64::from(u32::MAX) + 61;
        assert_eq!(
            limit_from_env::<RatePerSec>(past),
            Err(LimitRefusal::TooLarge(past))
        );
        assert_eq!(
            limit_from_env::<Burst>(past),
            Err(LimitRefusal::TooLarge(past))
        );
        assert_eq!(limit_from_env::<RatePerSec>(0), Err(LimitRefusal::Zero));
        assert_eq!(limit_from_env::<Burst>(0), Err(LimitRefusal::Zero));
        assert_eq!(limit_from_env::<RatePerSec>(60).map(Bounded::get), Ok(60));
        assert_eq!(
            limit_from_env::<Burst>(u64::from(u32::MAX)).map(Bounded::get),
            Ok(u32::MAX)
        );
    }

    #[test]
    fn a_deployment_that_sets_nothing_runs_with_the_stated_limits() {
        assert_eq!(Bounded::<RatePerSec>::default().get(), 60);
        assert_eq!(Bounded::<Burst>::default().get(), 120);
        assert_eq!(format!("{:?}", Bounded::<Burst>::default()), "Burst(120)");
    }
}
