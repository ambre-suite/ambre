//! The commands that wait on a converter module
//! (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! A Markdown or HTML import and export, and a paste that carries HTML from
//! another application, need the markup module. A Word import and a Word export need
//! the Office module. The Logic dispatcher asks
//! [`ProseState::conversion_family`] which module a command waits on, loads
//! it, then hands the command back through [`ProseState::handle_conversion`].
//!
//! A file import, a text export and a PDF export each call their module on
//! the converter thread, never on the Logic thread (`ARCH-122`,
//! `ARCH-124`). The document
//! owes the request, the Logic dispatcher drains it through
//! [`ProseState::take_file_requests`], and the answer comes back through
//! [`ProseState::handle_file_converted`]. A read lands then, on the path a
//! view command runs: caught up, refused while suggesting where it must be,
//! and named on a refusal. A paste still calls its module in place.

use ambre_bus::Request;
use ambre_bus::interop::InteropRequest;
use ambre_client_core::ExportFilename;
use ambre_convert::{Conversion, ConvertError, Converted, Converter, FileWork, TextRead};
use ambre_core::{ConverterFamily, Seq, StopCause};
use ambre_prose_form::clip::ClipItem;

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
    /// The items a paste's HTML half reads into, or none when the module
    /// could not read it.
    Html(Option<Vec<ClipItem>>),
    /// A file mapped onto the form, or why it was not.
    Document(Result<(ProseIr, ConversionReport), InteropError>),
}

impl ConvertedInput {
    pub(super) fn html(self) -> Option<Vec<ClipItem>> {
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

/// A file conversion the converter thread runs for this document, until its
/// answer lands (`ARCH-122`).
pub(super) enum FileJob {
    /// The module reads the file of this import command.
    Read(ProseCommand),
    /// The module writes the form of an export to `format`, which leaves
    /// under `filename`.
    Write {
        format: ExportFormat,
        filename: ExportFilename,
    },
    /// The PDF module writes the recorded pages of an export to `format`,
    /// which leaves under `filename` with `report` (`ARCH-124`).
    Print {
        format: ExportFormat,
        filename: ExportFilename,
        report: ConversionReport,
    },
}

/// The file conversions of one document on the converter thread.
pub(super) type FileJobs = ambre_convert::FileJobs<Request, FileJob>;

impl ambre_convert::FileJob for FileJob {
    fn work(&self) -> FileWork {
        match self {
            Self::Read(_) => FileWork::Read,
            Self::Write { .. } | Self::Print { .. } => FileWork::Write,
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

/// The request that reads the file of an import, or why none leaves.
fn read_request(import: &ProseImport) -> Result<Conversion, InteropError> {
    let ProseImport {
        format, data, zone, ..
    } = import;
    match format {
        ImportFormat::Markdown => markdown::read_request(data),
        ImportFormat::Html => html::read_request(data),
        ImportFormat::Word => word::read_request(data, zone, TextPackage::Word),
        ImportFormat::OpenDocument => word::read_request(data, zone, TextPackage::OpenDocument),
    }
}

/// The form the module answered the read of `format`.
fn read_answer(
    format: ImportFormat,
    answer: Result<Converted, ConvertError>,
) -> Result<(ProseIr, ConversionReport), InteropError> {
    match format {
        ImportFormat::Markdown => markdown::read_answer(answer),
        ImportFormat::Html => html::read_answer(answer),
        ImportFormat::Word => word::read_answer(answer, TextPackage::Word),
        ImportFormat::OpenDocument => word::read_answer(answer, TextPackage::OpenDocument),
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
    /// loaded. `Err` carries why the module did not load. An import then
    /// fails with it. A paste falls through to its plain text, so the user
    /// loses the formatting and keeps the words.
    ///
    /// An import owes the converter thread its read and changes nothing yet:
    /// it lands once [`Self::handle_file_converted`] hands the answer back
    /// (`ARCH-122`). A file refused before the read fails at once.
    pub fn handle_conversion(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, StopCause>,
    ) -> ProseOutcome {
        if let ProseCommand::Interop(InteropRequest::Import(import)) = &command {
            let request = match converter {
                Ok(_) => read_request(import),
                Err(cause) => Err(InteropError::Stopped(cause)),
            };
            return match request {
                Ok(conversion) => {
                    self.file_jobs.ask(
                        ambre_bus::next_request(),
                        conversion,
                        FileJob::Read(command),
                    );
                    ProseOutcome::nothing()
                }
                Err(e) => self.land_read(command, Err(e)),
            };
        }
        // A conversion that still runs lands before the next one starts, so
        // two never interleave on one document (`PROSE-IO-100`).
        let landed = self.land_conversion();
        let outcome = self.convert_command(command, converter);
        self.carry_landed(landed, outcome)
    }

    /// Run the import `command` over what its read gave, after the running
    /// conversion landed, so two never interleave (`PROSE-IO-100`).
    fn land_read(
        &mut self,
        command: ProseCommand,
        read: Result<(ProseIr, ConversionReport), InteropError>,
    ) -> ProseOutcome {
        let landed = self.land_conversion();
        self.converted = Some(ConvertedInput::Document(read));
        let outcome = self.handle_view_command(command);
        self.converted = None;
        self.carry_landed(landed, outcome)
    }

    /// The file conversions this document owes the converter thread,
    /// drained. The Logic dispatcher sends each one and never waits for it
    /// (`ARCH-122`).
    pub fn take_file_requests(&mut self) -> Vec<(Seq<Request>, Conversion)> {
        self.file_jobs.take_requests()
    }

    /// What the file conversion under `ticket` does, or `None` when the
    /// ticket is not one of this document's. A read writes the document once
    /// it lands; a write never does.
    #[must_use]
    pub fn file_work(&self, ticket: Seq<Request>) -> Option<FileWork> {
        self.file_jobs.work(ticket)
    }

    /// Land the answer of a file conversion this document asked for, or
    /// answer `None` when the ticket is not one of its own. A read runs its
    /// import now, after the running conversion landed (`PROSE-IO-100`). A
    /// write answers the file, or why there is none.
    pub fn handle_file_converted(
        &mut self,
        ticket: Seq<Request>,
        answer: Result<Converted, ConvertError>,
    ) -> Option<ProseOutcome> {
        Some(match self.file_jobs.land(ticket)? {
            FileJob::Read(command) => {
                let read = match &command {
                    ProseCommand::Interop(InteropRequest::Import(import)) => {
                        read_answer(import.format, answer)
                    }
                    _ => Err(InteropError::Stopped(StopCause::Misrouted)),
                };
                self.land_read(command, read)
            }
            FileJob::Write { format, filename } => {
                let written = match &format {
                    ExportFormat::Markdown(_) => markdown::write_answer(answer),
                    ExportFormat::Html(_) => html::write_answer(answer),
                    ExportFormat::OpenDocument(_) => {
                        word::write_answer(answer, TextPackage::OpenDocument)
                    }
                    ExportFormat::Word(_) => word::write_answer(answer, TextPackage::Word),
                    ExportFormat::Pdf(_) | ExportFormat::Ambre => {
                        Err(InteropError::Stopped(StopCause::Misrouted))
                    }
                };
                Self::exported(
                    &format,
                    written.map(|(data, report)| (filename, data, report)),
                )
            }
            FileJob::Print {
                format,
                filename,
                report,
            } => {
                let written = match answer {
                    Ok(Converted::File(data)) => Ok((filename, data, report)),
                    Ok(_) => Err(InteropError::Stopped(StopCause::ConverterAnswered)),
                    Err(e) => Err(crate::interop::stopped(&e)),
                };
                Self::exported(&format, written)
            }
        })
    }

    fn convert_command(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, StopCause>,
    ) -> ProseOutcome {
        self.converted = match &command {
            ProseCommand::Paste {
                html: Some(html), ..
            } => Some(ConvertedInput::Html(converter.ok().and_then(|converter| {
                match converter.read_text(&TextRead::HtmlPaste(html)) {
                    Ok(Converted::Pasted(blocks)) => Some(blocks),
                    _ => None,
                }
            }))),
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
        converter: Result<&Converter, StopCause>,
    ) -> ProseOutcome {
        match converter {
            Ok(_) => self.start_export(format),
            Err(cause) => ProseOutcome::display_only(vec![ProseEvent::Interop(
                InteropAnswer::failed(Direction::Export, InteropError::Stopped(cause)),
            )]),
        }
    }

    /// Owe the converter thread the write of the walked form `ir`
    /// (`ARCH-122`). The file answers once the module wrote it, and the
    /// report names what the format could not hold (`PROSE-IO-018`,
    /// `PROSE-IO-014`, `PROSE-IO-006`).
    ///
    /// Without the review, a Word export drops every thread and every
    /// suggestion first (`PROSE-IO-081`).
    pub(super) fn ask_write(&mut self, format: ExportFormat, ir: ProseIr) -> ProseOutcome {
        // The title is user text, so the name is cleaned of what a host
        // refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        let named = |extension| {
            ExportFilename::from_title(
                ir.meta.title.as_deref().unwrap_or_default(),
                "document",
                extension,
            )
        };
        let request = match &format {
            ExportFormat::Markdown(flavor) => {
                let filename = named(ambre_client_core::ExportExtension::Markdown);
                markdown::write_request(&ir, *flavor).map(|r| (filename, r))
            }
            // Without the review, an HTML export drops every thread and every
            // suggestion first, as a Word export does (`PROSE-IO-022`).
            ExportFormat::Html(review) => {
                let filename = named(ambre_client_core::ExportExtension::Html);
                let ir = match review {
                    ReviewExport::Included => ir,
                    ReviewExport::Omitted => ir.without_review(),
                };
                html::write_request(&ir).map(|r| (filename, r))
            }
            ExportFormat::Word(export) | ExportFormat::OpenDocument(export) => {
                let (extension, package) = match &format {
                    ExportFormat::OpenDocument(_) => (
                        ambre_client_core::ExportExtension::Odt,
                        TextPackage::OpenDocument,
                    ),
                    _ => (ambre_client_core::ExportExtension::Docx, TextPackage::Word),
                };
                let filename = named(extension);
                let ir = match export.review {
                    ReviewExport::Included => ir,
                    ReviewExport::Omitted => ir.without_review(),
                };
                word::write_request(&ir, &export.zone, package).map(|r| (filename, r))
            }
            // The pages of a PDF are written by `Self::ask_print`, from the
            // pages recorded, and never from the form.
            ExportFormat::Pdf(_) => Err(InteropError::Stopped(StopCause::Misrouted)),
            // The native file is the Logic dispatcher's (`PROSE-IO-028`), and
            // waits on no module.
            ExportFormat::Ambre => Err(InteropError::Stopped(StopCause::Misrouted)),
        };
        match request {
            Ok((filename, conversion)) => {
                self.file_jobs.ask(
                    ambre_bus::next_request(),
                    conversion,
                    FileJob::Write { format, filename },
                );
                ProseOutcome::nothing()
            }
            Err(e) => Self::exported(&format, Err(e)),
        }
    }

    /// Owe the converter thread the write of the recorded pages of an export
    /// to `format`, a PDF (`ARCH-124`). The file answers once the PDF module
    /// wrote it, with `report` (`PDF-015`). The export wrote nothing to the
    /// document.
    pub(super) fn ask_print(
        &mut self,
        format: ExportFormat,
        document: ambre_draw::list::PrintDocument,
        report: ConversionReport,
    ) -> ProseOutcome {
        let filename = ExportFilename::from_title(
            document.meta.title.as_deref().unwrap_or_default(),
            "document",
            ambre_client_core::ExportExtension::Pdf,
        );
        self.file_jobs.ask(
            ambre_bus::next_request(),
            Conversion::WritePdf { document },
            FileJob::Print {
                format,
                filename,
                report,
            },
        );
        ProseOutcome::nothing()
    }

    /// The answer of an export to `format`: the file under its name with its
    /// report, or the failure.
    fn exported(
        format: &ExportFormat,
        written: Result<
            (
                ExportFilename,
                ambre_core::Payload<ambre_client_core::FileBytes>,
                ConversionReport,
            ),
            InteropError,
        >,
    ) -> ProseOutcome {
        let answer = match written {
            Ok((filename, data, report)) => InteropAnswer::ExportReady {
                file: ExportedFile {
                    filename,
                    media_type: format.media_type(),
                    data,
                },
                report,
            },
            Err(e) => InteropAnswer::failed(Direction::Export, e),
        };
        ProseOutcome::display_only(vec![ProseEvent::Interop(answer)])
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
        self.drained(first)
    }

    /// `first`, then every slice left of the conversion it started, and the
    /// file conversions each owes, run in place. The update is the last
    /// delta a slice answered.
    pub(crate) fn drained(&mut self, mut first: ProseOutcome) -> ProseOutcome {
        self.serve_files(&mut first);
        while self.has_pending_work() {
            let next = self.resume_in_place();
            first.display.extend(next.display);
            if next.update.is_some() {
                first.update = next.update;
            }
        }
        first
    }

    /// [`Self::handle_conversion`], then the file conversions it owes the
    /// converter thread, run in place as the thread runs them.
    pub(crate) fn convert_in_place(
        &mut self,
        command: ProseCommand,
        converter: Result<&Converter, StopCause>,
    ) -> ProseOutcome {
        let mut outcome = self.handle_conversion(command, converter);
        self.serve_files(&mut outcome);
        outcome
    }

    /// [`Self::settle_conversion`], then the file conversions it owes the
    /// converter thread, run in place.
    pub(crate) fn settle_in_place(&mut self) -> ProseOutcome {
        let mut outcome = self.settle_conversion();
        self.serve_files(&mut outcome);
        outcome
    }

    /// [`Self::resume_conversion`], then the file conversions it owes the
    /// converter thread, run in place as the thread runs them.
    pub(crate) fn resume_in_place(&mut self) -> ProseOutcome {
        let mut outcome = self.resume_conversion();
        self.serve_files(&mut outcome);
        outcome
    }

    /// Run each file conversion this document owes the converter thread
    /// through the engine of its family, linked in place, and land each
    /// answer after `outcome`. The update is the last delta an answer gave.
    pub(crate) fn serve_files(&mut self, outcome: &mut ProseOutcome) {
        loop {
            let requests = self.take_file_requests();
            if requests.is_empty() {
                return;
            }
            for (ticket, conversion) in requests {
                let family = conversion.family();
                let answer = match Converter::load(family, &ambre_core::Payload::new(Vec::new())) {
                    Ok(converter) => converter.convert(&conversion),
                    Err(e) => panic!("the host links the {family} engine: {e}"),
                };
                if let Some(next) = self.handle_file_converted(ticket, answer) {
                    outcome.display.extend(next.display);
                    if next.update.is_some() {
                        outcome.update = next.update;
                    }
                }
            }
        }
    }

    /// [`Self::handle_conversion`] through the engine of the family the
    /// command waits on, linked in place as the host links it.
    pub(crate) fn handle_converted_in_family(&mut self, command: ProseCommand) -> ProseOutcome {
        let family = Self::conversion_family(&command).unwrap_or(ConverterFamily::Markup);
        match Converter::load(family, &ambre_core::Payload::new(Vec::new())) {
            Ok(converter) => {
                let first = self.handle_conversion(command, Ok(&converter));
                self.drained(first)
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
