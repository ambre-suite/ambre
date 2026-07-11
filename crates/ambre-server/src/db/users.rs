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
    /// Whether `email` was proven by a provider (OIDC/dev) rather than merely
    /// self-asserted at `/auth/setup`. A verified email wins any collision (#3):
    /// an OIDC login reclaims an unverified holder's email.
    pub email_verified: bool,
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
    /// Whether the email was provider-verified (see [`UserRow::email_verified`]).
    /// Surfaced in search so the sharer sees the TOFU signal for an unverified,
    /// possibly-squatted recipient before wrapping the doc key to them (#3).
    pub email_verified: bool,
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
        "SELECT id, oidc_provider, oidc_subject, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE id = $1"
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
            email_verified: r.try_get("email_verified")?,
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
        "SELECT id, oidc_provider, oidc_subject, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id FROM users WHERE lower(email) = lower($1)"
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
            email_verified: r.try_get("email_verified")?,
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
        "INSERT INTO users (email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, oidc_provider, oidc_subject, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id"
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
        email_verified: row.try_get("email_verified")?,
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
/// `display_name` on each login but preserves existing key material. The email
/// is provider-verified, so the row is marked `email_verified = true`.
///
/// Collision policy (#3, supersedes the earlier "always 409" note for the
/// verified-vs-unverified case): a provider-verified email wins over a
/// self-asserted one. Before upserting, this RECLAIMS the email from any
/// UNVERIFIED account currently holding it (that account is a `/auth/setup`
/// squatter; its email is unbound to a unique, non-resolvable sentinel). A
/// collision with another VERIFIED identity is left intact, so the upsert then
/// violates `users_email_lower_unique` and the login fails with 409 -- verified
/// identities never silently steal each other's email. Reclaim + upsert run in
/// one transaction so a concurrent login cannot observe a half-freed email.
pub async fn upsert_user_from_oidc(
    pool: &PgPool,
    oidc_provider: &str,
    oidc_subject: &str,
    email: &str,
    display_name: &str,
) -> Result<UserRow, AppError> {
    let mut tx = pool.begin().await?;

    // Reclaim the email from any UNVERIFIED holder that is not this same OIDC
    // identity. The sentinel keeps the NOT NULL + unique invariants while making
    // the squatter's address non-resolvable for sharing (`get_user_by_email`
    // will no longer return it) and re-settable by that account later. A verified
    // holder is deliberately NOT touched, so the upsert below surfaces 409.
    sqlx::query(
        "UPDATE users \
         SET email = 'reclaimed-' || id::text || '@invalid.local', email_verified = false \
         WHERE lower(email) = lower($1) \
           AND email_verified = false \
           AND (oidc_provider IS DISTINCT FROM $2 OR oidc_subject IS DISTINCT FROM $3)",
    )
    .bind(email)
    .bind(oidc_provider)
    .bind(oidc_subject)
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query(
        "INSERT INTO users (oidc_provider, oidc_subject, email, email_verified, display_name, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, true, $4, $5, $6) ON CONFLICT (oidc_provider, oidc_subject) DO UPDATE SET email = $3, email_verified = true, display_name = $4 RETURNING id, oidc_provider, oidc_subject, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id, (xmax = 0) AS inserted"
    )
    .bind(oidc_provider)
    .bind(oidc_subject)
    .bind(email)
    .bind(display_name)
    .bind(vec![0u8; 32])
    .bind(vec![0u8; 32])
    .fetch_one(&mut *tx)
    .await
    .map_err(map_email_conflict)?;

    let inserted: bool = row.try_get("inserted")?;
    let user = UserRow {
        id: row.try_get("id")?,
        oidc_provider: row.try_get("oidc_provider")?,
        oidc_subject: row.try_get("oidc_subject")?,
        email: row.try_get("email")?,
        email_verified: row.try_get("email_verified")?,
        display_name: row.try_get("display_name")?,
        master_key_enc: row.try_get("master_key_enc")?,
        master_key_salt: row.try_get("master_key_salt")?,
        signing_pubkey: row.try_get("signing_pubkey")?,
        exchange_pubkey: row.try_get("exchange_pubkey")?,
        org_id: row.try_get("org_id")?,
    };

    tx.commit().await?;

    if inserted {
        tracing::info!(user_id = %user.id, provider = %oidc_provider, "user created");
    }

    Ok(user)
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
        "INSERT INTO users (oidc_provider, oidc_subject, email, email_verified, display_name, signing_pubkey, exchange_pubkey) VALUES ('dev', $1, $2, true, $1, $3, $4) ON CONFLICT (oidc_provider, oidc_subject) DO UPDATE SET email = $2, email_verified = true, display_name = $1, signing_pubkey = $3, exchange_pubkey = $4 RETURNING id, oidc_provider, oidc_subject, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, org_id, (xmax = 0) AS inserted"
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
        email_verified: row.try_get("email_verified")?,
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
        "SELECT id, email, email_verified, display_name, signing_pubkey, exchange_pubkey FROM users WHERE email ILIKE $1 ESCAPE '\\' OR display_name ILIKE $1 ESCAPE '\\' LIMIT 20"
    )
    .bind(pattern)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UserInfoRow {
                id: r.try_get("id")?,
                email: r.try_get("email")?,
                email_verified: r.try_get("email_verified")?,
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

    /// #3: a provider-verified OIDC login RECLAIMS an email squatted by an
    /// unverified `/auth/setup` account. The login succeeds (closing the
    /// unauthenticated login-DoS), the OIDC row owns the verified email, and the
    /// squatter's row no longer resolves for that email.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn oidc_login_reclaims_email_from_unverified_squatter(pool: PgPool) {
        let squatter = create_user(
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
        assert!(!squatter.email_verified, "a setup email is unverified");

        let oidc = upsert_user_from_oidc(&pool, "google", "sub-123", "Alice@example.com", "Alice")
            .await
            .expect("verified login must reclaim the squatted email");
        assert!(oidc.email_verified, "the OIDC email is verified");
        assert_ne!(oidc.id, squatter.id, "a distinct OIDC identity");

        // The email now resolves to the verified OIDC account, not the squatter.
        let resolved = get_user_by_email(&pool, "alice@example.com")
            .await
            .expect("query")
            .expect("email still resolves");
        assert_eq!(resolved.id, oidc.id);

        // The squatter row survives but its email was unbound to a sentinel.
        let squatter_now = get_user(
            &pool,
            &ambre_core::UserId::from_uuid(ambre_core::Uuid::from_bytes(*squatter.id.as_bytes())),
        )
        .await
        .expect("query")
        .expect("squatter row still exists");
        assert_ne!(
            squatter_now.email.to_lowercase(),
            "alice@example.com",
            "the squatter no longer holds the reclaimed email"
        );
    }

    /// #3: a verified email is NOT reclaimable. Once an OIDC identity owns it, a
    /// later `/auth/setup` claim for the same email fails with 409.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn verified_email_cannot_be_squatted_by_setup(pool: PgPool) {
        upsert_user_from_oidc(&pool, "google", "sub-1", "dave@example.com", "Dave")
            .await
            .expect("verified login");

        let err = create_user(
            &pool,
            "DAVE@example.com",
            "Impostor",
            &[9u8; 32],
            &[8u8; 16],
            &[7u8; 32],
            &[6u8; 32],
        )
        .await
        .expect_err("a verified email must not be claimable via setup");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// #3: two DIFFERENT verified identities cannot hold the same email -- the
    /// second login surfaces 409 rather than silently stealing it (reclaim only
    /// applies to UNVERIFIED holders).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn verified_email_collision_is_conflict(pool: PgPool) {
        upsert_user_from_oidc(&pool, "google", "sub-a", "erin@example.com", "Erin")
            .await
            .expect("first verified login");

        let err = upsert_user_from_oidc(&pool, "github", "sub-b", "erin@example.com", "Erin")
            .await
            .expect_err("a second verified identity must not steal the email");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// A returning OIDC identity refreshing its own email is idempotent (the
    /// reclaim skips its own row, so it does not self-conflict).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn oidc_relogin_keeps_its_own_email(pool: PgPool) {
        let first = upsert_user_from_oidc(&pool, "google", "sub-x", "frank@example.com", "Frank")
            .await
            .expect("first login");
        let second = upsert_user_from_oidc(&pool, "google", "sub-x", "frank@example.com", "Frank")
            .await
            .expect("re-login must not self-conflict");
        assert_eq!(first.id, second.id);
        assert!(second.email_verified);
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
