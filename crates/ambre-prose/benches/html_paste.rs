//! Criterion benchmark for an HTML paste, read and written into a document.
//!
//! Target from `ambre-spec-budgets.md`, the "Prose HTML clipboard import"
//! row: 500 KB under 100 ms on the Logic thread. Each fixture is repeated to
//! 500 KB, so the figure covers the parse, the walk and the Loro writes.

#![expect(clippy::panic, reason = "a bench aborts on a fixture it cannot run")]

use ambre_core::index::Pos;
use std::hint::black_box;

use ambre_prose::logic::ProseState;
use ambre_prose::message::{DocChar, ProseCommand};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

const SIZE: usize = 500 * 1024;

/// One fixture, repeated until it weighs `SIZE` bytes.
fn corpus(fragment: &str) -> String {
    let mut html = String::with_capacity(SIZE + fragment.len());
    while html.len() < SIZE {
        html.push_str(fragment);
    }
    html
}

/// A document with one empty paragraph, and a paste of `html` into it.
fn paste(html: &str) -> impl FnMut() -> (ProseState, ProseCommand) + '_ {
    move || {
        let Some(state) = ProseState::with_peer(ambre_core::authorship::PeerId::new(1)) else {
            panic!("a document is built");
        };
        let Some(&first) = state.root_blocks().first() else {
            panic!("a document holds a block");
        };
        let command = ProseCommand::Paste {
            at: (first, Pos::<DocChar>::ZERO),
            text: String::new(),
            html: Some(html.to_owned()),
            payload: None,
        };
        (state, command)
    }
}

fn bench_html_paste(c: &mut Criterion) {
    let fixtures = [
        (
            "google_docs",
            include_str!("../src/logic/clipboard/fixtures/google-docs.html"),
        ),
        (
            "web_page",
            include_str!("../src/logic/clipboard/fixtures/web-page.html"),
        ),
        (
            "word_fragment",
            include_str!("../src/logic/clipboard/fixtures/word.html"),
        ),
    ];
    let mut group = c.benchmark_group("html_paste_500kb");
    group.sample_size(20);
    for (name, fragment) in fixtures {
        let html = corpus(fragment);
        group.bench_function(name, |b| {
            b.iter_batched(
                paste(&html),
                |(mut state, command)| black_box(state.handle_command(command)),
                BatchSize::LargeInput,
            );
        });
    }
    // The worst nesting the reader builds, just under the cap, at the same size.
    let deep = corpus(&format!("{}x{}", "<div>".repeat(127), "</div>".repeat(127)));
    group.bench_function("nested_127", |b| {
        b.iter_batched(
            paste(&deep),
            |(mut state, command)| black_box(state.handle_command(command)),
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_html_paste);
criterion_main!(benches);
