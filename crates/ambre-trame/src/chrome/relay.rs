// ambre-trame/src/chrome/relay.rs
// ============================================================================
// What Trame's chrome learns from its events, and what its panels ask of the
// sheet (`ARCH-018`).
//
// The launcher hands every event from the Logic thread here and names no
// variant. The sheet tabs, the status band, the filter popover and the sort
// dialog read the signals this fills. The canvas view gets the same events on
// its own queue, and publishes what it keeps of them (`UI-THREAD-003`).
//
// The other way, a panel's intent becomes a `TrameCommand` here. The launcher
// only sends it to the routed document.
// ============================================================================

use ambre_bus::{
    AppCommand, AppEvent, FilterKind, SheetName, SheetTab, StylePatch, TrameCommand, TrameEvent,
    UiEvent,
};
use ambre_core::id::{Id, Sheet};
use leptos::prelude::*;

use super::find_replace::{FindQuery, ReplaceRequest};
use super::popovers::FilterPopoverState;
use super::sheet_tabs::ReorderSheet;
use super::sort_dialog::{SortApply, SortDialogState};
use super::status::SelStats;

/// What one of Trame's panels asks of the sheet.
pub enum TrameIntent {
    /// Restyle the selection from the format bar (`trame-cell-styles`).
    Style(StylePatch),
    /// Filter the active column from the popover (`trame-shared-filter`).
    Filter(FilterKind),
    /// Sort the selection on the dialog's levels (`trame-sort`).
    Sort(SortApply),
    /// Step to the next match (`trame-find-replace`); `None` clears the highlight.
    Find(Option<FindQuery>),
    /// Replace the focused match.
    Replace(ReplaceRequest),
    /// Replace every match.
    ReplaceAll(ReplaceRequest),
    /// Show this sheet (`trame-sheets`).
    ActivateSheet(Id<Sheet>),
    /// Add a sheet after the last one.
    CreateSheet,
    /// Rename a sheet, as `(id, name)`.
    RenameSheet(Id<Sheet>, SheetName),
    /// Delete a sheet.
    DeleteSheet(Id<Sheet>),
    /// Move a sheet to another position.
    ReorderSheet(ReorderSheet),
}

/// The chrome signals Trame's events fill. Each is `Copy`, so the relay is too.
#[derive(Clone, Copy)]
pub struct TrameRelay {
    /// The sort dialog, opened over the range a sort would order
    /// (`trame-sort`).
    pub sort_dialog: RwSignal<Option<SortDialogState>>,
    /// The filter popover, pinned to the column its values came from.
    pub filter_popover: RwSignal<Option<FilterPopoverState>>,
    /// The active column's distinct values (`trame-shared-filter`).
    pub filter_values: RwSignal<Vec<String>>,
    /// The sheet tabs, in strip order (`trame-sheets`). The active sheet, the
    /// filter flag and the find counter are the view's to publish: it keeps
    /// each of them (`UI-THREAD-003`).
    pub sheets: RwSignal<Vec<SheetTab>>,
    /// The aggregates of a multi-cell selection (`trame-status-bar`).
    pub sel_stats: RwSignal<Option<SelStats>>,
}

impl TrameRelay {
    /// The command `intent` becomes. The Logic thread resolves the geometry
    /// from the selection it tracks, and answers with the events above.
    pub fn command(&self, intent: TrameIntent) -> AppCommand {
        AppCommand::Trame(match intent {
            TrameIntent::Style(patch) => TrameCommand::SetCellStyle { patch },
            TrameIntent::Filter(kind) => TrameCommand::ApplyFilterToActiveColumn { kind },
            // The dialog's snapshotted bounds only label its column picker, so
            // the keys are range-relative, as the Logic thread reads them.
            TrameIntent::Sort(req) => TrameCommand::SortSelectionMulti {
                sort_keys: req.sort_keys,
                header: req.header,
            },
            TrameIntent::Find(query) => TrameCommand::FindNext { query },
            TrameIntent::Replace(r) => TrameCommand::ReplaceOne {
                query: Some(r.query),
                replacement: r.replacement,
            },
            TrameIntent::ReplaceAll(r) => TrameCommand::ReplaceAll {
                query: Some(r.query),
                replacement: r.replacement,
            },
            TrameIntent::ActivateSheet(sheet_id) => TrameCommand::ActivateSheet { sheet_id },
            // `Sheet` and a number is always a valid name, and the Logic thread
            // makes a taken one free (`TRAME-078`).
            TrameIntent::CreateSheet => TrameCommand::CreateSheet {
                name: SheetName::new(&format!("Sheet{}", self.sheets.get_untracked().len() + 1))
                    .unwrap_or_default(),
            },
            TrameIntent::RenameSheet(sheet_id, name) => {
                TrameCommand::RenameSheet { sheet_id, name }
            }
            TrameIntent::DeleteSheet(sheet_id) => TrameCommand::DeleteSheet { sheet_id },
            TrameIntent::ReorderSheet(req) => TrameCommand::MoveSheet {
                sheet_id: req.id,
                to_index: req.to_index,
            },
        })
    }

    /// Fill the chrome from `ev`. Any event not Trame's is a no-op.
    pub fn absorb(&self, ev: &UiEvent) {
        let UiEvent::Viewport {
            event: AppEvent::Trame(te),
            ..
        } = ev
        else {
            return;
        };
        match te {
            // A band selection has no extent the canvas knows. This reply to
            // `RequestSortRange` is what opens the dialog.
            TrameEvent::SortRange { r1, c1, r2, c2 } => {
                self.sort_dialog.set(Some(SortDialogState {
                    r1: *r1,
                    c1: *c1,
                    r2: *r2,
                    c2: *c2,
                }));
            }
            // The popover was opened on a provisional column. The reply pins
            // it to the real one.
            TrameEvent::ColumnValues { col, values } => {
                self.filter_values.set(values.clone());
                self.filter_popover.update(|st| {
                    if let Some(st) = st {
                        st.col = *col;
                    }
                });
            }
            // A filter took effect, or was cleared: the popover closes either
            // way. The view publishes the flag.
            TrameEvent::FilterApplied { .. } | TrameEvent::FilterCleared => {
                self.filter_popover.set(None);
            }
            // The view publishes the active sheet it adopts.
            TrameEvent::SheetList { sheets, .. } => self.sheets.set(sheets.clone()),
            // The view clears the band on a single cell. The Logic thread's
            // totals fill it here.
            TrameEvent::SelectionStats { count, numbers } => self.sel_stats.set(Some(SelStats {
                count: *count,
                numbers: *numbers,
            })),
            _ => {}
        }
    }
}
