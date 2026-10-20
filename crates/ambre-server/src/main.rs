//! Blind Relay server for Ambre.

mod auth;
mod compaction;
mod db;
mod deletion;
mod error;
mod extract;
mod hot_tail;
mod jwt;
mod middleware;
mod oidc;
mod payload;
mod redis_ops;
mod routes;
mod s3;
mod state;
mod supervise;
mod ws;

// Test-only: the crate is a binary, so no build outside the tests can call these.
#[cfg(test)]
mod test_helpers;

use axum::Router;
use axum::http::HeaderValue;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
#[allow(clippy::single_component_path_imports)]
use tracing_subscriber;

use state::{AppState, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ambre_server=debug,tower_http=info".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ambre:ambre@localhost:5432/ambre_dev".into());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let s3_endpoint = std::env::var("S3_ENDPOINT")
        .ok()
        .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok());
    let s3_bucket = std::env::var("S3_BUCKET").unwrap_or_else(|_| "ambre-dev".into());
    let s3_env_prefix = std::env::var("S3_ENV_PREFIX").unwrap_or_else(|_| "dev-ambre".into());
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    // rustis accepts a &str URL via IntoConfig trait
    let redis = rustis::client::Client::connect(redis_url.as_str())
        .await
        .map_err(|e| format!("failed to connect to Redis: {e}"))?;

    // `Config::builder()` alone carries no region or credentials, so the SDK
    // falls back to the default chain (ending at the EC2 metadata endpoint).
    // That chain is unreachable off-EC2, and surfaces as a "dispatch failure" on
    // every request. Set both explicitly. Credentials come from the environment.
    // When an endpoint override is present (local MinIO) we default to its dev
    // root user, so the stack works out of the box.
    let aws_region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
    let mut s3_config =
        aws_sdk_s3::Config::builder().region(aws_sdk_s3::config::Region::new(aws_region));
    let local_dev = s3_endpoint.is_some();
    let default_cred = || {
        if local_dev {
            "minioadmin".to_owned()
        } else {
            String::new()
        }
    };
    let access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_else(|_| default_cred());
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_else(|_| default_cred());
    s3_config = s3_config.credentials_provider(aws_sdk_s3::config::Credentials::new(
        access_key,
        secret_key,
        None,
        None,
        "ambre-env",
    ));
    if let Some(endpoint) = s3_endpoint {
        s3_config = s3_config.endpoint_url(&endpoint).force_path_style(true);
    }
    let s3 = aws_sdk_s3::Client::from_conf(s3_config.build());

    let oidc_google_client_id = std::env::var("OIDC_GOOGLE_CLIENT_ID").unwrap_or_default();
    let oidc_google_client_secret = std::env::var("OIDC_GOOGLE_CLIENT_SECRET").unwrap_or_default();
    let oidc_google_redirect_uri = std::env::var("OIDC_GOOGLE_REDIRECT_URI").unwrap_or_default();
    let dev_mode = std::env::var("DEV_MODE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
        .unwrap_or(false);
    if dev_mode {
        tracing::warn!("DEV_MODE enabled: POST /api/auth/dev/login is active (insecure, dev only)");
    }

    // Access-token signing key. In a multi-instance deployment the seed MUST be
    // shared, so any instance can verify a token minted by another. Absent
    // (dev), generate an ephemeral one so a restart simply forces clients to
    // refresh.
    let jwt_seed: [u8; 32] = match std::env::var("JWT_SIGNING_SEED") {
        Ok(hex_seed) => {
            let bytes = hex::decode(hex_seed.trim())
                .map_err(|e| format!("JWT_SIGNING_SEED is not valid hex: {e}"))?;
            bytes
                .try_into()
                .map_err(|_| "JWT_SIGNING_SEED must be 32 bytes (64 hex chars)")?
        }
        Err(_) if dev_mode => {
            tracing::warn!(
                "JWT_SIGNING_SEED not set: generating an ephemeral signing key (tokens \
                 do not survive a restart, and multi-instance verification will fail)"
            );
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).map_err(|e| format!("no CSPRNG for the JWT seed: {e}"))?;
            seed
        }
        Err(_) => {
            // Outside DEV_MODE an ephemeral key is a misconfiguration: it breaks
            // multi-instance token verification and silently logs everyone out on
            // restart. Fail fast so the operator sets a shared seed.
            return Err("JWT_SIGNING_SEED must be set (64 hex chars) outside DEV_MODE".into());
        }
    };
    let jwt_keys = std::sync::Arc::new(jwt::JwtKeys::from_seed(&jwt_seed));

    let env_ttl = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };

    // Only trust `X-Forwarded-For` when explicitly opted in (server behind a
    // reverse proxy that overwrites the header). Otherwise a client could forge
    // the header to rotate its rate-limit bucket and defeat brute-force limits.
    let trust_proxy_headers = std::env::var("TRUST_PROXY_HEADERS")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
        .unwrap_or(false);

    let config = Config {
        s3_env_prefix,
        oidc_google_client_id: oidc_google_client_id.clone(),
        oidc_google_client_secret: oidc::OidcSecret::new(oidc_google_client_secret.clone()),
        oidc_google_redirect_uri: oidc_google_redirect_uri.clone(),
        dev_mode,
        trust_proxy_headers,
        access_ttl_secs: env_ttl("ACCESS_TTL_SECS", 900),
        refresh_ttl_secs: env_ttl("REFRESH_TTL_SECS", 30 * 24 * 3600),
        media_max_bytes: ambre_core::ByteSize(env_ttl("MEDIA_MAX_BYTES", 100 * 1024 * 1024)),
        snapshot_max_bytes: ambre_core::ByteSize(env_ttl("SNAPSHOT_MAX_BYTES", 50 * 1024 * 1024)),
        delta_max_bytes: env_ttl("DELTA_MAX_BYTES", 4 * 1024 * 1024) as usize,
        ws_msg_rate_per_sec: env_ttl("WS_MSG_RATE_PER_SEC", 60) as u32,
        ws_msg_burst: env_ttl("WS_MSG_BURST", 120) as u32,
        ephemeral_max_bytes: env_ttl("EPHEMERAL_MAX_BYTES", 64 * 1024) as usize,
        ..Default::default()
    };

    // OIDC is optional (a self-host / dev deployment may run without provider
    // login). But when credentials ARE configured, a discovery failure is a
    // misconfiguration we must not silently swallow into "login disabled". Log
    // at ERROR and fail-fast, so the operator notices at startup (M7).
    let oidc_clients = match oidc::start_oidc_clients(
        oidc::GOOGLE_ISSUER,
        &oidc_google_client_id,
        &oidc_google_client_secret,
        &oidc_google_redirect_uri,
    )
    .await
    {
        Ok(clients) => clients,
        Err(e) => {
            tracing::error!("OIDC discovery failed with configured credentials: {e}");
            return Err(format!("OIDC initialization failed: {e}").into());
        }
    };
    if oidc_clients.is_empty() {
        tracing::warn!("OIDC not configured (provider login disabled)");
    }

    // One process-wide PubSub subscriber on the shared RESP3 connection, fanned
    // out in-process to interested WS connections. Cloned into every AppState.
    let pubsub = ws::PubSubHub::spawn(redis.clone());

    let state = Arc::new(AppState {
        db,
        redis: redis.clone(),
        pubsub: pubsub.clone(),
        s3,
        s3_bucket,
        config: Arc::new(config.clone()),
        jwt: jwt_keys.clone(),
        oidc_clients: oidc_clients.clone(),
    });

    // Spawn S3 flusher
    let flush_state = AppState {
        db: state.db.clone(),
        redis: redis.clone(),
        pubsub: pubsub.clone(),
        s3: state.s3.clone(),
        s3_bucket: state.s3_bucket.clone(),
        config: Arc::new(config.clone()),
        jwt: jwt_keys.clone(),
        oidc_clients: oidc_clients.clone(),
    };
    tokio::spawn(s3::global_s3_flusher(flush_state));

    // Spawn snapshot compaction task (hourly)
    let compact_state = AppState {
        db: state.db.clone(),
        redis: redis.clone(),
        pubsub: pubsub.clone(),
        s3: state.s3.clone(),
        s3_bucket: state.s3_bucket.clone(),
        config: Arc::new(config),
        jwt: jwt_keys.clone(),
        oidc_clients: oidc_clients.clone(),
    };
    tokio::spawn(snapshot_compaction_task(compact_state));

    // Config-driven, default-deny CORS (M17). `CORS_ALLOWED_ORIGINS` is a
    // comma-separated allow-list of the self-hoster's origins. A wildcard
    // (`allow_origin(Any)`) is only reachable behind `DEV_MODE`, so a production
    // deployment cannot accidentally ship an open origin.
    let cors = build_cors(
        &std::env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default(),
        dev_mode,
    );

    let app = Router::new()
        .nest("/api", routes::api_router(Arc::clone(&state)))
        // A panicking handler answers the generic `500` and its connection
        // keeps serving (`SRV-101`).
        .layer(supervise::catch_panics())
        .layer(cors)
        // Per-request access log (method, path, status, latency). The span
        // records only the URI *path*, never the query string, so a query a
        // caller adds stays out of the log (M15, `SRV-031`). The socket takes
        // its token in its first frame, never in its address. Enable with
        // `tower_http=debug`.
        .layer(TraceLayer::new_for_http().make_span_with(
            |req: &axum::http::Request<axum::body::Body>| {
                tracing::debug_span!(
                    "request",
                    method = %req.method(),
                    path = %req.uri().path(),
                )
            },
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("Ambre Blind Relay listening on {}", bind_addr);

    // `into_make_service_with_connect_info` is REQUIRED for the rate limiter to
    // see the peer socket address (`ConnectInfo<SocketAddr>`). Without it the
    // extension is absent and every unauthenticated client collapses into one
    // shared `ip:unknown` bucket.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Builds the CORS layer from a comma-separated allow-list, default-deny (M17).
///
/// - A non-empty, parseable allow-list -> exactly those origins are allowed.
/// - Empty list + `DEV_MODE` -> permissive `Any` (logged), for local dev only.
/// - Empty list + production -> no origins allowed (cross-origin denied).
///
/// `Any` is never reachable outside `DEV_MODE`, so a production deployment
/// cannot accidentally ship a wildcard origin.
fn build_cors(allowed: &str, dev_mode: bool) -> CorsLayer {
    let origins = parse_cors_origins(allowed);

    if !origins.is_empty() {
        return CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(Any)
            .allow_headers(Any);
    }

    if dev_mode {
        tracing::warn!("DEV_MODE: CORS allow_origin(Any) (insecure, dev only)");
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        tracing::warn!("CORS_ALLOWED_ORIGINS not set: all cross-origin requests will be denied");
        CorsLayer::new()
    }
}

/// Parses a comma-separated CORS allow-list into valid origin header values,
/// dropping empty and unparseable entries. Split out from `build_cors` so the
/// parsing is unit-testable without constructing a `CorsLayer`.
fn parse_cors_origins(allowed: &str) -> Vec<HeaderValue> {
    allowed
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|o| o.parse().ok())
        .collect()
}

/// Background task that periodically compacts old snapshots.
///
/// Runs once per Axum instance. Every hour it queries for snapshots
/// beyond the configured keep limit per document (default 10), deletes
/// them from S3 and PostgreSQL. Snapshots with most recent `created_at`
/// are kept. A pass that panics is lost alone, and the next hour runs the
/// next one (`SRV-101`).
async fn snapshot_compaction_task(state: state::AppState) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        supervise::contained("snapshot compaction", compact_once(&state)).await;
    }
}

/// One compaction pass: the old snapshots, the delta index they cover, then
/// the inline tails of idle documents.
async fn compact_once(state: &state::AppState) {
    const KEEP_PER_DOCUMENT: i64 = 10;

    // 1. Compact old snapshots: keep the N most recent per document, delete
    //    the rest from S3 and Postgres.
    match db::find_compactable_snapshots(&state.db, KEEP_PER_DOCUMENT).await {
        Ok(compactable) if !compactable.is_empty() => {
            tracing::info!(
                count = compactable.len(),
                "compaction: deleting old snapshots"
            );
            for snap in &compactable {
                // Delete S3 object (best-effort; orphaned objects are harmless).
                if let Err(e) = state
                    .s3
                    .delete_object()
                    .bucket(&state.s3_bucket)
                    .key(&snap.s3_key)
                    .send()
                    .await
                {
                    tracing::warn!(
                        snapshot_id = %snap.id,
                        s3_key = %snap.s3_key,
                        error = %e,
                        "compaction: S3 delete failed, removing DB row anyway"
                    );
                }

                if let Err(e) = db::delete_snapshot(&state.db, &snap.id).await {
                    tracing::error!(
                        snapshot_id = %snap.id,
                        error = %e,
                        "compaction: DB delete failed"
                    );
                }
            }
        }
        Ok(_) => tracing::debug!("compaction: no snapshots to delete"),
        Err(e) => tracing::error!("compaction query failed: {e:?}"),
    }

    // 2. Trim the delta index: drop `updates` every retained snapshot has
    //    already folded in (seq <= MIN(covered_seq) per document). Runs every
    //    cycle regardless of snapshot deletion -- a document can gain a
    //    covering snapshot without crossing the keep threshold. Without this
    //    the `updates` table grows unbounded (the dropped snapshot FK no
    //    longer cascades old deltas away).
    match hot_tail::trim(state).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(purged = n, "compaction: trimmed superseded updates"),
        Err(e) => tracing::error!("compaction: purge superseded updates failed: {e:?}"),
    }

    // 3. Move the inline tail of every idle document into a batch object.
    hot_tail::move_idle_tails(state).await;
}

#[cfg(test)]
mod cors_tests {
    use super::parse_cors_origins;

    #[test]
    fn parses_and_trims_multiple_origins() {
        let origins = parse_cors_origins(" https://a.example , https://b.example ");
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0], "https://a.example");
        assert_eq!(origins[1], "https://b.example");
    }

    #[test]
    fn empty_list_yields_no_origins() {
        assert!(parse_cors_origins("").is_empty());
        assert!(parse_cors_origins("   ").is_empty());
        assert!(parse_cors_origins(" , , ").is_empty());
    }
}

#[cfg(test)]
mod isolation_tests {
    /// The development server's configuration, the one page server this
    /// repository ships.
    const TRUNK: &str = include_str!("../../../Trunk.toml");

    /// `SRV-171`: the page that loads the client is served cross-origin
    /// isolated, both headers set on the development server.
    #[test]
    fn the_page_is_served_cross_origin_isolated() {
        let headers = TRUNK
            .split("[serve.headers]")
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .unwrap_or("");
        for line in [
            r#""Cross-Origin-Opener-Policy" = "same-origin""#,
            r#""Cross-Origin-Embedder-Policy" = "credentialless""#,
        ] {
            assert!(headers.lines().any(|l| l.trim() == line), "{line}");
        }
    }
}
