//! Each Word and OpenDocument input fixture through Prose and back
//! (`features/ambre-spec-prose-import-export.md`, `PROSE-IO-114` to
//! `PROSE-IO-116`).
//!
//! The codec round trip holds the codec against its own reader. This one
//! runs the path a user runs. The file lands in a document, and the
//! document leaves as a file of the same format. That file lands in a
//! second document. The two documents are compared as data: the blocks, the runs
//! and their marks, the threads and the suggestions with their instants in
//! UTC. The bytes of the two files are never compared.

// The codecs read a file back on the host alone.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use ambre_bus::RestEvent;
use ambre_bus::interop::{InteropAnswer, InteropRequest, Outcome};
use ambre_core::{Authored, ContentHash, ConverterFamily, Instant, Payload, Validated};
use ambre_prose_form::{IrBlock, IrRun, ProseIr};

use super::ProseState;
use crate::events::ProseEvent;
use crate::message::{
    ConversionReport, ExportFormat, ImportFormat, MediaRequest, ProseCommand, ProseConstruct,
    ProseImport, ReviewExport, TimeZone, WordExport,
};

/// The Word input fixtures of `ambre-docx`: every file its tests read.
const DOCX: [&str; 17] = [
    "pandoc/block_quotes.docx",
    "pandoc/codeblock.docx",
    "pandoc/comments.docx",
    "pandoc/headers.docx",
    "pandoc/image.docx",
    "pandoc/inline_formatting.docx",
    "pandoc/links.docx",
    "pandoc/lists.docx",
    "pandoc/notes.docx",
    "pandoc/tables.docx",
    "pandoc/track_changes_deletion.docx",
    "pandoc/track_changes_insertion.docx",
    "pandoc/track_changes_move.docx",
    "pandoc/track_changes_scrubbed_metadata.docx",
    "open-xml-sdk/Comment004.docx",
    "open-xml-sdk/strict-numbering.docx",
    "libreoffice/features.docx",
];

/// The OpenDocument text input fixtures of `ambre-odt`.
const ODT: [&str; 3] = [
    "libreoffice/features.odt",
    "pandoc/inline.odt",
    "pandoc/structure.odt",
];

/// The two text formats, each with the directory of its fixtures.
#[derive(Clone, Copy, Debug)]
enum Text {
    Word,
    OpenDocument,
}

impl Text {
    fn fixture(self, name: &str) -> Vec<u8> {
        let crate_dir = match self {
            Self::Word => "ambre-docx",
            Self::OpenDocument => "ambre-odt",
        };
        let path = format!(
            "{}/../{crate_dir}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    const fn import(self) -> ImportFormat {
        match self {
            Self::Word => ImportFormat::Word,
            Self::OpenDocument => ImportFormat::OpenDocument,
        }
    }

    fn export(self, zone: &TimeZone) -> ExportFormat {
        let options = WordExport {
            review: ReviewExport::Included,
            zone: zone.clone(),
        };
        match self {
            Self::Word => ExportFormat::Word(options),
            Self::OpenDocument => ExportFormat::OpenDocument(options),
        }
    }
}

/// The blob store the Sync worker stands for: an upload answers an address,
/// a download answers the bytes stored there.
#[derive(Default)]
struct Store {
    blobs: HashMap<Validated<ContentHash>, Vec<u8>>,
}

impl Store {
    fn put(&mut self, bytes: Vec<u8>) -> Validated<ContentHash> {
        let at = self
            .blobs
            .iter()
            .find_map(|(at, held)| (*held == bytes).then(|| at.clone()));
        at.unwrap_or_else(|| {
            let at = Validated::<ContentHash>::parse(format!("{:064x}", self.blobs.len() + 1))
                .expect("64 hex digits");
            self.blobs.insert(at.clone(), bytes);
            at
        })
    }

    /// Answer every media request `state` owes, and report whether one was.
    fn answer(&mut self, state: &mut ProseState, events: &mut Vec<ProseEvent>) -> bool {
        let requests = state.take_media_requests();
        let asked = !requests.is_empty();
        for request in requests {
            let outcome = match request {
                MediaRequest::Upload { id, data } => state.handle_media_event(
                    id,
                    RestEvent::MediaUploaded {
                        document_id: document_id(),
                        content_hash: self.put(data.into_bytes()),
                    },
                ),
                MediaRequest::Download { id, content_hash } => {
                    let data = self
                        .blobs
                        .get(&content_hash)
                        .cloned()
                        .expect("a download names an address the store holds");
                    state.handle_media_event(
                        id,
                        RestEvent::MediaDownloaded {
                            document_id: document_id(),
                            content_hash,
                            data: Payload::new(data),
                        },
                    )
                }
            };
            events.extend(outcome.expect("this document's").display);
        }
        asked
    }

    /// Run `command` to its end: each media request answered, each slice
    /// resumed. Hand back every event it answered.
    fn run(&mut self, state: &mut ProseState, command: ProseCommand) -> Vec<ProseEvent> {
        let mut events = state.handle_converted_in_family(command).display;
        loop {
            let asked = self.answer(state, &mut events);
            if state.has_pending_work() {
                events.extend(state.resume_in_place().display);
            } else if !asked {
                return events;
            }
        }
    }
}

fn document_id() -> ambre_core::id::Id<ambre_core::id::Document> {
    ambre_core::id::Id::from_bytes([1; 16])
}

/// A document whose user is known, so a thread and a suggestion land.
fn signed_in() -> ProseState {
    let mut state = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document");
    state.set_user_id(ambre_core::id::Id::from_bytes([7; 16]));
    state
}

/// The instant an import stamps on what the file left undated.
const NOW: Instant<Authored> = Instant::new(1_800_000_000_000);

/// Import `bytes` as `format`, read under `zone`, into a fresh document.
fn import(
    store: &mut Store,
    format: Text,
    bytes: Vec<u8>,
    zone: &TimeZone,
) -> (ProseState, ConversionReport) {
    let mut state = signed_in();
    let events = store.run(
        &mut state,
        ProseCommand::Interop(InteropRequest::Import(ProseImport {
            format: format.import(),
            data: Payload::new(bytes),
            now: NOW,
            zone: zone.clone(),
        })),
    );
    let report = events
        .into_iter()
        .find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ImportCompleted { report }) => Some(report),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the import of a {format:?} fixture completes"));
    (state, report)
}

/// Export `state` as `format`, its dates written under `zone`.
fn export(
    store: &mut Store,
    state: &mut ProseState,
    format: Text,
    zone: &TimeZone,
) -> (Vec<u8>, ConversionReport) {
    let events = store.run(
        state,
        ProseCommand::Interop(InteropRequest::Export(format.export(zone))),
    );
    events
        .into_iter()
        .find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, report }) => {
                Some((file.data.into_bytes(), report))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the export as {format:?} answers a file"))
}

/// The document as data, two neighbour runs of the same marks joined:
/// where a file cuts its runs is no data the user reads. The threads are in
/// the order of the text they anchor on.
fn data(state: &ProseState) -> ProseIr {
    let mut ir = state.walk();
    joined(&mut ir.blocks);
    for note in &mut ir.footnotes {
        join(&mut note.runs);
    }
    threads_in_text_order(&mut ir);
    ir
}

/// Each run of `ir`, in the order of the text: the blocks, then the notes.
fn each_run(ir: &mut ProseIr, f: &mut impl FnMut(&mut IrRun)) {
    fn walk(blocks: &mut [IrBlock], f: &mut impl FnMut(&mut IrRun)) {
        for block in blocks {
            match block {
                IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => {
                    runs.iter_mut().for_each(&mut *f);
                }
                IrBlock::ListItem { runs, children, .. } => {
                    runs.iter_mut().for_each(&mut *f);
                    walk(children, f);
                }
                IrBlock::Table { rows, .. } => {
                    for cell in rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                        walk(&mut cell.blocks, f);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&mut ir.blocks, f);
    for note in &mut ir.footnotes {
        note.runs.iter_mut().for_each(&mut *f);
    }
}

/// Order the threads by the first run each covers, then by what they say.
/// Two threads over the same text have no order the user reads: the walk
/// lists them by the ids the import minted.
fn threads_in_text_order(ir: &mut ProseIr) {
    let mut first: Vec<Option<usize>> = vec![None; ir.comments.len()];
    let mut seen = 0usize;
    each_run(ir, &mut |run| {
        for at in &run.comments {
            if let Some(slot) = first.get_mut(at.to_usize()) {
                slot.get_or_insert(seen);
            }
        }
        seen += 1;
    });
    let says = |thread: &ambre_prose_form::IrComment| -> Vec<(Option<u64>, String, String)> {
        thread
            .replies
            .iter()
            .map(|reply| {
                (
                    reply.created_at.map(Instant::get),
                    reply
                        .author
                        .as_ref()
                        .map(|a| a.as_str().to_owned())
                        .unwrap_or_default(),
                    reply.text.clone(),
                )
            })
            .collect()
    };
    let mut order: Vec<usize> = (0..ir.comments.len()).collect();
    order.sort_by_key(|&at| (first.get(at).copied().flatten(), says(&ir.comments[at])));
    let mut rank = vec![0usize; order.len()];
    for (new, &old) in order.iter().enumerate() {
        rank[old] = new;
    }
    let comments = std::mem::take(&mut ir.comments);
    let mut slots: Vec<_> = comments.into_iter().map(Some).collect();
    ir.comments = order.iter().filter_map(|&old| slots[old].take()).collect();
    each_run(ir, &mut |run| {
        for at in &mut run.comments {
            *at = ambre_core::index::Pos::from_usize(rank[at.to_usize()]);
        }
        run.comments.sort();
    });
}

fn joined(blocks: &mut [IrBlock]) {
    for block in blocks {
        match block {
            IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => join(runs),
            IrBlock::ListItem { runs, children, .. } => {
                join(runs);
                joined(children);
            }
            IrBlock::Table { rows, .. } => {
                for cell in rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                    joined(&mut cell.blocks);
                }
            }
            _ => {}
        }
    }
}

fn join(runs: &mut Vec<IrRun>) {
    let mut out: Vec<IrRun> = Vec::new();
    for run in runs.drain(..) {
        if let Some(last) = out.last_mut() {
            let (mut a, mut b) = (last.clone(), run.clone());
            a.text.clear();
            b.text.clear();
            if a == b {
                last.text.push_str(&run.text);
                continue;
            }
        }
        out.push(run);
    }
    *runs = out;
}

/// The constructs a report names as lost: moved, approximated or dropped.
fn losses(report: &ConversionReport) -> Vec<(ProseConstruct, Outcome)> {
    report
        .entries()
        .iter()
        .filter(|entry| entry.outcome != Outcome::Carried)
        .map(|entry| (entry.construct, entry.outcome))
        .collect()
}

fn paris() -> TimeZone {
    TimeZone::named("Europe/Paris").expect("an IANA name")
}

/// Every text fixture, with its format.
fn fixtures() -> impl Iterator<Item = (Text, &'static str)> {
    DOCX.into_iter()
        .map(|name| (Text::Word, name))
        .chain(ODT.into_iter().map(|name| (Text::OpenDocument, name)))
}

/// `PROSE-IO-114`: each Word and OpenDocument input fixture, imported,
/// exported in its own format and imported again, lands as the same
/// document. The zone is Paris, so a date read in one zone and written in
/// another moves by two hours and fails the comparison.
#[test]
fn every_text_fixture_lands_the_same_document_after_an_export_and_an_import() {
    let zone = paris();
    for (format, name) in fixtures() {
        let mut store = Store::default();
        let (mut first, _) = import(&mut store, format, format.fixture(name), &zone);
        let (file, exported) = export(&mut store, &mut first, format, &zone);
        let (second, _) = import(&mut store, format, file, &zone);
        let (before, after) = (data(&first), data(&second));
        assert!(
            !before.blocks.is_empty(),
            "{name}: the fixture lands something"
        );
        assert_eq!(
            after,
            before,
            "{name}: the export named {:?}",
            losses(&exported)
        );
    }
}

/// `PROSE-IO-115`: the export of a document that came from a fixture names
/// no loss but the ones its format always owes. Word and OpenDocument hold
/// no page format of Prose's own, so the page leaves approximated, and
/// nothing else leaves short of what the document held.
#[test]
fn an_export_of_a_text_fixture_names_no_loss_its_format_does_not_owe() {
    let zone = paris();
    let page = (
        ProseConstruct::Feature(crate::message::UnsupportedFeature::PageFormat),
        Outcome::Approximated,
    );
    for (format, name) in fixtures() {
        let mut store = Store::default();
        let (mut first, _) = import(&mut store, format, format.fixture(name), &zone);
        let (_, exported) = export(&mut store, &mut first, format, &zone);
        let unowed: Vec<_> = losses(&exported)
            .into_iter()
            .filter(|loss| *loss != page)
            .collect();
        assert!(unowed.is_empty(), "{name}: {unowed:?}");
    }
}

/// The offset of `Etc/GMT-2`, whose clocks run two hours ahead of UTC all
/// year round. The sign of the IANA name is the reverse of the offset.
const TWO_HOURS: u64 = 2 * 60 * 60 * 1000;

/// How many comments of a fixture carry a date with no zone. A Word file
/// reads through its codec, which tells a wall time from a UTC instant. An
/// OpenDocument file is read in its text: `dc:date` with no `Z` and no
/// offset is a wall time.
fn wall_dated_comments(format: Text, bytes: &[u8]) -> usize {
    match format {
        Text::Word => ambre_docx::read(bytes)
            .expect("the fixture reads")
            .document
            .comments
            .iter()
            .filter(|c| matches!(c.date, Some(ambre_docx::model::WordDate::Floating(_))))
            .count(),
        Text::OpenDocument => {
            let mut package =
                ambre_docx::opc::odf::open(bytes, ambre_docx::opc::odf::OdfType::TEXT)
                    .expect("the fixture opens");
            let name = ambre_docx::opc::PartName::new("content.xml").expect("a name");
            let content = package.part(&name).expect("reads").expect("a content part");
            let text = String::from_utf8(content.as_bytes().to_vec()).expect("UTF-8");
            text.split("<office:annotation")
                .skip(1)
                .filter_map(|annotation| {
                    let start = annotation.find("<dc:date>")? + "<dc:date>".len();
                    let end = annotation.get(start..)?.find('<')?;
                    annotation.get(start..start + end)
                })
                .filter(|date| {
                    let time = date.split_once('T').map_or("", |(_, time)| time);
                    !time.contains(['Z', '+', '-'])
                })
                .count()
        }
    }
}

/// The form the Office converter reads `bytes` of `format` into, under UTC.
fn form(format: Text, bytes: Vec<u8>) -> ProseIr {
    let office = ambre_convert::Converter::load(ConverterFamily::Office, &Payload::new(vec![]))
        .expect("the host links the Office engine");
    let (data, zone) = (Payload::new(bytes), TimeZone::UTC);
    let conversion = match format {
        Text::Word => ambre_convert::Conversion::ReadDocx { data, zone },
        Text::OpenDocument => ambre_convert::Conversion::ReadOdt { data, zone },
    };
    match office.convert(&conversion).expect("the fixture converts") {
        ambre_convert::Converted::Form(bytes) => {
            let (ir, _): (ProseIr, ambre_prose_form::FormReport) =
                ambre_prose_form::decode(&bytes).expect("the form decodes");
            ir
        }
        _ => panic!("a text read answers a form"),
    }
}

/// How many entries of `report` count `construct` under `outcome`.
fn count(report: &ConversionReport, construct: ProseConstruct, outcome: Outcome) -> u32 {
    report
        .entries()
        .iter()
        .filter(|entry| entry.construct == construct && entry.outcome == outcome)
        .map(|entry| entry.count.get())
        .sum()
}

/// `PROSE-IO-116`: a fixture is imported under UTC, then under a zone two
/// hours ahead. A comment the file dated by a wall time alone lands two
/// hours earlier in the second, and each other comment at one instant. The
/// oracle is the offset, not the reader: a reader that ignored the zone
/// would land both at one instant. The report counts each wall time.
#[test]
fn a_wall_time_moves_by_the_zone_offset_and_a_utc_date_stays() {
    let ahead = TimeZone::named("Etc/GMT-2").expect("an IANA name");
    let local = ProseConstruct::Word(ambre_docx::WordConstruct::LocalDate);
    let mut walls = 0;
    for (format, name) in fixtures() {
        let bytes = format.fixture(name);
        let floating = wall_dated_comments(format, &bytes);
        let mut store = Store::default();
        let (utc, utc_report) = import(&mut store, format, bytes.clone(), &TimeZone::UTC);
        let (east, east_report) = import(&mut store, format, bytes, &ahead);
        let replies = |state: &ProseState| -> Vec<Option<Instant<Authored>>> {
            data(state)
                .comments
                .iter()
                .flat_map(|thread| thread.replies.iter().map(|reply| reply.created_at))
                .collect()
        };
        let (at_utc, at_east) = (replies(&utc), replies(&east));
        assert_eq!(at_utc.len(), at_east.len(), "{name}");
        let mut moved = 0;
        for (a, b) in at_utc.iter().zip(&at_east) {
            match (a, b) {
                (Some(a), Some(b)) if a != b => {
                    assert_eq!(a.get() - b.get(), TWO_HOURS, "{name}: {a:?} and {b:?}");
                    moved += 1;
                }
                (a, b) => assert_eq!(a, b, "{name}"),
            }
        }
        assert_eq!(
            moved, floating,
            "{name}: each wall time moved, and no other"
        );
        let counted = count(&utc_report, local, Outcome::Approximated);
        assert_eq!(
            counted,
            count(&east_report, local, Outcome::Approximated),
            "{name}"
        );
        assert!(
            u32::try_from(moved).is_ok_and(|moved| counted >= moved),
            "{name}: each wall time is counted"
        );
        walls += moved;
    }
    assert!(walls > 0, "a fixture dates a comment by its wall time");
}

/// `PROSE-IO-118`: a tracked change lands as a suggestion without its date,
/// since a suggestion holds none, and the report names each date dropped.
#[test]
fn a_dated_tracked_change_lands_without_its_date_and_is_named() {
    let dropped = ProseConstruct::Feature(crate::message::UnsupportedFeature::SuggestionDate);
    let mut dated_total = 0;
    for (format, name) in fixtures() {
        let bytes = format.fixture(name);
        let form = form(format, bytes.clone());
        let dated = form.suggestions.iter().filter(|s| s.date.is_some()).count();
        let mut store = Store::default();
        let (state, report) = import(&mut store, format, bytes, &TimeZone::UTC);
        let ir = data(&state);
        assert_eq!(ir.suggestions.len(), form.suggestions.len(), "{name}");
        assert!(ir.suggestions.iter().all(|s| s.date.is_none()), "{name}");
        assert_eq!(
            usize::try_from(count(&report, dropped, Outcome::Dropped)).ok(),
            Some(dated),
            "{name}"
        );
        dated_total += dated;
    }
    assert!(dated_total > 0, "a fixture dates a tracked change");
}
