// ambre-server/src/db/read_cursors.rs
// The read cursors: one number per member and document, the only thing the
// activity answer compares (`ambre-spec-server.md section 3`, `SRV-109`).

use super::*;

/// One channel document the caller reads, with what the register knows of it.
#[derive(Debug)]
pub struct ActivityRow {
    pub channel_id: uuid::Uuid,
    pub document_id: uuid::Uuid,
    /// The caller's cursor on the document, 0 when it never wrote one.
    pub cursor: i64,
    /// The highest sequence number the register holds for the document: its
    /// flushed deltas and the horizon of its snapshots. The ephemeral store
    /// may hold a higher one not yet flushed.
    pub durable_last: i64,
}

/// Move the caller's cursor on a document to `seq`, forward or back.
///
/// Back is legal: marking a message unread moves the cursor before it.
pub async fn set_read_cursor(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
    document_id: &ambre_core::DocumentId,
    seq: i64,
) -> Result<(), AppError> {
    set_read_cursors(pool, user_id, &[(id_to_uuid(*document_id), seq)]).await
}

/// Move several of the caller's cursors in one statement. Touches the
/// caller's rows alone (the Murmure unread spec).
pub async fn set_read_cursors(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
    cursors: &[(uuid::Uuid, i64)],
) -> Result<(), AppError> {
    let (documents, seqs): (Vec<uuid::Uuid>, Vec<i64>) = cursors.iter().copied().unzip();
    sqlx::query(
        "INSERT INTO read_cursors (user_id, document_id, seq) \
         SELECT $1, d, s FROM UNNEST($2::uuid[], $3::bigint[]) AS t(d, s) \
         ON CONFLICT (user_id, document_id) DO UPDATE SET seq = EXCLUDED.seq",
    )
    .bind(id_to_uuid(*user_id))
    .bind(&documents)
    .bind(&seqs)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every segment and thread of every channel the caller is a member of, with
/// the caller's cursor and the register's last number.
///
/// Membership is the member rows of each channel's `meta` (`SRV-103`). The
/// `meta` and amendments documents carry no message a member reads, so they
/// never make a channel active.
pub async fn channel_activity(
    pool: &PgPool,
    user_id: &ambre_core::UserId,
) -> Result<Vec<ActivityRow>, AppError> {
    let rows = sqlx::query(
        "SELECT d.channel_id, d.document_id, COALESCE(c.seq, 0) AS cursor, \
                GREATEST( \
                    COALESCE((SELECT MAX(u.seq) FROM updates u \
                              WHERE u.document_id = d.document_id), 0), \
                    COALESCE((SELECT MAX(s.covered_seq) FROM snapshots s \
                              WHERE s.document_id = d.document_id), 0) \
                ) AS durable_last \
         FROM channel_documents m \
         JOIN document_members dm ON dm.document_id = m.document_id AND dm.user_id = $1 \
         JOIN channel_documents d ON d.channel_id = m.channel_id \
                                 AND d.kind IN ('segment', 'thread') \
         LEFT JOIN read_cursors c ON c.document_id = d.document_id AND c.user_id = $1 \
         WHERE m.kind = 'meta' \
         ORDER BY d.channel_id, d.document_id",
    )
    .bind(id_to_uuid(*user_id))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(ActivityRow {
                channel_id: r.try_get("channel_id")?,
                document_id: r.try_get("document_id")?,
                cursor: r.try_get("cursor")?,
                durable_last: r.try_get("durable_last")?,
            })
        })
        .collect()
}

/// Delete the cursors a removal leaves behind (`SRV-137`): the member's
/// cursor on the document. A channel's `meta` holds the channel's member
/// rows, so a removal from it deletes the cursors of the whole channel.
pub(super) async fn delete_read_cursors_of(
    conn: &mut sqlx::PgConnection,
    document_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM read_cursors WHERE user_id = $2 AND ( \
             document_id = $1 OR document_id IN ( \
                 SELECT d.document_id FROM channel_documents m \
                 JOIN channel_documents d ON d.channel_id = m.channel_id \
                 WHERE m.document_id = $1 AND m.kind = 'meta'))",
    )
    .bind(document_id)
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}
