// ambre-prose/src/chrome/handler.rs
// ============================================================================
// Prose's command handler (`ARCH-018`): what each Prose `StaticKey<Commands>` means.
//
// The file commands each become one typed command: the native save, and the
// Markdown import and export (`PROSE-IO-020`, `PROSE-IO-027`). A formatting
// chord lands where the format bar's controls leave their id, and the view
// drains both the same way (`PROSE-UI-056`).
// ============================================================================

use ambre_bus::AppCommand;
use ambre_bus::app::prose::{ExportFormat, ImportFormat, MarkdownFlavor, ProseCommand};
use ambre_chrome::{CommandHandler, FileAccept, FilePick, KeyRoute, ShellAction};
use ambre_commands::{ClipboardWrite, Commands, PasteForm, StaticKey};
use leptos::prelude::*;

use crate::commands::{
    COPY, CUT, EXPORT_MARKDOWN, IMPORT_MARKDOWN, PALETTE, PASTE, PASTE_PLAIN, REDO, SAVE_FILE,
    UNDO, mark_key,
};

/// The signal Prose's chords act through. `Copy`, so the handler is too.
#[derive(Clone, Copy)]
pub struct ProseHandler {
    /// The formatting id the view drains, shared with the format bar.
    pub format: RwSignal<Option<StaticKey<Commands>>>,
}

impl CommandHandler for ProseHandler {
    type Command = AppCommand;

    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<AppCommand>> {
        let export = |format| AppCommand::Prose(ProseCommand::ExportDocument { format });
        let action = match id {
            PALETTE => ShellAction::OpenPalette,
            SAVE_FILE => ShellAction::Send(export(ExportFormat::Ambre)),
            EXPORT_MARKDOWN => {
                ShellAction::Send(export(ExportFormat::Markdown(MarkdownFlavor::Gfm)))
            }
            // The bytes are converted on the client (`PROSE-IO-001`).
            IMPORT_MARKDOWN => ShellAction::PickFile(FilePick {
                accept: FileAccept::Markdown,
                unreadable: "Could not read the Markdown file",
                read: Box::new(|data| {
                    AppCommand::Prose(ProseCommand::ImportDocument {
                        format: ImportFormat::Markdown,
                        data,
                    })
                }),
            }),
            // A menu row or the palette; the chords are the view's.
            UNDO => ShellAction::Send(AppCommand::Prose(ProseCommand::Undo)),
            REDO => ShellAction::Send(AppCommand::Prose(ProseCommand::Redo)),
            COPY => ShellAction::Copy(ClipboardWrite::Copy),
            CUT => ShellAction::Copy(ClipboardWrite::Cut),
            PASTE => ShellAction::Paste(PasteForm::Rich),
            PASTE_PLAIN => ShellAction::Paste(PasteForm::Plain),
            _ => return Vec::new(),
        };
        vec![action]
    }

    fn on_key(
        &self,
        id: StaticKey<Commands>,
        target: ambre_chrome::KeyTarget,
    ) -> KeyRoute<AppCommand> {
        if id == PALETTE {
            return KeyRoute::Run(vec![ShellAction::OpenPalette]);
        }
        // A chrome text field keeps its own formatting chords.
        if mark_key(id).is_some() {
            if target == ambre_chrome::KeyTarget::TextField {
                return KeyRoute::Yield;
            }
            self.format.set(Some(id));
        }
        KeyRoute::Run(Vec::new())
    }

    fn sealed_save(&self, _passphrase: ambre_core::Passphrase) -> Option<AppCommand> {
        // Prose declares no sealed save.
        None
    }
}
