// ambre-server/src/db/users.rs
// User rows and account/auth queries.

use super::*;

/// A `users` row, with the columns the relay reads back.
#[derive(Debug)]
pub struct UserRow {
    pub id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether `email` was proven by a provider (OIDC/dev) rather than merely
    /// self-asserted at the retired account setup (`SRV-319`). A verified email
    /// wins any collision (#3): an OIDC login reclaims an unverified holder's
    /// email.
    pub email_verified: EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub master_key_enc: Option<ambre_core::Encoded<ambre_core::WrappedMasterKey>>,
    pub master_key_salt: Option<FixedBytes<ambre_core::MasterKeySalt>>,
    /// `None` until the account registers its keys: a provider login creates
    /// the account first (`SRV-027`). Both keys are set or neither is.
    pub signing_pubkey: Option<FixedBytes<ambre_core::PublicKey>>,
    pub exchange_pubkey: Option<FixedBytes<ambre_core::ExchangePublicKey>>,
}

/// Projection of `users` for search results (no key material).
#[derive(Debug)]
pub struct UserInfoRow {
    pub id: Id<User>,
    pub email: ambre_core::Email,
    /// Whether the email was provider-verified (see [`UserRow::email_verified`]).
    /// Surfaced in search so the sharer sees the TOFU signal for an unverified,
    /// possibly-squatted recipient before wrapping the doc key to them (#3).
    pub email_verified: EmailProof,
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    pub signing_pubkey: FixedBytes<ambre_core::PublicKey>,
    pub exchange_pubkey: FixedBytes<ambre_core::ExchangePublicKey>,
}

/// Whether a provider proved an address. The type lives in `ambre-core`, so
/// the client reads the same flag the relay writes; the column stays a `bool`.
pub use ambre_core::EmailProof;

/// Fetches a user by their typed `Id<User>`.
pub async fn get_user(pool: &PgPool, user_id: &Id<User>) -> Result<Option<UserRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    let row = sqlx::query(
        "SELECT id, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey FROM users WHERE id = $1"
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get::<PgId, _>("id")?.into(),
            email: ambre_core::Email::new(r.try_get("email")?),
            email_verified: ambre_core::Flag::from_bool(r.try_get("email_verified")?),
            display_name: decode_display_name(r.try_get("display_name")?)?,
            master_key_enc: r
                .try_get::<Option<Vec<u8>>, _>("master_key_enc")?
                .map(ambre_core::Encoded::new),
            master_key_salt: r
                .try_get::<Option<Vec<u8>>, _>("master_key_salt")?
                .map(decode_salt)
                .transpose()?,
            signing_pubkey: decode_opt_key(r.try_get("signing_pubkey")?)?,
            exchange_pubkey: decode_opt_exchange_key(r.try_get("exchange_pubkey")?)?,
        })),
        None => Ok(None),
    }
}

/// Maps a unique violation on `users_email_lower_unique` to a 409 Conflict
/// (N5 policy: the email namespace is global and first-claimant-wins). So does
/// one on `users_signing_pubkey_unique`: a signing key belongs to the first
/// account that registers it (`SRV-293`). Any other error stays an
/// infrastructure-level `Database` error (500).
pub(crate) fn map_email_conflict(e: sqlx::Error) -> AppError {
    match &e {
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some("users_email_lower_unique") =>
        {
            AppError::Conflict {
                detail: "email already registered".into(),
            }
        }
        sqlx::Error::Database(db)
            if db.is_unique_violation()
                && db.constraint() == Some("users_signing_pubkey_unique") =>
        {
            AppError::Conflict {
                detail: "signing key already registered".into(),
            }
        }
        _ => AppError::Database(e),
    }
}

/// Fetches a user by email address. Case-insensitive, matching the
/// `users_email_lower_unique` index, so at most one row can match.
pub async fn get_user_by_email(
    pool: &PgPool,
    email: &ambre_core::Email,
) -> Result<Option<UserRow>, AppError> {
    let row = sqlx::query(
        "SELECT id, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey FROM users WHERE lower(email) = lower($1)"
    )
    .bind(email.as_str())
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(UserRow {
            id: r.try_get::<PgId, _>("id")?.into(),
            email: ambre_core::Email::new(r.try_get("email")?),
            email_verified: ambre_core::Flag::from_bool(r.try_get("email_verified")?),
            display_name: decode_display_name(r.try_get("display_name")?)?,
            master_key_enc: r
                .try_get::<Option<Vec<u8>>, _>("master_key_enc")?
                .map(ambre_core::Encoded::new),
            master_key_salt: r
                .try_get::<Option<Vec<u8>>, _>("master_key_salt")?
                .map(decode_salt)
                .transpose()?,
            signing_pubkey: decode_opt_key(r.try_get("signing_pubkey")?)?,
            exchange_pubkey: decode_opt_exchange_key(r.try_get("exchange_pubkey")?)?,
        })),
        None => Ok(None),
    }
}

/// Inserts a new user row with a self-asserted address and returns it.
///
/// No route of the relay creates such an account since the anonymous account
/// setup was retired (`SRV-319`). Rows written before it remain, so the tests
/// of the address namespace and of the reclaim create them here. A duplicate
/// email (any provider, any case) is rejected with 409 Conflict via
/// `map_email_conflict` (N5).
#[cfg(test)]
pub async fn create_user(
    pool: &PgPool,
    email: &ambre_core::Email,
    display_name: &ambre_core::Validated<ambre_core::DisplayName>,
    master_key_enc: &ambre_core::Encoded<ambre_core::WrappedMasterKey>,
    master_key_salt: &FixedBytes<ambre_core::MasterKeySalt>,
    signing_pubkey: &FixedBytes<ambre_core::PublicKey>,
    exchange_pubkey: &FixedBytes<ambre_core::ExchangePublicKey>,
) -> Result<UserRow, AppError> {
    let row = sqlx::query(
        "INSERT INTO users (email, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey"
    )
    .bind(email.as_str())
    .bind(display_name.as_str())
    .bind(master_key_enc.as_bytes())
    .bind(master_key_salt.as_slice())
    .bind(signing_pubkey.as_slice())
    .bind(exchange_pubkey.as_slice())
    .fetch_one(pool)
    .await
    .map_err(map_email_conflict)?;

    tracing::debug!(user_id = %row.try_get::<uuid::Uuid, _>("id")?, provider = "setup", "user created");

    Ok(UserRow {
        id: row.try_get::<PgId, _>("id")?.into(),
        email: ambre_core::Email::new(row.try_get("email")?),
        email_verified: ambre_core::Flag::from_bool(row.try_get("email_verified")?),
        display_name: decode_display_name(row.try_get("display_name")?)?,
        master_key_enc: row
            .try_get::<Option<Vec<u8>>, _>("master_key_enc")?
            .map(ambre_core::Encoded::new),
        master_key_salt: row
            .try_get::<Option<Vec<u8>>, _>("master_key_salt")?
            .map(decode_salt)
            .transpose()?,
        signing_pubkey: decode_opt_key(row.try_get("signing_pubkey")?)?,
        exchange_pubkey: decode_opt_exchange_key(row.try_get("exchange_pubkey")?)?,
    })
}

/// Upserts a user from an OIDC provider login.
///
/// Uses `ON CONFLICT (oidc_issuer, oidc_subject)` to handle returning users.
/// The account is keyed by the issuer and the subject, never by the address
/// or the provider word. So two issuers that assert one subject reach two
/// accounts (`SRV-344`).
/// New users are inserted with placeholder (zero-filled) key columns because
/// the master key, signing key, and exchange key are only set later via `PUT
/// /keys/master`. The `ON CONFLICT DO UPDATE` refreshes `email` and
/// `display_name` on each login but preserves existing key material. The email
/// is provider-verified, so the row is marked `email_verified = true`.
///
/// Collision policy (#3, supersedes the earlier "always 409" note for the
/// verified-vs-unverified case): a provider-verified email wins over a
/// self-asserted one. Before upserting, this RECLAIMS the email from any
/// UNVERIFIED account currently holding it. That account is a self-asserted
/// squatter, and its email is unbound to a unique, non-resolvable sentinel. A
/// collision with another VERIFIED identity is left intact, so the upsert then
/// violates `users_email_lower_unique` and the login fails with 409. Verified
/// identities never silently steal each other's email. Reclaim + upsert run in
/// one transaction so a concurrent login cannot observe a half-freed email.
///
/// The answer names each inviter of a reclaimed account, per document, so the
/// caller tells them that the address left the member they invited
/// (`SRV-295`).
pub async fn sign_in_from_oidc(
    pool: &PgPool,
    oidc_provider: ambre_core::OidcProvider,
    oidc_issuer: &openidconnect::IssuerUrl,
    oidc_subject: &openidconnect::SubjectIdentifier,
    email: &ambre_core::Email,
    display_name: &ambre_core::Validated<ambre_core::DisplayName>,
) -> Result<OidcSignIn, AppError> {
    let mut tx = pool.begin().await?;

    // Reclaim the email from any UNVERIFIED holder that is not this same OIDC
    // identity. The sentinel keeps the NOT NULL + unique invariants. It makes the
    // squatter's address non-resolvable for sharing (`get_user_by_email` will no
    // longer return it), and re-settable by that account later. A verified holder
    // is deliberately NOT touched, so the upsert below surfaces 409.
    let reclaimed: Vec<Id<User>> = sqlx::query_scalar::<_, PgId>(
        "UPDATE users \
         SET email = 'reclaimed-' || id::text || '@invalid.local', email_verified = false \
         WHERE lower(email) = lower($1) \
           AND email_verified = false \
           AND (oidc_issuer IS DISTINCT FROM $2 OR oidc_subject IS DISTINCT FROM $3) \
         RETURNING id",
    )
    .bind(email.as_str())
    .bind(oidc_issuer.as_str())
    .bind(oidc_subject.as_str())
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(Id::from)
    .collect();
    // The reclaim leaves every member row and wrapped key of the account. Its
    // inviters are read in the same transaction, so none is missed.
    let reclaims = reclaim_notices(&mut tx, &reclaimed).await?;

    let row = sqlx::query(
        "INSERT INTO users (oidc_provider, oidc_issuer, oidc_subject, email, email_verified, display_name) VALUES ($1, $2, $3, $4, true, $5) ON CONFLICT (oidc_issuer, oidc_subject) DO UPDATE SET email = $4, email_verified = true, display_name = $5 RETURNING id, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, (xmax = 0) AS inserted"
    )
    .bind(oidc_provider.as_str())
    .bind(oidc_issuer.as_str())
    .bind(oidc_subject.as_str())
    .bind(email.as_str())
    .bind(display_name.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(map_email_conflict)?;

    let inserted: bool = row.try_get("inserted")?;
    let user = UserRow {
        id: row.try_get::<PgId, _>("id")?.into(),
        email: ambre_core::Email::new(row.try_get("email")?),
        email_verified: ambre_core::Flag::from_bool(row.try_get("email_verified")?),
        display_name: decode_display_name(row.try_get("display_name")?)?,
        master_key_enc: row
            .try_get::<Option<Vec<u8>>, _>("master_key_enc")?
            .map(ambre_core::Encoded::new),
        master_key_salt: row
            .try_get::<Option<Vec<u8>>, _>("master_key_salt")?
            .map(decode_salt)
            .transpose()?,
        signing_pubkey: decode_opt_key(row.try_get("signing_pubkey")?)?,
        exchange_pubkey: decode_opt_exchange_key(row.try_get("exchange_pubkey")?)?,
    };

    tx.commit().await?;

    if inserted {
        tracing::debug!(user_id = %user.id, provider = %oidc_provider, "user created");
    }

    Ok(OidcSignIn { user, reclaims })
}

/// Whether `email` is the stand-in a reclaim writes in place of the address
/// it took (`SRV-321`). The reclaim of `sign_in_from_oidc` writes the one
/// form this reads. No provider verifies an address of that domain, and no
/// route takes a self-asserted one, so no other account holds such an
/// address. An account that sets an address again holds it.
#[must_use]
pub fn address_standing(email: &ambre_core::Email) -> ambre_core::AddressStanding {
    if email.as_str().starts_with("reclaimed-") && email.as_str().ends_with("@invalid.local") {
        ambre_core::AddressStanding::Reclaimed
    } else {
        ambre_core::AddressStanding::Held
    }
}

/// What a provider sign-in wrote: the account, and the inviters each reclaim
/// of its address owes a notice (`SRV-295`).
#[derive(Debug)]
pub struct OidcSignIn {
    pub user: UserRow,
    pub reclaims: Vec<ReclaimNotice>,
}

/// One inviter of an account whose address a provider sign-in reclaimed: on
/// `document`, `inviter` invited `member`, which no longer holds the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimNotice {
    pub document: Id<ambre_core::id::Document>,
    pub inviter: Id<User>,
    pub member: Id<User>,
}

/// The inviters of the accounts `reclaimed`, one per document and member. A
/// row with no inviter, or that the member wrapped for itself, owes nobody a
/// notice.
async fn reclaim_notices(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    reclaimed: &[Id<User>],
) -> Result<Vec<ReclaimNotice>, AppError> {
    if reclaimed.is_empty() {
        return Ok(Vec::new());
    }
    let accounts: Vec<uuid::Uuid> = reclaimed.iter().map(|id| id_to_uuid(*id)).collect();
    let rows = sqlx::query(
        "SELECT DISTINCT document_id, invited_by, user_id FROM document_members \
         WHERE user_id = ANY($1) AND invited_by IS NOT NULL AND invited_by <> user_id",
    )
    .bind(accounts)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ReclaimNotice {
                document: row.try_get::<PgId, _>("document_id")?.into(),
                inviter: row.try_get::<PgId, _>("invited_by")?.into(),
                member: row.try_get::<PgId, _>("user_id")?.into(),
            })
        })
        .collect()
}

/// The account a provider sign-in wrote, without the notices its reclaims
/// owe. The tests of the address namespace read the account alone.
#[cfg(test)]
pub async fn upsert_user_from_oidc(
    pool: &PgPool,
    oidc_provider: ambre_core::OidcProvider,
    oidc_subject: &openidconnect::SubjectIdentifier,
    email: &ambre_core::Email,
    display_name: &ambre_core::Validated<ambre_core::DisplayName>,
) -> Result<UserRow, AppError> {
    let issuer = crate::oidc::provider_issuer(oidc_provider, None)?;
    sign_in_from_oidc(
        pool,
        oidc_provider,
        &issuer,
        oidc_subject,
        email,
        display_name,
    )
    .await
    .map(|signed_in| signed_in.user)
}

/// The two public keys a development sign-in registers.
pub struct DevKeys<'a> {
    pub signing: &'a FixedBytes<ambre_core::PublicKey>,
    pub exchange: &'a FixedBytes<ambre_core::ExchangePublicKey>,
}

/// Dev-only: upserts a user identified by the name it signs in under.
///
/// Stored under `oidc_provider = 'dev'`, `oidc_subject = username`, with no
/// issuer, so the index of development accounts keeps it idempotent across
/// refreshes. The supplied
/// pubkeys (derived deterministically client-side from the username) are written
/// once, on insert or on the first sign-in of an account that holds none.
/// `master_key_*` stay NULL: dev keys never round-trip through the server.
/// Gated by `Config::dev_mode` at the call site.
///
/// A row that holds keys keeps them. A sign-in that names other keys writes
/// nothing and fails with a conflict, so a caller cannot swap the signing key
/// of a member (`SRV-337`).
///
/// With `keys` `None`, the row keeps the keys it holds, or holds none. The
/// account then owes its key setup, as a provider account does (`SRV-319`).
pub async fn dev_upsert_user(
    pool: &PgPool,
    username: &ambre_core::Validated<ambre_core::DisplayName>,
    keys: Option<DevKeys<'_>>,
) -> Result<UserRow, AppError> {
    let email = format!("{}@dev.local", username.as_str());
    let signing = keys.as_ref().map(|keys| keys.signing.as_slice());
    let exchange = keys.as_ref().map(|keys| keys.exchange.as_slice());
    // `(xmax = 0)` is true only for a freshly INSERTed row (vs an ON CONFLICT
    // UPDATE), so we can log creation without a prior existence check.
    let row = sqlx::query(
        "INSERT INTO users (oidc_provider, oidc_subject, email, email_verified, display_name, signing_pubkey, exchange_pubkey) VALUES ('dev', $1, $2, true, $1, $3, $4) ON CONFLICT (oidc_subject) WHERE oidc_provider = 'dev' DO UPDATE SET email = $2, email_verified = true, display_name = $1, signing_pubkey = COALESCE(users.signing_pubkey, $3), exchange_pubkey = COALESCE(users.exchange_pubkey, $4) WHERE users.signing_pubkey IS NULL OR $3::bytea IS NULL OR (users.signing_pubkey = $3 AND users.exchange_pubkey = $4) RETURNING id, email, email_verified, display_name, master_key_enc, master_key_salt, signing_pubkey, exchange_pubkey, (xmax = 0) AS inserted"
    )
    .bind(username.as_str())
    .bind(&email)
    .bind(signing)
    .bind(exchange)
    .fetch_optional(pool)
    .await
    .map_err(map_email_conflict)?
    // The guard of the update refused the row: it holds other keys.
    .ok_or_else(|| AppError::Conflict {
        detail: "the account holds other keys".into(),
    })?;

    let inserted: bool = row.try_get("inserted")?;
    if inserted {
        tracing::debug!(user_id = %row.try_get::<uuid::Uuid, _>("id")?, provider = "dev", "user created");
    }

    Ok(UserRow {
        id: row.try_get::<PgId, _>("id")?.into(),
        email: ambre_core::Email::new(row.try_get("email")?),
        email_verified: ambre_core::Flag::from_bool(row.try_get("email_verified")?),
        display_name: decode_display_name(row.try_get("display_name")?)?,
        master_key_enc: row
            .try_get::<Option<Vec<u8>>, _>("master_key_enc")?
            .map(ambre_core::Encoded::new),
        master_key_salt: row
            .try_get::<Option<Vec<u8>>, _>("master_key_salt")?
            .map(decode_salt)
            .transpose()?,
        signing_pubkey: decode_opt_key(row.try_get("signing_pubkey")?)?,
        exchange_pubkey: decode_opt_exchange_key(row.try_get("exchange_pubkey")?)?,
    })
}

pub use ambre_core::{UserSearch, Validated};

/// The account whose whole address is `query`, compared without regard to
/// case, the rule of `users_email_lower_unique`. A part of an address or of a
/// name matches nothing, so a session cannot list the directory (`SRV-280`).
/// The answer holds one account at most.
///
/// An account without keys is left out: nothing can be shared with it until it
/// registers them (`SRV-163`).
pub async fn search_users(
    pool: &PgPool,
    query: &Validated<UserSearch>,
) -> Result<Vec<UserInfoRow>, AppError> {
    let rows = sqlx::query(
        "SELECT id, email, email_verified, display_name, signing_pubkey, exchange_pubkey FROM users WHERE signing_pubkey IS NOT NULL AND lower(email) = lower($1)"
    )
    .bind(query.as_str())
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UserInfoRow {
                id: r.try_get::<PgId, _>("id")?.into(),
                email: ambre_core::Email::new(r.try_get("email")?),
                email_verified: ambre_core::Flag::from_bool(r.try_get("email_verified")?),
                display_name: decode_display_name(r.try_get("display_name")?)?,
                signing_pubkey: decode_key(r.try_get("signing_pubkey")?)?,
                exchange_pubkey: decode_exchange_key(r.try_get("exchange_pubkey")?)?,
            })
        })
        .collect()
}

/// Replaces the sealed Master Seed of `user_id` with `sealed`, only while the
/// stored salt is still `replaces` (`SRV-279`).
///
/// The guard and the write are one statement, so two re-seals that read the
/// same seal cannot both land: the second finds another salt. A replayed proof
/// names a salt the account no longer holds, and fails the same way. An
/// account with no sealed seed holds no salt, so it matches nothing.
pub async fn replace_master_key(
    pool: &PgPool,
    user_id: &Id<User>,
    replaces: &FixedBytes<ambre_core::MasterKeySalt>,
    sealed: &ambre_protocol::master_rotation::SealedSeed,
) -> Result<(), AppError> {
    let uid = id_to_uuid(*user_id);
    let result = sqlx::query(
        "UPDATE users SET master_key_enc = $2, master_key_salt = $3 WHERE id = $1 AND master_key_salt = $4",
    )
    .bind(uid)
    .bind(sealed.master_key_enc.as_bytes())
    .bind(sealed.master_key_salt.as_slice())
    .bind(replaces.as_slice())
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict {
            detail: "the sealed master key changed since the re-seal read it".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ZK-018`, `ZK-055`: the account row holds what the relay may learn of a user in
    /// the clear. Beside it sit the identifier it mints and the sealed keys.
    /// A column added here widens what the relay learns, so it fails this test
    /// before it ships.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_account_table_holds_only_what_the_relay_may_learn(pool: PgPool) {
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'users'",
        )
        .fetch_all(&pool)
        .await
        .expect("query");
        let got: std::collections::BTreeSet<&str> = columns.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            [
                // What the rule allows in the clear.
                "display_name",
                "email",
                "email_verified",
                "exchange_pubkey",
                "oidc_issuer",
                "oidc_provider",
                "oidc_subject",
                "signing_pubkey",
                // The identifier the relay mints, and the sealed key material.
                "id",
                "master_key_enc",
                "master_key_salt",
                // Which document holds the settings of the account, the one
                // thing the relay learns of them (`ZK-055`).
                "settings_document",
            ]
            .into_iter()
            .collect()
        );
    }

    /// #3: a provider-verified OIDC login RECLAIMS an email squatted by an
    /// unverified account of the retired setup. The login succeeds (closing the
    /// unauthenticated login-DoS), the OIDC row owns the verified email, and the
    /// squatter's row no longer resolves for that email.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn oidc_login_reclaims_email_from_unverified_squatter(pool: PgPool) {
        let squatter = create_user(
            &pool,
            &ambre_core::Email::from("alice@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
            &ambre_core::Encoded::new([1u8; 32].to_vec()),
            &FixedBytes::<ambre_core::MasterKeySalt>::from_array([2u8; 32]),
            &FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([4u8; 32]),
        )
        .await
        .expect("setup account");
        assert_eq!(
            squatter.email_verified,
            EmailProof::Asserted,
            "a setup email is unverified"
        );

        let oidc = upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-123".to_owned()),
            &ambre_core::Email::from("Alice@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Alice").unwrap(),
        )
        .await
        .expect("verified login must reclaim the squatted email");
        assert_eq!(
            oidc.email_verified,
            EmailProof::Verified,
            "the OIDC email is verified"
        );
        assert_ne!(oidc.id, squatter.id, "a distinct OIDC identity");

        // The email now resolves to the verified OIDC account, not the squatter.
        let resolved = get_user_by_email(&pool, &ambre_core::Email::from("alice@example.com"))
            .await
            .expect("query")
            .expect("email still resolves");
        assert_eq!(resolved.id, oidc.id);

        // The squatter row survives but its email was unbound to a sentinel.
        let squatter_now = get_user(&pool, &squatter.id)
            .await
            .expect("query")
            .expect("squatter row still exists");
        assert_ne!(
            squatter_now.email.as_str().to_lowercase(),
            "alice@example.com",
            "the squatter no longer holds the reclaimed email"
        );
        // The stand-in reads as a reclaim, and neither live address does
        // (`SRV-321`).
        assert_eq!(
            address_standing(&squatter_now.email),
            ambre_core::AddressStanding::Reclaimed
        );
        assert_eq!(
            address_standing(&oidc.email),
            ambre_core::AddressStanding::Held
        );
        assert_eq!(
            address_standing(&squatter.email),
            ambre_core::AddressStanding::Held
        );
    }

    /// #3: a verified email is NOT reclaimable. Once an OIDC identity owns it, a
    /// later self-asserted claim for the same email fails with 409.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn verified_email_cannot_be_squatted_by_setup(pool: PgPool) {
        upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-1".to_owned()),
            &ambre_core::Email::from("dave@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Dave").unwrap(),
        )
        .await
        .expect("verified login");

        let err = create_user(
            &pool,
            &ambre_core::Email::from("DAVE@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Impostor").unwrap(),
            &ambre_core::Encoded::new([9u8; 32].to_vec()),
            &FixedBytes::<ambre_core::MasterKeySalt>::from_array([8u8; 32]),
            &FixedBytes::<ambre_core::PublicKey>::from_array([7u8; 32]),
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([6u8; 32]),
        )
        .await
        .expect_err("a verified email must not be claimable via setup");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// #3: two DIFFERENT verified identities (one provider, two subjects) cannot
    /// hold the same email -- the second login surfaces 409 rather than
    /// silently stealing it (reclaim only applies to UNVERIFIED holders).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn verified_email_collision_is_conflict(pool: PgPool) {
        upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-a".to_owned()),
            &ambre_core::Email::from("erin@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Erin").unwrap(),
        )
        .await
        .expect("first verified login");

        let err = upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-b".to_owned()),
            &ambre_core::Email::from("erin@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Erin").unwrap(),
        )
        .await
        .expect_err("a second verified identity must not steal the email");
        assert!(matches!(err, AppError::Conflict { .. }), "got {err:?}");
    }

    /// A returning OIDC identity refreshing its own email is idempotent (the
    /// reclaim skips its own row, so it does not self-conflict).
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn oidc_relogin_keeps_its_own_email(pool: PgPool) {
        let first = upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-x".to_owned()),
            &ambre_core::Email::from("frank@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Frank").unwrap(),
        )
        .await
        .expect("first login");
        let second = upsert_user_from_oidc(
            &pool,
            ambre_core::OidcProvider::Google,
            &openidconnect::SubjectIdentifier::new("sub-x".to_owned()),
            &ambre_core::Email::from("frank@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Frank").unwrap(),
        )
        .await
        .expect("re-login must not self-conflict");
        assert_eq!(first.id, second.id);
        assert_eq!(second.email_verified, EmailProof::Verified);
    }

    /// `SRV-344`: an account is keyed by the issuer and the subject. Two
    /// issuers that assert one subject sign two people in to two accounts,
    /// and Google's issuer a third. A second sign-in through one issuer finds
    /// its own account. A second issuer never takes the verified address of
    /// the first: its sign-in is a conflict.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_account_is_keyed_by_its_issuer_and_subject(pool: PgPool) {
        let subject = openidconnect::SubjectIdentifier::new("same-subject".to_owned());
        let name = ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").unwrap();
        let issuer = |text: &str| openidconnect::IssuerUrl::new(text.to_owned()).unwrap();
        let sign_in = |provider, issuer: openidconnect::IssuerUrl, email: &'static str| {
            let pool = pool.clone();
            let subject = subject.clone();
            let name = name.clone();
            async move {
                sign_in_from_oidc(
                    &pool,
                    provider,
                    &issuer,
                    &subject,
                    &ambre_core::Email::from(email),
                    &name,
                )
                .await
                .map(|signed_in| signed_in.user.id)
            }
        };
        let oidc = ambre_core::OidcProvider::Oidc;
        let google = ambre_core::OidcProvider::Google;
        let google_issuer = crate::oidc::provider_issuer(google, None).unwrap();

        let a = sign_in(
            oidc,
            issuer("https://a.example.org/realms/x"),
            "a@example.org",
        )
        .await
        .unwrap();
        let b = sign_in(
            oidc,
            issuer("https://b.example.org/realms/x"),
            "b@example.org",
        )
        .await
        .unwrap();
        let g = sign_in(google, google_issuer, "g@example.org")
            .await
            .unwrap();
        assert_ne!(a, b, "two issuers, one subject");
        assert_ne!(a, g);
        assert_ne!(b, g);
        let again = sign_in(
            oidc,
            issuer("https://a.example.org/realms/x"),
            "a@example.org",
        )
        .await
        .unwrap();
        assert_eq!(again, a, "the same issuer and subject");
        let taken = sign_in(
            oidc,
            issuer("https://c.example.org/realms/x"),
            "a@example.org",
        )
        .await;
        assert!(matches!(taken, Err(AppError::Conflict { .. })), "{taken:?}");
    }

    /// N5 guard: uniqueness is case-insensitive across setup accounts too.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn create_user_duplicate_email_is_conflict(pool: PgPool) {
        create_user(
            &pool,
            &ambre_core::Email::from("bob@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").unwrap(),
            &ambre_core::Encoded::new([1u8; 32].to_vec()),
            &FixedBytes::<ambre_core::MasterKeySalt>::from_array([2u8; 32]),
            &FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([4u8; 32]),
        )
        .await
        .expect("first claim");

        let err = create_user(
            &pool,
            &ambre_core::Email::from("BOB@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Impostor").unwrap(),
            &ambre_core::Encoded::new([9u8; 32].to_vec()),
            &FixedBytes::<ambre_core::MasterKeySalt>::from_array([8u8; 32]),
            &FixedBytes::<ambre_core::PublicKey>::from_array([7u8; 32]),
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([6u8; 32]),
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
            &ambre_core::Email::from("carol@example.com"),
            &ambre_core::Validated::<ambre_core::DisplayName>::parse("Carol").unwrap(),
            &ambre_core::Encoded::new([1u8; 32].to_vec()),
            &FixedBytes::<ambre_core::MasterKeySalt>::from_array([2u8; 32]),
            &FixedBytes::<ambre_core::PublicKey>::from_array([3u8; 32]),
            &FixedBytes::<ambre_core::ExchangePublicKey>::from_array([4u8; 32]),
        )
        .await
        .expect("create user");

        let found = get_user_by_email(&pool, &ambre_core::Email::from("CAROL@EXAMPLE.COM"))
            .await
            .expect("query")
            .expect("must resolve despite case difference");
        assert_eq!(found.id, created.id);
    }
}
