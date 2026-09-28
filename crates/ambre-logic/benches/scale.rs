//! The scale benches (`ambre-spec-fixtures.md` section 3).
//!
//! Each group reads a corpus from `fixtures/bench/`, built to a figure
//! `ambre-spec-budgets.md` already states. This file repeats no target. A bench
//! reports a number, and the budget row says what the number must stay under.
//!
//! **Run these with `just fixtures-bench`**, which builds the corpus first. A
//! missing corpus fails the bench with the name of the artifact, rather than
//! silently measuring nothing.
//!
//! **No CI job runs them** (`FIXTURE-023`). `criterion` measures wall clock, so
//! a figure means nothing without the machine it came from. The author of a
//! performance-relevant change puts both in the pull request.
//!
//! **No figure here is a frame rate** (`FIXTURE-024`). The frame-rate budget is
//! GPU-bound and stays validated visually. What these measure is the CPU half of
//! the frame: the import, the recompute, the cull and the derivation.

use std::hint::black_box;

use ambre_bus::TrameCommand;
use ambre_bus::app::toile::{ToileCommand, Viewport};
use ambre_logic::DocState;
use ambre_logic::fixtures::load_bench_fixture;
use criterion::{Criterion, criterion_group, criterion_main};
use kurbo::{Point, Vec2};

/// Read one scale corpus, or stop with the recipe that writes it.
///
/// A panic rather than a skip: a bench that quietly measures nothing reports a
/// figure of zero, which reads as an improvement.
fn corpus(name: &str) -> DocState {
    match load_bench_fixture(name) {
        Ok((_, state)) => state,
        Err(e) => panic!("{name}: {e}. Run `just fixtures-bench-corpus`"),
    }
}

/// The `.ambre` payload of a corpus, for the import benches.
fn payload(name: &str) -> Vec<u8> {
    corpus(name).export_snapshot()
}

/// Snapshot import: the whole cold-open path, container aside.
///
/// Trame's import triggers a full engine reseed, which re-parses and recompiles
/// every formula. The formula corpus and the literal one therefore take
/// separate rows. Toile's import bulk-loads the spatial index.
///
/// **`bench_formula_grid_100k` is not in this list.** One import of it measured
/// 102 s, so ten samples of it alone run for seventeen minutes and an author
/// stops running the recipe. That corpus serves the DAG and cache *memory*
/// budget, which names megabytes. `criterion` cannot measure those, so
/// `just fixtures-bench-corpus` reports the artifact size instead.
fn bench_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("import_snapshot");
    // Seconds per iteration: the default hundred samples would run for an hour.
    // Ten is `criterion`'s floor and still yields a confidence interval.
    group.sample_size(10);
    for name in [
        "trame/bench_dense_grid_100k",
        "trame/bench_formula_dag_10k",
        "toile/bench_board_50k",
    ] {
        let bytes = payload(name);
        let doc_type = corpus(name).doc_type();
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut fresh = DocState::new(doc_type).expect("a known engine");
                let _ = fresh.import_snapshot(black_box(&bytes));
                fresh
            });
        });
    }
    group.finish();
}

/// Trame recompute: one edit at the root of a 10 000-cell formula walk.
///
/// The edit lands on the literal column every formula reads, so the walk it
/// triggers covers the whole cascade rather than one cell.
fn bench_trame_recompute(c: &mut Criterion) {
    let mut state = match corpus("trame/bench_formula_dag_10k") {
        DocState::Trame(state) => state,
        other => panic!("the formula corpus is a workbook, not {other:?}"),
    };
    let mut round = 0u32;
    let mut group = c.benchmark_group("trame_recompute");
    group.sample_size(10);
    group.bench_function("10k_dag", |b| {
        b.iter(|| {
            // A different value each round: an identical write is a no-op, and a
            // no-op measures the equality check rather than the recompute.
            round += 1;
            state.handle_command(TrameCommand::SetCellText {
                row: 0,
                col: 0,
                text: round.to_string(),
            })
        });
    });
    group.finish();
}

/// Trame batch apply: 50 cells in one command, on a 100 000-cell workbook.
///
/// The batch size is the one the budget names. The workbook under it is the
/// large one: the row is about what a batch pays for the rest of the sheet.
fn bench_trame_batch(c: &mut Criterion) {
    let mut state = match corpus("trame/bench_dense_grid_100k") {
        DocState::Trame(state) => state,
        other => panic!("the dense corpus is a workbook, not {other:?}"),
    };
    let mut round = 0u32;
    c.bench_function("trame_batch_50_cells_on_100k", |b| {
        b.iter(|| {
            round += 1;
            let cells = (0..50)
                .map(|i| (i, 0, format!("{round}-{i}")))
                .collect::<Vec<_>>();
            state.handle_command(TrameCommand::SetCellsBatch { cells })
        });
    });
}

/// Toile cull and derivation: one camera move over a 50 000-element board, and
/// one over a board whose 2 000 elements are all visible.
///
/// The two are the ends of one measurement. The spread board charges the
/// visible-set query against a large index. The dense one charges the
/// derivation against a large visible set, the CPU half of the paint budget.
fn bench_toile_viewport(c: &mut Criterion) {
    let mut group = c.benchmark_group("toile_viewport");
    // Seconds per iteration on the spread board today, for the same reason as
    // the import group above.
    group.sample_size(10);
    for (name, step) in [
        ("toile/bench_board_50k", 400.0),
        ("toile/bench_dense_viewport_2k", 4.0),
    ] {
        let mut state = match corpus(name) {
            DocState::Toile(state) => state,
            other => panic!("{name} is a board, not {other:?}"),
        };
        let mut round = 0f64;
        group.bench_function(name, |b| {
            b.iter(|| {
                // A moving camera each round. An unchanged viewport may hit a
                // cache, and a cache hit is not what the budget covers.
                round += 1.0;
                state.handle_command(ToileCommand::SetViewport(Viewport {
                    center: Point::new(round * step, 0.0),
                    ..Viewport::default()
                }))
            });
        });
    }
    group.finish();
}

/// Toile index update: one element moved on a 50 000-element board.
///
/// A drag mutates one element per input event, so this is the per-event cost the
/// budget names.
fn bench_toile_index_update(c: &mut Criterion) {
    let mut state = match corpus("toile/bench_board_50k") {
        DocState::Toile(state) => state,
        other => panic!("the spread board is a board, not {other:?}"),
    };
    let Some(id) = state.layer_order().iter().find_map(|node| match node {
        ambre_toile::NodeId::Element(id) => Some(id.clone()),
        ambre_toile::NodeId::Connector(_) => None,
    }) else {
        panic!("the spread board carries elements");
    };
    let mut sign = 1.0f64;
    let mut group = c.benchmark_group("toile_index_update");
    group.sample_size(10);
    group.bench_function("move_one_of_50k", |b| {
        b.iter(|| {
            // Back and forth, so the element stays where the corpus put it and
            // the index never drifts across rounds.
            sign = -sign;
            state.handle_command(ToileCommand::MoveElements {
                ids: vec![id.clone()],
                delta: Vec2::new(sign, 0.0),
            })
        });
    });
    group.finish();
}

/// The deck's PDF export, 50 slides.
///
/// The whole walk: one derivation per slide, then the page assembly. It runs
/// inside the Logic command loop, so the figure is also how long the document
/// stops accepting edits.
fn bench_deck_pdf(c: &mut Criterion) {
    let state = match corpus("toile/bench_deck_50") {
        DocState::Toile(state) => state,
        other => panic!("the deck is a board, not {other:?}"),
    };
    let mut font_cx = parley::FontContext::new();
    let mut group = c.benchmark_group("toile_pdf_export");
    // A whole export is seconds, so the default 100 samples would run for an
    // hour. Ten is what `criterion` accepts as a floor and still yields a
    // confidence interval.
    group.sample_size(10);
    group.bench_function("deck_50_slides", |b| {
        b.iter(|| {
            state
                .export_pdf_document(Default::default(), &mut font_cx)
                .map(|bytes| bytes.len())
        });
    });
    group.finish();
}

/// One remote delta applied to a live 100 000-cell workbook.
///
/// A second peer produces the delta, and no file beside the corpus carries it.
/// Deriving it from the artifact keeps the two in step, and spares the corpus a
/// second artifact format.
fn bench_remote_delta(c: &mut Criterion) {
    let recorded = {
        let mut writer = match corpus("trame/bench_dense_grid_100k") {
            DocState::Trame(state) => state,
            other => panic!("the dense corpus is a workbook, not {other:?}"),
        };
        let cells = (0..50)
            .map(|i| (i, 1, format!("remote {i}")))
            .collect::<Vec<_>>();
        writer
            .handle_command(TrameCommand::SetCellsBatch { cells })
            .update
            .expect("a batch of 50 cells produces a delta")
    };
    let mut state = match corpus("trame/bench_dense_grid_100k") {
        DocState::Trame(state) => state,
        other => panic!("the dense corpus is a workbook, not {other:?}"),
    };
    c.bench_function("trame_remote_delta_on_100k", |b| {
        b.iter(|| state.apply_remote(black_box(&recorded)));
    });
}

/// One paragraph shaped whole, as the view shapes the paragraph an edit in
/// flight changes (`features/ambre-spec-prose-text-editing.md section 3`).
///
/// No corpus: the text is built here, at the lengths the budget row of text
/// layout is read at (`ambre-spec-budgets.md section 2`). A native figure;
/// the browser's is slower, and the row counts that apart.
fn bench_prose_paragraph_shape(c: &mut Criterion) {
    use ambre_prose::layout::{fonts, measure};
    const FACES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
    let mut font_cx = parley::FontContext::new();
    for face in ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"] {
        let bytes =
            std::fs::read(format!("{FACES}{face}")).unwrap_or_else(|e| panic!("{face}: {e}"));
        fonts::register_faces(&mut font_cx, bytes);
    }
    let mut layout_cx = parley::LayoutContext::new();
    let style = measure::EffectiveStyle::from_parts(
        &ambre_prose::doc::DocDefaults::default(),
        &ambre_bus::app::prose::ParagraphStyle::default(),
        false,
    );
    let width = style.text_width_px(600.0);
    let mut group = c.benchmark_group("prose_paragraph_shape");
    for chars in [1_000_usize, 5_000, 20_000] {
        let text: String = "lorem ipsum dolor sit amet "
            .chars()
            .cycle()
            .take(chars)
            .collect();
        group.bench_function(format!("{chars}_chars"), |b| {
            b.iter(|| {
                measure::shape_paragraph(
                    &mut font_cx,
                    &mut layout_cx,
                    (black_box(&text), &[]),
                    &style,
                    width,
                )
                .height()
            });
        });
    }
    group.finish();
}

criterion_group!(
    scale,
    bench_prose_paragraph_shape,
    bench_import,
    bench_trame_recompute,
    bench_trame_batch,
    bench_toile_viewport,
    bench_toile_index_update,
    bench_deck_pdf,
    bench_remote_delta,
);
criterion_main!(scale);
