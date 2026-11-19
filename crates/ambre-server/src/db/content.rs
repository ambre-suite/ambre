// ambre-server/src/db/content.rs
// Snapshot / update / media rows and their queries (incl. compaction).

use super::*;
use crate::payload::StoredPayload;
use ambre_core::Count;

/// Row from `snapshots`.
#[derive(Debug)]
pub struct SnapshotRow {
    pub id: Id<Snapshot>,
    pub author_pubkey: FixedBytes<ambre_core::PublicKey>,
    pub s3_key: ambre_core::ObjectKey,
    /// Highest delta `seq` folded into this snapshot. Clients gate their
    /// post-open `RequestMissing` on this value to skip the replayed history.
    pub covered_seq: ambre_core::Seq<ambre_core::Relay>,
    /// Who vouched for it (`SRV-126`).
    pub status: SnapshotStatus,
    /// The confirmation another member signed, `None` until one does
    /// (`SRV-284`).
    pub confirmation: Option<SignedConfirmation>,
}

pub use ambre_protocol::snapshot_confirm::SignedConfirmation;

pub use ambre_core::SnapshotStatus;

/// The stored spelling, which the migration checks (`SRV-180`).
fn parse_status(text: &str) -> Result<SnapshotStatus, AppError> {
    text.parse()
        .map_err(|error| AppError::Internal(format!("snapshot {error}")))
}

/// The columns [`snapshot_row`] reads, in one place for every query.
const SNAPSHOT_COLUMNS: &str =
    "id, author_pubkey, s3_key, covered_seq, status, confirmer_pubkey, confirm_signature";

/// One `snapshots` row, read from [`SNAPSHOT_COLUMNS`].
fn snapshot_row(r: &sqlx::postgres::PgRow) -> Result<SnapshotRow, AppError> {
    let status: String = r.try_get("status")?;
    Ok(SnapshotRow {
        id: r.try_get::<PgId, _>("id")?.into(),
        author_pubkey: decode_key(r.try_get("author_pubkey")?)?,
        s3_key: ambre_core::ObjectKey::new(r.try_get("s3_key")?),
        covered_seq: decode_seq(r.try_get("covered_seq")?)?,
        status: parse_status(&status)?,
        confirmation: {
            // The migration sets both columns or neither, so one without the
            // other reads as a corrupt row.
            let confirmer: Option<Vec<u8>> = r.try_get("confirmer_pubkey")?;
            let signature: Option<Vec<u8>> = r.try_get("confirm_signature")?;
            match (confirmer, signature) {
                (None, None) => None,
                (Some(confirmer), Some(signature)) => Some(SignedConfirmation {
                    confirmer: decode_key(confirmer)?,
                    signature: FixedBytes::from_slice(&signature)
                        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                }),
                _ => {
                    return Err(AppError::Internal(
                        "a snapshot confirmation half stored".to_owned(),
                    ));
                }
            }
        },
    })
}

/// Row from `updates` (encrypted deltas indexed by seq). No `Debug`: it may
/// hold a frame (`ZK-006`).
pub struct UpdateRow {
    pub author_pubkey: FixedBytes<ambre_core::PublicKey>,
    pub clock: ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
    pub seq: ambre_core::Seq<ambre_core::Relay>,
    /// The object of its own, for a delta over the inline size (`SRV-149`).
    pub s3_key: Option<ambre_core::ObjectKey>,
    /// The sealed envelope itself, for a delta in the hot tail (`SRV-148`).
    pub wire: Option<StoredPayload>,
    /// The batch object that holds the frame (`SRV-151`).
    pub batch_key: Option<ambre_core::ObjectKey>,
}

/// How many snapshots of one document a compaction keeps.
pub enum Snapshots {}

impl ambre_core::NarrowCount for Snapshots {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SnapshotCount");
}

pub use ambre_core::Deltas;
use ambre_core::{Sec, StoredAsI64, TimeSpan};

/// The bounds of a document's inline tail (`SRV-150`).
#[derive(Debug, Clone, Copy)]
pub struct InlineCap {
    pub deltas: Count<Deltas>,
    pub bytes: ambre_core::Count<ambre_core::Octets>,
}

impl InlineCap {
    /// Three quarters of the cap in both bounds, rounded up: a tail at or
    /// past it holds at least three quarters of a bound.
    #[must_use]
    pub const fn three_quarters(self) -> Self {
        Self {
            deltas: Count::<Deltas>::new(self.deltas.get().saturating_mul(3).div_ceil(4)),
            bytes: ambre_core::Count::<ambre_core::Octets>::new(
                self.bytes.get().saturating_mul(3).div_ceil(4),
            ),
        }
    }

    /// A quarter of the cap in both bounds, rounded down: a tail at or under
    /// it holds at most a quarter of either bound.
    #[must_use]
    pub const fn quarter(self) -> Self {
        Self {
            deltas: Count::<Deltas>::new(self.deltas.get() / 4),
            bytes: ambre_core::Count::<ambre_core::Octets>::new(self.bytes.get() / 4),
        }
    }
}

/// What an insert into the inline tail did.
#[derive(Debug, PartialEq, Eq)]
pub enum InlineInsert {
    /// The row holds the frame. The tail now holds `deltas` rows and `bytes`
    /// bytes, counting this one.
    Stored {
        deltas: Count<Deltas>,
        bytes: ambre_core::Count<ambre_core::Octets>,
    },
    /// The tail is at its cap, and nothing was written.
    Full,
}

/// What a trim of the delta index removed.
#[derive(Debug, Default)]
pub struct Trim {
    /// The rows removed, one delta each.
    pub rows: ambre_core::Count<ambre_core::Deltas>,
    /// The batch objects no row names any more, which the caller deletes.
    pub batch_keys: Vec<ambre_core::ObjectKey>,
}

/// Inserts snapshot metadata under `snapshot_id`, the id the relay minted for
/// its upload link (`SRV-247`), and returns it.
///
/// # Errors
///
/// [`AppError::Conflict`] when a row already holds that id: a second
/// confirmation of one upload.
pub async fn create_snapshot(
    pool: &PgPool,
    document_id: &Id<Document>,
    snapshot_id: Id<Snapshot>,
    author_pubkey: &FixedBytes<ambre_core::PublicKey>,
    s3_key: &ambre_core::ObjectKey,
    covered_seq: ambre_core::Seq<ambre_core::Relay>,
) -> Result<Id<Snapshot>, AppError> {
    let did = id_to_uuid(*document_id);
    let inserted = sqlx::query(
        "INSERT INTO snapshots (id, document_id, author_pubkey, s3_key, covered_seq) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING"
    )
    .bind(id_to_uuid(snapshot_id))
    .bind(did)
    .bind(author_pubkey.as_slice())
    .bind(s3_key.as_str())
    .bind(covered_seq.to_i64_saturating())
    .execute(pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(AppError::Conflict {
            detail: "snapshot already stored".into(),
        });
    }
    Ok(snapshot_id)
}

/// Returns the snapshot an open is served: the most recent one somebody
/// confirmed, its author included (`SRV-128`). A pending one is never served.
pub async fn get_latest_snapshot(
    pool: &PgPool,
    document_id: &Id<Document>,
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
    document_id: &Id<Document>,
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
///
/// Another member's confirmation keeps the signature it carries, which the
/// caller has verified (`SRV-284`). The author's keeps none: an open never
/// trusts a snapshot whole on its author's word.
pub async fn confirm_snapshot(
    pool: &PgPool,
    document_id: &Id<Document>,
    snapshot_id: &Id<Snapshot>,
    confirmation: &SignedConfirmation,
) -> Result<Option<SnapshotStatus>, AppError> {
    let did = id_to_uuid(*document_id);
    let sid = id_to_uuid(*snapshot_id);
    sqlx::query(
        "UPDATE snapshots s
         SET status = CASE WHEN s.author_pubkey = $3 THEN 'self' ELSE 'confirmed' END,
             confirmer_pubkey = CASE WHEN s.author_pubkey = $3 THEN NULL ELSE $3 END,
             confirm_signature = CASE WHEN s.author_pubkey = $3 THEN NULL ELSE $4 END
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
    .bind(confirmation.confirmer.as_slice())
    .bind(confirmation.signature.as_slice())
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
    /// [`ambre_core::AfterDrop::Refinalize`] when it was the final snapshot
    /// of a segment, which is then to be finalized again.
    pub after: ambre_core::AfterDrop,
}

/// Drops a snapshot nobody confirmed yet (`SRV-133`). A confirmed one stays,
/// and so does every delta (`SRV-135`).
///
/// A final snapshot takes its finalization with it, in the same transaction.
/// The segment stays `final` and locked, and a new lease finalizes it again
/// at the same horizon (the Murmure segments spec).
pub async fn drop_pending_snapshot(
    pool: &PgPool,
    document_id: &Id<Document>,
    snapshot_id: &Id<Snapshot>,
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
    let cleared = sqlx::query(
        "UPDATE channel_documents SET final_snapshot = NULL, finalized_at = NULL \
         WHERE document_id = $1 AND final_snapshot = $2",
    )
    .bind(did)
    .bind(sid)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(Some(DroppedSnapshot {
        s3_key: ambre_core::ObjectKey::new(s3_key),
        after: if cleared == 1 {
            ambre_core::AfterDrop::Refinalize
        } else {
            ambre_core::AfterDrop::Keep
        },
    }))
}

/// The highest delta a served snapshot of the document folds in, 0 with none.
/// What lies past it is the history an open replays (`SRV-038`).
pub async fn served_horizon(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<ambre_core::Seq<ambre_core::Relay>, AppError> {
    let horizon: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(covered_seq) FROM snapshots WHERE document_id = $1 AND status <> 'pending'",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_one(pool)
    .await?;
    Ok(horizon.map(decode_seq).transpose()?.unwrap_or_default())
}

/// Fetches a specific snapshot by id, scoped to its owning document.
///
/// The `document_id` filter is a mandatory object-level authorization guard.
/// Callers authorize membership on a document, and must only ever resolve
/// snapshots belonging to that same document (this prevents a cross-document
/// IDOR).
pub async fn get_snapshot(
    pool: &PgPool,
    document_id: &Id<Document>,
    snapshot_id: &Id<Snapshot>,
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
    document_id: &Id<Document>,
    author_pubkey: &FixedBytes<ambre_core::PublicKey>,
    snapshot_id: &Id<Snapshot>,
    clock: ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
    seq: ambre_core::Seq<ambre_core::Relay>,
    s3_key: &ambre_core::ObjectKey,
) -> Result<(), AppError> {
    let did = id_to_uuid(*document_id);
    let sid = id_to_uuid(*snapshot_id);
    sqlx::query(
        "INSERT INTO updates (document_id, author_pubkey, snapshot_id, clock, seq, s3_key) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (document_id, author_pubkey, clock) DO NOTHING"
    )
    .bind(did)
    .bind(author_pubkey.as_slice())
    .bind(sid)
    .bind(clock.to_i64_saturating())
    .bind(seq.to_i64_saturating())
    .bind(s3_key.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// The key columns of one `updates` row, which every insert of a delta
/// writes together.
pub struct NewUpdate<'a> {
    pub document_id: &'a Id<Document>,
    pub author_pubkey: &'a FixedBytes<ambre_core::PublicKey>,
    pub snapshot_id: &'a Id<Snapshot>,
    pub clock: ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
    pub seq: ambre_core::Seq<ambre_core::Relay>,
}

/// Inserts a delta with its sealed envelope in the row, unless the document's
/// inline tail is at `cap` (`SRV-148`, `SRV-150`). The count and the insert
/// are one statement. Two writers racing at the cap may each see room, so the
/// tail can pass it by the writers in flight at once.
///
/// The count reads the primary key range of the document past its newest batch
/// row. A move takes the oldest inline deltas and keeps the newest. No
/// index reads `wire`, so the move stays a heap-only update. A frame put back
/// in an older row (`attach_wire`) falls out of the count, and the tail can
/// pass the cap by that delta.
pub async fn insert_inline_update(
    pool: &PgPool,
    update: &NewUpdate<'_>,
    wire: &StoredPayload,
    cap: InlineCap,
) -> Result<InlineInsert, AppError> {
    let (deltas, bytes, room): (i64, i64, bool) = sqlx::query_as(
        "WITH tail AS ( \
             SELECT COUNT(*) AS n, COALESCE(SUM(octet_length(wire)), 0)::BIGINT AS bytes \
             FROM updates WHERE document_id = $1 AND wire IS NOT NULL \
               AND seq > COALESCE(( \
                   SELECT seq FROM updates WHERE document_id = $1 AND batch_key IS NOT NULL \
                   ORDER BY seq DESC LIMIT 1), 0) \
         ), ins AS ( \
             INSERT INTO updates (document_id, author_pubkey, snapshot_id, clock, seq, wire) \
             SELECT $1, $2, $3, $4, $5, $6 FROM tail \
             WHERE tail.n < $7 AND tail.bytes + octet_length($6) <= $8 \
             ON CONFLICT (document_id, author_pubkey, clock) DO NOTHING \
             RETURNING 1 \
         ) \
         SELECT n, bytes, n < $7 AND bytes + octet_length($6) <= $8 FROM tail",
    )
    .bind(id_to_uuid(*update.document_id))
    .bind(update.author_pubkey.as_slice())
    .bind(id_to_uuid(*update.snapshot_id))
    .bind(update.clock.to_i64_saturating())
    .bind(update.seq.to_i64_saturating())
    .bind(wire)
    .bind(cap.deltas.to_i64_saturating())
    .bind(cap.bytes.to_i64_saturating())
    .fetch_one(pool)
    .await?;
    if !room {
        return Ok(InlineInsert::Full);
    }
    Ok(InlineInsert::Stored {
        deltas: Count::<Deltas>::from_i64(deltas).incremented(),
        bytes: Count::<ambre_core::Octets>::from_i64(bytes)
            .plus(ambre_core::Count::<ambre_core::Octets>::of(wire.len())),
    })
}

/// Puts the sealed envelope of delta `seq` in its index row, the fallback when
/// its object can never be queued (`SRV-034`). The row drops the object key it
/// pointed at, since a delta has one home. A row already holding its frame or
/// pointing at a batch is left as it is. Returns whether a row took it.
pub async fn attach_wire(
    pool: &PgPool,
    document_id: &Id<Document>,
    seq: ambre_core::Seq<ambre_core::Relay>,
    wire: &StoredPayload,
) -> Result<bool, AppError> {
    let attached = sqlx::query(
        "UPDATE updates SET wire = $3, s3_key = NULL \
         WHERE document_id = $1 AND seq = $2 AND wire IS NULL AND batch_key IS NULL",
    )
    .bind(id_to_uuid(*document_id))
    .bind(seq.to_i64_saturating())
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
    document_id: &Id<Document>,
) -> Result<Option<Vec<(ambre_core::Seq<ambre_core::Relay>, StoredPayload)>>, AppError> {
    let rows: Result<Vec<(i64, StoredPayload)>, _> = sqlx::query_as(
        "SELECT seq, wire FROM updates WHERE document_id = $1 AND wire IS NOT NULL \
         ORDER BY seq FOR UPDATE NOWAIT",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_all(tx)
    .await;
    match rows {
        Ok(rows) => Ok(Some(
            rows.into_iter()
                .map(|(seq, wire)| Ok((decode_seq(seq)?, wire)))
                .collect::<Result<_, sqlx::Error>>()?,
        )),
        // `lock_not_available`: a move already runs.
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("55P03") => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Points the rows `seqs` at the batch object `batch_key`, and drops their
/// inline frames. The caller holds their locks.
pub async fn point_rows_at_batch(
    tx: &mut sqlx::PgConnection,
    document_id: &Id<Document>,
    seqs: &[ambre_core::Seq<ambre_core::Relay>],
    batch_key: &ambre_core::ObjectKey,
) -> Result<ambre_core::Count<ambre_core::Deltas>, AppError> {
    let moved = sqlx::query(
        "UPDATE updates SET wire = NULL, batch_key = $3 \
         WHERE document_id = $1 AND seq = ANY($2) AND wire IS NOT NULL",
    )
    .bind(id_to_uuid(*document_id))
    .bind(
        seqs.iter()
            .map(|s| s.to_i64_saturating())
            .collect::<Vec<_>>(),
    )
    .bind(batch_key.as_str())
    .execute(tx)
    .await?
    .rows_affected();
    Ok(ambre_core::Count::new(moved))
}

/// The documents with an inline tail and no write for `idle`, at most
/// `limit` (`SRV-152`).
pub async fn idle_inline_documents(
    pool: &PgPool,
    idle: TimeSpan<Sec>,
    limit: ambre_core::Bounded<QueryLimit>,
) -> Result<Vec<Id<Document>>, AppError> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT u.document_id FROM updates u WHERE u.wire IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM updates v WHERE v.document_id = u.document_id \
                         AND v.created_at > now() - make_interval(secs => $1::BIGINT::FLOAT8)) \
         LIMIT $2",
    )
    .bind(i64::try_from(idle.get()).unwrap_or(i64::MAX))
    .bind(i64::from(limit.get()))
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().map(uuid_row_to_id).collect())
}

/// Returns updates with seq > `after_seq` for delta-sync after reconnect.
pub async fn get_updates_after(
    pool: &PgPool,
    document_id: &Id<Document>,
    after_seq: ambre_core::Seq<ambre_core::Relay>,
    limit: ambre_core::Bounded<QueryLimit>,
) -> Result<Vec<UpdateRow>, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT author_pubkey, clock, seq, s3_key, wire, batch_key FROM updates WHERE document_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3"
    )
    .bind(did)
    .bind(after_seq.to_i64_saturating())
    .bind(i64::from(limit.get()))
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UpdateRow {
                author_pubkey: decode_key(r.try_get("author_pubkey")?)?,
                clock: decode_clock(r.try_get("clock")?)?,
                seq: decode_seq(r.try_get("seq")?)?,
                s3_key: r
                    .try_get::<Option<String>, _>("s3_key")?
                    .map(ambre_core::ObjectKey::new),
                wire: r.try_get("wire")?,
                batch_key: r
                    .try_get::<Option<String>, _>("batch_key")?
                    .map(ambre_core::ObjectKey::new),
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
    pub max_seq: ambre_core::Seq<ambre_core::Relay>,
    /// Per-author `(author_pubkey, max_clock)` high-water marks.
    pub author_clocks: Vec<(
        FixedBytes<ambre_core::PublicKey>,
        ambre_protocol::messages::Seq<ambre_protocol::messages::DeltaAuthor>,
    )>,
}

/// Reads the durable clock/seq high-water marks for a document. A cold Redis
/// can then be reseeded to a state consistent with the history clients have
/// already replayed (see [`ClockState`]).
///
/// The marks come from the `updates` rows and from what the trim kept of the
/// rows it removed (`SRV-300`). The counter also reaches the highest snapshot
/// horizon. So a trimmed document never numbers a delta at or below a horizon
/// again (`SRV-288`).
pub async fn get_clock_state(
    pool: &PgPool,
    document_id: &Id<Document>,
) -> Result<ClockState, AppError> {
    let did = id_to_uuid(*document_id);
    let rows = sqlx::query(
        "SELECT author_pubkey, MAX(clock) AS max_clock FROM (
             SELECT author_pubkey, clock FROM updates WHERE document_id = $1
             UNION ALL
             SELECT author_pubkey, max_clock FROM trimmed_high_water WHERE document_id = $1
         ) marks GROUP BY author_pubkey",
    )
    .bind(did)
    .fetch_all(pool)
    .await?;

    let mut state = ClockState {
        max_seq: durable_seq(pool, document_id).await?,
        author_clocks: Vec::with_capacity(rows.len()),
    };
    for r in &rows {
        let author = decode_key(r.try_get("author_pubkey")?)?;
        let max_clock = decode_clock(r.try_get("max_clock")?)?;
        state.author_clocks.push((author, max_clock));
    }
    Ok(state)
}

/// The highest delta number the durable store knows for a document. It reads
/// the `updates` rows, what the trim kept of the rows it removed, and the
/// snapshot horizons. Zero for a document with none of them.
///
/// A reset of the ephemeral store may leave its counter lower. So a number
/// the relay records for good takes the larger of the two (`SRV-288`,
/// `SRV-290`).
pub async fn durable_seq<'e, E>(
    executor: E,
    document_id: &Id<Document>,
) -> Result<ambre_core::Seq<ambre_core::Relay>, AppError>
where
    E: sqlx::PgExecutor<'e>,
{
    let max: i64 = sqlx::query_scalar(
        "SELECT GREATEST(
             (SELECT MAX(seq) FROM updates WHERE document_id = $1),
             (SELECT MAX(max_seq) FROM trimmed_high_water WHERE document_id = $1),
             (SELECT MAX(covered_seq) FROM snapshots WHERE document_id = $1),
             0)::BIGINT",
    )
    .bind(id_to_uuid(*document_id))
    .fetch_one(executor)
    .await?;
    Ok(decode_seq(max)?)
}

/// What each delete of [`purge_superseded_updates`] returns of a trimmed row.
const GONE: &str = "u.document_id, u.batch_key, u.author_pubkey, u.clock, u.seq";

/// The tail of each delete of [`purge_superseded_updates`]. It raises the
/// high-water marks of the trimmed authors and never lowers one. It answers
/// the document and the batch of each trimmed row.
const KEEP_HIGH_WATER: &str = ", kept AS (
        INSERT INTO trimmed_high_water (document_id, author_pubkey, max_clock, max_seq)
        SELECT document_id, author_pubkey, MAX(clock), MAX(seq) FROM gone
        GROUP BY document_id, author_pubkey
        ON CONFLICT (document_id, author_pubkey) DO UPDATE SET
            max_clock = GREATEST(trimmed_high_water.max_clock, EXCLUDED.max_clock),
            max_seq = GREATEST(trimmed_high_water.max_seq, EXCLUDED.max_seq))
        SELECT document_id, batch_key FROM gone";

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
/// `updates.snapshot_id` column carries no foreign key, as the `updates` table
/// of the initial schema states. Deleting an old snapshot therefore does not
/// cascade its deltas away. Without this pass the table would grow without bound.
/// Run once per compaction cycle, independent of snapshot deletion.
///
/// A batch object goes only once no row names it, so a batch that still holds
/// an uncovered delta stays (`SRV-154`).
pub async fn purge_superseded_updates(pool: &PgPool) -> Result<Trim, AppError> {
    // Three deletes, one per rule. One statement that joins the rules with
    // `OR` ran three correlated subqueries per row of `updates`. It was 25
    // times slower, and the pass runs each period (`SRV-277`).
    //
    // Each delete raises the high-water marks of the authors it trims, in the
    // same statement. A cold store then reseeds each author's clock and the
    // counter past every delta the trim removed (`SRV-288`).
    let statements = [
        // At or below the lowest horizon of the confirmed snapshots.
        format!(
            "WITH horizon AS (
            SELECT document_id, MIN(covered_seq) AS min_covered
            FROM snapshots
            WHERE status = 'confirmed'
            GROUP BY document_id
        ), gone AS (
        DELETE FROM updates u USING horizon h
        WHERE u.document_id = h.document_id AND u.seq <= h.min_covered
        RETURNING {GONE})
        {KEEP_HIGH_WATER}"
        ),
        // The author's own deltas under its latest self-confirmed snapshot.
        format!(
            "WITH reference AS (
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
        DELETE FROM updates u USING author_span a
        WHERE a.document_id = u.document_id AND a.author_pubkey = u.author_pubkey
          AND u.seq > a.from_seq AND u.seq <= a.to_seq
        RETURNING {GONE})
        {KEEP_HIGH_WATER}"
        ),
        // Under the confirmed final snapshot of a final segment.
        format!(
            "WITH gone AS (
        DELETE FROM updates u
        USING channel_documents c
        JOIN snapshots f ON f.id = c.final_snapshot AND f.document_id = c.document_id
        WHERE c.document_id = u.document_id AND c.state = 'final'
          AND f.status = 'confirmed' AND u.seq <= f.covered_seq
        RETURNING {GONE})
        {KEEP_HIGH_WATER}"
        ),
    ];

    let mut tx = pool.begin().await?;
    let mut rows: u64 = 0;
    let mut touched: Vec<(uuid::Uuid, String)> = Vec::new();
    for statement in &statements {
        let gone: Vec<(uuid::Uuid, Option<String>)> =
            sqlx::query_as(statement).fetch_all(&mut *tx).await?;
        rows = rows.saturating_add(u64::try_from(gone.len()).unwrap_or(u64::MAX));
        touched.extend(gone.into_iter().filter_map(|(d, k)| k.map(|k| (d, k))));
    }
    tx.commit().await?;
    touched.sort_unstable_by(|a, b| a.1.cmp(&b.1));
    touched.dedup();
    let (documents, touched): (Vec<uuid::Uuid>, Vec<String>) = touched.into_iter().unzip();
    // A key the deletes returned may still name a delta no rule covers, so
    // whether a batch is still named is a second question. A batch belongs to
    // one document, and no index reads `batch_key`, so the question reads the
    // primary key range of that document.
    let batch_keys: Vec<String> = sqlx::query_scalar(
        "SELECT k FROM unnest($1::UUID[], $2::TEXT[]) AS t(d, k) \
         WHERE NOT EXISTS (SELECT 1 FROM updates WHERE document_id = d AND batch_key = k)",
    )
    .bind(&documents)
    .bind(&touched)
    .fetch_all(pool)
    .await?;
    Ok(Trim {
        rows: ambre_core::Count::new(rows),
        batch_keys: batch_keys
            .into_iter()
            .map(ambre_core::ObjectKey::new)
            .collect(),
    })
}

/// Confirms a completed S3 upload by inserting a `media` row.
pub async fn insert_media(
    pool: &PgPool,
    document_id: &Id<Document>,
    uploaded_by: &Id<User>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
    size: ambre_core::Count<ambre_core::Octets>,
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
    .bind(i64::try_from(size.get()).unwrap_or(i64::MAX))
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
    document_id: &Id<Document>,
    user: &Id<User>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
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
        s3_key: ambre_core::ObjectKey::new(s3_key),
    })
}

/// Whether the media index records `content_hash` in `document_id`.
pub async fn document_holds_media(
    pool: &PgPool,
    document_id: &Id<Document>,
    content_hash: &ambre_core::Validated<ambre_core::ContentHash>,
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
    pub id: Id<Snapshot>,
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
    keep: Count<Snapshots>,
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
    .bind(i64::from(keep.get()))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Deletes a single snapshot row.
///
/// This does NOT touch `updates`. The `updates.snapshot_id` column carries no
/// foreign key, as the `updates` table of the initial schema states. The
/// column is an opaque client-side correlation tag with no cascade. Superseded deltas are trimmed
/// separately by [`purge_superseded_updates`]; the caller removes the S3 blob.
pub async fn delete_snapshot(pool: &PgPool, snapshot_id: &Id<Snapshot>) -> Result<(), AppError> {
    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(id_to_uuid(*snapshot_id))
        .execute(pool)
        .await?;
    Ok(())
}

/// A confirmation by `confirmer` under a blank signature, for a test of the
/// database layer, which stores the signature its caller has verified.
#[cfg(test)]
pub(crate) fn vouched_by(confirmer: &FixedBytes<ambre_core::PublicKey>) -> SignedConfirmation {
    SignedConfirmation {
        confirmer: *confirmer,
        signature: FixedBytes::from_array([0; 64]),
    }
}
