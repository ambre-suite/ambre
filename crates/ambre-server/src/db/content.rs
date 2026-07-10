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
/// The `document_id` filter is a mandatory object-level authorization guard:
/// callers authorize membership on a document and must only ever resolve
/// snapshots belonging to that same document (prevents a cross-document IDOR).
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

/// Deletes updates whose seq <= `before_seq` (called after compaction).
#[allow(dead_code)]
pub async fn purge_updates_before_seq(
    pool: &PgPool,
    document_id: &ambre_core::DocumentId,
    before_seq: i64,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    sqlx::query("DELETE FROM updates WHERE document_id = $1 AND seq <= $2")
        .bind(did)
        .bind(before_seq)
        .execute(pool)
        .await?;
    Ok(())
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
    // Idempotent like `insert_update`: the `id` PK is the content hash, so a
    // retried `confirm` (after the S3 PUT already succeeded) or two clients
    // confirming the same blob would otherwise raise a unique violation and
    // surface as a 500. The row is content-addressed, so a re-insert is a no-op.
    sqlx::query(
        "INSERT INTO media (id, document_id, uploaded_by, mime_type, size_bytes, s3_key) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"
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

/// Deletes a single snapshot row (cascades to updates via FK).
pub async fn delete_snapshot(pool: &PgPool, snapshot_id: uuid::Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(snapshot_id)
        .execute(pool)
        .await?;
    Ok(())
}
