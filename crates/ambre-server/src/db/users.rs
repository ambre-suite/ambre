// ambre-server/src/db/users.rs
// User rows and account/auth queries.

use super::*;

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
