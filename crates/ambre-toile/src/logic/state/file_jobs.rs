//! The board files a converter module reads or writes, on the converter
//! thread and never on the Logic thread (`ARCH-129`).
//!
//! [`ToileState::handle_conversion`] owes the module call as a request under
//! a ticket, and changes nothing yet. The Logic dispatcher drains it through
//! [`ToileState::take_file_requests`]. The answer comes back through
//! [`ToileState::handle_file_converted`]. A read then runs its import command
//! in the usual frame, with the drawing or the pixels the module read. A
//! write answers the file.
//!
//! A format that a module reads adds an arm to [`read_conversion`] and to
//! [`read_of`], and nothing else: the mechanism stays one.

use ambre_bus::interop::{Direction, ExportedFile, InteropAnswer, InteropError, InteropRequest};
use ambre_convert::{Conversion, ConvertError, Converted, FileWork};
use ambre_core::Seq;
use kurbo::Point;

use super::ToileState;
use super::import::{ConvertedFile, ReadDeck};
use super::pptx_export::{PPTX_MEDIA_TYPE, PPTX_STEM};
use crate::logic::command::{Request, ToileEvent, ToileOutcome};
use crate::message::{ToileCommand, ToileExport, ToileImport, ToileImportFormat, ToileReport};

/// A file conversion the converter thread runs for this board, until its
/// answer lands. Each variant holds what the answer needs and the module call
/// does not carry.
pub(super) enum ToileFileJob {
    /// The module reads a file of `format`, which lands at `at`. The bytes
    /// travel with the module call alone.
    Read {
        format: ToileImportFormat,
        at: Point,
    },
    /// The module writes the deck as a PowerPoint package. `report` holds
    /// what the board walk approximated and dropped.
    WritePptx { report: Box<ToileReport> },
}

impl ambre_convert::FileJob for ToileFileJob {
    fn work(&self) -> FileWork {
        match self {
            Self::Read { .. } => FileWork::Read,
            Self::WritePptx { .. } => FileWork::Write,
        }
    }
}

/// The file conversions of one board on the converter thread.
pub(super) type FileJobs = ambre_convert::FileJobs<Request, ToileFileJob>;

/// The module call that reads a file of `format`, or `None` when no module
/// reads that file. A picture other than WebP is read on the Logic thread, as
/// it came.
fn read_conversion(
    format: &ToileImportFormat,
    data: &ambre_core::Payload<ambre_client_core::FileBytes>,
) -> Option<Conversion> {
    let bound = crate::logic::interop::Limits::default().max_image;
    match format {
        ToileImportFormat::Svg => Some(Conversion::ReadSvg { data: data.clone() }),
        ToileImportFormat::Board => Some(Conversion::ReadBoard { data: data.clone() }),
        ToileImportFormat::Pptx => Some(Conversion::ReadPptx {
            data: data.clone(),
            bound,
        }),
        ToileImportFormat::Odp => Some(Conversion::ReadOdp {
            data: data.clone(),
            bound,
        }),
        ToileImportFormat::Image(_) if ambre_convert::is_webp(data) => {
            Some(Conversion::DecodeWebp { data: data.clone() })
        }
        ToileImportFormat::Image(_) => None,
    }
}

/// What the import of `format` reads from the answer of its module call.
fn read_of(
    format: &ToileImportFormat,
    answer: Result<Converted, ConvertError>,
) -> Option<ConvertedFile> {
    let read = answer.map_err(|e| match e {
        ConvertError::Refused(refusal) => InteropError::ParseFailed(refusal),
        e => {
            tracing::warn!("Toile: a conversion stopped: {e}");
            InteropError::Stopped(e.stop_cause())
        }
    });
    let malformed = || InteropError::Stopped(ambre_core::StopCause::ConverterAnswered);
    Some(match format {
        ToileImportFormat::Svg => ConvertedFile::Drawing(read.and_then(|c| match c {
            Converted::Svg(tree) => Ok(tree),
            _ => Err(malformed()),
        })),
        ToileImportFormat::Image(_) => ConvertedFile::Picture(read.and_then(|c| match c {
            Converted::Pixels(pixels) => Ok(pixels),
            _ => Err(malformed()),
        })),
        ToileImportFormat::Board => ConvertedFile::Board(read.and_then(|c| match c {
            Converted::Board(bytes) => {
                ambre_convert::boards::decode_board(&bytes).map_err(|_| malformed())
            }
            _ => Err(malformed()),
        })),
        ToileImportFormat::Pptx | ToileImportFormat::Odp => {
            ConvertedFile::Deck(read.and_then(|c| {
                match c {
                    Converted::Deck { deck, thumbs } => ambre_convert::decode_imported_deck(&deck)
                        .map(|deck| ReadDeck { deck, thumbs })
                        .map_err(|_| malformed()),
                    _ => Err(malformed()),
                }
            }))
        }
    })
}

impl ToileState {
    /// The converter family `cmd` waits on, if any. The images module reads
    /// an SVG drawing and a WebP picture, which Main cannot decode. The boards
    /// module reads a Figma or a Miro board. The Office module reads a
    /// presentation and writes a PowerPoint export. The PDF
    /// export waits on the PDF module, which the Logic dispatcher serves
    /// itself.
    #[must_use]
    pub fn conversion_family(cmd: &ToileCommand) -> Option<ambre_core::ConverterFamily> {
        match cmd {
            ToileCommand::Interop(InteropRequest::Import(ToileImport { format, data, .. })) => {
                read_conversion(format, data).map(|c| c.family())
            }
            ToileCommand::Interop(InteropRequest::Export(ToileExport::Pptx)) => {
                Some(ambre_core::ConverterFamily::Office)
            }
            _ => None,
        }
    }

    /// Serve a command [`Self::conversion_family`] names, once its module is
    /// loaded. The module call runs on the converter thread, never here
    /// (`ARCH-129`). This owes the request and changes nothing yet. A
    /// PowerPoint export walks the board now, so its file holds the board as
    /// the command found it. Any other command is served as
    /// [`handle_command`](Self::handle_command) serves it.
    pub fn handle_conversion(&mut self, cmd: ToileCommand) -> ToileOutcome {
        match cmd {
            ToileCommand::Interop(InteropRequest::Export(ToileExport::Pptx)) => {
                self.ask_pptx_write()
            }
            ToileCommand::Interop(InteropRequest::Import(ToileImport { format, data, at })) => {
                match read_conversion(&format, &data) {
                    Some(conversion) => {
                        self.file_jobs.ask(
                            ambre_bus::next_request(),
                            conversion,
                            ToileFileJob::Read { format, at },
                        );
                        ToileOutcome::display_only(Vec::new())
                    }
                    None => self.handle_command(ToileCommand::Interop(InteropRequest::Import(
                        ToileImport { format, data, at },
                    ))),
                }
            }
            other => self.handle_command(other),
        }
    }

    /// The file conversions this board owes the converter thread, drained.
    /// The Logic dispatcher sends each one and never waits for it
    /// (`ARCH-129`).
    pub fn take_file_requests(&mut self) -> Vec<(Seq<Request>, Conversion)> {
        self.file_jobs.take_requests()
    }

    /// What the file conversion under `ticket` does, or `None` when the
    /// ticket is not one of this board's. A read writes the board once it
    /// lands; a write never does.
    #[must_use]
    pub fn file_work(&self, ticket: Seq<Request>) -> Option<FileWork> {
        self.file_jobs.work(ticket)
    }

    /// Land the answer of a file conversion this board asked for, or answer
    /// `None` when the ticket is not one of its own (`ARCH-129`). A read runs
    /// its import command now, in the frame of any command, with what the
    /// module read. A write answers the file, or why there is none.
    pub fn handle_file_converted(
        &mut self,
        ticket: Seq<Request>,
        answer: Result<Converted, ConvertError>,
    ) -> Option<ToileOutcome> {
        Some(match self.file_jobs.land(ticket)? {
            ToileFileJob::Read { format, at } => {
                self.converted = read_of(&format, answer);
                // The bytes left with the module call. The import arm reads
                // what the module read, never the bytes.
                let outcome = self.handle_command(ToileCommand::import(
                    format,
                    ambre_core::Payload::new(Vec::new()),
                    at,
                ));
                self.converted = None;
                outcome
            }
            ToileFileJob::WritePptx { report } => pptx_written(answer, *report),
        })
    }

    /// Owe the converter thread the write of the deck as a PowerPoint file
    /// (`TOILE-IO-126`). The board walk runs now. A board with no element, or
    /// a deck that does not encode, fails at once with its reason and no file
    /// (`TOILE-IO-128`). Read-only: the outcome carries no update
    /// (`TOILE-IO-127`).
    fn ask_pptx_write(&mut self) -> ToileOutcome {
        let Some(crate::logic::interop::PptxExport { deck, report }) = self.export_pptx_deck()
        else {
            return Self::interop_failed(
                Direction::Export,
                InteropError::Stopped(ambre_core::StopCause::NothingToConvert),
            );
        };
        match ambre_convert::encode_deck(&deck) {
            Ok(deck) => {
                self.file_jobs.ask(
                    ambre_bus::next_request(),
                    Conversion::WritePptx { deck },
                    ToileFileJob::WritePptx {
                        report: Box::new(report),
                    },
                );
                ToileOutcome::display_only(Vec::new())
            }
            Err(e) => {
                tracing::warn!("Toile: the PowerPoint export stopped: {e}");
                Self::interop_failed(Direction::Export, InteropError::Stopped(e.stop_cause()))
            }
        }
    }
}

/// The answer to a PowerPoint export once the module wrote the package. It is
/// one [`InteropAnswer::ExportReady`] the UI thread saves and reports
/// (`TOILE-IO-126`), or why there is none (`TOILE-IO-128`). The report is the
/// map's: the writer carries every item it receives (`TOILE-IO-125`).
fn pptx_written(answer: Result<Converted, ConvertError>, report: ToileReport) -> ToileOutcome {
    let written = match answer {
        Ok(Converted::File(data)) => Ok(data),
        Ok(_) => Err(ambre_core::StopCause::ConverterAnswered),
        Err(e) => {
            tracing::warn!("Toile: the PowerPoint export stopped: {e}");
            Err(e.stop_cause())
        }
    };
    match written {
        Ok(data) => {
            ToileOutcome::display_only(vec![ToileEvent::Interop(InteropAnswer::ExportReady {
                file: ExportedFile {
                    filename: ambre_client_core::ExportFilename::named(
                        PPTX_STEM,
                        ambre_client_core::ExportExtension::Pptx,
                    ),
                    media_type: ambre_core::MediaType::from(PPTX_MEDIA_TYPE),
                    data,
                },
                report,
            })])
        }
        Err(cause) => ToileState::interop_failed(Direction::Export, InteropError::Stopped(cause)),
    }
}

#[cfg(test)]
impl ToileState {
    /// Serve `cmd` as the Logic dispatcher and the converter thread do. The
    /// host links the module of its family in place. This owes the call, runs
    /// it and lands the answer. The outcome of the command comes first.
    pub(crate) fn handle_converted(&mut self, cmd: ToileCommand) -> ToileOutcome {
        let mut outcome = self.handle_conversion(cmd);
        for (ticket, conversion) in self.take_file_requests() {
            let answer = match ambre_convert::Converter::load(
                conversion.family(),
                &ambre_core::Payload::new(Vec::new()),
            ) {
                Ok(converter) => converter.convert(&conversion),
                Err(e) => panic!("the host links the module: {e}"),
            };
            if let Some(landed) = self.handle_file_converted(ticket, answer) {
                outcome.display.extend(landed.display);
                outcome.update = landed.update.or(outcome.update);
                outcome.ephemeral = landed.ephemeral.or(outcome.ephemeral);
                outcome.requests.extend(landed.requests);
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use ambre_bus::interop::{InteropAnswer, InteropRequest};
    use ambre_convert::{Conversion, FileWork};
    use ambre_core::{Count, Elements};
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::state::ToileState;
    use crate::message::{ToileExport, ToileImportFormat};

    /// A drawing of one rect.
    const SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="4" height="4"/></svg>"#;

    /// The module the host links for `conversion`, run in place.
    fn serve(
        conversion: &Conversion,
    ) -> Result<ambre_convert::Converted, ambre_convert::ConvertError> {
        ambre_convert::Converter::load(conversion.family(), &ambre_core::Payload::new(Vec::new()))
            .and_then(|converter| converter.convert(conversion))
    }

    /// `ARCH-129`: an SVG import owes its read to the converter thread and
    /// writes nothing. The answer lands the drawing.
    #[test]
    fn an_svg_import_owes_its_read_to_the_converter_thread_and_writes_on_the_answer() {
        let mut st = ToileState::new();
        let out = st.handle_conversion(ToileCommand::import(
            ToileImportFormat::Svg,
            ambre_core::Payload::new(SVG.to_vec()),
            Point::ZERO,
        ));
        assert!(
            out.update.is_none() && out.display.is_empty(),
            "nothing lands yet"
        );
        let requests = st.take_file_requests();
        let [(ticket, conversion)] = &requests[..] else {
            panic!("one read owed: {}", requests.len());
        };
        assert!(matches!(conversion, Conversion::ReadSvg { .. }));
        assert!(st.take_file_requests().is_empty(), "drained once");
        assert_eq!(st.file_work(*ticket), Some(FileWork::Read));
        assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        let landed = st
            .handle_file_converted(*ticket, serve(conversion))
            .expect("the ticket is the board's");
        assert!(landed.update.is_some(), "the answer writes the board");
        assert!(st.element_count() > Count::<Elements>::ZERO);
        assert_eq!(st.file_work(*ticket), None, "the job left");
        assert!(
            st.handle_file_converted(*ticket, serve(conversion))
                .is_none(),
            "a ticket lands once"
        );
    }

    /// `ARCH-129`: a Figma or Miro board and a deck each owe their read to
    /// the module of their family. The board stays as it was until the
    /// answer lands.
    #[test]
    fn a_board_or_deck_import_owes_its_read_to_the_converter_thread() {
        let cases = [
            (
                ToileImportFormat::Board,
                ambre_core::ConverterFamily::Boards,
            ),
            (ToileImportFormat::Pptx, ambre_core::ConverterFamily::Office),
            (ToileImportFormat::Odp, ambre_core::ConverterFamily::Office),
        ];
        for (format, family) in cases {
            let mut st = ToileState::new();
            let out = st.handle_conversion(ToileCommand::import(
                format.clone(),
                ambre_core::Payload::new(b"{}".to_vec()),
                Point::ZERO,
            ));
            assert!(
                out.update.is_none() && out.display.is_empty(),
                "{format:?}: nothing lands yet"
            );
            let requests = st.take_file_requests();
            let [(ticket, conversion)] = &requests[..] else {
                panic!("{format:?}: one read owed: {}", requests.len());
            };
            assert_eq!(conversion.family(), family, "{format:?}");
            assert!(
                matches!(
                    conversion,
                    Conversion::ReadBoard { .. }
                        | Conversion::ReadPptx { .. }
                        | Conversion::ReadOdp { .. }
                ),
                "{format:?}"
            );
            assert_eq!(st.file_work(*ticket), Some(FileWork::Read));
            assert_eq!(st.element_count(), Count::<Elements>::ZERO);
        }
    }

    /// `ARCH-129`: a PowerPoint export walks the board on its command and owes
    /// the write. The answer hands the file over, and writes nothing.
    #[test]
    fn a_pptx_export_owes_its_write_to_the_converter_thread() {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(10.0, 20.0),
            size: Size::new(30.0, 40.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        let out = st.handle_conversion(ToileCommand::Interop(InteropRequest::Export(
            ToileExport::Pptx,
        )));
        assert!(out.display.is_empty(), "the write is in flight");
        let requests = st.take_file_requests();
        let [(ticket, conversion)] = &requests[..] else {
            panic!("one write owed: {}", requests.len());
        };
        assert!(matches!(conversion, Conversion::WritePptx { .. }));
        assert_eq!(st.file_work(*ticket), Some(FileWork::Write));
        let landed = st
            .handle_file_converted(*ticket, serve(conversion))
            .expect("the ticket is the board's");
        assert!(landed.update.is_none(), "an export writes nothing");
        assert!(matches!(
            landed.display.as_slice(),
            [ToileEvent::Interop(InteropAnswer::ExportReady { file, .. })]
                if file.data.as_bytes().starts_with(b"PK")
        ));
    }
}
