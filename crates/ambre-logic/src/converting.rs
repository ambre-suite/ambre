// ambre-logic/src/converting.rs
// ============================================================================
// The converter slot: the module the Logic thread loads on the first command
// that reads or writes a file (`ambre-spec-architecture.md section 2`, the
// converter module).
//
// The thread cannot fetch, so it asks the Sync worker once, and the commands
// that need the converter wait here until the checked bytes land. A load that
// fails answers every waiting command with the reason. The next command asks
// again: a failed fetch is often a network that came back.
// ============================================================================

use super::*;
use ambre_convert::Converter;
use ambre_core::Count;
use ambre_toile::message::{ToileCommand, ToileEvent};
use ambre_trame::message::TrameEvent;

/// How many commands may wait for the converter at once. A user may click an
/// export a few times while the module loads. Past this bound, the click is
/// told to wait, rather than held without bound (`ARCH-081`).
pub(crate) const WAITING_LIMIT: Count<WaitingConversions> = Count::new(16);

/// The commands that wait for the converter.
pub(crate) enum WaitingConversions {}

impl ambre_core::CountKind for WaitingConversions {
    type Repr = u32;
    const NAME: &'static str = "WaitingConversionCount";
}

/// Where the converter stands for this session.
pub(crate) enum ConverterSlot {
    /// Not loaded, and not asked for.
    Absent,
    /// Asked of the Sync worker. The commands wait in arrival order.
    Asked(Vec<WaitingConversion>),
    /// Loaded: every later conversion runs at once.
    Ready(Converter),
}

/// A command that waits for the converter, and the document it targets.
pub(crate) struct WaitingConversion {
    document_id: Id<Document>,
    command: AppCommand,
}

/// Whether `command` reads or writes a file through the converter.
pub(crate) fn needs_converter(command: &AppCommand) -> bool {
    match command {
        AppCommand::Toile(cmd) => matches!(cmd, ToileCommand::ExportPdf),
        AppCommand::Trame(cmd) => cmd.needs_converter(),
        _ => false,
    }
}

impl LogicState {
    /// Serve a command that needs the converter: at once when it is loaded,
    /// otherwise after the Sync worker brought it. The first such command of
    /// the session asks for it (`ARCH-081`).
    pub(crate) fn convert_for(&mut self, document_id: Id<Document>, command: AppCommand) {
        match &mut self.converter {
            ConverterSlot::Ready(_) => self.run_conversion(document_id, command),
            ConverterSlot::Asked(waiting) if Count::of(waiting.len()) < WAITING_LIMIT => {
                waiting.push(WaitingConversion {
                    document_id,
                    command,
                });
            }
            ConverterSlot::Asked(_) => {
                self.answer_unconverted(
                    document_id,
                    &command,
                    "the converter is still loading: try again in a moment",
                );
            }
            ConverterSlot::Absent => {
                self.converter = ConverterSlot::Asked(vec![WaitingConversion {
                    document_id,
                    command,
                }]);
                self.send_to_sync(SyncCommand::FetchConverter);
            }
        }
    }

    /// The Sync worker brought the module and checked it against the build's
    /// digest (`ARCH-080`): load it, then run every waiting command in order.
    pub(crate) fn converter_fetched(&mut self, module: ambre_core::Payload<ConverterModule>) {
        match Converter::load(&module) {
            Ok(converter) => {
                let waiting = self.take_waiting();
                self.converter = ConverterSlot::Ready(converter);
                for w in waiting {
                    self.run_conversion(w.document_id, w.command);
                }
            }
            Err(error) => self.converter_refused(&error.to_string()),
        }
    }

    /// The module did not arrive, or did not load: every waiting command
    /// learns why, and the next one asks again.
    pub(crate) fn converter_refused(&mut self, reason: &str) {
        let waiting = self.take_waiting();
        self.converter = ConverterSlot::Absent;
        let reason = format!("the converter could not be loaded: {reason}");
        for w in waiting {
            self.answer_unconverted(w.document_id, &w.command, &reason);
        }
    }

    fn take_waiting(&mut self) -> Vec<WaitingConversion> {
        match std::mem::replace(&mut self.converter, ConverterSlot::Absent) {
            ConverterSlot::Asked(waiting) => waiting,
            ConverterSlot::Ready(converter) => {
                self.converter = ConverterSlot::Ready(converter);
                Vec::new()
            }
            ConverterSlot::Absent => Vec::new(),
        }
    }

    /// Run one conversion on the loaded module. The document may have closed
    /// while the module loaded: its command is then dropped, as any command of
    /// a closed document is.
    fn run_conversion(&mut self, document_id: Id<Document>, command: AppCommand) {
        match command {
            AppCommand::Toile(ToileCommand::ExportPdf) => self.export_toile_pdf(document_id),
            AppCommand::Trame(cmd) => {
                self.settle_import(document_id);
                let Self {
                    docs, converter, ..
                } = self;
                let (Some(DocState::Trame(state)), ConverterSlot::Ready(converter)) = (
                    docs.get_mut(&document_id).map(|d| &mut d.state),
                    &*converter,
                ) else {
                    return;
                };
                let outcome = state.handle_conversion(cmd, converter);
                self.dispatch_outcome(
                    document_id,
                    outcome.display.into_iter().map(AppEvent::Trame).collect(),
                    outcome.update,
                    outcome.ephemeral,
                    Vec::new(),
                );
            }
            _ => {}
        }
    }

    /// Answer a command the converter could not serve, in the words its
    /// application shows. Nothing is silent (`ARCH-081`).
    fn answer_unconverted(
        &mut self,
        document_id: Id<Document>,
        command: &AppCommand,
        reason: &str,
    ) {
        let event = match command {
            AppCommand::Toile(_) => AppEvent::Toile(ToileEvent::ExportFailed(reason.to_owned())),
            AppCommand::Trame(_) => AppEvent::Trame(TrameEvent::Error {
                message: reason.to_owned(),
            }),
            _ => return,
        };
        if self.docs.contains_key(&document_id) {
            self.dispatch_outcome(document_id, vec![event], None, None, Vec::new());
        }
    }
}
