// ambre-trame/src/ui/panel.rs
// ============================================================================
// What the sheet's context panel shows (`ambre-spec-shell.md` section 2).
// Pure, so the host tier reads it.
//
// A sheet always holds an active cell, so the panel always shows the same
// four sections. They are the sheet, the cells, the rows and columns, and the
// sort and filter. The docked format toolbar left once the panel held its controls
// (`CHROME-159`).
// ============================================================================

use ambre_commands::{Locale, Property, Reach};
use ambre_core::{Commands, PanelSections, Properties, StaticKey};

use super::commands as cmd;

/// The sheet: its page setup, its frozen panes and find.
pub const SHEET: StaticKey<PanelSections> = StaticKey::new("trame.sheet");
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
}

/// The sections the panel shows, in order.
pub const SECTIONS: [StaticKey<PanelSections>; 4] = [SHEET, CELLS, ROWS_COLS, SORT_FILTER];

/// The commands the sheet section shows.
pub const SHEET_COMMANDS: [StaticKey<Commands>; 13] = [
    cmd::FIND,
    cmd::PAGE_PORTRAIT,
    cmd::PAGE_LANDSCAPE,
    cmd::PAPER_A4,
    cmd::PAPER_A3,
    cmd::PAPER_LETTER,
    cmd::PAPER_LEGAL,
    cmd::MARGINS_NORMAL,
    cmd::MARGINS_NARROW,
    cmd::MARGINS_WIDE,
    cmd::FREEZE_ROWS,
    cmd::FREEZE_COLS,
    cmd::UNFREEZE,
];

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
