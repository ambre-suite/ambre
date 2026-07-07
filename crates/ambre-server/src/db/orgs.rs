// ambre-server/src/db/orgs.rs
// Organization rows and org membership / recovery queries.

use super::*;

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
