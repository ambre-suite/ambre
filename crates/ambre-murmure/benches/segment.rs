//! The segment benches: what an active segment costs as its history grows.
//!
//! The segment is built in process by `common/segment.rs`, which the phone page
//! of `ambre-murmure` includes too. No fixture file is read.
//! A bench reports a number, and `ambre-spec-budgets.md` says what it must stay
//! under.
//!
//! **Run these with `cargo bench -p ambre-murmure --bench segment`.** No CI job
//! runs them: `criterion` measures wall clock, so a figure means nothing
//! without the machine it came from.
//!
//! The sizes are printed once per corpus, on standard error, because `criterion`
//! measures time and not bytes.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a bench aborts on a fixture it cannot build"
)]

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use loro::{ExportMode, JsonSchema, LoroDoc};

/// The message counts of an active segment. 5000 is the close threshold of
/// `ambre-spec-budgets.md` section 6.
const SIZES: [usize; 3] = [500, 2000, 5000];

#[path = "common/segment.rs"]
mod corpus;

use corpus::{Segment, build};

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
