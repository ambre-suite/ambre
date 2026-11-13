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

#![expect(clippy::panic, reason = "a bench aborts on a fixture it cannot run")]

use ambre_core::index::Len;
use std::hint::black_box;

use ambre_core::{Doc, Px};
use ambre_draw::ParleyBrush;
use ambre_prose::doc::DocDefaults;
use ambre_prose::layout::fonts::register_faces;
use ambre_prose::layout::measure::{EffectiveStyle, Measurer, reshape_paragraph};
use ambre_prose::layout::run::{ParleyRunData, RunLine, ShapedRun};
use ambre_prose::message::ParagraphStyle;
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
            if run.data().segment_count() == Len::<ambre_prose::layout::run::RunSegment>::ONE {
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
    if run.data().line_count() < Len::<RunLine>::new(2) {
        panic!("a long paragraph breaks into lines");
    }
}

/// The blocks of the document the undo rows type into: about a hundred pages
/// of paragraphs of `PARAGRAPH_CHARS` characters each.
const PAGE_BLOCKS: usize = 300;

/// A document of `PAGE_BLOCKS` paragraphs, each inserted and typed by its own
/// commands. One key typed into the middle one is the edit an undo takes back.
fn typed_document() -> ambre_prose::logic::ProseState {
    use ambre_core::index::Pos;
    use ambre_prose::message::{BlockInit, DocChar, ProseCommand};
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let mut last = state.root_blocks().first().copied();
    for i in 0..PAGE_BLOCKS {
        let before = state.root_blocks();
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: last,
            init: BlockInit::Paragraph,
            style_id: None,
        });
        let Some(block) = state
            .root_blocks()
            .into_iter()
            .find(|id| !before.contains(id))
        else {
            panic!("an insert adds a block");
        };
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: paragraph(WORDS.get(i % WORDS.len()).copied().unwrap_or("flowing")),
        });
        last = Some(block);
    }
    let Some(middle) = state.root_blocks().get(PAGE_BLOCKS / 2).copied() else {
        panic!("the document holds its paragraphs");
    };
    state.handle_command(ProseCommand::InsertText {
        block_id: middle,
        pos: Pos::<DocChar>::new(250),
        text: "k".to_owned(),
    });
    // The view lays the page out after each answer, as the Logic thread
    // does, so the undo starts from a laid-out document.
    let _ = state.paginate(&[]);
    state
}

/// One `Undo` of a typed key, from the command to its outcome: the replay,
/// the commit, the caret and the delta. Counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_prose_key_undo(state: &mut ambre_prose::logic::ProseState) {
    let out = state.handle_command(black_box(ambre_prose::message::ProseCommand::Undo));
    assert!(out.update.is_some(), "the undo takes the key out");
}

/// One `Redo` of that key, counted the same way.
#[inline(never)]
fn ambre_budget_prose_key_redo(state: &mut ambre_prose::logic::ProseState) {
    let out = state.handle_command(black_box(ambre_prose::message::ProseCommand::Redo));
    assert!(out.update.is_some(), "the redo types the key again");
}

/// The undo of one key on a document of about a hundred pages, the "Prose
/// local text insert" row of `ambre-spec-budgets.md section 1`.
fn key_undo() {
    let mut state = typed_document();
    ambre_budget_prose_key_undo(&mut state);
}

/// The redo of that key, after its undo outside the count.
fn key_redo() {
    let mut state = typed_document();
    let out = state.handle_command(ambre_prose::message::ProseCommand::Undo);
    assert!(out.update.is_some(), "the undo takes the key out");
    let _ = state.paginate(&[]);
    ambre_budget_prose_key_redo(&mut state);
}

/// The paragraphs of the Markdown file the slice rows import. Each holds
/// about seventy words and a bold run, and a heading opens every twenty.
const IMPORTED_PARAGRAPHS: usize = 2_000;

/// A Markdown file of `paragraphs` paragraphs, as a novel exported from a
/// word processor reads.
fn novel(paragraphs: usize) -> Vec<u8> {
    let mut text = String::new();
    for i in 0..paragraphs {
        if i % 20 == 0 {
            text.push_str(&format!("## Chapter {}\n\n", i / 20 + 1));
        }
        text.push_str(
            "The river ran **past the mill** and under the old bridge, where the \
             children of the village met each evening to watch the boats come in \
             from the sea. Nobody remembered who had built the bridge, or when, but \
             everyone agreed that it had always been there, and that it would stay \
             there long after the mill had fallen and the boats had gone.\n\n",
        );
    }
    text.into_bytes()
}

/// The markup engine the host links in place.
fn markup() -> ambre_convert::Converter {
    match ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Markup,
        &ambre_core::Payload::new(Vec::new()),
    ) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the markup engine: {e}"),
    }
}

/// The import of `data` as Markdown.
fn import_command(data: Vec<u8>) -> ambre_prose::message::ProseCommand {
    use ambre_prose::message::{ImportFormat, ProseCommand, ProseImport, TimeZone};
    ProseCommand::Interop(ambre_bus::interop::InteropRequest::Import(ProseImport {
        format: ImportFormat::Markdown,
        data: ambre_core::Payload::<ambre_core::FileBytes>::new(data),
        now: ambre_core::Instant::<ambre_core::Authored>::new(1_800_000_000_000),
        zone: TimeZone::UTC,
    }))
}

/// The progress a slice reported, if it reported one.
fn progress_of(
    outcome: &ambre_prose::ProseOutcome,
) -> Option<ambre_bus::interop::ConversionProgress> {
    outcome.display.iter().find_map(|event| match event {
        ambre_prose::ProseEvent::Interop(ambre_bus::interop::InteropAnswer::Progress(progress)) => {
            Some(*progress)
        }
        _ => None,
    })
}

/// One slice of a running import, counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_prose_import_slice(
    state: &mut ambre_prose::logic::ProseState,
) -> ambre_prose::ProseOutcome {
    state.resume_conversion(None)
}

/// One slice of a running export, counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_prose_export_slice(
    state: &mut ambre_prose::logic::ProseState,
    converter: &ambre_convert::Converter,
) -> ambre_prose::ProseOutcome {
    state.resume_conversion(Some(converter))
}

/// A slice of the write into the document, the dearer half of an import.
/// Each block lands with its text, its history record and its delta. The
/// slice counted is the one past the middle of the write, so the document
/// already holds a thousand blocks of the file.
fn import_slice() {
    let converter = markup();
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let first = state.handle_conversion(import_command(novel(IMPORTED_PARAGRAPHS)), Ok(&converter));
    let Some(started) = progress_of(&first) else {
        panic!("a large import runs in slices");
    };
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion(None)).is_some_and(|p| p.done().get() < middle) {}
    let out = ambre_budget_prose_import_slice(black_box(&mut state));
    assert!(out.update.is_some(), "a slice of the write sends its delta");
    assert!(
        progress_of(&out).is_some(),
        "the slice counted is not the last"
    );
}

/// A slice of the walk of an export, over a document of the imported file.
/// The slice counted is the one past the middle of the walk.
fn export_slice() {
    use ambre_prose::message::{ExportFormat, MarkdownFlavor, ProseCommand};
    let converter = markup();
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let _ = state.handle_conversion(import_command(novel(IMPORTED_PARAGRAPHS)), Ok(&converter));
    let _ = state.settle_conversion(None);
    let export = ProseCommand::Interop(ambre_bus::interop::InteropRequest::Export(
        ExportFormat::Markdown(MarkdownFlavor::Gfm),
    ));
    let first = state.handle_conversion(export, Ok(&converter));
    let Some(started) = progress_of(&first) else {
        panic!("a large export runs in slices");
    };
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion(Some(&converter)))
        .is_some_and(|p| p.done().get() < middle)
    {}
    let out = ambre_budget_prose_export_slice(black_box(&mut state), &converter);
    assert!(
        progress_of(&out).is_some(),
        "the slice counted is not the last"
    );
}

/// One slice of a running PDF export. `scripts/instructions.py` counts each
/// call apart: from a dump at its entry to a dump at the entry of
/// [`ambre_pass_end_prose_print`], called right after it.
#[inline(never)]
fn ambre_budget_prose_print_slice(
    state: &mut ambre_prose::logic::ProseState,
    converter: &ambre_convert::Converter,
) -> ambre_prose::ProseOutcome {
    // A body unlike the export slice's: two identical functions are merged
    // into one symbol, and `callgrind` then counts nothing in one of them.
    let out = state.resume_conversion(Some(converter));
    black_box(state.has_pending_work());
    out
}

/// The end of one counted slice, and the pages the export has drawn after
/// it. Never inlined: the dump at its entry closes the count of the slice.
#[inline(never)]
fn ambre_pass_end_prose_print(
    out: &ambre_prose::ProseOutcome,
) -> Option<ambre_bus::interop::ConversionProgress> {
    progress_of(black_box(out))
}

/// Count each slice of a running PDF export until `pages` pages are drawn.
fn print_slices_until(
    state: &mut ambre_prose::logic::ProseState,
    converter: &ambre_convert::Converter,
    pages: u32,
) {
    loop {
        let out = ambre_budget_prose_print_slice(black_box(state), converter);
        let Some(progress) = ambre_pass_end_prose_print(&out) else {
            panic!("the export ended before page {pages}");
        };
        if progress.done().get() >= pages {
            return;
        }
    }
}

/// Every slice of the drawing of two pages of a PDF export, over a document
/// of the imported file on the shipped faces. Page 1 is near the view, so
/// its runs are held. A page past the middle is far from it, so its runs
/// were dropped and are shaped again over several slices (`PROSE-136`).
fn print_slice() {
    print_slice_of(Pictures::None);
}

/// The same slices, with pictures among the blocks of the first pages: each
/// one is handed to the recorder as its stored file, decoded nowhere.
fn print_slice_picture() {
    print_slice_of(Pictures::FirstPages);
}

/// Whether the document of a print slice shows pictures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pictures {
    None,
    /// One after every second block of the first pages, so page 1 shows one
    /// whatever its breaks.
    FirstPages,
}

fn print_slice_of(pictures: Pictures) {
    use ambre_prose::message::{ExportFormat, PdfExport, ProseCommand};
    let markup = markup();
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let faces: Vec<_> = faces()
        .into_iter()
        .map(ambre_core::Payload::<ambre_core::FontBytes>::new)
        .collect();
    state.load_faces(&faces);
    let _ = state.handle_conversion(import_command(novel(IMPORTED_PARAGRAPHS)), Ok(&markup));
    let _ = state.settle_conversion(None);
    let _ = state.layout_event();
    let pictured = pictures == Pictures::FirstPages;
    if pictured {
        let roots = state.root_blocks();
        for after in roots.iter().take(40).step_by(2) {
            let _ = state.handle_command(ProseCommand::InsertImage {
                parent_id: None,
                after_id: Some(*after),
                content_hash: picture_address(),
                alt: "a picture".to_owned(),
                size: ambre_core::Size::new(ambre_core::Px::new(320.0), ambre_core::Px::new(80.0)),
            });
        }
        let _ = state.take_media_requests();
    }
    let pdf = match ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Pdf,
        &ambre_core::Payload::new(Vec::new()),
    ) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the PDF engine: {e}"),
    };
    let export = ProseCommand::Interop(ambre_bus::interop::InteropRequest::Export(
        ExportFormat::Pdf(PdfExport::default()),
    ));
    let mut first = state.handle_conversion(export, Ok(&pdf));
    if pictured {
        // The export asks for its pictures first; the Sync worker answers.
        for request in state.take_media_requests() {
            if let ambre_prose::message::MediaRequest::Download { id, content_hash } = request {
                let _ = state.handle_media_event(
                    id,
                    ambre_bus::RestEvent::MediaDownloaded {
                        document_id: ambre_core::id::Id::from_bytes([1; 16]),
                        content_hash,
                        data: ambre_core::Payload::new(picture_file()),
                    },
                );
            }
        }
        first = state.resume_conversion(Some(&pdf));
    }
    let Some(started) = progress_of(&first) else {
        panic!("a large export runs in slices");
    };
    assert_eq!(started.done().get(), 1, "the first slice draws page 0");
    print_slices_until(&mut state, &pdf, 2);
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion(Some(&pdf))).is_some_and(|p| p.done().get() < middle)
    {
    }
    print_slices_until(&mut state, &pdf, middle + 1);
}

/// The address the pictures of the picture slice hold.
fn picture_address() -> ambre_core::Validated<ambre_core::ContentHash> {
    match ambre_core::Validated::<ambre_core::ContentHash>::parse("e".repeat(64)) {
        Ok(address) => address,
        Err(e) => panic!("a hex address: {e}"),
    }
}

/// The file the Sync worker answers for them: a PNG of 64 by 16 pixels.
fn picture_file() -> Vec<u8> {
    let mut bytes = Vec::new();
    let encoded = image::RgbaImage::from_pixel(64, 16, image::Rgba([90, 120, 200, 255])).write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    );
    if let Err(e) = encoded {
        panic!("a PNG encodes: {e}");
    }
    bytes
}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn()); 7] = [
    ("paragraph_shape", paragraph_shape),
    ("key_undo", key_undo),
    ("key_redo", key_redo),
    ("import_slice", import_slice),
    ("export_slice", export_slice),
    ("print_slice", print_slice),
    ("print_slice_picture", print_slice_picture),
];

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
