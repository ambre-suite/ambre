// ambre-server/src/compaction.rs
// When the relay asks a client for a snapshot of a document (`SRV-038`).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::state::AppState;
use crate::{db, redis_ops};
use ambre_core::id::{Document, Id};
use ambre_core::{Count, Sec, TimeSpan};

/// The deltas past a document's served snapshot at which the relay asks for a
/// new one (`ambre-spec-budgets.md section 6`, the snapshot threshold row).
pub const SNAPSHOT_THRESHOLD: Count<db::Deltas> = Count::<db::Deltas>::new(500);

/// Deltas one document sends in a second.
pub enum DeltasPerSecond {}

impl ambre_core::NarrowCount for DeltasPerSecond {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeltasPerSecond");
}

/// The delta rate of an active document that the pass period is sized for
/// (`ambre-spec-budgets.md section 6`, the design delta rate row).
pub const DESIGN_DELTA_RATE: Count<DeltasPerSecond> = Count::<DeltasPerSecond>::new(5);

/// The period of the compaction pass: the time a document at the design rate
/// takes to cross the snapshot threshold (`SRV-277`). So one pass at most
/// falls between two snapshots, and a document adds one threshold of deltas
/// between two passes (`ambre-spec-budgets.md section 6`, the pass period row).
pub const PASS_PERIOD: TimeSpan<Sec> =
    TimeSpan::<Sec>::new(SNAPSHOT_THRESHOLD.get() / DESIGN_DELTA_RATE.get() as u64);

/// Runs `pass` once each `period`, start to start, for as long as the task
/// lives. The first pass runs one period after the call. A pass that outlasts
/// the period delays the next one, and no pass is ever run twice to catch up.
pub async fn every<F, Fut>(period: TimeSpan<Sec>, mut pass: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let period = period.to_duration();
    let mut ticks = tokio::time::interval(period);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick completes at once: the boot owes no pass.
    ticks.tick().await;
    loop {
        ticks.tick().await;
        pass().await;
    }
}

/// The snapshots per document each pass keeps, besides the reference and
/// the latest one its author confirmed alone (`SRV-131`, `SRV-264`). The trim
/// stops at the older of the two. A document keeps the deltas past it
/// (`ambre-spec-budgets.md section 6`, the delta rows kept row). The older
/// snapshot lets an open served it a moment before finish its replay.
pub const SNAPSHOTS_KEPT: Count<db::Snapshots> = Count::<db::Snapshots>::new(2);

/// The threshold of a Murmure segment, whose cold open replays its deltas one
/// by one (`ambre-spec-budgets.md section 6`, the active segment row). A
/// crossing is seen at most `CHECK_EVERY - 1` deltas late, so the ask leaves
/// before the 250th delta past the snapshot (the Murmure segments spec).
pub const SEGMENT_SNAPSHOT_THRESHOLD: Count<db::Deltas> =
    Count::<db::Deltas>::new(250 - CHECK_EVERY.get());

/// How often, in deltas, the write path compares a document to the threshold.
/// The numbers of a document are consecutive, so one write in this many reads
/// the horizon, and no crossing goes unseen.
const CHECK_EVERY: Count<db::Deltas> = Count::<db::Deltas>::new(32);

/// How long one request stands before the relay may ask again. A client that
/// never answers is asked again on the next crossing past it.
const ASK_TTL: TimeSpan<Sec> = TimeSpan::<Sec>::new(300);

/// Whether the history past `horizon` at delta `seq` reaches `threshold`.
pub fn past_threshold(
    seq: ambre_core::Seq<ambre_core::Relay>,
    horizon: ambre_core::Seq<ambre_core::Relay>,
    threshold: Count<db::Deltas>,
) -> bool {
    seq.since(horizon) >= threshold
}

/// Whether delta `seq` is one the write path compares to the threshold: one
/// in every `CHECK_EVERY`.
fn is_checked(seq: ambre_core::Seq<ambre_core::Relay>) -> bool {
    seq.get().is_multiple_of(CHECK_EVERY.get())
}

/// The threshold of a document: a Murmure segment's own, or the default.
async fn threshold_of(state: &AppState, document_id: &Id<Document>) -> Count<db::Deltas> {
    match db::channel_document_kind(&state.db, document_id).await {
        Ok(Some(ambre_core::ChannelDocumentKind::Segment)) => SEGMENT_SNAPSHOT_THRESHOLD,
        Ok(_) => SNAPSHOT_THRESHOLD,
        Err(e) => {
            tracing::warn!("channel document kind read failed: {e}");
            SNAPSHOT_THRESHOLD
        }
    }
}

/// Whether the writer of delta `seq` is the one to ask for a snapshot: the
/// history passed the threshold, and no request already stands.
pub async fn should_ask(
    state: &AppState,
    document_id: &Id<Document>,
    seq: ambre_core::Seq<ambre_core::Relay>,
) -> bool {
    if !is_checked(seq) {
        return false;
    }
    let horizon = match db::served_horizon(&state.db, document_id).await {
        Ok(horizon) => horizon,
        Err(e) => {
            tracing::warn!("snapshot horizon read failed: {e}");
            return false;
        }
    };
    if !past_threshold(seq, horizon, threshold_of(state, document_id).await) {
        return false;
    }
    match redis_ops::claim_snapshot_ask(&state.redis, document_id, ASK_TTL).await {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::warn!("snapshot ask claim failed: {e}");
            false
        }
    }
}

/// Asks the writer of delta `seq` for a snapshot when it is due, off the write
/// path: the delta is already stored and broadcast.
pub fn ask_when_due(
    state: &Arc<AppState>,
    document_id: Id<Document>,
    seq: ambre_core::Seq<ambre_core::Relay>,
    writer: &mpsc::Sender<crate::payload::StoredPayload>,
) {
    if !is_checked(seq) {
        return;
    }
    let state = Arc::clone(state);
    let writer = writer.clone();
    tokio::spawn(async move {
        if should_ask(&state, &document_id, seq).await {
            let ask = crate::payload::StoredPayload::server_msg(
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
        let at = threshold.get();
        assert!(!past_threshold(
            ambre_core::Seq::<ambre_core::Relay>::new(at - 1),
            ambre_core::Seq::<ambre_core::Relay>::new(0),
            threshold
        ));
        assert!(past_threshold(
            ambre_core::Seq::<ambre_core::Relay>::new(at),
            ambre_core::Seq::<ambre_core::Relay>::new(0),
            threshold
        ));
        assert!(!past_threshold(
            ambre_core::Seq::<ambre_core::Relay>::new(at + 10),
            ambre_core::Seq::<ambre_core::Relay>::new(20),
            threshold
        ));
        assert!(past_threshold(
            ambre_core::Seq::<ambre_core::Relay>::new(at + 20),
            ambre_core::Seq::<ambre_core::Relay>::new(20),
            threshold
        ));
        // A horizon past the counter, as after a cold counter, asks nothing.
        assert!(!past_threshold(
            ambre_core::Seq::<ambre_core::Relay>::new(10),
            ambre_core::Seq::<ambre_core::Relay>::new(900),
            threshold
        ));
    }

    /// `SRV-277`: at the design rate, a document adds one snapshot threshold
    /// of deltas between two passes, and no more.
    #[test]
    fn the_pass_period_lets_one_threshold_in_at_the_design_rate() {
        let rate = u64::from(DESIGN_DELTA_RATE.get());
        let added = PASS_PERIOD.get() * rate;
        assert!(added <= SNAPSHOT_THRESHOLD.get(), "{added} deltas a period");
        assert!(added + rate > SNAPSHOT_THRESHOLD.get());
    }

    /// `SRV-277`: the loop runs one pass each period, start to start, and the
    /// first one a period after it starts.
    #[tokio::test(start_paused = true)]
    async fn the_loop_runs_one_pass_each_period() {
        let passes = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = std::sync::Arc::clone(&passes);
        let task = tokio::spawn(every(PASS_PERIOD, move || {
            let counted = std::sync::Arc::clone(&counted);
            async move {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));
        // Each read falls one second off a tick, so the paused clock never
        // wakes the test and the loop at the same instant.
        let period = PASS_PERIOD.to_duration();
        let second = std::time::Duration::from_secs(1);
        let read = || passes.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(period - second).await;
        assert_eq!(read(), 0, "a pass before the first period");
        for expected in 1..=3 {
            tokio::time::sleep(2 * second).await;
            assert_eq!(read(), expected);
            tokio::time::sleep(period - 2 * second).await;
            assert_eq!(read(), expected, "a pass inside the period");
        }
        task.abort();
    }

    /// The bound of the Murmure segments spec: a segment's crossing is seen
    /// by the 250th delta past its snapshot, wherever the snapshot fell.
    #[test]
    fn a_segment_is_asked_before_its_250th_delta() {
        for horizon in 0..CHECK_EVERY.get() {
            let seen = (horizon + 1..)
                .filter(|&seq| is_checked(ambre_core::Seq::<ambre_core::Relay>::new(seq)))
                .find(|&seq| {
                    past_threshold(
                        ambre_core::Seq::<ambre_core::Relay>::new(seq),
                        ambre_core::Seq::<ambre_core::Relay>::new(horizon),
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
            ambre_protocol::messages::Seq::<ambre_protocol::messages::DeltaAuthor>::new(
                seq.unsigned_abs(),
            ),
            ambre_core::Seq::<ambre_core::Relay>::new(seq.unsigned_abs()),
            &ambre_core::ObjectKey::new(format!("k{seq}")),
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
        let key = format!("s3/{covered}");
        let key = ambre_core::ObjectKey::new(key);
        let covered =
            ambre_core::Seq::<ambre_core::Relay>::try_from(covered).expect("a relay number");
        db::create_snapshot(
            &state.db,
            doc,
            ambre_core::id::Id::<ambre_core::id::Snapshot>::mint(),
            &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(*author),
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
            ambre_core::Seq::<ambre_core::Relay>::new(0),
            ambre_core::Bounded::<crate::db::QueryLimit>::of::<1000>(),
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
        assert_eq!(trim.rows.get(), 0);
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
        assert_eq!(trim.rows.get(), 3);
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
        let dropped =
            db::find_compactable_snapshots(&state.db, ambre_core::Count::<db::Snapshots>::new(1))
                .await
                .unwrap();
        let ids: Vec<Id<Snapshot>> = dropped.into_iter().map(|s| s.id).collect();
        assert!(!ids.contains(&reference));
        assert!(!ids.contains(&own));
        assert!(!ids.contains(&newer));
    }

    /// `SRV-264`: snapshots come at the threshold. A pass keeps the two latest
    /// and the deltas past the older one, under the bound of the budget row.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn a_pass_keeps_the_deltas_past_the_older_kept_snapshot(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let every = i64::try_from(SNAPSHOT_THRESHOLD.get()).unwrap();
        let snapshots = 5;
        let last = every * snapshots + every / 2;
        for seq in 1..=last {
            delta(&state, &doc, &ALICE, seq).await;
            if seq % every == 0 {
                let sid = snapshot(&state, &doc, &ALICE, seq).await;
                db::confirm_snapshot(
                    &state.db,
                    &doc,
                    &sid,
                    &ambre_core::FixedBytes::<ambre_core::PublicKey>::from_array(BOB),
                )
                .await
                .unwrap();
            }
        }
        crate::compact_once(&state).await;

        let kept: Vec<i64> = sqlx::query_scalar(
            "SELECT covered_seq FROM snapshots WHERE document_id = $1 ORDER BY covered_seq",
        )
        .bind(crate::db::id_to_uuid(doc))
        .fetch_all(&state.db)
        .await
        .unwrap();
        let held = i64::from(SNAPSHOTS_KEPT.get());
        let older = every * (snapshots - held + 1);
        let expected: Vec<i64> = (snapshots - held + 1..=snapshots)
            .map(|k| k * every)
            .collect();
        assert_eq!(kept, expected);

        let (rows, lowest): (i64, i64) =
            sqlx::query_as("SELECT COUNT(*), MIN(seq) FROM updates WHERE document_id = $1")
                .bind(crate::db::id_to_uuid(doc))
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(lowest, older + 1);
        assert_eq!(rows, last - older);
        assert!(rows <= every * held, "{rows} rows kept");
    }

    /// `SRV-038`: the writer is asked once the history past the served
    /// snapshot reaches the threshold, and once only while the ask stands.
    #[sqlx::test]
    #[cfg_attr(not(feature = "db"), ignore = "needs postgres; run with --features db")]
    async fn the_writer_is_asked_once_past_the_threshold(pool: sqlx::PgPool) {
        let (state, doc) = setup(pool).await;
        let every = CHECK_EVERY.get();
        let below = every * (SNAPSHOT_THRESHOLD.get() / every);
        assert!(
            !should_ask(
                &state,
                &doc,
                ambre_core::Seq::<ambre_core::Relay>::new(below)
            )
            .await
        );
        let due = below + every;
        assert!(should_ask(&state, &doc, ambre_core::Seq::<ambre_core::Relay>::new(due)).await);
        assert!(
            !should_ask(
                &state,
                &doc,
                ambre_core::Seq::<ambre_core::Relay>::new(due + every)
            )
            .await
        );
        // Only a served snapshot moves the horizon.
        let covered = i64::try_from(due).unwrap();
        let sid = snapshot(&state, &doc, &ALICE, covered).await;
        assert_eq!(
            db::served_horizon(&state.db, &doc).await.unwrap(),
            ambre_core::Seq::<ambre_core::Relay>::new(0)
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
            ambre_core::Seq::<ambre_core::Relay>::new(due)
        );
    }
}
