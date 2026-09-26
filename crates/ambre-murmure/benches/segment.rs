//! The segment benches: what an active segment costs as its history grows.
//!
//! A segment is built in process, one delta per commit as a client sends it.
//! Most deltas are messages, and a few are reactions or edits. No fixture file
//! is read.
//! A bench reports a number, and `ambre-spec-budgets.md` says what it must stay
//! under.
//!
//! **Run these with `cargo bench -p ambre-murmure --bench segment`.** No CI job
//! runs them: `criterion` measures wall clock, so a figure means nothing
//! without the machine it came from.
//!
//! The sizes are printed once per corpus, on standard error, because `criterion`
//! measures time and not bytes.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use loro::{ExportMode, JsonSchema, LoroDoc, LoroList, LoroMap, LoroText, VersionVector};

/// The message counts of an active segment. 5000 is the close threshold of
/// `ambre-spec-budgets.md` section 6.
const SIZES: [usize; 3] = [500, 2000, 5000];

/// The writers of the channel, one Loro peer each.
const WRITERS: u64 = 20;

/// One synthetic segment: its document, and every delta in the order sent.
struct Segment {
    doc: LoroDoc,
    deltas: Vec<Vec<u8>>,
}

/// A body of 40 to 200 bytes, the length of a chat line.
fn body(i: usize) -> String {
    let words = [
        "ship",
        "the",
        "review",
        "today",
        "après",
        "le",
        "déploiement",
        "ok",
        "merci",
    ];
    let len = 6 + (i * 7919) % 30;
    (0..len)
        .map(|w| words[(i + w * 31) % words.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Commit the pending operations of `doc` and keep them as one delta: every
/// operation after `sent`, the version the previous delta ended on.
fn commit_delta(doc: &LoroDoc, sent: &mut VersionVector, deltas: &mut Vec<Vec<u8>>) {
    doc.commit();
    deltas.push(
        doc.export(ExportMode::updates(sent))
            .expect("export updates"),
    );
    *sent = doc.oplog_vv();
}

/// Build a segment of `count` messages. One message in five draws a reaction
/// and one in twenty an edit, each its own delta, as each is its own gesture.
fn build(count: usize) -> Segment {
    let doc = LoroDoc::new();
    let messages: LoroList = doc.get_list("messages");
    let mut deltas = Vec::with_capacity(count + count / 4);
    let mut sent = VersionVector::default();
    for i in 0..count {
        doc.set_peer_id(1 + (i as u64 * 7) % WRITERS)
            .expect("peer id");
        let message = messages
            .insert_container(i, LoroMap::new())
            .expect("message");
        message.insert("id", format!("m{i:016x}")).expect("id");
        message
            .insert("author", format!("u{:08x}", i % WRITERS as usize))
            .expect("author");
        message
            .insert("ts", 1_700_000_000_000_i64 + i as i64 * 30_000)
            .expect("ts");
        let content = message
            .insert_container("content", LoroText::new())
            .expect("content");
        content.insert(0, &body(i)).expect("body");
        commit_delta(&doc, &mut sent, &mut deltas);

        if i % 5 == 4 {
            doc.set_peer_id(1 + (i as u64 * 3) % WRITERS)
                .expect("peer id");
            let reactions = message
                .insert_container("reactions", LoroMap::new())
                .expect("reactions");
            let users = reactions
                .insert_container("\u{1F44D}", LoroMap::new())
                .expect("emoji");
            users
                .insert(
                    "u00000003",
                    1_700_000_000_000_i64 + i as i64 * 30_000 + 5_000,
                )
                .expect("reaction");
            commit_delta(&doc, &mut sent, &mut deltas);
        }
        if i % 20 == 19 {
            doc.set_peer_id(1 + (i as u64 * 7) % WRITERS)
                .expect("peer id");
            content.insert(0, "edit: ").expect("edit");
            message.insert("edited", true).expect("edited");
            commit_delta(&doc, &mut sent, &mut deltas);
        }
    }
    let segment = Segment { doc, deltas };
    assert_replays(&segment, count);
    segment
}

/// Stop when the deltas do not rebuild the segment: a bench over empty deltas
/// reports a figure that reads as an improvement.
fn assert_replays(segment: &Segment, count: usize) {
    let doc = LoroDoc::new();
    for delta in &segment.deltas {
        doc.import(delta).expect("import");
    }
    assert_eq!(
        doc.get_list("messages").len(),
        count,
        "the deltas miss messages"
    );
    assert_eq!(
        doc.get_deep_value(),
        segment.doc.get_deep_value(),
        "the deltas diverge"
    );
}

/// Report the byte sizes once per corpus: the deltas the relay stores, and the
/// two snapshots a compaction could write in their place.
fn report_sizes(count: usize, segment: &Segment) {
    let delta_bytes: usize = segment.deltas.iter().map(Vec::len).sum();
    let full = segment
        .doc
        .export(ExportMode::Snapshot)
        .expect("snapshot")
        .len();
    let shallow = segment
        .doc
        .export(ExportMode::shallow_snapshot(&segment.doc.state_frontiers()))
        .expect("shallow")
        .len();
    eprintln!(
        "segment {count}: {} deltas, {delta_bytes} delta bytes ({} avg), \
         snapshot {full} bytes, shallow snapshot {shallow} bytes",
        segment.deltas.len(),
        delta_bytes / segment.deltas.len(),
    );
}

/// A cold open with no snapshot: every delta imported one by one, as the
/// replay pages deliver them.
fn bench_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_replay_one_by_one");
    group.sample_size(10);
    for count in SIZES {
        let segment = build(count);
        report_sizes(count, &segment);
        group.bench_function(count.to_string(), |b| {
            b.iter(|| {
                let doc = LoroDoc::new();
                for delta in &segment.deltas {
                    doc.import(delta).expect("import");
                }
                black_box(doc)
            });
        });
    }
    group.finish();
}

/// A cold open with no snapshot, the deltas imported in one batch.
fn bench_replay_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_replay_batch");
    group.sample_size(10);
    for count in SIZES {
        let segment = build(count);
        group.bench_function(count.to_string(), |b| {
            b.iter(|| {
                let doc = LoroDoc::new();
                doc.import_batch(&segment.deltas).expect("import batch");
                black_box(doc)
            });
        });
    }
    group.finish();
}

/// A cold open from a snapshot: the path a compaction buys.
fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_snapshot");
    group.sample_size(10);
    for count in SIZES {
        let segment = build(count);
        let full = segment.doc.export(ExportMode::Snapshot).expect("snapshot");
        group.bench_function(format!("export/{count}"), |b| {
            b.iter(|| black_box(segment.doc.export(ExportMode::Snapshot).expect("snapshot")));
        });
        group.bench_function(format!("import/{count}"), |b| {
            b.iter(|| {
                let doc = LoroDoc::new();
                doc.import(&full).expect("import snapshot");
                black_box(doc)
            });
        });
    }
    group.finish();
}

/// The authorship check with no fork. The last delta travels as a Loro JSON
/// update, so the receiver reads its operations with no document. It then
/// imports exactly what it read.
fn bench_json_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_json_check");
    group.sample_size(20);
    for count in SIZES {
        let segment = build(count);
        let (last, head) = segment.deltas.split_last().expect("one delta at least");
        let doc = LoroDoc::new();
        doc.import_batch(head).expect("import head");
        let before = doc.oplog_vv();
        let probe = LoroDoc::new();
        probe.import_batch(&segment.deltas).expect("import all");
        let json = serde_json::to_vec(&probe.export_json_updates(&before, &probe.oplog_vv()))
            .expect("json");
        if count == SIZES[SIZES.len() - 1] {
            eprintln!(
                "last delta: {} bytes binary, {} bytes JSON",
                last.len(),
                json.len()
            );
        }
        group.bench_function(format!("decode_and_scan/{count}"), |b| {
            b.iter(|| {
                let schema: JsonSchema = serde_json::from_slice(black_box(&json)).expect("decode");
                let ops: usize = schema.changes.iter().map(|change| change.ops.len()).sum();
                black_box((schema, ops))
            });
        });
        group.bench_function(format!("import_json/{count}"), |b| {
            b.iter_batched(
                || {
                    (
                        doc.fork(),
                        serde_json::from_slice::<JsonSchema>(&json).expect("decode"),
                    )
                },
                |(target, schema)| {
                    target.import_json_updates(schema).expect("import json");
                    black_box(target)
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("import_binary/{count}"), |b| {
            b.iter_batched(
                || doc.fork(),
                |target| {
                    target.import(last).expect("import");
                    black_box(target)
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// The authorship check: one received delta imported into a fork of its
/// segment (`ambre-spec-op-authorship.md`).
fn bench_fork_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("segment_fork_check");
    group.sample_size(20);
    for count in SIZES {
        let segment = build(count);
        let (last, head) = segment.deltas.split_last().expect("one delta at least");
        let doc = LoroDoc::new();
        doc.import_batch(head).expect("import head");
        group.bench_function(count.to_string(), |b| {
            b.iter_batched(
                || (),
                |()| {
                    let fork = doc.fork();
                    fork.import(last).expect("import into fork");
                    black_box(fork)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_replay,
    bench_replay_batch,
    bench_snapshot,
    bench_fork_check,
    bench_json_check
);
criterion_main!(benches);
