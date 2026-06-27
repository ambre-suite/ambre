use crate::error::AppError;
use sqlx::{PgPool, Row};

/// Row returned by `SELECT * FROM users`.
#[derive(Debug)]
pub struct UserRow {
    pub id: uuid::Uuid,
    #[allow(dead_code)]
    pub oidc_provider: Option<String>,
    #[allow(dead_code)]
    pub oidc_subject: Option<String>,
    pub email: String,
    pub display_name: String,
    pub master_key_enc: Option<Vec<u8>>,
    pub master_key_salt: Option<Vec<u8>>,
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

/// `document_members` joined with `users` for display purposes. Carries the
/// member's public keys and their wrapped copy of the document key so a client
/// can locate its own entry and unwrap the key (or rewrap on rotation).
#[derive(Debug)]
pub struct MemberWithUserRow {
    pub user_id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub signing_pubkey: Vec<u8>,
    pub exchange_pubkey: Vec<u8>,
    pub doc_key_enc: Vec<u8>,
    #[allow(dead_code)]
    pub invited_by: Option<uuid::Uuid>,
    /// Exchange public key of the member who wrapped `doc_key_enc` (the inviter,
    /// or the member themselves for a self-wrap at creation). The recipient needs
    /// this peer key to perform the ECDH unwrap -- their own key is the wrong DH
    /// partner. `None` only for legacy rows with no `invited_by`.
    pub wrapper_exchange_pubkey: Option<Vec<u8>>,
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
    /// Highest delta `seq` folded into this snapshot. Clients gate their
    /// post-open `RequestMissing` on this value to skip the replayed history.
    pub covered_seq: i64,
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
pub async fn get_user(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
) -> Result<Option<UserRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    let row = sqlx::query(
        "SELECT id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE id = $1"
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get("id")?,
            oidc_provider: r.try_get("oidc_provider")?,
            oidc_subject: r.try_get("oidc_subject")?,
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
        "SELECT id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE email = $1"
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get("id")?,
            oidc_provider: r.try_get("oidc_provider")?,
            oidc_subject: r.try_get("oidc_subject")?,
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
        "INSERT INTO users (email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id"
    )
    .bind(email)
    .bind(display_name)
    .bind(master_key_enc)
    .bind(master_key_salt)
    .bind(signing_pubkey)
    .bind(exchange_pubkey)
    .fetch_one(pool)
    .await?;

    tracing::info!(user_id = %row.try_get::<uuid::Uuid, _>("id")?, provider = "setup", "user created");

    Ok(UserRow {
        id: row.try_get("id")?,
        oidc_provider: row.try_get("oidc_provider")?,
        oidc_subject: row.try_get("oidc_subject")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        master_key_enc: row.try_get("master_key_enc")?,
        master_key_salt: row.try_get("master_key_salt")?,
        signing_pubkey: row.try_get("signing_pubkey")?,
        exchange_pubkey: row.try_get("exchange_pubkey")?,
        org_id: row.try_get("org_id")?,
    })
}

/// Upserts a user from an OIDC provider login.
///
/// Uses `ON CONFLICT (oidc_provider, oidc_subject)` to handle returning users.
/// New users are inserted with placeholder (zero-filled) key columns because
/// the master key, signing key, and exchange key are only set later via
/// `PUT /keys/master`. The `ON CONFLICT DO UPDATE` refreshes `email` and
/// `display_name` on each login but preserves existing key material.
pub async fn upsert_user_from_oidc(
    pool: &PgPool,
    oidc_provider: &str,
    oidc_subject: &str,
    email: &str,
    display_name: &str,
) -> Result<UserRow, AppError> {
    let row = sqlx::query(
        "INSERT INTO users (oidc_provider, oidc_subject, email, display_name, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (oidc_provider, oidc_subject) DO UPDATE SET email = $3, display_name = $4 RETURNING id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id, (xmax = 0) AS inserted"
    )
    .bind(oidc_provider)
    .bind(oidc_subject)
    .bind(email)
    .bind(display_name)
    .bind(vec![0u8; 32])
    .bind(vec![0u8; 32])
    .fetch_one(pool)
    .await?;

    let inserted: bool = row.try_get("inserted")?;
    if inserted {
        tracing::info!(user_id = %row.try_get::<uuid::Uuid, _>("id")?, provider = %oidc_provider, "user created");
    }

    Ok(UserRow {
        id: row.try_get("id")?,
        oidc_provider: row.try_get("oidc_provider")?,
        oidc_subject: row.try_get("oidc_subject")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        master_key_enc: row.try_get("master_key_enc")?,
        master_key_salt: row.try_get("master_key_salt")?,
        signing_pubkey: row.try_get("signing_pubkey")?,
        exchange_pubkey: row.try_get("exchange_pubkey")?,
        org_id: row.try_get("org_id")?,
    })
}

/// Dev-only: upserts a user identified by a `?user=<username>` parameter.
///
/// Stored under `oidc_provider = 'dev'`, `oidc_subject = username` so it shares
/// the OIDC unique index and is idempotent across refreshes. The supplied
/// pubkeys (derived deterministically client-side from the username) are written
/// on insert and refreshed on conflict, so the server can verify the user's
/// signatures. `master_key_*` stay NULL: dev keys never round-trip through the
/// server. Gated by `Config::dev_mode` at the call site.
pub async fn dev_upsert_user(
    pool: &PgPool,
    username: &str,
    signing_pubkey: &[u8],
    exchange_pubkey: &[u8],
) -> Result<UserRow, AppError> {
    let email = format!("{username}@dev.local");
    // `(xmax = 0)` is true only for a freshly INSERTed row (vs an ON CONFLICT
    // UPDATE), so we can log creation without a prior existence check.
    let row = sqlx::query(
        "INSERT INTO users (oidc_provider, oidc_subject, email, display_name, signing_pubkey, exchange_pubkey) VALUES ('dev', $1, $2, $1, $3, $4) ON CONFLICT (oidc_provider, oidc_subject) DO UPDATE SET email = $2, display_name = $1, signing_pubkey = $3, exchange_pubkey = $4 RETURNING id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id, (xmax = 0) AS inserted"
    )
    .bind(username)
    .bind(&email)
    .bind(signing_pubkey)
    .bind(exchange_pubkey)
    .fetch_one(pool)
    .await?;

    let inserted: bool = row.try_get("inserted")?;
    if inserted {
        tracing::info!(user_id = %row.try_get::<uuid::Uuid, _>("id")?, provider = "dev", "user created");
    }

    Ok(UserRow {
        id: row.try_get("id")?,
        oidc_provider: row.try_get("oidc_provider")?,
        oidc_subject: row.try_get("oidc_subject")?,
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

    rows.into_iter()
        .map(|r| {
            Ok(UserInfoRow {
                id: r.try_get("id")?,
                email: r.try_get("email")?,
                display_name: r.try_get("display_name")?,
                signing_pubkey: r.try_get("signing_pubkey")?,
                exchange_pubkey: r.try_get("exchange_pubkey")?,
            })
        })
        .collect()
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
pub async fn get_document(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<DocumentRow>, AppError> {
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
pub async fn list_documents_for_user(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
) -> Result<Vec<DocumentSummaryRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    let rows = sqlx::query(
        "SELECT d.id, d.owner_id, d.doc_type, d.name_enc, d.updated_at, dm.role, dm.doc_key_enc FROM documents d INNER JOIN document_members dm ON dm.document_id = d.id WHERE dm.user_id = $1 ORDER BY d.updated_at DESC"
    )
    .bind(uid)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(DocumentSummaryRow {
                id: r.try_get("id")?,
                owner_id: r.try_get("owner_id")?,
                doc_type: r.try_get("doc_type")?,
                name_enc: r.try_get("name_enc")?,
                updated_at: r.try_get("updated_at")?,
                role: r.try_get("role")?,
                doc_key_enc: r.try_get("doc_key_enc")?,
            })
        })
        .collect()
}

/// Creates a document and returns the full row.
/// Creates a document, optionally with a client-supplied id (lazy creation: the
/// WASM client generates the `DocumentId` up front and only materializes the row
/// on the first modification). Returns the row plus `created` = whether this call
/// actually inserted it (false when an idempotent retry hit an existing id).
///
/// When `document_id` is provided the insert is idempotent (`ON CONFLICT DO
/// NOTHING`): a second call with the same id returns the existing row without
/// touching its `owner_id`, so a different caller can never seize ownership of an
/// id that already exists.
pub async fn create_document(
    pool: &PgPool,
    owner_id: &ambre_core::UserId,
    doc_type: &str,
    name_enc: Option<&[u8]>,
    document_id: Option<&ambre_core::DocumentId>,
) -> Result<(DocumentRow, bool), AppError> {
    let owner_uid = id_to_uuid(*owner_id);

    let row_to_doc = |row: sqlx::postgres::PgRow| -> Result<DocumentRow, AppError> {
        Ok(DocumentRow {
            id: row.try_get("id")?,
            owner_id: row.try_get("owner_id")?,
            doc_type: row.try_get("doc_type")?,
            name_enc: row.try_get("name_enc")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    };

    match document_id {
        Some(did) => {
            let did = id_to_uuid(*did);
            // Idempotent insert: DO NOTHING returns no row on conflict.
            let inserted = sqlx::query(
                "INSERT INTO documents (id, owner_id, doc_type, name_enc) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING RETURNING id, owner_id, doc_type, name_enc, created_at, updated_at"
            )
            .bind(did)
            .bind(owner_uid)
            .bind(doc_type)
            .bind(name_enc)
            .fetch_optional(pool)
            .await?;

            match inserted {
                Some(row) => Ok((row_to_doc(row)?, true)),
                None => {
                    // Already existed: return it untouched (owner preserved).
                    let row = sqlx::query(
                        "SELECT id, owner_id, doc_type, name_enc, created_at, updated_at FROM documents WHERE id = $1"
                    )
                    .bind(did)
                    .fetch_one(pool)
                    .await?;
                    Ok((row_to_doc(row)?, false))
                }
            }
        }
        None => {
            let row = sqlx::query(
                "INSERT INTO documents (owner_id, doc_type, name_enc) VALUES ($1, $2, $3) RETURNING id, owner_id, doc_type, name_enc, created_at, updated_at"
            )
            .bind(owner_uid)
            .bind(doc_type)
            .bind(name_enc)
            .fetch_one(pool)
            .await?;
            Ok((row_to_doc(row)?, true))
        }
    }
}

/// Updates the encrypted name of a document.
pub async fn rename_document(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    name_enc: &[u8],
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("UPDATE documents SET name_enc = $2, updated_at = now() WHERE id = $1")
        .bind(did)
        .bind(name_enc)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cascade-deletes a document and all associated rows (members, snapshots, updates).
pub async fn delete_document(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<(), AppError> {
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
pub async fn list_document_members(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Vec<MemberWithUserRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT dm.user_id, u.email, u.display_name, dm.role, u.signing_pubkey, u.exchange_pubkey, dm.doc_key_enc, dm.invited_by, inv.exchange_pubkey AS wrapper_exchange_pubkey FROM document_members dm INNER JOIN users u ON u.id = dm.user_id LEFT JOIN users inv ON inv.id = dm.invited_by WHERE dm.document_id = $1"
    )
    .bind(did)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(MemberWithUserRow {
                user_id: r.try_get("user_id")?,
                email: r.try_get("email")?,
                display_name: r.try_get("display_name")?,
                role: r.try_get("role")?,
                signing_pubkey: r.try_get("signing_pubkey")?,
                exchange_pubkey: r.try_get("exchange_pubkey")?,
                doc_key_enc: r.try_get("doc_key_enc")?,
                invited_by: r.try_get("invited_by")?,
                wrapper_exchange_pubkey: r.try_get("wrapper_exchange_pubkey")?,
            })
        })
        .collect()
}

/// Removes a user from a document's membership.
pub async fn remove_document_member(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
) -> Result<(), AppError> {
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
pub async fn update_document_member_role(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
    role: &str,
) -> Result<(), AppError> {
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
    rotated_by: &ambre_core::UserId,
    keys: &[(ambre_core::UserId, Vec<u8>)],
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    // The rotator re-wrapped every key with ITS exchange key, so it becomes the
    // new wrapper of record: update `invited_by` alongside `doc_key_enc` or the
    // ECDH unwrap would later use the stale inviter's key.
    let rotator = id_to_uuid(*rotated_by);
    let mut tx = pool.begin().await?;
    for (user_id, key_enc) in keys {
        let uid = id_to_uuid(*user_id);
        sqlx::query("UPDATE document_members SET doc_key_enc = $3, invited_by = $4 WHERE document_id = $1 AND user_id = $2")
            .bind(did)
            .bind(uid)
            .bind(key_enc)
            .bind(rotator)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// Snapshots ----------------------------------------------------------

/// Inserts snapshot metadata and returns the assigned `SnapshotId`.
#[allow(clippy::too_many_arguments)]
pub async fn create_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    parent_hash: Option<&[u8]>,
    parent_snapshot_proof: Option<&[u8]>,
    update_clocks: &serde_json::Value,
    s3_key: &str,
    covered_seq: i64,
) -> Result<ambre_core::SnapshotId, AppError> {
    let did = id_to_uuid(*document_id);
    let snapshot_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO snapshots (id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(snapshot_id)
    .bind(did)
    .bind(author_pubkey)
    .bind(parent_hash)
    .bind(parent_snapshot_proof)
    .bind(update_clocks)
    .bind(s3_key)
    .bind(covered_seq)
    .execute(pool)
    .await?;

    let core_id = ambre_core::Uuid::from_bytes(*snapshot_id.as_bytes());
    Ok(ambre_core::SnapshotId::from_uuid(core_id))
}

/// Returns the most recent snapshot for a document (if any).
pub async fn get_latest_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<SnapshotRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq, created_at FROM snapshots WHERE document_id = $1 ORDER BY created_at DESC LIMIT 1"
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
            covered_seq: r.try_get("covered_seq")?,
            created_at: r.try_get("created_at")?,
        })),
        None => Ok(None),
    }
}

/// Fetches a specific snapshot by id.
pub async fn get_snapshot(
    pool: &PgPool,
    snapshot_id: &ambre_core::SnapshotId,
) -> Result<Option<SnapshotRow>, AppError> {
    let sid = id_to_uuid(*snapshot_id);
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq, created_at FROM snapshots WHERE id = $1"
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
            covered_seq: r.try_get("covered_seq")?,
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
pub async fn get_updates_after(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<UpdateRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT id, document_id, author_pubkey, snapshot_id, clock, seq, s3_key, created_at FROM updates WHERE document_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
    )
    .bind(did)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
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
        })
        .collect()
}

/// Deletes updates whose seq <= `before_seq` (called after compaction).
#[allow(dead_code)]
pub async fn purge_updates_before_seq(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    before_seq: i64,
) -> Result<(), AppError> {
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

// Organizations ------------------------------------------------------

/// Row from the `organizations` table.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OrganizationRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub org_pubkey: Vec<u8>,
    pub shamir_threshold: i16,
    pub shamir_total: i16,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Row from `organization_members` joined with `users`.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OrgMemberRow {
    pub user_id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub shamir_share_enc: Option<Vec<u8>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Creates an organization and inserts its first member as owner.
pub async fn create_organization(
    pool: &PgPool,
    name: &str,
    org_pubkey: &[u8],
    shamir_threshold: i16,
    shamir_total: i16,
    owner_id: &ambre_core::UserId,
    owner_share_enc: Option<&[u8]>,
) -> Result<OrganizationRow, AppError> {
    let org_id = uuid::Uuid::new_v4();
    let uid = id_to_uuid(*owner_id);

    let mut tx = pool.begin().await?;

    let org = sqlx::query_as::<_, OrganizationRow>(
        "INSERT INTO organizations (id, name, org_pubkey, shamir_threshold, shamir_total)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total, created_at",
    )
    .bind(org_id)
    .bind(name)
    .bind(org_pubkey)
    .bind(shamir_threshold)
    .bind(shamir_total)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role, shamir_share_enc)
         VALUES ($1, $2, 'owner', $3)",
    )
    .bind(org_id)
    .bind(uid)
    .bind(owner_share_enc)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(org)
}

/// Fetches an organization by id.
pub async fn get_organization(
    pool: &PgPool,
    org_id: uuid::Uuid,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "SELECT id, name, org_pubkey, shamir_threshold, shamir_total, created_at
         FROM organizations WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Lists organizations for a given user.
pub async fn list_user_organizations(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
) -> Result<Vec<OrganizationRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    sqlx::query_as::<_, OrganizationRow>(
        "SELECT o.id, o.name, o.org_pubkey, o.shamir_threshold, o.shamir_total, o.created_at
         FROM organizations o
         JOIN organization_members om ON om.org_id = o.id
         WHERE om.user_id = $1
         ORDER BY o.created_at",
    )
    .bind(uid)
    .fetch_all(pool)
    .await
    .map_err(AppError::from)
}

/// Updates an organization name (owner only).
pub async fn update_organization(
    pool: &PgPool,
    org_id: uuid::Uuid,
    name: &str,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "UPDATE organizations SET name = $2 WHERE id = $1
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total, created_at",
    )
    .bind(org_id)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Deletes an organization (cascades to members via FK constraint).
pub async fn delete_organization(pool: &PgPool, org_id: uuid::Uuid) -> Result<bool, AppError> {
    let rows = sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(org_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Adds a member to an organization with an optional encrypted Shamir share.
pub async fn add_org_member(
    pool: &PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
    role: &str,
    shamir_share_enc: Option<&[u8]>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role, shamir_share_enc)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = $3, shamir_share_enc = COALESCE($4, organization_members.shamir_share_enc)"
    )
    .bind(org_id)
    .bind(user_id)
    .bind(role)
    .bind(shamir_share_enc)
    .execute(pool)
    .await?;
    Ok(())
}

/// Lists members of an organization.
pub async fn list_org_members(
    pool: &PgPool,
    org_id: uuid::Uuid,
) -> Result<Vec<OrgMemberRow>, AppError> {
    sqlx::query_as::<_, OrgMemberRow>(
        "SELECT om.user_id, u.email, u.display_name, om.role, om.shamir_share_enc,
                om.created_at
         FROM organization_members om
         JOIN users u ON u.id = om.user_id
         WHERE om.org_id = $1
         ORDER BY om.created_at",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::from)
}

/// Fetches a single org membership row (raw, without user join).
pub async fn get_org_member(
    pool: &PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> Result<Option<OrgMembershipRow>, AppError> {
    sqlx::query_as::<_, OrgMembershipRow>(
        "SELECT org_id, user_id, role, shamir_share_enc, created_at
         FROM organization_members
         WHERE org_id = $1 AND user_id = $2",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Raw `organization_members` row (no join).
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OrgMembershipRow {
    #[allow(dead_code)]
    pub org_id: uuid::Uuid,
    #[allow(dead_code)]
    pub user_id: uuid::Uuid,
    pub role: String,
    #[allow(dead_code)]
    pub shamir_share_enc: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Removes a member from an organization.
pub async fn remove_org_member(
    pool: &PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> Result<bool, AppError> {
    let rows = sqlx::query("DELETE FROM organization_members WHERE org_id = $1 AND user_id = $2")
        .bind(org_id)
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Updates a member's role.
pub async fn update_org_member_role(
    pool: &PgPool,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
    role: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE organization_members SET role = $3 WHERE org_id = $1 AND user_id = $2")
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    Ok(())
}

/// Collects Shamir shares for a given organization from the database.
#[allow(dead_code)]
pub async fn get_org_shamir_shares(
    pool: &PgPool,
    org_id: uuid::Uuid,
) -> Result<Vec<(uuid::Uuid, Vec<u8>)>, AppError> {
    let rows = sqlx::query(
        "SELECT user_id, shamir_share_enc FROM organization_members WHERE org_id = $1 AND shamir_share_enc IS NOT NULL"
    )
    .bind(org_id)
    .fetch_all(pool)
    .await?;
    let shares: Vec<(uuid::Uuid, Vec<u8>)> = rows
        .iter()
        .map(|r| {
            let uid: uuid::Uuid = r.try_get("user_id").unwrap();
            let share: Vec<u8> = r.try_get("shamir_share_enc").unwrap();
            (uid, share)
        })
        .collect();
    Ok(shares)
}

/// Logs a master key recovery event.
pub async fn log_org_recovery(
    pool: &PgPool,
    org_id: uuid::Uuid,
    target_user_id: uuid::Uuid,
    recovered_by: uuid::Uuid,
    reason: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO org_recovery_log (org_id, target_user_id, recovered_by, reason)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(org_id)
    .bind(target_user_id)
    .bind(recovered_by)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetches the organization master key for a specific user (encrypted with org pubkey).
pub async fn get_user_org_master_key_enc(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> Result<Option<Vec<u8>>, AppError> {
    let row = sqlx::query("SELECT org_master_key_enc FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<Vec<u8>>, _>("org_master_key_enc")
            .ok()
            .flatten()
    }))
}

// Compaction ----------------------------------------------------------

/// Snapshot compacted to be deleted (id + S3 key for cleanup).
#[derive(Debug, sqlx::FromRow)]
pub struct CompactSnapshot {
    pub id: uuid::Uuid,
    pub s3_key: String,
}

/// Finds snapshots beyond the keep limit (oldest N) per document.
///
/// Returns a list of snapshot ids and their S3 keys that should be
/// deleted. Keeps at most `keep` snapshots per document (most recent
/// by created_at DESC).
pub async fn find_compactable_snapshots(
    pool: &PgPool,
    keep: i64,
) -> Result<Vec<CompactSnapshot>, AppError> {
    let rows = sqlx::query_as::<_, CompactSnapshot>(
        "WITH ranked AS (
            SELECT id, s3_key,
                   ROW_NUMBER() OVER (PARTITION BY document_id ORDER BY created_at DESC) AS rn
            FROM snapshots
        )
        SELECT id, s3_key FROM ranked WHERE rn > $1",
    )
    .bind(keep)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Deletes a single snapshot row (cascades to updates via FK).
pub async fn delete_snapshot(pool: &PgPool, snapshot_id: uuid::Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(snapshot_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bridge a stored `uuid::Uuid` (FK columns) back to a typed core id.
    fn core_id<T: From<ambre_core::Uuid>>(id: uuid::Uuid) -> T {
        ambre_core::Uuid::from_bytes(*id.as_bytes()).into()
    }

    /// `covered_seq` survives the insert and both read paths. This is the value a
    /// client gates its post-open `RequestMissing` on, so a silent reset to 0
    /// would resurrect the full-history replay.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_snapshot_roundtrips_covered_seq(pool: PgPool) {
        // A snapshot row needs an owning document, which needs an owner user (FKs).
        let user = create_user(
            &pool,
            "snap@test.dev",
            "Snap",
            &[1u8; 32],
            &[2u8; 16],
            &[3u8; 32],
            &[4u8; 32],
        )
        .await
        .expect("create user");
        let owner: ambre_core::UserId = core_id(user.id);
        let (doc, created) = create_document(&pool, &owner, "trame", None, None)
            .await
            .expect("create document");
        assert!(created);
        let document_id: ambre_core::DocumentId = core_id(doc.id);

        let author = [5u8; 32];
        let clocks = serde_json::json!({});
        let sid = create_snapshot(
            &pool,
            &document_id,
            &author,
            None,
            None,
            &clocks,
            "s3/key/snap",
            1500,
        )
        .await
        .expect("create snapshot");

        // get_latest_snapshot (the open path's GET /snapshot) carries it.
        let latest = get_latest_snapshot(&pool, &document_id)
            .await
            .expect("query latest")
            .expect("snapshot exists");
        assert_eq!(latest.id, id_to_uuid(sid));
        assert_eq!(latest.s3_key, "s3/key/snap");
        assert_eq!(latest.covered_seq, 1500);

        // get_snapshot (the download-url path) carries it too.
        let by_id = get_snapshot(&pool, &sid)
            .await
            .expect("query by id")
            .expect("snapshot exists");
        assert_eq!(by_id.covered_seq, 1500);
    }
}
