use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;
use std::sync::Arc;

use crate::oidc::OidcClients;

/// Application state shared across all Axum handlers via `State<Arc<Self>>`.
///
/// Holds handles to the three external services the Blind Relay depends on.
/// PostgreSQL for metadata, Valkey/Redis for sessions and real-time state,
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
    pub s3_bucket: crate::s3::BucketName,
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
    pub access_ttl: ambre_core::Seconds,
    /// Refresh-token lifetime in seconds (default 30 days). Stored in Redis and
    /// revocable (logout / rotation).
    pub refresh_ttl: ambre_core::Seconds,
    /// Presigned S3 URL lifetime in seconds (default 15 min).
    pub presigned_url_ttl: ambre_core::Seconds,
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
    pub s3_env_prefix: crate::s3::EnvPrefix,
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
    pub ws_msg_rate_per_sec: RatePerSec,
    /// Burst allowance for the per-connection message bucket (default 120). Lets a
    /// client send a short batch without being throttled, such as the initial
    /// subscribes plus a rapid edit burst. The sustained rate stays capped.
    pub ws_msg_burst: Burst,
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

/// A per-connection message limit: a positive count that fits 32 bits.
/// Built from the 64-bit value an environment variable parses to. The
/// conversion is checked: an oversized value is refused at startup, never
/// truncated (`SRV-237`).
macro_rules! message_limit {
    ($(#[$doc:meta])* $name:ident, $default:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name(u32);

        impl $name {
            /// The value a deployment that sets nothing runs with.
            pub const DEFAULT: Self = Self($default);

            /// Reads the limit from the value its variable parsed to.
            ///
            /// # Errors
            ///
            /// [`LimitRefusal::Zero`] on zero, and [`LimitRefusal::TooLarge`]
            /// past `u32::MAX`.
            pub fn from_env(value: u64) -> Result<Self, LimitRefusal> {
                match u32::try_from(value) {
                    Ok(0) => Err(LimitRefusal::Zero),
                    Ok(count) => Ok(Self(count)),
                    Err(_) => Err(LimitRefusal::TooLarge(value)),
                }
            }

            /// The count, for the token bucket's arithmetic.
            pub const fn get(self) -> u32 {
                self.0
            }
        }
    };
}

message_limit!(
    /// The sustained inbound-message rate of one WebSocket, in messages per
    /// second.
    RatePerSec,
    60
);
message_limit!(
    /// The burst allowance of one WebSocket: the messages it may send at once.
    Burst,
    120
);

impl Default for Config {
    fn default() -> Self {
        Self {
            access_ttl: ambre_core::Seconds(900),
            refresh_ttl: ambre_core::Seconds(30 * 24 * 3600),
            presigned_url_ttl: ambre_core::Seconds(900),
            media_max_size: ambre_core::ByteSize(100 * 1024 * 1024),
            snapshot_max_size: ambre_core::ByteSize(50 * 1024 * 1024),
            delta_max_size: ambre_core::ByteSize(4 * 1024 * 1024),
            s3_env_prefix: crate::s3::EnvPrefix::new("dev-ambre".to_owned()),
            oidc_google: None,
            dev_mode: DevMode::Off,
            trust_proxy_headers: ProxyTrust::Direct,
            ws_msg_rate_per_sec: RatePerSec::DEFAULT,
            ws_msg_burst: Burst::DEFAULT,
            ephemeral_max_size: ambre_core::ByteSize(64 * 1024),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Burst, LimitRefusal, RatePerSec};

    #[test]
    fn a_message_limit_past_u32_is_refused_not_truncated() {
        // `u32::MAX + 61` used to truncate to 60 through `as u32`.
        let past = u64::from(u32::MAX) + 61;
        assert_eq!(
            RatePerSec::from_env(past),
            Err(LimitRefusal::TooLarge(past))
        );
        assert_eq!(Burst::from_env(past), Err(LimitRefusal::TooLarge(past)));
        assert_eq!(RatePerSec::from_env(0), Err(LimitRefusal::Zero));
        assert_eq!(Burst::from_env(0), Err(LimitRefusal::Zero));
        assert_eq!(RatePerSec::from_env(60).map(RatePerSec::get), Ok(60));
        assert_eq!(
            Burst::from_env(u64::from(u32::MAX)).map(Burst::get),
            Ok(u32::MAX)
        );
    }
}
