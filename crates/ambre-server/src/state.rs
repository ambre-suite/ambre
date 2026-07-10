use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;
use std::sync::Arc;

use crate::oidc::OidcClients;

/// Application state shared across all Axum handlers via `State<Arc<Self>>`.
///
/// Holds handles to the three external services the Blind Relay depends on:
/// PostgreSQL for metadata, Valkey/Redis for sessions and real-time state,
/// S3 for encrypted blobs (snapshots, deltas, media), and OIDC clients for
/// authentication.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: rustis::client::Client,
    /// Process-wide PubSub fan-out hub. ONE Redis subscriber per channel,
    /// fanned out in-process to every WS connection watching it (Redis PubSub
    /// subscriptions are connection-global, so a single shared subscriber is
    /// the only model that supports several connections on the same channel).
    pub pubsub: crate::ws::PubSubHub,
    pub s3: S3Client,
    pub s3_bucket: String,
    pub config: Arc<Config>,
    /// Server Ed25519 keys for minting/verifying access JWTs. Derived once at
    /// startup from `JWT_SIGNING_SEED` (shared across instances).
    pub jwt: Arc<crate::jwt::JwtKeys>,
    /// Per-provider OIDC clients ("google" → `OidcProviderState`).
    /// Used by `GET /auth/{provider}/login` and `POST /auth/{provider}/callback`.
    pub oidc_clients: OidcClients,
}

/// Operational knobs read from environment variables at startup.
///
/// Includes URL prefixes, S3 path conventions, TTLs for sessions
/// and presigned URLs, and OIDC provider configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Base URL of this server, used for redirect callbacks (OIDC).
    #[allow(dead_code)]
    pub base_url: String,
    /// Access-token (JWT) lifetime in seconds (default 15 min). Short: it is not
    /// revocable before expiry, so the window of a leaked token is bounded.
    pub access_ttl_secs: u64,
    /// Refresh-token lifetime in seconds (default 30 days). Stored in Redis and
    /// revocable (logout / rotation).
    pub refresh_ttl_secs: u64,
    /// Presigned S3 URL lifetime in seconds (default 15 min).
    pub presigned_url_ttl_secs: u64,
    /// Maximum size in bytes of a single encrypted media blob (default 100 MiB).
    /// The declared size is validated against this cap and then bound into the
    /// presigned PUT URL (`Content-Length`), so a member cannot upload an
    /// arbitrarily large object to the bucket.
    pub media_max_bytes: u64,
    /// Maximum size in bytes of a single encrypted snapshot blob (default 50 MiB).
    /// Bound into the presigned PUT URL exactly like `media_max_bytes`.
    pub snapshot_max_bytes: u64,
    /// Maximum size in bytes of a single encrypted delta (`Update`) ciphertext
    /// carried inline over the WebSocket (default 4 MiB). A CRDT delta is small;
    /// a large blob belongs on the media path. Bounding this stops an authorized
    /// writer from pushing oversized deltas that are persisted durably (S3 +
    /// Redis pending) and fanned out to every subscriber -- the same storage/cost
    /// abuse the media and snapshot caps close, on the hot write path.
    pub delta_max_bytes: usize,
    /// Prefix for S3 object keys (e.g. "dev-ambre").
    pub s3_env_prefix: String,
    /// Google OIDC client ID.
    #[allow(dead_code)]
    pub oidc_google_client_id: String,
    /// Google OIDC client secret.
    #[allow(dead_code)]
    pub oidc_google_client_secret: String,
    /// Google OIDC redirect URI (e.g. "http://localhost:3000/api/auth/google/callback").
    #[allow(dead_code)]
    pub oidc_google_redirect_uri: String,
    /// Dev-only switch: when true, the `POST /api/auth/dev/login` endpoint is
    /// enabled, letting `?user=<name>` authenticate as an on-the-fly user
    /// (deterministic keys, no OIDC). MUST stay false in production.
    pub dev_mode: bool,
    /// Whether to trust the `X-Forwarded-For` header for the client IP used in
    /// rate limiting. MUST stay false unless the server runs behind a reverse
    /// proxy that OVERWRITES `X-Forwarded-For` with the real client address
    /// (e.g. nginx `proxy_set_header X-Forwarded-For $remote_addr`). When false,
    /// the header is ignored and the socket peer address is used, so a client
    /// cannot spoof its rate-limit bucket by forging the header.
    pub trust_proxy_headers: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".into(),
            access_ttl_secs: 900,
            refresh_ttl_secs: 30 * 24 * 3600,
            presigned_url_ttl_secs: 900,
            media_max_bytes: 100 * 1024 * 1024,
            snapshot_max_bytes: 50 * 1024 * 1024,
            delta_max_bytes: 4 * 1024 * 1024,
            s3_env_prefix: "dev-ambre".into(),
            oidc_google_client_id: String::new(),
            oidc_google_client_secret: String::new(),
            oidc_google_redirect_uri: String::new(),
            dev_mode: false,
            trust_proxy_headers: false,
        }
    }
}
