//! Blind Relay server for Ambre.

mod auth;
mod db;
mod error;
mod jwt;
mod middleware;
mod oidc;
mod redis_ops;
mod routes;
mod s3;
mod state;
mod ws;

#[cfg(any(test, feature = "test-helpers"))]
mod test_helpers;

use axum::Router;
use axum::http::HeaderValue;
use axum::middleware as axum_mw;
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
    // falls back to the default chain (ending at the EC2 metadata endpoint),
    // which is unreachable off-EC2 and surfaces as a "dispatch failure" on every
    // request. Set both explicitly. Credentials come from the environment; when
    // an endpoint override is present (local MinIO) we default to its dev root
    // user so the stack works out of the box.
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
    // shared so any instance can verify a token minted by another; absent (dev),
    // generate an ephemeral one so a restart simply forces clients to refresh.
    let jwt_seed: [u8; 32] = match std::env::var("JWT_SIGNING_SEED") {
        Ok(hex_seed) => {
            let bytes = hex::decode(hex_seed.trim())
                .map_err(|e| format!("JWT_SIGNING_SEED is not valid hex: {e}"))?;
            bytes
                .try_into()
                .map_err(|_| "JWT_SIGNING_SEED must be 32 bytes (64 hex chars)")?
        }
        Err(_) => {
            tracing::warn!(
                "JWT_SIGNING_SEED not set: generating an ephemeral signing key (tokens \
                 do not survive a restart, and multi-instance verification will fail)"
            );
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).expect("CSPRNG");
            seed
        }
    };
    let jwt_keys = std::sync::Arc::new(jwt::JwtKeys::from_seed(&jwt_seed));

    let env_ttl = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };

    let config = Config {
        s3_env_prefix,
        oidc_google_client_id: oidc_google_client_id.clone(),
        oidc_google_client_secret: oidc_google_client_secret.clone(),
        oidc_google_redirect_uri: oidc_google_redirect_uri.clone(),
        dev_mode,
        access_ttl_secs: env_ttl("ACCESS_TTL_SECS", 900),
        refresh_ttl_secs: env_ttl("REFRESH_TTL_SECS", 30 * 24 * 3600),
        ..Default::default()
    };

    // OIDC is optional (a self-host / dev deployment may run without provider
    // login). But when credentials ARE configured, a discovery failure is a
    // misconfiguration we must not silently swallow into "login disabled": log
    // at ERROR and fail-fast so the operator notices at startup (M7).
    let oidc_configured = !oidc_google_client_id.is_empty();
    let oidc_clients = match oidc::init_oidc_clients(
        &oidc_google_client_id,
        &oidc_google_client_secret,
        &oidc_google_redirect_uri,
    )
    .await
    {
        Ok(clients) => clients,
        Err(e) if oidc_configured => {
            tracing::error!("OIDC discovery failed with configured credentials: {e}");
            return Err(format!("OIDC initialization failed: {e}").into());
        }
        Err(e) => {
            tracing::warn!("OIDC not configured (provider login disabled): {e}");
            Default::default()
        }
    };

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

    let api_router = Router::new()
        .route("/auth/google/login", axum::routing::get(routes::oidc_login))
        .route(
            "/auth/google/callback",
            axum::routing::post(routes::oidc_callback),
        )
        .route("/auth/setup", axum::routing::post(routes::setup_account))
        .route("/auth/dev/login", axum::routing::post(routes::dev_login))
        .route("/auth/config", axum::routing::get(routes::auth_config))
        .route("/auth/refresh", axum::routing::post(routes::refresh))
        .route("/auth/me", axum::routing::get(routes::me))
        .route("/auth/logout", axum::routing::post(routes::logout))
        .route(
            "/keys/master",
            axum::routing::put(routes::store_master_keys).get(routes::get_master_keys),
        )
        .route(
            "/keys/master/rotate",
            axum::routing::put(routes::rotate_master_keys),
        )
        .route("/users/search", axum::routing::get(routes::search_users))
        .route(
            "/users/{id}/pubkeys",
            axum::routing::get(routes::get_pubkeys),
        )
        .route(
            "/documents",
            axum::routing::get(routes::list_documents).post(routes::create_document),
        )
        .route(
            "/documents/{id}",
            axum::routing::get(routes::get_document)
                .delete(routes::delete_document)
                .patch(routes::rename_document),
        )
        .route(
            "/documents/{id}/snapshot",
            axum::routing::get(routes::get_snapshot_metadata).post(routes::create_snapshot),
        )
        .route(
            "/documents/{id}/snapshot/upload-url",
            axum::routing::post(routes::snapshot_upload_url),
        )
        .route(
            "/documents/{id}/snapshot/download-url",
            axum::routing::post(routes::snapshot_download_url),
        )
        .route(
            "/documents/{id}/deltas",
            axum::routing::get(routes::get_deltas),
        )
        .route(
            "/documents/{id}/share",
            axum::routing::post(routes::invite_to_document),
        )
        .route(
            "/documents/{id}/share/{uid}",
            axum::routing::delete(routes::revoke_from_document),
        )
        .route(
            "/documents/{id}/members",
            axum::routing::get(routes::list_document_members),
        )
        .route(
            "/documents/{id}/members/{uid}",
            axum::routing::patch(routes::update_document_member_role),
        )
        .route(
            "/documents/{id}/rotate-key",
            axum::routing::post(routes::rotate_document_key),
        )
        .route(
            "/documents/{id}/media/upload-url",
            axum::routing::post(routes::media_upload_url),
        )
        .route(
            "/documents/{id}/media/download-url",
            axum::routing::post(routes::media_download_url),
        )
        .route(
            "/documents/{id}/media",
            axum::routing::post(routes::media_confirm_upload),
        )
        .route(
            "/organizations",
            axum::routing::get(routes::list_orgs).post(routes::create_org),
        )
        .route(
            "/organizations/{id}",
            axum::routing::get(routes::get_org)
                .patch(routes::update_org)
                .delete(routes::delete_org),
        )
        .route(
            "/organizations/{id}/members",
            axum::routing::post(routes::add_member).get(routes::list_members),
        )
        .route(
            "/organizations/{id}/members/{uid}",
            axum::routing::delete(routes::remove_member).patch(routes::update_member_role),
        )
        .route(
            "/organizations/{id}/recover/{uid}",
            axum::routing::post(routes::recover_master_key),
        )
        .route("/ws", axum::routing::get(ws::ws_upgrade))
        .layer({
            let state = state.clone();
            axum_mw::from_fn(move |req: axum::extract::Request, next: axum_mw::Next| {
                let state = Arc::clone(&state);
                async move { middleware::rate_limit::rate_limit_inner(state, req, next).await }
            })
        });

    let app = Router::new()
        .nest("/api", api_router)
        .layer(cors)
        // Per-request access log (method, path, status, latency). The span
        // records only the URI *path*, never the query string: the WS upgrade
        // carries the access JWT as `?token=`, and the default `new_for_http`
        // span would log the full URI (M15). Enable with `tower_http=debug`.
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

    axum::serve(listener, app).await?;

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
/// are kept.
async fn snapshot_compaction_task(state: state::AppState) {
    const KEEP_PER_DOCUMENT: i64 = 10;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;

        let compactable = match db::find_compactable_snapshots(&state.db, KEEP_PER_DOCUMENT).await {
            Ok(snapshots) => snapshots,
            Err(e) => {
                tracing::error!("compaction query failed: {e:?}");
                continue;
            }
        };

        if compactable.is_empty() {
            tracing::debug!("compaction: no snapshots to delete");
            continue;
        }

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

            // Delete the snapshot row (cascades to updates via FK).
            if let Err(e) = db::delete_snapshot(&state.db, snap.id).await {
                tracing::error!(
                    snapshot_id = %snap.id,
                    error = %e,
                    "compaction: DB delete failed"
                );
            }
        }
    }
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
