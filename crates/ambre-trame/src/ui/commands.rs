// ambre-trame/src/ui/commands.rs
// ============================================================================
// Trame's command registry: one `Command` per affordance, and the pure
// `CommandId -> AppCommand` mapping (`ambre-spec-commands.md`, UI-THREAD-019).
//
// Host-compiled, like `ambre-toile`'s `ui::menu`, and for the same reason. The
// description is what every view derives from: the context menu, the toolbar and
// the command palette. A test must reach it without a browser. The wasm-only
// half (the `MenuEntry` builders, the command store, the toast mirror) stays in
// `chrome::commands`.
//
// The registry carries `IconId`, a string. `chrome::icons` resolves it to the
// concrete `icondata::Icon`, which keeps `ambre-commands` a leaf with an empty
// dependency table (UI-THREAD-027). It also keeps Trame's icon set out of the
// shared crate.
// ============================================================================

use ambre_bus::{AppCommand, TrameCommand};
use ambre_commands::{
    Command, CommandCategory, CommandId, IconId, KeyBinding, KeyChar, Keymap, Locale, shared,
};

use crate::ui::grid::CtxScope;

/// Declare one command per line: the constant, its identity, its label, its
/// icon, its default chord and its category.
///
/// A macro rather than 26 hand-written blocks, because `Command::title` is a
/// plain `fn` pointer. It captures nothing, so every command needs a function
/// item of its own. Generating them from one line each is what makes the single
/// declaration site real. A second label for a command cannot be written
/// without a second line here.
macro_rules! commands {
    ($($name:ident = $id:expr, $label:literal, $icon:expr, $chords:expr, $category:expr;)*) => {
        $(pub const $name: CommandId = $id;)*

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
        /// the command store and the keymap from this table. Every view of a
        /// command reads its label and its icon here (`UI-THREAD-019`).
        pub const COMMANDS: &[Command] = &[
            $(Command {
                id: $name,
                title: titles::$name,
                icon: $icon,
                keybindings: $chords,
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
/// Commands that belong to the application shell rather than the document.
const GENERAL: CommandCategory = CommandCategory("general");

// The labels are the ones the chrome shows today: the context-menu entries of
// `chrome::commands` and the toolbar `aria-label`s of `chrome::bars`. Those
// carried "Copy", "Bold" and "Undo" twice over.
//
// The clipboard rows name the shared identities and the shared chords. A copy
// is then the same command in every application and answers the legacy chord
// as well (`UI-THREAD-011`). Undo and Redo take the shared chord lists, which
// is where Ctrl+Y is declared beside Ctrl+Shift+Z.
commands! {
    BOLD = CommandId("trame.format.bold"), "Bold", Some(IconId("format-bold")), &[KeyBinding::primary(KeyChar::B)], FORMAT;
    ITALIC = CommandId("trame.format.italic"), "Italic", Some(IconId("format-italic")), &[KeyBinding::primary(KeyChar::I)], FORMAT;
    UNDO = CommandId("trame.history.undo"), "Undo", Some(IconId("undo")), shared::UNDO_CHORDS, HISTORY;
    REDO = CommandId("trame.history.redo"), "Redo", Some(IconId("redo")), shared::REDO_CHORDS, HISTORY;
    INSERT_ROW_ABOVE = CommandId("trame.row.insert_above"), "Insert row above", Some(IconId("table-row-plus-before")), &[], STRUCTURE;
    INSERT_ROW_BELOW = CommandId("trame.row.insert_below"), "Insert row below", Some(IconId("table-row-plus-after")), &[], STRUCTURE;
    INSERT_COL_LEFT = CommandId("trame.col.insert_left"), "Insert column left", Some(IconId("table-column-plus-before")), &[], STRUCTURE;
    INSERT_COL_RIGHT = CommandId("trame.col.insert_right"), "Insert column right", Some(IconId("table-column-plus-after")), &[], STRUCTURE;
    DELETE_ROWS = CommandId("trame.row.delete"), "Delete rows", Some(IconId("table-row-remove")), &[], STRUCTURE;
    DELETE_COLS = CommandId("trame.col.delete"), "Delete columns", Some(IconId("table-column-remove")), &[], STRUCTURE;
    MERGE = CommandId("trame.merge"), "Merge cells", Some(IconId("table-merge-cells")), &[], STRUCTURE;
    UNMERGE = CommandId("trame.unmerge"), "Unmerge cells", Some(IconId("table-split-cell")), &[], STRUCTURE;
    SORT_RANGE_ASC = CommandId("trame.sort.range_asc"), "Sort range ascending", Some(IconId("sort-ascending")), &[], SORT;
    SORT_RANGE_DESC = CommandId("trame.sort.range_desc"), "Sort range descending", Some(IconId("sort-descending")), &[], SORT;
    SORT_SHEET_ASC = CommandId("trame.sort.sheet_asc"), "Sort sheet by column (asc)", Some(IconId("sort")), &[], SORT;
    SORT_SHEET_DESC = CommandId("trame.sort.sheet_desc"), "Sort sheet by column (desc)", Some(IconId("sort")), &[], SORT;
    SORT_RANGE_MULTI = CommandId("trame.sort.range_multi"), "Sort range...", Some(IconId("sort-variant")), &[], SORT;
    FILTER_COLUMN = CommandId("trame.filter.column"), "Filter by column", Some(IconId("filter")), &[], FILTER;
    CLEAR_FILTER = CommandId("trame.filter.clear"), "Remove filter", Some(IconId("filter-off")), &[], FILTER;
    FIND = CommandId("trame.find.open"), "Find and replace", Some(IconId("search")), &[KeyBinding::primary(KeyChar::F)], FIND_REPLACE;
    REPLACE = CommandId("trame.find.replace"), "Replace", None, &[KeyBinding::primary(KeyChar::H)], FIND_REPLACE;
    COPY = shared::COPY, "Copy", Some(IconId("content-copy")), shared::COPY_CHORDS, CLIPBOARD;
    CUT = shared::CUT, "Cut", Some(IconId("content-cut")), shared::CUT_CHORDS, CLIPBOARD;
    PASTE = shared::PASTE, "Paste", Some(IconId("content-paste")), shared::PASTE_CHORDS, CLIPBOARD;
    IMPORT_CSV = CommandId("trame.interop.import_csv"), "Import CSV", Some(IconId("import")), &[], INTEROP;
    EXPORT_CSV = CommandId("trame.interop.export_csv"), "Export CSV", Some(IconId("export")), &[], INTEROP;
    SAVE_FILE = CommandId("trame.interop.save_file"), "Save to a file (.ambre, not encrypted)", Some(IconId("content-save")), &[], INTEROP;
    SAVE_SEALED_FILE = CommandId("trame.interop.save_sealed_file"), "Save to an encrypted file (.ambre)...", Some(IconId("lock-outline")), &[], INTEROP;
    PALETTE = CommandId("trame.palette.open"), "Command palette", None, &[KeyBinding::primary(KeyChar::K)], GENERAL;
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
/// Single-column shared sort over the selection: the reference column is the
/// selection's first column, and the Logic thread no-ops an empty/single-row
/// range. Also the whole-sheet sort by the active column: a whole-column
/// selection sorts the full used range so rows stay aligned.
const SORT_GROUP: &[CommandId] = &[
    SORT_RANGE_ASC,
    SORT_RANGE_DESC,
    SORT_SHEET_ASC,
    SORT_SHEET_DESC,
    SORT_RANGE_MULTI,
];

/// Clipboard group (`trame-clipboard`): cell-only, driven by the launcher.
const CLIPBOARD_GROUP: &[CommandId] = &[CUT, COPY, PASTE];

/// Merge group (`trame-cell-merges`): cell-only. Always listed, and the Logic
/// thread guards it. A single-cell Merge is a no-op, a covered cell replies
/// MergeRejected -> toast, and Unmerge outside a merge is a no-op.
const MERGE_GROUP: &[CommandId] = &[MERGE, UNMERGE];

/// Whole-sheet sort, for the corner menu.
const CORNER_SORT_GROUP: &[CommandId] = &[SORT_SHEET_ASC, SORT_SHEET_DESC];

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
pub fn filter_command(filter: SheetFilter) -> CommandId {
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
            group(&mut rows, &[filter_command(filter)]);
            rows.push(MenuRow::Separator);
            group(&mut rows, MERGE_GROUP);
        }
        CtxScope::Column => {
            group(&mut rows, COLUMN_STRUCTURAL_GROUP);
            rows.push(MenuRow::Separator);
            group(&mut rows, SORT_GROUP);
            group(&mut rows, &[filter_command(filter)]);
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

/// Translate a Trame `CommandId` into the `AppCommand` IPC payload the Logic
/// thread expects. `None` for a command Trame does not own, and for one
/// `chrome::TrameHandler` resolves before the bus.
pub fn command_to_app(id: CommandId) -> Option<AppCommand> {
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
        // Parameterless: the Logic thread serializes the active sheet itself.
        EXPORT_CSV => TrameCommand::ExportActiveSheetCsv,
        // The native file, which drops nothing (`DOCFILE-001`). Also
        // parameterless, and also served by the Logic thread -- there because
        // the container's header carries the document's identity.
        SAVE_FILE => TrameCommand::ExportAmbreFile,
        // BOLD / ITALIC (style toggles), FILTER_COLUMN, FIND / REPLACE and the
        // clipboard commands are UI-only intents the handler resolves first.
        // BOLD/ITALIC become a `SetCellStyle` patch toggled against the tracked
        // style state, FILTER_COLUMN opens the popover, and FIND/REPLACE open
        // the find/replace panel. None has a direct AppCommand here.
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
            assert!(!label.is_empty(), "{:?} has no label", cmd.id);
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
            label.contains("not encrypted"),
            "the export surface must state that the file is plaintext: {label}"
        );
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
        assert!(matches!(
            command_to_app(SAVE_FILE),
            Some(AppCommand::Trame(TrameCommand::ExportAmbreFile))
        ));
        // A UI-only intent the launcher intercepts: no bus payload.
        assert!(command_to_app(BOLD).is_none());
        assert!(command_to_app(CommandId("trame.unknown")).is_none());
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
