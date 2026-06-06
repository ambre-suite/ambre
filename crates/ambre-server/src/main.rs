//! Blind Relay server for Ambre.

mod auth;
mod db;
mod error;
mod redis_ops;
mod routes;
mod s3;
mod state;
mod ws;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
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
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".into());
    let s3_endpoint = std::env::var("S3_ENDPOINT")
        .ok()
        .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok());
    let s3_bucket = std::env::var("S3_BUCKET")
        .unwrap_or_else(|_| "ambre-dev".into());
    let s3_env_prefix = std::env::var("S3_ENV_PREFIX")
        .unwrap_or_else(|_| "dev-ambre".into());
    let bind_addr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".into());

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    // rustis accepts a &str URL via IntoConfig trait
    let redis = rustis::client::Client::connect(redis_url.as_str())
        .await
        .map_err(|e| format!("failed to connect to Redis: {e}"))?;

    let mut s3_config = aws_sdk_s3::Config::builder();
    if let Some(endpoint) = s3_endpoint {
        s3_config = s3_config.endpoint_url(&endpoint).force_path_style(true);
    }
    let s3 = aws_sdk_s3::Client::from_conf(s3_config.build());

    let config = Config {
        s3_env_prefix,
        ..Default::default()
    };

    let state = Arc::new(AppState {
        db,
        redis: redis.clone(),
        s3,
        s3_bucket,
        config: Arc::new(config.clone()),
    });

    // Spawn S3 flusher
    let flush_state = AppState {
        db: state.db.clone(),
        redis: redis.clone(),
        s3: state.s3.clone(),
        s3_bucket: state.s3_bucket.clone(),
        config: Arc::new(config),
    };
    tokio::spawn(s3::global_s3_flusher(flush_state));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_router = Router::new()
        .route("/auth/setup", axum::routing::post(routes::setup_account))
        .route("/auth/me", axum::routing::get(routes::me))
        .route("/auth/logout", axum::routing::post(routes::logout))
        .route("/keys/master", axum::routing::put(routes::store_master_keys).get(routes::get_master_keys))
        .route("/keys/master/rotate", axum::routing::put(routes::rotate_master_keys))
        .route("/users/search", axum::routing::get(routes::search_users))
        .route("/users/{id}/pubkeys", axum::routing::get(routes::get_pubkeys))
        .route("/documents", axum::routing::get(routes::list_documents).post(routes::create_document))
        .route("/documents/{id}", axum::routing::get(routes::get_document).delete(routes::delete_document).patch(routes::rename_document))
        .route("/documents/{id}/snapshot", axum::routing::get(routes::get_snapshot_metadata).post(routes::create_snapshot))
        .route("/documents/{id}/snapshot/upload-url", axum::routing::post(routes::snapshot_upload_url))
        .route("/documents/{id}/snapshot/download-url", axum::routing::post(routes::snapshot_download_url))
        .route("/documents/{id}/deltas", axum::routing::get(routes::get_deltas))
        .route("/documents/{id}/share", axum::routing::post(routes::invite_to_document))
        .route("/documents/{id}/share/{uid}", axum::routing::delete(routes::revoke_from_document))
        .route("/documents/{id}/members", axum::routing::get(routes::list_document_members))
        .route("/documents/{id}/members/{uid}", axum::routing::patch(routes::update_document_member_role))
        .route("/documents/{id}/rotate-key", axum::routing::post(routes::rotate_document_key))
        .route("/documents/{id}/media/upload-url", axum::routing::post(routes::media_upload_url))
        .route("/documents/{id}/media/download-url", axum::routing::post(routes::media_download_url))
        .route("/documents/{id}/media", axum::routing::post(routes::media_confirm_upload))
        .route("/ws", axum::routing::get(ws::ws_upgrade));

    let app = Router::new()
        .nest("/api", api_router)
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("Ambre Blind Relay listening on {}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
