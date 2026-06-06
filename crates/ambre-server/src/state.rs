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
    pub s3: S3Client,
    pub s3_bucket: String,
    pub config: Arc<Config>,
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
    /// Session lifetime in seconds (default 24h).
    pub session_ttl_secs: u64,
    /// Presigned S3 URL lifetime in seconds (default 15 min).
    pub presigned_url_ttl_secs: u64,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".into(),
            session_ttl_secs: 86400,
            presigned_url_ttl_secs: 900,
            s3_env_prefix: "dev-ambre".into(),
            oidc_google_client_id: String::new(),
            oidc_google_client_secret: String::new(),
            oidc_google_redirect_uri: String::new(),
        }
    }
}
