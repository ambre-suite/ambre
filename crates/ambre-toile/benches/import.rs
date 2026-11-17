//! Criterion benchmark for an SVG import, converted and applied to a board.
//!
//! Targets from `ambre-spec-budgets.md`. The "Toile SVG import, 5 000 nodes"
//! row holds the whole import under 500 ms. The import is applied in slices
//! (`TOILE-IO-028`). The `slice` case reads one, which is what a camera move
//! waits behind.

#![expect(clippy::panic, reason = "a bench aborts on an engine it cannot load")]

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
    ToileCommand::import(
        ambre_toile::message::ToileImportFormat::Svg,
        ambre_core::Payload::<ambre_client_core::FileBytes>::new(bytes.to_vec()),
        Point::ZERO,
    )
}

/// The images engine the host links in place: the bench reads the file
/// through the request bytes a module reads, as the Logic thread does.
fn converter() -> ambre_convert::Converter {
    match ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Images,
        &ambre_core::Payload::new(Vec::new()),
    ) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the images engine: {e}"),
    }
}

/// Serve `command` as the Logic dispatcher and the converter thread do: owe
/// the read, run it on `converter`, land the answer.
fn convert(state: &mut ToileState, command: ToileCommand, converter: &ambre_convert::Converter) {
    state.handle_conversion(command);
    for (ticket, conversion) in state.take_file_requests() {
        state.handle_file_converted(ticket, converter.convert(&conversion));
    }
}

fn bench_import(c: &mut Criterion) {
    let bytes = drawing(5_000);
    let converter = converter();
    let mut group = c.benchmark_group("svg_import_5000");
    group.sample_size(10);
    // The whole import: the first slice, then every slice the loop resumes.
    group.bench_function("whole", |b| {
        b.iter_batched(
            || (ToileState::new(), command(&bytes)),
            |(mut state, command)| {
                convert(&mut state, command, &converter);
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
                convert(&mut state, command(&bytes), &converter);
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
