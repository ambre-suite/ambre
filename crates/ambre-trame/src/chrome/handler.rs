// ambre-trame/src/chrome/handler.rs
// ============================================================================
// Trame's command handler (`ARCH-018`): what each Trame `CommandId` means.
//
// The shell resolves a trigger to an id and asks here. The panels Trame owns
// -- find and replace, the filter popover -- open from here, through their
// signals. What reaches the shell is only what the shell performs: a command
// for the sheet, the palette, the sealed-save prompt, the clipboard, a picker.
// ============================================================================

use ambre_bus::{AppCommand, StylePatch, TrameCommand};
use ambre_chrome::commands::{ClipboardWrite, CommandId, PasteForm};
use ambre_chrome::{CommandHandler, FilePick, KeyRoute, ShellAction};
use leptos::prelude::*;

use super::commands::{
    BOLD, COPY, CUT, FILTER_COLUMN, FIND, IMPORT_CSV, ITALIC, PALETTE, PASTE, REPLACE,
    SAVE_SEALED_FILE, SORT_RANGE_MULTI, command_to_app,
};
use super::find_replace::{FindMode, FindReplaceState};
use super::popovers::FilterPopoverState;
use super::style::StyleState;

/// The signals Trame's commands open its panels through. Each is `Copy`, so
/// the handler is too.
#[derive(Clone, Copy)]
pub struct TrameHandler {
    /// The find and replace panel, and the mode it opens in.
    pub find: RwSignal<Option<FindReplaceState>>,
    /// The filter popover, anchored under the active column.
    pub filter_popover: RwSignal<Option<FilterPopoverState>>,
    /// The popover's distinct values, refilled by the Logic thread's reply.
    pub filter_values: RwSignal<Vec<String>>,
    /// The active cell's style, which the bold and italic toggles flip.
    pub style: StyleState,
    /// Where the view last drew the active cell, in canvas CSS pixels.
    pub anchor: RwSignal<Option<(f64, f64)>>,
}

impl TrameHandler {
    /// Open the filter popover under the active column, and return the request
    /// for that column's values (`trame-shared-filter`). The menu entry and
    /// the in-canvas filter button both come here, so they share one path.
    pub fn open_filter(&self) -> AppCommand {
        let (x, y) = client_anchor(self.anchor.get_untracked());
        // `col` is provisional; the ColumnValues reply carries the real one.
        self.filter_popover.set(Some(FilterPopoverState {
            col: ambre_bus::Col::ZERO,
            x,
            y,
        }));
        self.filter_values.set(Vec::new());
        AppCommand::Trame(TrameCommand::RequestActiveColumnValues)
    }

    /// Open find and replace in `mode`, the panel that emits the search itself.
    fn open_find(&self, mode: FindMode) {
        self.find.set(Some(FindReplaceState { mode }));
    }

    /// Flip bold or italic against the tracked style (`trame-cell-styles`).
    fn toggle_style(&self, bold: bool) -> AppCommand {
        let mut patch = StylePatch::default();
        if bold {
            patch.bold = Some(!self.style.bold.get_untracked());
        } else {
            patch.italic = Some(!self.style.italic.get_untracked());
        }
        AppCommand::Trame(TrameCommand::SetCellStyle { patch })
    }
}

impl CommandHandler for TrameHandler {
    type Command = AppCommand;

    fn on_command(&self, id: CommandId) -> Vec<ShellAction<AppCommand>> {
        let action = match id {
            PALETTE => ShellAction::OpenPalette,
            SAVE_SEALED_FILE => ShellAction::OpenSealedSave,
            FIND => {
                self.open_find(FindMode::Find);
                return Vec::new();
            }
            REPLACE => {
                self.open_find(FindMode::Replace);
                return Vec::new();
            }
            // A band selection has no extent the canvas knows, so the sort
            // dialog opens on the Logic thread's `SortRange` reply.
            SORT_RANGE_MULTI => {
                ShellAction::Send(AppCommand::Trame(TrameCommand::RequestSortRange))
            }
            FILTER_COLUMN => ShellAction::Send(self.open_filter()),
            BOLD | ITALIC => ShellAction::Send(self.toggle_style(id == BOLD)),
            COPY => ShellAction::Copy(ClipboardWrite::Copy),
            CUT => ShellAction::Copy(ClipboardWrite::Cut),
            PASTE => ShellAction::Paste(PasteForm::Rich),
            // The CSV becomes a new sheet, converted on the client
            // (`trame-import-export`).
            IMPORT_CSV => ShellAction::PickFile(FilePick {
                accept: ".csv,text/csv",
                unreadable: "Could not read the CSV file",
                read: Box::new(|data| {
                    AppCommand::Trame(TrameCommand::ImportCsv {
                        data,
                        delimiter: None,
                        header: ambre_bus::HeaderRow::Absent,
                        target_sheet_id: None,
                        start_row: ambre_bus::Row::ZERO,
                        start_col: ambre_bus::Col::ZERO,
                    })
                }),
            }),
            _ => match command_to_app(id) {
                Some(command) => ShellAction::Send(command),
                None => {
                    tracing::warn!("no Trame command for {id:?}");
                    return Vec::new();
                }
            },
        };
        vec![action]
    }

    fn on_key(&self, id: CommandId, _target: ambre_chrome::KeyTarget) -> KeyRoute<AppCommand> {
        // A chord opens the palette and find and replace like the menu does.
        // Every other chord
        // maps straight through the table: the toggles and the clipboard keep
        // their keys in the view and the browser.
        match id {
            PALETTE => return KeyRoute::Run(vec![ShellAction::OpenPalette]),
            FIND => self.open_find(FindMode::Find),
            REPLACE => self.open_find(FindMode::Replace),
            _ => {
                return KeyRoute::Run(
                    command_to_app(id)
                        .map(ShellAction::Send)
                        .into_iter()
                        .collect(),
                );
            }
        }
        KeyRoute::Run(Vec::new())
    }

    fn sealed_save(&self, passphrase: String) -> Option<AppCommand> {
        // The typed text becomes a `SecretBytes` here and goes nowhere else
        // (`ARCH-004`).
        let passphrase = ambre_bus::SecretBytes::from(passphrase.into_bytes());
        Some(AppCommand::Trame(TrameCommand::ExportAmbreFileSealed {
            passphrase,
        }))
    }
}

/// Convert a canvas-relative anchor (CSS px) into client space, by adding the
/// canvas's offset in the viewport. With no anchor, the canvas's top-left.
fn client_anchor(anchor: Option<(f64, f64)>) -> (f64, f64) {
    let (ox, oy) = document()
        .get_element_by_id("ambre-canvas")
        .map(|c| {
            let r = c.get_bounding_client_rect();
            (r.left(), r.top())
        })
        .unwrap_or((0.0, 0.0));
    let (ax, ay) = anchor.unwrap_or((0.0, 0.0));
    (ox + ax, oy + ay)
}
