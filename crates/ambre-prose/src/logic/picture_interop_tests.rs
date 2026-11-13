//! A picture through an import and an export
//! (`features/ambre-spec-prose-images.md`, sections 1.8 and 1.9).
//!
//! Each test drives the media flow as the Sync worker answers it: an upload
//! answers an address, a download answers the bytes or an error.

// The codecs read a file back on the host alone.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use ambre_bus::RestEvent;
use ambre_bus::interop::{InteropAnswer, InteropRequest, Outcome};
use ambre_core::{ContentHash, Payload, Validated};
use ambre_docx::WordDocument;
use ambre_docx::model::{
    Alignment, Block, Image, ImageFormat, Inline, Paragraph, Table, TableCell, TableRow,
};

use super::ProseState;
use crate::events::ProseEvent;
use crate::logic::tests::{hash_of, png};
use crate::message::{
    BlockType, ConversionReport, ExportFormat, ImportFormat, MediaRequest, ProseCommand,
    ProseConstruct, ProseImport,
};

fn picture(alt: &str, bytes: Vec<u8>) -> Inline {
    Inline::Image(Image {
        description: alt.to_owned(),
        width: ambre_core::PrintLength::new(150.0),
        height: ambre_core::PrintLength::new(75.0),
        format: ImageFormat::Png,
        bytes: Payload::new(bytes),
    })
}

fn paragraph(inlines: Vec<Inline>) -> Block {
    Block::Paragraph(Paragraph {
        inlines,
        ..Paragraph::default()
    })
}

fn word(body: Vec<Block>) -> Vec<u8> {
    ambre_docx::write(&WordDocument {
        body,
        ..WordDocument::default()
    })
    .expect("the model writes")
    .into_bytes()
}

fn document_id() -> ambre_core::id::Id<ambre_core::id::Document> {
    ambre_core::id::Id::from_bytes([1; 16])
}

/// Start the import of `bytes` as `format`.
fn start_import(
    state: &mut ProseState,
    format: ImportFormat,
    bytes: Vec<u8>,
) -> crate::events::ProseOutcome {
    state.handle_converted_in_family(ProseCommand::Interop(InteropRequest::Import(ProseImport {
        format,
        data: Payload::new(bytes),
        now: ambre_core::Instant::<ambre_core::Authored>::new(1_800_000_000_000),
        zone: crate::message::TimeZone::UTC,
    })))
}

/// The uploads the document owes, by request, with their bytes.
fn uploads(state: &mut ProseState) -> Vec<(ambre_core::Seq<ambre_bus::rest::Request>, Vec<u8>)> {
    state
        .take_media_requests()
        .into_iter()
        .filter_map(|request| match request {
            MediaRequest::Upload { id, data } => Some((id, data.into_bytes())),
            MediaRequest::Download { .. } => None,
        })
        .collect()
}

/// Run the conversion the answers started to its end, and hand back every
/// event it answered.
fn drain(state: &mut ProseState, family: Option<ambre_core::ConverterFamily>) -> Vec<ProseEvent> {
    let converter = family.map(|family| {
        ambre_convert::Converter::load(family, &Payload::new(Vec::new()))
            .unwrap_or_else(|e| panic!("the host links the {family} engine: {e}"))
    });
    let mut events = Vec::new();
    while state.has_pending_work() {
        events.extend(state.resume_conversion(converter.as_ref()).display);
    }
    events
}

fn report_of(events: &[ProseEvent]) -> ConversionReport {
    events
        .iter()
        .find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ImportCompleted { report }) => Some(report.clone()),
            _ => None,
        })
        .expect("the import completes")
}

fn count(report: &ConversionReport, construct: ProseConstruct, outcome: Outcome) -> u32 {
    report
        .entries()
        .iter()
        .filter(|entry| entry.construct == construct && entry.outcome == outcome)
        .map(|entry| entry.count.get())
        .sum()
}

fn pictures_of(state: &ProseState) -> Vec<crate::picture::ImageRecord> {
    state
        .root_blocks()
        .into_iter()
        .filter_map(|id| state.image_record(&crate::events::tree_id(id)))
        .collect()
}

/// `PROSE-FEAT-319`, `PROSE-FEAT-206`: an import stores each distinct
/// picture once, writes nothing before the last address answers, then
/// lands whole. Two copies of one picture hold one address.
#[test]
fn an_import_lands_once_every_picture_is_stored() {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    let red = png(4, 2);
    let blue = png(3, 3);
    let file = word(vec![
        paragraph(vec![picture("first", red.clone())]),
        paragraph(vec![Inline::Run(ambre_docx::model::Run::plain("between"))]),
        paragraph(vec![picture("again", red.clone())]),
        paragraph(vec![picture("other", blue.clone())]),
    ]);
    let started = start_import(&mut state, ImportFormat::Word, file);
    assert!(
        started.update.is_none(),
        "nothing written before the addresses"
    );
    let asked = uploads(&mut state);
    assert_eq!(asked.len(), 2, "one upload per distinct picture");
    let before = state.export_snapshot();
    let addresses: HashMap<Vec<u8>, Validated<ContentHash>> =
        [(red, hash_of('1')), (blue, hash_of('2'))]
            .into_iter()
            .collect();
    let (first, rest) = asked.split_first().expect("two uploads");
    let answer = |state: &mut ProseState, (id, data): &(_, Vec<u8>)| {
        state.handle_media_event(
            *id,
            RestEvent::MediaUploaded {
                document_id: document_id(),
                content_hash: addresses[data].clone(),
            },
        )
    };
    answer(&mut state, first);
    assert_eq!(
        state.export_snapshot(),
        before,
        "one answer of two writes nothing"
    );
    let mut events = Vec::new();
    for upload in rest {
        events.extend(answer(&mut state, upload).expect("this document's").display);
    }
    events.extend(drain(&mut state, None));
    let report = report_of(&events);
    let held: Vec<_> = pictures_of(&state)
        .into_iter()
        .map(|record| (record.alt, record.address))
        .collect();
    assert_eq!(
        held,
        [
            ("first".to_owned(), Some(hash_of('1'))),
            ("again".to_owned(), Some(hash_of('1'))),
            ("other".to_owned(), Some(hash_of('2'))),
        ]
    );
    assert_eq!(
        count(
            &report,
            ProseConstruct::Element(crate::message::DroppedElement::Image),
            Outcome::Dropped
        ),
        0
    );
}

/// `PROSE-FEAT-321`: a picture whose upload failed is left out and named,
/// and the rest of the file lands.
#[test]
fn a_picture_whose_upload_failed_is_left_out_and_named() {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    let file = word(vec![
        paragraph(vec![picture("kept", png(2, 2))]),
        paragraph(vec![Inline::Run(ambre_docx::model::Run::plain(
            "text stays",
        ))]),
        paragraph(vec![picture("lost", png(5, 5))]),
    ]);
    start_import(&mut state, ImportFormat::Word, file);
    let asked = uploads(&mut state);
    let lost = png(5, 5);
    let mut events = Vec::new();
    for (id, data) in asked {
        let outcome = if data == lost {
            state.handle_media_error(id)
        } else {
            state.handle_media_event(
                id,
                RestEvent::MediaUploaded {
                    document_id: document_id(),
                    content_hash: hash_of('7'),
                },
            )
        };
        events.extend(outcome.expect("this document's").display);
    }
    events.extend(drain(&mut state, None));
    let report = report_of(&events);
    let alts: Vec<_> = pictures_of(&state)
        .into_iter()
        .map(|record| record.alt)
        .collect();
    assert_eq!(alts, ["kept"]);
    assert!(
        state
            .root_blocks()
            .iter()
            .any(|id| state.text_of(id).as_deref() == Some("text stays"))
    );
    assert_eq!(
        count(
            &report,
            ProseConstruct::Element(crate::message::DroppedElement::Image),
            Outcome::Dropped
        ),
        1
    );
}

/// `PROSE-FEAT-053`, `PROSE-FEAT-225`, `PROSE-FEAT-226`: a picture in a
/// table cell lands beside the table, never dropped, and the report names
/// the lift. Its address, size and alternative text are those it had.
#[test]
fn a_picture_in_a_cell_lands_beside_the_table_and_is_named() {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    let file = word(vec![Block::Table(Table {
        columns: Vec::new(),
        rows: vec![TableRow {
            cells: vec![TableCell {
                blocks: vec![paragraph(vec![picture("in a cell", png(2, 2))])],
                ..TableCell::default()
            }],
            ..TableRow::default()
        }],
    })]);
    start_import(&mut state, ImportFormat::Word, file);
    let mut events = Vec::new();
    for (id, _) in uploads(&mut state) {
        events.extend(
            state
                .handle_media_event(
                    id,
                    RestEvent::MediaUploaded {
                        document_id: document_id(),
                        content_hash: hash_of('9'),
                    },
                )
                .expect("this document's")
                .display,
        );
    }
    events.extend(drain(&mut state, None));
    let report = report_of(&events);
    let types: Vec<_> = state
        .root_blocks()
        .iter()
        .filter_map(|id| state.block_type_of(id))
        .collect();
    assert!(
        types.contains(&BlockType::Table) && types.contains(&BlockType::Image),
        "{types:?}"
    );
    let [record] = &pictures_of(&state)[..] else {
        panic!("one picture at the root");
    };
    assert_eq!(record.address, Some(hash_of('9')));
    assert_eq!(record.alt, "in a cell");
    assert_eq!(
        record.size,
        ambre_core::Size::new(ambre_core::Px::new(200.0), ambre_core::Px::new(100.0))
    );
    assert_eq!(
        count(
            &report,
            ProseConstruct::Block(crate::message::LiftedBlock::Image),
            Outcome::Moved
        ),
        1,
        "{:?}",
        report.entries()
    );
}

/// A document holding one centred picture of `bytes`, after a paragraph.
fn with_picture(bytes_alt: &str) -> ProseState {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    let first = state.root_blocks()[0];
    state.handle_command(ProseCommand::InsertText {
        block_id: first,
        pos: ambre_core::index::Pos::new(0),
        text: "Text before.".to_owned(),
    });
    state.handle_command(ProseCommand::InsertImage {
        parent_id: None,
        after_id: Some(first),
        content_hash: hash_of('a'),
        alt: bytes_alt.to_owned(),
        size: ambre_core::Size::new(ambre_core::Px::new(200.0), ambre_core::Px::new(100.0)),
    });
    let image = state.root_blocks()[1];
    state.handle_command(ProseCommand::SetImageAlign {
        block_id: image,
        align: crate::message::ImageAlign::Center,
    });
    state
}

/// The downloads the document owes, by request, with their address.
fn downloads(
    state: &mut ProseState,
) -> Vec<(
    ambre_core::Seq<ambre_bus::rest::Request>,
    Validated<ContentHash>,
)> {
    state
        .take_media_requests()
        .into_iter()
        .filter_map(|request| match request {
            MediaRequest::Download { id, content_hash } => Some((id, content_hash)),
            MediaRequest::Upload { .. } => None,
        })
        .collect()
}

/// Export `format`, answer each download with `bytes`, or with an error
/// when `bytes` is `None`, and hand back the file and its report.
fn export(
    state: &mut ProseState,
    format: ExportFormat,
    bytes: Option<Vec<u8>>,
) -> (ambre_bus::interop::ExportedFile, ConversionReport) {
    let family = super::sliced::export_family(&format);
    let started =
        state.handle_converted_in_family(ProseCommand::Interop(InteropRequest::Export(format)));
    let mut events = started.display;
    for (id, address) in downloads(state) {
        let outcome = match &bytes {
            Some(bytes) => state.handle_media_event(
                id,
                RestEvent::MediaDownloaded {
                    document_id: document_id(),
                    content_hash: address,
                    data: Payload::new(bytes.clone()),
                },
            ),
            None => state.handle_media_error(id),
        };
        events.extend(outcome.expect("this document's").display);
    }
    events.extend(drain(state, family));
    events
        .into_iter()
        .find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, report }) => {
                Some((file, report))
            }
            _ => None,
        })
        .expect("the export answers with a file")
}

fn word_export() -> ExportFormat {
    ExportFormat::Word(crate::message::WordExport {
        review: crate::message::ReviewExport::Omitted,
        zone: crate::message::TimeZone::UTC,
    })
}

/// The pictures of a Word package, with the alignment of their paragraph.
fn read_back(data: &[u8]) -> Vec<(Image, Option<Alignment>)> {
    let decoded = ambre_docx::read(data).expect("the package reads");
    decoded
        .document
        .body
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .flat_map(|paragraph| {
            paragraph
                .inlines
                .iter()
                .filter_map(move |inline| match inline {
                    Inline::Image(image) => Some((image.clone(), paragraph.align)),
                    _ => None,
                })
        })
        .collect()
}

/// The text of every run of a Word package, in order.
fn text_of(data: &[u8]) -> String {
    let decoded = ambre_docx::read(data).expect("the package reads");
    decoded
        .document
        .body
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .flat_map(|paragraph| paragraph.inlines.iter())
        .filter_map(|inline| match inline {
            Inline::Run(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect()
}

/// `PROSE-IO-106`, `PROSE-IO-007`: a picture comes back from a Word and an
/// OpenDocument export. It holds the bytes the media flow answered, its
/// alternative text, its size and its centring.
#[test]
fn a_picture_comes_back_from_a_word_and_an_opendocument_export() {
    let bytes = png(4, 2);
    for format in [
        word_export(),
        ExportFormat::OpenDocument(crate::message::WordExport {
            review: crate::message::ReviewExport::Omitted,
            zone: crate::message::TimeZone::UTC,
        }),
    ] {
        let mut state = with_picture("a red bar");
        let odt = matches!(format, ExportFormat::OpenDocument(_));
        let (file, report) = export(&mut state, format, Some(bytes.clone()));
        let pictures = if odt {
            let decoded = ambre_odt::read(file.data.as_bytes()).expect("the package reads");
            decoded
                .document
                .body
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(paragraph),
                    Block::Table(_) => None,
                })
                .flat_map(|paragraph| {
                    paragraph
                        .inlines
                        .iter()
                        .filter_map(move |inline| match inline {
                            Inline::Image(image) => Some((image.clone(), paragraph.align)),
                            _ => None,
                        })
                })
                .collect()
        } else {
            read_back(file.data.as_bytes())
        };
        let [(image, align)] = &pictures[..] else {
            panic!("one picture in the file, got {}", pictures.len());
        };
        assert_eq!(image.bytes.as_bytes(), bytes.as_slice());
        assert_eq!(image.description, "a red bar");
        assert!((image.width.get() - 150.0).abs() < 0.5, "{:?}", image.width);
        assert!(
            (image.height.get() - 75.0).abs() < 0.5,
            "{:?}",
            image.height
        );
        assert_eq!(*align, Some(Alignment::Center));
        assert!(
            report
                .entries()
                .iter()
                .all(|e| e.outcome != Outcome::Dropped)
        );
    }
}

/// `PROSE-IO-008`: a picture whose bytes did not come back is a
/// placeholder in the file and a line in the report; the export finishes.
#[test]
fn a_picture_that_did_not_come_back_is_a_placeholder_and_a_report_line() {
    let mut state = with_picture("lost picture");
    let (file, report) = export(&mut state, word_export(), None);
    assert!(read_back(file.data.as_bytes()).is_empty());
    let text = text_of(file.data.as_bytes());
    assert!(text.contains("[lost picture]"), "{text}");
    assert_eq!(
        count(
            &report,
            ProseConstruct::Feature(crate::message::UnsupportedFeature::PictureMissing),
            Outcome::Approximated
        ),
        1
    );
}

/// `PROSE-IO-108`, `PROSE-IO-040`, `PROSE-IO-041`: an export asks for
/// every picture the document addresses, once each, and answers no file
/// while it waits. A command runs meanwhile, and the export writes nothing.
#[test]
fn an_export_waits_for_its_pictures_and_serves_other_commands() {
    let mut state = with_picture("waited");
    let first = state.root_blocks()[0];
    state.handle_command(ProseCommand::InsertImage {
        parent_id: None,
        after_id: Some(first),
        content_hash: hash_of('a'),
        alt: "same bytes".to_owned(),
        size: ambre_core::Size::new(ambre_core::Px::new(10.0), ambre_core::Px::new(10.0)),
    });
    let before = state.export_snapshot();
    let started = state
        .handle_converted_in_family(ProseCommand::Interop(InteropRequest::Export(word_export())));
    assert!(started.update.is_none());
    assert!(!started.display.iter().any(|event| matches!(
        event,
        ProseEvent::Interop(InteropAnswer::ExportReady { .. })
    )));
    assert_eq!(state.export_snapshot(), before, "the export writes nothing");
    let asked = downloads(&mut state);
    assert_eq!(asked.len(), 1, "one download per distinct address");
    let typed = state.handle_command(ProseCommand::InsertText {
        block_id: first,
        pos: ambre_core::index::Pos::new(0),
        text: "Meanwhile. ".to_owned(),
    });
    assert!(
        typed.update.is_some(),
        "a command runs while the export waits"
    );
}

/// `PROSE-IO-103`: the export walks the document as it stands once its
/// pictures answered. A picture added during the wait is a placeholder.
#[test]
fn a_picture_added_during_the_wait_is_a_placeholder() {
    let mut state = with_picture("first");
    let started = state
        .handle_converted_in_family(ProseCommand::Interop(InteropRequest::Export(word_export())));
    assert!(started.update.is_none());
    let asked = downloads(&mut state);
    let first = state.root_blocks()[0];
    state.handle_command(ProseCommand::InsertImage {
        parent_id: None,
        after_id: Some(first),
        content_hash: hash_of('e'),
        alt: "late".to_owned(),
        size: ambre_core::Size::new(ambre_core::Px::new(10.0), ambre_core::Px::new(10.0)),
    });
    let mut events = Vec::new();
    for (id, address) in asked {
        events.extend(
            state
                .handle_media_event(
                    id,
                    RestEvent::MediaDownloaded {
                        document_id: document_id(),
                        content_hash: address,
                        data: Payload::new(png(4, 2)),
                    },
                )
                .expect("this document's")
                .display,
        );
    }
    events.extend(drain(&mut state, Some(ambre_core::ConverterFamily::Office)));
    let (file, report) = events
        .into_iter()
        .find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, report }) => {
                Some((file, report))
            }
            _ => None,
        })
        .expect("a file");
    assert_eq!(
        read_back(file.data.as_bytes()).len(),
        1,
        "the fetched picture is in the file"
    );
    let text = text_of(file.data.as_bytes());
    assert!(text.contains("[late]"), "{text}");
    assert_eq!(
        count(
            &report,
            ProseConstruct::Feature(crate::message::UnsupportedFeature::PictureMissing),
            Outcome::Approximated
        ),
        1
    );
}

/// `PROSE-IO-107`: an HTML export holds each picture inline as a `data:`
/// address, so the file names nothing outside itself.
#[test]
fn an_html_export_holds_each_picture_as_a_data_address() {
    let mut state = with_picture("inline");
    let (file, _) = export(
        &mut state,
        ExportFormat::Html(crate::message::ReviewExport::Omitted),
        Some(png(1, 1)),
    );
    let html = String::from_utf8(file.data.into_bytes()).expect("the file is UTF-8");
    assert!(
        html.contains("<img src=\"data:image/png;base64,iVBORw0KGgo"),
        "{html}"
    );
    assert!(html.contains("alt=\"inline\""));
    assert!(html.contains("width=\"200\" height=\"100\""));
    assert!(
        !html.contains("src=\"http"),
        "the file names nothing outside itself"
    );
}

/// `PROSE-FEAT-329`: a picture an import carries is stored without the
/// metadata its file held, as one chosen from a file is.
#[test]
fn an_imported_picture_is_stored_without_its_metadata() {
    use crate::picture::tests::{contains, jpeg_with_metadata};
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    let file = word(vec![paragraph(vec![Inline::Image(Image {
        description: "a photograph".to_owned(),
        width: ambre_core::PrintLength::new(60.0),
        height: ambre_core::PrintLength::new(30.0),
        format: ImageFormat::Jpeg,
        bytes: Payload::new(jpeg_with_metadata()),
    })])]);
    start_import(&mut state, ImportFormat::Word, file);
    let asked = uploads(&mut state);
    let [(_, data)] = &asked[..] else {
        panic!("one upload");
    };
    for left_out in [b"ACME".as_slice(), b"secret place", b"a secret comment"] {
        assert!(!contains(data, left_out), "{left_out:?} left");
    }
}
