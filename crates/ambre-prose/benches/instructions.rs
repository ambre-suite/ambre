//! Operations whose budget is held by a count of instructions, not by a time.
//!
//! A time measured on a shared CI runner moves by tens of percent between two
//! runs, so a threshold on it fails on noise. An instruction count under
//! `callgrind` is stable, so `scripts/instructions.py` fails the build when an
//! operation outgrows its ceiling (`ambre-spec-testing.md section 2`).
//!
//! Each operation runs once, inside a function that is never inlined and whose
//! name is unique. The script names the operation on the command line and tells
//! `callgrind` to count only inside that function, so the setup costs nothing.
//! Run with no operation named, as `cargo bench` does, every operation runs
//! once: a smoke test.

#![allow(clippy::panic, reason = "a bench aborts on a fixture it cannot run")]

use ambre_core::index::Count;
use std::hint::black_box;

use ambre_bus::app::prose::ParagraphStyle;
use ambre_core::{Doc, Px};
use ambre_draw::ParleyBrush;
use ambre_prose::doc::DocDefaults;
use ambre_prose::layout::fonts::register_faces;
use ambre_prose::layout::measure::{EffectiveStyle, Measurer, reshape_paragraph};
use ambre_prose::layout::run::{ParleyRunData, RunLine, ShapedRun};
use parley::{FontContext, LayoutContext};

/// The longest paragraph the "Text layout" row holds to its budget at its
/// worst. That is a key whose new breaks run to the end of the paragraph.
const PARAGRAPH_CHARS: usize = 500;

/// The width of a page's text column, in pixels: an A4 page and its default
/// margins.
const COLUMN_PX: f32 = 624.0;

/// The faces the application ships, read from the workspace tree.
fn faces() -> Vec<Vec<u8>> {
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
    .collect()
}

/// A paragraph of `PARAGRAPH_CHARS` characters, `word` over and over, so
/// every line holds the same words. A key the first line has no room for
/// pushes one word down from every line after it. No line then breaks where
/// it broke before: the cascade the window cannot stop.
fn paragraph(word: &str) -> String {
    format!("{word} ")
        .chars()
        .cycle()
        .take(PARAGRAPH_CHARS)
        .collect()
}

/// The words a paragraph is tried with, until one leaves its lines too
/// little room for a key.
const WORDS: [&str; 6] = [
    "flowing", "streams", "rivers", "quickly", "lantern", "harbour",
];

/// One key typed into a laid paragraph, shaped as the view shapes it.
/// Counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_prose_paragraph_shape(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    laid: &ParleyRunData,
    text: &str,
    style: &EffectiveStyle,
) -> ShapedRun {
    reshape_paragraph(
        font_cx,
        layout_cx,
        laid,
        (text, &[]),
        style,
        Px::<Doc>::new(COLUMN_PX),
    )
}

/// The shape the row names at its worst: a key typed in the first line of a
/// paragraph. Its line breaks never fall back on the old ones. The view
/// tries a window, misses, and shapes the rest of the paragraph
/// (`PROSE-UI-215`). The contexts are warm, as the view's are once its first
/// paragraph is drawn.
fn paragraph_shape() {
    let mut measurer = Measurer::new();
    let mut font_cx = FontContext::new();
    for bytes in faces() {
        let face = ambre_core::Payload::<ambre_core::FontBytes>::new(bytes);
        measurer.register_faces(face.clone());
        register_faces(&mut font_cx, face);
    }
    let mut layout_cx = LayoutContext::new();
    let style = EffectiveStyle::from_parts(
        &DocDefaults::default(),
        &ParagraphStyle::default(),
        ambre_core::Weight::Regular,
    );
    // The first paragraph and key, typed in its first word, whose breaks
    // run to the end. The view shapes it again whole, from its first line.
    // Shaped once here, which warms the contexts too.
    let mut found = None;
    'search: for word in WORDS {
        let text = paragraph(word);
        let (_, _, laid) =
            measurer.measure_with_lines((&text, &[]), &style, Px::<Doc>::new(COLUMN_PX), &[], None);
        for key in ['i', 'x', 'm', 'W'] {
            let mut typed = text.clone();
            typed.insert(1, key);
            let run = reshape_paragraph(
                &mut font_cx,
                &mut layout_cx,
                laid.data(),
                (&typed, &[]),
                &style,
                Px::<Doc>::new(COLUMN_PX),
            );
            if run.data().segment_count() == Count::<ambre_prose::layout::run::RunSegment>::ONE {
                found = Some((laid, typed));
                break 'search;
            }
        }
    }
    let Some((laid, typed)) = found else {
        panic!("no key moves every break of a paragraph");
    };
    let run = ambre_budget_prose_paragraph_shape(
        &mut font_cx,
        &mut layout_cx,
        laid.data(),
        black_box(&typed),
        &style,
    );
    if run.data().line_count() < Count::<RunLine>::new(2) {
        panic!("a long paragraph breaks into lines");
    }
}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn()); 1] = [("paragraph_shape", paragraph_shape)];

fn main() {
    // `cargo bench` passes flags such as `--bench`; only a bare word names an
    // operation.
    let named: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    for (name, run) in OPERATIONS {
        if named.is_empty() || named.iter().any(|n| n == name) {
            run();
        }
    }
}
