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
use crate::converter_thread::{Job, JobFailure, JobOwner};
use ambre_bus::interop::{InteropAnswer, InteropError, InteropRequest};
use ambre_convert::Converter;
use ambre_core::{ConverterFamily, Count};
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

impl ambre_core::CountKind for WaitingConversions {
    type Repr = u32;
    const NAME: &'static str = "WaitingConversionCount";
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
                self.answer_unconverted(
                    document_id,
                    command,
                    "the converter is still loading: try again in a moment",
                );
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
            Err(error) => self.converter_refused(family, &error.to_string()),
        }
    }

    /// The module of `family` did not arrive, or did not load: every waiting
    /// command of the family learns why, and the next one asks again.
    pub(crate) fn converter_refused(&mut self, family: ConverterFamily, reason: &str) {
        let (commands, jobs) = self.take_waiting(family);
        let reason = format!("the {family} converter could not be loaded: {reason}");
        for w in commands {
            self.answer_unconverted(w.document_id, w.command, &reason);
        }
        if !jobs.is_empty() {
            tracing::debug!("{reason}");
        }
        for job in jobs {
            self.answer_job(job.ticket, job.owner, Err(JobFailure::Unavailable));
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
    pub(crate) fn offload(&mut self, job: Job) {
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
    /// send the jobs the answers made room for. Called once a pass.
    pub(crate) fn take_converted(&mut self) {
        loop {
            let answers = self.offload.take_answers();
            let released = self.offload.release();
            if answers.is_empty() && released.is_empty() {
                return;
            }
            for (ticket, owner, answer) in answers {
                self.answer_job(ticket, owner, answer);
            }
            for job in released {
                if let Err((ticket, owner, failure)) = self.offload.send(job) {
                    self.answer_job(ticket, owner, Err(failure));
                }
            }
        }
    }

    /// Hand one job's answer to its owner: a Prose picture, a Toile insert or
    /// a Murmure preview. An owner closed meanwhile drops it.
    pub(crate) fn answer_job(
        &mut self,
        ticket: ambre_core::Seq<ambre_bus::Request>,
        owner: JobOwner,
        answer: Result<ambre_convert::Converted, JobFailure>,
    ) {
        let pixels = answer.and_then(|converted| match converted {
            ambre_convert::Converted::Pixels(pixels) => Ok(pixels),
            _ => {
                tracing::debug!("{}", ambre_convert::ConvertError::Malformed);
                Err(JobFailure::Refused)
            }
        });
        match owner {
            JobOwner::Murmure {
                channel_id,
                media_ref,
            } => self.previewed(channel_id, media_ref, pixels),
            JobOwner::Prose(document_id) => {
                let Some(DocState::Prose(state)) =
                    self.docs.get_mut(&document_id).map(|d| &mut d.state)
                else {
                    return;
                };
                let decoded = pixels.map_err(|failure| match failure {
                    JobFailure::Unavailable => ambre_prose::message::MediaFailure::Unavailable,
                    JobFailure::Refused => ambre_prose::message::MediaFailure::Undecodable,
                });
                if let Some(outcome) = state.handle_decoded(ticket, decoded) {
                    self.dispatch_prose_media(document_id, outcome);
                }
            }
            JobOwner::Toile(document_id) => {
                let Some(DocState::Toile(state)) =
                    self.docs.get_mut(&document_id).map(|d| &mut d.state)
                else {
                    return;
                };
                let outcome = state.handle_decoded(ticket, pixels);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    outcome.requests,
                );
            }
        }
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
                let outcome = state.handle_conversion(cmd, converter);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    Vec::new(),
                );
            }
            (DocState::Toile(state), AppCommand::Toile(cmd)) => {
                let outcome = state.handle_conversion(cmd, Ok(converter));
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Toile).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    outcome.requests,
                );
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
                self.owe_layout(document_id);
            }
            _ => {}
        }
    }

    /// Answer a command the converter could not serve, in the words its
    /// application shows. Nothing is silent (`ARCH-081`). A Prose or a Toile
    /// command learns the reason through its own state: a Prose paste then
    /// lands its plain text.
    fn answer_unconverted(&mut self, document_id: Id<Document>, command: AppCommand, reason: &str) {
        let stopped = || InteropError::Stopped(reason.to_owned());
        let events = match (
            self.docs.get_mut(&document_id).map(|d| &mut d.state),
            command,
        ) {
            (None, _) => return,
            (Some(DocState::Prose(state)), AppCommand::Prose(cmd)) => {
                let outcome = state.handle_conversion(cmd, Err(reason));
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
