// ambre-logic/src/converting.rs
// ============================================================================
// The converter slots: one module per format family, each loaded on the first
// command of its family (`ambre-spec-architecture.md section 2`, the converter
// modules).
//
// The thread cannot fetch, so it asks the Sync worker once per family. The
// commands of that family wait here until the checked bytes land. A load that
// fails answers every waiting command of its family with the reason. The next
// command asks again: a failed fetch is often a network that came back. Each
// family waits on its own: a slow PDF module never holds a CSV import.
// ============================================================================

use super::*;
use crate::converter_thread::{Job, JobFailure, JobOwner, failure_of};
use ambre_bus::interop::{InteropAnswer, InteropError, InteropRequest};
use ambre_convert::{ConvertError, Converter};
use ambre_core::{ConverterFamily, ConverterRefusal, Count, StopCause};
use ambre_prose::logic::ProseState;
use ambre_toile::message::{ToileCommand, ToileEvent, ToileExport};
use ambre_trame::message::{TrameCommand, TrameEvent};
use std::collections::BTreeMap;

/// How many commands may wait for one family's module at once. A user may
/// click an export a few times while the module loads. Past this bound, the
/// click is told to wait, rather than held without bound (`ARCH-081`).
pub(crate) const WAITING_LIMIT: Count<WaitingConversions> = Count::new(16);

/// The commands that wait for a converter module.
pub(crate) enum WaitingConversions {}

impl ambre_core::NarrowCount for WaitingConversions {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("WaitingConversionCount");
}

/// The memory a module instance of the Logic thread may span after a pass
/// before the thread renews it (`ARCH-125`, `ambre-spec-budgets.md section
/// 6`). A WebAssembly memory never shrinks, so the instance that read a large
/// file would hold its peak for the session. A module starts at about
/// 1 MiB, so a small paste or a CSV keeps its instance and its memory.
pub(crate) const LOGIC_RESET_FOOTPRINT: Count<ambre_core::Octets> = Count::new(16 * 1024 * 1024);

/// Whether a Logic-thread instance whose memory spans `footprint` is renewed
/// (`ARCH-125`). The host runs the engine in its own heap, and reports no
/// footprint.
pub(crate) fn renews(footprint: Option<Count<ambre_core::Octets>>) -> bool {
    footprint.is_some_and(|bytes| bytes > LOGIC_RESET_FOOTPRINT)
}

/// Where each family's module stands for this session. A family absent from
/// the map is not loaded and not asked for.
#[derive(Default)]
pub(crate) struct Converters {
    slots: BTreeMap<ConverterFamily, ConverterSlot>,
}

impl Converters {
    /// The loaded module of `family`, if it is loaded.
    pub(crate) fn ready(&self, family: ConverterFamily) -> Option<&Converter> {
        match self.slots.get(&family) {
            Some(ConverterSlot::Ready { converter, .. }) => Some(converter),
            _ => None,
        }
    }

    /// Renew each loaded instance whose memory grew past
    /// [`LOGIC_RESET_FOOTPRINT`] (`ARCH-125`). Called once a pass, after
    /// every conversion of the pass. A module keeps no state between two
    /// calls, so a slice of a running export loses nothing. An instance
    /// the browser refuses to renew is compiled again from its bytes. One
    /// that still fails keeps its old instance, which works.
    pub(crate) fn give_back(&mut self) {
        for (family, slot) in &mut self.slots {
            let ConverterSlot::Ready { converter, module } = slot else {
                continue;
            };
            if !renews(converter.footprint()) {
                continue;
            }
            match converter
                .renew()
                .or_else(|_| Converter::load(*family, module))
            {
                Ok(fresh) => *converter = fresh,
                Err(error) => {
                    tracing::warn!("Logic: the {family} converter kept its memory: {error}");
                }
            }
        }
    }

    /// The span of `family`'s loaded instance now. The browser's tests alone.
    #[cfg(all(test, target_arch = "wasm32"))]
    fn footprint(&self, family: ConverterFamily) -> Option<Count<ambre_core::Octets>> {
        self.ready(family).and_then(Converter::footprint)
    }
}

/// Where one family's module stands.
pub(crate) enum ConverterSlot {
    /// Asked of the Sync worker. The commands and the jobs of the converter
    /// thread wait in arrival order.
    Asked {
        commands: Vec<WaitingConversion>,
        jobs: Vec<Job>,
    },
    /// Loaded: every later conversion of the family runs at once. The bytes
    /// stay, so the converter thread can load its own instance of them
    /// (`ARCH-098`).
    Ready {
        converter: Converter,
        module: ambre_core::Payload<ConverterModule>,
    },
}

/// A command that waits for a module, and the document it targets.
pub(crate) struct WaitingConversion {
    document_id: Id<Document>,
    command: AppCommand,
}

/// The family whose module `command` reads or writes a file through, if any.
pub(crate) fn needs_converter(command: &AppCommand) -> Option<ConverterFamily> {
    match command {
        AppCommand::Toile(ToileCommand::Interop(InteropRequest::Export(ToileExport::Pdf))) => {
            Some(ConverterFamily::Pdf)
        }
        AppCommand::Toile(cmd) => ambre_toile::ToileState::conversion_family(cmd),
        AppCommand::Trame(cmd) => cmd.converter_family(),
        AppCommand::Prose(cmd) => ProseState::conversion_family(cmd),
    }
}

impl LogicState {
    /// Serve a command that needs the module of `family`: at once when it is
    /// loaded, otherwise after the Sync worker brought it. The first such
    /// command of the session asks for it (`ARCH-081`).
    pub(crate) fn convert_for(
        &mut self,
        family: ConverterFamily,
        document_id: Id<Document>,
        command: AppCommand,
    ) {
        match self.converters.slots.get_mut(&family) {
            Some(ConverterSlot::Ready { .. }) => self.run_conversion(family, document_id, command),
            Some(ConverterSlot::Asked { commands, .. })
                if Count::of(commands.len()) < WAITING_LIMIT =>
            {
                commands.push(WaitingConversion {
                    document_id,
                    command,
                });
            }
            Some(ConverterSlot::Asked { .. }) => {
                self.answer_unconverted(document_id, command, StopCause::ConverterBusy);
            }
            None => {
                self.converters.slots.insert(
                    family,
                    ConverterSlot::Asked {
                        commands: vec![WaitingConversion {
                            document_id,
                            command,
                        }],
                        jobs: Vec::new(),
                    },
                );
                self.send_to_sync(SyncCommand::FetchConverter { family });
            }
        }
    }

    /// The Sync worker brought the module of `family`, checked against the
    /// build's digest (`ARCH-080`). Load it, then run every waiting command of
    /// the family in order.
    pub(crate) fn converter_fetched(
        &mut self,
        family: ConverterFamily,
        module: ambre_core::Payload<ConverterModule>,
    ) {
        match Converter::load(family, &module) {
            Ok(converter) => {
                let (commands, jobs) = self.take_waiting(family);
                self.converters
                    .slots
                    .insert(family, ConverterSlot::Ready { converter, module });
                for w in commands {
                    self.run_conversion(family, w.document_id, w.command);
                }
                for job in jobs {
                    self.offload(job);
                }
            }
            Err(error) => {
                tracing::warn!("Logic: the {family} converter did not load: {error}");
                self.converter_refused(family, ConverterRefusal::NotLoaded);
            }
        }
    }

    /// The module of `family` did not arrive, or did not load: every waiting
    /// command of the family learns why, and the next one asks again.
    pub(crate) fn converter_refused(&mut self, family: ConverterFamily, refusal: ConverterRefusal) {
        let (commands, jobs) = self.take_waiting(family);
        tracing::warn!("Logic: the {family} converter could not be loaded: {refusal}");
        for w in commands {
            self.answer_unconverted(
                w.document_id,
                w.command,
                StopCause::ConverterRefused(refusal),
            );
        }
        for job in jobs {
            self.answer_job(job.ticket, job.owner, Err(ConvertError::Absent));
        }
    }

    /// The commands and the jobs waiting on `family`, which leave its slot.
    /// A slot that was asked is left absent; a loaded slot keeps its module.
    fn take_waiting(&mut self, family: ConverterFamily) -> (Vec<WaitingConversion>, Vec<Job>) {
        match self.converters.slots.remove(&family) {
            Some(ConverterSlot::Asked { commands, jobs }) => (commands, jobs),
            Some(ready @ ConverterSlot::Ready { .. }) => {
                self.converters.slots.insert(family, ready);
                (Vec::new(), Vec::new())
            }
            None => (Vec::new(), Vec::new()),
        }
    }

    // -- The converter thread (`ARCH-097`, `ARCH-098`) ---------------------

    /// Hand `job` to the converter thread, and never wait for its answer.
    /// The thread is handed its family's module first. A module not loaded
    /// yet is asked of the Sync worker once, as a command's is (`ARCH-081`),
    /// and the job waits in its slot. A job never counts against the waiting
    /// limit of the commands: a page of pictures asks one job per picture.
    /// The first job spawns the converter thread, so its worker starts while
    /// the module downloads (`ARCH-112`).
    pub(crate) fn offload(&mut self, job: Job) {
        self.offload.start();
        let family = job.conversion.family();
        if !self.offload.holds(family) {
            match self.converters.slots.get_mut(&family) {
                Some(ConverterSlot::Ready { module, .. }) => {
                    if let Err(failure) = self.offload.load(family, module.clone()) {
                        return self.answer_job(job.ticket, job.owner, Err(failure));
                    }
                }
                Some(ConverterSlot::Asked { jobs, .. }) => return jobs.push(job),
                None => {
                    self.converters.slots.insert(
                        family,
                        ConverterSlot::Asked {
                            commands: Vec::new(),
                            jobs: vec![job],
                        },
                    );
                    return self.send_to_sync(SyncCommand::FetchConverter { family });
                }
            }
        }
        if let Err((ticket, owner, failure)) = self.offload.send(job) {
            self.answer_job(ticket, owner, Err(failure));
        }
    }

    /// Hand each answer of the converter thread to the owner that asked, then
    /// send the jobs the answers made room for. Called once a pass. A thread
    /// past its answer deadline fails its jobs here, and a job released to
    /// the thread spawned after it loads its module first (`ARCH-108`).
    pub(crate) fn take_converted(&mut self) {
        loop {
            let answers = self.offload.take_answers(crate::converter_thread::now());
            let released = self.offload.release();
            if answers.is_empty() && released.is_empty() {
                return;
            }
            for (ticket, owner, answer) in answers {
                self.answer_job(ticket, owner, answer);
            }
            for job in released {
                self.offload(job);
            }
        }
    }

    /// Hand one job's answer to its owner: a Prose picture or file, a Toile
    /// insert or a Murmure preview. An owner closed meanwhile drops it.
    pub(crate) fn answer_job(
        &mut self,
        ticket: ambre_core::Seq<ambre_bus::Request>,
        owner: JobOwner,
        answer: Result<ambre_convert::Converted, ConvertError>,
    ) {
        match owner {
            JobOwner::ToilePng(document_id) => {
                return self
                    .answer_toile_png(document_id, answer.map_err(|error| failure_of(&error)));
            }
            JobOwner::ProseFile(document_id) => {
                return self.answer_prose_file(document_id, ticket, answer);
            }
            JobOwner::DeckPdf {
                document_id,
                report,
            } => {
                return self.answer_deck_pdf(document_id, *report, answer);
            }
            _ => {}
        }
        if let JobOwner::TrameFile(document_id) = owner {
            return self.answer_trame_file(document_id, ticket, answer);
        }
        if let JobOwner::ToileFile(document_id) = owner {
            return self.answer_toile_file(document_id, ticket, answer);
        }
        // A still answer for the owner that reads pixels alone; Prose and
        // Murmure read the frames of an animation too.
        let answer = answer.map_err(|error| failure_of(&error));
        let pixels = |answer: Result<ambre_convert::Converted, JobFailure>| {
            answer.and_then(|converted| match converted {
                ambre_convert::Converted::Pixels(pixels) => Ok(pixels),
                _ => {
                    tracing::debug!("{}", ConvertError::Malformed);
                    Err(JobFailure::Refused)
                }
            })
        };
        match owner {
            JobOwner::Murmure {
                channel_id,
                media_ref,
            } => self.previewed(channel_id, media_ref, answer),
            JobOwner::Prose(document_id) => {
                let Some(DocState::Prose(state)) =
                    self.docs.get_mut(&document_id).map(|d| &mut d.state)
                else {
                    return;
                };
                // A spelling answer goes to the view alone. The document is
                // answered again this pass, which sends the next check
                // (`4-Prose/features/ambre-spec-prose-spell-check.md`).
                if let Some(outcome) = state.handle_spelled(ticket, &answer) {
                    return self.dispatch_prose_media(document_id, outcome);
                }
                let decoded = answer.map_err(|failure| match failure {
                    JobFailure::Unavailable => ambre_prose::message::MediaFailure::Unavailable,
                    JobFailure::Refused => ambre_prose::message::MediaFailure::Undecodable,
                });
                if let Some(outcome) = state.handle_decoded(ticket, decoded) {
                    self.dispatch_prose_media(document_id, outcome);
                }
            }
            // Served above, before the answer is read as pixels.
            JobOwner::ProseFile(_)
            | JobOwner::TrameFile(_)
            | JobOwner::DeckPdf { .. }
            | JobOwner::ToileFile(_) => {}
            JobOwner::Toile(document_id) => {
                let Some(DocState::Toile(state)) =
                    self.docs.get_mut(&document_id).map(|d| &mut d.state)
                else {
                    return;
                };
                let outcome = state.handle_decoded(ticket, pixels(answer));
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    outcome.requests,
                );
            }
            // Answered above, as a file.
            JobOwner::ToilePng(_) => {}
        }
    }

    /// Land the answer of a Prose file read or write. A read writes the
    /// document, so a replay still being imported lands first, as before any
    /// import (`SYNC-131`), and the view is owed a layout. A document closed
    /// meanwhile drops it.
    fn answer_prose_file(
        &mut self,
        document_id: Id<Document>,
        ticket: ambre_core::Seq<ambre_bus::Request>,
        answer: Result<ambre_convert::Converted, ConvertError>,
    ) {
        let reads = match self.docs.get(&document_id).map(|d| &d.state) {
            Some(DocState::Prose(state)) => match state.file_work(ticket) {
                Some(work) => work == ambre_convert::FileWork::Read,
                None => return,
            },
            _ => return,
        };
        if reads {
            self.settle_import(document_id);
        }
        let Some(DocState::Prose(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        let Some(outcome) = state.handle_file_converted(ticket, answer) else {
            return;
        };
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Prose).collect(),
            outcome.update,
            outcome.ephemeral,
            Vec::new(),
        );
        if reads {
            self.owe_layout(document_id);
        }
    }

    /// Land the answer of a Trame file read or write (`ARCH-123`). A read
    /// writes the workbook, so a replay or a PDF export still running lands
    /// first, as before any import. A document closed meanwhile drops it.
    fn answer_trame_file(
        &mut self,
        document_id: Id<Document>,
        ticket: ambre_core::Seq<ambre_bus::Request>,
        answer: Result<ambre_convert::Converted, ConvertError>,
    ) {
        let reads = match self.docs.get(&document_id).map(|d| &d.state) {
            Some(DocState::Trame(state)) => match state.file_work(ticket) {
                Some(work) => work == ambre_convert::FileWork::Read,
                None => return,
            },
            _ => return,
        };
        if reads {
            self.settle_import(document_id);
        }
        let Some(DocState::Trame(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        let Some(outcome) = state.handle_file_converted(ticket, answer) else {
            return;
        };
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Trame).collect(),
            outcome.update,
            outcome.ephemeral,
            Vec::new(),
        );
    }

    /// Hand over the file the PDF module wrote for a deck's export, or why
    /// there is none (`ARCH-124`). A board closed meanwhile drops it.
    fn answer_deck_pdf(
        &mut self,
        document_id: Id<Document>,
        report: ambre_toile::message::ToileReport,
        answer: Result<ambre_convert::Converted, ConvertError>,
    ) {
        if !matches!(
            self.docs.get(&document_id).map(|d| &d.state),
            Some(DocState::Toile(_))
        ) {
            return;
        }
        let file = match answer {
            Ok(ambre_convert::Converted::File(data)) => Ok(data),
            Ok(_) => Err(StopCause::ConverterAnswered),
            Err(error) => {
                tracing::warn!("Logic: the PDF export stopped: {error}");
                Err(error.stop_cause())
            }
        };
        self.dispatch_outcome(
            document_id,
            vec![AppEvent::Toile(data::deck_file_answer(file, report))],
            None,
            None,
            Vec::new(),
        );
    }

    /// Land the answer of a Toile file read or write (`ARCH-129`). A read
    /// runs its import command in the frame of any command, so a running
    /// import lands whole first. A document closed meanwhile drops it.
    fn answer_toile_file(
        &mut self,
        document_id: Id<Document>,
        ticket: ambre_core::Seq<ambre_bus::Request>,
        answer: Result<ambre_convert::Converted, ConvertError>,
    ) {
        let Some(DocState::Toile(state)) = self.docs.get_mut(&document_id).map(|d| &mut d.state)
        else {
            return;
        };
        let Some(outcome) = state.handle_file_converted(ticket, answer) else {
            return;
        };
        self.dispatch_outcome(
            document_id,
            outcome.display.into_iter().map(AppEvent::Toile).collect(),
            outcome.update,
            outcome.ephemeral,
            outcome.requests,
        );
    }

    /// Run one conversion on the loaded module of `family`. The document may
    /// have closed while the module loaded: its command is then dropped, as
    /// any command of a closed document is.
    fn run_conversion(
        &mut self,
        family: ConverterFamily,
        document_id: Id<Document>,
        command: AppCommand,
    ) {
        if let AppCommand::Toile(ToileCommand::Interop(InteropRequest::Export(ToileExport::Pdf))) =
            command
        {
            return self.export_toile_pdf(document_id);
        }
        if let AppCommand::Trame(TrameCommand::Interop(InteropRequest::Export(
            ambre_trame::message::TrameExport::Pdf(options),
        ))) = command
        {
            return self.start_trame_print(document_id, options);
        }
        if matches!(command, AppCommand::Trame(_) | AppCommand::Prose(_)) {
            self.settle_import(document_id);
        }
        let Self {
            docs, converters, ..
        } = self;
        let (Some(state), Some(converter)) = (
            docs.get_mut(&document_id).map(|d| &mut d.state),
            converters.ready(family),
        ) else {
            return;
        };
        // Each arm hands on what the same command hands on outside a
        // conversion: Toile its service-plane requests, Prose a layout owed.
        match (state, command) {
            (DocState::Trame(state), AppCommand::Trame(cmd)) => {
                let outcome = state.handle_conversion(cmd);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    Vec::new(),
                );
                self.offload_trame_files(document_id);
            }
            (DocState::Toile(state), AppCommand::Toile(cmd)) => {
                let outcome = state.handle_conversion(cmd);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    outcome.requests,
                );
                self.offload_toile_files(document_id);
            }
            (DocState::Prose(state), AppCommand::Prose(cmd)) => {
                let outcome = state.handle_conversion(cmd, Ok(converter));
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Prose).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    Vec::new(),
                );
                self.offload_prose_files(document_id);
                self.owe_layout(document_id);
            }
            _ => {}
        }
    }

    /// Answer a command the converter could not serve, with the cause its
    /// application words. Nothing is silent (`ARCH-081`). A Prose or a Toile
    /// command learns the cause through its own state: a Prose paste then
    /// lands its plain text.
    fn answer_unconverted(
        &mut self,
        document_id: Id<Document>,
        command: AppCommand,
        cause: StopCause,
    ) {
        let stopped = || InteropError::Stopped(cause);
        let events = match (
            self.docs.get_mut(&document_id).map(|d| &mut d.state),
            command,
        ) {
            (None, _) => return,
            (Some(DocState::Prose(state)), AppCommand::Prose(cmd)) => {
                let outcome = state.handle_conversion(cmd, Err(cause));
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Prose).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    Vec::new(),
                );
                return self.owe_layout(document_id);
            }
            (Some(_), AppCommand::Toile(ToileCommand::Interop(request))) => (
                vec![AppEvent::Toile(ToileEvent::Interop(InteropAnswer::failed(
                    request.direction(),
                    stopped(),
                )))],
                None,
                None,
            ),
            (Some(_), AppCommand::Trame(TrameCommand::Interop(request))) => (
                vec![AppEvent::Trame(TrameEvent::Interop(InteropAnswer::failed(
                    request.direction(),
                    stopped(),
                )))],
                None,
                None,
            ),
            (Some(_), _) => return,
        };
        let (display, update, ephemeral) = events;
        self.dispatch_outcome(document_id, display, update, ephemeral, Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ARCH-125`: the decision. An instance past the bound is renewed, one
    /// at the bound keeps its memory, and the host, which reports no
    /// footprint, never renews.
    #[test]
    fn a_logic_instance_past_the_bound_is_renewed_and_the_host_never_is() {
        assert!(!renews(None));
        assert!(!renews(Some(LOGIC_RESET_FOOTPRINT)));
        assert!(renews(Some(LOGIC_RESET_FOOTPRINT.plus(Count::new(1)))));
    }

    /// `ARCH-125`: on the host the pass renews nothing, and every loaded
    /// module stays ready for the next conversion.
    #[test]
    fn a_pass_on_the_host_keeps_every_loaded_module() {
        let mut converters = Converters::default();
        for family in [ConverterFamily::Markup, ConverterFamily::Pdf] {
            let module = ambre_core::Payload::new(Vec::new());
            let converter = Converter::load(family, &module).expect("the host links the engine");
            converters
                .slots
                .insert(family, ConverterSlot::Ready { converter, module });
        }
        converters.give_back();
        assert!(converters.ready(ConverterFamily::Markup).is_some());
        assert!(converters.ready(ConverterFamily::Pdf).is_some());
    }
}

#[cfg(test)]
#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;
    use crate::converter_thread::wasm_tests::GROWS;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// `ARCH-125`: a Logic-thread instance that grew past the bound in a
    /// conversion comes back at its first span after the pass, and still
    /// converts. An instance under the bound keeps its memory.
    #[wasm_bindgen_test]
    fn a_logic_instance_grown_past_the_bound_comes_back_at_its_first_span() {
        let module = ambre_core::Payload::new(GROWS.to_vec());
        let converter =
            Converter::load(ConverterFamily::Tables, &module).expect("the module loads");
        let mut converters = Converters::default();
        converters.slots.insert(
            ConverterFamily::Tables,
            ConverterSlot::Ready { converter, module },
        );
        let page = Count::<ambre_core::Octets>::new(64 * 1024);
        converters.give_back();
        assert_eq!(converters.footprint(ConverterFamily::Tables), Some(page));
        let csv = ambre_convert::Conversion::WriteCsv {
            records: Vec::new(),
            delimiter: ambre_convert::CsvDelimiter::COMMA,
        };
        for _ in 0..2 {
            let converter = converters
                .ready(ConverterFamily::Tables)
                .expect("the module stays loaded");
            // The module answers no bytes, so the conversion is malformed:
            // what counts is that its `run` grew the memory.
            let _ = converter.convert(&csv);
            assert!(renews(converter.footprint()), "the run grew the memory");
            converters.give_back();
            assert_eq!(converters.footprint(ConverterFamily::Tables), Some(page));
        }
    }
}
