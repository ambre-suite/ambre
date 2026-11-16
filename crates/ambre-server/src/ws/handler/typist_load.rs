//! The load run of the write path: what N typists at five keys a second cost
//! the relay and its database tier.
//!
//! Each typist is one session. It holds a `WsConnectionState` and a signing
//! key of its own, and a task that calls `handle_update` as the socket loop does.
//! Every key is one delta, since the send window of 25 ms groups no key at five
//! keys a second (`ambre-spec-budgets.md section 8`). So every key takes the
//! whole path: the signature check, the ordering script, the index row, the
//! hot-tail move it may start, and the broadcast. No step is copied here.
//!
//! One reader per document subscribes through the relay's own PubSub hub. Each
//! ciphertext opens on the instant its key left the typist. The relay never
//! reads it, so the reader measures the transit to a peer's socket.
//!
//! The run is by hand, alone on the machine, services up:
//!
//! ```text
//! LOAD_TYPISTS=200 cargo test -p ambre-server --release --features db,test-helpers \
//!     -- --ignored --nocapture typist_load
//! ```
//!
//! These variables change the run:
//!
//! | Variable | Meaning | Default |
//! | :--- | :--- | :--- |
//! | `LOAD_TYPISTS` | typists | 100 |
//! | `LOAD_PER_DOC` | typists per document | 4 |
//! | `LOAD_KEYS` | keys a second per typist | 5 |
//! | `LOAD_SECONDS` | seconds of the counted cycle | 60 |
//! | `LOAD_WARM` | seconds of warm-up, sent but not counted | 10 |
//! | `LOAD_POOL` | connections of the relay's pool, as `main.rs` | 20 |
//! | `LOAD_SEED` | seed of the phases and the sizes | 1 |
//!
//! The run prints the figures of the budget row and fails on none of them. A
//! shared runner gives no stable processor, so no CI job runs it.
//!
//! The processor time of Postgres is read from `/proc`, which shows the
//! processes of a container on a Linux host. Elsewhere it prints nothing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{Ciphertext, CommitmentTag, FixedBytes, Nonce, PublicKey, Signature};
use ambre_protocol::Update;
use ambre_protocol::messages::{DeltaAuthor, Seq};
use ambre_protocol::public_data::UpdatePublicData;
use rustis::commands::ServerCommands;
use sqlx::postgres::PgPoolOptions;

use super::*;
use crate::payload::StoredPayload;
use crate::test_helpers::{
    S3Stub, create_test_document, create_test_user, offline_s3, test_app_state, with_s3,
};

fn setting(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
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
}

/// The bytes of the instant a key left its typist, in microseconds since the
/// run began: the head of a ciphertext the relay never opens.
const STAMP: usize = 8;

/// One key: a delta of one character, sealed and signed as the Sync worker
/// sends it. A sealed delta of one character is a few hundred bytes, mostly the
/// Loro header and the seal (`ambre-spec-sequencer.md section 3`).
fn key_update(
    key: &ed25519_dalek::SigningKey,
    doc: Id<Document>,
    clock: u64,
    len: usize,
    stamp: u64,
) -> Update {
    use ed25519_dalek::Signer;
    let public_data = UpdatePublicData {
        document_id: doc,
        pub_key: FixedBytes::<PublicKey>::from_array(key.verifying_key().to_bytes()),
        key_id: FixedBytes::<ambre_core::KeyId>::from_array([0u8; 16]),
        snapshot_id: Id::<Snapshot>::from_bytes([0u8; 16]),
        clock: Seq::<DeltaAuthor>::new(clock),
        log_head: ambre_protocol::LogHead::EMPTY,
        log_entry: None,
    };
    let mut ciphertext = vec![0xA5u8; len.max(STAMP)];
    ciphertext[..STAMP].copy_from_slice(&stamp.to_le_bytes());
    let nonce = FixedBytes::<Nonce>::from_array([5u8; 24]);
    let commitment_tag = FixedBytes::<CommitmentTag>::from_array([6u8; 32]);
    let mut signed = Vec::new();
    push_framed(&mut signed, nonce.as_slice());
    push_framed(&mut signed, &ciphertext);
    push_framed(&mut signed, &postcard::to_allocvec(&public_data).unwrap());
    push_framed(&mut signed, commitment_tag.as_slice());
    Update {
        ciphertext: ambre_core::Payload::<Ciphertext>::new(ciphertext),
        nonce,
        public_data,
        commitment_tag,
        signature: FixedBytes::<Signature>::from_array(key.sign(&signed).to_bytes()),
    }
}

/// What one typist measured over the counted cycle.
#[derive(Default)]
struct Tally {
    /// Microseconds of each `handle_update`, from the frame to the broadcast.
    handle_us: Vec<u32>,
    /// Keys that left more than one period behind their due instant.
    late: u64,
    /// Answers other than a compaction request: a refusal is a defect of the run.
    refused: u64,
}

/// What one reader measured over the counted cycle.
#[derive(Default)]
struct Heard {
    /// Microseconds from the key leaving its typist to the reader's socket.
    transit_us: Vec<u32>,
}

fn percentile(sorted: &[u32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f64 * p).round() as usize;
    f64::from(sorted[rank.min(sorted.len() - 1)]) / 1000.0
}

fn report(name: &str, mut us: Vec<u32>) {
    us.sort_unstable();
    println!(
        "{name:<22} n={:<8} p50 {:>7.2} ms  p95 {:>7.2} ms  p99 {:>7.2} ms  p99.9 {:>7.2} ms  max {:>8.2} ms",
        us.len(),
        percentile(&us, 0.50),
        percentile(&us, 0.95),
        percentile(&us, 0.99),
        percentile(&us, 0.999),
        percentile(&us, 1.0),
    );
}

/// The processor time of this process, in clock ticks (`/proc/self/stat`).
fn own_ticks() -> Option<u64> {
    ticks_of(&std::fs::read_to_string("/proc/self/stat").ok()?)
}

/// The user and system ticks of one `/proc/<pid>/stat` line.
fn ticks_of(stat: &str) -> Option<u64> {
    let tail = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// The processor time of every `postgres` process this host shows, in ticks.
fn postgres_ticks() -> Option<u64> {
    let mut sum = 0;
    let mut seen = false;
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let path = entry.path();
        let Ok(comm) = std::fs::read_to_string(path.join("comm")) else {
            continue;
        };
        if comm.trim() != "postgres" {
            continue;
        }
        if let Some(t) = std::fs::read_to_string(path.join("stat"))
            .ok()
            .and_then(|s| ticks_of(&s))
        {
            sum += t;
            seen = true;
        }
    }
    seen.then_some(sum)
}

/// The processor seconds Redis reports of itself, and its commands processed.
async fn redis_counters(state: &AppState) -> (f64, u64) {
    let cpu: String = state
        .redis
        .info([rustis::commands::InfoSection::Cpu])
        .await
        .unwrap();
    let stats: String = state
        .redis
        .info([rustis::commands::InfoSection::Stats])
        .await
        .unwrap();
    let field = |text: &str, name: &str| -> f64 {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.0)
    };
    (
        field(&cpu, "used_cpu_sys") + field(&cpu, "used_cpu_user"),
        field(&stats, "total_commands_processed") as u64,
    )
}

/// Committed transactions and the WAL position of the cluster.
async fn pg_counters(pool: &sqlx::PgPool) -> (i64, String) {
    sqlx::query_as(
        "SELECT (SELECT SUM(xact_commit)::BIGINT FROM pg_stat_database), \
                pg_current_wal_insert_lsn()::TEXT",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The load run of the budget register, by hand. Its rustdoc is the module's.
#[sqlx::test]
#[ignore = "a real-time load run of a minute or more; run it by hand, in release, as its rustdoc says"]
async fn typist_load(pool: sqlx::PgPool) {
    // `#[sqlx::test]` runs on a runtime of one thread. The relay runs on the
    // runtime of `#[tokio::main]`, a worker per core, so the run builds that
    // runtime. Its pools connect there, since a connection keeps its runtime.
    let options = (*pool.connect_options()).clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(load(options));
    })
    .await
    .unwrap();
    drop(pool);
}

/// The run itself, on the database `options` names.
async fn load(options: sqlx::postgres::PgConnectOptions) {
    let typists = setting("LOAD_TYPISTS", 100).max(1);
    let per_doc = setting("LOAD_PER_DOC", 4).max(1);
    let keys = setting("LOAD_KEYS", 5).max(1);
    let seconds = setting("LOAD_SECONDS", 60).max(1);
    let warm = setting("LOAD_WARM", 10);
    let seed = setting("LOAD_SEED", 1).max(1);
    let pool_size = u32::try_from(setting("LOAD_POOL", 20)).unwrap();

    // The relay's own pool size, on the database `#[sqlx::test]` made. A
    // second pool sets the run up and reads the counters, off the relay's.
    let relay_pool = PgPoolOptions::new()
        .max_connections(pool_size)
        .connect_with(options.clone())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    let stub = S3Stub::spawn([]).await;
    let base = test_app_state(relay_pool.clone()).await;
    let state = with_s3(&base, offline_s3(Some(&stub.endpoint)));

    let docs_n = typists.div_ceil(per_doc);
    let owner = create_test_user(&pool, "owner@load.ambre.app").await;
    let mut docs = Vec::new();
    for _ in 0..docs_n {
        docs.push(create_test_document(&pool, owner, "prose").await);
    }

    // The clock of the run: every stamp and every due instant reads it. It
    // starts once every session exists, so no typist starts behind.
    let (go, started) = tokio::sync::watch::channel(None::<Instant>);
    let began_of = |mut started: tokio::sync::watch::Receiver<Option<Instant>>| async move {
        let began = *started.wait_for(Option::is_some).await.unwrap();
        began.unwrap()
    };
    let warm_end = Duration::from_secs(warm);
    let end = warm_end + Duration::from_secs(seconds);

    // One reader per document, through the relay's PubSub hub.
    let mut readers = Vec::new();
    for (d, doc) in docs.iter().enumerate() {
        let (tx, mut rx) = mpsc::channel::<StoredPayload>(4096);
        state
            .pubsub
            .subscribe(
                Topic::document(doc),
                ConnectionId::named(&format!("reader-{d}")),
                tx,
            )
            .await
            .unwrap();
        let began = began_of(started.clone());
        readers.push(tokio::spawn(async move {
            let began = began.await;
            let mut heard = Heard::default();
            let deadline = tokio::time::Instant::from_std(began + end + Duration::from_secs(2));
            while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                let at = began.elapsed();
                let decoded = ambre_protocol::ws::decode_server_msg(&ambre_core::Payload::new(
                    frame.expose_bytes().to_vec(),
                ));
                let Ok(WsServerMsg::Update { update, .. }) = decoded else {
                    continue;
                };
                let mut stamp = [0u8; STAMP];
                stamp.copy_from_slice(&update.ciphertext.as_bytes()[..STAMP]);
                let sent = Duration::from_micros(u64::from_le_bytes(stamp));
                if sent >= warm_end {
                    let us = at.saturating_sub(sent).as_micros();
                    heard.transit_us.push(u32::try_from(us).unwrap_or(u32::MAX));
                }
            }
            heard
        }));
    }

    // One task per typist, as one socket loop each.
    let period = Duration::from_secs_f64(1.0 / keys as f64);
    let mut draw = Draw(seed);
    let mut writers = Vec::new();
    for t in 0..typists {
        let d = usize::try_from(t / per_doc).unwrap();
        let doc = docs[d];
        let user = create_test_user(&pool, &format!("typist-{t}@load.ambre.app")).await;
        db::add_document_member(
            &pool,
            &doc,
            &db::NewMember {
                user_id: &user,
                role: ambre_core::Role::Editor,
                doc_key_enc: &ambre_core::Encoded::new([5u8; 32].to_vec()),
                invited_by: &owner,
            },
            None,
        )
        .await
        .unwrap();
        let mut secret = [0u8; 32];
        secret[..8].copy_from_slice(&(t + 1).to_le_bytes());
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        let mut conn = WsConnectionState {
            user_id: user,
            signing_pubkey: FixedBytes::<PublicKey>::from_array(key.verifying_key().to_bytes()),
            doc_subscriptions: HashSet::from([doc]),
            doc_access_verified_at: HashMap::from([(doc, Instant::now())]),
            doc_write: HashMap::from([(doc, DocWrite::Allowed)]),
            last_replay_at: HashMap::new(),
            msg_bucket: TokenBucket::of(1000, 1000),
            clock_fences: ClockFences::default(),
            doc_channels: HashMap::new(),
        };
        // A random phase, so the typists do not press in step.
        let phase = Duration::from_micros(draw.below(u64::try_from(period.as_micros()).unwrap()));
        let mut sizes = Draw(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (t + 1));
        let state = Arc::clone(&state);
        let began = began_of(started.clone());
        writers.push(tokio::spawn(async move {
            let began = began.await;
            let (tx, mut rx) = mpsc::channel::<StoredPayload>(64);
            let mut tally = Tally::default();
            let mut clock = 0u64;
            let mut due = phase;
            while due < end {
                let now = began.elapsed();
                if due > now {
                    tokio::time::sleep(due - now).await;
                }
                let left = began.elapsed();
                clock += 1;
                let len = usize::try_from(160 + sizes.below(160)).unwrap();
                let stamp = u64::try_from(left.as_micros()).unwrap();
                let update = key_update(&key, doc, clock, len, stamp);
                let start = Instant::now();
                handle_update(&state, &mut conn, update, &tx).await;
                let took = start.elapsed();
                while let Ok(answer) = rx.try_recv() {
                    let decoded = ambre_protocol::ws::decode_server_msg(&ambre_core::Payload::new(
                        answer.expose_bytes().to_vec(),
                    ));
                    if !matches!(decoded, Ok(WsServerMsg::SnapshotRequired { .. })) {
                        tally.refused += 1;
                    }
                }
                if left >= warm_end {
                    let us = u32::try_from(took.as_micros()).unwrap_or(u32::MAX);
                    tally.handle_us.push(us);
                    if left > due + period {
                        tally.late += 1;
                    }
                }
                due += period;
            }
            tally
        }));
    }
    println!(
        "set-up: {typists} typists over {docs_n} documents, {keys} keys a second each, pool {pool_size}"
    );
    let began = Instant::now();
    go.send(Some(began)).unwrap();

    // The counters open with the counted cycle.
    let now = began.elapsed();
    if warm_end > now {
        tokio::time::sleep(warm_end - now).await;
    }
    let ticks0 = own_ticks();
    let pg0 = postgres_ticks();
    let (redis_cpu0, redis_cmds0) = redis_counters(&state).await;
    let (xact0, lsn0) = pg_counters(&pool).await;
    let cycle = Instant::now();

    let mut handle_us = Vec::new();
    let mut late = 0;
    let mut refused = 0;
    for w in writers {
        let tally = w.await.unwrap();
        handle_us.extend(tally.handle_us);
        late += tally.late;
        refused += tally.refused;
    }
    let elapsed = cycle.elapsed().as_secs_f64();
    let ticks1 = own_ticks();
    let pg1 = postgres_ticks();
    let (redis_cpu1, redis_cmds1) = redis_counters(&state).await;
    let (xact1, lsn1) = pg_counters(&pool).await;
    let wal: i64 = sqlx::query_scalar("SELECT ($2::pg_lsn - $1::pg_lsn)::BIGINT")
        .bind(&lsn0)
        .bind(&lsn1)
        .fetch_one(&pool)
        .await
        .unwrap();

    let mut transit_us = Vec::new();
    for r in readers {
        if let Ok(Ok(heard)) = tokio::time::timeout(Duration::from_secs(5), r).await {
            transit_us.extend(heard.transit_us);
        }
    }

    let counted = handle_us.len() as f64;
    let expected = (typists * keys * seconds) as f64;
    // The kernel counts processor time at 100 ticks a second on Linux.
    let cpu = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => format!("{:.2} vCPU", (b.saturating_sub(a)) as f64 / 100.0 / elapsed),
        _ => "not readable".to_owned(),
    };
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM updates")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tail: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM updates WHERE wire IS NOT NULL")
        .fetch_one(&pool)
        .await
        .unwrap();

    println!();
    println!(
        "cycle: {elapsed:.1} s, {counted:.0} keys counted of {expected:.0} due ({:.0} keys/s), {late} late by over one period, {refused} refused",
        counted / elapsed
    );
    report("handle_update", handle_us);
    report("transit to a reader", transit_us);
    println!("relay process          {}", cpu(ticks0, ticks1));
    println!("postgres processes     {}", cpu(pg0, pg1));
    println!(
        "redis                  {:.2} vCPU, {:.0} commands/s ({:.1} per key)",
        (redis_cpu1 - redis_cpu0) / elapsed,
        (redis_cmds1 - redis_cmds0) as f64 / elapsed,
        (redis_cmds1 - redis_cmds0) as f64 / counted.max(1.0)
    );
    println!(
        "postgres               {:.0} commits/s ({:.2} per key), WAL {:.0} KiB/s ({:.0} B per key)",
        (xact1 - xact0) as f64 / elapsed,
        (xact1 - xact0) as f64 / counted.max(1.0),
        wal as f64 / 1024.0 / elapsed,
        wal as f64 / counted.max(1.0)
    );
    println!("updates at the end: {rows} rows, {tail} in the inline tail");

    // The read each inline insert makes of its document's tail, alone: the
    // count and the bytes `db::insert_inline_update` aggregates before it
    // writes. The longest tail of the run shows the dearest key.
    let longest: (uuid::Uuid, i64) = sqlx::query_as(
        "SELECT document_id, COUNT(*) FROM updates WHERE wire IS NOT NULL \
         GROUP BY 1 ORDER BY 2 DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut times = Vec::new();
    for _ in 0..200 {
        let plan: Vec<String> = sqlx::query_scalar(
            "EXPLAIN (ANALYZE, TIMING OFF, FORMAT TEXT) \
             SELECT COUNT(*), COALESCE(SUM(octet_length(wire)), 0)::BIGINT \
             FROM updates WHERE document_id = $1 AND wire IS NOT NULL",
        )
        .bind(longest.0)
        .fetch_all(&pool)
        .await
        .unwrap();
        if let Some(ms) = plan.iter().find_map(|l| {
            l.trim()
                .strip_prefix("Execution Time: ")?
                .strip_suffix(" ms")?
                .parse::<f64>()
                .ok()
        }) {
            times.push(ms);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "tail read of {} rows: median {:.3} ms over {} runs",
        longest.1,
        times.get(times.len() / 2).copied().unwrap_or(0.0),
        times.len()
    );
}
