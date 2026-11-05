// ambre-app/src/shell/commands.rs
// ============================================================================
// The shell's half of the command path (`ARCH-018`).
//
// A toolbar button, a menu entry, the palette and a chord all resolve to a
// `StaticKey<Commands>`. The booted application's handler says what the id means. This
// module only performs the answer. It wraps a command for the routed document,
// opens an overlay the shell owns, drives the clipboard or opens a picker.
// Nothing here matches on an application's id.
// ============================================================================

use ambre_bus::Embed;
use ambre_chrome::{CommandHandler, KeyRoute, ShellAction};
use ambre_core::{Commands, StaticKey};
use ambre_murmure::chrome::MurmureHandler;
use ambre_prose::chrome::ProseHandler;
use ambre_toile::chrome::ToileHandler;
use ambre_trame::chrome::TrameHandler;

use super::super::*;
use super::app_root::{exec_clipboard, read_clipboard};

/// The booted application's handler. The set is closed, as `AppView`'s is:
/// naming the applications is the launcher's integration.
#[derive(Clone, Copy)]
pub(crate) enum AppHandler {
    Trame(TrameHandler),
    Toile(ToileHandler),
    Prose(ProseHandler),
    Murmure(MurmureHandler),
}

/// The sink of a Murmure page. Its commands address the user's channels,
/// not the routed document, so they go to the Logic thread unchanged.
pub(crate) fn murmure_send(
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
) -> Callback<ambre_bus::LogicCommand> {
    Callback::new(move |command: ambre_bus::LogicCommand| {
        // A Murmure command carries no document command, so it retypes as any.
        if let Err(e) = to_logic.try_send(command.map(|never| match never {})) {
            tracing::warn!("Murmure command dropped, Logic channel full ({e:?})");
        }
    })
}

/// A handler as the shell drives it. Each application answers in its own
/// command, and the answer is embedded into the sum the Logic thread takes
/// (`BUS-032`). The shell then never names an application's command.
trait EmbeddedHandler {
    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<AppCommand>>;
    fn on_key(
        &self,
        id: StaticKey<Commands>,
        target: ambre_chrome::KeyTarget,
    ) -> KeyRoute<AppCommand>;
    fn sealed_save(
        &self,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<AppCommand>;
}

impl<H> EmbeddedHandler for H
where
    H: CommandHandler,
    H::Command: 'static,
    AppCommand: Embed<H::Command>,
{
    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<AppCommand>> {
        CommandHandler::on_command(self, id)
            .into_iter()
            .map(|action| action.map(AppCommand::embed))
            .collect()
    }

    fn on_key(
        &self,
        id: StaticKey<Commands>,
        target: ambre_chrome::KeyTarget,
    ) -> KeyRoute<AppCommand> {
        CommandHandler::on_key(self, id, target).map(AppCommand::embed)
    }

    fn sealed_save(
        &self,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<AppCommand> {
        CommandHandler::sealed_save(self, passphrase).map(AppCommand::embed)
    }
}

impl AppHandler {
    fn get(&self) -> &dyn EmbeddedHandler {
        match self {
            Self::Trame(h) => h,
            Self::Toile(h) => h,
            Self::Prose(h) => h,
            Self::Murmure(h) => h,
        }
    }

    /// Murmure's handler, when a Murmure page is booted.
    pub(crate) fn murmure(&self) -> Option<MurmureHandler> {
        match self {
            Self::Murmure(h) => Some(*h),
            _ => None,
        }
    }

    /// Trame's handler, when a sheet is booted.
    pub(crate) fn trame(&self) -> Option<TrameHandler> {
        match self {
            Self::Trame(h) => Some(*h),
            _ => None,
        }
    }
}

/// Everything the shell needs to perform what a handler answers. Every field
/// is `Send`, so a callback crossing the router's boundary can hold it.
#[derive(Clone)]
pub(crate) struct CommandPath {
    pub(crate) handler: AppHandler,
    pub(crate) current_doc: RwSignal<Option<Id<Document>>>,
    pub(crate) to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    pub(crate) clipboard_paste: ambre_bus::conduit::Sender<ClipboardPaste>,
    pub(crate) toasts: Toasts,
    /// The command palette, a shell overlay.
    pub(crate) palette_open: RwSignal<ambre_chrome::Openness>,
    /// The passphrase prompt of a sealed save, a shell overlay.
    pub(crate) seal_open: RwSignal<ambre_chrome::Openness>,
}

impl CommandPath {
    /// Run the command `id` from a toolbar, a menu or the palette.
    pub(crate) fn run(&self, id: StaticKey<Commands>) {
        for action in self.handler.get().on_command(id) {
            self.perform(action);
        }
    }

    /// Run the chord that resolved to `id`. `false` when the key yields to the
    /// focused field, whose default must then stand.
    pub(crate) fn key(&self, id: StaticKey<Commands>, target: ambre_chrome::KeyTarget) -> bool {
        match self.handler.get().on_key(id, target) {
            KeyRoute::Yield => false,
            KeyRoute::Run(actions) => {
                for action in actions {
                    self.perform(action);
                }
                true
            }
        }
    }

    /// End the sealed save with the passphrase the prompt collected.
    pub(crate) fn seal(&self, passphrase: ambre_core::Confidential<ambre_core::Passphrase>) {
        if let Some(command) = self.handler.get().sealed_save(passphrase) {
            self.send(command);
        }
    }

    /// Send `command` to the routed document. On the landing route no document
    /// is open, and nothing could receive it.
    pub(crate) fn send<C>(&self, command: C)
    where
        AppCommand: Embed<C>,
    {
        let Some(document_id) = self.current_doc.get_untracked() else {
            return;
        };
        if let Err(e) = self.to_logic.try_send(LogicCommand::App {
            document_id,
            command: AppCommand::embed(command),
        }) {
            tracing::warn!("command dropped, Logic channel full ({e:?})");
        }
    }

    fn perform(&self, action: ShellAction<AppCommand>) {
        match action {
            ShellAction::Send(command) => self.send(command),
            ShellAction::OpenPalette => self.palette_open.set(ambre_chrome::Openness::Open),
            ShellAction::OpenSealedSave => self.seal_open.set(ambre_chrome::Openness::Open),
            ShellAction::Copy(write) => exec_clipboard(write),
            ShellAction::CopyText(text) => {
                if let Some(win) = web_sys::window() {
                    let _ = win.navigator().clipboard().write_text(&text);
                }
            }
            ShellAction::Paste(form) => {
                read_clipboard(self.clipboard_paste.clone(), self.toasts, form);
            }
            ShellAction::RefreshDocuments => {
                let _ = self.to_logic.try_send(LogicCommand::Rest {
                    id: ambre_bus::next_request(),
                    cmd: RestCommand::ListDocuments,
                });
            }
            ShellAction::PickFile(pick) => {
                // The document is the one open when the picker opened, not
                // whichever is open once the user has chosen.
                let Some(document_id) = self.current_doc.get_untracked() else {
                    return;
                };
                let tx = self.to_logic.clone();
                let toasts = self.toasts;
                let read = Rc::new(pick.read);
                open_file_picker(pick.accept, pick.unreadable, toasts, move |data| {
                    let command = read(data);
                    // A full queue loses the file: say so, and the user picks
                    // it again.
                    if let Err(e) = tx.try_send(LogicCommand::App {
                        document_id,
                        command,
                    }) {
                        tracing::warn!("file import dropped, Logic channel full ({e:?})");
                        toasts.warning("The file did not import: the document is busy. Try again.");
                    }
                });
            }
        }
    }
}
