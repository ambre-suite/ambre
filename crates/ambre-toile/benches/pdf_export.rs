//! Criterion benchmark for the deck's PDF export, 50 slides.
//!
//! Targets from `ambre-spec-budgets.md section 1`. The "Toile PDF export, 50
//! slides" row holds the whole export under 3 s. The pages are recorded one
//! per pass of the Logic thread (`TOILE-IO-099`), so each case below is one
//! span the thread spends on it:
//!
//! * `whole`: the start, every page, and the write;
//! * `start`: the pages and the pictures read off the board, in the pass of
//!   the request;
//! * `page`: one page recorded, what an edit waits behind;
//! * `write`: the PDF module writing the recorded pages, in the pass of the
//!   last page.

#![expect(clippy::panic, reason = "a bench aborts on a fixture it cannot build")]

mod common;

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

/// The PDF engine the host links in place, as the Logic thread's host build
/// hands it in.
fn pdf_converter() -> ambre_convert::Converter {
    ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Pdf,
        &ambre_core::Payload::new(Vec::new()),
    )
    .unwrap_or_else(|e| panic!("the host links the PDF engine: {e}"))
}

fn bench_pdf_export(c: &mut Criterion) {
    let st = common::deck(common::SLIDES);
    let converter = pdf_converter();
    let mut group = c.benchmark_group("pdf_export_50_slides");
    group.sample_size(10);
    group.bench_function("whole", |b| {
        b.iter_batched(
            common::shipped_fonts,
            |mut fonts| {
                let mut recording = st
                    .record_deck(Default::default())
                    .unwrap_or_else(|e| panic!("the deck has pages: {e}"));
                while !recording.is_recorded() {
                    recording.record_next(&st, &mut fonts);
                }
                black_box(
                    ambre_toile::logic::interop::write_pdf(recording.into_document(), &converter)
                        .unwrap_or_else(|e| panic!("the module writes the deck: {e}")),
                )
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("start", |b| {
        b.iter(|| black_box(st.record_deck(Default::default())));
    });
    // The first page shapes every run cold, so it is the dearest page.
    group.bench_function("page", |b| {
        b.iter_batched(
            || {
                let recording = st
                    .record_deck(Default::default())
                    .unwrap_or_else(|e| panic!("the deck has pages: {e}"));
                (recording, common::shipped_fonts())
            },
            |(mut recording, mut fonts)| {
                recording.record_next(&st, &mut fonts);
                black_box(recording)
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("write", |b| {
        b.iter_batched(
            || {
                let mut fonts = common::shipped_fonts();
                let mut recording = st
                    .record_deck(Default::default())
                    .unwrap_or_else(|e| panic!("the deck has pages: {e}"));
                while !recording.is_recorded() {
                    recording.record_next(&st, &mut fonts);
                }
                recording.into_document()
            },
            |document| black_box(ambre_toile::logic::interop::write_pdf(document, &converter)),
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_pdf_export);
criterion_main!(benches);
