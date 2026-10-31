// ambre-server/src/compaction.rs
// When the relay asks a client for a snapshot of a document (`SRV-038`).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::state::AppState;
use crate::{db, redis_ops};
use ambre_core::id::{Document, Id};

/// The deltas past a document's served snapshot at which the relay asks for a
/// new one (`ambre-spec-budgets.md section 6`, the snapshot threshold row).
pub const SNAPSHOT_THRESHOLD: db::DeltaCount = db::DeltaCount(500);

/// The threshold of a Murmure segment, whose cold open replays its deltas one
/// by one (`ambre-spec-budgets.md section 6`, the active segment row). A
/// crossing is seen at most `CHECK_EVERY - 1` deltas late, so the ask leaves
/// before the 250th delta past the snapshot (the Murmure segments spec).
pub const SEGMENT_SNAPSHOT_THRESHOLD: db::DeltaCount = db::DeltaCount(250 - CHECK_EVERY.0);

/// How often, in deltas, the write path compares a document to the threshold.
/// The numbers of a document are consecutive, so one write in this many reads
/// the horizon, and no crossing goes unseen.
const CHECK_EVERY: db::DeltaCount = db::DeltaCount(32);

/// How long one request stands before the relay may ask again. A client that
/// never answers is asked again on the next crossing past it.
const ASK_TTL: ambre_core::Seconds = ambre_core::Seconds(300);

/// Whether the history past `horizon` at delta `seq` reaches `threshold`.
pub fn past_threshold(
    seq: ambre_core::Seq,
    horizon: ambre_core::Seq,
    threshold: db::DeltaCount,
) -> bool {
    seq.since(horizon) >= threshold
}

/// Whether delta `seq` is one the write path compares to the threshold: one
/// in every `CHECK_EVERY`.
fn is_checked(seq: ambre_core::Seq) -> bool {
    seq.0.is_multiple_of(CHECK_EVERY.0)
}

/// The threshold of a document: a Murmure segment's own, or the default.
async fn threshold_of(state: &AppState, document_id: &Id<Document>) -> db::DeltaCount {
    match db::channel_document_kind(&state.db, document_id).await {
        Ok(Some(ambre_core::ChannelDocumentKind::Segment)) => SEGMENT_SNAPSHOT_THRESHOLD,
        Ok(_) => SNAPSHOT_THRESHOLD,
        Err(e) => {
            tracing::warn!("channel document kind read failed doc={document_id}: {e}");
            SNAPSHOT_THRESHOLD
        }
    }
}

/// Whether the writer of delta `seq` is the one to ask for a snapshot: the
/// history passed the threshold, and no request already stands.
pub async fn should_ask(
    state: &AppState,
    document_id: &Id<Document>,
    seq: ambre_core::Seq,
) -> bool {
    if !is_checked(seq) {
        return false;
    }
    let horizon = match db::served_horizon(&state.db, document_id).await {
        Ok(horizon) => horizon,
        Err(e) => {
            tracing::warn!("snapshot horizon read failed doc={document_id}: {e}");
            return false;
        }
    };
    if !past_threshold(seq, horizon, threshold_of(state, document_id).await) {
        return false;
    }
    match redis_ops::claim_snapshot_ask(&state.redis, document_id, ASK_TTL).await {
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
    document_id: Id<Document>,
    seq: ambre_core::Seq,
    writer: &mpsc::Sender<crate::payload::Payload>,
) {
    if !is_checked(seq) {
        return;
    }
    let state = Arc::clone(state);
    let writer = writer.clone();
    tokio::spawn(async move {
        if should_ask(&state, &document_id, seq).await {
            let ask = crate::payload::Payload::server_msg(
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
        let at = threshold.0;
        assert!(!past_threshold(
            ambre_core::Seq(at - 1),
            ambre_core::Seq(0),
            threshold
        ));
        assert!(past_threshold(
            ambre_core::Seq(at),
            ambre_core::Seq(0),
            threshold
        ));
        assert!(!past_threshold(
            ambre_core::Seq(at + 10),
            ambre_core::Seq(20),
            threshold
        ));
        assert!(past_threshold(
            ambre_core::Seq(at + 20),
            ambre_core::Seq(20),
            threshold
        ));
        // A horizon past the counter, as after a cold counter, asks nothing.
        assert!(!past_threshold(
            ambre_core::Seq(10),
            ambre_core::Seq(900),
            threshold
        ));
    }

    /// The bound of the Murmure segments spec: a segment's crossing is seen
    /// by the 250th delta past its snapshot, wherever the snapshot fell.
    #[test]
    fn a_segment_is_asked_before_its_250th_delta() {
        for horizon in 0..CHECK_EVERY.0 {
            let seen = (horizon + 1..)
                .filter(|&seq| is_checked(ambre_core::Seq(seq)))
                .find(|&seq| {
                    past_threshold(
                        ambre_core::Seq(seq),
                        ambre_core::Seq(horizon),
                        SEGMENT_SNAPSHOT_THRESHOLD,
                    )
                })
                .expect("a crossing");
            assert!(seen - horizon < 250, "seen {} deltas past", seen - horizon);
        }
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::db::SnapshotStatus;
    use crate::test_helpers::{create_test_document, create_test_user, test_app_state};
    use ambre_core::id::Snapshot;

    const ALICE: [u8; 32] = [1u8; 32];
    const BOB: [u8; 32] = [2u8; 32];
    const CAROL: [u8; 32] = [3u8; 32];

    /// A state over `pool` and a fresh document.
    async fn setup(pool: sqlx::PgPool) -> (Arc<AppState>, Id<Document>) {
        let state = test_app_state(pool.clone()).await;
        let owner = create_test_user(&pool, "compaction@ambre.app").await;
        let doc = create_test_document(&pool, owner, "prose").await;
        (state, doc)
    }

    /// Stores delta `seq` under `author`.
    async fn delta(state: &AppState, doc: &Id<Document>, author: &[u8; 32], seq: i64) {
        let tag = Id::<Snapshot>::mint();
        db::insert_update(
            &state.db,
            doc,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(*author),
            &tag,
            ambre_protocol::messages::Clock(seq.unsigned_abs()),
            ambre_core::Seq(seq.unsigned_abs()),
            &ambre_core::ObjectKey(format!("k{seq}")),
        )
        .await
        .unwrap();
    }

    /// Stores a pending snapshot by `author` folding in `covered`.
    async fn snapshot(
        state: &AppState,
        doc: &Id<Document>,
        author: &[u8; 32],
        covered: i64,
    ) -> Id<Snapshot> {
        let json = serde_json::json!({});
        let key = format!("s3/{covered}");
        let key = ambre_core::ObjectKey(key);
        let covered = ambre_core::Seq::try_from(covered).expect("a relay number");
        db::create_snapshot(
            &state.db,
            doc,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(*author),
            &json,
            &key,
            covered,
        )
        .await
        .unwrap()
    }

    /// Each remaining delta's number and author tag.
    async fn rows(state: &AppState, doc: &Id<Document>) -> Vec<(i64, u8)> {
        db::get_updates_after(
            &state.db,
            doc,
            ambre_core::Seq(0),
            crate::db::QueryLimit::saturating(1000),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.seq.to_i64_saturating(), r.author_pubkey.as_array()[0]))
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
        let got = db::confirm_snapshot(
            &state.db,
            &doc,
            &sid,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(BOB),
        )
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
        db::confirm_snapshot(
            &state.db,
            &doc,
            &reference,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(ALICE),
        )
        .await
        .unwrap();
        let own = snapshot(&state, &doc, &ALICE, 6).await;
        let got = db::confirm_snapshot(
            &state.db,
            &doc,
            &own,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(ALICE),
        )
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
        db::confirm_snapshot(
            &state.db,
            &doc,
            &first,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(ALICE),
        )
        .await
        .unwrap();
        let second = snapshot(&state, &doc, &BOB, 6).await;
        let got = db::confirm_snapshot(
            &state.db,
            &doc,
            &second,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(BOB),
        )
        .await
        .unwrap();
        assert_eq!(got, Some(SnapshotStatus::Pending));
        // Another member's word still confirms it.
        let got = db::confirm_snapshot(
            &state.db,
            &doc,
            &second,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(CAROL),
        )
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
        db::confirm_snapshot(
            &state.db,
            &doc,
            &reference,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(BOB),
        )
        .await
        .unwrap();
        let own = snapshot(&state, &doc, &ALICE, 2).await;
        db::confirm_snapshot(
            &state.db,
            &doc,
            &own,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(ALICE),
        )
        .await
        .unwrap();
        let newer = snapshot(&state, &doc, &CAROL, 3).await;
        let dropped = db::find_compactable_snapshots(&state.db, db::SnapshotCount::new(1))
            .await
            .unwrap();
        let ids: Vec<Id<Snapshot>> = dropped.into_iter().map(|s| s.id).collect();
        assert!(!ids.contains(&reference));
        assert!(!ids.contains(&own));
        assert!(!ids.contains(&newer));
    }

    /// `SRV-038`: the writer is asked once the history past the served
    /// snapshot reaches the threshold, and once only while the ask stands.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_writer_is_asked_once_past_the_threshold(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let every = CHECK_EVERY.0;
        let below = every * (SNAPSHOT_THRESHOLD.0 / every);
        assert!(!should_ask(&state, &doc, ambre_core::Seq(below)).await);
        let due = below + every;
        assert!(should_ask(&state, &doc, ambre_core::Seq(due)).await);
        assert!(!should_ask(&state, &doc, ambre_core::Seq(due + every)).await);
        // Only a served snapshot moves the horizon.
        let covered = i64::try_from(due).unwrap();
        let sid = snapshot(&state, &doc, &ALICE, covered).await;
        assert_eq!(
            db::served_horizon(&state.db, &doc).await.unwrap(),
            ambre_core::Seq(0)
        );
        db::confirm_snapshot(
            &state.db,
            &doc,
            &sid,
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(ALICE),
        )
        .await
        .unwrap();
        assert_eq!(
            db::served_horizon(&state.db, &doc).await.unwrap(),
            ambre_core::Seq(due)
        );
    }
}
