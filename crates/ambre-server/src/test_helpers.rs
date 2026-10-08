#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;

use crate::db;
use crate::jwt::JwtKeys;
use crate::oidc::OidcClients;
use crate::state::{AppState, Config};

/// Empty OIDC clients map -- no providers configured.
pub fn empty_oidc_clients() -> OidcClients {
    Arc::new(HashMap::new())
}

/// Builds a test [`AppState`] with a real Redis connection (falls back to
/// `redis://localhost:6379`) and a bare S3 client. Requires a running Redis;
/// S3-operations are not exercised by the auth-gating tests that use this.
pub async fn test_app_state(pool: sqlx::PgPool) -> Arc<AppState> {
    let redis = rustis::client::Client::connect("redis://localhost:6379")
        .await
        .expect("need Redis for tests");
    let s3_config = aws_sdk_s3::Config::builder().build();
    let s3 = aws_sdk_s3::Client::from_conf(s3_config);
    let pubsub = crate::ws::PubSubHub::spawn(redis.clone());
    Arc::new(AppState {
        db: pool,
        redis,
        pubsub,
        s3,
        s3_bucket: "test-bucket".into(),
        config: Arc::new(Config::default()),
        jwt: Arc::new(JwtKeys::from_seed(&[9u8; 32])),
        oidc_clients: empty_oidc_clients(),
    })
}

/// Creates a test user directly in the database (bypasses OIDC flow).
pub async fn create_test_user(pool: &PgPool, email: &str) -> ambre_core::UserId {
    let user = db::create_user(
        pool,
        email,
        "Test User",
        &[1u8; 32],
        &[2u8; 16],
        &[3u8; 32],
        &[4u8; 32],
    )
    .await
    .expect("failed to create test user");
    ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*user.id.as_bytes()))
}

/// Creates a test document and adds the owner as an owner member.
pub async fn create_test_document(
    pool: &PgPool,
    owner_id: ambre_core::UserId,
    doc_type: &str,
) -> ambre_core::DocumentId {
    let (doc, _created) = db::create_document(pool, &owner_id, doc_type, None, None)
        .await
        .expect("failed to create test document");
    let doc_id =
        ambre_core::DocumentId::from_uuid(ambre_core::Uuid::from_bytes(*doc.id.as_bytes()));
    db::add_document_member(pool, &doc_id, &owner_id, "owner", &[5u8; 32], &owner_id)
        .await
        .expect("failed to add owner member");
    doc_id
}
