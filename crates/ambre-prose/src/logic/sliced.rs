//! A conversion applied one slice per pass of the Logic thread
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! A converter module reads a file, or writes one, in one call. What costs
//! the time is the part that sees the document: an import writes each block,
//! and an export walks each one. So that part runs in slices, and the Logic
//! thread serves the other documents and the view between two of them
//! (`PROSE-IO-048`, `ARCH-030`).
//!
//! An import first checks its whole list against the nesting rule, so a
//! refusal writes nothing (`PROSE-IO-011`). It then writes the list into the
//! document, one commit per slice, in one step of the history
//! (`PROSE-IO-051`). An export walks the top-level blocks, then hands the form
//! to the module. Each slice short of the end reports its progress
//! (`PROSE-IO-021`, `PROSE-IO-057`).
//!
//! A command that writes the document lands the running conversion first
//! (`PROSE-IO-100`). An edit then never joins the import's undo step, and an
//! export holds the document as it stood when the user asked for it.

use std::collections::HashMap;

use ambre_bus::interop::{ConversionProgress, Direction, InteropAnswer, InteropError, Steps};
use ambre_convert::Converter;
use ambre_core::{ConverterFamily, Count};

use super::ProseState;
use super::interop::{ImportAnnotations, ImportOp, ImportWriter, Walk};
use super::print::PrintJob;
use crate::events::{ProseEvent, ProseOutcome};
use crate::message::{ConversionReport, ExportFormat};
use ambre_draw::list::PrintDocument;

/// The steps one pass of a running import takes: one operation written into
/// the document is one step. The `import_slice` operation of
/// `benches/instructions.rs` holds one slice to its budget row
/// (`PROSE-IO-099`).
pub(super) const IMPORT_SLICE: Count<Steps> = Count::new(12);

/// The top-level blocks one pass of a running export walks. The
/// `export_slice` operation of `benches/instructions.rs` holds one slice to
/// its budget row (`PROSE-IO-099`).
pub(super) const EXPORT_SLICE: Count<Steps> = Count::new(48);

/// How much of a running conversion one call takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pace {
    /// One slice, so the other documents and the view wait no longer.
    Slice,
    /// Every step left, before what must read the document whole.
    Whole,
}

/// Who exports the delta a call commits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flush {
    /// The call: it answers the delta it wrote.
    Now,
    /// The command the conversion lands for: its own commit carries both.
    ByCaller,
}

/// The conversion a document is applying one slice per pass.
pub(super) enum Job {
    Import(Box<ImportJob>),
    Export(Box<ExportJob>),
}

/// An import between two slices: the list, what the reader lost, the
/// annotations the last slice writes, and where the write stands.
pub(super) struct ImportJob {
    ops: Vec<ImportOp>,
    report: ConversionReport,
    annotations: Option<ImportAnnotations>,
    writer: ImportWriter,
}

impl ImportJob {
    /// Every step of the import: each operation, written once.
    fn total(&self) -> Count<Steps> {
        Count::<Steps>::of(self.ops.len())
    }

    fn done(&self) -> Count<Steps> {
        self.writer.written()
    }

    fn progress(&self) -> ConversionProgress {
        ConversionProgress::new(Direction::Import, self.done(), self.total())
    }
}

/// An export between two slices: the format asked for, and the walk.
pub(super) struct ExportJob {
    format: ExportFormat,
    stage: ExportStage,
}

/// Where an export stands.
enum ExportStage {
    /// Top-level blocks are left to walk.
    Walking(Box<Walk>),
    /// Pages of a PDF are left to draw.
    Printing(Box<PrintJob>),
    /// Every page is drawn: the PDF module writes them on the next call
    /// that holds it.
    Printed(Box<(PrintDocument, ConversionReport)>),
}

/// The family whose module writes `format`, if a module does.
pub(crate) const fn export_family(format: &ExportFormat) -> Option<ConverterFamily> {
    match format {
        ExportFormat::Markdown(_) | ExportFormat::Html(_) => Some(ConverterFamily::Markup),
        ExportFormat::Word(_) | ExportFormat::OpenDocument(_) => Some(ConverterFamily::Office),
        ExportFormat::Pdf(_) => Some(ConverterFamily::Pdf),
        ExportFormat::Ambre => None,
    }
}

impl ProseState {
    /// Whether a conversion is still being applied (`PROSE-IO-048`). The
    /// Logic thread reads it to come back to this document without waiting
    /// for a message (`ARCH-030`).
    #[must_use]
    pub fn has_pending_work(&self) -> bool {
        self.conversion.is_some() || !self.queued.is_empty()
    }

    /// The family whose module the running conversion still calls on this
    /// thread. A PDF export hands its pages to it once the last one is
    /// drawn. A text export hands its form to the converter thread
    /// (`ARCH-122`), and an import needs none.
    #[must_use]
    pub fn pending_family(&self) -> Option<ConverterFamily> {
        let format = match &self.conversion {
            Some(Job::Export(job)) => &job.format,
            Some(Job::Import(_)) => return None,
            None => match self.queued.front() {
                Some(super::pictures::Queued::Export { format, .. }) => format,
                _ => return None,
            },
        };
        match format {
            ExportFormat::Pdf(_) => export_family(format),
            _ => None,
        }
    }

    /// Take the next slice of the running conversion, and answer its delta.
    /// `converter` is the module of [`Self::pending_family`], which the last
    /// slice of an export writes through. With nothing running it answers
    /// nothing.
    pub fn resume_conversion(&mut self, converter: Option<&Converter>) -> ProseOutcome {
        let mut outcome = self.run_conversion(Pace::Slice, Flush::Now, converter);
        self.report_history(&mut outcome);
        outcome
    }

    /// Land the rest of the running conversion now, and answer its delta:
    /// before a remote delta merges or the document closes (`ARCH-031`).
    pub fn settle_conversion(&mut self, converter: Option<&Converter>) -> ProseOutcome {
        let mut outcome = self.run_conversion(Pace::Whole, Flush::Now, converter);
        self.report_history(&mut outcome);
        outcome
    }

    /// Land the rest of the running conversion before a command, without
    /// exporting its delta: the command's own commit carries it. An export
    /// walks to its end and waits for its module. Answers the events of the
    /// landing, or `None` when nothing ran.
    pub(super) fn land_conversion(
        &mut self,
        converter: Option<&Converter>,
    ) -> Option<Vec<ProseEvent>> {
        self.conversion.as_ref()?;
        Some(
            self.run_conversion(Pace::Whole, Flush::ByCaller, converter)
                .display,
        )
    }

    /// Carry what [`Self::land_conversion`] answered into the outcome of the
    /// command it ran for. A command that committed nothing exports the
    /// landing's delta here (`PROSE-IO-100`).
    pub(super) fn carry_landed(
        &mut self,
        landed: Option<Vec<ProseEvent>>,
        mut outcome: ProseOutcome,
    ) -> ProseOutcome {
        let Some(mut landed) = landed else {
            return outcome;
        };
        let mut front = Vec::new();
        if outcome.update.is_none() {
            let flushed = self.commit_with(Vec::new());
            outcome.update = flushed.update;
            front = flushed.display;
        }
        front.append(&mut landed);
        front.append(&mut outcome.display);
        outcome.display = front;
        outcome
    }

    /// Start an import of `ops` after the last block, and take its first
    /// slice. A list of no operation completes at once. A list the nesting
    /// rule refuses answers the refusal and writes nothing (`PROSE-IO-011`).
    pub(super) fn start_import(
        &mut self,
        ops: Vec<ImportOp>,
        report: ConversionReport,
        annotations: Option<ImportAnnotations>,
    ) -> ProseOutcome {
        if ops.is_empty() {
            return ProseOutcome::display_only(vec![ProseEvent::Interop(
                InteropAnswer::ImportCompleted { report },
            )]);
        }
        if let Err(refused) = Self::check_import(&ops) {
            return refused;
        }
        // The writes that follow are one step of the history, however many
        // commits they take. The steps before it stay aside, so a refusal
        // gives them back (`PROSE-IO-120`).
        let writer = ImportWriter::start(self);
        self.history.hold();
        self.conversion = Some(Job::Import(Box::new(ImportJob {
            ops,
            report,
            annotations,
            writer,
        })));
        self.run_conversion(Pace::Slice, Flush::Now, None)
    }

    /// Start an export to `format`, and take its first slice. It writes
    /// nothing to the document (`PROSE-IO-017`).
    pub(super) fn start_export(
        &mut self,
        format: ExportFormat,
        converter: &Converter,
    ) -> ProseOutcome {
        // The pictures come first, through the media flow; a document with
        // none starts at once (`PROSE-IO-007`).
        let waited = self.fetch_pictures(format);
        if self.conversion.is_none() {
            return waited;
        }
        let mut outcome = self.run_conversion(Pace::Slice, Flush::Now, Some(converter));
        let mut display = waited.display;
        display.append(&mut outcome.display);
        outcome.display = display;
        outcome
    }

    /// Set up the walk of an export to `format` over the pictures `fetched`.
    /// The first slice is the caller's.
    pub(super) fn start_export_on(
        &mut self,
        format: ExportFormat,
        fetched: HashMap<
            ambre_core::Validated<ambre_core::ContentHash>,
            ambre_core::Payload<ambre_core::MediaBytes>,
        >,
    ) -> ProseOutcome {
        let stage = match &format {
            ExportFormat::Pdf(options) => match self.print_start(options, fetched) {
                Ok(job) => ExportStage::Printing(Box::new(job)),
                Err(e) => {
                    return ProseOutcome::display_only(vec![ProseEvent::Interop(
                        InteropAnswer::failed(Direction::Export, e),
                    )]);
                }
            },
            _ => {
                let mut walk = self.walk_start();
                walk.maps.pictures = fetched;
                ExportStage::Walking(Box::new(walk))
            }
        };
        self.conversion = Some(Job::Export(Box::new(ExportJob { format, stage })));
        ProseOutcome::nothing()
    }

    fn run_conversion(
        &mut self,
        pace: Pace,
        flush: Flush,
        converter: Option<&Converter>,
    ) -> ProseOutcome {
        if self.conversion.is_none()
            && let Some(next) = self.queued.pop_front()
        {
            // A conversion whose media answered while another ran starts
            // now, and this call takes its first slice.
            let started = self.start_queued(next);
            if self.conversion.is_none() {
                return started;
            }
        }
        let Some(job) = self.conversion.take() else {
            return ProseOutcome::nothing();
        };
        match job {
            Job::Import(job) => self.run_import(*job, pace, flush),
            Job::Export(job) => self.run_export(*job, pace, flush, converter),
        }
    }

    /// Take `pace` of a running import. A slice short of the end reports its
    /// progress, and the last one answers the report.
    fn run_import(&mut self, mut job: ImportJob, pace: Pace, flush: Flush) -> ProseOutcome {
        let room = match pace {
            Pace::Slice => IMPORT_SLICE,
            Pace::Whole => job.total(),
        };
        // The check of the list passed, so only an error of the CRDT engine
        // refuses here, and no input is known to raise one. The slices
        // already sent are taken back (`PROSE-IO-112`).
        let wrote = match self.write_ops(&mut job.writer, &mut job.ops, room) {
            Ok(taken) => taken > Count::ZERO,
            Err(_) => return self.withdraw_import(flush, &job.writer, job.annotations.as_ref()),
        };
        if job.done() < job.total() {
            let progress = ProseEvent::Interop(InteropAnswer::Progress(job.progress()));
            self.conversion = Some(Job::Import(Box::new(job)));
            return if wrote {
                self.commit_slice(vec![progress])
            } else {
                ProseOutcome::display_only(vec![progress])
            };
        }
        let ImportJob {
            report,
            annotations,
            mut writer,
            ..
        } = job;
        let anchors = match self.finish_write(&mut writer) {
            Ok(anchors) => anchors,
            Err(_) => return self.withdraw_import(flush, &writer, annotations.as_ref()),
        };
        let mut display = vec![ProseEvent::Interop(InteropAnswer::ImportCompleted {
            report,
        })];
        if let Some(annotations) = annotations.as_ref().filter(|a| !a.is_empty()) {
            if self.write_annotations(annotations, &anchors).is_err() {
                return self.withdraw_import(flush, &writer, Some(annotations));
            }
            display.push(ProseEvent::CommentsUpdated {
                threads: crate::comment::threads(&self.doc),
            });
            display.push(self.suggestions_event());
        }
        let outcome = match flush {
            Flush::Now => self.commit_with(display),
            Flush::ByCaller => {
                self.doc.commit();
                ProseOutcome::display_only(display)
            }
        };
        self.history.release();
        outcome
    }

    /// Take `pace` of a running export. A slice short of the end of the walk
    /// reports its progress. Once the walk ended, the form leaves for the
    /// converter thread (`ARCH-122`). The pages of a PDF are written
    /// once a call holds the PDF module. An export writes nothing to the
    /// document, so it flushes nothing.
    fn run_export(
        &mut self,
        mut job: ExportJob,
        pace: Pace,
        flush: Flush,
        converter: Option<&Converter>,
    ) -> ProseOutcome {
        match &mut job.stage {
            ExportStage::Walking(walk) => {
                let room = match pace {
                    Pace::Slice => EXPORT_SLICE,
                    Pace::Whole => walk.total(),
                };
                self.walk_roots(walk, room);
                if walk.walked() < walk.total() {
                    let progress =
                        ConversionProgress::new(Direction::Export, walk.walked(), walk.total());
                    self.conversion = Some(Job::Export(Box::new(job)));
                    return ProseOutcome::display_only(vec![ProseEvent::Interop(
                        InteropAnswer::Progress(progress),
                    )]);
                }
                let ExportStage::Walking(walk) = job.stage else {
                    return ProseOutcome::nothing();
                };
                // The form leaves for the converter thread, so the walk ends
                // the conversion of this document (`ARCH-122`).
                return self.ask_write(job.format, walk.ir);
            }
            ExportStage::Printing(print) => {
                match pace {
                    Pace::Slice => self.print_slice(print),
                    Pace::Whole => {
                        let room = print.total();
                        self.print_pages(print, room);
                    }
                }
                if print.printed() < print.total() {
                    let progress =
                        ConversionProgress::new(Direction::Export, print.printed(), print.total());
                    self.conversion = Some(Job::Export(Box::new(job)));
                    return ProseOutcome::display_only(vec![ProseEvent::Interop(
                        InteropAnswer::Progress(progress),
                    )]);
                }
                let ExportStage::Printing(print) = job.stage else {
                    return ProseOutcome::nothing();
                };
                job.stage = ExportStage::Printed(Box::new(print.finish()));
            }
            ExportStage::Printed(_) => {}
        }
        let family = export_family(&job.format);
        match converter.filter(|c| Some(c.family()) == family) {
            Some(converter) => match job.stage {
                ExportStage::Printed(printed) => {
                    let (document, report) = *printed;
                    Self::write_pdf(document, report, converter)
                }
                ExportStage::Walking(_) | ExportStage::Printing(_) => ProseOutcome::nothing(),
            },
            // A landing before a command holds no module: the file waits
            // for the next pass, which holds it.
            None if flush == Flush::ByCaller => {
                self.conversion = Some(Job::Export(Box::new(job)));
                ProseOutcome::nothing()
            }
            None => ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(ambre_core::StopCause::ConverterUnavailable),
            ))]),
        }
    }
}

impl ProseState {
    /// Commit one slice of an import and answer its delta, as
    /// [`Self::commit_with`] does. The anchors and the root are left to the
    /// last slice: each reads the whole document, and a slice reads only
    /// what it writes.
    /// Take back every write of an import the document refused midway, and
    /// answer the refusal (`PROSE-IO-112`).
    ///
    /// The slices already committed were sent to the peers, so a rollback of
    /// the local state alone would leave them on every other screen. The
    /// inverse of the import's history step commits instead, under the
    /// replay origin, and leaves as a delta like any other. No redo writes
    /// the partial import again.
    ///
    /// A history may hold no step of the import, and then takes nothing
    /// back. A commit it cannot replay cleared it, or a peer's change ended
    /// the step. The import then takes back what `writer` and `annotations`
    /// wrote, from the record the writer kept (`PROSE-IO-113`).
    ///
    /// Either way the history then stands as it did before the import
    /// (`PROSE-IO-120`). The steps before it undo again, and no step of the
    /// import stays.
    fn withdraw_import(
        &mut self,
        flush: Flush,
        writer: &ImportWriter,
        annotations: Option<&ImportAnnotations>,
    ) -> ProseOutcome {
        // The writes the refused slice left pending join the import's step,
        // so the inverse takes them back with the rest.
        self.doc.commit();
        let withdrawn = self.history.withdraw(&self.doc);
        let replay = ambre_core::crdt::CommitOrigin::Recorded.as_str();
        let taken_back = match withdrawn {
            ambre_crdt::history::Replayed::Step => Ok(()),
            ambre_crdt::history::Replayed::Nothing => {
                let taken_back = self.take_back_import(writer, annotations);
                self.doc.set_next_commit_origin(replay);
                self.doc.commit();
                taken_back
            }
        };
        self.doc.set_next_commit_origin(replay);
        self.doc.commit();
        self.history.restore();
        if let Err(e) = taken_back {
            tracing::warn!("Prose: a refused import was not taken back whole: {e:?}");
            // A write of the import stays, so a step before it is no longer
            // the last thing this peer did.
            self.history.clear();
        }
        let mut display = Vec::new();
        if self.may_need_repair() {
            display = self.regrow_root();
            self.doc.set_next_commit_origin(replay);
            self.doc.commit();
        }
        display.push(ProseEvent::Interop(InteropAnswer::failed(
            Direction::Import,
            InteropError::Stopped(ambre_core::StopCause::WriteRefused),
        )));
        match flush {
            Flush::Now => self.commit_slice(display),
            Flush::ByCaller => ProseOutcome::display_only(display),
        }
    }

    fn commit_slice(&mut self, mut display: Vec<ProseEvent>) -> ProseOutcome {
        self.doc.commit();
        let update = self.flush_delta();
        if let Some(delta) = update.clone() {
            display.insert(
                0,
                ProseEvent::DocUpdated {
                    payload: ambre_core::LoroPayload::Delta(delta),
                },
            );
        }
        if let Some(moved) = self.text_changed(super::catchup::ChangeSource::Command) {
            display.insert(usize::from(update.is_some()), moved);
        }
        ProseOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}

/// The test seam of `PROSE-IO-112`: an engine error no input is known to
/// raise, struck at one step of the write. It never compiles outside a test.
#[cfg(test)]
pub(super) mod fault {
    use std::cell::Cell;

    use ambre_bus::interop::Steps;
    use ambre_core::Count;

    thread_local! {
        /// The step the next import on this thread fails at, if any. Each
        /// test runs on a thread of its own, so no test arms another's.
        static AT: Cell<Option<Count<Steps>>> = const { Cell::new(None) };
    }

    /// Fail the write of the operation at `at`, counted from the first.
    pub(in crate::logic) fn arm(at: Count<Steps>) {
        AT.with(|cell| cell.set(Some(at)));
    }

    /// Whether the write of the next operation, after `written` of them,
    /// fails.
    pub(in crate::logic) fn strikes(written: Count<Steps>) -> bool {
        AT.with(|cell| cell.get() == Some(written))
    }

    thread_local! {
        /// Whether the next import on this thread fails once its threads
        /// are written, before its suggestions.
        static AFTER_THREADS: Cell<bool> = const { Cell::new(false) };
    }

    /// Fail the next import once its threads are written.
    pub(in crate::logic) fn arm_after_threads() {
        AFTER_THREADS.with(|cell| cell.set(true));
    }

    /// Whether the import fails once its threads are written.
    pub(in crate::logic) fn strikes_after_threads() -> bool {
        AFTER_THREADS.with(Cell::get)
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::ProseCommand;
    use crate::logic::FileWork;
    use crate::logic::conversion::host_converter;
    use crate::message::{DocChar, ImportFormat, MarkdownFlavor, Page, ProseImport};
    use ambre_bus::interop::InteropRequest;
    use ambre_convert::{Conversion, ConvertError};
    use ambre_core::index::Pos;

    fn document() -> ProseState {
        ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built")
    }

    /// A Markdown file of `paragraphs` paragraphs: many more operations than
    /// one slice takes.
    fn file(paragraphs: usize) -> Vec<u8> {
        (0..paragraphs)
            .map(|i| format!("Paragraph {i} of a long **imported** file.\n\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn import(data: Vec<u8>) -> ProseCommand {
        ProseCommand::Interop(InteropRequest::Import(ProseImport {
            format: ImportFormat::Markdown,
            data: ambre_core::Payload::<ambre_client_core::FileBytes>::new(data),
            now: ambre_core::Instant::<ambre_core::Authored>::new(1_800_000_000_000),
            zone: crate::message::TimeZone::UTC,
        }))
    }

    fn export() -> ProseCommand {
        ProseCommand::Interop(InteropRequest::Export(ExportFormat::Markdown(
            MarkdownFlavor::Gfm,
        )))
    }

    fn progress_of(outcome: &ProseOutcome) -> Option<ConversionProgress> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::Progress(progress)) => Some(*progress),
            _ => None,
        })
    }

    fn completed(outcome: &ProseOutcome) -> bool {
        outcome.display.iter().any(|event| {
            matches!(
                event,
                ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )
        })
    }

    fn file_of(outcome: &ProseOutcome) -> Option<String> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Interop(InteropAnswer::ExportReady { file, .. }) => {
                Some(String::from_utf8_lossy(file.data.as_bytes()).into_owned())
            }
            _ => None,
        })
    }

    fn texts(state: &ProseState) -> Vec<String> {
        state
            .root_blocks()
            .iter()
            .filter_map(|block| state.text_of(block))
            .collect()
    }

    /// A document that holds `paragraphs` imported paragraphs, landed whole.
    fn imported(paragraphs: usize) -> ProseState {
        let mut state = document();
        let converter = host_converter();
        let _ = state.convert_in_place(import(file(paragraphs)), Ok(&converter));
        let _ = state.settle_in_place(None);
        state
    }

    /// `ARCH-122`: an import owes the converter thread its read, and
    /// the document changes nothing until the answer lands. The answer then
    /// starts the import, slice by slice.
    #[test]
    fn an_import_owes_its_read_to_the_converter_thread_and_writes_on_the_answer() {
        let mut state = document();
        let before = state.root_blocks();
        let converter = host_converter();
        let asked = state.handle_conversion(import(file(100)), Ok(&converter));
        assert!(asked.display.is_empty() && asked.update.is_none());
        assert!(!state.has_pending_work(), "no slice runs before the read");
        assert_eq!(state.root_blocks(), before, "the document is untouched");
        let requests = state.take_file_requests();
        let [(ticket, conversion)] = requests.as_slice() else {
            panic!("one read is owed: {}", requests.len());
        };
        assert!(matches!(conversion, Conversion::ReadMarkdown { .. }));
        assert!(state.take_file_requests().is_empty(), "drained once");
        assert_eq!(state.file_work(*ticket), Some(FileWork::Read));
        let first = state
            .handle_file_converted(*ticket, converter.convert(conversion))
            .expect("the ticket is the document's own");
        assert!(progress_of(&first).is_some(), "the answer starts the write");
        assert!(
            state
                .handle_file_converted(*ticket, Err(ConvertError::Absent))
                .is_none()
        );
        let _ = state.settle_in_place(None);
        assert_eq!(state.root_blocks().len(), 100);
    }

    /// `ARCH-122`: a text export walks the document in slices, then
    /// owes the converter thread the write of the form. The file answers
    /// once the thread does, and a module that did not run fails it.
    #[test]
    fn an_export_owes_its_write_to_the_converter_thread() {
        let mut state = imported(10);
        let converter = host_converter();
        let walked = state.handle_conversion(export(), Ok(&converter));
        assert!(file_of(&walked).is_none(), "the walk answers no file");
        assert!(!state.has_pending_work(), "the walk fits one slice");
        let requests = state.take_file_requests();
        let [(ticket, conversion)] = requests.as_slice() else {
            panic!("one write is owed: {}", requests.len());
        };
        assert!(matches!(conversion, Conversion::WriteMarkdown { .. }));
        assert_eq!(state.file_work(*ticket), Some(FileWork::Write));
        let written = state
            .handle_file_converted(*ticket, converter.convert(conversion))
            .expect("the ticket is the document's own");
        let file = file_of(&written).expect("the answer is the file");
        assert!(file.contains("Paragraph 9 of"));
        let _ = state.handle_conversion(export(), Ok(&converter));
        let [(lost, _)] = state.take_file_requests()[..] else {
            panic!("one write is owed");
        };
        let failed = state
            .handle_file_converted(lost, Err(ConvertError::Absent))
            .expect("the ticket is the document's own");
        assert!(matches!(
            failed.display.as_slice(),
            [ProseEvent::Interop(InteropAnswer::Failed(_))]
        ));
    }

    /// `PROSE-IO-021`, `PROSE-IO-048`, `INTEROP-009`: a large import writes
    /// one slice in the command and one per resume. Each slice short of the
    /// end reports further on against one total, and every slice of the
    /// write sends its delta. The last one reports the report instead, and
    /// the document holds what a whole import holds.
    #[test]
    fn a_large_import_lands_one_slice_at_a_time() {
        let mut state = document();
        let converter = host_converter();
        let first = state.convert_in_place(import(file(100)), Ok(&converter));
        let started = progress_of(&first).expect("the first slice reports");
        assert!(!completed(&first));
        assert!(state.has_pending_work());
        let (mut reached, total) = (started.done(), started.total());
        let mut slices = 1;
        let last = loop {
            let out = state.resume_in_place(None);
            slices += 1;
            match progress_of(&out) {
                Some(progress) => {
                    assert_eq!(progress.total(), total, "one total");
                    assert!(progress.done() > reached, "a slice reports further on");
                    assert!(progress.done() < total);
                    reached = progress.done();
                    assert!(!completed(&out));
                }
                None => break out,
            }
        };
        assert!(completed(&last), "the last slice answers the report");
        assert!(!state.has_pending_work());
        assert!(slices > 4, "the import took {slices} slices");
        let texts = texts(&state);
        assert_eq!(texts.len(), 100, "the blank paragraph gave way");
        assert_eq!(
            texts.first().map(String::as_str),
            Some("Paragraph 0 of a long imported file.")
        );
    }

    /// `PROSE-IO-048`: each slice of the write sends the delta it wrote,
    /// so a peer sees the file land block by block.
    #[test]
    fn every_slice_of_the_write_sends_its_delta() {
        let mut state = document();
        let converter = host_converter();
        let _ = state.convert_in_place(import(file(100)), Ok(&converter));
        let mut deltas = 0;
        while state.has_pending_work() {
            deltas += usize::from(state.resume_in_place(None).update.is_some());
        }
        assert!(deltas > 2, "{deltas} deltas");
    }

    /// `PROSE-IO-101`: a command that writes nothing, a scroll or a count,
    /// is served between two slices, and the import still runs.
    #[test]
    fn a_scroll_between_two_slices_is_served_and_the_import_still_runs() {
        let mut state = document();
        let converter = host_converter();
        let _ = state.convert_in_place(import(file(100)), Ok(&converter));
        while progress_of(&state.resume_in_place(None))
            .is_some_and(|p| p.done() < p.total().minus(Count::new(60)))
        {}
        let blocks = state.root_blocks().len();
        let scroll = state.handle_view_command(ProseCommand::ShowPages {
            first: Pos::<Page>::ZERO,
            last: Pos::<Page>::new(2),
        });
        assert!(!completed(&scroll));
        assert!(scroll.update.is_none(), "a scroll writes nothing");
        assert!(state.has_pending_work(), "the import still runs");
        assert_eq!(state.root_blocks().len(), blocks);
        let count = state.handle_view_command(ProseCommand::CountWords);
        assert!(count.update.is_none());
        assert!(state.has_pending_work(), "the import still runs");
    }

    /// `PROSE-IO-100`, `PROSE-IO-051`: an edit mid-import lands the import
    /// first, in the same answer, then the edit. The edit is one undo step
    /// and the whole import is the step before it.
    #[test]
    fn an_edit_lands_the_import_first_and_undo_removes_it_whole() {
        let mut state = document();
        let block = state.root_blocks()[0];
        let _ = state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "Before".to_owned(),
        });
        let converter = host_converter();
        let _ = state.convert_in_place(import(file(100)), Ok(&converter));
        // Into the write: some blocks of the file already landed.
        while state.root_blocks().len() < 20 {
            let _ = state.resume_in_place(None);
        }
        let edit = state.handle_view_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(6),
            text: "!".to_owned(),
        });
        assert!(completed(&edit), "the import landed in the edit's answer");
        assert!(edit.update.is_some());
        assert!(!state.has_pending_work());
        assert_eq!(state.root_blocks().len(), 101);
        assert_eq!(state.text_of(&block).as_deref(), Some("Before!"));
        let _ = state.handle_command(ProseCommand::Undo);
        assert_eq!(
            state.text_of(&block).as_deref(),
            Some("Before"),
            "the edit went alone"
        );
        assert_eq!(state.root_blocks().len(), 101, "the import stays");
        let _ = state.handle_command(ProseCommand::Undo);
        assert_eq!(
            state.root_blocks(),
            vec![block],
            "one undo took the file back"
        );
    }

    /// `PROSE-IO-057`, `PROSE-IO-048`: a large export walks one slice in the
    /// command and one per resume. Each slice short of the end reports
    /// further on against one total, and the last one answers the file the
    /// whole walk writes. The export writes nothing.
    #[test]
    fn a_large_export_walks_one_slice_at_a_time() {
        let mut state = imported(200);
        let converter = host_converter();
        let whole = crate::interop::markdown::write(&state.walk(), MarkdownFlavor::Gfm, &converter)
            .map(|(file, _)| String::from_utf8_lossy(file.as_bytes()).into_owned())
            .expect("the whole walk writes");
        let vv = state.doc.oplog_vv();
        let first = state.convert_in_place(export(), Ok(&converter));
        let started = progress_of(&first).expect("the first slice reports");
        assert_eq!(started.total(), Count::new(200));
        let mut reached = started.done();
        let last = loop {
            let out = state.resume_in_place(Some(&converter));
            match progress_of(&out) {
                Some(progress) => {
                    assert_eq!(progress.total(), started.total(), "one total");
                    assert!(progress.done() > reached, "a slice reports further on");
                    reached = progress.done();
                }
                None => break out,
            }
        };
        assert_eq!(file_of(&last), Some(whole));
        assert!(last.update.is_none());
        assert_eq!(state.doc.oplog_vv(), vv, "the export wrote nothing");
    }

    /// `PROSE-IO-100`: an edit mid-export lands the walk first. The file
    /// holds the document as it stood when the user asked, and the next pass
    /// hands it over through the module.
    #[test]
    fn an_edit_mid_export_leaves_the_file_as_asked() {
        let mut state = imported(200);
        let converter = host_converter();
        let first = state.convert_in_place(export(), Ok(&converter));
        assert!(progress_of(&first).is_some());
        let last = *state.root_blocks().last().expect("a block");
        let edit = state.handle_view_command(ProseCommand::InsertText {
            block_id: last,
            pos: Pos::<DocChar>::ZERO,
            text: "LATER ".to_owned(),
        });
        assert!(edit.update.is_some(), "the edit is served");
        assert!(
            !state.has_pending_work(),
            "the walk landed, and its form left for the converter thread"
        );
        let mut written = ProseOutcome::nothing();
        state.serve_files(&mut written);
        let file = file_of(&written).expect("the thread's answer hands the file over");
        assert!(file.contains("Paragraph 199 of"));
        assert!(!file.contains("LATER"), "the edit came after the walk");
    }

    /// The Logic thread settles a running import before a remote delta or a
    /// close. A settle lands it whole, and answers its delta and its report.
    #[test]
    fn a_settle_lands_the_rest_at_once() {
        let mut state = document();
        let converter = host_converter();
        let _ = state.convert_in_place(import(file(100)), Ok(&converter));
        let settled = state.settle_in_place(None);
        assert!(completed(&settled));
        assert!(settled.update.is_some());
        assert!(
            settled.display.iter().any(|e| matches!(
                e,
                ProseEvent::UndoState {
                    undo: ambre_core::Availability::Enabled,
                    ..
                }
            )),
            "the chrome learns the import can be undone"
        );
        assert_eq!(state.root_blocks().len(), 100);
    }

    /// Whether `outcome` answers the refusal of an import the document
    /// refused midway (`PROSE-IO-112`).
    fn refused_midway(outcome: &ProseOutcome) -> bool {
        outcome.display.iter().any(|event| {
            matches!(
                event,
                ProseEvent::Interop(InteropAnswer::Failed(ambre_bus::interop::InteropFailure {
                    direction: Direction::Import,
                    error: InteropError::Stopped(ambre_core::StopCause::WriteRefused),
                }))
            )
        })
    }

    /// An author who typed one paragraph, and a peer who holds it.
    fn author_and_peer() -> (ProseState, ProseState) {
        let mut author = document();
        let block = author.root_blocks()[0];
        let _ = author.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "Kept".to_owned(),
        });
        let mut peer =
            ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
        peer.import_snapshot(&author.export_snapshot())
            .expect("the seed imports");
        (author, peer)
    }

    /// Hand `peer` the delta `outcome` sends, if any.
    fn forward(outcome: &ProseOutcome, peer: &mut ProseState) {
        if let Some(delta) = &outcome.update {
            let _ = peer.apply_remote(delta);
        }
    }

    /// `PROSE-IO-112`: the engine refuses a write in the third slice, after
    /// two slices reached the peer. The author sees the refusal, and the
    /// author and the peer both read the document as it was. No redo writes
    /// the partial import again, and the step before the import stays.
    #[test]
    fn an_engine_error_midway_takes_the_import_back_on_every_peer() {
        let (mut author, mut peer) = author_and_peer();
        let before = author.shape();
        assert_eq!(peer.shape(), before);
        fault::arm(Count::new(IMPORT_SLICE.get() * 2 + 5));
        let converter = host_converter();
        let mut outcomes = vec![author.convert_in_place(import(file(100)), Ok(&converter))];
        forward(&outcomes[0], &mut peer);
        let mut most = peer.shape().blocks();
        while author.has_pending_work() {
            let out = author.resume_in_place(None);
            forward(&out, &mut peer);
            most = most.max(peer.shape().blocks());
            outcomes.push(out);
        }
        assert!(
            most > before.blocks().plus(Count::new(20)),
            "two slices reached the peer"
        );
        assert!(
            outcomes.iter().any(refused_midway),
            "the author sees the refusal"
        );
        assert!(!outcomes.iter().any(completed));
        assert_eq!(author.shape(), before);
        assert_eq!(peer.shape(), before, "the peer received the undo");
        assert_eq!(
            author.history.can_redo(),
            ambre_core::Availability::Disabled
        );
        let _ = author.handle_command(ProseCommand::Undo);
        let block = author.root_blocks()[0];
        assert_eq!(author.text_of(&block).as_deref(), Some(""));
    }

    /// Run the import of `file(100)` to its end, the fault armed at `at`.
    /// `between` runs after the first slice. Every delta reaches `peer`.
    /// Answers the outcomes and the most blocks the peer held.
    fn run_refused(
        author: &mut ProseState,
        peer: &mut ProseState,
        at: Count<Steps>,
        between: impl FnOnce(&mut ProseState, &mut ProseState),
    ) -> (Vec<ProseOutcome>, Count<crate::logic::fuzz::Level>) {
        fault::arm(at);
        let converter = host_converter();
        let first = author.convert_in_place(import(file(100)), Ok(&converter));
        forward(&first, peer);
        between(author, peer);
        let mut most = peer.shape().blocks();
        let mut outcomes = vec![first];
        while author.has_pending_work() {
            let out = author.resume_in_place(None);
            forward(&out, peer);
            most = most.max(peer.shape().blocks());
            outcomes.push(out);
        }
        (outcomes, most)
    }

    /// `PROSE-IO-113`: a commit the history cannot replay clears it after
    /// the first slice reached the peer. The refusal in the third slice
    /// finds no step to withdraw, and the import still leaves both peers as
    /// they were. No undo or redo brings the partial import back.
    #[test]
    fn a_refusal_after_the_history_was_cleared_takes_the_import_back_on_every_peer() {
        let (mut author, mut peer) = author_and_peer();
        let before = author.shape();
        let (outcomes, most) = run_refused(
            &mut author,
            &mut peer,
            Count::new(IMPORT_SLICE.get() * 2 + 5),
            lose_history,
        );
        assert!(most > before.blocks().plus(Count::new(20)));
        assert!(outcomes.iter().any(refused_midway));
        assert_eq!(author.shape(), before);
        assert_eq!(peer.shape(), before, "the peer received the take-back");
        let _ = author.handle_command(ProseCommand::Undo);
        let _ = author.handle_command(ProseCommand::Redo);
        assert_eq!(author.shape(), before);
    }

    /// A local commit the history cannot replay, a counter. It stands in for
    /// a write of the import that no step replays, and it writes no block.
    fn lose_history(author: &mut ProseState, _: &mut ProseState) {
        author
            .doc
            .get_counter("lost")
            .increment(1.0)
            .expect("a counter counts");
        author.doc.commit();
    }

    /// The peer types "!" after "Kept", and the author receives it.
    fn peer_edit(author: &mut ProseState, peer: &mut ProseState) {
        let block = peer.root_blocks()[0];
        let edit = peer.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(4),
            text: "!".to_owned(),
        });
        forward(&edit, author);
    }

    /// `PROSE-IO-120`: a refused import leaves the history as it stood
    /// before the import, whichever way the refusal finds the history. The
    /// typing before the import undoes on both peers and redoes, and no step
    /// of the import stays.
    #[test]
    fn a_refused_import_leaves_the_history_as_it_stood() {
        type Between = fn(&mut ProseState, &mut ProseState);
        let cases: [(&str, Between, &str); 3] = [
            ("the import's step held", |_, _| {}, ""),
            ("the history lost midway", lose_history, ""),
            ("a peer's edit ended the step", peer_edit, "!"),
        ];
        for (case, between, undone) in cases {
            let (mut author, mut peer) = author_and_peer();
            let depth = author.history.depth();
            let (outcomes, most) = run_refused(
                &mut author,
                &mut peer,
                Count::new(IMPORT_SLICE.get() * 2 + 5),
                between,
            );
            assert!(most.get() > 20, "{case}: slices reached the peer");
            assert!(outcomes.iter().any(refused_midway), "{case}");
            assert_eq!(author.history.depth(), depth, "{case}");
            assert_eq!(
                author.history.can_redo(),
                ambre_core::Availability::Disabled,
                "{case}"
            );
            let undo = author.handle_command(ProseCommand::Undo);
            forward(&undo, &mut peer);
            let roots = author.root_blocks();
            assert_eq!(roots.len(), 1, "{case}: no block of the import stayed");
            assert_eq!(author.text_of(&roots[0]).as_deref(), Some(undone), "{case}");
            assert_eq!(author.shape(), peer.shape(), "{case}: the peer undid too");
            let _ = author.handle_command(ProseCommand::Undo);
            assert_eq!(author.text_of(&roots[0]).as_deref(), Some(undone), "{case}");
            let _ = author.handle_command(ProseCommand::Redo);
            let redone = format!("Kept{undone}");
            assert_eq!(
                author.text_of(&roots[0]).as_deref(),
                Some(redone.as_str()),
                "{case}"
            );
        }
    }

    /// `PROSE-IO-120`: the redo side stands too. The typing undone before
    /// the import redoes after the refusal.
    #[test]
    fn a_refused_import_keeps_the_redo_side() {
        let (mut author, mut peer) = author_and_peer();
        let undo = author.handle_command(ProseCommand::Undo);
        forward(&undo, &mut peer);
        let (outcomes, _) = run_refused(
            &mut author,
            &mut peer,
            Count::new(IMPORT_SLICE.get() * 2 + 5),
            lose_history,
        );
        assert!(outcomes.iter().any(refused_midway));
        let roots = author.root_blocks();
        assert_eq!(roots.len(), 1, "no block of the import stayed");
        assert_eq!(author.text_of(&roots[0]).as_deref(), Some(""));
        let _ = author.handle_command(ProseCommand::Redo);
        assert_eq!(author.text_of(&roots[0]).as_deref(), Some("Kept"));
    }

    /// `PROSE-IO-113`: a peer's edit lands between two slices. The refusal
    /// still leaves both peers as they were, the peer's edit kept.
    #[test]
    fn a_refusal_after_a_peer_edit_ended_the_step_keeps_only_the_edit() {
        let (mut author, mut peer) = author_and_peer();
        let (outcomes, _) = run_refused(
            &mut author,
            &mut peer,
            Count::new(IMPORT_SLICE.get() * 2 + 5),
            peer_edit,
        );
        assert!(outcomes.iter().any(refused_midway));
        assert_eq!(author.shape(), peer.shape());
        let roots = author.root_blocks();
        assert_eq!(roots.len(), 1, "no block of the import stayed");
        assert_eq!(author.text_of(&roots[0]).as_deref(), Some("Kept!"));
    }

    /// `PROSE-IO-112`: a refusal in the first slice takes back the writes it
    /// left pending. No slice committed yet, and the step of the command
    /// before the import stays.
    #[test]
    fn an_engine_error_in_the_first_slice_keeps_the_step_before_it() {
        let (mut author, mut peer) = author_and_peer();
        let before = author.shape();
        let depth = author.history.depth();
        fault::arm(Count::new(5));
        let converter = host_converter();
        let out = author.convert_in_place(import(file(100)), Ok(&converter));
        forward(&out, &mut peer);
        assert!(refused_midway(&out));
        assert!(!author.has_pending_work());
        assert_eq!(author.shape(), before);
        assert_eq!(peer.shape(), before);
        assert_eq!(author.history.depth(), depth);
    }

    /// `PROSE-IO-112`: an edit lands the rest of the import first, and the
    /// document refuses it there. The edit's answer carries
    /// the refusal, and the edit lands on the document as it was.
    #[test]
    fn an_import_refused_while_an_edit_lands_it_leaves_the_edit_alone() {
        let (mut author, mut peer) = author_and_peer();
        let block = author.root_blocks()[0];
        fault::arm(Count::new(IMPORT_SLICE.get() + 3));
        let converter = host_converter();
        let first = author.convert_in_place(import(file(100)), Ok(&converter));
        forward(&first, &mut peer);
        let edit = author.handle_view_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(4),
            text: "!".to_owned(),
        });
        forward(&edit, &mut peer);
        assert!(refused_midway(&edit));
        assert!(!author.has_pending_work());
        assert_eq!(author.shape(), peer.shape());
        let roots = author.root_blocks();
        assert_eq!(roots.len(), 1, "no block of the import stayed");
        assert_eq!(author.text_of(&roots[0]).as_deref(), Some("Kept!"));
    }
}
