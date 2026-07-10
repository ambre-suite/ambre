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

/// Maps a unique violation on `users_email_lower_unique` to a 409 Conflict
/// (N5 policy: the email namespace is global and first-claimant-wins). Any
/// other error stays an infrastructure-level `Database` error (500).
fn map_email_conflict(e: sqlx::Error) -> AppError {
    match &e {
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some("users_email_lower_unique") =>
        {
            AppError::Conflict {
                detail: "email already registered".into(),
            }
        }
        _ => AppError::Database(e),
    }
}

/// Fetches a user by email address. Case-insensitive, matching the
/// `users_email_lower_unique` index, so at most one row can match.
pub async fn get_user_by_email(pool: &PgPool, email: &str) -> Result<Option<UserRow>, AppError> {
    let row = sqlx::query(
        "SELECT id, oidc_provider, oidc_subject, email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE lower(email) = lower($1)"
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
///
/// A duplicate email (any provider, any case) is rejected with 409 Conflict
/// via `map_email_conflict` (N5).
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
    .await
    .map_err(map_email_conflict)?;

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
///
/// N5 collision policy: if the IdP-supplied email is already held by a
/// DIFFERENT account (e.g. claimed earlier through /auth/setup, where emails
/// are unverified), the insert or the email refresh violates
/// `users_email_lower_unique` and the login fails with 409. Surfacing the
/// squat explicitly is preferred over silently rebinding the email.
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
    .await
    .map_err(map_email_conflict)?;

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
    .await
    .map_err(map_email_conflict)?;

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
    // Escape the LIKE metacharacters (`\ % _`) in the user-supplied query so they
    // match literally. Without this a caller could pass `%` / `_` as wildcards and
    // widen `ILIKE '%q%'` into a directory-enumeration primitive, defeating the
    // minimum-length guard at the route. `ESCAPE '\'` binds the escape char.
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let rows = sqlx::query(
        "SELECT id, email, display_name, signing_pubkey, exchange_pubkey FROM users WHERE email ILIKE $1 ESCAPE '\\' OR display_name ILIKE $1 ESCAPE '\\' LIMIT 20"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// N5 guard: an OIDC login whose IdP email is already claimed by a setup
    /// account fails with `Conflict` (409), not an opaque `Database` (500).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn oidc_login_with_squatted_email_is_conflict(pool: PgPool) {
        create_user(
            &pool,
            "alice@example.com",
            "Alice",
            &[1u8; 32],
            &[2u8; 16],
            &[3u8; 32],
            &[4u8; 32],
        )
        .await
        .expect("setup account");

        let err = upsert_user_from_oidc(&pool, "google", "sub-123", "Alice@example.com", "Alice")
            .await
            .expect_err("email is already claimed");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// N5 guard: uniqueness is case-insensitive across setup accounts too.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_user_duplicate_email_is_conflict(pool: PgPool) {
        create_user(
            &pool,
            "bob@example.com",
            "Bob",
            &[1u8; 32],
            &[2u8; 16],
            &[3u8; 32],
            &[4u8; 32],
        )
        .await
        .expect("first claim");

        let err = create_user(
            &pool,
            "BOB@example.com",
            "Impostor",
            &[9u8; 32],
            &[8u8; 16],
            &[7u8; 32],
            &[6u8; 32],
        )
        .await
        .expect_err("second claim must conflict");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// The sharing resolver matches case-insensitively, consistent with the
    /// unique index, so at most one row can match.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn get_user_by_email_is_case_insensitive(pool: PgPool) {
        let created = create_user(
            &pool,
            "carol@example.com",
            "Carol",
            &[1u8; 32],
            &[2u8; 16],
            &[3u8; 32],
            &[4u8; 32],
        )
        .await
        .expect("create user");

        let found = get_user_by_email(&pool, "CAROL@EXAMPLE.COM")
            .await
            .expect("query")
            .expect("must resolve despite case difference");
        assert_eq!(found.id, created.id);
    }
}
