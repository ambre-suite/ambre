//! Criterion benchmarks for what one change costs on a large board.
//!
//! Targets from `ambre-spec-budgets.md`. The "Toile index update, one element"
//! row holds a drag step under 50 us (`TOILE-091`). The "Toile remote change"
//! row holds one received move under 2 ms, derivation included (`TOILE-092`).
//!
//! The remote case runs twice. `opened` is a reader that opened the board
//! from the author's snapshot and wrote nothing since. Its own scaffolding
//! stays concurrent with every change the author sends (`TOILE-265`).
//! `synced` is the same reader once the author has seen that scaffolding.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a bench aborts on a fixture it cannot run"
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use ambre_toile::{
    ElementBounds, ElementId, ElementKind, InitContent, ShapeKind, SpatialIndex, ToileCommand,
    ToileEvent, ToileState,
};
use criterion::{Criterion, criterion_group, criterion_main};
use kurbo::{Point, Rect, Size, Vec2};

/// A leaf of a 50 000-element grid, nudged by `dy`.
fn leaf(i: usize, dy: f64) -> ElementBounds {
    let (x, y) = ((i % 250) as f64 * 40.0, (i / 250) as f64 * 40.0 + dy);
    ElementBounds::from_rect(
        ElementId(format!("e{i}")),
        Rect::new(x, y, x + 30.0, y + 30.0),
    )
}

fn bench_index_update(c: &mut Criterion) {
    let mut index = SpatialIndex::build((0..50_000).map(|i| leaf(i, 0.0)));
    let mut step = 0usize;
    c.bench_function("index_update_one_of_50000", |b| {
        b.iter(|| {
            // One drag step: the same element, a little further each time.
            step += 1;
            index.update(black_box(leaf(25_125, (step % 20) as f64)));
        });
    });
}

/// A board of `n` rectangles, and their ids.
fn board(n: usize) -> (ToileState, Vec<ElementId>) {
    let mut st = ToileState::new();
    let ids = (0..n)
        .map(|i| {
            let out = st.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new((i % 100) as f64 * 40.0, (i / 100) as f64 * 40.0),
                size: Size::new(30.0, 30.0),
                content: InitContent::None,
                fill: None,
                correlate: None,
            });
            out.display
                .iter()
                .find_map(|e| match e {
                    ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .expect("a create names its element")
        })
        .collect();
    (st, ids)
}

/// Time `iters` remote moves, each written by `author` and read by `reader`.
/// The author's write is not the reader's cost: only the import, the refresh
/// and the derivation are timed.
fn remote_moves(
    author: &mut ToileState,
    reader: &mut ToileState,
    ids: &[ElementId],
    iters: u64,
) -> Duration {
    let mut spent = Duration::ZERO;
    for i in 0..iters {
        let id = ids[(i as usize * 37) % ids.len()].clone();
        let out = author.handle_command(ToileCommand::MoveElements {
            ids: vec![id],
            delta: Vec2::new(0.0, 1.0),
        });
        let update = out.update.expect("a move writes");
        let start = Instant::now();
        black_box(reader.apply_remote(&update));
        spent += start.elapsed();
    }
    spent
}

fn bench_remote_change(c: &mut Criterion) {
    let (mut author, ids) = board(5_000);
    let mut opened = ToileState::new();
    opened
        .import_snapshot(&author.export_snapshot())
        .expect("the snapshot imports");
    let mut group = c.benchmark_group("remote_change_5000");
    group.sample_size(10);
    group.bench_function("opened", |b| {
        b.iter_custom(|iters| remote_moves(&mut author, &mut opened, &ids, iters));
    });
    let (mut author, ids) = board(5_000);
    let mut synced = ToileState::new();
    synced
        .import_snapshot(&author.export_snapshot())
        .expect("the snapshot imports");
    author
        .import_snapshot(&synced.export_snapshot())
        .expect("the snapshot imports");
    group.bench_function("synced", |b| {
        b.iter_custom(|iters| remote_moves(&mut author, &mut synced, &ids, iters));
    });
    group.finish();
}

criterion_group!(benches, bench_index_update, bench_remote_change);
criterion_main!(benches);
