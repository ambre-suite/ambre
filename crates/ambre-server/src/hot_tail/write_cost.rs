//! The load run of the delta index: what one delta costs the database.
//!
//! The run drives the relay's own write path of a delta, in three steps:
//!
//! 1. the insert of its index row, `db::insert_inline_update`;
//! 2. the move of the tail into a batch object, `hot_tail::move_to_batch`;
//! 3. the compaction pass that deletes the covered rows, `compact_once`.
//!
//! A fourth bucket records the snapshot each document receives every
//! `SNAPSHOT_THRESHOLD` deltas, since the delete waits on it. A delta past the
//! inline size takes `db::insert_update`. No SQL of the relay is copied here,
//! and the compaction runs every `LOAD_COMPACT_EVERY` seconds, not every hour.
//!
//! The run reads the cluster's write-ahead log (WAL) through `pg_walinspect`.
//! It tags each record with the step whose window holds it, and each block
//! reference with its relation. It also reads the block writes of
//! `pg_stat_io` over one checkpoint cycle that the run opens and closes.
//!
//! No CI job holds it: a shared runner gives no stable disk. It prints the
//! figures the budget register quotes (`ambre-spec-budgets.md section 3`),
//! and fails on none of them. Run it alone on the cluster, services up:
//!
//! ```text
//! cargo test -p ambre-server --release --features db -- --ignored --nocapture write_cost_per_delta
//! ```
//!
//! `LOAD_DOCS`, `LOAD_RATE` (deltas a second), `LOAD_SECONDS`, `LOAD_COMPACT_EVERY`
//! (seconds of the compressed compaction period), `LOAD_BUCKET` (seconds of one
//! report row) and `LOAD_SEED` change the run.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{Count, FixedBytes, PublicKey};
use futures_util::StreamExt;

use super::{INLINE_CAP, INLINE_DELTA_MAX_SIZE, move_to_batch, past_watermark};
use crate::db::{self, InlineInsert};
use crate::payload::StoredPayload;
use crate::state::AppState;
use crate::test_helpers::{
    S3Stub, create_test_document, create_test_user, offline_s3, test_app_state, with_s3,
};

fn setting(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

/// The authors who write each document. Two of them confirm each other's
/// snapshots, so every snapshot is `confirmed` and moves the trim.
const AUTHORS: usize = 3;

/// One step of a delta's life in the index, as the report names it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Step {
    Insert,
    Move,
    Delete,
    Snapshot,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Insert => "insert",
            Step::Move => "move",
            Step::Delete => "delete",
            Step::Snapshot => "snapshot",
        }
    }
}

/// A seeded xorshift: the run needs a reproducible shape, not a strong source.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }

    /// The sealed size of one delta. A delta is a few hundred bytes
    /// (`ambre-spec-sequencer.md section 3`); one in two hundred is a paste of
    /// a few kilobytes. Every size stays under the inline size.
    fn delta_size(&mut self) -> usize {
        let size = if self.below(200) == 0 {
            4096 + self.below(28 * 1024)
        } else {
            120 + self.below(780)
        };
        usize::try_from(size).unwrap_or(usize::MAX)
    }

    /// Bytes as a sealed envelope looks to the relay: no structure, so no
    /// compression of TOAST or of the WAL gains on them.
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 8);
        while out.len() < len {
            out.extend_from_slice(&self.next().to_le_bytes());
        }
        out.truncate(len);
        out
    }
}

/// One document of the run, and the numbers its next delta takes.
struct Doc {
    id: Id<Document>,
    authors: [FixedBytes<PublicKey>; AUTHORS],
    clocks: [u64; AUTHORS],
    seq: u64,
}

/// One window of the run: a step, and the WAL positions around it.
struct Mark {
    step: Step,
    start: String,
    end: String,
    bucket: u64,
}

async fn wal_position(state: &AppState) -> String {
    sqlx::query_scalar("SELECT pg_current_wal_insert_lsn()::TEXT")
        .fetch_one(&state.db)
        .await
        .expect("the WAL position")
}

/// The windows of the measured cycle. The warm-up records none.
struct Recorder {
    on: bool,
    bucket: u64,
    marks: Vec<Mark>,
}

impl Recorder {
    fn off() -> Self {
        Self {
            on: false,
            bucket: 0,
            marks: Vec::new(),
        }
    }

    async fn open(&mut self, state: &AppState, step: Step) {
        if self.on {
            let start = wal_position(state).await;
            self.marks.push(Mark {
                step,
                end: start.clone(),
                start,
                bucket: self.bucket,
            });
        }
    }

    async fn close(&mut self, state: &AppState) {
        if self.on
            && let Some(last) = self.marks.last_mut()
        {
            last.end = wal_position(state).await;
        }
    }
}

/// Writes one delta of `doc` through the relay's own steps. The insert takes
/// the branch `store_index_row` takes; a move runs at once where the relay
/// spawns it, so its window holds it alone.
async fn one_delta(state: &Arc<AppState>, doc: &mut Doc, draw: &mut Draw, rec: &mut Recorder) {
    doc.seq += 1;
    let who = usize::try_from(draw.below(AUTHORS as u64)).unwrap_or(0);
    doc.clocks[who] += 1;
    let snap = Id::<Snapshot>::mint();
    let len = draw.delta_size();
    let wire = StoredPayload::new(draw.bytes(len));
    let key = db::NewUpdate {
        document_id: &doc.id,
        author_pubkey: &doc.authors[who],
        snapshot_id: &snap,
        clock: ambre_protocol::messages::Seq::<ambre_protocol::messages::DeltaAuthor>::new(
            doc.clocks[who],
        ),
        seq: ambre_core::Seq::<ambre_core::Relay>::new(doc.seq),
    };

    rec.open(state, Step::Insert).await;
    let mut move_now = false;
    if Count::<ambre_core::Octets>::of(wire.len()) < INLINE_DELTA_MAX_SIZE {
        match db::insert_inline_update(&state.db, &key, &wire, INLINE_CAP)
            .await
            .expect("an inline insert")
        {
            InlineInsert::Stored { deltas, bytes } => move_now = past_watermark(deltas, bytes),
            InlineInsert::Full => {
                move_now = true;
                insert_own_object(state, doc, &key).await;
            }
        }
    } else {
        insert_own_object(state, doc, &key).await;
    }
    rec.close(state).await;

    if move_now {
        rec.open(state, Step::Move).await;
        move_to_batch(state, &doc.id, None)
            .await
            .expect("a move to a batch");
        rec.close(state).await;
    }

    if doc
        .seq
        .is_multiple_of(crate::compaction::SNAPSHOT_THRESHOLD.get())
    {
        rec.open(state, Step::Snapshot).await;
        let object = ambre_core::ObjectKey::new(format!("load/{}/{}", doc.id, doc.seq));
        let id = db::create_snapshot(
            &state.db,
            &doc.id,
            &doc.authors[0],
            &object,
            ambre_core::Seq::<ambre_core::Relay>::new(doc.seq),
        )
        .await
        .expect("a snapshot row");
        db::confirm_snapshot(&state.db, &doc.id, &id, &doc.authors[1])
            .await
            .expect("a confirmation");
        rec.close(state).await;
    }
}

/// The row of a delta that keeps an object of its own (`SRV-149`). The run
/// writes no object: the object store is outside what it measures.
async fn insert_own_object(state: &AppState, doc: &Doc, key: &db::NewUpdate<'_>) {
    let object = ambre_core::ObjectKey::new(format!("load/{}/delta/{}", doc.id, doc.seq));
    db::insert_update(
        &state.db,
        key.document_id,
        key.author_pubkey,
        key.snapshot_id,
        key.clock,
        key.seq,
        &object,
    )
    .await
    .expect("an index row");
}

/// One pass of the relay's hourly compaction, the step that deletes rows.
async fn compaction(state: &Arc<AppState>, rec: &mut Recorder) {
    rec.open(state, Step::Delete).await;
    crate::compact_once(state).await;
    rec.close(state).await;
}

/// The block writes of every process since the statistics began, and the
/// checkpoints run.
async fn io_counters(state: &AppState) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT COALESCE(SUM(writes), 0)::BIGINT FROM pg_stat_io), \
                (SELECT checkpoints_timed + checkpoints_req FROM pg_stat_bgwriter), \
                (SELECT buffers_checkpoint FROM pg_stat_bgwriter)",
    )
    .fetch_one(&state.db)
    .await
    .expect("the I/O statistics")
}

async fn checkpoint(state: &AppState) {
    sqlx::query("CHECKPOINT")
        .execute(&state.db)
        .await
        .expect("a checkpoint");
}

#[sqlx::test]
#[ignore = "a real-time load run of minutes; run it by hand, in release, as its rustdoc says"]
async fn write_cost_per_delta(pool: sqlx::PgPool) {
    let docs_n = setting("LOAD_DOCS", 32);
    let rate = setting("LOAD_RATE", 200).max(1);
    let seconds = setting("LOAD_SECONDS", 240);
    let compact_every = setting("LOAD_COMPACT_EVERY", 30).max(1);
    let bucket_secs = setting("LOAD_BUCKET", 30).max(1);
    let seed = setting("LOAD_SEED", 1).max(1);

    let stub = S3Stub::spawn([]).await;
    let base = test_app_state(pool.clone()).await;
    let state = with_s3(&base, offline_s3(Some(&stub.endpoint)));
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(&pool)
        .await
        .expect("pg_walinspect, a contrib module of the Postgres image");

    let owner = create_test_user(&pool, "load@ambre.app").await;
    let mut docs = Vec::new();
    for d in 0..docs_n {
        let id = create_test_document(&pool, owner, "prose").await;
        let authors = std::array::from_fn(|a| {
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&d.to_le_bytes());
            key[8] = u8::try_from(a).unwrap_or(0);
            FixedBytes::<PublicKey>::from_array(key)
        });
        docs.push(Doc {
            id,
            authors,
            clocks: [0; AUTHORS],
            seq: 0,
        });
    }

    // Warm-up: each document passes the snapshots compaction keeps, so the
    // index holds what a long-lived document holds, and each step has run.
    let warm = (crate::compaction::SNAPSHOT_THRESHOLD.get() * 11) + 250;
    let started = Instant::now();
    let docs: Vec<Doc> = futures_util::stream::iter(docs.into_iter().enumerate())
        .map(|(i, mut doc)| {
            let state = Arc::clone(&state);
            async move {
                let mut draw = Draw(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64 + 1));
                let mut rec = Recorder::off();
                for _ in 0..warm {
                    one_delta(&state, &mut doc, &mut draw, &mut rec).await;
                }
                doc
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut docs = docs;
    compaction(&state, &mut Recorder::off()).await;
    sqlx::query("VACUUM ANALYZE updates")
        .execute(&pool)
        .await
        .expect("a vacuum");
    println!(
        "warm-up: {} deltas over {docs_n} documents in {:.0} s",
        warm * docs_n,
        started.elapsed().as_secs_f64()
    );

    // The measured cycle: it opens on a checkpoint, so its first rows carry
    // the full-page images a checkpoint causes.
    checkpoint(&state).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    let (writes0, checkpoints0, buffers0) = io_counters(&state).await;
    let start_lsn = wal_position(&state).await;
    let mut rec = Recorder {
        on: true,
        bucket: 0,
        marks: Vec::new(),
    };
    let mut per_bucket: BTreeMap<u64, u64> = BTreeMap::new();
    let mut draw = Draw(seed ^ 0xD1B5_4A32_D192_ED03);
    let total = rate * seconds;
    let began = Instant::now();
    let tick = Duration::from_secs_f64(1.0 / rate as f64);
    let mut next_compaction = Duration::from_secs(compact_every);
    let mut late = 0u64;
    for i in 0..total {
        let due = tick.mul_f64(i as f64);
        let now = began.elapsed();
        if due > now {
            tokio::time::sleep(due - now).await;
        } else if now - due > tick * 10 {
            late += 1;
        }
        rec.bucket = began.elapsed().as_secs() / bucket_secs;
        let index = usize::try_from(draw.below(docs_n)).unwrap_or(0);
        one_delta(&state, &mut docs[index], &mut draw, &mut rec).await;
        *per_bucket.entry(rec.bucket).or_default() += 1;
        if began.elapsed() >= next_compaction {
            compaction(&state, &mut rec).await;
            next_compaction += Duration::from_secs(compact_every);
        }
    }
    let elapsed = began.elapsed().as_secs_f64();
    let end_lsn = wal_position(&state).await;
    let wal_bytes: i64 = sqlx::query_scalar("SELECT ($2::pg_lsn - $1::pg_lsn)::BIGINT")
        .bind(&start_lsn)
        .bind(&end_lsn)
        .fetch_one(&pool)
        .await
        .expect("a WAL distance");

    // The analysis reads the WAL before the closing checkpoint can recycle it.
    report_wal(&pool, &rec.marks, &per_bucket, &start_lsn, &end_lsn).await;

    checkpoint(&state).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    let (writes1, checkpoints1, buffers1) = io_counters(&state).await;

    let deltas = total as f64;
    println!();
    println!(
        "run: {total} deltas over {docs_n} documents in {elapsed:.0} s, {:.0} deltas/s, {late} late by over 10 ticks",
        deltas / elapsed
    );
    println!(
        "cycle: WAL {:.0} B/delta by LSN; block writes {:.2}/delta ({} blocks, pg_stat_io); checkpoint buffers {:.2}/delta; {} checkpoints",
        wal_bytes as f64 / deltas,
        (writes1 - writes0) as f64 / deltas,
        writes1 - writes0,
        (buffers1 - buffers0) as f64 / deltas,
        checkpoints1 - checkpoints0
    );
    let rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM updates), \
                pg_relation_size('updates'), pg_indexes_size('updates')",
    )
    .fetch_one(&pool)
    .await
    .expect("the size of updates");
    println!(
        "updates at the end: {} rows, heap {} KiB, indexes {} KiB",
        rows.0,
        rows.1 / 1024,
        rows.2 / 1024
    );
}

/// Tags each WAL record and each block reference of the cycle with its step.
/// It prints the cost per delta by step, by relation and by bucket of time.
async fn report_wal(
    pool: &sqlx::PgPool,
    marks: &[Mark],
    per_bucket: &BTreeMap<u64, u64>,
    start: &str,
    end: &str,
) {
    let mut conn = pool.acquire().await.expect("a connection");
    sqlx::query(
        "CREATE TEMP TABLE marks (s pg_lsn PRIMARY KEY, e pg_lsn, step TEXT, bucket BIGINT)",
    )
    .execute(&mut *conn)
    .await
    .expect("the marks table");
    for chunk in marks.chunks(10_000) {
        let s: Vec<&str> = chunk.iter().map(|m| m.start.as_str()).collect();
        let e: Vec<&str> = chunk.iter().map(|m| m.end.as_str()).collect();
        let step: Vec<&str> = chunk.iter().map(|m| m.step.label()).collect();
        let bucket: Vec<i64> = chunk
            .iter()
            .map(|m| i64::try_from(m.bucket).unwrap_or(0))
            .collect();
        // Two windows that wrote nothing share a start; the first keeps it.
        sqlx::query(
            "INSERT INTO marks SELECT s::pg_lsn, e::pg_lsn, step, bucket \
             FROM UNNEST($1::TEXT[], $2::TEXT[], $3::TEXT[], $4::BIGINT[]) AS t(s, e, step, bucket) \
             WHERE s::pg_lsn < e::pg_lsn ON CONFLICT DO NOTHING",
        )
        .bind(&s)
        .bind(&e)
        .bind(&step)
        .bind(&bucket)
        .execute(&mut *conn)
        .await
        .expect("the marks");
    }
    sqlx::query("ANALYZE marks")
        .execute(&mut *conn)
        .await
        .expect("an analyze");

    let windows: Vec<(String, i64)> =
        sqlx::query_as("SELECT step, COUNT(*) FROM marks GROUP BY step ORDER BY step")
            .fetch_all(&mut *conn)
            .await
            .expect("the windows");
    let deltas: u64 = per_bucket.values().sum();
    let deltas = deltas as f64;

    let tag = "LEFT JOIN LATERAL (SELECT m.e, m.step, m.bucket FROM marks m \
               WHERE m.s <= r.start_lsn ORDER BY m.s DESC LIMIT 1) w ON TRUE";
    let step = "CASE WHEN r.start_lsn < w.e THEN w.step ELSE 'between' END";

    let by_step: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(&format!(
        "SELECT {step} AS step, COUNT(*), SUM(r.record_length)::BIGINT, \
                SUM(r.fpi_length)::BIGINT, COUNT(*) FILTER (WHERE r.fpi_length > 0) \
         FROM pg_get_wal_records_info($1::pg_lsn, $2::pg_lsn) r {tag} \
         GROUP BY 1 ORDER BY 1"
    ))
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await
    .expect("the WAL records by step");
    println!();
    println!("WAL per delta, by step (records, bytes, of which full-page images):");
    println!(
        "{:<9} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "step", "windows", "rec/delta", "B/delta", "fpiB/delta", "fpi/delta"
    );
    for (name, records, bytes, fpi, fpis) in &by_step {
        let windows = windows
            .iter()
            .find(|(s, _)| s == name)
            .map_or(0, |(_, n)| *n);
        println!(
            "{name:<9} {windows:>8} {:>10.2} {:>10.0} {:>10.0} {:>10.3}",
            *records as f64 / deltas,
            *bytes as f64 / deltas,
            *fpi as f64 / deltas,
            *fpis as f64 / deltas
        );
    }
    let all: (i64, i64) = by_step
        .iter()
        .fold((0, 0), |acc, r| (acc.0 + r.2, acc.1 + r.3));
    println!(
        "{:<9} {:>8} {:>10} {:>10.0} {:>10.0}",
        "total",
        "",
        "",
        all.0 as f64 / deltas,
        all.1 as f64 / deltas
    );

    let by_bucket: Vec<(i64, i64, i64)> = sqlx::query_as(&format!(
        "SELECT w.bucket, SUM(r.record_length)::BIGINT, SUM(r.fpi_length)::BIGINT \
         FROM pg_get_wal_records_info($1::pg_lsn, $2::pg_lsn) r {tag} \
         GROUP BY 1 ORDER BY 1"
    ))
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await
    .expect("the WAL records by bucket");
    println!();
    println!("WAL per delta, by bucket of time since the checkpoint:");
    println!(
        "{:<7} {:>7} {:>10} {:>10}",
        "bucket", "deltas", "B/delta", "fpiB/delta"
    );
    for (bucket, bytes, fpi) in &by_bucket {
        let n = per_bucket
            .get(&u64::try_from(*bucket).unwrap_or(0))
            .copied()
            .unwrap_or(0)
            .max(1) as f64;
        println!(
            "{bucket:<7} {n:>7.0} {:>10.0} {:>10.0}",
            *bytes as f64 / n,
            *fpi as f64 / n
        );
    }

    let by_relation: Vec<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(&format!(
        "SELECT {step} AS step, \
                COALESCE(c.relname, 'other:' || r.relfilenode::TEXT) AS relation, \
                COUNT(*), SUM(r.block_data_length)::BIGINT, SUM(r.block_fpi_length)::BIGINT, \
                COUNT(*) FILTER (WHERE r.block_fpi_length > 0), \
                COUNT(DISTINCT (r.relforknumber, r.relblocknumber)) \
         FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) r {tag} \
         LEFT JOIN pg_class c ON c.relfilenode = r.relfilenode \
              AND r.reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         GROUP BY 1, 2 ORDER BY 1, 4 + 5 DESC"
    ))
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await
    .expect("the WAL blocks by relation");
    println!();
    println!("WAL block references per delta, by step and relation:");
    println!(
        "{:<9} {:<46} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "step", "relation", "refs", "dataB", "fpiB", "fpi", "blocks"
    );
    for (name, relation, refs, data, fpi, fpis, blocks) in &by_relation {
        if (*data + *fpi) as f64 / deltas < 1.0 && *refs as f64 / deltas < 0.01 {
            continue;
        }
        println!(
            "{name:<9} {relation:<46} {:>9.3} {:>9.0} {:>9.0} {:>9.3} {:>10}",
            *refs as f64 / deltas,
            *data as f64 / deltas,
            *fpi as f64 / deltas,
            *fpis as f64 / deltas,
            blocks
        );
    }

    let distinct: Vec<(String, i64)> = sqlx::query_as(
        "SELECT COALESCE(c.relname, 'other'), COUNT(DISTINCT (r.relfilenode, r.relforknumber, r.relblocknumber)) \
         FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) r \
         LEFT JOIN pg_class c ON c.relfilenode = r.relfilenode \
              AND r.reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         GROUP BY 1 ORDER BY 2 DESC",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await
    .expect("the distinct blocks");
    println!();
    println!("Distinct blocks the cycle modified, per delta (what the checkpoint writes):");
    let mut sum = 0i64;
    for (relation, blocks) in &distinct {
        sum += blocks;
        if *blocks as f64 / deltas >= 0.001 {
            println!("{relation:<46} {:>8.4} ({blocks})", *blocks as f64 / deltas);
        }
    }
    println!("{:<46} {:>8.4} ({sum})", "total", sum as f64 / deltas);
}
