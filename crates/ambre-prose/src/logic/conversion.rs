//! The commands that wait on a converter module
//! (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! A Markdown import and a paste that carries HTML from another application
//! need the markup module. A Word import and a Word export need the OOXML
//! module. The Logic dispatcher asks
//! [`ProseState::conversion_family`] which module a command waits on, loads
//! it, then hands the command back through [`ProseState::handle_conversion`].
//! The module reads first. The command then runs the path a view command
//! runs: caught up, refused while suggesting where it must be, and named on a
//! refusal.

use ambre_bus::interop::InteropRequest;
use ambre_convert::markup::HtmlTree;
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::ConverterFamily;

use crate::events::ProseEvent;
use crate::interop::word;
use crate::message::{ExportFormat, ImportFormat, ReviewExport};
use ambre_bus::interop::{Direction, ExportedFile, InteropAnswer};

use super::ProseState;
use super::clipboard;
use crate::events::ProseOutcome;
use crate::interop::{ProseIr, markdown_read};
use crate::message::{ConversionReport, InteropError, ProseCommand, ProseImport};

/// What a module read for the command in flight.
pub(super) enum ConvertedInput {
    /// The tree of a paste's HTML half, or none when the module could not
    /// read it.
    Html(Option<HtmlTree>),
    /// A file mapped onto the form, or why it was not.
    Document(Result<(ProseIr, ConversionReport), InteropError>),
}

impl ConvertedInput {
    pub(super) fn html(self) -> Option<HtmlTree> {
        match self {
            Self::Html(tree) => tree,
            Self::Document(_) => None,
        }
    }

    pub(super) fn document(self) -> Option<Result<(ProseIr, ConversionReport), InteropError>> {
        match self {
            Self::Document(read) => Some(read),
            Self::Html(_) => None,
        }
    }
}

/// The family whose module reads `format`.
pub(super) const fn family_of(format: ImportFormat) -> ConverterFamily {
    match format {
        ImportFormat::Markdown => ConverterFamily::Markup,
        ImportFormat::Word => ConverterFamily::Ooxml,
    }
}

impl ProseState {
    /// The converter family `command` waits on, if any. A Markdown import
    /// waits on the markup module, as does a paste whose richest readable
    /// half is HTML. A Word import and a Word export wait on the OOXML
    /// module. A paste of this application's own bytes waits on nothing.
    #[must_use]
    pub fn conversion_family(command: &ProseCommand) -> Option<ConverterFamily> {
        match command {
            ProseCommand::Paste {
                html: Some(_),
                payload,
                ..
            } if !clipboard::is_own(payload.as_ref()) => Some(ConverterFamily::Markup),
            ProseCommand::Interop(InteropRequest::Import(ProseImport { format, .. })) => {
                Some(family_of(*format))
            }
            ProseCommand::Interop(InteropRequest::Export(ExportFormat::Word(_))) => {
                Some(ConverterFamily::Ooxml)
            }
            _ => None,
        }
    }

    /// Run a view command [`Self::conversion_family`] names, once its module
    /// read what the command carries. `Err` carries the reason the module did
    /// not load. An import then fails with it. A paste falls through to its
    /// plain text, so the user loses the formatting and keeps the words.
    pub fn handle_conversion(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, &str>,
    ) -> ProseOutcome {
        self.converted = match &command {
            ProseCommand::Paste {
                html: Some(html), ..
            } => Some(ConvertedInput::Html(converter.ok().and_then(|converter| {
                match converter.convert(&Conversion::ReadHtml { html: html.clone() }) {
                    Ok(Converted::Html(tree)) => Some(tree),
                    _ => None,
                }
            }))),
            ProseCommand::Interop(InteropRequest::Import(ProseImport { format, data, .. })) => {
                Some(ConvertedInput::Document(match (converter, format) {
                    (Ok(converter), ImportFormat::Markdown) => markdown_read::read(data, converter),
                    (Ok(converter), ImportFormat::Word) => word::read(data, converter),
                    (Err(reason), _) => Err(InteropError::Stopped(reason.to_owned())),
                }))
            }
            // An export writes nothing, so it waits on no view command: the
            // walk, the mapping and the module answer it here.
            ProseCommand::Interop(InteropRequest::Export(ExportFormat::Word(review))) => {
                return self.export_word(*review, converter);
            }
            _ => None,
        };
        let outcome = self.handle_view_command(command);
        self.converted = None;
        outcome
    }
}

impl ProseState {
    /// The document as a Word package: the one walk, the mapping onto the
    /// Word model, then the writer of the OOXML module. Read-only: no delta
    /// (`PROSE-IO-017`). The report names what Word could not hold
    /// (`PROSE-IO-014`, `PROSE-IO-006`).
    ///
    /// Without the review, the walk drops every thread and every suggestion
    /// first (`PROSE-IO-081`).
    fn export_word(
        &self,
        review: ReviewExport,
        converter: Result<&Converter, &str>,
    ) -> ProseOutcome {
        let failed = |error: InteropError| {
            ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                error,
            ))])
        };
        let converter = match converter {
            Ok(converter) => converter,
            Err(reason) => return failed(InteropError::Stopped(reason.to_owned())),
        };
        let ir = match review {
            ReviewExport::Included => self.walk(),
            ReviewExport::Omitted => self.walk().without_review(),
        };
        // The title is user text, so the name is cleaned of what a host
        // refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        let filename = ambre_core::ExportFilename::from_title(
            ir.meta.title.as_deref().unwrap_or_default(),
            "document",
            ambre_core::ExportExtension::Docx,
        );
        match word::write(&ir, converter) {
            Ok((data, report)) => {
                ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::ExportReady {
                    file: ExportedFile {
                        filename,
                        media_type: ExportFormat::Word(review).media_type(),
                        data,
                    },
                    report,
                })])
            }
            Err(e) => failed(e),
        }
    }
}

#[cfg(test)]
impl ProseState {
    /// [`Self::handle_conversion`] through the markup engine the host links
    /// in place. A test drives a paste or an import as the Logic dispatcher
    /// does once the module loaded.
    pub(crate) fn handle_converted(&mut self, command: ProseCommand) -> ProseOutcome {
        self.handle_conversion(command, Ok(&host_converter()))
    }

    /// [`Self::handle_conversion`] through the engine of the family the
    /// command waits on, linked in place as the host links it.
    pub(crate) fn handle_converted_in_family(&mut self, command: ProseCommand) -> ProseOutcome {
        let family = Self::conversion_family(&command).unwrap_or(ConverterFamily::Markup);
        match Converter::load(family, &ambre_core::Payload::new(Vec::new())) {
            Ok(converter) => self.handle_conversion(command, Ok(&converter)),
            Err(e) => panic!("the host links the {family} engine: {e}"),
        }
    }
}

/// The markup engine the host links in place: a test converts through the
/// request bytes a module reads.
#[cfg(test)]
pub(crate) fn host_converter() -> Converter {
    match Converter::load(
        ConverterFamily::Markup,
        &ambre_core::Payload::new(Vec::new()),
    ) {
        Ok(converter) => converter,
        Err(e) => panic!("the host links the markup engine: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ProseEvent;
    use crate::message::DocChar;
    use ambre_core::index::Pos;

    /// `ARCH-086`: a paste of Ambre's own bytes reads no HTML, so it waits on
    /// no module. The same HTML from another application waits on the markup
    /// module.
    #[test]
    fn a_paste_of_ambre_bytes_waits_on_no_module() {
        let mut state = ProseState::new().expect("a document");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "own".to_owned(),
        });
        let copied = state.handle_command(ProseCommand::CopyRange {
            anchor: (block, Pos::<DocChar>::ZERO),
            focus: (block, Pos::<DocChar>::new(3)),
        });
        let Some(ProseEvent::Clipboard { payload, html, .. }) = copied.display.into_iter().next()
        else {
            panic!("a copy answers with the clipboard");
        };
        let paste = |payload| ProseCommand::Paste {
            at: (block, Pos::<DocChar>::ZERO),
            text: "own".to_owned(),
            html: Some(html.clone()),
            payload,
        };
        assert_eq!(ProseState::conversion_family(&paste(Some(payload))), None);
        assert_eq!(
            ProseState::conversion_family(&paste(None)),
            Some(ConverterFamily::Markup)
        );
    }
}
