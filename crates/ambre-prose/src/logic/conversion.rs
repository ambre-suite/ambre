//! The commands that wait on a converter module
//! (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! A Markdown or HTML import and export, and a paste that carries HTML from
//! another application, need the markup module. A Word import and a Word export need
//! the Office module. The Logic dispatcher asks
//! [`ProseState::conversion_family`] which module a command waits on, loads
//! it, then hands the command back through [`ProseState::handle_conversion`].
//! The module reads first. The command then runs the path a view command
//! runs: caught up, refused while suggesting where it must be, and named on a
//! refusal.

use ambre_bus::interop::InteropRequest;
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::ConverterFamily;
use ambre_prose_form::clip::ClipBlock;

use crate::events::ProseEvent;
use crate::interop::word::TextPackage;
use crate::interop::{html, markdown, word};
use crate::message::{ExportFormat, ImportFormat, ReviewExport};
use ambre_bus::interop::{Direction, ExportedFile, InteropAnswer};

use super::ProseState;
use super::clipboard;
use crate::events::ProseOutcome;
use crate::interop::ProseIr;
use crate::message::{ConversionReport, InteropError, ProseCommand, ProseImport};

/// What a module read for the command in flight.
pub(super) enum ConvertedInput {
    /// The blocks a paste's HTML half reads into, or none when the module
    /// could not read it.
    Html(Option<Vec<ClipBlock>>),
    /// A file mapped onto the form, or why it was not.
    Document(Result<(ProseIr, ConversionReport), InteropError>),
}

impl ConvertedInput {
    pub(super) fn html(self) -> Option<Vec<ClipBlock>> {
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
        ImportFormat::Markdown | ImportFormat::Html => ConverterFamily::Markup,
        ImportFormat::Word | ImportFormat::OpenDocument => ConverterFamily::Office,
    }
}

impl ProseState {
    /// The converter family `command` waits on, if any. A Markdown or HTML
    /// import and export wait on the markup module, as does a paste whose richest
    /// readable half is HTML. A Word import and a Word export wait on the
    /// Office module. A paste of this application's own bytes, and the native
    /// export, wait on nothing.
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
            ProseCommand::Interop(InteropRequest::Export(
                ExportFormat::Markdown(_) | ExportFormat::Html(_),
            )) => Some(ConverterFamily::Markup),
            ProseCommand::Interop(InteropRequest::Export(
                ExportFormat::Word(_) | ExportFormat::OpenDocument(_),
            )) => Some(ConverterFamily::Office),
            ProseCommand::Interop(InteropRequest::Export(ExportFormat::Pdf(_))) => {
                Some(ConverterFamily::Pdf)
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
        // A conversion that still runs lands before the next one starts, so
        // two never interleave on one document (`PROSE-IO-100`).
        let landed = self.land_conversion(converter.ok());
        let outcome = self.convert_command(command, converter);
        self.carry_landed(landed, outcome)
    }

    fn convert_command(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, &str>,
    ) -> ProseOutcome {
        self.converted = match &command {
            ProseCommand::Paste {
                html: Some(html), ..
            } => Some(ConvertedInput::Html(converter.ok().and_then(|converter| {
                match converter.convert(&Conversion::ReadHtml { html: html.clone() }) {
                    Ok(Converted::Pasted(blocks)) => Some(blocks),
                    _ => None,
                }
            }))),
            ProseCommand::Interop(InteropRequest::Import(ProseImport {
                format,
                data,
                zone,
                ..
            })) => Some(ConvertedInput::Document(match (converter, format) {
                (Ok(converter), ImportFormat::Markdown) => markdown::read(data, converter),
                (Ok(converter), ImportFormat::Html) => html::read(data, converter),
                (Ok(converter), ImportFormat::Word) => {
                    word::read(data, zone, converter, TextPackage::Word)
                }
                (Ok(converter), ImportFormat::OpenDocument) => {
                    word::read(data, zone, converter, TextPackage::OpenDocument)
                }
                (Err(reason), _) => Err(InteropError::Stopped(reason.to_owned())),
            })),
            // An export writes nothing, so it waits on no view command: the
            // walk, the mapping and the module answer it here.
            ProseCommand::Interop(InteropRequest::Export(
                format @ (ExportFormat::Markdown(_)
                | ExportFormat::Html(_)
                | ExportFormat::Word(_)
                | ExportFormat::OpenDocument(_)
                | ExportFormat::Pdf(_)),
            )) => {
                return self.export_through(format.clone(), converter);
            }
            _ => None,
        };
        let outcome = self.handle_view_command(command);
        self.converted = None;
        outcome
    }
}

impl ProseState {
    /// The document in a format a module writes: the one walk, one slice
    /// per pass, then the mapping and the writer of the module
    /// (`PROSE-IO-048`). Read-only: no delta (`PROSE-IO-017`).
    fn export_through(
        &mut self,
        format: ExportFormat,
        converter: Result<&Converter, &str>,
    ) -> ProseOutcome {
        match converter {
            Ok(converter) => self.start_export(format, converter),
            Err(reason) => ProseOutcome::display_only(vec![ProseEvent::Interop(
                InteropAnswer::failed(Direction::Export, InteropError::Stopped(reason.to_owned())),
            )]),
        }
    }

    /// Hand the walked form `ir` to the module, and answer the file. The
    /// report names what the format could not hold (`PROSE-IO-018`,
    /// `PROSE-IO-014`, `PROSE-IO-006`).
    ///
    /// Without the review, a Word export drops every thread and every
    /// suggestion first (`PROSE-IO-081`).
    pub(super) fn write_export(
        &self,
        format: ExportFormat,
        ir: ProseIr,
        converter: &Converter,
    ) -> ProseOutcome {
        let failed = |error: InteropError| {
            ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                error,
            ))])
        };
        // The title is user text, so the name is cleaned of what a host
        // refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        let named = |extension| {
            ambre_core::ExportFilename::from_title(
                ir.meta.title.as_deref().unwrap_or_default(),
                "document",
                extension,
            )
        };
        let written = match &format {
            ExportFormat::Markdown(flavor) => {
                let filename = named(ambre_core::ExportExtension::Markdown);
                markdown::write(&ir, *flavor, converter).map(|w| (filename, w))
            }
            // Without the review, an HTML export drops every thread and every
            // suggestion first, as a Word export does (`PROSE-IO-022`).
            ExportFormat::Html(review) => {
                let filename = named(ambre_core::ExportExtension::Html);
                let ir = match review {
                    ReviewExport::Included => ir,
                    ReviewExport::Omitted => ir.without_review(),
                };
                html::write(&ir, converter).map(|w| (filename, w))
            }
            ExportFormat::Word(export) | ExportFormat::OpenDocument(export) => {
                let (extension, package) = match &format {
                    ExportFormat::OpenDocument(_) => {
                        (ambre_core::ExportExtension::Odt, TextPackage::OpenDocument)
                    }
                    _ => (ambre_core::ExportExtension::Docx, TextPackage::Word),
                };
                let filename = named(extension);
                let ir = match export.review {
                    ReviewExport::Included => ir,
                    ReviewExport::Omitted => ir.without_review(),
                };
                word::write(&ir, &export.zone, converter, package).map(|w| (filename, w))
            }
            // The pages of a PDF are written by `Self::write_pdf`, from the
            // pages recorded, and never from the form.
            ExportFormat::Pdf(_) => Err(InteropError::Stopped(
                "a PDF is written from its pages".to_owned(),
            )),
            // The native file is the Logic dispatcher's (`PROSE-IO-028`), and
            // waits on no module.
            ExportFormat::Ambre => Err(InteropError::Stopped(
                "the native file is written by the Logic dispatcher".to_owned(),
            )),
        };
        match written {
            Ok((filename, (data, report))) => {
                ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::ExportReady {
                    file: ExportedFile {
                        filename,
                        media_type: format.media_type(),
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
    ///
    /// A conversion that runs in slices runs to its end, one slice per call,
    /// as the Logic thread takes them. Every event comes back in order.
    pub(crate) fn handle_converted(&mut self, command: ProseCommand) -> ProseOutcome {
        let converter = host_converter();
        let first = self.handle_conversion(command, Ok(&converter));
        self.drained(first, &converter)
    }

    /// `first`, then every slice left of the conversion it started, through
    /// `converter`. The update is the last delta a slice answered.
    pub(crate) fn drained(
        &mut self,
        mut first: ProseOutcome,
        converter: &Converter,
    ) -> ProseOutcome {
        while self.has_pending_work() {
            let next = self.resume_conversion(Some(converter));
            first.display.extend(next.display);
            if next.update.is_some() {
                first.update = next.update;
            }
        }
        first
    }

    /// [`Self::handle_conversion`] through the engine of the family the
    /// command waits on, linked in place as the host links it.
    pub(crate) fn handle_converted_in_family(&mut self, command: ProseCommand) -> ProseOutcome {
        let family = Self::conversion_family(&command).unwrap_or(ConverterFamily::Markup);
        match Converter::load(family, &ambre_core::Payload::new(Vec::new())) {
            Ok(converter) => {
                let first = self.handle_conversion(command, Ok(&converter));
                self.drained(first, &converter)
            }
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
