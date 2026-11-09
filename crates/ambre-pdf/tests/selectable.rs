//! A word on a written page reads back as characters (`PDF-005`).
//!
//! Each test shapes real text on the faces the application ships. It draws
//! the text through the shared surface and writes the PDF. Then it reads the
//! text back with `pdf-extract`. That extractor shares no code with krilla,
//! so a mapping krilla writes wrong reads back wrong.
//!
//! A lazy script names its face: a sealed context does not fall back to it
//! from the base family (`ambre-roadmap.md`, `Lazy faces in the sealed
//! fallback`).

// The helpers outside the test functions write and read a file, and a test
// fails at the step that refused.
#![expect(clippy::expect_used, reason = "a helper stops the test that called it")]

use ambre_core::PrintLength;
use ambre_core::index::Pos;
use ambre_draw::list::{DrawOp, ListRecorder, PrintDocument, PrintPage, RecorderTables};
use ambre_draw::testing::shipped_font_context;
use ambre_draw::{Affine, Brush, DrawSurface, LayoutText, ParleyBrush, run_text};
use ambre_pdf::{KrillaSurface, write_document};
use parley::{
    FontContext, FontFamily, FontFamilyName, Layout, LayoutContext, PositionedLayoutItem,
    StyleProperty,
};

/// `text` shaped in `family` at 20 px, broken at `width` or not at all.
fn shape(
    font_cx: &mut FontContext,
    text: &str,
    family: &str,
    width: Option<f32>,
) -> Layout<ParleyBrush> {
    let mut layout_cx: LayoutContext<ParleyBrush> = LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
        FontFamilyName::Named(family.into()),
    )));
    builder.push_default(StyleProperty::FontSize(20.0));
    let mut layout = builder.build(text);
    layout.break_all_lines(width);
    layout
}

/// Every glyph run of `layout`, drawn on `surface` with `text` as its source.
fn draw(surface: &mut impl DrawSurface, layout: &Layout<ParleyBrush>, text: LayoutText<'_>) {
    let ink = Brush::Solid(peniko::color::palette::css::BLACK);
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(run) = item {
                let at = Affine::translate((10.0, 40.0 + f64::from(run.baseline())));
                surface.draw_glyphs(&run, text, &ink, at);
            }
        }
    }
}

fn page() -> PrintPage {
    PrintPage {
        width: PrintLength::new(600.0),
        height: PrintLength::new(200.0),
    }
}

/// The recorded page of `text` in `family`, as the converter module gets it.
fn record(text: &str, family: &str) -> PrintDocument {
    let mut font_cx = shipped_font_context();
    let layout = shape(&mut font_cx, text, family, None);
    let mut document = PrintDocument::new(page());
    let mut tables = RecorderTables::default();
    let mut recorder = ListRecorder::new(&mut document, &mut tables);
    draw(&mut recorder, &layout, LayoutText::whole(text));
    recorder.finish();
    document
}

/// The text a reader extracts from `document`, once written.
fn read_back(document: &PrintDocument) -> String {
    let bytes = write_document(document).expect("the page writes");
    pdf_extract::extract_text_from_mem(bytes.as_bytes()).expect("the file reads")
}

/// The extracted text with the spacing an extractor guesses from gaps removed.
fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

/// How many glyphs the recorded page draws.
fn glyph_count(document: &PrintDocument) -> usize {
    document
        .pages
        .iter()
        .flatten()
        .map(|op| match op {
            DrawOp::Glyphs { glyphs, .. } => glyphs.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn a_latin_line_reads_back() {
    let read = read_back(&record("Ambre exports text", "Noto Sans"));
    assert_eq!(read.trim(), "Ambre exports text");
}

#[test]
fn a_ligature_reads_back_as_its_letters() {
    let text = "office fit";
    let document = record(text, "DejaVu Serif");
    // The face ligates "ffi" and "fi": fewer glyphs than characters, so the
    // test reads a ligature and not one glyph per letter.
    assert!(glyph_count(&document) < text.chars().count());
    assert_eq!(read_back(&document).trim(), text);
}

#[test]
fn a_combining_mark_reads_back_with_its_base() {
    // A base and a combining acute: the face composes each pair into one
    // glyph, which reads back as the two code points the user typed.
    let text = "e\u{301}te\u{301}";
    let document = record(text, "Noto Sans");
    assert!(glyph_count(&document) < text.chars().count());
    assert_eq!(read_back(&document).trim(), text);
}

#[test]
fn a_cluster_of_several_glyphs_reads_back_once() {
    // A Devanagari conjunct with a vowel sign: one cluster, two glyphs. The
    // writer maps the first glyph to the cluster and leaves the second empty,
    // so the text reads back once and not twice.
    let text = "\u{0915}\u{094D}\u{0937}\u{093F}";
    let document = record(text, "Noto Sans Devanagari");
    assert!(glyph_count(&document) > 1);
    assert_eq!(read_back(&document).trim(), text);
}

#[test]
fn emoji_read_back_as_their_sequences() {
    // A single emoji and a family joined by zero-width joiners: one glyph
    // each, every code point of the sequence read back.
    let text = "\u{1F600}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let document = record(text, "Noto Color Emoji");
    assert_eq!(glyph_count(&document), 2);
    assert_eq!(read_back(&document).trim(), text);
}

#[test]
fn a_right_to_left_word_reads_back_in_logical_order() {
    let text = "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}";
    let document = record(text, "Noto Sans Arabic");
    // The page draws the glyphs left to right, so the file holds the word in
    // visual order, as a PDF holds any right-to-left text. A reader restores
    // the logical order with its own bidi pass. `pdf-extract` runs none, so
    // the test reverses what it reads.
    let read = compact(&read_back(&document));
    let logical: String = read.chars().rev().collect();
    assert_eq!(logical, text);
}

#[test]
fn the_live_surface_reads_back_as_the_recorded_one() {
    let text = "office fit e\u{301}";
    let mut font_cx = shipped_font_context();
    let layout = shape(&mut font_cx, text, "DejaVu Serif", None);
    let mut pdf = krilla::Document::new();
    let mut page = pdf
        .start_page_with(krilla::page::PageSettings::from_wh(600.0, 200.0).expect("a page size"));
    {
        let mut surface = page.surface();
        let mut live = KrillaSurface::new(&mut surface);
        draw(&mut live, &layout, LayoutText::whole(text));
    }
    page.finish();
    let bytes = pdf.finish().expect("the page writes");
    let live = pdf_extract::extract_text_from_mem(&bytes).expect("the file reads");
    assert_eq!(compact(&live), compact(text));
    assert_eq!(
        compact(&read_back(&record(text, "DejaVu Serif"))),
        compact(text)
    );
}

#[test]
fn a_text_that_is_not_the_layouts_own_draws_shapes_alone() {
    let mut font_cx = shipped_font_context();
    let layout = shape(&mut font_cx, "Ambre", "Noto Sans", None);
    let mut document = PrintDocument::new(page());
    let mut tables = RecorderTables::default();
    let mut recorder = ListRecorder::new(&mut document, &mut tables);
    // Two bytes where the layout counted five: the clusters fall past the end.
    draw(&mut recorder, &layout, LayoutText::whole("Am"));
    recorder.finish();
    assert_eq!(glyph_count(&document), 5, "the shapes are still drawn");
    assert_eq!(
        read_back(&document).trim(),
        "",
        "and no character is claimed"
    );
}

#[test]
fn a_tail_of_the_text_serves_the_lines_it_holds() {
    // A run re-shaped from a line on keeps only the text from that line. The
    // second line reads the same from the tail as from the whole text.
    let text = "Ambre exports text";
    let mut font_cx = shipped_font_context();
    let layout = shape(&mut font_cx, text, "Noto Sans", Some(80.0));
    let lines: Vec<_> = layout.lines().collect();
    let last = lines.last().expect("a broken layout has lines");
    let start = last.text_range().start;
    assert!(start > 0, "the text broke into several lines");
    let tail = LayoutText::from_byte(&text[start..], Pos::from_usize(start));
    for item in last.items() {
        if let PositionedLayoutItem::GlyphRun(run) = item {
            let whole = run_text(&run, LayoutText::whole(text)).expect("the whole text maps");
            assert_eq!(run_text(&run, tail), Some(whole.clone()));
            assert!(text.ends_with(whole.text.trim_end()));
            // A line before the tail cannot read from it.
            let first = lines.first().expect("a first line");
            for item in first.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    assert_eq!(run_text(&run, tail), None);
                }
            }
        }
    }
}
