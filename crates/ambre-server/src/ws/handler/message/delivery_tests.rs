//! The path of a numbered delta, driven through its failures. The register
//! refuses an index write, the queue refuses a frame, and the index breaks
//! under a replay. Reads finish out of order, and one replay is asked twice.
//! Each test reads the stores back, so it needs Postgres and Redis.

use super::hot_tail_tests::{replay, signed_update, writer};
use super::*;
use crate::test_helpers::S3Stub;
use ambre_protocol::messages::Seq;
use ambre_protocol::ws::decode_server_msg;
use rustis::commands::{GenericCommands, ListCommands, StringCommands};

/// Makes the register refuse the next `count` inserts into the delta index. A
/// sequence counts them, since it is not rolled back with the refused insert.
async fn refuse_index_writes(pool: &sqlx::PgPool, count: i64) {
    let sql = format!(
        "CREATE SEQUENCE refusals; \
         CREATE FUNCTION refuse_insert() RETURNS trigger AS $$ BEGIN \
           IF nextval('refusals') <= {count} THEN RAISE EXCEPTION 'refused by the test'; END IF; \
           RETURN NEW; \
         END $$ LANGUAGE plpgsql; \
         CREATE TRIGGER refuse BEFORE INSERT ON updates \
           FOR EACH ROW EXECUTE FUNCTION refuse_insert();"
    );
    sqlx::raw_sql(&sql).execute(pool).await.unwrap();
}

/// The dead letters of `doc_id`, each as its number, its object key and the
/// frame it carries.
async fn dead_letters(state: &AppState, doc_id: Id<Document>) -> Vec<(u64, String, Vec<u8>)> {
    use base64::Engine;
    let blobs: Vec<String> = state
        .redis
        .lrange(format!("doc:{doc_id}:deadletter"), 0, -1)
        .await
        .unwrap();
    blobs
        .iter()
        .map(|blob| {
            let entry: serde_json::Value = serde_json::from_str(blob).unwrap();
            let wire = base64::engine::general_purpose::URL_SAFE
                .decode(entry["update_b64"].as_str().unwrap())
                .unwrap();
            (
                entry["seq"].as_u64().unwrap(),
                entry["s3_key"].as_str().unwrap().to_owned(),
                wire,
            )
        })
        .collect()
}

/// Drops the keys a test left for `doc_id` in the shared ephemeral store.
async fn forget(state: &AppState, doc_id: Id<Document>) {
    let _: usize = state
        .redis
        .del(crate::redis_keys::DocumentStateKeys::of(&doc_id).texts())
        .await
        .unwrap();
}

/// The deltas of the one page a replay sent, in the order sent.
fn replayed(frames: &[Vec<u8>]) -> Vec<ambre_protocol::SeqUpdate> {
    match frames
        .iter()
        .map(|f| decode_server_msg(&ambre_core::Payload::new(f.clone())).unwrap())
        .collect::<Vec<_>>()
        .as_slice()
    {
        [WsServerMsg::MissingUpdates { updates, .. }] => updates.clone(),
        other => panic!("expected one page, got {other:?}"),
    }
}

/// `SRV-033`: an index write the register refuses twice is tried again, and
/// the row lands. Nothing is dead-lettered.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_refused_index_write_is_retried_until_the_row_lands(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[51u8; 32]);
    let (state, mut conn, doc_id) = writer(pool.clone(), &stub, &key).await;
    refuse_index_writes(&pool, 2).await;
    let (tx, _rx) = mpsc::channel::<Payload>(8);

    handle_update(&state, &mut conn, signed_update(&key, doc_id, 1, 16), &tx).await;

    let rows = db::get_updates_after(
        &state.db,
        &doc_id,
        ambre_core::Seq::new(0),
        crate::db::QueryLimit::saturating(10),
    )
    .await
    .unwrap();
    assert!(matches!(rows.as_slice(), [r] if r.seq == ambre_core::Seq::new(1) && r.wire.is_some()));
    assert!(dead_letters(&state, doc_id).await.is_empty());
    forget(&state, doc_id).await;
}

/// `SRV-082`: an index write refused past every retry leaves the whole signed
/// frame in the dead letters, with its number and its object key. An operator
/// replaying it holds what the author sent, signature included.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn an_index_write_that_never_lands_leaves_the_whole_frame(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[52u8; 32]);
    let (state, mut conn, doc_id) = writer(pool.clone(), &stub, &key).await;
    refuse_index_writes(&pool, 1_000).await;
    let (tx, _rx) = mpsc::channel::<Payload>(8);
    let update = signed_update(&key, doc_id, 1, 16);

    handle_update(&state, &mut conn, update.clone(), &tx).await;

    assert!(
        db::get_updates_after(
            &state.db,
            &doc_id,
            ambre_core::Seq::new(0),
            crate::db::QueryLimit::saturating(10)
        )
        .await
        .unwrap()
        .is_empty()
    );
    let object = crate::s3::format_s3_delta_key(
        &state.config.s3_env_prefix,
        &doc_id,
        ambre_core::Seq::new(1),
    );
    match dead_letters(&state, doc_id).await.as_slice() {
        [(1, s3_key, wire)] => {
            assert_eq!(s3_key, object.as_str());
            assert_eq!(wire, &postcard::to_allocvec(&update).unwrap());
            let kept: ambre_protocol::Update = postcard::from_bytes(wire).unwrap();
            assert!(verify_update_signature(&kept.public_data.pub_key, &kept).is_ok());
        }
        other => panic!("expected one dead letter, got {other:?}"),
    }
    forget(&state, doc_id).await;
}

/// `SRV-034`: a large delta whose queue refuses it keeps its frame in its
/// index row, so a replay still delivers it. The failure is dead-lettered too.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_refused_queue_leaves_the_frame_in_its_row(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[53u8; 32]);
    let (state, mut conn, doc_id) = writer(pool, &stub, &key).await;
    // A pending key that is not a list refuses every push.
    let () = state
        .redis
        .set(format!("doc:{doc_id}:pending"), "not a list")
        .await
        .unwrap();
    let (tx, _rx) = mpsc::channel::<Payload>(8);
    let large = crate::hot_tail::INLINE_DELTA_MAX_SIZE.as_len() + 1024;
    let update = signed_update(&key, doc_id, 1, large);

    handle_update(&state, &mut conn, update.clone(), &tx).await;

    let rows = db::get_updates_after(
        &state.db,
        &doc_id,
        ambre_core::Seq::new(0),
        crate::db::QueryLimit::saturating(10),
    )
    .await
    .unwrap();
    assert!(matches!(rows.as_slice(), [r] if r.wire.is_some() && r.s3_key.is_none()));
    let sent = replayed(&replay(&state, &mut conn, doc_id).await);
    assert!(matches!(sent.as_slice(), [d] if d.seq.get() == 1 && d.update == update));
    assert_eq!(dead_letters(&state, doc_id).await.len(), 1);
    forget(&state, doc_id).await;
}

/// `SRV-040`, `SRV-184`: an index that cannot be read under a replay still
/// answers it. The stop names the cursor the replay started past, since no
/// page was delivered.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_replay_over_a_broken_index_is_still_answered(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[54u8; 32]);
    let (state, mut conn, doc_id) = writer(pool.clone(), &stub, &key).await;
    sqlx::raw_sql("ALTER TABLE updates RENAME TO updates_away")
        .execute(&pool)
        .await
        .unwrap();

    let frames = replay(&state, &mut conn, doc_id).await;
    let sent: Vec<_> = frames
        .iter()
        .map(|f| decode_server_msg(&ambre_core::Payload::new(f.clone())).unwrap())
        .collect();
    assert!(
        matches!(
            sent.as_slice(),
            [WsServerMsg::ReplayStopped { document_id, after_seq }] if *document_id == doc_id && after_seq.get() == 0
        ),
        "sent {sent:?}"
    );
    forget(&state, doc_id).await;
}

/// `SRV-184`: a delta whose object is lost stops the replay behind the deltas
/// before it, and the stop names the last of them.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_replay_stopped_at_a_lost_delta_names_the_last_one_delivered(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[56u8; 32]);
    let (state, mut conn, doc_id) = writer(pool.clone(), &stub, &key).await;
    let (tx, _rx) = mpsc::channel::<Payload>(8);
    for clock in 1..=3 {
        handle_update(
            &state,
            &mut conn,
            signed_update(&key, doc_id, clock, 16),
            &tx,
        )
        .await;
    }
    // Delta 2 lives in an object the store no longer holds.
    sqlx::query(
        "UPDATE updates SET wire = NULL, s3_key = 'lost' WHERE document_id = $1 AND seq = 2",
    )
    .bind(uuid::Uuid::from_bytes(*doc_id.uuid().as_bytes()))
    .execute(&pool)
    .await
    .unwrap();

    let frames = replay(&state, &mut conn, doc_id).await;
    let sent: Vec<_> = frames
        .iter()
        .map(|f| decode_server_msg(&ambre_core::Payload::new(f.clone())).unwrap())
        .collect();
    match sent.as_slice() {
        [
            WsServerMsg::MissingUpdates { updates, .. },
            WsServerMsg::ReplayStopped {
                document_id,
                after_seq,
            },
        ] => {
            let seqs: Vec<u64> = updates.iter().map(|u| u.seq.get()).collect();
            assert_eq!(seqs, [1], "delta 3 waits behind the lost one");
            assert_eq!((*document_id, *after_seq), (doc_id, Seq::new(1)));
        }
        other => panic!("expected a page then a stop, got {other:?}"),
    }
    forget(&state, doc_id).await;
}

/// `SRV-084`: the reads of one page are issued together and finish in reverse
/// order, and the page still carries its deltas by number.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_page_read_out_of_order_is_sent_in_order(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[55u8; 32]);
    let (state, mut conn, doc_id) = writer(pool, &stub, &key).await;
    let (tx, _rx) = mpsc::channel::<Payload>(8);
    let large = crate::hot_tail::INLINE_DELTA_MAX_SIZE.as_len() + 1024;
    for clock in 1..=3 {
        handle_update(
            &state,
            &mut conn,
            signed_update(&key, doc_id, clock, large),
            &tx,
        )
        .await;
        // The first delta's read is held longest, the last one's not at all.
        let object = crate::s3::format_s3_delta_key(
            &state.config.s3_env_prefix,
            &doc_id,
            ambre_core::Seq::new(clock),
        );
        stub.delay(
            object.into_inner(),
            Duration::from_millis(100 * (3 - clock)),
        );
    }

    let sent = replayed(&replay(&state, &mut conn, doc_id).await);
    let seqs: Vec<u64> = sent.iter().map(|d| d.seq.get()).collect();
    assert_eq!(seqs, [1, 2, 3]);
    let first = crate::s3::format_s3_delta_key(
        &state.config.s3_env_prefix,
        &doc_id,
        ambre_core::Seq::new(1),
    );
    assert_eq!(
        stub.gets(first.as_str()),
        1,
        "each delta was asked of the store"
    );
    forget(&state, doc_id).await;
}

/// `SRV-086`: a second replay of a document inside the throttle window is
/// refused, and nothing is sent for it.
#[sqlx::test]
#[cfg_attr(
    not(feature = "db"),
    ignore = "needs postgres+redis; run with --features db"
)]
async fn a_second_replay_inside_the_window_is_answered_with_nothing(pool: sqlx::PgPool) {
    let stub = S3Stub::spawn([]).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[56u8; 32]);
    let (state, mut conn, doc_id) = writer(pool, &stub, &key).await;
    let (tx, mut rx) = mpsc::channel::<Payload>(8);
    handle_update(&state, &mut conn, signed_update(&key, doc_id, 1, 16), &tx).await;
    while rx.try_recv().is_ok() {}

    conn.last_replay_at.clear();
    handle_request_missing(&state, &mut conn, doc_id, Seq::new(0), &tx).await;
    assert!(rx.try_recv().is_ok(), "the first replay is served");
    handle_request_missing(&state, &mut conn, doc_id, Seq::new(0), &tx).await;
    assert!(rx.try_recv().is_err(), "the second one sends nothing");
    forget(&state, doc_id).await;
}
