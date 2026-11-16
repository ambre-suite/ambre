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
// The PDF module writes the recorded pages in the pass that records the last
// one.
// ============================================================================

use super::*;
use ambre_bus::interop::{Direction, InteropAnswer, InteropError};
use ambre_core::StopCause;
use ambre_toile::message::ToileEvent;

/// The recording of a deck export. A build without the export compiled in
/// starts none, so its type there holds no value.
#[cfg(feature = "toile-pdf")]
pub(crate) type Recording = ambre_toile::logic::interop::DeckRecording;

/// The recording of a deck export, in a build without the export: no value.
#[cfg(not(feature = "toile-pdf"))]
pub(crate) enum Recording {}

impl LogicState {
    /// Start a deck export of `document_id`, and record its first page.
    /// `recording` is the board read at the request. A deck export still
    /// running on the same board ends first, so each request answers its own
    /// file.
    pub(crate) fn start_deck_export(&mut self, document_id: Id<Document>, recording: Recording) {
        self.settle_deck_export(document_id);
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        doc.deck_export = Some(Box::new(recording));
        self.resume_deck_export(document_id);
    }

    /// The boards whose deck export still records pages.
    pub(crate) fn decks_exporting(&self) -> Vec<Id<Document>> {
        self.docs
            .iter()
            .filter(|(_, doc)| doc.deck_export.is_some())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Record the rest of a board's running deck export now, and answer its
    /// file.
    fn settle_deck_export(&mut self, document_id: Id<Document>) {
        while self
            .docs
            .get(&document_id)
            .is_some_and(|doc| doc.deck_export.is_some())
        {
            self.resume_deck_export(document_id);
        }
    }

    /// Record the next page of a board's running deck export. A page short of
    /// the end answers the progress. The last page writes the
    /// file and answers it. A board with no export running answers nothing.
    pub(crate) fn resume_deck_export(&mut self, document_id: Id<Document>) {
        let Self {
            docs,
            export_cx,
            converters,
            ..
        } = self;
        let Some(doc) = docs.get_mut(&document_id) else {
            return;
        };
        let Some(recording) = doc.deck_export.as_deref_mut() else {
            return;
        };
        let answer = match export_cx.as_mut() {
            Some(font_cx) => step(recording, font_cx),
            // The faces never leave once seeded, so this holds no start can
            // reach. The export still ends with a reason, never in silence.
            None => Step::Stopped(StopCause::FontsLoading),
        };
        let event = match answer {
            Step::Progress(progress) => ToileEvent::Interop(InteropAnswer::Progress(progress)),
            Step::Recorded => match doc.deck_export.take() {
                Some(recording) => {
                    let converter = converters.ready(ambre_core::ConverterFamily::Pdf);
                    data::deck_file_answer(write(*recording, converter))
                }
                None => return,
            },
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

#[cfg(not(feature = "toile-pdf"))]
fn step(recording: &mut Recording, _font_cx: &mut parley::FontContext) -> Step {
    match *recording {}
}

/// Write the recorded pages through the PDF module, or say why there is no
/// file.
#[cfg(feature = "toile-pdf")]
fn write(
    recording: Recording,
    converter: Option<&ambre_convert::Converter>,
) -> Result<ambre_core::Payload<ambre_client_core::FileBytes>, StopCause> {
    let Some(converter) = converter else {
        return Err(StopCause::ConverterUnavailable);
    };
    ambre_toile::logic::interop::write_pdf(recording.into_document(), converter).map_err(|e| {
        tracing::warn!("Logic: the PDF export stopped: {e}");
        data::pdf_stop_cause(e)
    })
}

#[cfg(not(feature = "toile-pdf"))]
fn write(
    recording: Recording,
    _converter: Option<&ambre_convert::Converter>,
) -> Result<ambre_core::Payload<ambre_client_core::FileBytes>, StopCause> {
    match recording {}
}
