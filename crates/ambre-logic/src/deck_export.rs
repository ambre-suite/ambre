// ambre-logic/src/deck_export.rs
// ============================================================================
// The deck's PDF export, recorded one page per pass of the Logic thread
// (`3-Toile/features/ambre-spec-toile-pdf-export.md`).
//
// The export reads the board once, when it starts: every page and every
// picture. The pages are then recorded one per pass. Between two pages the
// thread serves the other messages, a local edit or a remote delta of the
// same board included. The recording holds no reference to the board. So
// such an edit reaches the board at once, and no page of the file.
//
// Before the first page, the export asks the Sync worker for the stored file
// of every picture it draws, and records nothing until each answered
// (`TOILE-IO-100`). The Sync worker fetches the ciphertext and opens it; this
// thread only routes the request and the plaintext answer. The messages
// between are served as usual, and the loop does not spin on the wait.
//
// The pass that records the last page hands the recorded pages to the
// converter thread, and never waits (`ARCH-124`). The PDF module writes them
// there, and the answer hands over the file. The recorded pages name no source
// of the picture bytes they hold.
// ============================================================================

use super::*;
use ambre_bus::interop::{Direction, InteropAnswer, InteropError};
use ambre_core::StopCause;
use ambre_toile::message::ToileEvent;

/// The recording of a deck export. A build without the export compiled in
/// starts none, so its type there holds no value.
#[cfg(feature = "toile-pdf")]
pub(crate) type Recording = ambre_toile::logic::interop::DeckRecording;

/// Where a deck export stands.
#[cfg(feature = "toile-pdf")]
pub(crate) type Stage = ambre_toile::logic::interop::DeckStage;

/// Where a deck export stands, in a build without the export: it records
/// none, so both stages are never reached.
#[cfg(not(feature = "toile-pdf"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Fetching,
    Recording,
}

/// The recording of a deck export, in a build without the export: no value.
#[cfg(not(feature = "toile-pdf"))]
pub(crate) enum Recording {}

impl LogicState {
    /// Start a deck export of `document_id`, and record its first page.
    /// `recording` is the board read at the request. A deck export still
    /// running on the same board ends first, so each request answers its own
    /// file.
    /// The first page waits for the stored file of every picture the deck
    /// draws, which the Sync worker fetches and opens (`TOILE-IO-100`).
    pub(crate) fn start_deck_export(
        &mut self,
        document_id: Id<Document>,
        mut recording: Recording,
    ) {
        self.settle_deck_export(document_id);
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let requests = fetch_full(&mut recording);
        doc.deck_export = Some(Box::new(recording));
        if requests.is_empty() {
            self.resume_deck_export(document_id);
        } else {
            self.dispatch_outcome(document_id, Vec::new(), None, None, requests);
        }
    }

    /// The boards whose deck export records pages now: not one still waiting
    /// for a stored file.
    pub(crate) fn decks_exporting(&self) -> Vec<Id<Document>> {
        self.docs
            .iter()
            .filter(|(_, doc)| {
                doc.deck_export
                    .as_deref()
                    .is_some_and(|recording| stage(recording) == Stage::Recording)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The address download `id` fetches, when the deck export of
    /// `document_id` asked for it and still owes it.
    pub(crate) fn deck_fetch_owed(
        &self,
        document_id: Id<Document>,
        id: Seq<Request>,
    ) -> Option<&ambre_core::Validated<ambre_core::ContentHash>> {
        self.docs
            .get(&document_id)
            .and_then(|doc| doc.deck_export.as_deref())
            .and_then(|recording| owes(recording, id))
    }

    /// Hand the answer to download `id` to the deck export of `document_id`:
    /// the opened bytes, or `None` when the Sync worker failed or refused it.
    /// The last answer records the first page.
    pub(crate) fn answer_deck_fetch(
        &mut self,
        document_id: Id<Document>,
        id: Seq<Request>,
        data: Option<ambre_core::Payload<ambre_core::MediaBytes>>,
    ) {
        let Some(recording) = self
            .docs
            .get_mut(&document_id)
            .and_then(|doc| doc.deck_export.as_deref_mut())
        else {
            return;
        };
        answer(recording, id, data);
        if stage(recording) == Stage::Recording {
            self.resume_deck_export(document_id);
        }
    }

    /// Record the rest of a board's running deck export now, and answer its
    /// file. A stored file still owed is given up: its picture goes out from
    /// the thumbnail, and the report counts it.
    fn settle_deck_export(&mut self, document_id: Id<Document>) {
        if let Some(recording) = self
            .docs
            .get_mut(&document_id)
            .and_then(|doc| doc.deck_export.as_deref_mut())
        {
            stop_waiting(recording);
        }
        while self
            .docs
            .get(&document_id)
            .is_some_and(|doc| doc.deck_export.is_some())
        {
            self.resume_deck_export(document_id);
        }
    }

    /// Record the next page of a board's running deck export. A page short of
    /// the end answers the progress. The last page owes the converter thread
    /// the write of the file, which answers it (`ARCH-124`). A board with no
    /// export running answers nothing.
    pub(crate) fn resume_deck_export(&mut self, document_id: Id<Document>) {
        let Self {
            docs, export_cx, ..
        } = self;
        let Some(doc) = docs.get_mut(&document_id) else {
            return;
        };
        let Some(recording) = doc.deck_export.as_deref_mut() else {
            return;
        };
        if stage(recording) == Stage::Fetching {
            return;
        }
        let answer = match export_cx.as_mut() {
            Some(font_cx) => step(recording, font_cx),
            // The faces never leave once seeded, so this holds no start can
            // reach. The export still ends with a reason, never in silence.
            None => Step::Stopped(StopCause::FontsLoading),
        };
        let event = match answer {
            Step::Progress(progress) => ToileEvent::Interop(InteropAnswer::Progress(progress)),
            Step::Recorded => {
                if let Some(recording) = doc.deck_export.take() {
                    let report = Box::new(report(&recording));
                    self.offload(crate::converter_thread::Job {
                        ticket: ambre_bus::next_request(),
                        owner: crate::converter_thread::JobOwner::DeckPdf {
                            document_id,
                            report,
                        },
                        conversion: write_request(*recording),
                    });
                }
                return;
            }
            Step::Stopped(cause) => {
                doc.deck_export = None;
                ToileEvent::Interop(InteropAnswer::failed(
                    Direction::Export,
                    InteropError::Stopped(cause),
                ))
            }
        };
        self.dispatch_outcome(
            document_id,
            vec![AppEvent::Toile(event)],
            None,
            None,
            Vec::new(),
        );
    }
}

/// What one step of a deck export answers.
#[cfg_attr(
    not(feature = "toile-pdf"),
    expect(dead_code, reason = "a build without the export records no page")
)]
enum Step {
    /// A page short of the end is recorded.
    Progress(ambre_bus::interop::ConversionProgress),
    /// The last page is recorded: the file is ready to write.
    Recorded,
    /// The export cannot go on, for this reason.
    Stopped(StopCause),
}

/// Record one page of `recording`.
#[cfg(feature = "toile-pdf")]
fn step(recording: &mut Recording, font_cx: &mut parley::FontContext) -> Step {
    recording.record_next(font_cx);
    if recording.is_recorded() {
        Step::Recorded
    } else {
        Step::Progress(recording.progress())
    }
}

/// Ask for the stored file of every picture `recording` draws.
#[cfg(feature = "toile-pdf")]
fn fetch_full(recording: &mut Recording) -> Vec<ambre_toile::logic::command::ToileRequest> {
    recording.fetch_full()
}

/// Where `recording` stands: fetching stored files, or recording pages.
#[cfg(feature = "toile-pdf")]
pub(crate) fn stage(recording: &Recording) -> Stage {
    recording.stage()
}

#[cfg(feature = "toile-pdf")]
fn owes(
    recording: &Recording,
    id: Seq<Request>,
) -> Option<&ambre_core::Validated<ambre_core::ContentHash>> {
    recording.owes(id)
}

#[cfg(feature = "toile-pdf")]
fn answer(
    recording: &mut Recording,
    id: Seq<Request>,
    data: Option<ambre_core::Payload<ambre_core::MediaBytes>>,
) {
    recording.answer(id, data);
}

#[cfg(feature = "toile-pdf")]
fn stop_waiting(recording: &mut Recording) {
    recording.stop_waiting();
}

#[cfg(feature = "toile-pdf")]
fn report(recording: &Recording) -> ambre_toile::message::ToileReport {
    recording.report()
}

#[cfg(not(feature = "toile-pdf"))]
fn fetch_full(recording: &mut Recording) -> Vec<ambre_toile::logic::command::ToileRequest> {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
pub(crate) fn stage(recording: &Recording) -> Stage {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
fn owes(
    recording: &Recording,
    _id: Seq<Request>,
) -> Option<&ambre_core::Validated<ambre_core::ContentHash>> {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
fn answer(
    recording: &mut Recording,
    _id: Seq<Request>,
    _data: Option<ambre_core::Payload<ambre_core::MediaBytes>>,
) {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
fn stop_waiting(recording: &mut Recording) {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
fn report(recording: &Recording) -> ambre_toile::message::ToileReport {
    match *recording {}
}

#[cfg(not(feature = "toile-pdf"))]
fn step(recording: &mut Recording, _font_cx: &mut parley::FontContext) -> Step {
    match *recording {}
}

/// The request that writes the recorded pages through the PDF module.
#[cfg(feature = "toile-pdf")]
fn write_request(recording: Recording) -> ambre_convert::Conversion {
    ambre_convert::Conversion::WritePdf {
        document: recording.into_document(),
    }
}

#[cfg(not(feature = "toile-pdf"))]
fn write_request(recording: Recording) -> ambre_convert::Conversion {
    match recording {}
}
