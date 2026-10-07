// ambre-toile/src/chrome/handler.rs
// ============================================================================
// Toile's command handler (`ARCH-018`): what each Toile `CommandId` means.
//
// The shell resolves a trigger to an id and asks here. The panels and dialogs
// Toile owns open from here, through their signals. Every other id goes to the
// canvas view, which resolves it against its own selection (`toile-menus`).
//
// A file reaches the board the same way from the picker and from a drop. Its
// bytes are read off the main thread, then published for the view to import
// (`toile-import-export`). The board decides what it reads, so the shell holds
// no format list.
// ============================================================================

use ambre_bus::AppCommand;
use ambre_bus::app::toile::ToileCommand;
use ambre_chrome::commands::CommandId;
use ambre_chrome::{CommandHandler, KeyRoute, ShellAction, Toasts};
use ambre_draw::Point;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

use crate::ui::Tool;
use crate::ui::interop::{DroppedFile, IMPORT_ACCEPT, accepts};
use crate::ui::palette::tool_for_command;

use super::{EMBED_DOC, EMBED_WEB, FIND, IMPORT_FILE, LINK_SET, PALETTE, SAVE_SEALED_FILE};

/// The open state and the typed text of Toile's three choosers: a web embed,
/// a document embed and a link (`TOILE-087`, `TOILE-086`, `toile-links`).
#[derive(Clone, Copy)]
pub struct ToileDialogs {
    /// The web embed dialog is open.
    pub embed_web_open: RwSignal<bool>,
    /// The address typed into the web embed dialog.
    pub embed_web_text: RwSignal<String>,
    /// Why the typed address was refused, read beside the field.
    pub embed_web_error: RwSignal<Option<&'static str>>,
    /// The document embed dialog is open.
    pub embed_doc_open: RwSignal<bool>,
    /// The link chooser is open.
    pub link_open: RwSignal<bool>,
    /// The address typed into the link chooser.
    pub link_text: RwSignal<String>,
    /// Why the typed link address was refused.
    pub link_error: RwSignal<Option<&'static str>>,
}

impl ToileDialogs {
    /// Every chooser closed and empty.
    pub fn new() -> Self {
        Self {
            embed_web_open: RwSignal::new(false),
            embed_web_text: RwSignal::new(String::new()),
            embed_web_error: RwSignal::new(None),
            embed_doc_open: RwSignal::new(false),
            link_open: RwSignal::new(false),
            link_text: RwSignal::new(String::new()),
            link_error: RwSignal::new(None),
        }
    }
}

impl Default for ToileDialogs {
    fn default() -> Self {
        Self::new()
    }
}

/// The signals Toile's commands act through. Each is `Copy`, so the handler
/// is too.
#[derive(Clone, Copy)]
pub struct ToileHandler {
    /// The find panel is open.
    pub find: RwSignal<bool>,
    /// The tool a chord arms.
    pub tool: RwSignal<Tool>,
    /// The id the canvas view resolves against its selection, next frame.
    pub menu_cmd: RwSignal<Option<CommandId>>,
    /// A file read for the board, which the view imports next frame.
    pub dropped: RwSignal<Option<DroppedFile>>,
    /// Where a refused file is explained.
    pub toasts: Toasts,
    /// The three choosers.
    pub dialogs: ToileDialogs,
}

impl CommandHandler for ToileHandler {
    type Command = AppCommand;

    fn on_command(&self, id: CommandId) -> Vec<ShellAction<AppCommand>> {
        let dialogs = self.dialogs;
        match id {
            PALETTE => return vec![ShellAction::OpenPalette],
            SAVE_SEALED_FILE => return vec![ShellAction::OpenSealedSave],
            FIND => self.find.set(true),
            // The file dialog is the board's (`TOILE-IO-020`): the board learns
            // of the file only once its bytes are read.
            IMPORT_FILE => open_import_picker(self.dropped, self.toasts),
            EMBED_WEB => {
                dialogs.embed_web_text.set(String::new());
                dialogs.embed_web_error.set(None);
                dialogs.embed_web_open.set(true);
            }
            EMBED_DOC => {
                // A document shared since the landing screen fetched is one the
                // user expects to find here.
                dialogs.embed_doc_open.set(true);
                return vec![ShellAction::RefreshDocuments];
            }
            LINK_SET => {
                dialogs.link_text.set(String::new());
                dialogs.link_error.set(None);
                dialogs.link_open.set(true);
                // Both lists are read again. A frame may have been added since
                // the panel last read the deck, and a document shared since the
                // landing screen fetched.
                return vec![
                    ShellAction::Send(AppCommand::Toile(ToileCommand::ReadDeck)),
                    ShellAction::RefreshDocuments,
                ];
            }
            _ => self.menu_cmd.set(Some(id)),
        }
        Vec::new()
    }

    fn on_key(&self, id: CommandId, in_text_field: bool) -> KeyRoute<AppCommand> {
        if let Some(tool) = tool_for_command(id) {
            // A bare tool key typed into a text field is a character.
            if in_text_field {
                return KeyRoute::Yield;
            }
            self.tool.set(tool);
            return KeyRoute::Run(Vec::new());
        }
        match id {
            PALETTE => return KeyRoute::Run(vec![ShellAction::OpenPalette]),
            FIND => self.find.set(true),
            // `ToileView::handle_key` decodes every other chord on the canvas.
            _ => {}
        }
        KeyRoute::Run(Vec::new())
    }

    fn sealed_save(&self, passphrase: String) -> Option<AppCommand> {
        // The typed text becomes a `SecretBytes` here and goes nowhere else
        // (`ARCH-004`).
        let passphrase = ambre_bus::SecretBytes::from(passphrase.into_bytes());
        Some(AppCommand::Toile(ToileCommand::ExportAmbreFileSealed {
            passphrase,
        }))
    }
}

/// Bring a file dropped on the canvas onto the board, at the point it landed
/// (`TOILE-IO-020`, `TOILE-IO-023`). The listener is installed once the canvas element
/// exists, and lives as long as the page.
///
/// The drop point stays in CSS pixels, the logical space the canvas view works
/// in, as the pointer path does.
pub fn install_file_drop(dropped: RwSignal<Option<DroppedFile>>, toasts: Toasts) {
    Effect::new(move |_| {
        let Some(canvas) = document().get_element_by_id("ambre-canvas") else {
            return;
        };
        // Without a `dragover` that prevents the default, the browser navigates
        // away to the dropped file and the drop never fires.
        let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|e: web_sys::DragEvent| {
            e.prevent_default();
        });
        let _ = canvas.add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref());
        over.forget();

        let cb = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
            e.prevent_default();
            let Some(file) = e
                .data_transfer()
                .and_then(|dt| dt.files())
                .and_then(|fs| fs.get(0))
            else {
                return;
            };
            let at = Point::new(f64::from(e.offset_x()), f64::from(e.offset_y()));
            read_file_into(file, Some(at), dropped, toasts);
        });
        let _ = canvas.add_event_listener_with_callback("drop", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}

/// Open a transient `<input type="file">` for a board import (`TOILE-IO-020`).
/// The picker offers what the board reads and nothing else. A file picked this
/// way has no drop point, so the view lands it in the middle of the viewport.
fn open_import_picker(dropped: RwSignal<Option<DroppedFile>>, toasts: Toasts) {
    let Some(input) = document()
        .create_element("input")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    input.set_type("file");
    let _ = input.set_attribute("accept", IMPORT_ACCEPT);
    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        if let Some(file) = picker.files().and_then(|fs| fs.get(0)) {
            read_file_into(file, None, dropped, toasts);
        }
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

/// Read `file` into memory and publish it for the board, or tell the user why
/// not (`TOILE-IO-022`). The check refuses a type no converter reads before it
/// touches the bytes, so a dropped video never costs a copy of itself.
fn read_file_into(
    file: web_sys::File,
    at: Option<Point>,
    dropped: RwSignal<Option<DroppedFile>>,
    toasts: Toasts,
) {
    // The browser leaves the type empty for a format it does not know. Fall
    // back to the extension, all an `.svg` drop carries on some platforms.
    let mut mime = file.type_();
    if mime.is_empty() && file.name().to_ascii_lowercase().ends_with(".svg") {
        mime = "image/svg+xml".to_owned();
    }
    if !accepts(&mime) {
        toasts.warning(format!(
            "Ambre cannot bring a {} file onto a board",
            label(&mime)
        ));
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
            Ok(buf) => {
                let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                dropped.set(Some(DroppedFile { bytes, mime, at }));
            }
            Err(_) => toasts.warning("Could not read the file"),
        }
    });
}

/// A media type as a user reads it: the subtype alone (`png`). Failing that, a
/// word for a file whose type the browser could not name.
fn label(mime: &str) -> String {
    match mime.rsplit('/').next() {
        Some(sub) if !sub.is_empty() => sub.to_owned(),
        _ => "file of that kind".to_owned(),
    }
}
