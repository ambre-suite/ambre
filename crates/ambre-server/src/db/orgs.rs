// ambre-server/src/db/orgs.rs
// Organization rows and org membership / recovery queries.

use super::*;
use ambre_core::{Bounded, Bounds, Refusal};

/// The error of a name the rule of an [`OrgName`] refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "an organization name is blank or past {} characters",
    ambre_core::NAME_MAX_CHARS
)]
pub struct InvalidOrgName;

/// The rule of the name an organization is created or renamed with: trimmed,
/// never blank, at most [`ambre_core::NAME_MAX_CHARS`] characters
/// (`SRV-233`). A `Validated<OrgName>` holds one, and the rule keeps the
/// trimmed text. A row read back keeps its name as text, so a name stored
/// before the rule still lists. A request body decodes it through the rule,
/// so a body naming a name that breaks the rule does not decode. `Debug`
/// prints the length alone.
pub enum OrgName {}

impl ambre_core::Rule for OrgName {
    type Error = InvalidOrgName;

    fn admit(text: String) -> Result<String, InvalidOrgName> {
        let trimmed = text.trim();
        if trimmed.is_empty() || !ambre_core::name_fits(trimmed) {
            Err(InvalidOrgName)
        } else if trimmed.len() == text.len() {
            Ok(text)
        } else {
            Ok(trimmed.to_owned())
        }
    }
}

/// The k-of-n split an organization declares for its recovery key: `k`
/// shares of `n` reconstruct it. [`ShamirParams::new`] holds
/// `1 <= threshold <= total`, so no stored or decoded split breaks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "ShamirWire", into = "ShamirWire")]
pub struct ShamirParams(ShamirWire);

/// How many shares of a split reconstruct the key: the `k` of k-of-n, at
/// least 1. A quantity apart from [`ShareTotal`], so a split never takes the
/// two swapped. The value is a `Bounded<ShareThreshold>`, a bare `i16` on the
/// wire and in the `smallint` column.
pub enum ShareThreshold {}

impl Bounds for ShareThreshold {
    type Repr = i16;
    const MIN: i16 = 1;
    const MAX: i16 = i16::MAX;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ShareThreshold");
    const REFUSAL: Refusal = Refusal::new("a share threshold is at least 1, not ", "");
}

/// How many shares a split cuts the key into: the `n` of k-of-n, at least 1.
/// The value is a `Bounded<ShareTotal>`, a bare `i16` on the wire and in the
/// `smallint` column.
pub enum ShareTotal {}

impl Bounds for ShareTotal {
    type Repr = i16;
    const MIN: i16 = 1;
    const MAX: i16 = i16::MAX;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ShareTotal");
    const REFUSAL: Refusal = Refusal::new("a share total is at least 1, not ", "");
}

/// The two fields of a split as the request body, the answer and the
/// `organizations` row spell them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShamirWire {
    shamir_threshold: Bounded<ShareThreshold>,
    shamir_total: Bounded<ShareTotal>,
}

impl ShamirParams {
    /// The split of `total` shares that `threshold` of them reconstruct.
    /// Refuses a threshold above the total. Each quantity already holds its
    /// count at 1 or more.
    pub fn new(
        threshold: Bounded<ShareThreshold>,
        total: Bounded<ShareTotal>,
    ) -> Result<Self, AppError> {
        if threshold.get() > total.get() {
            return Err(AppError::Conflict {
                detail: "shamir_threshold/shamir_total out of range (need 1 <= threshold <= total)"
                    .into(),
            });
        }
        Ok(Self(ShamirWire {
            shamir_threshold: threshold,
            shamir_total: total,
        }))
    }

    /// How many shares reconstruct the key.
    pub fn threshold(self) -> Bounded<ShareThreshold> {
        self.0.shamir_threshold
    }

    /// How many shares the key is split into.
    pub fn total(self) -> Bounded<ShareTotal> {
        self.0.shamir_total
    }
}

impl TryFrom<ShamirWire> for ShamirParams {
    type Error = AppError;

    fn try_from(wire: ShamirWire) -> Result<Self, AppError> {
        Self::new(wire.shamir_threshold, wire.shamir_total)
    }
}

impl From<ShamirParams> for ShamirWire {
    fn from(params: ShamirParams) -> Self {
        params.0
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ShamirParams {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        let decode = |e: &dyn std::fmt::Display| sqlx::Error::Decode(e.to_string().into());
        let threshold =
            Bounded::<ShareThreshold>::try_new(row.try_get::<i16, _>("shamir_threshold")?)
                .map_err(|e| decode(&e))?;
        let total = Bounded::<ShareTotal>::try_new(row.try_get::<i16, _>("shamir_total")?)
            .map_err(|e| decode(&e))?;
        Self::new(threshold, total).map_err(|e| decode(&e))
    }
}

/// Row from the `organizations` table.
#[derive(Debug, sqlx::FromRow)]
pub struct OrganizationRow {
    #[sqlx(try_from = "PgId")]
    pub id: Id<Org>,
    pub name: String,
    #[sqlx(try_from = "PgKey")]
    pub org_pubkey: FixedBytes<ambre_core::ExchangePublicKey>,
    #[sqlx(flatten)]
    pub shamir: ShamirParams,
}

/// Row from `organization_members` joined with `users`.
#[derive(Debug, sqlx::FromRow)]
pub struct OrgMemberRow {
    #[sqlx(try_from = "PgId")]
    pub user_id: Id<User>,
    #[sqlx(try_from = "String")]
    pub email: ambre_core::Email,
    #[sqlx(try_from = "String")]
    pub display_name: ambre_core::Validated<ambre_core::DisplayName>,
    #[sqlx(try_from = "String")]
    pub role: ambre_core::Role,
    #[sqlx(try_from = "PgOptBytes")]
    pub shamir_share_enc: Option<ambre_core::Encoded<ambre_core::SealedShare>>,
}

/// Creates an organization and inserts its first member as owner.
pub async fn create_organization(
    pool: &PgPool,
    name: &ambre_core::Validated<OrgName>,
    org_pubkey: &FixedBytes<ambre_core::ExchangePublicKey>,
    shamir: ShamirParams,
    owner_id: &Id<User>,
) -> Result<OrganizationRow, AppError> {
    let org_id = &Id::<Org>::mint();
    let uid = id_to_uuid(*owner_id);

    let mut tx = pool.begin().await?;

    let org = sqlx::query_as::<_, OrganizationRow>(
        "INSERT INTO organizations (id, name, org_pubkey, shamir_threshold, shamir_total)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total",
    )
    .bind(id_to_uuid(*org_id))
    .bind(name.as_str())
    .bind(org_pubkey.as_slice())
    .bind(shamir.threshold().get())
    .bind(shamir.total().get())
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(id_to_uuid(*org_id))
    .bind(uid)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(org)
}

/// Fetches an organization by id.
pub async fn get_organization(
    pool: &PgPool,
    org_id: &Id<Org>,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "SELECT id, name, org_pubkey, shamir_threshold, shamir_total
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
    user_id: &Id<User>,
) -> Result<Vec<OrganizationRow>, AppError> {
    let uid = id_to_uuid(*user_id);
    sqlx::query_as::<_, OrganizationRow>(
        "SELECT o.id, o.name, o.org_pubkey, o.shamir_threshold, o.shamir_total
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
    org_id: &Id<Org>,
    name: &ambre_core::Validated<OrgName>,
) -> Result<Option<OrganizationRow>, AppError> {
    sqlx::query_as::<_, OrganizationRow>(
        "UPDATE organizations SET name = $2 WHERE id = $1
         RETURNING id, name, org_pubkey, shamir_threshold, shamir_total",
    )
    .bind(id_to_uuid(*org_id))
    .bind(name.as_str())
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

/// Deletes an organization (cascades to members via FK constraint).
pub async fn delete_organization(pool: &PgPool, org_id: &Id<Org>) -> Result<bool, AppError> {
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
            AppError::NotFound {
                resource: crate::error::Resource::User,
            }
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
    org_id: &Id<Org>,
    user_id: &Id<User>,
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

    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role)
         VALUES ($1, $2, $3)
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = $3",
    )
    .bind(id_to_uuid(*org_id))
    .bind(id_to_uuid(*user_id))
    .bind(role.as_str())
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
    org_id: &Id<Org>,
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
    org_id: &Id<Org>,
) -> Result<Vec<OrgMemberRow>, AppError> {
    sqlx::query_as::<_, OrgMemberRow>(
        "SELECT om.user_id, u.email, u.display_name, om.role, om.shamir_share_enc
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
    org_id: &Id<Org>,
    user_id: &Id<User>,
) -> Result<Option<OrgMembershipRow>, AppError> {
    sqlx::query_as::<_, OrgMembershipRow>(
        "SELECT role
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
pub struct OrgMembershipRow {
    #[sqlx(try_from = "String")]
    pub role: ambre_core::Role,
}

/// Removes a member from an organization.
///
/// Refuses to delete the organization's only remaining owner (last-owner
/// lockout): with no owner left, nobody could manage membership or recover
/// member keys. The check and the delete share one transaction with the target
/// row locked so two concurrent removals cannot both see two owners.
pub async fn remove_org_member(
    pool: &PgPool,
    org_id: &Id<Org>,
    user_id: &Id<User>,
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
    org_id: &Id<Org>,
    user_id: &Id<User>,
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

/// Logs a master key recovery event.
pub async fn log_org_recovery(
    pool: &PgPool,
    org_id: &Id<Org>,
    target_user_id: &Id<User>,
    recovered_by: &Id<User>,
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
    user_id: &Id<User>,
) -> Result<Option<ambre_core::Encoded<ambre_core::EscrowedKey>>, AppError> {
    let row = sqlx::query("SELECT org_master_key_enc FROM users WHERE id = $1")
        .bind(id_to_uuid(*user_id))
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<Vec<u8>>, _>("org_master_key_enc")
            .ok()
            .flatten()
            .map(ambre_core::Encoded::new)
    }))
}

#[cfg(test)]
mod org_name_tests {
    use super::OrgName;
    use ambre_core::Validated;

    /// A body decodes a name trimmed, and refuses a blank one with the
    /// words of the rule.
    #[test]
    fn a_body_decodes_a_trimmed_name_and_refuses_a_blank_one() {
        let name: Validated<OrgName> = serde_json::from_str("\" Acme \"").expect("a name");
        assert_eq!(name.as_str(), "Acme");
        assert_eq!(serde_json::to_string(&name).expect("a name"), "\"Acme\"");
        assert_eq!(format!("{name:?}"), "4 chars");
        let refused = serde_json::from_str::<Validated<OrgName>>("\"  \"")
            .expect_err("blank")
            .to_string();
        assert!(
            refused.starts_with("an organization name is blank"),
            "{refused}"
        );
    }
}
