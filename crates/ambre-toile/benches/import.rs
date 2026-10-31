//! Criterion benchmark for an SVG import, converted and applied to a board.
//!
//! Targets from `ambre-spec-budgets.md`. The "Toile SVG import, 5 000 nodes"
//! row holds the whole import under 500 ms. The import is applied in slices
//! (`TOILE-IO-028`). The `slice` case reads one, which is what a camera move
//! waits behind.

use std::hint::black_box;

use ambre_toile::{ToileCommand, ToileState};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use kurbo::Point;

/// An SVG of `n` rectangles, one per node, on a grid.
fn drawing(n: usize) -> Vec<u8> {
    let mut svg =
        String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="4000">"#);
    for i in 0..n {
        let (x, y) = ((i % 100) * 40, (i / 100) * 40);
        svg.push_str(&format!(
            r##"<rect x="{x}" y="{y}" width="30" height="30" fill="#3366cc"/>"##
        ));
    }
    svg.push_str("</svg>");
    svg.into_bytes()
}

fn command(bytes: &[u8]) -> ToileCommand {
    ToileCommand::ImportSvg {
        bytes: ambre_core::Payload::<ambre_core::FileBytes>::new(bytes.to_vec()),
        at: Point::ZERO,
    }
}

fn bench_import(c: &mut Criterion) {
    let bytes = drawing(5_000);
    let mut group = c.benchmark_group("svg_import_5000");
    group.sample_size(10);
    // The whole import: the first slice, then every slice the loop resumes.
    group.bench_function("whole", |b| {
        b.iter_batched(
            || (ToileState::new(), command(&bytes)),
            |(mut state, command)| {
                state.handle_command(command);
                while state.has_pending_work() {
                    black_box(state.resume());
                }
            },
            BatchSize::LargeInput,
        );
    });
    // One slice: the longest a command waits behind the import.
    group.bench_function("slice", |b| {
        b.iter_batched(
            || {
                let mut state = ToileState::new();
                state.handle_command(command(&bytes));
                state
            },
            |mut state| black_box(state.resume()),
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_import);
criterion_main!(benches);
