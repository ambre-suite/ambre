// ambre-server/src/db/content.rs
// Snapshot / update / media rows and their queries (incl. compaction).

use super::*;
use crate::payload::Payload;

/// Row from `snapshots`.
#[derive(Debug)]
pub struct SnapshotRow {
    pub id: ambre_core::SnapshotId,
    #[allow(dead_code)]
    pub document_id: ambre_core::DocumentId,
    #[allow(dead_code)]
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub parent_hash: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub parent_snapshot_proof: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub update_clocks: serde_json::Value,
    pub s3_key: ambre_core::ObjectKey,
    /// Highest delta `seq` folded into this snapshot. Clients gate their
    /// post-open `RequestMissing` on this value to skip the replayed history.
    pub covered_seq: ambre_core::Seq,
    /// Who vouched for it (`SRV-126`).
    pub status: SnapshotStatus,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub use ambre_core::SnapshotStatus;

/// The stored spelling, which the migration checks (`SRV-180`).
fn parse_status(text: &str) -> Result<SnapshotStatus, AppError> {
    text.parse()
        .map_err(|error| AppError::Internal(format!("snapshot {error}")))
}

/// The columns [`snapshot_row`] reads, in one place for every query.
const SNAPSHOT_COLUMNS: &str = "id, document_id, author_pubkey, parent_hash, \
    parent_snapshot_proof, update_clocks, s3_key, covered_seq, status, created_at";

/// One `snapshots` row, read from [`SNAPSHOT_COLUMNS`].
fn snapshot_row(r: &sqlx::postgres::PgRow) -> Result<SnapshotRow, AppError> {
    let status: String = r.try_get("status")?;
    Ok(SnapshotRow {
        id: r.try_get::<PgId, _>("id")?.into(),
        document_id: r.try_get::<PgId, _>("document_id")?.into(),
        author_pubkey: r.try_get("author_pubkey")?,
        parent_hash: r.try_get("parent_hash")?,
        parent_snapshot_proof: r.try_get("parent_snapshot_proof")?,
        update_clocks: r.try_get("update_clocks")?,
        s3_key: ambre_core::ObjectKey(r.try_get("s3_key")?),
        covered_seq: decode_seq(r.try_get("covered_seq")?)?,
        status: parse_status(&status)?,
        created_at: r.try_get("created_at")?,
    })
}

/// Row from `updates` (encrypted deltas indexed by seq). No `Debug`: it may
/// hold a frame (`ZK-006`).
pub struct UpdateRow {
    #[allow(dead_code)]
    pub document_id: ambre_core::DocumentId,
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub snapshot_id: ambre_core::SnapshotId,
    pub clock: i64,
    pub seq: ambre_core::Seq,
    /// The object of its own, for a delta over the inline size (`SRV-149`).
    pub s3_key: Option<ambre_core::ObjectKey>,
    /// The sealed envelope itself, for a delta in the hot tail (`SRV-148`).
    pub wire: Option<Payload>,
    /// The batch object that holds the frame (`SRV-151`).
    pub batch_key: Option<ambre_core::ObjectKey>,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The bounds of a document's inline tail (`SRV-150`).
#[derive(Debug, Clone, Copy)]
pub struct InlineCap {
    pub deltas: i64,
    pub bytes: i64,
}

/// What an insert into the inline tail did.
#[derive(Debug, PartialEq, Eq)]
pub enum InlineInsert {
    /// The row holds the frame. The tail now holds `deltas` rows and `bytes`
    /// bytes, counting this one.
    Stored { deltas: i64, bytes: i64 },
    /// The tail is at its cap, and nothing was written.
    Full,
}

/// What a trim of the delta index removed.
#[derive(Debug, Default)]
pub struct Trim {
    /// The rows removed.
    pub rows: u64,
    /// The batch objects no row names any more, which the caller deletes.
    pub batch_keys: Vec<ambre_core::ObjectKey>,
}

/// Inserts snapshot metadata and returns the assigned `SnapshotId`.
#[allow(clippy::too_many_arguments)]
pub async fn create_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    parent_hash: Option<&[u8]>,
    parent_snapshot_proof: Option<&[u8]>,
    update_clocks: &serde_json::Value,
    s3_key: &ambre_core::ObjectKey,
    covered_seq: ambre_core::Seq,
) -> Result<ambre_core::SnapshotId, AppError> {
    let did = id_to_uuid(*document_id);
    let snapshot_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO snapshots (id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(snapshot_id)
    .bind(did)
    .bind(author_pubkey)
    .bind(parent_hash)
    .bind(parent_snapshot_proof)
    .bind(update_clocks)
    .bind(s3_key.as_str())
    .bind(covered_seq.to_i64_saturating())
    .execute(pool)
    .await?;

    let core_id = ambre_core::Uuid::from_bytes(*snapshot_id.as_bytes());
    Ok(ambre_core::SnapshotId::from_uuid(core_id))
}

/// Returns the snapshot an open is served: the most recent one somebody
/// confirmed, its author included (`SRV-128`). A pending one is never served.
pub async fn get_latest_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<SnapshotRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(&format!(
        "SELECT {SNAPSHOT_COLUMNS} FROM snapshots WHERE document_id = $1 \
         AND status <> 'pending' ORDER BY created_at DESC LIMIT 1"
    ))
    .bind(did)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(snapshot_row).transpose()
}

/// Returns the reference snapshot: the most recent one a member other than
/// its author confirmed (`SRV-131`).
pub async fn get_reference_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<SnapshotRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(&format!(
        "SELECT {SNAPSHOT_COLUMNS} FROM snapshots WHERE document_id = $1 \
         AND status = 'confirmed' ORDER BY created_at DESC LIMIT 1"
    ))
    .bind(did)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(snapshot_row).transpose()
}

/// Records that `confirmer` vouched for a snapshot, and returns its standing
/// after it, `None` when the document holds no such snapshot (`SRV-126`).
///
/// A member other than the author makes it `confirmed`. The author makes a
/// pending one `self`, unless a snapshot another author confirmed alone
/// already stands past the reference. The trim past the reference then drops
/// one author's deltas only, the ones that author's snapshot holds.
///
/// A snapshot of a locked segment is the final one, and only another member
/// confirms it (`SRV-118`). Its author's confirmation leaves it pending.
pub async fn confirm_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    snapshot_id: &ambre_core::SnapshotId,
    confirmer: &[u8],
) -> Result<Option<SnapshotStatus>, AppError> {
    let did = id_to_uuid(*document_id);
    let sid = id_to_uuid(*snapshot_id);
    sqlx::query(
        "UPDATE snapshots s
         SET status = CASE WHEN s.author_pubkey = $3 THEN 'self' ELSE 'confirmed' END
         WHERE s.id = $1 AND s.document_id = $2 AND (
             (s.author_pubkey <> $3 AND s.status <> 'confirmed')
             OR (s.author_pubkey = $3 AND s.status = 'pending' AND NOT EXISTS (
                 SELECT 1 FROM channel_documents c
                 WHERE c.document_id = s.document_id AND c.horizon IS NOT NULL
             ) AND NOT EXISTS (
                 SELECT 1 FROM snapshots o
                 WHERE o.document_id = s.document_id AND o.status = 'self'
                   AND o.author_pubkey <> s.author_pubkey
                   AND o.created_at > COALESCE(
                       (SELECT MAX(r.created_at) FROM snapshots r
                        WHERE r.document_id = s.document_id AND r.status = 'confirmed'),
                       '-infinity'::timestamptz))))",
    )
    .bind(sid)
    .bind(did)
    .bind(confirmer)
    .execute(pool)
    .await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM snapshots WHERE id = $1 AND document_id = $2")
            .bind(sid)
            .bind(did)
            .fetch_optional(pool)
            .await?;
    status.as_deref().map(parse_status).transpose()
}

/// A snapshot a report dropped.
#[derive(Debug, PartialEq, Eq)]
pub struct DroppedSnapshot {
    /// Its object key, for the caller to delete.
    pub s3_key: ambre_core::ObjectKey,
    /// Whether it was the final snapshot of a segment, which is then to be
    /// finalized again.
    pub refinalize: bool,
}

/// Drops a snapshot nobody confirmed yet (`SRV-133`). A confirmed one stays,
/// and so does every delta (`SRV-135`).
///
/// A final snapshot takes its finalization with it, in the same transaction.
/// The segment stays `final` and locked, and a new lease finalizes it again
/// at the same horizon (the Murmure segments spec).
pub async fn drop_pending_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    snapshot_id: &ambre_core::SnapshotId,
) -> Result<Option<DroppedSnapshot>, AppError> {
    let (sid, did) = (id_to_uuid(*snapshot_id), id_to_uuid(*document_id));
    let mut tx = pool.begin().await?;
    let key: Option<String> = sqlx::query_scalar(
        "DELETE FROM snapshots WHERE id = $1 AND document_id = $2 AND status = 'pending' \
         RETURNING s3_key",
    )
    .bind(sid)
    .bind(did)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(s3_key) = key else {
        return Ok(None);
    };
    let refinalize = sqlx::query(
        "UPDATE channel_documents SET final_snapshot = NULL, finalized_at = NULL \
         WHERE document_id = $1 AND final_snapshot = $2",
    )
    .bind(did)
    .bind(sid)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    tx.commit().await?;
    Ok(Some(DroppedSnapshot {
        s3_key: ambre_core::ObjectKey(s3_key),
        refinalize,
    }))
}

/// The highest delta a served snapshot of the document folds in, 0 with none.
/// What lies past it is the history an open replays (`SRV-038`).
pub async fn served_horizon(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<i64, AppError> {
    let horizon: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(covered_seq) FROM snapshots WHERE document_id = $1 AND status <> 'pending'",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_one(pool)
    .await?;
    Ok(horizon.unwrap_or(0))
}

/// Fetches a specific snapshot by id, scoped to its owning document.
///
/// The `document_id` filter is a mandatory object-level authorization guard.
/// Callers authorize membership on a document, and must only ever resolve
/// snapshots belonging to that same document (this prevents a cross-document
/// IDOR).
pub async fn get_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    snapshot_id: &ambre_core::SnapshotId,
) -> Result<Option<SnapshotRow>, AppError> {
    let sid = id_to_uuid(*snapshot_id);
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(&format!(
        "SELECT {SNAPSHOT_COLUMNS} FROM snapshots WHERE id = $1 AND document_id = $2"
    ))
    .bind(sid)
    .bind(did)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(snapshot_row).transpose()
}

/// Inserts an update metadata row (idempotent on conflict).
pub async fn insert_update(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    snapshot_id: &ambre_core::SnapshotId,
    clock: i64,
    seq: i64,
    s3_key: &ambre_core::ObjectKey,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let sid = id_to_uuid(*snapshot_id);
    sqlx::query(
        "INSERT INTO updates (document_id, author_pubkey, snapshot_id, clock, seq, s3_key) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (document_id, author_pubkey, clock) DO NOTHING"
    )
    .bind(did)
    .bind(author_pubkey)
    .bind(sid)
    .bind(clock)
    .bind(seq)
    .bind(s3_key.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// Inserts a delta with its sealed envelope in the row, unless the document's
/// inline tail is at `cap` (`SRV-148`, `SRV-150`). The count and the insert
/// are one statement. Two writers racing at the cap may each see room, so the
/// tail can pass it by the writers in flight at once.
#[allow(clippy::too_many_arguments)]
pub async fn insert_inline_update(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    snapshot_id: &ambre_core::SnapshotId,
    clock: i64,
    seq: i64,
    wire: &Payload,
    cap: InlineCap,
) -> Result<InlineInsert, AppError> {
    let (deltas, bytes, room): (i64, i64, bool) = sqlx::query_as(
        "WITH tail AS ( \
             SELECT COUNT(*) AS n, COALESCE(SUM(octet_length(wire)), 0)::BIGINT AS bytes \
             FROM updates WHERE document_id = $1 AND wire IS NOT NULL \
         ), ins AS ( \
             INSERT INTO updates (document_id, author_pubkey, snapshot_id, clock, seq, wire) \
             SELECT $1, $2, $3, $4, $5, $6 FROM tail \
             WHERE tail.n < $7 AND tail.bytes + octet_length($6) <= $8 \
             ON CONFLICT (document_id, author_pubkey, clock) DO NOTHING \
             RETURNING 1 \
         ) \
         SELECT n, bytes, n < $7 AND bytes + octet_length($6) <= $8 FROM tail",
    )
    .bind(id_to_uuid(*document_id))
    .bind(author_pubkey)
    .bind(id_to_uuid(*snapshot_id))
    .bind(clock)
    .bind(seq)
    .bind(wire)
    .bind(cap.deltas)
    .bind(cap.bytes)
    .fetch_one(pool)
    .await?;
    if !room {
        return Ok(InlineInsert::Full);
    }
    let size = i64::try_from(wire.len()).unwrap_or(i64::MAX);
    Ok(InlineInsert::Stored {
        deltas: deltas + 1,
        bytes: bytes.saturating_add(size),
    })
}

/// Puts the sealed envelope of delta `seq` in its index row, the fallback when
/// its object can never be queued (`SRV-034`). The row drops the object key it
/// pointed at, since a delta has one home. A row already holding its frame or
/// pointing at a batch is left as it is. Returns whether a row took it.
pub async fn attach_wire(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    seq: i64,
    wire: &Payload,
) -> Result<bool, AppError> {
    let attached = sqlx::query(
        "UPDATE updates SET wire = $3, s3_key = NULL \
         WHERE document_id = $1 AND seq = $2 AND wire IS NULL AND batch_key IS NULL",
    )
    .bind(id_to_uuid(*document_id))
    .bind(seq)
    .bind(wire)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(attached == 1)
}

/// Locks the inline tail of a document, oldest first, for a move into a batch
/// object. `None` when another move holds it.
pub async fn lock_inline_tail(
    tx: &mut sqlx::PgConnection,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<Vec<(i64, Payload)>>, AppError> {
    let rows = sqlx::query_as(
        "SELECT seq, wire FROM updates WHERE document_id = $1 AND wire IS NOT NULL \
         ORDER BY seq FOR UPDATE NOWAIT",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_all(tx)
    .await;
    match rows {
        Ok(rows) => Ok(Some(rows)),
        // `lock_not_available`: a move already runs.
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("55P03") => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Points the rows `seqs` at the batch object `batch_key`, and drops their
/// inline frames. The caller holds their locks.
pub async fn point_rows_at_batch(
    tx: &mut sqlx::PgConnection,
    document_id: &ambre_core::DocumentId,
    seqs: &[i64],
    batch_key: &ambre_core::ObjectKey,
) -> Result<u64, AppError> {
    let moved = sqlx::query(
        "UPDATE updates SET wire = NULL, batch_key = $3 \
         WHERE document_id = $1 AND seq = ANY($2) AND wire IS NOT NULL",
    )
    .bind(id_to_uuid(*document_id))
    .bind(seqs)
    .bind(batch_key.as_str())
    .execute(tx)
    .await?
    .rows_affected();
    Ok(moved)
}

/// The documents with an inline tail and no write for `idle_secs`, at most
/// `limit` (`SRV-152`).
pub async fn idle_inline_documents(
    pool: &PgPool,
    idle_secs: i64,
    limit: i64,
) -> Result<Vec<ambre_core::DocumentId>, AppError> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT u.document_id FROM updates u WHERE u.wire IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM updates v WHERE v.document_id = u.document_id \
                         AND v.created_at > now() - make_interval(secs => $1::BIGINT::FLOAT8)) \
         LIMIT $2",
    )
    .bind(idle_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().map(uuid_row_to_id).collect())
}

/// Returns updates with seq > `after_seq` for delta-sync after reconnect.
pub async fn get_updates_after(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    after_seq: ambre_core::Seq,
    limit: i64,
) -> Result<Vec<UpdateRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT document_id, author_pubkey, snapshot_id, clock, seq, s3_key, wire, batch_key, created_at FROM updates WHERE document_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
    )
    .bind(did)
    .bind(after_seq.to_i64_saturating())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UpdateRow {
                document_id: r.try_get::<PgId, _>("document_id")?.into(),
                author_pubkey: r.try_get("author_pubkey")?,
                snapshot_id: r.try_get::<PgId, _>("snapshot_id")?.into(),
                clock: r.try_get("clock")?,
                seq: decode_seq(r.try_get("seq")?)?,
                s3_key: r
                    .try_get::<Option<String>, _>("s3_key")?
                    .map(ambre_core::ObjectKey),
                wire: r.try_get("wire")?,
                batch_key: r
                    .try_get::<Option<String>, _>("batch_key")?
                    .map(ambre_core::ObjectKey),
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

/// The durable clock/seq state for a document, read from the `updates` index.
/// Used to rehydrate Redis (the clock-validation authority) when it is cold --
/// for example after a Redis restart while Postgres kept the delta history. Without this
/// the per-author clock counter resets to 0 while clients still hold advanced
/// clocks, and every subsequent update fails `CLOCK_MISMATCH`.
#[derive(Debug, Default)]
pub struct ClockState {
    /// Highest `seq` ever assigned for the document (0 when it has no updates).
    pub max_seq: i64,
    /// Per-author `(author_pubkey, max_clock)` high-water marks.
    pub author_clocks: Vec<(Vec<u8>, i64)>,
}

/// Reads the durable clock/seq high-water marks for a document from the `updates`
/// index. A cold Redis can then be reseeded to a state consistent with the
/// history clients have already replayed (see [`ClockState`]).
pub async fn get_clock_state(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<ClockState, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT author_pubkey, MAX(clock) AS max_clock, MAX(seq) AS max_seq
         FROM updates WHERE document_id = $1 GROUP BY author_pubkey",
    )
    .bind(did)
    .fetch_all(pool)
    .await?;

    let mut state = ClockState::default();
    for r in &rows {
        let author: Vec<u8> = r.try_get("author_pubkey")?;
        let max_clock: i64 = r.try_get("max_clock")?;
        let max_seq: i64 = r.try_get("max_seq")?;
        state.max_seq = state.max_seq.max(max_seq);
        state.author_clocks.push((author, max_clock));
    }
    Ok(state)
}

/// Trims the delta index of every `updates` row that all retained snapshots have
/// already folded in, returning the number of rows deleted.
///
/// The purge horizon per document is `MIN(covered_seq)` across its confirmed
/// snapshots. A client restores from one of those snapshots, and only ever needs
/// updates past its `covered_seq`. An update with `seq <= MIN(covered_seq)` is
/// therefore redundant for every possible restore point. A pending or
/// self-confirmed snapshot moves nothing (`SRV-127`). Documents with no
/// confirmed snapshot yet are skipped (nothing covers their updates).
///
/// The latest self-confirmed snapshot past the reference also drops its
/// author's own deltas between the two (`SRV-130`). An open checks that
/// snapshot against the deltas of every other user, which stay.
///
/// A final segment whose final snapshot another member confirmed drops every
/// delta that snapshot covers (`SRV-118`). No delta lies past it: the lock
/// refused them all.
///
/// This is the delta index's only garbage collector for a long-lived document. The
/// `updates.snapshot_id -> snapshots(id)` FK was dropped (see the
/// `drop_updates_snapshot_fk` migration), so deleting an old snapshot no longer
/// cascades its deltas away. Without this pass the table would grow without bound.
/// Run once per compaction cycle, independent of snapshot deletion.
///
/// A batch object goes only once no row names it, so a batch that still holds
/// an uncovered delta stays (`SRV-154`).
pub async fn purge_superseded_updates(pool: &PgPool) -> Result<Trim, AppError> {
    let (rows, touched): (i64, Vec<String>) = sqlx::query_as(
        "WITH horizon AS (
            SELECT document_id, MIN(covered_seq) AS min_covered
            FROM snapshots
            WHERE status = 'confirmed'
            GROUP BY document_id
        ), reference AS (
            SELECT DISTINCT ON (document_id) document_id, covered_seq, created_at
            FROM snapshots
            WHERE status = 'confirmed'
            ORDER BY document_id, created_at DESC
        ), author_span AS (
            SELECT DISTINCT ON (s.document_id) s.document_id, s.author_pubkey,
                   COALESCE(r.covered_seq, 0) AS from_seq, s.covered_seq AS to_seq
            FROM snapshots s
            LEFT JOIN reference r ON r.document_id = s.document_id
            WHERE s.status = 'self' AND (r.created_at IS NULL OR s.created_at > r.created_at)
            ORDER BY s.document_id, s.created_at DESC
        ), gone AS (
            DELETE FROM updates u
            WHERE EXISTS (
                SELECT 1 FROM horizon h
                WHERE h.document_id = u.document_id AND u.seq <= h.min_covered
            ) OR EXISTS (
                SELECT 1 FROM author_span a
                WHERE a.document_id = u.document_id AND a.author_pubkey = u.author_pubkey
                  AND u.seq > a.from_seq AND u.seq <= a.to_seq
            ) OR EXISTS (
                SELECT 1 FROM channel_documents c
                JOIN snapshots f ON f.id = c.final_snapshot AND f.document_id = c.document_id
                WHERE c.document_id = u.document_id AND c.state = 'final'
                  AND f.status = 'confirmed' AND u.seq <= f.covered_seq
            )
            RETURNING u.batch_key
        )
        SELECT (SELECT COUNT(*) FROM gone),
               ARRAY(SELECT DISTINCT batch_key FROM gone WHERE batch_key IS NOT NULL)",
    )
    .fetch_one(pool)
    .await?;
    // The statement above reads the rows as they were before its delete, so
    // whether a batch is still named is a second question.
    let batch_keys: Vec<String> = sqlx::query_scalar(
        "SELECT k FROM unnest($1::TEXT[]) AS k \
         WHERE NOT EXISTS (SELECT 1 FROM updates WHERE batch_key = k)",
    )
    .bind(&touched)
    .fetch_all(pool)
    .await?;
    Ok(Trim {
        rows: u64::try_from(rows).unwrap_or(0),
        batch_keys: batch_keys.into_iter().map(ambre_core::ObjectKey).collect(),
    })
}

/// Confirms a completed S3 upload by inserting a `media` row.
pub async fn insert_media(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    uploaded_by: &ambre_core::UserId,
    content_hash: &ambre_core::ContentHash,
    size_bytes: ambre_core::ByteSize,
    s3_key: &ambre_core::ObjectKey,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*uploaded_by);
    // Idempotent like `insert_update`. The key is the document and the content
    // hash together. A retried `confirm` would otherwise raise a unique
    // violation and surface as a 500. So would two clients confirming the same
    // blob. The row is content-addressed, so a re-insert is a no-op, and the
    // same file in another document gets a row of its own (`SRV-058`). Each
    // uploader adds a claim of its own beside the row (`SRV-138`).
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO media (id, document_id, uploaded_by, size_bytes, s3_key) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (document_id, id) DO NOTHING"
    )
    .bind(content_hash.as_str())
    .bind(did)
    .bind(uid)
    .bind(i64::try_from(size_bytes.0).unwrap_or(i64::MAX))
    .bind(s3_key.as_str())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO media_uploaders (document_id, id, user_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(did)
    .bind(content_hash.as_str())
    .bind(uid)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// What an erase found, once the caller's claim is gone.
#[derive(Debug, PartialEq, Eq)]
pub enum MediaErase {
    /// The caller never uploaded this blob to this document (`SRV-140`).
    NotUploader,
    /// Another uploader still holds the blob, so the object stays.
    Kept,
    /// The last claim went: the object under this key must go with the row.
    Last { s3_key: ambre_core::ObjectKey },
}

/// Erases `user`'s claim on a blob of `document_id`, inside `tx`.
///
/// The media row is locked first, so a confirmation racing the erase waits
/// for it. The caller commits once the object is gone, so a failed object
/// delete leaves the row and the claim for a retry (`SRV-139`).
pub async fn erase_media_claim(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: &ambre_core::DocumentId,
    user: &ambre_core::UserId,
    content_hash: &ambre_core::ContentHash,
) -> Result<MediaErase, AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*user);
    let s3_key: Option<String> = sqlx::query_scalar(
        "SELECT s3_key FROM media WHERE document_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(did)
    .bind(content_hash.as_str())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(s3_key) = s3_key else {
        return Ok(MediaErase::NotUploader);
    };
    let claimed = sqlx::query(
        "DELETE FROM media_uploaders WHERE document_id = $1 AND id = $2 AND user_id = $3",
    )
    .bind(did)
    .bind(content_hash.as_str())
    .bind(uid)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Ok(MediaErase::NotUploader);
    }
    let others: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM media_uploaders WHERE document_id = $1 AND id = $2)",
    )
    .bind(did)
    .bind(content_hash.as_str())
    .fetch_one(&mut **tx)
    .await?;
    if others {
        return Ok(MediaErase::Kept);
    }
    sqlx::query("DELETE FROM media WHERE document_id = $1 AND id = $2")
        .bind(did)
        .bind(content_hash.as_str())
        .execute(&mut **tx)
        .await?;
    Ok(MediaErase::Last {
        s3_key: ambre_core::ObjectKey(s3_key),
    })
}

/// Whether the media index records `content_hash` in `document_id`.
pub async fn document_holds_media(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    content_hash: &ambre_core::ContentHash,
) -> Result<bool, AppError> {
    let did = id_to_uuid(*document_id);
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM media WHERE document_id = $1 AND id = $2)",
    )
    .bind(did)
    .bind(content_hash.as_str())
    .fetch_one(pool)
    .await?;
    Ok(held)
}

/// Snapshot compacted to be deleted (id + S3 key for cleanup).
#[derive(Debug, sqlx::FromRow)]
pub struct CompactSnapshot {
    #[sqlx(try_from = "PgId")]
    pub id: ambre_core::SnapshotId,
    #[sqlx(try_from = "String")]
    pub s3_key: ambre_core::ObjectKey,
}

/// Finds snapshots beyond the keep limit (oldest N) per document.
///
/// Returns a list of snapshot ids and their S3 keys that should be
/// deleted. Keeps at most `keep` snapshots per document (most recent
/// by created_at DESC). The reference snapshot and the latest self-confirmed
/// one stay whatever their rank, since an open checks the one against the
/// other (`SRV-131`).
pub async fn find_compactable_snapshots(
    pool: &PgPool,
    keep: i64,
) -> Result<Vec<CompactSnapshot>, AppError> {
    let rows = sqlx::query_as::<_, CompactSnapshot>(
        "WITH ranked AS (
            SELECT id, s3_key,
                   ROW_NUMBER() OVER (PARTITION BY document_id ORDER BY created_at DESC) AS rn
            FROM snapshots
        ), held AS (
            (SELECT DISTINCT ON (document_id) id FROM snapshots
             WHERE status = 'confirmed' ORDER BY document_id, created_at DESC)
            UNION
            (SELECT DISTINCT ON (document_id) id FROM snapshots
             WHERE status = 'self' ORDER BY document_id, created_at DESC)
        )
        SELECT id, s3_key FROM ranked WHERE rn > $1 AND id NOT IN (SELECT id FROM held)",
    )
    .bind(keep)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Deletes a single snapshot row.
///
/// This does NOT touch `updates`. The `updates.snapshot_id` FK was dropped (see
/// the `drop_updates_snapshot_fk` migration), so `snapshot_id` is now an opaque
/// client-side correlation tag with no cascade. Superseded deltas are trimmed
/// separately by [`purge_superseded_updates`]; the caller removes the S3 blob.
pub async fn delete_snapshot(
    pool: &PgPool,
    snapshot_id: &ambre_core::SnapshotId,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(id_to_uuid(*snapshot_id))
        .execute(pool)
        .await?;
    Ok(())
}
