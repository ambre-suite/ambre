// ambre-trame/src/ui/commands.rs
// ============================================================================
// Trame's command registry: one `Command` per affordance, and the pure
// `CommandId -> AppCommand` mapping (`ambre-spec-commands.md`, UI-THREAD-019).
//
// Host-compiled, like `ambre-toile`'s `ui::menu`, and for the same reason: the
// description is what every view derives from -- the context menu, the toolbar
// and the command palette -- so a test must reach it without a browser. The
// wasm-only half (the `MenuEntry` builders, the command store, the toast
// mirror) stays in `chrome::commands`.
//
// The registry carries `IconId`, a string. `chrome::icons` resolves it to the
// concrete `icondata::Icon`, which keeps `ambre-commands` a leaf with an empty
// dependency table (UI-THREAD-027) and keeps Trame's icon set out of the
// shared crate.
// ============================================================================

use ambre_bus::{AppCommand, TrameCommand};
use ambre_commands::{Command, CommandCategory, CommandId, IconId, KeyBinding, Locale};

use crate::ui::grid::CtxScope;

/// Declare one command per line: the constant, its identity, its label, its
/// icon, its default chord and its category.
///
/// A macro rather than 26 hand-written blocks, because `Command::title` is a
/// plain `fn` pointer: it captures nothing, so every command needs a function
/// item of its own. Generating them from one line each is what makes the single
/// declaration site real -- a second label for a command cannot be written
/// without a second line here.
macro_rules! commands {
    ($($name:ident = $id:literal, $label:literal, $icon:expr, $chord:expr, $category:expr;)*) => {
        $(pub const $name: CommandId = CommandId($id);)*

        // The generated title functions. Named after the constants, so the
        // module carries the case exemption rather than every call site.
        #[allow(non_snake_case)]
        mod titles {
            use super::Locale;
            // One language ships today and nothing supplies an active locale,
            // so the label is the same for both variants
            // (`ambre-spec-commands.md` section 5).
            $(pub fn $name(_: &Locale) -> String { $label.into() })*
        }

        /// Every command Trame contributes, described once. The launcher builds
        /// the command store and the keymap from this table, and every view of a
        /// command reads its label and its icon here (`UI-THREAD-019`).
        pub const COMMANDS: &[Command] = &[
            $(Command {
                id: $name,
                title: titles::$name,
                icon: $icon,
                default_keybinding: $chord,
                category: $category,
            },)*
        ];
    };
}

const FORMAT: CommandCategory = CommandCategory("format");
const HISTORY: CommandCategory = CommandCategory("history");
const STRUCTURE: CommandCategory = CommandCategory("structure");
const SORT: CommandCategory = CommandCategory("sort");
const FILTER: CommandCategory = CommandCategory("filter");
const FIND_REPLACE: CommandCategory = CommandCategory("find");
const CLIPBOARD: CommandCategory = CommandCategory("clipboard");
const INTEROP: CommandCategory = CommandCategory("interop");

// The labels are the ones the chrome shows today: the context-menu entries of
// `chrome::commands` and the toolbar `aria-label`s of `chrome::bars`, which
// carried "Copy", "Bold" and "Undo" twice over.
//
// Redo answers two chords, Ctrl+Shift+Z and Ctrl+Y (the Excel and Sheets
// convention). A description holds one default chord, so the table carries the
// first and `chrome::commands::bind_keys` still binds the alias.
commands! {
    BOLD = "trame.format.bold", "Bold", Some(IconId("format-bold")), Some(KeyBinding::ctrl('b')), FORMAT;
    ITALIC = "trame.format.italic", "Italic", Some(IconId("format-italic")), Some(KeyBinding::ctrl('i')), FORMAT;
    UNDO = "trame.history.undo", "Undo", Some(IconId("undo")), Some(KeyBinding::ctrl('z')), HISTORY;
    REDO = "trame.history.redo", "Redo", Some(IconId("redo")), Some(KeyBinding::ctrl_shift('z')), HISTORY;
    INSERT_ROW_ABOVE = "trame.row.insert_above", "Insert row above", Some(IconId("table-row-plus-before")), None, STRUCTURE;
    INSERT_ROW_BELOW = "trame.row.insert_below", "Insert row below", Some(IconId("table-row-plus-after")), None, STRUCTURE;
    INSERT_COL_LEFT = "trame.col.insert_left", "Insert column left", Some(IconId("table-column-plus-before")), None, STRUCTURE;
    INSERT_COL_RIGHT = "trame.col.insert_right", "Insert column right", Some(IconId("table-column-plus-after")), None, STRUCTURE;
    DELETE_ROWS = "trame.row.delete", "Delete rows", Some(IconId("table-row-remove")), None, STRUCTURE;
    DELETE_COLS = "trame.col.delete", "Delete columns", Some(IconId("table-column-remove")), None, STRUCTURE;
    MERGE = "trame.merge", "Merge cells", Some(IconId("table-merge-cells")), None, STRUCTURE;
    UNMERGE = "trame.unmerge", "Unmerge cells", Some(IconId("table-split-cell")), None, STRUCTURE;
    SORT_RANGE_ASC = "trame.sort.range_asc", "Sort range ascending", Some(IconId("sort-ascending")), None, SORT;
    SORT_RANGE_DESC = "trame.sort.range_desc", "Sort range descending", Some(IconId("sort-descending")), None, SORT;
    SORT_SHEET_ASC = "trame.sort.sheet_asc", "Sort sheet by column (asc)", Some(IconId("sort")), None, SORT;
    SORT_SHEET_DESC = "trame.sort.sheet_desc", "Sort sheet by column (desc)", Some(IconId("sort")), None, SORT;
    SORT_RANGE_MULTI = "trame.sort.range_multi", "Sort range...", Some(IconId("sort-variant")), None, SORT;
    FILTER_COLUMN = "trame.filter.column", "Filter by column", Some(IconId("filter")), None, FILTER;
    CLEAR_FILTER = "trame.filter.clear", "Remove filter", Some(IconId("filter-off")), None, FILTER;
    FIND = "trame.find.open", "Find and replace", Some(IconId("search")), Some(KeyBinding::ctrl('f')), FIND_REPLACE;
    REPLACE = "trame.find.replace", "Replace", None, Some(KeyBinding::ctrl('h')), FIND_REPLACE;
    COPY = "trame.clipboard.copy", "Copy", Some(IconId("content-copy")), None, CLIPBOARD;
    CUT = "trame.clipboard.cut", "Cut", Some(IconId("content-cut")), None, CLIPBOARD;
    PASTE = "trame.clipboard.paste", "Paste", Some(IconId("content-paste")), None, CLIPBOARD;
    IMPORT_CSV = "trame.interop.import_csv", "Import CSV", Some(IconId("import")), None, INTEROP;
    EXPORT_CSV = "trame.interop.export_csv", "Export CSV", Some(IconId("export")), None, INTEROP;
}

/// The label a view shows for `id`, in `locale`. `None` for a command Trame
/// does not declare.
///
/// The one reader of a command's title. A view that wants a label calls this
/// and never writes one of its own (`UI-THREAD-019`).
pub fn title(id: CommandId, locale: &Locale) -> Option<String> {
    ambre_commands::find(COMMANDS, id).map(|c| (c.title)(locale))
}

/// The icon handle `id` declares, if it has one. The chrome resolves it to the
/// asset (`chrome::icons`).
pub fn icon(id: CommandId) -> Option<IconId> {
    ambre_commands::find(COMMANDS, id).and_then(|c| c.icon)
}

// -- Context-menu layouts (`trame-context-menu`) -------------------------------
//
// A layout is a list of identities and separators. The label and the icon of an
// entry come from the table above (`UI-THREAD-019`), so a layout declares an
// order and a grouping, never a word the user reads. `chrome::commands` turns a
// row into the `MenuEntry` the DOM menu renders; the layout stays here, where a
// host test reaches it.

/// One row of a context-menu layout: a command, or a group separator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    /// A selectable command, shown with the label and icon it declares.
    Command(CommandId),
    /// A divider between two groups.
    Separator,
}

/// Structural ops of the full cell menu.
const STRUCTURAL_GROUP: &[CommandId] = &[
    INSERT_ROW_ABOVE,
    INSERT_ROW_BELOW,
    INSERT_COL_LEFT,
    INSERT_COL_RIGHT,
    DELETE_ROWS,
    DELETE_COLS,
];

/// Column-only structural ops, for the column-header menu.
const COLUMN_STRUCTURAL_GROUP: &[CommandId] = &[INSERT_COL_LEFT, INSERT_COL_RIGHT, DELETE_COLS];

/// Row-only structural ops, for the row-header menu.
const ROW_STRUCTURAL_GROUP: &[CommandId] = &[INSERT_ROW_ABOVE, INSERT_ROW_BELOW, DELETE_ROWS];

/// Sort group, shared by the cell and the header menus (`trame-sort`).
///
/// Single-column shared sort over the selection (reference column = the
/// selection's first column; the Logic thread no-ops an empty/single-row range)
/// and the whole-sheet sort by the active column (whole-column selection: sorts
/// the full used range so rows stay aligned).
const SORT_GROUP: &[CommandId] = &[
    SORT_RANGE_ASC,
    SORT_RANGE_DESC,
    SORT_SHEET_ASC,
    SORT_SHEET_DESC,
    SORT_RANGE_MULTI,
];

/// Clipboard group (`trame-clipboard`): cell-only, driven by the launcher.
const CLIPBOARD_GROUP: &[CommandId] = &[CUT, COPY, PASTE];

/// Merge group (`trame-cell-merges`): cell-only. Always listed; the Logic thread
/// guards it (single-cell Merge is a no-op, a covered cell replies
/// MergeRejected -> toast, Unmerge outside a merge is a no-op).
const MERGE_GROUP: &[CommandId] = &[MERGE, UNMERGE];

/// Whole-sheet sort, for the corner menu.
const CORNER_SORT_GROUP: &[CommandId] = &[SORT_SHEET_ASC, SORT_SHEET_DESC];

/// The shared-filter entry (`trame-shared-filter`).
///
/// Truly contextual: a single entry that toggles meaning with the active
/// sheet's filter state. With no filter it offers "Filter by column" (opens the
/// popover); with a filter active it offers "Remove filter" (drops it). Never
/// both -- the user complained the old menu always listed a clear-filter that
/// did nothing.
pub fn filter_command(filter_active: bool) -> CommandId {
    if filter_active {
        CLEAR_FILTER
    } else {
        FILTER_COLUMN
    }
}

/// The right-click context-menu layout Trame contributes, grouped and
/// separated (`trame-context-menu`). `scope` selects it: the cell menu
/// (clipboard + full structural + sort/filter + merge), the column- or
/// row-header menu (axis-scoped structural + sort/filter, no clipboard/merge),
/// or the corner menu (whole-sheet sort + filter). `filter_active` drives the
/// dynamic [`filter_command`] (Filter by column XOR Remove filter).
pub fn context_layout(scope: CtxScope, filter_active: bool) -> Vec<MenuRow> {
    /// Append one group of commands to a layout under construction.
    fn group(rows: &mut Vec<MenuRow>, ids: &[CommandId]) {
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
            group(&mut rows, &[filter_command(filter_active)]);
            rows.push(MenuRow::Separator);
            group(&mut rows, MERGE_GROUP);
        }
        CtxScope::Column => {
            group(&mut rows, COLUMN_STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            group(&mut rows, &[filter_command(filter_active)]);
        }
        CtxScope::Row => {
            group(&mut rows, ROW_STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            // Filtering is column-oriented, so a row menu only ever offers to
            // drop an active filter (never to create one from a row).
            if filter_active {
                group(&mut rows, &[CLEAR_FILTER]);
            }
        }
        CtxScope::Corner => {
            // The corner selects the whole sheet: only whole-sheet ops apply.
            group(&mut rows, CORNER_SORT_GROUP);
            if filter_active {
                rows.push(MenuRow::Separator);
                group(&mut rows, &[CLEAR_FILTER]);
            }
        }
    }
    rows
}

/// Translate a Trame `CommandId` into the `AppCommand` IPC payload the Logic
/// thread expects. `None` for a command Trame does not own (the launcher logs
/// it), and for a command the launcher intercepts before the bus.
pub fn command_to_app(id: CommandId) -> Option<AppCommand> {
    // The identities are constants of a derived `PartialEq` type, so they are
    // patterns: one decision over the identity string, and no literal repeated
    // beside the table that declares it.
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
        // Parameterless: the Logic thread serializes the active sheet itself.
        EXPORT_CSV => TrameCommand::ExportActiveSheetCsv,
        // BOLD / ITALIC (style toggles), FILTER_COLUMN, FIND / REPLACE and the
        // clipboard commands are UI-only intents the launcher intercepts:
        // BOLD/ITALIC become a `SetCellStyle` patch toggled against the tracked
        // style state, FILTER_COLUMN opens the popover, FIND/REPLACE open the
        // find/replace panel. None has a direct AppCommand here.
        _ => return None,
    };
    Some(AppCommand::Trame(cmd))
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

    /// A command carries its own label, and a view reads it from here. An empty
    /// title is a command no menu and no palette can show.
    #[test]
    fn every_command_has_a_label() {
        for cmd in COMMANDS {
            let label = (cmd.title)(&Locale::En);
            assert!(!label.is_empty(), "{:?} has no label", cmd.id);
            assert_eq!(title(cmd.id, &Locale::En), Some(label));
        }
    }

    /// The chords the table declares must not collide: a chord resolves to at
    /// most one command (`UI-THREAD-026`), and the keymap keeps the last seen,
    /// so a collision here would silently drop a command's default.
    #[test]
    fn declared_chords_are_distinct() {
        let chords: Vec<KeyBinding> = COMMANDS
            .iter()
            .filter_map(|c| c.default_keybinding)
            .collect();
        for (i, chord) in chords.iter().enumerate() {
            assert!(
                !chords[i + 1..].contains(chord),
                "{chord:?} is declared twice"
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
            for filter_active in [false, true] {
                for row in context_layout(scope, filter_active) {
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
        assert_eq!(filter_command(false), FILTER_COLUMN);
        assert_eq!(filter_command(true), CLEAR_FILTER);
        let cell = context_layout(CtxScope::Cell, true);
        assert!(cell.contains(&MenuRow::Command(CLEAR_FILTER)));
        assert!(!cell.contains(&MenuRow::Command(FILTER_COLUMN)));
    }

    #[test]
    fn command_to_app_maps_the_logic_thread_commands() {
        // `AppCommand` carries no `PartialEq`, so the variant is matched.
        assert!(matches!(
            command_to_app(UNDO),
            Some(AppCommand::Trame(TrameCommand::Undo))
        ));
        assert!(matches!(
            command_to_app(DELETE_ROWS),
            Some(AppCommand::Trame(TrameCommand::DeleteSelectedRows))
        ));
        // A UI-only intent the launcher intercepts: no bus payload.
        assert!(command_to_app(BOLD).is_none());
        assert!(command_to_app(CommandId("trame.unknown")).is_none());
    }
}
