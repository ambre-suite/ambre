// ambre-trame/src/chrome/handler.rs
// ============================================================================
// Trame's command handler (`ARCH-018`): what each Trame `StaticKey<Commands>` means.
//
// The shell resolves a trigger to an id and asks here. The panels Trame owns
// -- find and replace, the filter popover -- open from here, through their
// signals. What reaches the shell is only what the shell performs: a command
// for the sheet, the palette, the sealed-save prompt, the clipboard, a picker.
// ============================================================================

use crate::message::{StylePatch, TrameCommand};
use ambre_chrome::commands::{ClipboardWrite, PasteForm};
use ambre_chrome::{CommandHandler, FileAccept, FilePick, KeyRoute, ShellAction};
use ambre_core::index::Pos;
use ambre_core::{Commands, StaticKey};
use ambre_core::{Css, Point, Px, View};
use leptos::prelude::*;

use super::commands::{
    BOLD, COPY, CUT, FILTER_COLUMN, FIND, IMPORT_CSV, IMPORT_ODS, IMPORT_XLSX, ITALIC, PALETTE,
    PASTE, REPLACE, SAVE_SEALED_FILE, SORT_RANGE_MULTI, ZOOM_TO_SELECTION, command_to_app,
};
use super::find_replace::{FindMode, FindReplaceState};
use super::popovers::FilterPopoverState;
use super::style::{Flip, StyleState};

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
    /// Where the view last drew the active cell, on the canvas.
    pub anchor: RwSignal<Option<Point<Px<View>>>>,
    /// A command the grid view performs itself, a zoom to the selection
    /// (`ZOOM-007`). The view takes it on its next frame; nothing reaches the
    /// Logic thread but the cull of the new view (`ZOOM-001`).
    pub view_cmd: RwSignal<Option<StaticKey<Commands>>>,
}

impl TrameHandler {
    /// Open the filter popover under the active column, and return the request
    /// for that column's values (`trame-shared-filter`). The menu entry and
    /// the in-canvas filter button both come here, so they share one path.
    pub fn open_filter(&self) -> TrameCommand {
        let at = client_anchor(self.anchor.get_untracked());
        // `col` is provisional; the ColumnValues reply carries the real one.
        self.filter_popover.set(Some(FilterPopoverState {
            col: Pos::<ambre_bus::Col>::ZERO,
            at,
        }));
        self.filter_values.set(Vec::new());
        TrameCommand::RequestActiveColumnValues
    }

    /// Open find and replace in `mode`, the panel that emits the search itself.
    fn open_find(&self, mode: FindMode) {
        self.find.set(Some(FindReplaceState { mode }));
    }

    /// Flip bold or italic against the tracked style (`trame-cell-styles`).
    fn toggle_style(&self, style: StyleToggle) -> TrameCommand {
        let mut patch = StylePatch::default();
        match style {
            StyleToggle::Bold => {
                patch.bold = Some(self.style.bold.get_untracked().flipped());
            }
            StyleToggle::Italic => {
                patch.italic = Some(self.style.italic.get_untracked().flipped());
            }
        }
        TrameCommand::SetCellStyle { patch }
    }
}

/// The style a toolbar toggle flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StyleToggle {
    Bold,
    Italic,
}

impl CommandHandler for TrameHandler {
    type Command = TrameCommand;

    fn on_command(&self, id: StaticKey<Commands>) -> Vec<ShellAction<TrameCommand>> {
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
            ZOOM_TO_SELECTION => {
                self.view_cmd.set(Some(id));
                return Vec::new();
            }
            // A band selection has no extent the canvas knows, so the sort
            // dialog opens on the Logic thread's `SortRange` reply.
            SORT_RANGE_MULTI => ShellAction::Send(TrameCommand::RequestSortRange),
            FILTER_COLUMN => ShellAction::Send(self.open_filter()),
            BOLD => ShellAction::Send(self.toggle_style(StyleToggle::Bold)),
            ITALIC => ShellAction::Send(self.toggle_style(StyleToggle::Italic)),
            COPY => ShellAction::Copy(ClipboardWrite::Copy),
            CUT => ShellAction::Copy(ClipboardWrite::Cut),
            PASTE => ShellAction::Paste(PasteForm::Rich),
            // The CSV becomes a new sheet, converted on the client
            // (`trame-import-export`).
            IMPORT_CSV => ShellAction::PickFile(FilePick {
                accept: FileAccept::Csv,
                unreadable: "Could not read the CSV file",
                read: Box::new(|data| {
                    TrameCommand::import_csv(crate::message::CsvImport {
                        data,
                        delimiter: None,
                        header: crate::message::HeaderRow::Absent,
                        target_sheet_id: None,
                        start_row: Pos::<ambre_bus::Row>::ZERO,
                        start_col: Pos::<ambre_bus::Col>::ZERO,
                    })
                }),
            }),
            // Each sheet of the workbook becomes a new sheet, converted on
            // the client (`trame-import-export`).
            IMPORT_XLSX => ShellAction::PickFile(FilePick {
                accept: FileAccept::Excel,
                unreadable: "Could not read the Excel file",
                read: Box::new(|data| {
                    TrameCommand::Interop(ambre_bus::interop::InteropRequest::Import(
                        crate::message::TrameImport::Xlsx(crate::message::WorkbookImport { data }),
                    ))
                }),
            }),
            // The same workbook an Excel file of the same content lands as
            // (`TRAME-IO-029`).
            IMPORT_ODS => ShellAction::PickFile(FilePick {
                accept: FileAccept::OpenDocumentSpreadsheet,
                unreadable: "Could not read the OpenDocument spreadsheet",
                read: Box::new(|data| {
                    TrameCommand::Interop(ambre_bus::interop::InteropRequest::Import(
                        crate::message::TrameImport::Ods(crate::message::WorkbookImport { data }),
                    ))
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

    fn on_key(
        &self,
        id: StaticKey<Commands>,
        _target: ambre_chrome::KeyTarget,
    ) -> KeyRoute<TrameCommand> {
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

    fn sealed_save(
        &self,
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    ) -> Option<TrameCommand> {
        // The passphrase goes into the command and nowhere else (`ARCH-004`).
        Some(TrameCommand::export(
            crate::message::TrameExport::AmbreSealed { passphrase },
        ))
    }
}

/// Convert a canvas-relative anchor into client space, by adding the canvas's
/// offset in the viewport. With no anchor, the canvas's top-left.
fn client_anchor(anchor: Option<Point<Px<View>>>) -> Point<Px<Css>> {
    let origin = document()
        .get_element_by_id("ambre-canvas")
        .map(|c| {
            let r = c.get_bounding_client_rect();
            // The DOM measures in `f64`; a CSS pixel is an `f32`.
            Point::new(
                Px::<Css>::new(r.left() as f32),
                Px::<Css>::new(r.top() as f32),
            )
        })
        .unwrap_or_default();
    anchor.unwrap_or_default().on_page(origin)
}
