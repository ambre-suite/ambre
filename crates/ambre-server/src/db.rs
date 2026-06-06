use sqlx::{PgPool, Row};
use crate::error::AppError;

/// Row returned by `SELECT * FROM users`.
#[derive(Debug)]
pub struct UserRow {
    pub id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub master_key_enc: Vec<u8>,
    pub master_key_salt: Vec<u8>,
    pub signing_pubkey: Vec<u8>,
    pub exchange_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub org_id: Option<uuid::Uuid>,
}

/// Projection of `users` for search results (no key material).
#[derive(Debug)]
pub struct UserInfoRow {
    pub id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub signing_pubkey: Vec<u8>,
    pub exchange_pubkey: Vec<u8>,
}

/// Row returned by `SELECT * FROM documents`.
#[derive(Debug)]
pub struct DocumentRow {
    pub id: uuid::Uuid,
    pub owner_id: uuid::Uuid,
    pub doc_type: String,
    pub name_enc: Option<Vec<u8>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Document joined with the caller's membership row.
#[derive(Debug)]
pub struct DocumentSummaryRow {
    pub id: uuid::Uuid,
    pub owner_id: uuid::Uuid,
    pub doc_type: String,
    pub name_enc: Option<Vec<u8>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub role: String,
    pub doc_key_enc: Vec<u8>,
}

/// Row from `document_members`.
#[derive(Debug)]
pub struct MemberRow {
    #[allow(dead_code)]
    pub document_id: uuid::Uuid,
    #[allow(dead_code)]
    pub user_id: uuid::Uuid,
    pub role: String,
    pub doc_key_enc: Vec<u8>,
    #[allow(dead_code)]
    pub invited_by: Option<uuid::Uuid>,
}

/// `document_members` joined with `users` for display purposes.
#[derive(Debug)]
pub struct MemberWithUserRow {
    pub user_id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    #[allow(dead_code)]
    pub invited_by: Option<uuid::Uuid>,
}

/// Row from `snapshots`.
#[derive(Debug)]
pub struct SnapshotRow {
    pub id: uuid::Uuid,
    #[allow(dead_code)]
    pub document_id: uuid::Uuid,
    #[allow(dead_code)]
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub parent_hash: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub parent_snapshot_proof: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub update_clocks: serde_json::Value,
    pub s3_key: String,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Row from `updates` (encrypted deltas indexed by seq).
#[derive(Debug)]
pub struct UpdateRow {
    #[allow(dead_code)]
    pub id: uuid::Uuid,
    #[allow(dead_code)]
    pub document_id: uuid::Uuid,
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub snapshot_id: uuid::Uuid,
    pub clock: i64,
    pub seq: i64,
    pub s3_key: String,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// Helpers -----------------------------------------------------------

#[allow(dead_code)]
fn uuid_to_core(u: uuid::Uuid) -> ambre_core::Uuid {
    ambre_core::Uuid::from_bytes(*u.as_bytes())
}

#[allow(dead_code)]
fn uuid_row_to_id<T: From<ambre_core::Uuid>>(u: uuid::Uuid) -> T {
    T::from(uuid_to_core(u))
}

fn id_to_uuid(id: impl Into<ambre_core::Uuid>) -> uuid::Uuid {
    let core_uuid: ambre_core::Uuid = id.into();
    uuid::Uuid::from_bytes(*core_uuid.as_bytes())
}

// Users --------------------------------------------------------------

/// Fetches a user by their typed `UserId`.
pub async fn get_user(pool: &PgPool, user_id: &ambre_core::UserId) -> Result<Option<UserRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    let row = sqlx::query(
        "SELECT id, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE id = $1"
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get("id")?,
            email: r.try_get("email")?,
            display_name: r.try_get("display_name")?,
            master_key_enc: r.try_get("master_key_enc")?,
            master_key_salt: r.try_get("master_key_salt")?,
            signing_pubkey: r.try_get("signing_pubkey")?,
            exchange_pubkey: r.try_get("exchange_pubkey")?,
            org_id: r.try_get("org_id")?,
        })),
        None => Ok(None),
    }
}

/// Fetches a user by email address.
pub async fn get_user_by_email(pool: &PgPool, email: &str) -> Result<Option<UserRow>, AppError> {
    let row = sqlx::query(
        "SELECT id, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE email = $1"
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get("id")?,
            email: r.try_get("email")?,
            display_name: r.try_get("display_name")?,
            master_key_enc: r.try_get("master_key_enc")?,
            master_key_salt: r.try_get("master_key_salt")?,
            signing_pubkey: r.try_get("signing_pubkey")?,
            exchange_pubkey: r.try_get("exchange_pubkey")?,
            org_id: r.try_get("org_id")?,
        })),
        None => Ok(None),
    }
}

/// Inserts a new user row and returns it.
pub async fn create_user(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    master_key_enc: &[u8],
    master_key_salt: &[u8],
    signing_pubkey: &[u8],
    exchange_pubkey: &[u8],
) -> Result<UserRow, AppError> {
    let row = sqlx::query(
        "INSERT INTO users (email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id"
    )
    .bind(email)
    .bind(display_name)
    .bind(master_key_enc)
    .bind(master_key_salt)
    .bind(signing_pubkey)
    .bind(exchange_pubkey)
    .fetch_one(pool)
    .await?;

    Ok(UserRow {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        master_key_enc: row.try_get("master_key_enc")?,
        master_key_salt: row.try_get("master_key_salt")?,
        signing_pubkey: row.try_get("signing_pubkey")?,
        exchange_pubkey: row.try_get("exchange_pubkey")?,
        org_id: row.try_get("org_id")?,
    })
}

/// Full-text search over users by email or display name (ILIKE).
pub async fn search_users(pool: &PgPool, query: &str) -> Result<Vec<UserInfoRow>, AppError> {
    let pattern = format!("%{query}%");
    let rows = sqlx::query(
        "SELECT id, email, display_name, signing_pubkey, exchange_pubkey FROM users WHERE email ILIKE $1 OR display_name ILIKE $1 LIMIT 20"
    )
    .bind(pattern)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| {
        Ok(UserInfoRow {
            id: r.try_get("id")?,
            email: r.try_get("email")?,
            display_name: r.try_get("display_name")?,
            signing_pubkey: r.try_get("signing_pubkey")?,
            exchange_pubkey: r.try_get("exchange_pubkey")?,
        })
    }).collect()
}

/// Updates the encrypted master key and its salt for passphrase rotation.
pub async fn update_master_key(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
    master_key_enc: &[u8],
    master_key_salt: &[u8],
) -> Result<(), AppError> {
    let uid = id_to_uuid(*user_id);
    sqlx::query(
        "UPDATE users SET master_key_enc = $2, master_key_salt = $3, updated_at = now() WHERE id = $1"
    )
    .bind(uid)
    .bind(master_key_enc)
    .bind(master_key_salt)
    .execute(pool)
    .await?;
    Ok(())
}

// Documents ----------------------------------------------------------

/// Fetches a document by id.
pub async fn get_document(pool: &PgPool, document_id: &ambre_core::DocumentId) -> Result<Option<DocumentRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(
        "SELECT id, owner_id, doc_type, name_enc, created_at, updated_at FROM documents WHERE id = $1"
    )
    .bind(did)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(DocumentRow {
            id: r.try_get("id")?,
            owner_id: r.try_get("owner_id")?,
            doc_type: r.try_get("doc_type")?,
            name_enc: r.try_get("name_enc")?,
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
        })),
        None => Ok(None),
    }
}

/// Lists all documents the user is a member of, with role and doc_key_enc.
pub async fn list_documents_for_user(pool: &PgPool, user_id: &ambre_core::UserId) -> Result<Vec<DocumentSummaryRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    let rows = sqlx::query(
        "SELECT d.id, d.owner_id, d.doc_type, d.name_enc, d.updated_at, dm.role, dm.doc_key_enc FROM documents d INNER JOIN document_members dm ON dm.document_id = d.id WHERE dm.user_id = $1 ORDER BY d.updated_at DESC"
    )
    .bind(uid)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| {
        Ok(DocumentSummaryRow {
            id: r.try_get("id")?,
            owner_id: r.try_get("owner_id")?,
            doc_type: r.try_get("doc_type")?,
            name_enc: r.try_get("name_enc")?,
            updated_at: r.try_get("updated_at")?,
            role: r.try_get("role")?,
            doc_key_enc: r.try_get("doc_key_enc")?,
        })
    }).collect()
}

/// Creates a document and returns the full row.
pub async fn create_document(
    pool: &PgPool,
    owner_id: &ambre_core::UserId,
    doc_type: &str,
    name_enc: Option<&[u8]>,
) -> Result<DocumentRow, AppError> {
    let owner_uid = id_to_uuid(*owner_id);
    let row = sqlx::query(
        "INSERT INTO documents (owner_id, doc_type, name_enc) VALUES ($1, $2, $3) RETURNING id, owner_id, doc_type, name_enc, created_at, updated_at"
    )
    .bind(owner_uid)
    .bind(doc_type)
    .bind(name_enc)
    .fetch_one(pool)
    .await?;

    Ok(DocumentRow {
        id: row.try_get("id")?,
        owner_id: row.try_get("owner_id")?,
        doc_type: row.try_get("doc_type")?,
        name_enc: row.try_get("name_enc")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Updates the encrypted name of a document.
pub async fn rename_document(pool: &PgPool, document_id: &ambre_core::DocumentId, name_enc: &[u8]) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("UPDATE documents SET name_enc = $2, updated_at = now() WHERE id = $1")
        .bind(did)
        .bind(name_enc)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cascade-deletes a document and all associated rows (members, snapshots, updates).
pub async fn delete_document(pool: &PgPool, document_id: &ambre_core::DocumentId) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(did)
        .execute(pool)
        .await?;
    Ok(())
}

// Document Members (ACL) ---------------------------------------------

/// Adds (or updates) a user to a document's membership with a role and encrypted document key.
pub async fn add_document_member(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
    role: &str,
    doc_key_enc: &[u8],
    invited_by: &ambre_core::UserId,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    let inviter = id_to_uuid(*invited_by);
    sqlx::query(
        "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (document_id, user_id) DO UPDATE SET role = $3, doc_key_enc = $4, invited_by = $5"
    )
    .bind(did)
    .bind(uid)
    .bind(role)
    .bind(doc_key_enc)
    .bind(inviter)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetches a single document membership row.
pub async fn get_document_member(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
) -> Result<Option<MemberRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    let row = sqlx::query(
        "SELECT document_id, user_id, role, doc_key_enc, invited_by FROM document_members WHERE document_id = $1 AND user_id = $2"
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(MemberRow {
            document_id: r.try_get("document_id")?,
            user_id: r.try_get("user_id")?,
            role: r.try_get("role")?,
            doc_key_enc: r.try_get("doc_key_enc")?,
            invited_by: r.try_get("invited_by")?,
        })),
        None => Ok(None),
    }
}

/// Lists all members of a document with user details.
pub async fn list_document_members(pool: &PgPool, document_id: &ambre_core::DocumentId) -> Result<Vec<MemberWithUserRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT dm.user_id, u.email, u.display_name, dm.role, dm.invited_by FROM document_members dm INNER JOIN users u ON u.id = dm.user_id WHERE dm.document_id = $1"
    )
    .bind(did)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| {
        Ok(MemberWithUserRow {
            user_id: r.try_get("user_id")?,
            email: r.try_get("email")?,
            display_name: r.try_get("display_name")?,
            role: r.try_get("role")?,
            invited_by: r.try_get("invited_by")?,
        })
    }).collect()
}

/// Removes a user from a document's membership.
pub async fn remove_document_member(pool: &PgPool, document_id: &ambre_core::DocumentId, user_id: &ambre_core::UserId) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    sqlx::query("DELETE FROM document_members WHERE document_id = $1 AND user_id = $2")
        .bind(did)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

/// Updates the role of a document member.
pub async fn update_document_member_role(pool: &PgPool, document_id: &ambre_core::DocumentId, user_id: &ambre_core::UserId, role: &str) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);
    sqlx::query("UPDATE document_members SET role = $3 WHERE document_id = $1 AND user_id = $2")
        .bind(did)
        .bind(uid)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(())
}

/// Atomically replaces all `doc_key_enc` values for a document inside a transaction.
///
/// Called after key rotation: the caller computes new encrypted keys for
/// each remaining member and this function applies them all at once.
pub async fn rotate_document_keys(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    keys: &[(ambre_core::UserId, Vec<u8>)],
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let mut tx = pool.begin().await?;
    for (user_id, key_enc) in keys {
        let uid = id_to_uuid(*user_id);
        sqlx::query("UPDATE document_members SET doc_key_enc = $3 WHERE document_id = $1 AND user_id = $2")
            .bind(did)
            .bind(uid)
            .bind(key_enc)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// Snapshots ----------------------------------------------------------

/// Inserts snapshot metadata and returns the assigned `SnapshotId`.
pub async fn create_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    parent_hash: Option<&[u8]>,
    parent_snapshot_proof: Option<&[u8]>,
    update_clocks: &serde_json::Value,
    s3_key: &str,
) -> Result<ambre_core::SnapshotId, AppError> {
    let did = id_to_uuid(*document_id);
    let snapshot_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO snapshots (id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key) VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(snapshot_id)
    .bind(did)
    .bind(author_pubkey)
    .bind(parent_hash)
    .bind(parent_snapshot_proof)
    .bind(update_clocks)
    .bind(s3_key)
    .execute(pool)
    .await?;

    let core_id = ambre_core::Uuid::from_bytes(*snapshot_id.as_bytes());
    Ok(ambre_core::SnapshotId::from_uuid(core_id))
}

/// Returns the most recent snapshot for a document (if any).
pub async fn get_latest_snapshot(pool: &PgPool, document_id: &ambre_core::DocumentId) -> Result<Option<SnapshotRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, created_at FROM snapshots WHERE document_id = $1 ORDER BY created_at DESC LIMIT 1"
    )
    .bind(did)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(SnapshotRow {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            author_pubkey: r.try_get("author_pubkey")?,
            parent_hash: r.try_get("parent_hash")?,
            parent_snapshot_proof: r.try_get("parent_snapshot_proof")?,
            update_clocks: r.try_get("update_clocks")?,
            s3_key: r.try_get("s3_key")?,
            created_at: r.try_get("created_at")?,
        })),
        None => Ok(None),
    }
}

/// Fetches a specific snapshot by id.
pub async fn get_snapshot(pool: &PgPool, snapshot_id: &ambre_core::SnapshotId) -> Result<Option<SnapshotRow>, AppError> {
    let sid = id_to_uuid(*snapshot_id);
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, created_at FROM snapshots WHERE id = $1"
    )
    .bind(sid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(SnapshotRow {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            author_pubkey: r.try_get("author_pubkey")?,
            parent_hash: r.try_get("parent_hash")?,
            parent_snapshot_proof: r.try_get("parent_snapshot_proof")?,
            update_clocks: r.try_get("update_clocks")?,
            s3_key: r.try_get("s3_key")?,
            created_at: r.try_get("created_at")?,
        })),
        None => Ok(None),
    }
}

// Updates (deltas) ---------------------------------------------------

/// Inserts an update metadata row (idempotent on conflict).
pub async fn insert_update(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    snapshot_id: &ambre_core::SnapshotId,
    clock: i64,
    seq: i64,
    s3_key: &str,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let sid = id_to_uuid(*snapshot_id);
    sqlx::query(
        "INSERT INTO updates (document_id, author_pubkey, snapshot_id, clock, seq, s3_key) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (document_id, author_pubkey, clock) DO NOTHING"
    )
    .bind(did)
    .bind(author_pubkey)
    .bind(sid)
    .bind(clock)
    .bind(seq)
    .bind(s3_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns updates with seq > `after_seq` for delta-sync after reconnect.
pub async fn get_updates_after(pool: &PgPool, document_id: &ambre_core::DocumentId, after_seq: i64, limit: i64) -> Result<Vec<UpdateRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT id, document_id, author_pubkey, snapshot_id, clock, seq, s3_key, created_at FROM updates WHERE document_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
    )
    .bind(did)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| {
        Ok(UpdateRow {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            author_pubkey: r.try_get("author_pubkey")?,
            snapshot_id: r.try_get("snapshot_id")?,
            clock: r.try_get("clock")?,
            seq: r.try_get("seq")?,
            s3_key: r.try_get("s3_key")?,
            created_at: r.try_get("created_at")?,
        })
    }).collect()
}

/// Deletes updates whose seq <= `before_seq` (called after compaction).
#[allow(dead_code)]
pub async fn purge_updates_before_seq(pool: &PgPool, document_id: &ambre_core::DocumentId, before_seq: i64) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("DELETE FROM updates WHERE document_id = $1 AND seq <= $2")
        .bind(did)
        .bind(before_seq)
        .execute(pool)
        .await?;
    Ok(())
}

// Media --------------------------------------------------------------

/// Confirms a completed S3 upload by inserting a `media` row.
pub async fn insert_media(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    uploaded_by: &ambre_core::UserId,
    content_hash: &str,
    mime_type: &str,
    size_bytes: i64,
    s3_key: &str,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*uploaded_by);
    sqlx::query(
        "INSERT INTO media (id, document_id, uploaded_by, mime_type, size_bytes, s3_key) VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(content_hash)
    .bind(did)
    .bind(uid)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(s3_key)
    .execute(pool)
    .await?;
    Ok(())
}
