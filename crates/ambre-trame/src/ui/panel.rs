// ambre-trame/src/ui/panel.rs
// ============================================================================
// What the sheet's context panel shows (`ambre-spec-shell.md` section 2).
// Pure, so the host tier reads it.
//
// A sheet always holds an active cell, so the panel always shows the same
// five sections. They are the sheet, the cells, the rows and columns, the sort
// and filter, and the print setup, folded first. The docked format toolbar
// left once the panel held its controls (`CHROME-159`). Find and replace has
// no control here: the Edit menu, its chord and the palette reach it.
// ============================================================================

use ambre_commands::{Locale, Property, Reach};
use ambre_core::{Commands, PanelSections, Properties, StaticKey};

use super::commands as cmd;

/// The sheet: its frozen panes.
pub const SHEET: StaticKey<PanelSections> = StaticKey::new("trame.sheet");
/// The page setup the sheet prints on, folded the first time it shows.
pub const PRINT: StaticKey<PanelSections> = StaticKey::new("trame.print");
/// The style and the format of the selected cells.
pub const CELLS: StaticKey<PanelSections> = StaticKey::new("trame.cells");
/// The rows and the columns of the selection.
pub const ROWS_COLS: StaticKey<PanelSections> = StaticKey::new("trame.rows_cols");
/// The sort and the filter of the selection.
pub const SORT_FILTER: StaticKey<PanelSections> = StaticKey::new("trame.sort_filter");

/// The number format of a cell.
pub const NUMBER_FORMAT: StaticKey<Properties> = StaticKey::new("trame.cell.number_format");
/// The decimals a number format shows.
pub const DECIMALS: StaticKey<Properties> = StaticKey::new("trame.cell.decimals");
/// The font family of a cell.
pub const FONT: StaticKey<Properties> = StaticKey::new("trame.cell.font");
/// The font size of a cell.
pub const SIZE: StaticKey<Properties> = StaticKey::new("trame.cell.size");
/// The text colour of a cell.
pub const TEXT_COLOUR: StaticKey<Properties> = StaticKey::new("trame.cell.text_colour");
/// The fill of a cell.
pub const FILL: StaticKey<Properties> = StaticKey::new("trame.cell.fill");
/// The underline of a cell.
pub const UNDERLINE: StaticKey<Properties> = StaticKey::new("trame.cell.underline");
/// The strikethrough of a cell.
pub const STRIKETHROUGH: StaticKey<Properties> = StaticKey::new("trame.cell.strikethrough");
/// The horizontal alignment of a cell.
pub const H_ALIGN: StaticKey<Properties> = StaticKey::new("trame.cell.h_align");
/// The vertical alignment of a cell.
pub const V_ALIGN: StaticKey<Properties> = StaticKey::new("trame.cell.v_align");
/// The borders of a range.
pub const BORDERS: StaticKey<Properties> = StaticKey::new("trame.cell.borders");
/// Whether a cell wraps its text.
pub const WRAP: StaticKey<Properties> = StaticKey::new("trame.cell.wrap");
/// The rows and the columns that stay in view.
pub const FREEZE: StaticKey<Properties> = StaticKey::new("trame.sheet.freeze");
/// The paper a sheet prints on.
pub const PAPER: StaticKey<Properties> = StaticKey::new("trame.page.paper");
/// The orientation a sheet prints in.
pub const ORIENTATION: StaticKey<Properties> = StaticKey::new("trame.page.orientation");
/// The margins a sheet prints with.
pub const MARGINS: StaticKey<Properties> = StaticKey::new("trame.page.margins");

/// Describe one property per line: its id, its title function and its label.
macro_rules! properties {
    ($($title_fn:ident: $id:expr, $label:literal;)*) => {
        #[expect(
            non_snake_case,
            reason = "each title function takes the name of its property constant"
        )]
        mod titles {
            use super::Locale;
            $(pub fn $title_fn(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
                ambre_core::Validated::new($label)
            })*
        }

        /// Every property the sheet's panel edits, described once.
        pub const PROPERTIES: &[Property] = &[
            $(Property { id: $id, title: titles::$title_fn, icon: None },)*
        ];
    };
}

properties! {
    NUMBER_FORMAT: NUMBER_FORMAT, "Number format";
    DECIMALS: DECIMALS, "Decimals";
    FONT: FONT, "Font";
    SIZE: SIZE, "Font size";
    TEXT_COLOUR: TEXT_COLOUR, "Text colour";
    FILL: FILL, "Fill colour";
    UNDERLINE: UNDERLINE, "Underline";
    STRIKETHROUGH: STRIKETHROUGH, "Strikethrough";
    H_ALIGN: H_ALIGN, "Horizontal alignment";
    V_ALIGN: V_ALIGN, "Vertical alignment";
    BORDERS: BORDERS, "Borders";
    WRAP: WRAP, "Wrap text";
    FREEZE: FREEZE, "Freeze";
    PAPER: PAPER, "Paper";
    ORIENTATION: ORIENTATION, "Orientation";
    MARGINS: MARGINS, "Margins";
}

/// The sections the panel shows, in order.
pub const SECTIONS: [StaticKey<PanelSections>; 5] = [SHEET, CELLS, ROWS_COLS, SORT_FILTER, PRINT];

/// The choices of the freeze list of the sheet section, in order: no pane,
/// the rows through the active cell, the columns through it.
pub const FREEZE_COMMANDS: [StaticKey<Commands>; 3] =
    [cmd::UNFREEZE, cmd::FREEZE_ROWS, cmd::FREEZE_COLS];

/// The choices of the paper list of the print section, in order.
pub const PAPER_COMMANDS: [StaticKey<Commands>; 4] = [
    cmd::PAPER_A4,
    cmd::PAPER_A3,
    cmd::PAPER_LETTER,
    cmd::PAPER_LEGAL,
];

/// The segments of the orientation bar of the print section, in order.
pub const ORIENTATION_COMMANDS: [StaticKey<Commands>; 2] =
    [cmd::PAGE_PORTRAIT, cmd::PAGE_LANDSCAPE];

/// The choices of the margins list of the print section, in order.
pub const MARGIN_COMMANDS: [StaticKey<Commands>; 3] =
    [cmd::MARGINS_NORMAL, cmd::MARGINS_NARROW, cmd::MARGINS_WIDE];

/// The commands the sheet and print sections reach, one choice each.
pub const SHEET_COMMANDS: [StaticKey<Commands>; 12] = [
    cmd::UNFREEZE,
    cmd::FREEZE_ROWS,
    cmd::FREEZE_COLS,
    cmd::PAPER_A4,
    cmd::PAPER_A3,
    cmd::PAPER_LETTER,
    cmd::PAPER_LEGAL,
    cmd::PAGE_PORTRAIT,
    cmd::PAGE_LANDSCAPE,
    cmd::MARGINS_NORMAL,
    cmd::MARGINS_NARROW,
    cmd::MARGINS_WIDE,
];

/// The place of the choice of the freeze list that `pane` reads: none, the
/// rows, or the columns. `None` when both axes freeze, which no single
/// choice names.
#[must_use]
pub fn freeze_place(pane: crate::message::FrozenPane) -> Option<usize> {
    match (pane.rows.get() > 0, pane.cols.get() > 0) {
        (false, false) => Some(0),
        (true, false) => Some(1),
        (false, true) => Some(2),
        (true, true) => None,
    }
}

/// The place of `paper` in the paper list.
#[must_use]
pub fn paper_place(paper: crate::message::PaperSize) -> usize {
    use crate::message::PaperSize;
    match paper {
        PaperSize::A4 => 0,
        PaperSize::A3 => 1,
        PaperSize::Letter => 2,
        PaperSize::Legal => 3,
    }
}

/// The place of `orientation` in the orientation bar.
#[must_use]
pub fn orientation_place(orientation: crate::message::PageOrientation) -> usize {
    match orientation {
        crate::message::PageOrientation::Portrait => 0,
        crate::message::PageOrientation::Landscape => 1,
    }
}

/// The place of `margins` in the margins list, or `None` for margins no
/// preset names, set by an import.
#[must_use]
pub fn margin_place(margins: crate::message::PageMargins) -> Option<usize> {
    use crate::message::PageMargins;
    [PageMargins::NORMAL, PageMargins::NARROW, PageMargins::WIDE]
        .iter()
        .position(|m| *m == margins)
}

/// The commands the rows and columns section shows.
pub const ROW_COL_COMMANDS: [StaticKey<Commands>; 6] = [
    cmd::INSERT_ROW_ABOVE,
    cmd::INSERT_ROW_BELOW,
    cmd::INSERT_COL_LEFT,
    cmd::INSERT_COL_RIGHT,
    cmd::DELETE_ROWS,
    cmd::DELETE_COLS,
];

/// The commands the sort and filter section shows.
pub const SORT_FILTER_COMMANDS: [StaticKey<Commands>; 7] = [
    cmd::SORT_RANGE_ASC,
    cmd::SORT_RANGE_DESC,
    cmd::SORT_RANGE_MULTI,
    cmd::SORT_SHEET_ASC,
    cmd::SORT_SHEET_DESC,
    cmd::FILTER_COLUMN,
    cmd::CLEAR_FILTER,
];

/// The entries of the context bar. The sheet lays out no floating bar yet,
/// so the list is empty, and the panel holds every entry it would carry.
pub const BAR: [Reach; 0] = [];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_reads_the_setup_it_writes() {
        use crate::message::{FrozenPane, PageMargins, PageOrientation, PaperSize};
        use ambre_core::index::Len;
        assert_eq!(freeze_place(FrozenPane::NONE), Some(0));
        let rows = FrozenPane {
            rows: Len::new(2),
            ..FrozenPane::NONE
        };
        assert_eq!(freeze_place(rows), Some(1));
        let both = FrozenPane {
            cols: Len::new(1),
            ..rows
        };
        assert_eq!(freeze_place(both), None, "no one choice names both axes");
        assert_eq!(FREEZE_COMMANDS[1], cmd::FREEZE_ROWS);
        assert_eq!(
            PAPER_COMMANDS[paper_place(PaperSize::Letter)],
            cmd::PAPER_LETTER
        );
        assert_eq!(
            ORIENTATION_COMMANDS[orientation_place(PageOrientation::Landscape)],
            cmd::PAGE_LANDSCAPE
        );
        assert_eq!(
            margin_place(PageMargins::WIDE).map(|p| MARGIN_COMMANDS[p]),
            Some(cmd::MARGINS_WIDE)
        );
        assert_eq!(
            margin_place(PageMargins::even(ambre_core::PrintLength::new(3.0))),
            None
        );
    }

    /// Find and replace has no control in the panel: the Edit menu, its chord
    /// and the palette reach it.
    #[test]
    fn find_leaves_the_panel_and_keeps_its_chord() {
        assert!(!SHEET_COMMANDS.contains(&cmd::FIND));
        let find = ambre_commands::find(cmd::COMMANDS, cmd::FIND).expect("declared");
        assert!(!find.keybindings.is_empty(), "Ctrl+F reaches it");
    }

    #[test]
    fn every_command_the_panel_shows_is_homed_in_the_panel() {
        let shown = SHEET_COMMANDS
            .iter()
            .chain(ROW_COL_COMMANDS.iter())
            .chain(SORT_FILTER_COMMANDS.iter())
            .map(|&id| Reach::Command(id))
            .collect::<Vec<_>>();
        assert_eq!(
            ambre_commands::bar_defects(&shown, cmd::COMMANDS, PROPERTIES),
            vec![]
        );
    }

    /// Every scope of the sheet's context menu lists its clipboard entries
    /// first, in the order cut, copy, paste. Then it lists commands the panel
    /// shows (`CHROME-179`, `CHROME-182`, `CHROME-209`).
    #[test]
    fn the_sheet_menu_opens_on_the_clipboard_then_reaches_the_panel() {
        use crate::ui::commands::{MenuRow, SheetFilter, context_layout};
        use crate::ui::grid::CtxScope;
        let shown: Vec<StaticKey<Commands>> = SHEET_COMMANDS
            .iter()
            .chain(ROW_COL_COMMANDS.iter())
            .chain(SORT_FILTER_COMMANDS.iter())
            .copied()
            .chain([cmd::BOLD, cmd::ITALIC, cmd::MERGE, cmd::UNMERGE])
            .collect();
        for scope in [
            CtxScope::Cell,
            CtxScope::Column,
            CtxScope::Row,
            CtxScope::Corner,
        ] {
            for filter in [SheetFilter::Unfiltered, SheetFilter::Filtered] {
                let menu: Vec<Reach> = context_layout(scope, filter)
                    .into_iter()
                    .filter_map(|row| match row {
                        MenuRow::Command(id) => Some(Reach::Command(id)),
                        MenuRow::Separator => None,
                    })
                    .collect();
                assert_eq!(
                    ambre_commands::menu_defects(&menu, cmd::COMMANDS, PROPERTIES),
                    vec![],
                    "{scope:?}"
                );
                for entry in menu {
                    if let Reach::Command(id) = entry
                        && !ambre_commands::shared::BROWSER_SERVICED.contains(&id)
                    {
                        assert!(shown.contains(&id), "{id:?} is a control of the panel");
                    }
                }
            }
        }
    }

    /// Cut, copy and paste live in the context menu (`CHROME-178`). Every
    /// formatting command of the sheet reaches the cells section
    /// (`CHROME-166`): bold and italic are commands, the rest properties. The
    /// sheet's empty bar holds no clipboard entry (`CHROME-151`).
    #[test]
    fn the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command() {
        use ambre_commands::{Home, shared};
        for id in [shared::CUT, shared::COPY, shared::PASTE] {
            let command = ambre_commands::find(cmd::COMMANDS, id).expect("declared");
            assert_eq!(command.home, Home::ContextMenu, "{id:?}");
        }
        let format: Vec<_> = cmd::COMMANDS
            .iter()
            .filter(|c| c.category.as_str() == "format")
            .map(|c| c.id)
            .collect();
        assert_eq!(format, vec![cmd::BOLD, cmd::ITALIC]);
        assert_eq!(
            ambre_commands::bar_defects(&BAR, cmd::COMMANDS, PROPERTIES),
            vec![]
        );
    }
}
