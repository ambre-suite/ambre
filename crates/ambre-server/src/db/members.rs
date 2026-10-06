// ambre-server/src/db/members.rs
// Document-member rows and membership / key-rotation queries.

use super::*;

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
/// member's public keys and their wrapped copy of the document key. A client
/// can then locate its own entry and unwrap the key, or rewrap on rotation.
#[derive(Debug)]
pub struct MemberWithUserRow {
    pub user_id: uuid::Uuid,
    pub email: String,
    /// Whether the member's email was provider-verified vs self-asserted at
    /// `/auth/setup` (#3). Lets the member list flag an unverified peer for the
    /// TOFU out-of-band key check.
    pub email_verified: bool,
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

/// Adds (or updates) a user to a document's membership with a role and encrypted document key.
///
/// The upsert doubles as the role-change path (`ON CONFLICT DO UPDATE`), so it enforces the
/// same last-owner invariant as `update_document_member_role`. It refuses to demote the
/// document's only remaining owner. The check and the write run in one transaction with the
/// target row locked (`FOR UPDATE`) so concurrent demotions cannot both pass the count.
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

    let mut tx = pool.begin().await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;

    if current_role.as_deref() == Some("owner") && role != "owner" {
        ensure_not_last_owner(&mut tx, did).await?;
    }

    sqlx::query(
        "INSERT INTO document_members (document_id, user_id, role, doc_key_enc, invited_by) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (document_id, user_id) DO UPDATE SET role = $3, doc_key_enc = $4, invited_by = $5"
    )
    .bind(did)
    .bind(uid)
    .bind(role)
    .bind(doc_key_enc)
    .bind(inviter)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Fails with a 409 if the document has only one owner left. Callers hold the
/// target owner's row locked, so the count is stable for the transaction.
///
/// A document also carries an `owner_id` column that survives membership
/// changes. But every management operation (invite, revoke, role change)
/// requires a live `owner` membership row. Without one, nobody could manage
/// sharing anymore.
async fn ensure_not_last_owner(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: uuid::Uuid,
) -> Result<(), AppError> {
    let owners: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM document_members WHERE document_id = $1 AND role = 'owner'",
    )
    .bind(document_id)
    .fetch_one(&mut **tx)
    .await?;
    if owners <= 1 {
        return Err(AppError::Conflict {
            detail: "cannot remove the last owner of the document".into(),
        });
    }
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
///
/// A member whose account holds no keys is left out. No wrap can address it,
/// so a key rotation that rewraps the listed members drops it (`SRV-163`).
/// Only a row written before the invite refused such an account can match.
pub async fn list_document_members(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Vec<MemberWithUserRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT dm.user_id, u.email, u.email_verified, u.display_name, dm.role, u.signing_pubkey, u.exchange_pubkey, dm.doc_key_enc, dm.invited_by, inv.exchange_pubkey AS wrapper_exchange_pubkey FROM document_members dm INNER JOIN users u ON u.id = dm.user_id LEFT JOIN users inv ON inv.id = dm.invited_by WHERE dm.document_id = $1 AND u.signing_pubkey IS NOT NULL"
    )
    .bind(did)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(MemberWithUserRow {
                user_id: r.try_get("user_id")?,
                email: r.try_get("email")?,
                email_verified: r.try_get("email_verified")?,
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

/// Removes a user from a document's membership, and leaves the document owing
/// a rotation (`SRV-122`). Every key it held is one the member removed holds.
///
/// Refuses to delete the document's only remaining owner (last-owner lockout):
/// with no owner left, nobody could manage sharing. The check and the delete
/// share one transaction with the target row locked so two concurrent removals
/// cannot both see two owners.
pub async fn remove_document_member(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let mut tx = pool.begin().await?;
    delete_member(&mut tx, did, id_to_uuid(*user_id)).await?;
    sqlx::query("UPDATE documents SET rotation_owed = true WHERE id = $1")
        .bind(did)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Removes a member and re-wraps the new key for every member who stays, in
/// one transaction (`ambre-spec-key-rotation.md`, `SYNC-126`). Either both
/// land or neither does: a removal never stands without its rotation.
pub async fn revoke_and_rotate(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
    rotated_by: &ambre_core::UserId,
    keys: &[(ambre_core::UserId, Vec<u8>)],
    link: &ambre_protocol::KeyLink,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let mut tx = pool.begin().await?;
    // The document row first, as every rotation takes it: two rotations of one
    // document queue on it instead of deadlocking on the member rows.
    lock_document(&mut tx, did).await?;
    delete_member(&mut tx, did, id_to_uuid(*user_id)).await?;
    rewrap_members(&mut tx, did, id_to_uuid(*rotated_by), keys, link).await?;
    tx.commit().await?;
    Ok(())
}

/// Locks the document row for the rest of the transaction. A rotation reads
/// the last link under this lock, so two rotations cannot both chain from it.
async fn lock_document(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: uuid::Uuid,
) -> Result<(), AppError> {
    let found: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM documents WHERE id = $1 FOR UPDATE")
            .bind(document_id)
            .fetch_optional(&mut **tx)
            .await?;
    found.map(|_| ()).ok_or(AppError::NotFound {
        resource: "document",
    })
}

/// Appends a rotation's link, inside the caller's transaction and under the
/// document lock (`SYNC-118`). A link that does not start from the last key
/// chained is refused: another rotation landed first, and its key is the one
/// to replace.
async fn append_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: uuid::Uuid,
    link: &ambre_protocol::KeyLink,
) -> Result<(), AppError> {
    let last: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT key_id FROM document_key_links WHERE document_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(document_id)
    .fetch_optional(&mut **tx)
    .await?;
    if last.is_some_and(|last| last != link.previous.as_array()) {
        return Err(AppError::Conflict {
            detail: "the key this rotation replaces is no longer the current one".into(),
        });
    }
    let bytes = link
        .encode()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let stored = sqlx::query(
        "INSERT INTO document_key_links (document_id, key_id, previous_key_id, link) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(document_id)
    .bind(link.key.as_array().as_slice())
    .bind(link.previous.as_array().as_slice())
    .bind(bytes)
    .execute(&mut **tx)
    .await?;
    if stored.rows_affected() == 0 {
        return Err(AppError::Conflict {
            detail: "a link from this key is already stored".into(),
        });
    }
    Ok(())
}

/// Every link of a document, in the order the rotations landed (`SYNC-117`).
pub async fn document_key_links(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Vec<Vec<u8>>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT link FROM document_key_links WHERE document_id = $1 ORDER BY seq",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_all(pool)
    .await?)
}

/// Whether the document owes a rotation (`SRV-122`).
pub async fn rotation_owed(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<bool, AppError> {
    let owed: Option<bool> =
        sqlx::query_scalar("SELECT rotation_owed FROM documents WHERE id = $1")
            .bind(id_to_uuid(*document_id))
            .fetch_optional(pool)
            .await?;
    Ok(owed.unwrap_or(false))
}

/// Deletes one member row, refusing the last owner. The target row is locked
/// so two concurrent removals cannot both see two owners.
async fn delete_member(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> Result<(), AppError> {
    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(document_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;

    if current_role.as_deref() == Some("owner") {
        ensure_not_last_owner(tx, document_id).await?;
    }

    sqlx::query("DELETE FROM document_members WHERE document_id = $1 AND user_id = $2")
        .bind(document_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Updates the role of a document member.
///
/// Enforces the last-owner invariant: demoting the document's only owner is
/// refused with a 409 (see `remove_document_member`).
pub async fn update_document_member_role(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    user_id: &ambre_core::UserId,
    role: &str,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user_id);

    let mut tx = pool.begin().await?;

    let current_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM document_members WHERE document_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;

    if current_role.as_deref() == Some("owner") && role != "owner" {
        ensure_not_last_owner(&mut tx, did).await?;
    }

    sqlx::query("UPDATE document_members SET role = $3 WHERE document_id = $1 AND user_id = $2")
        .bind(did)
        .bind(uid)
        .bind(role)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
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
    link: &ambre_protocol::KeyLink,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let mut tx = pool.begin().await?;
    lock_document(&mut tx, did).await?;
    rewrap_members(&mut tx, did, id_to_uuid(*rotated_by), keys, link).await?;
    tx.commit().await?;
    Ok(())
}

/// Stores the rotation's link and writes each member's new wrapped copy. It
/// clears the document's debt (`SRV-124`, `SYNC-118`), inside the caller's
/// transaction and under its document lock.
async fn rewrap_members(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: uuid::Uuid,
    rotator: uuid::Uuid,
    keys: &[(ambre_core::UserId, Vec<u8>)],
    link: &ambre_protocol::KeyLink,
) -> Result<(), AppError> {
    append_link(tx, document_id, link).await?;
    // The rotator re-wrapped every key with ITS exchange key, so it becomes the
    // new wrapper of record. Update `invited_by` alongside `doc_key_enc`, or
    // the ECDH unwrap would later use the stale inviter's key.
    for (user_id, key_enc) in keys {
        let uid = id_to_uuid(*user_id);
        sqlx::query("UPDATE document_members SET doc_key_enc = $3, invited_by = $4 WHERE document_id = $1 AND user_id = $2")
            .bind(document_id)
            .bind(uid)
            .bind(key_enc)
            .bind(rotator)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("UPDATE documents SET rotation_owed = false WHERE id = $1")
        .bind(document_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
