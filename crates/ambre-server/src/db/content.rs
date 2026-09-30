// ambre-server/src/db/content.rs
// Snapshot / update / media rows and their queries (incl. compaction).

use super::*;

/// Row from `snapshots`.
#[derive(Debug)]
pub struct SnapshotRow {
    pub id: uuid::Uuid,
    #[allow(dead_code)]
    pub document_id: uuid::Uuid,
    #[allow(dead_code)]
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub parent_hash: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub parent_snapshot_proof: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub update_clocks: serde_json::Value,
    pub s3_key: String,
    /// Highest delta `seq` folded into this snapshot. Clients gate their
    /// post-open `RequestMissing` on this value to skip the replayed history.
    pub covered_seq: i64,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Row from `updates` (encrypted deltas indexed by seq).
#[derive(Debug)]
pub struct UpdateRow {
    #[allow(dead_code)]
    pub id: uuid::Uuid,
    #[allow(dead_code)]
    pub document_id: uuid::Uuid,
    pub author_pubkey: Vec<u8>,
    #[allow(dead_code)]
    pub snapshot_id: uuid::Uuid,
    pub clock: i64,
    pub seq: i64,
    pub s3_key: String,
    #[allow(dead_code)]
    pub created_at: chrono::DateTime<chrono::Utc>,
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
    s3_key: &str,
    covered_seq: i64,
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
    .bind(s3_key)
    .bind(covered_seq)
    .execute(pool)
    .await?;

    let core_id = ambre_core::Uuid::from_bytes(*snapshot_id.as_bytes());
    Ok(ambre_core::SnapshotId::from_uuid(core_id))
}

/// Returns the most recent snapshot for a document (if any).
pub async fn get_latest_snapshot(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
) -> Result<Option<SnapshotRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq, created_at FROM snapshots WHERE document_id = $1 ORDER BY created_at DESC LIMIT 1"
    )
    .bind(did)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(SnapshotRow {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            author_pubkey: r.try_get("author_pubkey")?,
            parent_hash: r.try_get("parent_hash")?,
            parent_snapshot_proof: r.try_get("parent_snapshot_proof")?,
            update_clocks: r.try_get("update_clocks")?,
            s3_key: r.try_get("s3_key")?,
            covered_seq: r.try_get("covered_seq")?,
            created_at: r.try_get("created_at")?,
        })),
        None => Ok(None),
    }
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
    let row = sqlx::query(
        "SELECT id, document_id, author_pubkey, parent_hash, parent_snapshot_proof, update_clocks, s3_key, covered_seq, created_at FROM snapshots WHERE id = $1 AND document_id = $2"
    )
    .bind(sid)
    .bind(did)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(SnapshotRow {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            author_pubkey: r.try_get("author_pubkey")?,
            parent_hash: r.try_get("parent_hash")?,
            parent_snapshot_proof: r.try_get("parent_snapshot_proof")?,
            update_clocks: r.try_get("update_clocks")?,
            s3_key: r.try_get("s3_key")?,
            covered_seq: r.try_get("covered_seq")?,
            created_at: r.try_get("created_at")?,
        })),
        None => Ok(None),
    }
}

/// Inserts an update metadata row (idempotent on conflict).
pub async fn insert_update(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    author_pubkey: &[u8],
    snapshot_id: &ambre_core::SnapshotId,
    clock: i64,
    seq: i64,
    s3_key: &str,
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
    .bind(s3_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns updates with seq > `after_seq` for delta-sync after reconnect.
pub async fn get_updates_after(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<UpdateRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT id, document_id, author_pubkey, snapshot_id, clock, seq, s3_key, created_at FROM updates WHERE document_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
    )
    .bind(did)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UpdateRow {
                id: r.try_get("id")?,
                document_id: r.try_get("document_id")?,
                author_pubkey: r.try_get("author_pubkey")?,
                snapshot_id: r.try_get("snapshot_id")?,
                clock: r.try_get("clock")?,
                seq: r.try_get("seq")?,
                s3_key: r.try_get("s3_key")?,
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
/// The purge horizon per document is `MIN(covered_seq)` across its remaining
/// snapshots. A client restores from one of those snapshots, and only ever needs
/// updates past its `covered_seq`. An update with `seq <= MIN(covered_seq)` is
/// therefore redundant for every possible restore point. Documents with no
/// snapshot yet are skipped (nothing covers their updates).
///
/// This is the delta index's only garbage collector for a long-lived document. The
/// `updates.snapshot_id -> snapshots(id)` FK was dropped (see the
/// `drop_updates_snapshot_fk` migration), so deleting an old snapshot no longer
/// cascades its deltas away. Without this pass the table would grow without bound.
/// Run once per compaction cycle, independent of snapshot deletion.
pub async fn purge_superseded_updates(pool: &PgPool) -> Result<u64, AppError> {
    let deleted = sqlx::query(
        "WITH horizon AS (
            SELECT document_id, MIN(covered_seq) AS min_covered
            FROM snapshots
            GROUP BY document_id
        )
        DELETE FROM updates u
        USING horizon h
        WHERE u.document_id = h.document_id AND u.seq <= h.min_covered",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
}

/// Confirms a completed S3 upload by inserting a `media` row.
pub async fn insert_media(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    uploaded_by: &ambre_core::UserId,
    content_hash: &str,
    mime_type: &str,
    size_bytes: i64,
    s3_key: &str,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let uid = id_to_uuid(*uploaded_by);
    // Idempotent like `insert_update`. The key is the document and the content
    // hash together. A retried `confirm` would otherwise raise a unique
    // violation and surface as a 500. So would two clients confirming the same
    // blob. The row is content-addressed, so a re-insert is a no-op, and the
    // same file in another document gets a row of its own (`SRV-058`).
    sqlx::query(
        "INSERT INTO media (id, document_id, uploaded_by, mime_type, size_bytes, s3_key) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (document_id, id) DO NOTHING"
    )
    .bind(content_hash)
    .bind(did)
    .bind(uid)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(s3_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Whether the media index records `content_hash` in `document_id`.
pub async fn document_holds_media(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    content_hash: &str,
) -> Result<bool, AppError> {
    let did = id_to_uuid(*document_id);
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM media WHERE document_id = $1 AND id = $2)",
    )
    .bind(did)
    .bind(content_hash)
    .fetch_one(pool)
    .await?;
    Ok(held)
}

/// Snapshot compacted to be deleted (id + S3 key for cleanup).
#[derive(Debug, sqlx::FromRow)]
pub struct CompactSnapshot {
    pub id: uuid::Uuid,
    pub s3_key: String,
}

/// Finds snapshots beyond the keep limit (oldest N) per document.
///
/// Returns a list of snapshot ids and their S3 keys that should be
/// deleted. Keeps at most `keep` snapshots per document (most recent
/// by created_at DESC).
pub async fn find_compactable_snapshots(
    pool: &PgPool,
    keep: i64,
) -> Result<Vec<CompactSnapshot>, AppError> {
    let rows = sqlx::query_as::<_, CompactSnapshot>(
        "WITH ranked AS (
            SELECT id, s3_key,
                   ROW_NUMBER() OVER (PARTITION BY document_id ORDER BY created_at DESC) AS rn
            FROM snapshots
        )
        SELECT id, s3_key FROM ranked WHERE rn > $1",
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
pub async fn delete_snapshot(pool: &PgPool, snapshot_id: uuid::Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(snapshot_id)
        .execute(pool)
        .await?;
    Ok(())
}
