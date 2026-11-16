//! Import and export through the intermediate form
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! This module holds the intermediate form and the readers and writers over
//! it. None of them sees the document: no item here names a `LoroDoc` or the
//! engine. So a reader cannot emit a command (`PROSE-IO-009`), and a writer
//! cannot read the document (`PROSE-IO-045`). A reader and a writer meet in
//! [`ProseIr`] alone (`PROSE-IO-044`). The walk that fills the form and the
//! builder that drains it live beside the engine, in `logic::interop`.
//!
//! The form holds the constructs a landed format reads or writes. A field
//! joins it with the first reader or writer that needs it.

pub mod html;
pub mod markdown;
pub mod word;

// The form lives in its own crate, which the Office module links too.
pub use ambre_prose_form::{
    Banding, IrBlock, IrChange, IrComment, IrFootnote, IrListStyle, IrLook, IrMeta, IrOrderedStyle,
    IrReply, IrRun, IrSuggestion, IrTableCell, IrTableRow, IrThread, Merging, OwnStyle, Paging,
    ProseIr, Referencing,
};

use crate::message::{
    ConversionReport, DroppedElement, LiftedBlock, ProseConstruct, UnsupportedFeature,
};
use ambre_bus::interop::{Direction, FileFormat, Outcome};

/// The stop a converter error reads as. The loader's words go to the log
/// here: the bus carries the kind alone (`BUS-046`).
pub(crate) fn stopped(error: &ambre_convert::ConvertError) -> crate::message::InteropError {
    tracing::warn!("Prose: a conversion stopped: {error}");
    crate::message::InteropError::Stopped(error.stop_cause())
}

/// The form a module read a file of `format` into, with the report of its
/// mapping in Prose's terms. `refused` words a refusal of the reader.
///
/// # Errors
///
/// What `refused` makes of a refusal. A module that stops, answers out of
/// turn, or answers a form that does not decode fails as
/// [`InteropError::Stopped`](crate::message::InteropError::Stopped).
pub(crate) fn read_form(
    answer: Result<ambre_convert::Converted, ambre_convert::ConvertError>,
    format: FileFormat,
    refused: fn(ambre_core::FileRefusal) -> crate::message::InteropError,
) -> Result<(ProseIr, ConversionReport), crate::message::InteropError> {
    match answer {
        Ok(ambre_convert::Converted::Form(bytes)) => {
            let (ir, report): (ProseIr, ambre_prose_form::FormReport) =
                ambre_prose_form::decode(&bytes).map_err(|e| form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Import, format);
            warnings.form(&report);
            Ok((ir, warnings.report()))
        }
        Ok(_) => Err(stopped(&ambre_convert::ConvertError::Malformed)),
        Err(ambre_convert::ConvertError::Refused(reason)) => Err(refused(reason)),
        Err(e) => Err(stopped(&e)),
    }
}

/// The file a module wrote in `format`, with the report of what the format
/// could not hold.
///
/// # Errors
///
/// [`InteropError::Stopped`](crate::message::InteropError::Stopped) when the
/// module stops, answers out of turn, or refuses the form.
pub(crate) fn written_file(
    answer: Result<ambre_convert::Converted, ambre_convert::ConvertError>,
    format: FileFormat,
) -> Result<
    (
        ambre_core::Payload<ambre_client_core::FileBytes>,
        ConversionReport,
    ),
    crate::message::InteropError,
> {
    match answer {
        Ok(ambre_convert::Converted::Written { file, report }) => {
            let report: ambre_prose_form::FormReport =
                ambre_prose_form::decode(&report).map_err(|e| form_unreadable(&e))?;
            let mut warnings = Warnings::new(Direction::Export, format);
            warnings.form(&report);
            Ok((file, warnings.report()))
        }
        Ok(_) => Err(stopped(&ambre_convert::ConvertError::Malformed)),
        Err(e) => Err(stopped(&e)),
    }
}

/// The stop a refusal of the reader reads as, for a format whose reader
/// refuses only past its own bound.
pub(crate) fn refusal_stops(reason: ambre_core::FileRefusal) -> crate::message::InteropError {
    stopped(&ambre_convert::ConvertError::Refused(reason))
}

/// The stop of a form that did not encode or decode between the document and
/// the module.
pub(crate) fn form_unreadable(
    error: &ambre_prose_form::FormDecodeError,
) -> crate::message::InteropError {
    tracing::warn!("Prose: the conversion form did not read: {error}");
    crate::message::InteropError::Stopped(ambre_core::StopCause::FormUnreadable)
}

/// The losses of one conversion, each construct once per outcome with its
/// count, in the order they were first met. A thin writer over the shared
/// report, so each reader and writer names its losses in Prose's terms.
#[derive(Debug)]
pub struct Warnings(ConversionReport);

impl Warnings {
    #[must_use]
    pub const fn new(direction: Direction, format: FileFormat) -> Self {
        Self(ConversionReport::new(direction, format))
    }

    /// Carry on the losses a reader already recorded.
    #[must_use]
    pub const fn from_report(report: ConversionReport) -> Self {
        Self(report)
    }

    /// A construct the format has no syntax for (`PROSE-IO-004`).
    pub fn unsupported(&mut self, feature: UnsupportedFeature) {
        self.0
            .record(ProseConstruct::Feature(feature), Outcome::Dropped);
    }

    /// An element the reader does not recognise (`PROSE-IO-005`).
    pub fn dropped(&mut self, element: DroppedElement) {
        self.0
            .record(ProseConstruct::Element(element), Outcome::Dropped);
    }

    /// A block moved out of a parent that refuses it (`PROSE-IO-052`).
    pub fn lifted(&mut self, block: LiftedBlock) {
        self.0.record(ProseConstruct::Block(block), Outcome::Moved);
    }

    /// A construct a writer emitted as the nearest thing its format holds
    /// (`PROSE-IO-006`).
    pub fn approximated(&mut self, feature: UnsupportedFeature) {
        self.0
            .record(ProseConstruct::Feature(feature), Outcome::Approximated);
    }

    /// What a mapping in a converter module carried, approximated or
    /// dropped, each entry one to one (`INTEROP-003`).
    pub fn form(&mut self, report: &ambre_prose_form::FormReport) {
        for entry in report.entries() {
            let outcome = match entry.outcome {
                ambre_prose_form::FormOutcome::Carried => Outcome::Carried,
                ambre_prose_form::FormOutcome::Approximated => Outcome::Approximated,
                ambre_prose_form::FormOutcome::Dropped => Outcome::Dropped,
            };
            let construct = match entry.construct {
                ambre_prose_form::FormConstruct::Feature(feature) => {
                    ProseConstruct::Feature(feature)
                }
                ambre_prose_form::FormConstruct::Element(element) => {
                    ProseConstruct::Element(element)
                }
                ambre_prose_form::FormConstruct::Word(construct) => ProseConstruct::Word(construct),
            };
            self.0.record_count(
                construct,
                outcome,
                ambre_core::Count::new(entry.count.get()),
            );
        }
    }

    #[must_use]
    pub fn report(self) -> ConversionReport {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PROSE-IO-053`: a lossy report reads as one sentence naming each
    /// construct, and a clean one says nothing.
    #[test]
    fn a_report_reads_as_one_sentence() {
        let fresh = || Warnings::new(Direction::Export, FileFormat::Markdown);
        assert_eq!(fresh().report().sentence(), None);
        let mut warnings = fresh();
        warnings.unsupported(UnsupportedFeature::Underline);
        warnings.unsupported(UnsupportedFeature::Underline);
        warnings.dropped(DroppedElement::Html);
        warnings.lifted(LiftedBlock::Table);
        assert_eq!(
            warnings.report().sentence().as_deref(),
            Some("Moved: table; dropped: underline (2), HTML")
        );
    }
}
