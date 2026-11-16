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
        let face =
            ambre_draw::FontFace::new(ambre_core::Payload::<ambre_core::FontBytes>::new(bytes));
        measurer.register_faces(&face);
        register_faces(&mut font_cx, &face);
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

/// `command` through [`ambre_prose::logic::ProseState::handle_conversion`],
/// then each file conversion it owes the converter thread, run in place as
/// the thread runs it. The outcome is the last one.
fn converted(
    state: &mut ambre_prose::logic::ProseState,
    command: ambre_prose::message::ProseCommand,
    converter: &ambre_convert::Converter,
) -> ambre_prose::ProseOutcome {
    let mut out = state.handle_conversion(command, Ok(converter));
    for (ticket, conversion) in state.take_file_requests() {
        if let Some(next) = state.handle_file_converted(ticket, converter.convert(&conversion)) {
            out = next;
        }
    }
    out
}

/// The import of `data` as Markdown.
fn import_command(data: Vec<u8>) -> ambre_prose::message::ProseCommand {
    use ambre_prose::message::{ImportFormat, ProseCommand, ProseImport, TimeZone};
    ProseCommand::Interop(ambre_bus::interop::InteropRequest::Import(ProseImport {
        format: ImportFormat::Markdown,
        data: ambre_core::Payload::<ambre_client_core::FileBytes>::new(data),
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
    state.resume_conversion()
}

/// One slice of a running export, counted alone by `callgrind`.
#[inline(never)]
fn ambre_budget_prose_export_slice(
    state: &mut ambre_prose::logic::ProseState,
) -> ambre_prose::ProseOutcome {
    // A body unlike the import slice's: two identical functions are merged
    // into one symbol, and `callgrind` then counts nothing in one of them.
    let out = state.resume_conversion();
    black_box(out.update.is_some());
    out
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
    let first = converted(
        &mut state,
        import_command(novel(IMPORTED_PARAGRAPHS)),
        &converter,
    );
    let Some(started) = progress_of(&first) else {
        panic!("a large import runs in slices");
    };
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion()).is_some_and(|p| p.done().get() < middle) {}
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
    let _ = converted(
        &mut state,
        import_command(novel(IMPORTED_PARAGRAPHS)),
        &converter,
    );
    let _ = state.settle_conversion();
    let export = ProseCommand::Interop(ambre_bus::interop::InteropRequest::Export(
        ExportFormat::Markdown(MarkdownFlavor::Gfm),
    ));
    let first = state.handle_conversion(export, Ok(&converter));
    let Some(started) = progress_of(&first) else {
        panic!("a large export runs in slices");
    };
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion()).is_some_and(|p| p.done().get() < middle) {}
    let out = ambre_budget_prose_export_slice(black_box(&mut state));
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
) -> ambre_prose::ProseOutcome {
    // A body unlike the import and export slices': two identical functions
    // are merged into one symbol, and `callgrind` then counts nothing in one
    // of them.
    let out = state.resume_conversion();
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

/// [`ambre_budget_prose_print_slice`], for the pages with pictures. Each
/// operation counts in functions of its own, so no dump of one closes a
/// call of another; the tag keeps the bodies apart.
#[inline(never)]
fn ambre_budget_prose_picture_print(
    state: &mut ambre_prose::logic::ProseState,
) -> ambre_prose::ProseOutcome {
    black_box(Printed::PicturedFirstPages);
    let out = state.resume_conversion();
    black_box(state.has_pending_work());
    out
}

/// [`ambre_pass_end_prose_print`], for the pages with pictures.
#[inline(never)]
fn ambre_pass_end_prose_picture(
    out: &ambre_prose::ProseOutcome,
) -> Option<ambre_bus::interop::ConversionProgress> {
    black_box(Printed::PicturedFirstPages);
    progress_of(black_box(out))
}

/// [`ambre_budget_prose_print_slice`], for the dense pages.
#[inline(never)]
fn ambre_budget_prose_dense_print(
    state: &mut ambre_prose::logic::ProseState,
) -> ambre_prose::ProseOutcome {
    black_box(Printed::Dense);
    let out = state.resume_conversion();
    black_box(state.has_pending_work());
    out
}

/// [`ambre_pass_end_prose_print`], for the dense pages.
#[inline(never)]
fn ambre_pass_end_prose_dense(
    out: &ambre_prose::ProseOutcome,
) -> Option<ambre_bus::interop::ConversionProgress> {
    black_box(Printed::Dense);
    progress_of(black_box(out))
}

/// The two functions one operation counts a slice between: the slice, and
/// the end called right after it.
#[derive(Clone, Copy)]
struct Counted {
    slice: fn(&mut ambre_prose::logic::ProseState) -> ambre_prose::ProseOutcome,
    end: fn(&ambre_prose::ProseOutcome) -> Option<ambre_bus::interop::ConversionProgress>,
}

/// Count each slice of a running PDF export until `pages` pages are drawn.
fn print_slices_until(
    state: &mut ambre_prose::logic::ProseState,
    counted: Counted,
    pages: u32,
) {
    loop {
        let out = (counted.slice)(black_box(state));
        let Some(progress) = (counted.end)(&out) else {
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
    print_slice_of(Printed::Plain);
}

/// The same slices, with pictures among the blocks of the first pages: each
/// one is handed to the recorder as its stored file, decoded nowhere.
fn print_slice_picture() {
    print_slice_of(Printed::PicturedFirstPages);
}

/// The same slices, over pages dense with text and pictures. Each paragraph
/// is short and changes its style every few words. A picture follows every
/// third block of the document, near page and far page alike.
fn print_slice_dense() {
    print_slice_of(Printed::Dense);
}

/// What the pages of a print slice hold.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Printed {
    /// The paragraphs of [`novel`], one bold run each.
    Plain,
    /// The same paragraphs, and one picture after every second block of the
    /// first pages, so page 1 shows one whatever its breaks.
    PicturedFirstPages,
    /// The paragraphs of [`dense_novel`], and a picture after every third
    /// block of the document.
    Dense,
}

impl Printed {
    /// The functions this operation counts its slices between.
    fn counted(self) -> Counted {
        match self {
            Self::Plain => Counted {
                slice: ambre_budget_prose_print_slice,
                end: ambre_pass_end_prose_print,
            },
            Self::PicturedFirstPages => Counted {
                slice: ambre_budget_prose_picture_print,
                end: ambre_pass_end_prose_picture,
            },
            Self::Dense => Counted {
                slice: ambre_budget_prose_dense_print,
                end: ambre_pass_end_prose_dense,
            },
        }
    }

    /// The Markdown file the document imports.
    fn file(self) -> Vec<u8> {
        match self {
            Self::Plain | Self::PicturedFirstPages => novel(IMPORTED_PARAGRAPHS),
            Self::Dense => dense_novel(IMPORTED_PARAGRAPHS),
        }
    }

    /// The blocks a picture follows, among `roots`.
    fn pictured(
        self,
        roots: &[ambre_prose::message::BlockId],
    ) -> Vec<ambre_prose::message::BlockId> {
        match self {
            Self::Plain => Vec::new(),
            Self::PicturedFirstPages => roots.iter().take(40).step_by(2).copied().collect(),
            Self::Dense => roots.iter().step_by(3).copied().collect(),
        }
    }

    /// The size each picture is laid out at, in pixels.
    fn picture_size(self) -> ambre_core::Size<Px<Doc>> {
        let height = match self {
            Self::Plain | Self::PicturedFirstPages => 80.0,
            // Low, so a page holds more blocks of text beside its pictures.
            Self::Dense => 48.0,
        };
        ambre_core::Size::new(Px::new(320.0), Px::new(height))
    }
}

/// A Markdown file of `paragraphs` short paragraphs, each changing its style
/// every few words: bold, italic, struck, code and a link a reader may open.
/// Every change starts a run of glyphs of its own, the most a line draws.
fn dense_novel(paragraphs: usize) -> Vec<u8> {
    let mut text = String::new();
    for i in 0..paragraphs {
        if i % 20 == 0 {
            text.push_str(&format!("## Chapter {}\n\n", i / 20 + 1));
        }
        text.push_str(
            "The **river ran** past *the mill* and ~~under~~ the `old bridge`, \
             where [the children](https://example.org/village) met **each** \
             *evening* to `watch` the ~~boats~~ come [in](https://example.org/sea) \
             from **the sea**.\n\n",
        );
    }
    text.into_bytes()
}

fn print_slice_of(printed: Printed) {
    use ambre_prose::message::{ExportFormat, PdfExport, ProseCommand};
    let markup = markup();
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let faces: Vec<_> = faces()
        .into_iter()
        .map(ambre_core::Payload::<ambre_core::FontBytes>::new)
        .map(ambre_draw::FontFace::new)
        .collect();
    state.load_faces(&faces);
    let _ = converted(&mut state, import_command(printed.file()), &markup);
    let _ = state.settle_conversion();
    let _ = state.layout_event();
    let pictured = printed.pictured(&state.root_blocks());
    for after in &pictured {
        let _ = state.handle_command(ProseCommand::InsertImage {
            parent_id: None,
            after_id: Some(*after),
            content_hash: picture_address(),
            alt: ambre_core::Validated::new("a picture"),
            size: printed.picture_size(),
        });
    }
    let _ = state.take_media_requests();
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
    if !pictured.is_empty() {
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
        first = state.resume_conversion();
    }
    let Some(started) = progress_of(&first) else {
        panic!("a large export runs in slices");
    };
    assert_eq!(started.done().get(), 1, "the first slice draws page 0");
    print_slices_until(&mut state, printed.counted(), 2);
    let middle = started.total().get() / 2;
    while progress_of(&state.resume_conversion()).is_some_and(|p| p.done().get() < middle) {}
    print_slices_until(&mut state, printed.counted(), middle + 1);
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

/// The Logic thread's half of a picture that arrives. It reads the header
/// and holds it to the source bound, then hands the decode to the converter
/// thread. The count leaves the decode itself out (`PROSE-FEAT-330`).
fn arrive(
    state: &mut ambre_prose::logic::ProseState,
    id: ambre_core::Seq<ambre_bus::rest::Request>,
    data: ambre_core::Payload<ambre_core::MediaBytes>,
) {
    let outcome = state.handle_media_event(
        id,
        ambre_bus::RestEvent::MediaDownloaded {
            document_id: ambre_core::id::Id::from_bytes([1; 16]),
            content_hash: picture_address(),
            data,
        },
    );
    std::hint::black_box(outcome);
}

/// [`arrive`] for a JPEG, counted alone. Each of the three counted
/// functions names its format to `black_box`, so the linker never folds two
/// identical bodies into one symbol.
#[inline(never)]
fn ambre_budget_prose_picture_arrive(
    state: &mut ambre_prose::logic::ProseState,
    id: ambre_core::Seq<ambre_bus::rest::Request>,
    data: ambre_core::Payload<ambre_core::MediaBytes>,
) {
    std::hint::black_box("jpeg");
    arrive(state, id, data);
}

/// [`arrive`] for a PNG, counted alone (`PROSE-FEAT-332`).
#[inline(never)]
fn ambre_budget_prose_png_arrive(
    state: &mut ambre_prose::logic::ProseState,
    id: ambre_core::Seq<ambre_bus::rest::Request>,
    data: ambre_core::Payload<ambre_core::MediaBytes>,
) {
    std::hint::black_box("png");
    arrive(state, id, data);
}

/// [`arrive`] for a GIF, counted alone (`PROSE-FEAT-332`).
#[inline(never)]
fn ambre_budget_prose_gif_arrive(
    state: &mut ambre_prose::logic::ProseState,
    id: ambre_core::Seq<ambre_bus::rest::Request>,
    data: ambre_core::Payload<ambre_core::MediaBytes>,
) {
    std::hint::black_box("gif");
    arrive(state, id, data);
}

/// A page that shows one picture, and the ticket of the download its layout
/// asked for.
fn picture_page() -> (
    ambre_prose::logic::ProseState,
    ambre_core::Seq<ambre_bus::rest::Request>,
) {
    use ambre_prose::message::ProseCommand;
    let Some(mut state) = ambre_prose::logic::ProseState::with_peer(ambre_core::PeerId::new(1))
    else {
        panic!("a fresh document builds");
    };
    let first = state.root_blocks().first().copied();
    let _ = state.handle_command(ProseCommand::InsertImage {
        parent_id: None,
        after_id: first,
        content_hash: picture_address(),
        alt: ambre_core::Validated::new("a photograph"),
        size: ambre_core::Size::new(ambre_core::Px::new(640.0), ambre_core::Px::new(480.0)),
    });
    let _ = state.take_media_requests();
    let _ = state.layout_event();
    let Some(id) = state
        .take_media_requests()
        .into_iter()
        .find_map(|r| match r {
            ambre_prose::message::MediaRequest::Download { id, .. } => Some(id),
            ambre_prose::message::MediaRequest::Upload { .. } => None,
        })
    else {
        panic!("the layout asks for the picture");
    };
    (state, id)
}

/// A photograph of 4032 by 3024 pixels, with detail in every pixel.
fn photo() -> image::RgbImage {
    image::RgbImage::from_fn(4032, 3024, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])
    })
}

/// A JPEG photograph of 4032 by 3024 pixels arrives on a page that shows it.
fn picture_arrive() {
    let (mut state, id) = picture_page();
    let mut bytes = Vec::new();
    if let Err(e) = image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode_image(&photo()) {
        panic!("a JPEG encodes: {e}");
    }
    ambre_budget_prose_picture_arrive(&mut state, id, ambre_core::Payload::new(bytes));
    assert_eq!(state.take_decode_requests().len(), 1, "the decode leaves");
}

/// The same photograph arrives as a PNG.
fn picture_arrive_png() {
    let (mut state, id) = picture_page();
    let mut bytes = Vec::new();
    if let Err(e) = photo().write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    ) {
        panic!("a PNG encodes: {e}");
    }
    ambre_budget_prose_png_arrive(&mut state, id, ambre_core::Payload::new(bytes));
    assert_eq!(state.take_decode_requests().len(), 1, "the decode leaves");
}

/// A GIF of ten frames of 640 by 480 arrives on a page that shows it.
fn picture_arrive_gif() {
    let (mut state, id) = picture_page();
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut bytes, 30);
        for step in 0..10u32 {
            let frame = image::RgbaImage::from_fn(640, 480, |x, y| {
                image::Rgba([((x + step * 8) % 256) as u8, (y % 256) as u8, 90, 255])
            });
            if let Err(e) = encoder.encode_frame(image::Frame::new(frame)) {
                panic!("a GIF frame encodes: {e}");
            }
        }
    }
    ambre_budget_prose_gif_arrive(&mut state, id, ambre_core::Payload::new(bytes));
    assert_eq!(state.take_decode_requests().len(), 1, "the decode leaves");
}

/// Each operation, by the name `scripts/instructions.py` passes.
const OPERATIONS: [(&str, fn()); 11] = [
    ("paragraph_shape", paragraph_shape),
    ("key_undo", key_undo),
    ("key_redo", key_redo),
    ("import_slice", import_slice),
    ("export_slice", export_slice),
    ("print_slice", print_slice),
    ("print_slice_picture", print_slice_picture),
    ("print_slice_dense", print_slice_dense),
    ("picture_arrive", picture_arrive),
    ("picture_arrive_png", picture_arrive_png),
    ("picture_arrive_gif", picture_arrive_gif),
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
