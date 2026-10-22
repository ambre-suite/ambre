// ambre-app/src/shell/commands.rs
// ============================================================================
// The shell's half of the command path (`ARCH-018`).
//
// A toolbar button, a menu entry, the palette and a chord all resolve to a
// `CommandId`. The booted application's handler says what the id means. This
// module only performs the answer. It wraps a command for the routed document,
// opens an overlay the shell owns, drives the clipboard or opens a picker.
// Nothing here matches on an application's id.
// ============================================================================

use ambre_chrome::{CommandHandler, KeyRoute, ShellAction};
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
) -> Callback<LogicCommand> {
    Callback::new(move |command: LogicCommand| {
        if let Err(e) = to_logic.try_send(command) {
            tracing::warn!("Murmure command dropped, Logic channel full ({e:?})");
        }
    })
}

impl AppHandler {
    fn get(&self) -> &dyn CommandHandler<Command = AppCommand> {
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
    pub(crate) current_doc: RwSignal<Option<DocumentId>>,
    pub(crate) to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    pub(crate) clipboard_paste: ambre_bus::conduit::Sender<ClipboardPaste>,
    pub(crate) toasts: Toasts,
    /// The command palette, a shell overlay.
    pub(crate) palette_open: RwSignal<bool>,
    /// The passphrase prompt of a sealed save, a shell overlay.
    pub(crate) seal_open: RwSignal<bool>,
}

impl CommandPath {
    /// Run the command `id` from a toolbar, a menu or the palette.
    pub(crate) fn run(&self, id: CommandId) {
        for action in self.handler.get().on_command(id) {
            self.perform(action);
        }
    }

    /// Run the chord that resolved to `id`. `false` when the key yields to the
    /// focused field, whose default must then stand.
    pub(crate) fn key(&self, id: CommandId, in_text_field: bool) -> bool {
        match self.handler.get().on_key(id, in_text_field) {
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
    pub(crate) fn seal(&self, passphrase: String) {
        if let Some(command) = self.handler.get().sealed_save(passphrase) {
            self.send(command);
        }
    }

    /// Send `command` to the routed document. On the landing route no document
    /// is open, and nothing could receive it.
    pub(crate) fn send(&self, command: AppCommand) {
        let Some(document_id) = self.current_doc.get_untracked() else {
            return;
        };
        if let Err(e) = self.to_logic.try_send(LogicCommand::App {
            document_id,
            command,
        }) {
            tracing::warn!("command dropped, Logic channel full ({e:?})");
        }
    }

    fn perform(&self, action: ShellAction<AppCommand>) {
        match action {
            ShellAction::Send(command) => self.send(command),
            ShellAction::OpenPalette => self.palette_open.set(true),
            ShellAction::OpenSealedSave => self.seal_open.set(true),
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
                    id: RequestId::next(),
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
