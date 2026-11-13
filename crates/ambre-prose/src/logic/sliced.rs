//! A conversion applied one slice per pass of the Logic thread
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! A converter module reads a file, or writes one, in one call. What costs
//! the time is the part that sees the document: an import writes each block,
//! and an export walks each one. So that part runs in slices, and the Logic
//! thread serves the other documents and the view between two of them
//! (`PROSE-IO-048`, `ARCH-030`).
//!
//! An import first rehearses its whole list on a scratch document, so a
//! refusal writes nothing (`PROSE-IO-011`). It then writes the list into the
//! document, one commit per slice, in one step of the history
//! (`PROSE-IO-051`). An export walks the top-level blocks, then hands the form
//! to the module. Each slice short of the end reports its progress
//! (`PROSE-IO-021`, `PROSE-IO-057`).
//!
//! A command that writes the document lands the running conversion first
//! (`PROSE-IO-100`). An edit then never joins the import's undo step, and an
//! export holds the document as it stood when the user asked for it.

use ambre_bus::interop::{ConversionProgress, Direction, InteropAnswer, InteropError, Steps};
use ambre_convert::Converter;
use ambre_core::{ConverterFamily, Count};

use super::ProseState;
use super::interop::{ImportAnnotations, ImportOp, ImportWriter, Walk};
use super::print::PrintJob;
use crate::doc;
use crate::events::{ProseEvent, ProseOutcome};
use crate::interop::ProseIr;
use crate::message::{ConversionReport, ExportFormat};
use ambre_draw::list::PrintDocument;

/// The steps one pass of a running import takes: each operation counts once
/// in the rehearsal and once in the document. The `import_slice` operation of
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

/// An import between two slices: the list, what the reader lost, and the
/// annotations the last slice writes.
pub(super) struct ImportJob {
    ops: Vec<ImportOp>,
    report: ConversionReport,
    annotations: Option<ImportAnnotations>,
    stage: ImportStage,
}

/// Where an import stands.
enum ImportStage {
    /// The list is written into a scratch document, which nothing reads.
    Rehearsal {
        scratch: Box<ProseState>,
        writer: ImportWriter,
    },
    /// The rehearsal passed: the list is written into the document.
    Write(ImportWriter),
}

impl ImportJob {
    /// Every step of the import: each operation rehearsed, then written.
    fn total(&self) -> Count<Steps> {
        let ops = Count::<Steps>::of(self.ops.len());
        ops.plus(ops)
    }

    fn done(&self) -> Count<Steps> {
        match &self.stage {
            ImportStage::Rehearsal { writer, .. } => writer.written(),
            ImportStage::Write(writer) => Count::<Steps>::of(self.ops.len()).plus(writer.written()),
        }
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
    /// The walk ended: the module writes the form on the next call that holds
    /// it.
    Walked(Box<ProseIr>),
    /// Pages of a PDF are left to draw.
    Printing(Box<PrintJob>),
    /// Every page is drawn: the PDF module writes them on the next call
    /// that holds it.
    Printed(Box<(PrintDocument, ConversionReport)>),
}

/// The family whose module writes `format`, if a module does.
pub(super) const fn export_family(format: &ExportFormat) -> Option<ConverterFamily> {
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
        self.conversion.is_some()
    }

    /// The family whose module the running conversion still needs: an export
    /// hands its form to it once the walk ends. An import needs none.
    #[must_use]
    pub fn pending_family(&self) -> Option<ConverterFamily> {
        match &self.conversion {
            Some(Job::Export(job)) => export_family(&job.format),
            Some(Job::Import(_)) | None => None,
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
    /// slice. A list of no operation completes at once.
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
        let Some(scratch) = Self::new() else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::NoScratchDocument,
            ));
        };
        let writer = ImportWriter::start(&scratch);
        self.conversion = Some(Job::Import(Box::new(ImportJob {
            ops,
            report,
            annotations,
            stage: ImportStage::Rehearsal {
                scratch: Box::new(scratch),
                writer,
            },
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
        let stage = match &format {
            ExportFormat::Pdf(options) => match self.print_start(options) {
                Ok(job) => ExportStage::Printing(Box::new(job)),
                Err(e) => {
                    return ProseOutcome::display_only(vec![ProseEvent::Interop(
                        InteropAnswer::failed(Direction::Export, e),
                    )]);
                }
            },
            _ => ExportStage::Walking(Box::new(self.walk_start())),
        };
        self.conversion = Some(Job::Export(Box::new(ExportJob { format, stage })));
        self.run_conversion(Pace::Slice, Flush::Now, Some(converter))
    }

    fn run_conversion(
        &mut self,
        pace: Pace,
        flush: Flush,
        converter: Option<&Converter>,
    ) -> ProseOutcome {
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
        let mut room = match pace {
            Pace::Slice => IMPORT_SLICE,
            Pace::Whole => job.total(),
        };
        let mut wrote = false;
        loop {
            match &mut job.stage {
                ImportStage::Rehearsal { scratch, writer } => {
                    let taken = match scratch.write_ops(writer, &job.ops, room) {
                        Ok(taken) => taken,
                        Err(refused) => return refused,
                    };
                    room = room.minus(taken);
                    if writer.written() < Count::<Steps>::of(job.ops.len()) {
                        break;
                    }
                    // The rehearsal passed. The writes that follow are one
                    // step of the history, however many commits they take.
                    job.stage = ImportStage::Write(ImportWriter::start(self));
                    self.history.seal();
                }
                ImportStage::Write(writer) => {
                    let taken = match self.write_ops(writer, &job.ops, room) {
                        Ok(taken) => taken,
                        Err(refused) => {
                            self.history.close();
                            return refused;
                        }
                    };
                    wrote |= taken > Count::ZERO;
                    break;
                }
            }
            if room == Count::ZERO {
                break;
            }
        }
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
            ops,
            report,
            annotations,
            stage: ImportStage::Write(writer),
        } = job
        else {
            return ProseOutcome::nothing();
        };
        let minted = match self.finish_write(writer) {
            Ok(minted) => minted,
            Err(refused) => {
                self.history.close();
                return refused;
            }
        };
        let mut display = vec![ProseEvent::Interop(InteropAnswer::ImportCompleted {
            report,
        })];
        if let Some(annotations) = annotations.as_ref().filter(|a| !a.is_empty()) {
            if let Err(e) = self.write_annotations(annotations, &ops, &minted) {
                self.history.close();
                return Self::err_write(e);
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
        self.history.close();
        outcome
    }

    /// Take `pace` of a running export. A slice short of the end of the walk
    /// reports its progress. The module writes the form once the walk ended
    /// and a call holds the module. An export writes nothing to the
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
                job.stage = ExportStage::Walked(Box::new(walk.ir));
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
            ExportStage::Walked(_) | ExportStage::Printed(_) => {}
        }
        let family = export_family(&job.format);
        match converter.filter(|c| Some(c.family()) == family) {
            Some(converter) => match job.stage {
                ExportStage::Walked(ir) => self.write_export(job.format, *ir, converter),
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
                InteropError::Stopped("the converter is not loaded".to_owned()),
            ))]),
        }
    }
}

impl ProseState {
    /// Commit one slice of an import and answer its delta, as
    /// [`Self::commit_with`] does. The anchors and the root are left to the
    /// last slice: each reads the whole document, and a slice reads only
    /// what it writes.
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

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::ProseCommand;
    use crate::logic::conversion::host_converter;
    use crate::message::{DocChar, ImportFormat, MarkdownFlavor, Page, ProseImport};
    use ambre_bus::interop::InteropRequest;
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
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(data),
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
        let _ = state.handle_conversion(import(file(paragraphs)), Ok(&converter));
        let _ = state.settle_conversion(None);
        state
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
        let first = state.handle_conversion(import(file(100)), Ok(&converter));
        let started = progress_of(&first).expect("the first slice reports");
        assert!(!completed(&first));
        assert!(state.has_pending_work());
        let (mut reached, total) = (started.done(), started.total());
        let mut slices = 1;
        let last = loop {
            let out = state.resume_conversion(None);
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
        let _ = state.handle_conversion(import(file(100)), Ok(&converter));
        let mut deltas = 0;
        while state.has_pending_work() {
            deltas += usize::from(state.resume_conversion(None).update.is_some());
        }
        assert!(deltas > 2, "{deltas} deltas");
    }

    /// `PROSE-IO-101`: a command that writes nothing, a scroll or a count,
    /// is served between two slices, and the import still runs.
    #[test]
    fn a_scroll_between_two_slices_is_served_and_the_import_still_runs() {
        let mut state = document();
        let converter = host_converter();
        let _ = state.handle_conversion(import(file(100)), Ok(&converter));
        while progress_of(&state.resume_conversion(None))
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
        let _ = state.handle_conversion(import(file(100)), Ok(&converter));
        // Into the write: some blocks of the file already landed.
        while state.root_blocks().len() < 20 {
            let _ = state.resume_conversion(None);
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
        let first = state.handle_conversion(export(), Ok(&converter));
        let started = progress_of(&first).expect("the first slice reports");
        assert_eq!(started.total(), Count::new(200));
        let mut reached = started.done();
        let last = loop {
            let out = state.resume_conversion(Some(&converter));
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
        let first = state.handle_conversion(export(), Ok(&converter));
        assert!(progress_of(&first).is_some());
        let last = *state.root_blocks().last().expect("a block");
        let edit = state.handle_view_command(ProseCommand::InsertText {
            block_id: last,
            pos: Pos::<DocChar>::ZERO,
            text: "LATER ".to_owned(),
        });
        assert!(edit.update.is_some(), "the edit is served");
        assert!(state.has_pending_work(), "the file waits for its module");
        let written = state.resume_conversion(Some(&converter));
        let file = file_of(&written).expect("the next pass hands the file over");
        assert!(!state.has_pending_work());
        assert!(file.contains("Paragraph 199 of"));
        assert!(!file.contains("LATER"), "the edit came after the walk");
    }

    /// The Logic thread settles a running import before a remote delta or a
    /// close. A settle lands it whole, and answers its delta and its report.
    #[test]
    fn a_settle_lands_the_rest_at_once() {
        let mut state = document();
        let converter = host_converter();
        let _ = state.handle_conversion(import(file(100)), Ok(&converter));
        let settled = state.settle_conversion(None);
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
}
