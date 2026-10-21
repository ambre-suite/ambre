// ambre-server/src/hot_tail.rs
// The hot tail of deltas in the index, and the batch objects its overflow goes to.

use std::collections::{BTreeMap, HashMap};

use crate::db::InlineCap;
use crate::error::AppError;
use crate::payload::Payload;
use crate::state::AppState;

/// A delta whose sealed envelope is under this size lives in its index row
/// (`ambre-spec-budgets.md`, the inline delta size row).
pub const INLINE_DELTA_MAX_BYTES: usize = 64 * 1024;

/// The most a document holds inline (`ambre-spec-budgets.md`, the inline
/// deltas per document row; `SRV-150`).
pub const INLINE_CAP: InlineCap = InlineCap {
    deltas: 256,
    bytes: 256 * 1024,
};

/// How long a document goes without a write before its whole tail moves out
/// (`ambre-spec-budgets.md section 8`; `SRV-152`).
pub const IDLE_SECS: i64 = 24 * 3600;

/// How many idle documents one pass moves.
const IDLE_PASS_LIMIT: i64 = 100;

/// Whether a tail of `deltas` rows and `bytes` bytes should move now. The move
/// starts at three quarters of the cap, so a write rarely meets a full tail.
pub fn past_watermark(deltas: i64, bytes: i64) -> bool {
    deltas * 4 >= INLINE_CAP.deltas * 3 || bytes * 4 >= INLINE_CAP.bytes * 3
}

/// How many of the newest frames of `tail` stay inline after a move. It is at
/// most a quarter of the cap, in deltas and in bytes alike.
fn kept_after_move(tail: &[(ambre_core::Seq, Payload)]) -> usize {
    let mut kept = 0usize;
    let mut bytes = 0i64;
    for (_, wire) in tail.iter().rev() {
        let size = i64::try_from(wire.len()).unwrap_or(i64::MAX);
        let next = i64::try_from(kept + 1).unwrap_or(i64::MAX);
        if next * 4 > INLINE_CAP.deltas || (bytes + size) * 4 > INLINE_CAP.bytes {
            break;
        }
        kept += 1;
        bytes += size;
    }
    kept
}

/// The object a batch of deltas is stored under, named by its first and last
/// numbers. It sits under the document's prefix, so a delete sweeps it.
pub fn batch_key(
    env_prefix: &str,
    document_id: &ambre_core::DocumentId,
    first: ambre_core::Seq,
    last: ambre_core::Seq,
) -> ambre_core::ObjectKey {
    ambre_core::ObjectKey(format!(
        "{env_prefix}/documents/{document_id}/batches/{:010}-{:010}",
        first.0, last.0
    ))
}

/// Encodes a batch: each delta's number and its sealed envelope, unchanged
/// (`SRV-153`).
fn encode_batch(frames: &[(ambre_core::Seq, Payload)]) -> Result<Payload, AppError> {
    Payload::batch(frames).map_err(|e| AppError::Internal(format!("batch encode: {e}")))
}

/// Decodes a batch into its frames by number.
fn decode_batch(batch: &Payload) -> Result<BTreeMap<ambre_core::Seq, Payload>, AppError> {
    batch
        .unbatch()
        .map_err(|e| AppError::Internal(format!("batch decode: {e}")))
}

/// Moves the oldest inline deltas of a document into one batch object, and
/// points their rows at it (`SRV-151`). The newest `keep` stay, or a quarter of
/// the cap when `keep` is `None`. Returns how many moved: none when the tail is
/// short, or when another move holds it.
///
/// The rows stay locked while the object is written, so a trim waits for the
/// move. A move whose commit fails leaves its object behind, unnamed, until
/// the document is deleted.
pub async fn move_to_batch(
    state: &AppState,
    document_id: &ambre_core::DocumentId,
    keep: Option<usize>,
) -> Result<usize, AppError> {
    let mut tx = state.db.begin().await?;
    let Some(tail) = crate::db::lock_inline_tail(&mut tx, document_id).await? else {
        return Ok(0);
    };
    let keep = keep.unwrap_or_else(|| kept_after_move(&tail));
    let take = tail.len().saturating_sub(keep);
    let Some(moved) = tail.get(..take).filter(|m| !m.is_empty()) else {
        return Ok(0);
    };
    let (Some((first, _)), Some((last, _))) = (moved.first(), moved.last()) else {
        return Ok(0);
    };
    let key = batch_key(&state.config.s3_env_prefix, document_id, *first, *last);
    let body = encode_batch(moved)?;
    state
        .s3
        .put_object()
        .bucket(&state.s3_bucket)
        .key(&key)
        .body(body.into_body())
        .send()
        .await
        .map_err(|e| AppError::S3(format!("batch put {key}: {e}")))?;
    let seqs: Vec<ambre_core::Seq> = moved.iter().map(|(seq, _)| *seq).collect();
    crate::db::point_rows_at_batch(&mut tx, document_id, &seqs, &key).await?;
    tx.commit().await?;
    Ok(seqs.len())
}

/// Starts a move off the write path. A failure is logged, and the next write
/// past the watermark starts another.
pub fn spawn_move(state: &std::sync::Arc<AppState>, document_id: ambre_core::DocumentId) {
    let state = std::sync::Arc::clone(state);
    tokio::spawn(async move {
        if let Err(e) = move_to_batch(&state, &document_id, None).await {
            tracing::warn!(document_id = %document_id, error = %e, "inline tail move failed");
        }
    });
}

/// Moves the whole tail of every document idle past the delay (`SRV-152`).
pub async fn move_idle_tails(state: &AppState) {
    let idle = match crate::db::idle_inline_documents(&state.db, IDLE_SECS, IDLE_PASS_LIMIT).await {
        Ok(idle) => idle,
        Err(e) => {
            tracing::error!("idle tails read failed: {e}");
            return;
        }
    };
    for doc in idle {
        if let Err(e) = move_to_batch(state, &doc, Some(0)).await {
            tracing::warn!(document_id = %doc, error = %e, "idle tail move failed");
        }
    }
}

/// Trims the delta index, then deletes each batch object no row names any more
/// (`SRV-039`, `SRV-154`). A batch whose delete fails stays, unnamed, until the
/// document is deleted.
pub async fn trim(state: &AppState) -> Result<u64, AppError> {
    let trim = crate::db::purge_superseded_updates(&state.db).await?;
    for key in &trim.batch_keys {
        if let Err(e) = state
            .s3
            .delete_object()
            .bucket(&state.s3_bucket)
            .key(key)
            .send()
            .await
        {
            tracing::warn!(key = %key, error = %e, "trimmed batch delete failed");
        }
    }
    Ok(trim.rows)
}

/// The batch objects one replay has fetched, each fetched once (`SRV-155`).
#[derive(Default)]
pub struct BatchCache {
    held: HashMap<ambre_core::ObjectKey, BTreeMap<ambre_core::Seq, Payload>>,
}

impl BatchCache {
    /// Fetches, together, every batch a page of rows names and the cache does
    /// not hold. A batch that fails to load stays out, and its deltas read as
    /// missing.
    pub async fn load(&mut self, state: &AppState, rows: &[crate::db::UpdateRow]) {
        let mut wanted: Vec<&ambre_core::ObjectKey> = rows
            .iter()
            .filter_map(|r| r.batch_key.as_ref())
            .filter(|k| !self.held.contains_key(*k))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let fetches = wanted.into_iter().map(|key| async move {
            let got = crate::s3::fetch_object(&state.s3, &state.s3_bucket, key).await;
            (key, got)
        });
        for (key, got) in futures_util::future::join_all(fetches).await {
            match got.map(|bytes| bytes.map(|b| decode_batch(&b))) {
                Ok(Some(Ok(frames))) => {
                    self.held.insert(key.clone(), frames);
                }
                Ok(Some(Err(e))) | Err(e) => {
                    tracing::error!(key = %key, error = %e, "replay: batch read failed")
                }
                Ok(None) => tracing::error!(key = %key, "replay: batch object missing"),
            }
        }
    }

    /// The frame of delta `seq` in batch `key`, when the batch is held.
    pub fn frame(&self, key: &ambre_core::ObjectKey, seq: ambre_core::Seq) -> Option<Payload> {
        self.held.get(key)?.get(&seq).cloned()
    }

    /// Forgets every batch whose last delta is at or below `seq`: a replay goes
    /// forward, so it never needs one again.
    pub fn forget_through(&mut self, seq: ambre_core::Seq) {
        self.held
            .retain(|_, frames| frames.last_key_value().is_some_and(|(last, _)| *last > seq));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(n: usize, size: usize) -> Vec<(ambre_core::Seq, Payload)> {
        (1..=n)
            .map(|s| {
                (
                    ambre_core::Seq(u64::try_from(s).unwrap()),
                    Payload::new(vec![0u8; size]),
                )
            })
            .collect()
    }

    /// `SRV-151`: the move starts at three quarters of the count or the bytes.
    #[test]
    fn a_move_starts_at_three_quarters_of_either_bound() {
        assert!(!past_watermark(191, 0));
        assert!(past_watermark(192, 0));
        assert!(!past_watermark(1, 192 * 1024 - 1));
        assert!(past_watermark(1, 192 * 1024));
    }

    /// `SRV-151`: a move keeps the newest quarter of either bound inline.
    #[test]
    fn a_move_keeps_a_quarter_of_the_cap() {
        assert_eq!(kept_after_move(&tail(200, 10)), 64);
        assert_eq!(kept_after_move(&tail(200, 16 * 1024)), 4);
        assert_eq!(kept_after_move(&tail(3, 10)), 3);
    }

    /// `SRV-153`: a batch decodes to the numbers and frames it was built from.
    #[test]
    fn a_batch_keeps_each_number_and_frame() {
        let frames = vec![(3, vec![1, 2]), (7, vec![]), (9, vec![5; 300])];
        let sealed: Vec<(ambre_core::Seq, Payload)> = frames
            .iter()
            .map(|(s, f)| (ambre_core::Seq(*s), Payload::new(f.clone())))
            .collect();
        let back = decode_batch(&encode_batch(&sealed).unwrap()).unwrap();
        let back: Vec<(u64, Vec<u8>)> = back
            .into_iter()
            .map(|(s, f)| (s.0, f.expose_bytes().to_vec()))
            .collect();
        assert_eq!(back, frames);
    }
}

// The tail, the move, the idle pass and the trim, read back from the register
// and from an in-process object store. Each test needs Postgres and Redis.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::db::InlineInsert;
    use crate::test_helpers::{
        S3Stub, create_test_document, create_test_user, offline_s3, test_app_state, with_s3,
    };
    use std::sync::Arc;

    /// A state over `pool` whose object store is `stub`, and a fresh document.
    async fn setup(pool: sqlx::PgPool, stub: &S3Stub) -> (Arc<AppState>, ambre_core::DocumentId) {
        let base = test_app_state(pool.clone()).await;
        let state = with_s3(&base, offline_s3(Some(&stub.endpoint)));
        let owner = create_test_user(&pool, "tail@ambre.app").await;
        let doc = create_test_document(&pool, owner, "prose").await;
        (state, doc)
    }

    /// Inserts delta `seq` with `wire` in its row, under `cap`.
    async fn inline(
        state: &AppState,
        doc: &ambre_core::DocumentId,
        seq: i64,
        wire: &[u8],
        cap: InlineCap,
    ) -> InlineInsert {
        let snap = ambre_core::SnapshotId::new();
        let wire = Payload::new(wire.to_vec());
        crate::db::insert_inline_update(
            &state.db,
            doc,
            &ambre_core::PublicKey::from_array([5u8; 32]),
            &snap,
            ambre_protocol::messages::Clock(seq.unsigned_abs()),
            ambre_core::Seq(seq.unsigned_abs()),
            &wire,
            cap,
        )
        .await
        .unwrap()
    }

    /// The frame of delta `seq`: a few bytes no other delta has.
    fn frame(seq: i64) -> Vec<u8> {
        format!("frame-{seq}").into_bytes()
    }

    /// Each row's number, inline frame and batch, in order.
    async fn homes(
        state: &AppState,
        doc: &ambre_core::DocumentId,
    ) -> Vec<(i64, bool, Option<String>)> {
        crate::db::get_updates_after(&state.db, doc, ambre_core::Seq(0), 1000)
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.seq.to_i64_saturating(),
                    r.wire.is_some(),
                    r.batch_key.map(|k| k.0),
                )
            })
            .collect()
    }

    /// `SRV-150`: a tail at its cap, in deltas or in bytes, takes no more, and
    /// the delta is left for an object of its own.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_full_tail_takes_no_more_inline(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let (state, doc) = setup(pool, &stub).await;
        let cap = INLINE_CAP.deltas;
        for seq in 1..=cap {
            let got = inline(&state, &doc, seq, &frame(seq), INLINE_CAP).await;
            assert!(matches!(got, InlineInsert::Stored { deltas, .. } if deltas == seq));
        }
        let full = inline(&state, &doc, cap + 1, &frame(cap + 1), INLINE_CAP).await;
        assert_eq!(full, InlineInsert::Full);
        assert_eq!(
            homes(&state, &doc).await.len(),
            usize::try_from(cap).unwrap()
        );

        let owner = create_test_user(&state.db, "bytes@ambre.app").await;
        let other = create_test_document(&state.db, owner, "prose").await;
        let small = InlineCap {
            deltas: 100,
            bytes: 10,
        };
        assert!(matches!(
            inline(&state, &other, 1, &[0; 6], small).await,
            InlineInsert::Stored { .. }
        ));
        assert_eq!(
            inline(&state, &other, 2, &[0; 6], small).await,
            InlineInsert::Full
        );
    }

    /// `SRV-151`, `SRV-153`: a move takes the oldest deltas into one batch
    /// object and keeps a quarter of the cap inline. Every number and every
    /// frame stays as it was.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_move_takes_the_oldest_and_keeps_the_newest(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let (state, doc) = setup(pool, &stub).await;
        for seq in 1..=200 {
            inline(&state, &doc, seq, &frame(seq), INLINE_CAP).await;
        }
        assert_eq!(move_to_batch(&state, &doc, None).await.unwrap(), 136);

        let key = batch_key(
            &state.config.s3_env_prefix,
            &doc,
            ambre_core::Seq(1),
            ambre_core::Seq(136),
        );
        let rows = homes(&state, &doc).await;
        assert!(
            rows.iter()
                .take(136)
                .all(|(_, w, b)| !w && b.as_deref() == Some(key.as_str()))
        );
        assert!(rows.iter().skip(136).all(|(_, w, b)| *w && b.is_none()));
        let held: BTreeMap<i64, Vec<u8>> =
            decode_batch(&Payload::new(stub.object(key.as_str()).unwrap()))
                .unwrap()
                .into_iter()
                .map(|(s, f)| (s.to_i64_saturating(), f.expose_bytes().to_vec()))
                .collect();
        let expected: BTreeMap<i64, Vec<u8>> = (1..=136).map(|s| (s, frame(s))).collect();
        assert_eq!(
            held, expected,
            "the batch holds each number and frame unchanged"
        );
    }

    /// `SRV-152`: a document idle past the delay has its whole tail moved, and
    /// one written since keeps its own.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn an_idle_tail_moves_whole(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let (state, idle) = setup(pool, &stub).await;
        let owner = create_test_user(&state.db, "busy@ambre.app").await;
        let busy = create_test_document(&state.db, owner, "prose").await;
        for seq in 1..=5 {
            inline(&state, &idle, seq, &frame(seq), INLINE_CAP).await;
            inline(&state, &busy, seq, &frame(seq), INLINE_CAP).await;
        }
        sqlx::query(
            "UPDATE updates SET created_at = now() - interval '25 hours' WHERE document_id = $1",
        )
        .bind(crate::db::id_to_uuid(idle))
        .execute(&state.db)
        .await
        .unwrap();
        move_idle_tails(&state).await;

        assert!(
            homes(&state, &idle)
                .await
                .iter()
                .all(|(_, w, b)| !w && b.is_some())
        );
        assert!(
            homes(&state, &busy)
                .await
                .iter()
                .all(|(_, w, b)| *w && b.is_none())
        );
    }

    /// `SRV-154`: a trim drops a batch object once no row names it, and keeps
    /// one that still holds a delta no snapshot covers.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_trim_drops_a_batch_only_once_no_row_names_it(pool: sqlx::PgPool) {
        let stub = S3Stub::spawn([]).await;
        let (state, doc) = setup(pool, &stub).await;
        for seq in 1..=5 {
            inline(&state, &doc, seq, &frame(seq), INLINE_CAP).await;
        }
        move_to_batch(&state, &doc, Some(0)).await.unwrap();
        for seq in 6..=10 {
            inline(&state, &doc, seq, &frame(seq), INLINE_CAP).await;
        }
        move_to_batch(&state, &doc, Some(0)).await.unwrap();
        let older = batch_key(
            &state.config.s3_env_prefix,
            &doc,
            ambre_core::Seq(1),
            ambre_core::Seq(5),
        );
        let newer = batch_key(
            &state.config.s3_env_prefix,
            &doc,
            ambre_core::Seq(6),
            ambre_core::Seq(10),
        );
        assert_eq!(stub.keys(), [older.0.clone(), newer.0.clone()]);

        let clocks = serde_json::json!({});
        let snap = crate::db::create_snapshot(
            &state.db,
            &doc,
            &ambre_core::PublicKey::from_array([5u8; 32]),
            &clocks,
            &ambre_core::ObjectKey::from("snap"),
            ambre_core::Seq(7),
        )
        .await
        .unwrap();
        crate::db::confirm_snapshot(
            &state.db,
            &doc,
            &snap,
            &ambre_core::PublicKey::from_array([9u8; 32]),
        )
        .await
        .unwrap();
        assert_eq!(trim(&state).await.unwrap(), 7);
        assert_eq!(
            stub.keys(),
            [newer.0],
            "the batch still naming 8 to 10 stays"
        );
    }
}
