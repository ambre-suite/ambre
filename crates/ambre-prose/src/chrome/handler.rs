// ambre-prose/src/chrome/handler.rs
// ============================================================================
// Prose's command handler (`ARCH-018`): what each Prose `StaticKey<Commands>` means.
//
// The file commands each become one typed command: the native save, the
// imports and exports of each format, and the PDF export (`PROSE-IO-020`,
// `PROSE-IO-027`, `PROSE-IO-033`). A formatting
// chord lands where the format bar's controls leave their id, and the view
// drains both the same way (`PROSE-UI-056`).
// ============================================================================

use crate::message::{
    ExportFormat, ImportFormat, MarkdownFlavor, PdfExport, ProseCommand, ProseImport, ReviewExport,
    WordExport,
};
use ambre_bus::interop::InteropRequest;
use ambre_chrome::{CommandHandler, FileAccept, FilePick, KeyRoute, ShellAction};
use ambre_commands::{ClipboardWrite, PasteForm};
use ambre_core::{Commands, StaticKey};
use leptos::prelude::*;

use crate::commands::{
    COPY, CUT, EXPORT_HTML, EXPORT_HTML_REVIEWED, EXPORT_MARKDOWN, EXPORT_ODT, EXPORT_ODT_REVIEWED,
    EXPORT_PDF, EXPORT_WORD, EXPORT_WORD_REVIEWED, IMPORT_HTML, IMPORT_MARKDOWN, IMPORT_ODT,
    IMPORT_WORD, PALETTE, PASTE, PASTE_PLAIN, REDO, SAVE_FILE, UNDO, mark_key,
};

/// The signal Prose's chords act through. `Copy`, so the handler is too.
#[derive(Clone, Copy)]
pub struct ProseHandler {
    /// The formatting id the view drains, shared with the format bar.
    pub format: RwSignal<Option<StaticKey<Commands>>>,
    /// The page's clock: an import dates a reply the file leaves undated at
    /// the moment the user chose the file.
    pub now: fn() -> ambre_core::Instant<ambre_core::Authored>,
    /// The session's time zone, by its IANA name. A Word import reads a
    /// wall time under its rules, and a Word export writes one under them.
    /// The reader and the writer never guess it from the machine.
    pub zone: fn() -> crate::message::TimeZone,
}

impl CommandHandler for ProseHandler {
    type Command = ProseCommand;

    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<ProseCommand>> {
        let export = |format| ProseCommand::Interop(InteropRequest::Export(format));
        let now = self.now;
        let zone = self.zone;
        let word = |review| {
            export(ExportFormat::Word(WordExport {
                review,
                zone: zone(),
            }))
        };
        let odt = |review| {
            export(ExportFormat::OpenDocument(WordExport {
                review,
                zone: zone(),
            }))
        };
        let pick = |format, accept, unreadable| {
            ShellAction::PickFile(FilePick {
                accept,
                unreadable,
                read: Box::new(move |data| {
                    ProseCommand::Interop(InteropRequest::Import(ProseImport {
                        format,
                        data,
                        now: now(),
                        zone: zone(),
                    }))
                }),
            })
        };
        let action = match id {
            PALETTE => ShellAction::OpenPalette,
            SAVE_FILE => ShellAction::Send(export(ExportFormat::Ambre)),
            EXPORT_MARKDOWN => {
                ShellAction::Send(export(ExportFormat::Markdown(MarkdownFlavor::Gfm)))
            }
            // The bytes are converted on the client (`PROSE-IO-001`).
            IMPORT_MARKDOWN => pick(
                ImportFormat::Markdown,
                FileAccept::Markdown,
                "Could not read the Markdown file",
            ),
            EXPORT_WORD => ShellAction::Send(word(ReviewExport::Omitted)),
            // Publishing the review is an act of its own (`PROSE-IO-081`).
            EXPORT_WORD_REVIEWED => ShellAction::Send(word(ReviewExport::Included)),
            IMPORT_WORD => pick(
                ImportFormat::Word,
                FileAccept::Word,
                "Could not read the Word file",
            ),
            EXPORT_ODT => ShellAction::Send(odt(ReviewExport::Omitted)),
            EXPORT_ODT_REVIEWED => ShellAction::Send(odt(ReviewExport::Included)),
            IMPORT_ODT => pick(
                ImportFormat::OpenDocument,
                FileAccept::OpenDocumentText,
                "Could not read the OpenDocument file",
            ),
            EXPORT_HTML => ShellAction::Send(export(ExportFormat::Html(ReviewExport::Omitted))),
            EXPORT_HTML_REVIEWED => {
                ShellAction::Send(export(ExportFormat::Html(ReviewExport::Included)))
            }
            // The file declares the document's language (`PROSE-IO-032`).
            EXPORT_PDF => ShellAction::Send(export(ExportFormat::Pdf(PdfExport::default()))),
            IMPORT_HTML => pick(
                ImportFormat::Html,
                FileAccept::Html,
                "Could not read the HTML document",
            ),
            // A menu row or the palette; the chords are the view's.
            UNDO => ShellAction::Send(ProseCommand::Undo),
            REDO => ShellAction::Send(ProseCommand::Redo),
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
    ) -> KeyRoute<ProseCommand> {
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

    fn sealed_save(
        &self,
        _passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<ProseCommand> {
        // Prose declares no sealed save.
        None
    }
}
