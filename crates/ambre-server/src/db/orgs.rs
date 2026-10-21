// ambre-server/src/db/orgs.rs
// Organization rows and org membership / recovery queries.

use super::*;

/// Row from the `organizations` table.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OrganizationRow {
    #[sqlx(try_from = "PgId")]
    pub id: ambre_core::OrgId,
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
    #[sqlx(try_from = "PgId")]
    pub user_id: ambre_core::UserId,
    #[sqlx(try_from = "String")]
    pub email: ambre_core::Email,
    pub display_name: String,
    #[sqlx(try_from = "String")]
    pub role: ambre_core::Role,
    #[sqlx(try_from = "PgOptBytes")]
    pub shamir_share_enc: Option<ambre_core::SealedShare>,
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
    owner_share_enc: Option<&ambre_core::SealedShare>,
) -> Result<OrganizationRow, AppError> {
    let org_id = &ambre_core::OrgId::new();
    let uid = id_to_uuid(*owner_id);

    let mut tx = pool.begin().await?;

    let org = sqlx::query_as::<_, OrganizationRow>(
        "INSERT INTO organizations (id, name, org_pubkey, shamir_threshold, shamir_total)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total, created_at",
    )
    .bind(id_to_uuid(*org_id))
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
    .bind(id_to_uuid(*org_id))
    .bind(uid)
    .bind(owner_share_enc.map(|v| v.as_bytes()))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(org)
}

/// Fetches an organization by id.
pub async fn get_organization(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "SELECT id, name, org_pubkey, shamir_threshold, shamir_total, created_at
         FROM organizations WHERE id = $1",
    )
    .bind(id_to_uuid(*org_id))
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
    org_id: &ambre_core::OrgId,
    name: &str,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "UPDATE organizations SET name = $2 WHERE id = $1
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total, created_at",
    )
    .bind(id_to_uuid(*org_id))
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Deletes an organization (cascades to members via FK constraint).
pub async fn delete_organization(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
) -> Result<bool, AppError> {
    let rows = sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(id_to_uuid(*org_id))
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Maps a foreign-key violation on the member's `user_id` to a clean 404
/// instead of surfacing the FK error as an opaque 500. The `user_id` in the
/// request is only format-validated as a UUID, so a well-formed but unknown
/// user reaches the INSERT and trips `organization_members_user_id_fkey`.
fn map_member_user_fk(e: sqlx::Error) -> AppError {
    match &e {
        sqlx::Error::Database(db)
            if db.is_foreign_key_violation()
                && db.constraint() == Some("organization_members_user_id_fkey") =>
        {
            AppError::NotFound { resource: "user" }
        }
        _ => AppError::Database(e),
    }
}

/// Adds a member to an organization with an optional encrypted Shamir share.
///
/// The upsert doubles as the role-change path (`ON CONFLICT DO UPDATE`), so it
/// enforces the same last-owner invariant as `update_org_member_role`. It
/// refuses to demote the organization's only remaining owner. That would leave
/// the organization with no one able to manage membership or recover keys. The
/// check and the write run in one transaction with the owner row locked (`FOR
/// UPDATE`) so concurrent demotions cannot both pass the count.
pub async fn add_org_member(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
    user_id: &ambre_core::UserId,
    role: ambre_core::Role,
    shamir_share_enc: Option<&ambre_core::SealedShare>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE org_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .fetch_optional(&mut *tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;

    if current_role == Some(ambre_core::Role::Owner) && role != ambre_core::Role::Owner {
        ensure_not_last_owner(&mut tx, org_id).await?;
    }

    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role, shamir_share_enc)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = $3, shamir_share_enc = COALESCE($4, organization_members.shamir_share_enc)"
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .bind(role.as_str())
    .bind(shamir_share_enc.map(|v| v.as_bytes()))
    .execute(&mut *tx)
    .await
    .map_err(map_member_user_fk)?;

    tx.commit().await?;
    Ok(())
}

/// Fails with a 409 if the organization has only one owner left. Callers hold
/// the target owner's row locked, so the count is stable for the transaction.
async fn ensure_not_last_owner(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: &ambre_core::OrgId,
) -> Result<(), AppError> {
    let owners: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM organization_members WHERE org_id = $1 AND role = 'owner'",
    )
    .bind(id_to_uuid(*org_id))
    .fetch_one(&mut **tx)
    .await?;
    if owners <= 1 {
        return Err(AppError::Conflict {
            detail: "cannot remove the last owner of the organization".into(),
        });
    }
    Ok(())
}

/// Lists members of an organization.
pub async fn list_org_members(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
) -> Result<Vec<OrgMemberRow>, AppError> {
    sqlx::query_as::<_, OrgMemberRow>(
        "SELECT om.user_id, u.email, u.display_name, om.role, om.shamir_share_enc,
                om.created_at
         FROM organization_members om
         JOIN users u ON u.id = om.user_id
         WHERE om.org_id = $1
         ORDER BY om.created_at",
    )
    .bind(id_to_uuid(*org_id))
    .fetch_all(pool)
    .await
    .map_err(AppError::from)
}

/// Fetches a single org membership row (raw, without user join).
pub async fn get_org_member(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
    user_id: &ambre_core::UserId,
) -> Result<Option<OrgMembershipRow>, AppError> {
    sqlx::query_as::<_, OrgMembershipRow>(
        "SELECT org_id, user_id, role, shamir_share_enc, created_at
         FROM organization_members
         WHERE org_id = $1 AND user_id = $2",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Raw `organization_members` row (no join).
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
pub struct OrgMembershipRow {
    #[allow(dead_code)]
    #[sqlx(try_from = "PgId")]
    pub org_id: ambre_core::OrgId,
    #[allow(dead_code)]
    #[sqlx(try_from = "PgId")]
    pub user_id: ambre_core::UserId,
    #[sqlx(try_from = "String")]
    pub role: ambre_core::Role,
    #[allow(dead_code)]
    #[sqlx(try_from = "PgOptBytes")]
    pub shamir_share_enc: Option<ambre_core::SealedShare>,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Removes a member from an organization.
///
/// Refuses to delete the organization's only remaining owner (last-owner
/// lockout): with no owner left, nobody could manage membership or recover
/// member keys. The check and the delete share one transaction with the target
/// row locked so two concurrent removals cannot both see two owners.
pub async fn remove_org_member(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
    user_id: &ambre_core::UserId,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE org_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .fetch_optional(&mut *tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;

    let Some(current_role) = current_role else {
        return Ok(false);
    };
    if current_role == ambre_core::Role::Owner {
        ensure_not_last_owner(&mut tx, org_id).await?;
    }

    let rows = sqlx::query("DELETE FROM organization_members WHERE org_id = $1 AND user_id = $2")
        .bind(id_to_uuid(*org_id))
        .bind(id_to_uuid(*user_id))
        .execute(&mut *tx)
        .await?
        .rows_affected();

    tx.commit().await?;
    Ok(rows > 0)
}

/// Updates a member's role.
///
/// Enforces the last-owner invariant: demoting the organization's only owner is
/// refused with a 409 (see `remove_org_member`).
pub async fn update_org_member_role(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
    user_id: &ambre_core::UserId,
    role: ambre_core::Role,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM organization_members WHERE org_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .fetch_optional(&mut *tx)
    .await?;
    let current_role = decode_role_opt(current_role)?;

    if current_role == Some(ambre_core::Role::Owner) && role != ambre_core::Role::Owner {
        ensure_not_last_owner(&mut tx, org_id).await?;
    }

    sqlx::query("UPDATE organization_members SET role = $3 WHERE org_id = $1 AND user_id = $2")
        .bind(id_to_uuid(*org_id))
        .bind(id_to_uuid(*user_id))
        .bind(role.as_str())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Collects Shamir shares for a given organization from the database.
#[allow(dead_code)]
pub async fn get_org_shamir_shares(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
) -> Result<Vec<(ambre_core::UserId, ambre_core::SealedShare)>, AppError> {
    let rows = sqlx::query(
        "SELECT user_id, shamir_share_enc FROM organization_members WHERE org_id = $1 AND shamir_share_enc IS NOT NULL"
    )
    .bind(id_to_uuid(*org_id))
    .fetch_all(pool)
    .await?;
    let shares = rows
        .iter()
        .map(|r| {
            Ok((
                r.try_get::<PgId, _>("user_id")?.into(),
                ambre_core::SealedShare::new(r.try_get("shamir_share_enc")?),
            ))
        })
        .collect::<Result<Vec<(ambre_core::UserId, ambre_core::SealedShare)>, sqlx::Error>>()?;
    Ok(shares)
}

/// Logs a master key recovery event.
pub async fn log_org_recovery(
    pool: &PgPool,
    org_id: &ambre_core::OrgId,
    target_user_id: &ambre_core::UserId,
    recovered_by: &ambre_core::UserId,
    reason: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO org_recovery_log (org_id, target_user_id, recovered_by, reason)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*target_user_id))
    .bind(id_to_uuid(*recovered_by))
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetches the organization master key for a specific user (encrypted with org pubkey).
pub async fn get_user_org_master_key_enc(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
) -> Result<Option<ambre_core::EscrowedKey>, AppError> {
    let row = sqlx::query("SELECT org_master_key_enc FROM users WHERE id = $1")
        .bind(id_to_uuid(*user_id))
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<Vec<u8>>, _>("org_master_key_enc")
            .ok()
            .flatten()
            .map(ambre_core::EscrowedKey::new)
    }))
}
