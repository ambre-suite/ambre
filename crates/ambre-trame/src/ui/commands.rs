// ambre-trame/src/ui/commands.rs
// ============================================================================
// Trame's command registry: one `Command` per affordance, and the pure
// `StaticKey<Commands> -> AppCommand` mapping (`ambre-spec-commands.md`, UI-THREAD-019).
//
// Host-compiled, like `ambre-toile`'s `ui::menu`, and for the same reason. The
// description is what every view derives from: the context menu, the toolbar and
// the command palette. A test must reach it without a browser. The wasm-only
// half (the `MenuEntry` builders, the command store, the toast mirror) stays in
// `chrome::commands`.
//
// The registry carries `StaticKey<Icons>`, a string. `chrome::icons` resolves it to the
// concrete `icondata::Icon`, which keeps `ambre-commands` a leaf with an empty
// dependency table (UI-THREAD-027). It also keeps Trame's icon set out of the
// shared crate.
// ============================================================================

use crate::message::TrameCommand;
use ambre_commands::{Command, Home, KeyBinding, KeyChar, Keymap, Locale, shared};
use ambre_core::{Categories, Commands, Icons, StaticKey};

use crate::ui::grid::CtxScope;

/// Declare one command per line: the constant, its identity, its label, its
/// icon, its default chord, its category and its home region (`CHROME-148`).
///
/// A macro rather than 26 hand-written blocks, because `Command::title` is a
/// plain `fn` pointer. It captures nothing, so every command needs a function
/// item of its own. Generating them from one line each is what makes the single
/// declaration site real. A second label for a command cannot be written
/// without a second line here.
macro_rules! commands {
    ($($name:ident = $id:expr, $label:literal, $icon:expr, $chords:expr, $category:expr, $home:ident;)*) => {
        $(pub const $name: StaticKey<Commands> = $id;)*

        // The generated title functions. Named after the constants, so the
        // module carries the case exemption rather than every call site.
        #[expect(
            non_snake_case,
            reason = "each title function takes the name of its command constant"
        )]
        mod titles {
            use super::Locale;
            // One language ships today and nothing supplies an active locale,
            // so the label is the same for both variants
            // (`ambre-spec-commands.md` section 5).
            $(pub fn $name(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> { ambre_core::Validated::new($label) })*
        }

        /// Every command Trame contributes, described once. The launcher builds
        /// the command store and the keymap from this table. Every view of a
        /// command reads its label and its icon here (`UI-THREAD-019`).
        pub const COMMANDS: &[Command] = &[
            $(Command {
                id: $name,
                title: titles::$name,
                icon: $icon,
                keybindings: $chords,
                category: $category,
                home: Home::$home,
                description: None,
            },)*
        ];
    };
}

const FORMAT: StaticKey<Categories> = StaticKey::<Categories>::new("format");
const HISTORY: StaticKey<Categories> = StaticKey::<Categories>::new("history");
const STRUCTURE: StaticKey<Categories> = StaticKey::<Categories>::new("structure");
const SORT: StaticKey<Categories> = StaticKey::<Categories>::new("sort");
const FILTER: StaticKey<Categories> = StaticKey::<Categories>::new("filter");
const FIND_REPLACE: StaticKey<Categories> = StaticKey::<Categories>::new("find");
const CLIPBOARD: StaticKey<Categories> = StaticKey::<Categories>::new("clipboard");
const INTEROP: StaticKey<Categories> = shared::FILE;
/// The page setup the sheet stores for its PDF export (`TRAME-IO-022`).
const PAGE: StaticKey<Categories> = StaticKey::<Categories>::new("page");
/// The rows and columns that stay on screen while the rest scrolls
/// (`TRAME-UI-029`).
const VIEW: StaticKey<Categories> = StaticKey::<Categories>::new("view");
/// Commands that belong to the application shell rather than the document.
const GENERAL: StaticKey<Categories> = StaticKey::<Categories>::new("general");

// The labels are the ones the chrome shows today: the context-menu entries of
// `chrome::commands` and the toolbar `aria-label`s of `chrome::bars`. Those
// carried "Copy", "Bold" and "Undo" twice over.
//
// The clipboard rows name the shared identities and the shared chords. A copy
// is then the same command in every application and answers the legacy chord
// as well (`UI-THREAD-011`). Undo and Redo take the shared chord lists, which
// is where Ctrl+Y is declared beside Ctrl+Shift+Z.
commands! {
    BOLD = StaticKey::<Commands>::new("trame.format.bold"), "Bold", Some(StaticKey::<Icons>::new("format-bold")), &[KeyBinding::primary(KeyChar::B)], FORMAT, ContextPanel;
    ITALIC = StaticKey::<Commands>::new("trame.format.italic"), "Italic", Some(StaticKey::<Icons>::new("format-italic")), &[KeyBinding::primary(KeyChar::I)], FORMAT, ContextPanel;
    UNDO = StaticKey::<Commands>::new("trame.history.undo"), "Undo", Some(StaticKey::<Icons>::new("undo")), shared::UNDO_CHORDS, HISTORY, AmbreMenu;
    REDO = StaticKey::<Commands>::new("trame.history.redo"), "Redo", Some(StaticKey::<Icons>::new("redo")), shared::REDO_CHORDS, HISTORY, AmbreMenu;
    INSERT_ROW_ABOVE = StaticKey::<Commands>::new("trame.row.insert_above"), "Insert row above", Some(StaticKey::<Icons>::new("table-row-plus-before")), &[], STRUCTURE, ContextPanel;
    INSERT_ROW_BELOW = StaticKey::<Commands>::new("trame.row.insert_below"), "Insert row below", Some(StaticKey::<Icons>::new("table-row-plus-after")), &[], STRUCTURE, ContextPanel;
    INSERT_COL_LEFT = StaticKey::<Commands>::new("trame.col.insert_left"), "Insert column left", Some(StaticKey::<Icons>::new("table-column-plus-before")), &[], STRUCTURE, ContextPanel;
    INSERT_COL_RIGHT = StaticKey::<Commands>::new("trame.col.insert_right"), "Insert column right", Some(StaticKey::<Icons>::new("table-column-plus-after")), &[], STRUCTURE, ContextPanel;
    DELETE_ROWS = StaticKey::<Commands>::new("trame.row.delete"), "Delete rows", Some(StaticKey::<Icons>::new("table-row-remove")), &[], STRUCTURE, ContextPanel;
    DELETE_COLS = StaticKey::<Commands>::new("trame.col.delete"), "Delete columns", Some(StaticKey::<Icons>::new("table-column-remove")), &[], STRUCTURE, ContextPanel;
    MERGE = StaticKey::<Commands>::new("trame.merge"), "Merge cells", Some(StaticKey::<Icons>::new("table-merge-cells")), &[], STRUCTURE, ContextPanel;
    UNMERGE = StaticKey::<Commands>::new("trame.unmerge"), "Unmerge cells", Some(StaticKey::<Icons>::new("table-split-cell")), &[], STRUCTURE, ContextPanel;
    SORT_RANGE_ASC = StaticKey::<Commands>::new("trame.sort.range_asc"), "Sort range ascending", Some(StaticKey::<Icons>::new("sort-ascending")), &[], SORT, ContextPanel;
    SORT_RANGE_DESC = StaticKey::<Commands>::new("trame.sort.range_desc"), "Sort range descending", Some(StaticKey::<Icons>::new("sort-descending")), &[], SORT, ContextPanel;
    SORT_SHEET_ASC = StaticKey::<Commands>::new("trame.sort.sheet_asc"), "Sort sheet by column (asc)", Some(StaticKey::<Icons>::new("sort")), &[], SORT, ContextPanel;
    SORT_SHEET_DESC = StaticKey::<Commands>::new("trame.sort.sheet_desc"), "Sort sheet by column (desc)", Some(StaticKey::<Icons>::new("sort")), &[], SORT, ContextPanel;
    SORT_RANGE_MULTI = StaticKey::<Commands>::new("trame.sort.range_multi"), "Sort range...", Some(StaticKey::<Icons>::new("sort-variant")), &[], SORT, ContextPanel;
    FILTER_COLUMN = StaticKey::<Commands>::new("trame.filter.column"), "Filter by column", Some(StaticKey::<Icons>::new("filter")), &[], FILTER, ContextPanel;
    CLEAR_FILTER = StaticKey::<Commands>::new("trame.filter.clear"), "Remove filter", Some(StaticKey::<Icons>::new("filter-off")), &[], FILTER, ContextPanel;
    FIND = StaticKey::<Commands>::new("trame.find.open"), "Find and replace", Some(StaticKey::<Icons>::new("search")), &[KeyBinding::primary(KeyChar::F)], FIND_REPLACE, AmbreMenu;
    REPLACE = StaticKey::<Commands>::new("trame.find.replace"), "Replace", None, &[KeyBinding::primary(KeyChar::H)], FIND_REPLACE, AmbreMenu;
    COPY = shared::COPY, "Copy", Some(StaticKey::<Icons>::new("content-copy")), shared::COPY_CHORDS, CLIPBOARD, ContextMenu;
    CUT = shared::CUT, "Cut", Some(StaticKey::<Icons>::new("content-cut")), shared::CUT_CHORDS, CLIPBOARD, ContextMenu;
    PASTE = shared::PASTE, "Paste", Some(StaticKey::<Icons>::new("content-paste")), shared::PASTE_CHORDS, CLIPBOARD, ContextMenu;
    IMPORT_CSV = StaticKey::<Commands>::new("trame.interop.import_csv"), "Import CSV", Some(StaticKey::<Icons>::new("import")), &[], INTEROP, AmbreMenu;
    EXPORT_CSV = StaticKey::<Commands>::new("trame.interop.export_csv"), "Export CSV", Some(StaticKey::<Icons>::new("export")), &[], INTEROP, AmbreMenu;
    IMPORT_XLSX = StaticKey::<Commands>::new("trame.interop.import_xlsx"), "Import Excel workbook", Some(StaticKey::<Icons>::new("import")), &[], INTEROP, AmbreMenu;
    EXPORT_XLSX = StaticKey::<Commands>::new("trame.interop.export_xlsx"), "Export Excel workbook", Some(StaticKey::<Icons>::new("export")), &[], INTEROP, AmbreMenu;
    IMPORT_ODS = StaticKey::<Commands>::new("trame.interop.import_ods"), "Import OpenDocument spreadsheet", Some(StaticKey::<Icons>::new("import")), &[], INTEROP, AmbreMenu;
    EXPORT_ODS = StaticKey::<Commands>::new("trame.interop.export_ods"), "Export OpenDocument spreadsheet", Some(StaticKey::<Icons>::new("export")), &[], INTEROP, AmbreMenu;
    EXPORT_PDF = StaticKey::<Commands>::new("trame.interop.export_pdf"), "Export PDF", Some(StaticKey::<Icons>::new("export")), &[], INTEROP, AmbreMenu;
    EXPORT_PDF_SELECTION = StaticKey::<Commands>::new("trame.interop.export_pdf_selection"), "Export selection as PDF", Some(StaticKey::<Icons>::new("export")), &[], INTEROP, AmbreMenu;
    PAGE_PORTRAIT = StaticKey::<Commands>::new("trame.page.portrait"), "Page setup: portrait", None, &[], PAGE, ContextPanel;
    PAGE_LANDSCAPE = StaticKey::<Commands>::new("trame.page.landscape"), "Page setup: landscape", None, &[], PAGE, ContextPanel;
    PAPER_A4 = StaticKey::<Commands>::new("trame.page.paper_a4"), "Page setup: A4 paper", None, &[], PAGE, ContextPanel;
    PAPER_A3 = StaticKey::<Commands>::new("trame.page.paper_a3"), "Page setup: A3 paper", None, &[], PAGE, ContextPanel;
    PAPER_LETTER = StaticKey::<Commands>::new("trame.page.paper_letter"), "Page setup: Letter paper", None, &[], PAGE, ContextPanel;
    PAPER_LEGAL = StaticKey::<Commands>::new("trame.page.paper_legal"), "Page setup: Legal paper", None, &[], PAGE, ContextPanel;
    MARGINS_NORMAL = StaticKey::<Commands>::new("trame.page.margins_normal"), "Page setup: normal margins", None, &[], PAGE, ContextPanel;
    MARGINS_NARROW = StaticKey::<Commands>::new("trame.page.margins_narrow"), "Page setup: narrow margins", None, &[], PAGE, ContextPanel;
    MARGINS_WIDE = StaticKey::<Commands>::new("trame.page.margins_wide"), "Page setup: wide margins", None, &[], PAGE, ContextPanel;
    SAVE_FILE = StaticKey::<Commands>::new("trame.interop.save_file"), "Save to a file (.ambre, not encrypted)", Some(StaticKey::<Icons>::new("content-save")), &[], INTEROP, AmbreMenu;
    SAVE_SEALED_FILE = StaticKey::<Commands>::new("trame.interop.save_sealed_file"), "Save to an encrypted file (.ambre)...", Some(StaticKey::<Icons>::new("lock-outline")), &[], INTEROP, AmbreMenu;
    FREEZE_ROWS = StaticKey::<Commands>::new("trame.view.freeze_rows"), "Freeze up to this row", None, &[], VIEW, ContextPanel;
    FREEZE_COLS = StaticKey::<Commands>::new("trame.view.freeze_cols"), "Freeze up to this column", None, &[], VIEW, ContextPanel;
    UNFREEZE = StaticKey::<Commands>::new("trame.view.unfreeze"), "Unfreeze rows and columns", None, &[], VIEW, ContextPanel;
    ZOOM_TO_SELECTION = StaticKey::<Commands>::new("trame.view.zoom_to_selection"), "Zoom to selection", None, &[], VIEW, StatusBar;
    PALETTE = StaticKey::<Commands>::new("trame.palette.open"), "Command palette", None, &[KeyBinding::primary(KeyChar::K)], GENERAL, AmbreMenu;
}

/// The label a view shows for `id`, in `locale`. `None` for a command Trame
/// does not declare.
///
/// The one reader of a command's title. A view that wants a label calls this
/// and never writes one of its own (`UI-THREAD-019`).
pub fn title(
    id: StaticKey<Commands>,
    locale: &Locale,
) -> Option<ambre_core::Validated<ambre_commands::CommandTitle>> {
    ambre_commands::find(COMMANDS, id).map(|c| (c.title)(locale))
}

/// The icon handle `id` declares, if it has one. The chrome resolves it to the
/// asset (`chrome::icons`).
pub fn icon(id: StaticKey<Commands>) -> Option<StaticKey<Icons>> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.icon)
}

/// Bind Trame's chords into the launcher's keymap.
///
/// Every chord comes from the table, so a chord is declared beside the command
/// it triggers and nowhere else. The clipboard chords are the exception the
/// shared crate holds: the table declares them so a surface announces them.
/// `Keymap::bind_table` leaves them out because the browser raises those events
/// itself (`ambre-spec-commands.md`, `UI-THREAD-011`).
///
/// Host-compiled, beside the table it reads, so a test resolves a chord without
/// a browser.
pub fn bind_keys(keymap: &mut Keymap) {
    keymap.bind_table(COMMANDS);
}

// -- Context-menu layouts (`trame-context-menu`) -------------------------------
//
// A layout is a list of identities and separators. The label and the icon of an
// entry come from the table above (`UI-THREAD-019`). A layout declares an order
// and a grouping, never a word the user reads. `chrome::commands` turns a row
// into the `MenuEntry` the DOM menu renders; the layout stays here, where a host
// test reaches it.

/// One row of a context-menu layout: a command, or a group separator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    /// A selectable command, shown with the label and icon it declares.
    Command(StaticKey<Commands>),
    /// A divider between two groups.
    Separator,
}

/// Structural ops of the full cell menu.
const STRUCTURAL_GROUP: &[StaticKey<Commands>] = &[
    INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW,
    INSERT_COL_LEFT,
    INSERT_COL_RIGHT,
    DELETE_ROWS,
    DELETE_COLS,
];

/// Column-only structural ops, for the column-header menu.
const COLUMN_STRUCTURAL_GROUP: &[StaticKey<Commands>] =
    &[INSERT_COL_LEFT, INSERT_COL_RIGHT, DELETE_COLS];

/// Row-only structural ops, for the row-header menu.
const ROW_STRUCTURAL_GROUP: &[StaticKey<Commands>] =
    &[INSERT_ROW_ABOVE, INSERT_ROW_BELOW, DELETE_ROWS];

/// Sort group, shared by the cell and the header menus (`trame-sort`).
///
/// Single-column shared sort over the selection: the reference column is the
/// selection's first column, and the Logic thread no-ops an empty/single-row
/// range. Also the whole-sheet sort by the active column: a whole-column
/// selection sorts the full used range so rows stay aligned.
const SORT_GROUP: &[StaticKey<Commands>] = &[
    SORT_RANGE_ASC,
    SORT_RANGE_DESC,
    SORT_SHEET_ASC,
    SORT_SHEET_DESC,
    SORT_RANGE_MULTI,
];

/// Clipboard group (`trame-clipboard`): cell-only, driven by the launcher.
const CLIPBOARD_GROUP: &[StaticKey<Commands>] = &[CUT, COPY, PASTE];

/// Merge group (`trame-cell-merges`): cell-only. Always listed, and the Logic
/// thread guards it. A single-cell Merge is a no-op, a covered cell replies
/// MergeRejected -> toast, and Unmerge outside a merge is a no-op.
const MERGE_GROUP: &[StaticKey<Commands>] = &[MERGE, UNMERGE];

/// Frozen pane group (`TRAME-UI-029`): freeze the band through the active
/// line, or freeze nothing.
const FREEZE_GROUP: &[StaticKey<Commands>] = &[FREEZE_ROWS, FREEZE_COLS, UNFREEZE];

/// Whole-sheet sort, for the corner menu.
const CORNER_SORT_GROUP: &[StaticKey<Commands>] = &[SORT_SHEET_ASC, SORT_SHEET_DESC];

/// Whether the active sheet is filtered (`trame-shared-filter`). The context
/// menu offers "Clear filter" only on a filtered sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SheetFilter {
    #[default]
    Unfiltered,
    Filtered,
}

impl SheetFilter {
    pub fn is_filtered(self) -> bool {
        self == Self::Filtered
    }
}

/// The shared-filter entry (`trame-shared-filter`).
///
/// Truly contextual: a single entry that toggles meaning with the active
/// sheet's filter state. With no filter it offers "Filter by column" (opens the
/// popover); with a filter active it offers "Remove filter" (drops it). Never
/// both -- the user complained the old menu always listed a clear-filter that
/// did nothing.
pub fn filter_command(filter: SheetFilter) -> StaticKey<Commands> {
    match filter {
        SheetFilter::Filtered => CLEAR_FILTER,
        SheetFilter::Unfiltered => FILTER_COLUMN,
    }
}

/// The right-click context-menu layout Trame contributes, grouped and
/// separated (`trame-context-menu`). `scope` selects it. The cell menu carries
/// clipboard + full structural + sort/filter + merge. The column- or
/// row-header menu carries axis-scoped structural + sort/filter, with no
/// clipboard or merge. The corner menu carries whole-sheet sort + filter.
/// `filter` drives the dynamic [`filter_command`] (Filter by column XOR
/// Remove filter).
pub fn context_layout(scope: CtxScope, filter: SheetFilter) -> Vec<MenuRow> {
    /// Append one group of commands to a layout under construction.
    fn group(rows: &mut Vec<MenuRow>, ids: &[StaticKey<Commands>]) {
        rows.extend(ids.iter().map(|&id| MenuRow::Command(id)));
    }

    let mut rows = Vec::new();
    match scope {
        CtxScope::Cell => {
            group(&mut rows, CLIPBOARD_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            group(&mut rows, &[filter_command(filter)]);
            rows.push(MenuRow::Separator);
            group(&mut rows, MERGE_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, FREEZE_GROUP);
        }
        CtxScope::Column => {
            group(&mut rows, COLUMN_STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            group(&mut rows, &[filter_command(filter)]);
            rows.push(MenuRow::Separator);
            group(&mut rows, &[FREEZE_COLS, UNFREEZE]);
        }
        CtxScope::Row => {
            group(&mut rows, ROW_STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            // Filtering is column-oriented, so a row menu only ever offers to
            // drop an active filter (never to create one from a row).
            if filter.is_filtered() {
                group(&mut rows, &[CLEAR_FILTER]);
            }
            rows.push(MenuRow::Separator);
            group(&mut rows, &[FREEZE_ROWS, UNFREEZE]);
        }
        CtxScope::Corner => {
            // The corner selects the whole sheet: only whole-sheet ops apply.
            group(&mut rows, CORNER_SORT_GROUP);
            if filter.is_filtered() {
                rows.push(MenuRow::Separator);
                group(&mut rows, &[CLEAR_FILTER]);
            }
        }
    }
    rows
}

/// Translate a Trame `StaticKey<Commands>` into the `TrameCommand` the Logic
/// thread expects. `None` for a command Trame does not own, and for one
/// `chrome::TrameHandler` resolves before the bus.
pub fn command_to_app(id: StaticKey<Commands>) -> Option<TrameCommand> {
    use crate::message::{FreezeIntent, PageMargins, PageOrientation, PageSetupPatch, PaperSize};
    let page = |patch: PageSetupPatch| TrameCommand::SetPageSetup {
        sheet_id: None,
        patch,
    };
    let paper = |size: PaperSize| {
        page(PageSetupPatch {
            paper: Some(size),
            ..PageSetupPatch::default()
        })
    };
    let margins = |bands: PageMargins| {
        page(PageSetupPatch {
            margins: Some(bands),
            ..PageSetupPatch::default()
        })
    };
    // The identities are constants of a derived `PartialEq` type, so they are
    // patterns. That means one decision over the identity string, and no
    // literal repeated beside the table that declares it.
    let cmd = match id {
        UNDO => TrameCommand::Undo,
        REDO => TrameCommand::Redo,
        INSERT_ROW_ABOVE => TrameCommand::InsertRowAbove,
        INSERT_ROW_BELOW => TrameCommand::InsertRowBelow,
        INSERT_COL_LEFT => TrameCommand::InsertColLeft,
        INSERT_COL_RIGHT => TrameCommand::InsertColRight,
        DELETE_ROWS => TrameCommand::DeleteSelectedRows,
        DELETE_COLS => TrameCommand::DeleteSelectedCols,
        MERGE => TrameCommand::MergeSelection,
        UNMERGE => TrameCommand::UnmergeActive,
        SORT_RANGE_ASC => TrameCommand::SortSelectionAsc,
        SORT_RANGE_DESC => TrameCommand::SortSelectionDesc,
        SORT_SHEET_ASC => TrameCommand::SortSheetAsc,
        SORT_SHEET_DESC => TrameCommand::SortSheetDesc,
        CLEAR_FILTER => TrameCommand::ClearActiveFilter,
        FREEZE_ROWS => TrameCommand::Freeze(FreezeIntent::RowsThroughActive),
        FREEZE_COLS => TrameCommand::Freeze(FreezeIntent::ColsThroughActive),
        UNFREEZE => TrameCommand::Freeze(FreezeIntent::Clear),
        // Parameterless: the Logic thread serializes the active sheet itself.
        EXPORT_CSV => TrameCommand::export(crate::message::TrameExport::Csv {
            sheet_id: None,
            delimiter: None,
            range: None,
        }),
        // The whole workbook, every sheet, as Excel (`trame-import-export`).
        EXPORT_XLSX => TrameCommand::export(crate::message::TrameExport::Xlsx),
        // The whole workbook as an OpenDocument spreadsheet, as the Excel
        // export writes it (`trame-import-export`).
        EXPORT_ODS => TrameCommand::export(crate::message::TrameExport::Ods),
        // The active sheet as pages, each column band fitted to the paper
        // and headed by the first row (`features/ambre-spec-trame-pdf-export.md`).
        EXPORT_PDF => TrameCommand::export(crate::message::TrameExport::Pdf(
            crate::message::PdfExport {
                range: crate::message::PrintRange::UsedRange,
                fit: crate::message::PrintFit::Width,
                title_row: crate::message::TitleRow::Repeated,
                page: None,
            },
        )),
        // The same, over the live selection alone.
        EXPORT_PDF_SELECTION => TrameCommand::export(crate::message::TrameExport::Pdf(
            crate::message::PdfExport {
                range: crate::message::PrintRange::Selection,
                fit: crate::message::PrintFit::Width,
                title_row: crate::message::TitleRow::Repeated,
                page: None,
            },
        )),
        // The page setup of the active sheet, one field at a time, so two
        // peers changing two fields keep both (`TRAME-IO-086`).
        PAGE_PORTRAIT => page(PageSetupPatch {
            orientation: Some(PageOrientation::Portrait),
            ..PageSetupPatch::default()
        }),
        PAGE_LANDSCAPE => page(PageSetupPatch {
            orientation: Some(PageOrientation::Landscape),
            ..PageSetupPatch::default()
        }),
        PAPER_A4 => paper(PaperSize::A4),
        PAPER_A3 => paper(PaperSize::A3),
        PAPER_LETTER => paper(PaperSize::Letter),
        PAPER_LEGAL => paper(PaperSize::Legal),
        MARGINS_NORMAL => margins(PageMargins::NORMAL),
        MARGINS_NARROW => margins(PageMargins::NARROW),
        MARGINS_WIDE => margins(PageMargins::WIDE),
        // The native file, which drops nothing (`DOCFILE-001`). Also
        // parameterless, and also served by the Logic thread -- there because
        // the container's header carries the document's identity.
        SAVE_FILE => TrameCommand::export(crate::message::TrameExport::Ambre),
        // BOLD / ITALIC (style toggles), FILTER_COLUMN, FIND / REPLACE and the
        // clipboard commands are UI-only intents the handler resolves first.
        // BOLD/ITALIC become a `SetCellStyle` patch toggled against the tracked
        // style state, FILTER_COLUMN opens the popover, and FIND/REPLACE open
        // the find/replace panel. None has a direct command here.
        _ => return None,
    };
    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is the single declaration site, so two commands that share
    /// an identity would make one of them unreachable through `find`.
    #[test]
    fn every_identity_is_declared_once() {
        for (i, cmd) in COMMANDS.iter().enumerate() {
            let twin = COMMANDS.iter().skip(i + 1).find(|c| c.id == cmd.id);
            assert!(twin.is_none(), "{:?} is declared twice", cmd.id);
        }
    }

    /// Each command is described once, with a title in every locale, a
    /// group, no blank icon and no chord another command holds
    /// (`UI-THREAD-019`, `UI-THREAD-037`).
    #[test]
    fn the_table_has_no_defect() {
        assert_eq!(ambre_commands::table_defects(COMMANDS), []);
    }

    /// A command carries its own label, and a view reads it from here. An empty
    /// title is a command no menu and no palette can show.
    #[test]
    fn every_command_has_a_label() {
        for cmd in COMMANDS {
            let label = (cmd.title)(&Locale::En);
            assert!(!label.as_str().is_empty(), "{:?} has no label", cmd.id);
            assert_eq!(title(cmd.id, &Locale::En), Some(label));
        }
    }

    /// The chords the table declares must not collide: a chord resolves to at
    /// most one command (`UI-THREAD-026`). The keymap keeps the last seen, so
    /// a collision here would silently drop a command's default.
    #[test]
    fn declared_chords_are_distinct() {
        let chords: Vec<KeyBinding> = COMMANDS
            .iter()
            .flat_map(|c| c.keybindings.iter().copied())
            .collect();
        for (i, chord) in chords.iter().enumerate() {
            assert!(
                !chords[i + 1..].contains(chord),
                "{chord:?} is declared twice"
            );
        }
    }

    /// Undo takes one chord and Redo takes two, Ctrl+Shift+Z and Ctrl+Y
    /// (`UNDO-011`). The test resolves them through the keymap the launcher
    /// builds, so a chord dropped from the table fails here.
    #[test]
    fn the_keymap_binds_both_redo_chords() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        assert_eq!(keymap.resolve(KeyBinding::primary(KeyChar::Z)), Some(UNDO));
        assert_eq!(
            keymap.resolve(KeyBinding::primary_shift(KeyChar::Z)),
            Some(REDO)
        );
        assert_eq!(keymap.resolve(KeyBinding::primary(KeyChar::Y)), Some(REDO));
    }

    /// The three clipboard intents are described under the shared identities
    /// and announce the shared chords, the modern one and the legacy one. A
    /// copy is therefore one command in every application (`UI-THREAD-011`).
    #[test]
    fn the_clipboard_intents_carry_the_shared_identity() {
        for (id, chords) in [
            (shared::CUT, shared::CUT_CHORDS),
            (shared::COPY, shared::COPY_CHORDS),
            (shared::PASTE, shared::PASTE_CHORDS),
        ] {
            let cmd = ambre_commands::find(COMMANDS, id).expect("described");
            assert_eq!(cmd.keybindings, chords, "{id:?} announces both chords");
        }
        // The identities Trame's surfaces name are those same three.
        assert_eq!(
            [CUT, COPY, PASTE],
            [shared::CUT, shared::COPY, shared::PASTE]
        );
    }

    /// The four clipboard chords reach no command: the browser raises `cut`,
    /// `copy` and `paste` itself, on the modern and the legacy chords alike. A
    /// second decoder here would double-handle them (`UI-THREAD-011`).
    #[test]
    fn the_keymap_binds_no_clipboard_chord() {
        let mut keymap = Keymap::new();
        bind_keys(&mut keymap);
        for chord in shared::CUT_CHORDS
            .iter()
            .chain(shared::COPY_CHORDS)
            .chain(shared::PASTE_CHORDS)
        {
            assert_eq!(
                keymap.resolve(*chord),
                None,
                "{chord} must reach no command"
            );
        }
    }

    /// Every row of every layout resolves in the table, so a menu never shows
    /// an entry the registry cannot label (`UI-THREAD-019`).
    #[test]
    fn every_layout_row_resolves_in_the_registry() {
        let scopes = [
            CtxScope::Cell,
            CtxScope::Column,
            CtxScope::Row,
            CtxScope::Corner,
        ];
        for scope in scopes {
            for filter in [SheetFilter::Unfiltered, SheetFilter::Filtered] {
                for row in context_layout(scope, filter) {
                    let MenuRow::Command(id) = row else { continue };
                    assert!(
                        title(id, &Locale::En).is_some(),
                        "{scope:?} lists {id:?}, which the registry does not declare"
                    );
                }
            }
        }
    }

    /// The filter entry offers one command, never both.
    #[test]
    fn the_filter_entry_switches_with_the_filter_state() {
        assert_eq!(filter_command(SheetFilter::Unfiltered), FILTER_COLUMN);
        assert_eq!(filter_command(SheetFilter::Filtered), CLEAR_FILTER);
        let cell = context_layout(CtxScope::Cell, SheetFilter::Filtered);
        assert!(cell.contains(&MenuRow::Command(CLEAR_FILTER)));
        assert!(!cell.contains(&MenuRow::Command(FILTER_COLUMN)));
    }

    /// `DOCFILE-009`: the surface that offers the export says the file carries
    /// no encryption of its own. The label is where it says it, the one string a
    /// user reads before clicking. So the rule is asserted on the label rather
    /// than on a comment about a tooltip.
    #[test]
    fn the_save_label_says_the_file_is_not_encrypted() {
        let label = title(SAVE_FILE, &Locale::En).expect("SAVE_FILE is declared");
        assert!(
            label.as_str().contains("not encrypted"),
            "the export surface must state that the file is plaintext: {label}"
        );
    }

    #[test]
    fn command_to_app_maps_the_logic_thread_commands() {
        // `TrameCommand` carries no `PartialEq`, so the variant is matched.
        assert!(matches!(command_to_app(UNDO), Some(TrameCommand::Undo)));
        assert!(matches!(
            command_to_app(DELETE_ROWS),
            Some(TrameCommand::DeleteSelectedRows)
        ));
        assert!(matches!(
            command_to_app(SAVE_FILE),
            Some(TrameCommand::Interop(
                ambre_bus::interop::InteropRequest::Export(crate::message::TrameExport::Ambre)
            ))
        ));
        // A UI-only intent the launcher intercepts: no bus payload.
        assert!(command_to_app(BOLD).is_none());
        assert!(command_to_app(StaticKey::<Commands>::new("trame.unknown")).is_none());
    }

    /// `TRAME-IO-086`: each page setup entry of the palette writes one field
    /// of the active sheet's setup, and leaves the other two alone.
    #[test]
    fn each_page_entry_patches_one_field_of_the_active_sheet() {
        use crate::message::{PageOrientation, PageSetupPatch, PaperSize};
        let patch = |id| match command_to_app(id) {
            Some(TrameCommand::SetPageSetup {
                sheet_id: None,
                patch,
            }) => patch,
            _ => panic!("{id:?} names no page setup of the active sheet"),
        };
        assert_eq!(
            patch(PAGE_LANDSCAPE),
            PageSetupPatch {
                orientation: Some(PageOrientation::Landscape),
                ..PageSetupPatch::default()
            }
        );
        assert_eq!(
            patch(PAPER_A3),
            PageSetupPatch {
                paper: Some(PaperSize::A3),
                ..PageSetupPatch::default()
            }
        );
        for id in [
            PAGE_PORTRAIT,
            PAPER_A4,
            PAPER_LETTER,
            PAPER_LEGAL,
            MARGINS_NORMAL,
            MARGINS_NARROW,
            MARGINS_WIDE,
        ] {
            let p = patch(id);
            let set = [
                p.paper.is_some(),
                p.orientation.is_some(),
                p.margins.is_some(),
            ];
            assert_eq!(set.iter().filter(|s| **s).count(), 1, "{id:?}");
        }
    }

    /// A row menu never offers to filter a row (`TRAME-UI-090`). With a filter
    /// active it offers only to remove it.
    #[test]
    fn a_row_menu_never_offers_to_filter() {
        for filter in [SheetFilter::Unfiltered, SheetFilter::Filtered] {
            let rows = context_layout(CtxScope::Row, filter);
            assert!(!rows.contains(&MenuRow::Command(FILTER_COLUMN)), "{rows:?}");
            assert_eq!(
                rows.contains(&MenuRow::Command(CLEAR_FILTER)),
                filter.is_filtered()
            );
        }
        // The column menu, by contrast, offers it.
        let cols = context_layout(CtxScope::Column, SheetFilter::Unfiltered);
        assert!(cols.contains(&MenuRow::Command(FILTER_COLUMN)));
    }
}
