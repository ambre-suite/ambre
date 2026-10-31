//! Criterion benchmark for the Logic pass that answers one typed key.
//!
//! Target from `ambre-spec-budgets.md`, the "Logic pass that answers a
//! keystroke" row: under 4 ms. The document is the probe's: 40 paragraphs of
//! about 2 000 characters, and the key lands at the end of the last one. The
//! whole pass is the command, whose delta is what local persistence logs, and
//! the layout handed to the view. The snapshot that folds the log is measured
//! apart: a keystroke pays it once every few hundred keys.

#![allow(clippy::panic, reason = "a bench aborts on a fixture it cannot run")]

use ambre_core::index::Pos;
use std::hint::black_box;

use ambre_bus::app::prose::{DocChar, ProseCommand};
use ambre_prose::events::ProseEvent;
use ambre_prose::logic::ProseState;
use criterion::{Criterion, criterion_group, criterion_main};

const PARAGRAPH: &str =
    "The quick brown fox jumps over the lazy dog while the text keeps flowing. ";

/// The faces the application ships, read from the workspace tree.
fn faces() -> Vec<ambre_core::Payload<ambre_core::FontBytes>> {
    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
    [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSans-Italic.ttf",
        "NotoSans-BoldItalic.ttf",
        "DejaVuSerif.ttf",
        "DejaVuSansMono.ttf",
    ]
    .iter()
    .filter_map(|file| std::fs::read(format!("{DIR}{file}")).ok())
    .map(ambre_core::Payload::<ambre_core::FontBytes>::new)
    .collect()
}

/// The probe's long document, laid out once, with the caret at the end of
/// its last paragraph.
fn document() -> (ProseState, ambre_bus::app::prose::BlockId, u32) {
    let Some(mut state) = ProseState::with_peer(ambre_core::authorship::PeerId::new(1)) else {
        panic!("a document is built");
    };
    state.load_faces(&faces());
    let Some(&first) = state.root_blocks().first() else {
        panic!("a document holds a block");
    };
    let text = vec![PARAGRAPH.repeat(26); 40].join("\n");
    let pasted = state.handle_view_command(ProseCommand::Paste {
        at: (first, Pos::<DocChar>::ZERO),
        text,
        html: None,
        payload: None,
    });
    // The view says it took the paste, as it does before it types again.
    // Without it every key lands at a position the view never saw, and is
    // refused.
    let seen = pasted
        .display
        .iter()
        .filter_map(|event| match event {
            ProseEvent::TextChanged { seq, .. } => Some(*seq),
            _ => None,
        })
        .max()
        .unwrap_or(ambre_core::Generation::ZERO);
    state.handle_view_command(ProseCommand::Sync { seen });
    let _ = state.layout_event();
    let Some(&last) = state.root_blocks().last() else {
        panic!("a document holds a block");
    };
    let pos = u32::try_from(PARAGRAPH.len() * 26).unwrap_or(u32::MAX);
    (state, last, pos)
}

fn key(block_id: ambre_bus::app::prose::BlockId, pos: u32) -> ProseCommand {
    ProseCommand::InsertText {
        block_id,
        pos: Pos::<DocChar>::new(pos),
        // Words of five letters, as a person types, and not one word
        // growing without end.
        text: if pos % 6 == 5 { " " } else { "a" }.to_owned(),
    }
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("keystroke");
    group.sample_size(50);

    let (mut state, block, mut pos) = document();
    let answer = state.handle_view_command(key(block, pos));
    assert!(
        !answer
            .display
            .iter()
            .any(|event| matches!(event, ProseEvent::Error(_))),
        "the key is taken"
    );
    pos += 1;
    group.bench_function("whole pass", |b| {
        b.iter(|| {
            black_box(state.handle_view_command(key(block, pos)));
            black_box(state.layout_event());
            pos += 1;
        });
    });

    let (mut state, block, mut pos) = document();
    group.bench_function("command", |b| {
        b.iter(|| {
            black_box(state.handle_view_command(key(block, pos)));
            pos += 1;
        });
    });

    let (state, _, _) = document();
    group.bench_function("fold into a snapshot", |b| {
        b.iter(|| black_box(state.export_snapshot()));
    });

    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
