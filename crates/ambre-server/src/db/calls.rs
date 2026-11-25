//! The one read of a call the database answers: the membership of its channel
//! (`ambre-spec-call-media.md` section 3). The state of a call lives in the ephemeral store
//! under a time to live, and no table holds a call (`SRV-250`).

use super::*;

/// Every channel `user` is a member of: a member row on its `meta` document (`SRV-047`).
/// One query for the whole set, so an ask of the live calls costs one read (`CALL-162`).
///
/// # Errors
/// A database error.
pub async fn member_channel_ids(
    pool: &PgPool,
    user: &Id<User>,
) -> Result<Vec<Id<Channel>>, AppError> {
    let rows: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT d.channel_id FROM channel_documents d \
         JOIN document_members m ON m.document_id = d.document_id \
         WHERE d.kind = 'meta' AND m.user_id = $1 ORDER BY d.channel_id",
    )
    .bind(id_to_uuid(*user))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(uuid_row_to_id).collect())
}

/// Whether `user` is a member of `channel`: a member row on its `meta` document (`SRV-047`).
///
/// # Errors
/// A database error.
pub async fn is_channel_member(
    pool: &PgPool,
    channel: &Id<Channel>,
    user: &Id<User>,
) -> Result<bool, AppError> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM channel_documents d \
         JOIN document_members m ON m.document_id = d.document_id \
         WHERE d.channel_id = $1 AND d.kind = 'meta' AND m.user_id = $2",
    )
    .bind(id_to_uuid(*channel))
    .bind(id_to_uuid(*user))
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}
