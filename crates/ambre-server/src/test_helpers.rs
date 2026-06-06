#![cfg(any(test, feature = "test-helpers"))]

use sqlx::PgPool;

use crate::db;

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
#[allow(dead_code)]
pub async fn create_test_document(
    pool: &PgPool,
    owner_id: ambre_core::UserId,
    doc_type: &str,
) -> ambre_core::DocumentId {
    let doc = db::create_document(pool, &owner_id, doc_type, None)
        .await
        .expect("failed to create test document");
    let doc_id = ambre_core::DocumentId::from_uuid(ambre_core::Uuid::from_bytes(*doc.id.as_bytes()));
    db::add_document_member(pool, &doc_id, &owner_id, "owner", &[5u8; 32], &owner_id)
        .await
        .expect("failed to add owner member");
    doc_id
}
