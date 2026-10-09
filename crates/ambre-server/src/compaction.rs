// ambre-server/src/compaction.rs
// When the relay asks a client for a snapshot of a document (`SRV-038`).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::state::AppState;
use crate::{db, redis_ops};

/// The deltas past a document's served snapshot at which the relay asks for a
/// new one (`ambre-spec-budgets.md section 6`, the snapshot threshold row).
pub const SNAPSHOT_THRESHOLD: u64 = 500;

/// How often, in deltas, the write path compares a document to the threshold.
/// The numbers of a document are consecutive, so one write in this many reads
/// the horizon, and no crossing goes unseen.
const CHECK_EVERY: u64 = 32;

/// How long one request stands before the relay may ask again. A client that
/// never answers is asked again on the next crossing past it.
const ASK_TTL_SECS: u64 = 300;

/// Whether the history past `horizon` at delta `seq` warrants a snapshot.
pub fn past_threshold(seq: u64, horizon: i64) -> bool {
    seq.saturating_sub(u64::try_from(horizon).unwrap_or(0)) >= SNAPSHOT_THRESHOLD
}

/// Whether the writer of delta `seq` is the one to ask for a snapshot: the
/// history passed the threshold, and no request already stands.
pub async fn should_ask(state: &AppState, document_id: &ambre_core::DocumentId, seq: u64) -> bool {
    if !seq.is_multiple_of(CHECK_EVERY) {
        return false;
    }
    let horizon = match db::served_horizon(&state.db, document_id).await {
        Ok(horizon) => horizon,
        Err(e) => {
            tracing::warn!("snapshot horizon read failed doc={document_id}: {e}");
            return false;
        }
    };
    if !past_threshold(seq, horizon) {
        return false;
    }
    match redis_ops::claim_snapshot_ask(&state.redis, document_id, ASK_TTL_SECS).await {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::warn!("snapshot ask claim failed doc={document_id}: {e}");
            false
        }
    }
}

/// Asks the writer of delta `seq` for a snapshot when it is due, off the write
/// path: the delta is already stored and broadcast.
pub fn ask_when_due(
    state: &Arc<AppState>,
    document_id: ambre_core::DocumentId,
    seq: u64,
    writer: &mpsc::Sender<Vec<u8>>,
) {
    if !seq.is_multiple_of(CHECK_EVERY) {
        return;
    }
    let state = Arc::clone(state);
    let writer = writer.clone();
    tokio::spawn(async move {
        if should_ask(&state, &document_id, seq).await {
            let ask = ambre_protocol::ws::encode_server_msg(
                &ambre_protocol::ws::WsServerMsg::SnapshotRequired { document_id },
            );
            let _ = writer.send(ask).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay asks once the history past the served snapshot reaches the
    /// threshold, and not before (`SRV-038`).
    #[test]
    fn the_history_past_the_served_snapshot_decides() {
        let threshold = SNAPSHOT_THRESHOLD;
        assert!(!past_threshold(threshold - 1, 0));
        assert!(past_threshold(threshold, 0));
        assert!(!past_threshold(threshold + 10, 20));
        assert!(past_threshold(threshold + 20, 20));
        // A horizon past the counter, as after a cold counter, asks nothing.
        assert!(!past_threshold(10, 900));
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::db::SnapshotStatus;
    use crate::test_helpers::{create_test_document, create_test_user, test_app_state};

    const ALICE: [u8; 32] = [1u8; 32];
    const BOB: [u8; 32] = [2u8; 32];
    const CAROL: [u8; 32] = [3u8; 32];

    /// A state over `pool` and a fresh document.
    async fn setup(pool: sqlx::PgPool) -> (Arc<AppState>, ambre_core::DocumentId) {
        let state = test_app_state(pool.clone()).await;
        let owner = create_test_user(&pool, "compaction@ambre.app").await;
        let doc = create_test_document(&pool, owner, "prose").await;
        (state, doc)
    }

    /// Stores delta `seq` under `author`.
    async fn delta(state: &AppState, doc: &ambre_core::DocumentId, author: &[u8; 32], seq: i64) {
        let tag = ambre_core::SnapshotId::new();
        db::insert_update(&state.db, doc, author, &tag, seq, seq, &format!("k{seq}"))
            .await
            .unwrap();
    }

    /// Stores a pending snapshot by `author` folding in `covered`.
    async fn snapshot(
        state: &AppState,
        doc: &ambre_core::DocumentId,
        author: &[u8; 32],
        covered: i64,
    ) -> ambre_core::SnapshotId {
        let json = serde_json::json!({});
        let key = format!("s3/{covered}");
        db::create_snapshot(&state.db, doc, author, None, None, &json, &key, covered)
            .await
            .unwrap()
    }

    /// Each remaining delta's number and author tag.
    async fn rows(state: &AppState, doc: &ambre_core::DocumentId) -> Vec<(i64, u8)> {
        db::get_updates_after(&state.db, doc, 0, 1000)
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.seq, r.author_pubkey.first().copied().unwrap_or(0)))
            .collect()
    }

    /// `SRV-127`: a snapshot nobody confirmed moves no trim, and once another
    /// member confirms it the deltas it folds in go.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_pending_snapshot_moves_no_trim(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        for seq in 1..=4 {
            delta(&state, &doc, &ALICE, seq).await;
        }
        let sid = snapshot(&state, &doc, &ALICE, 3).await;
        let trim = db::purge_superseded_updates(&state.db).await.unwrap();
        assert_eq!(trim.rows, 0);
        let got = db::confirm_snapshot(&state.db, &doc, &sid, &BOB)
            .await
            .unwrap();
        assert_eq!(got, Some(SnapshotStatus::Confirmed));
        let trim = db::purge_superseded_updates(&state.db).await.unwrap();
        assert_eq!(trim.rows, 3);
        assert_eq!(rows(&state, &doc).await, vec![(4, 1)]);
    }

    /// `SRV-130`, `SRV-132`: a snapshot its author alone confirmed drops that
    /// author's deltas past the reference, up to what it folds in, and no
    /// other member's.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_self_snapshot_trims_its_author_alone(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        for seq in 1..=8 {
            let author = if seq % 2 == 0 { &BOB } else { &ALICE };
            delta(&state, &doc, author, seq).await;
        }
        let reference = snapshot(&state, &doc, &BOB, 2).await;
        db::confirm_snapshot(&state.db, &doc, &reference, &ALICE)
            .await
            .unwrap();
        let own = snapshot(&state, &doc, &ALICE, 6).await;
        let got = db::confirm_snapshot(&state.db, &doc, &own, &ALICE)
            .await
            .unwrap();
        assert_eq!(got, Some(SnapshotStatus::SelfConfirmed));
        db::purge_superseded_updates(&state.db).await.unwrap();
        // 1..=2 under the reference; Alice's 3 and 5 under her own snapshot.
        assert_eq!(
            rows(&state, &doc).await,
            vec![(4, 2), (6, 2), (7, 1), (8, 2)]
        );
    }

    /// `SRV-126`: an author may not confirm alone while a snapshot another
    /// author confirmed alone stands past the reference.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn one_author_alone_stands_past_the_reference(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let first = snapshot(&state, &doc, &ALICE, 4).await;
        db::confirm_snapshot(&state.db, &doc, &first, &ALICE)
            .await
            .unwrap();
        let second = snapshot(&state, &doc, &BOB, 6).await;
        let got = db::confirm_snapshot(&state.db, &doc, &second, &BOB)
            .await
            .unwrap();
        assert_eq!(got, Some(SnapshotStatus::Pending));
        // Another member's word still confirms it.
        let got = db::confirm_snapshot(&state.db, &doc, &second, &CAROL)
            .await
            .unwrap();
        assert_eq!(got, Some(SnapshotStatus::Confirmed));
    }

    /// `SRV-131`: the keep pass never drops the reference nor the latest
    /// snapshot its author confirmed alone, however old they are.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_keep_pass_holds_the_reference_and_the_latest_self(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let reference = snapshot(&state, &doc, &ALICE, 1).await;
        db::confirm_snapshot(&state.db, &doc, &reference, &BOB)
            .await
            .unwrap();
        let own = snapshot(&state, &doc, &ALICE, 2).await;
        db::confirm_snapshot(&state.db, &doc, &own, &ALICE)
            .await
            .unwrap();
        let newer = snapshot(&state, &doc, &CAROL, 3).await;
        let dropped = db::find_compactable_snapshots(&state.db, 1).await.unwrap();
        let ids: Vec<uuid::Uuid> = dropped.into_iter().map(|s| s.id).collect();
        assert!(!ids.contains(&db::id_to_uuid(reference)));
        assert!(!ids.contains(&db::id_to_uuid(own)));
        assert!(!ids.contains(&db::id_to_uuid(newer)));
    }

    /// `SRV-038`: the writer is asked once the history past the served
    /// snapshot reaches the threshold, and once only while the ask stands.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_writer_is_asked_once_past_the_threshold(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let below = CHECK_EVERY * (SNAPSHOT_THRESHOLD / CHECK_EVERY);
        assert!(!should_ask(&state, &doc, below).await);
        let due = below + CHECK_EVERY;
        assert!(should_ask(&state, &doc, due).await);
        assert!(!should_ask(&state, &doc, due + CHECK_EVERY).await);
        // Only a served snapshot moves the horizon.
        let covered = i64::try_from(due).unwrap();
        let sid = snapshot(&state, &doc, &ALICE, covered).await;
        assert_eq!(db::served_horizon(&state.db, &doc).await.unwrap(), 0);
        db::confirm_snapshot(&state.db, &doc, &sid, &ALICE)
            .await
            .unwrap();
        assert_eq!(db::served_horizon(&state.db, &doc).await.unwrap(), covered);
    }
}
