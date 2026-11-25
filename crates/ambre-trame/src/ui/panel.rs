// ambre-trame/src/ui/panel.rs
// ============================================================================
// What the sheet's context panel shows (`ambre-spec-shell.md` section 2).
// Pure, so the host tier reads it.
//
// The shape of the selection picks the sections, never its content
// (`CHROME-253`). A lone cell shows the sheet first, then the cells, the rows
// and columns, the sort and filter, and the print setup, folded first. A range
// of two cells or more shows the cells first and no sheet. The docked format toolbar
// left once the panel held its controls (`CHROME-159`). Find and replace has
// no control here: the Edit menu, its chord and the palette reach it.
// ============================================================================

use ambre_commands::{Property, Reach};
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
/// The name of the active sheet.
pub const SHEET_NAME: StaticKey<Properties> = StaticKey::new("trame.sheet.name");
/// The paper a sheet prints on.
pub const PAPER: StaticKey<Properties> = StaticKey::new("trame.page.paper");
/// The orientation a sheet prints in.
pub const ORIENTATION: StaticKey<Properties> = StaticKey::new("trame.page.orientation");
/// The margins a sheet prints with.
pub const MARGINS: StaticKey<Properties> = StaticKey::new("trame.page.margins");

/// Describe one property per line: its id, its title function and its label.
macro_rules! properties {
    ($($title_fn:ident: $id:expr, $label:literal, $fr:literal;)*) => {

        /// Every property the sheet's panel edits, described once.
        pub const PROPERTIES: &[Property] = &[
            $(Property { id: $id, title: ambre_commands::TitleWords { en: $label, fr: $fr }, icon: None },)*
        ];
    };
}

properties! {
    NUMBER_FORMAT: NUMBER_FORMAT, "Number format", "Format de nombre";
    DECIMALS: DECIMALS, "Decimals", "D\u{e9}cimales";
    FONT: FONT, "Font", "Police";
    SIZE: SIZE, "Font size", "Taille de police";
    TEXT_COLOUR: TEXT_COLOUR, "Text colour", "Couleur du texte";
    FILL: FILL, "Fill colour", "Couleur de remplissage";
    UNDERLINE: UNDERLINE, "Underline", "Soulign\u{e9}";
    STRIKETHROUGH: STRIKETHROUGH, "Strikethrough", "Barr\u{e9}";
    H_ALIGN: H_ALIGN, "Horizontal alignment", "Alignement horizontal";
    V_ALIGN: V_ALIGN, "Vertical alignment", "Alignement vertical";
    BORDERS: BORDERS, "Borders", "Bordures";
    WRAP: WRAP, "Wrap text", "Renvoyer \u{e0} la ligne";
    FREEZE: FREEZE, "Freeze", "Figer";
    SHEET_NAME: SHEET_NAME, "Sheet name", "Nom de la feuille";
    PAPER: PAPER, "Paper", "Papier";
    ORIENTATION: ORIENTATION, "Orientation", "Orientation";
    MARGINS: MARGINS, "Margins", "Marges";
}

/// The sections the panel shows over a lone cell, in order.
pub const SECTIONS: [StaticKey<PanelSections>; 5] = [SHEET, CELLS, ROWS_COLS, SORT_FILTER, PRINT];

/// The sections the panel shows over a range of two cells or more, in order.
pub const RANGE_SECTIONS: [StaticKey<PanelSections>; 4] = [CELLS, ROWS_COLS, SORT_FILTER, PRINT];

/// The shape of the selection, which picks the sections (`CHROME-253`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionShape {
    /// The active cell alone.
    Cell,
    /// Two cells or more.
    Range,
}

impl SelectionShape {
    /// The shape a reference names: `B2:D5` is a range, `B2` a lone cell.
    /// The view writes the reference with a colon for a range alone
    /// (`CHROME-227`).
    #[must_use]
    pub fn of(reference: &ambre_core::Validated<RangeLabel>) -> Self {
        if reference.as_str().contains(':') {
            Self::Range
        } else {
            Self::Cell
        }
    }

    /// The sections the panel shows for this shape, in order.
    #[must_use]
    pub fn sections(self) -> &'static [StaticKey<PanelSections>] {
        match self {
            Self::Cell => &SECTIONS,
            Self::Range => &RANGE_SECTIONS,
        }
    }
}

/// Where the range bar sits: the centre of the top edge of the selection,
/// in view pixels (`CHROME-246`). A type of its own, so the shell signal
/// that carries it holds no value a view also keeps (`UI-THREAD-059`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeBarAnchor {
    /// The centre of the top edge of the selection.
    pub at: ambre_core::Point<ambre_core::Px<ambre_core::View>>,
}

/// The reference of a selection the header of the panel names, such as
/// `B2:D5`. `Validated<RangeLabel>`: the view writes it, no rule bounds it.
pub enum RangeLabel {}

impl ambre_core::Free for RangeLabel {}

impl ambre_core::Displayed for RangeLabel {}

/// What the header of the panel names of the selection: its reference, and
/// the number of cells it spans, which the subtitle counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSelection {
    /// The reference, such as `B2:D5`, which the title shows (`CHROME-227`).
    pub reference: ambre_core::Validated<RangeLabel>,
    /// The cells the selection spans, its hidden rows included.
    pub cells: ambre_core::Count<ambre_core::Cells>,
}

impl Default for PanelSelection {
    fn default() -> Self {
        Self {
            reference: ambre_core::Validated::new(""),
            cells: ambre_core::Count::new(1),
        }
    }
}

impl PanelSelection {
    /// The subtitle of the panel: the cells the selection spans, such as
    /// "12 cells" (`CHROME-241`). A sheet always has an active cell, so a
    /// sheet never reads as nothing selected.
    #[must_use]
    pub fn subtitle(&self) -> String {
        let n = self.cells.get();
        format!("{n} {}", if n == 1 { "cell" } else { "cells" })
    }
}

/// What the context panel shows of the active sheet: its page setup and its
/// frozen pane. The view publishes it from the sheet list (`UI-THREAD-003`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveSheet {
    /// The paper, the orientation and the margins it prints on.
    pub page: crate::message::PageSetup,
    /// The rows and the columns that stay in view.
    pub frozen: crate::message::FrozenPane,
}

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

/// The choice of the freeze list that `pane` reads: none, the rows, or the
/// columns. `None` when both axes freeze, which no single choice names.
#[must_use]
pub fn freeze_choice(pane: crate::message::FrozenPane) -> Option<StaticKey<Commands>> {
    match (pane.rows.is_zero(), pane.cols.is_zero()) {
        (true, true) => Some(cmd::UNFREEZE),
        (false, true) => Some(cmd::FREEZE_ROWS),
        (true, false) => Some(cmd::FREEZE_COLS),
        (false, false) => None,
    }
}

/// The choice of the paper list that `paper` reads.
#[must_use]
pub fn paper_choice(paper: crate::message::PaperSize) -> StaticKey<Commands> {
    use crate::message::PaperSize;
    match paper {
        PaperSize::A4 => cmd::PAPER_A4,
        PaperSize::A3 => cmd::PAPER_A3,
        PaperSize::Letter => cmd::PAPER_LETTER,
        PaperSize::Legal => cmd::PAPER_LEGAL,
    }
}

/// The segment of the orientation bar that `orientation` reads.
#[must_use]
pub fn orientation_choice(orientation: crate::message::PageOrientation) -> StaticKey<Commands> {
    match orientation {
        crate::message::PageOrientation::Portrait => cmd::PAGE_PORTRAIT,
        crate::message::PageOrientation::Landscape => cmd::PAGE_LANDSCAPE,
    }
}

/// The choice of the margins list that `margins` reads, or `None` for
/// margins no preset names, set by an import.
#[must_use]
pub fn margin_choice(margins: crate::message::PageMargins) -> Option<StaticKey<Commands>> {
    use crate::message::PageMargins;
    [
        (PageMargins::NORMAL, cmd::MARGINS_NORMAL),
        (PageMargins::NARROW, cmd::MARGINS_NARROW),
        (PageMargins::WIDE, cmd::MARGINS_WIDE),
    ]
    .into_iter()
    .find_map(|(m, id)| (m == margins).then_some(id))
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

/// The entries of the range bar over a selection of two cells or more, in
/// order (`CHROME-246`). Each one is a control of the context panel
/// (`CHROME-208`). The bar shows unmerge in place of merge on a merged cell.
pub const BAR: [Reach; 7] = [
    Reach::Command(cmd::BOLD),
    Reach::Property(FILL),
    Reach::Property(NUMBER_FORMAT),
    Reach::Command(cmd::MERGE),
    Reach::Command(cmd::UNMERGE),
    Reach::Command(cmd::SORT_RANGE_ASC),
    Reach::Command(cmd::SORT_RANGE_DESC),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_list_reads_the_setup_it_writes() {
        use crate::message::{FrozenPane, PageMargins, PageOrientation, PaperSize};
        use ambre_core::index::Len;
        assert_eq!(freeze_choice(FrozenPane::NONE), Some(cmd::UNFREEZE));
        let rows = FrozenPane {
            rows: Len::new(2),
            ..FrozenPane::NONE
        };
        assert_eq!(freeze_choice(rows), Some(cmd::FREEZE_ROWS));
        let both = FrozenPane {
            cols: Len::new(1),
            ..rows
        };
        assert_eq!(freeze_choice(both), None, "no one choice names both axes");
        assert_eq!(paper_choice(PaperSize::Letter), cmd::PAPER_LETTER);
        assert_eq!(
            orientation_choice(PageOrientation::Landscape),
            cmd::PAGE_LANDSCAPE
        );
        assert_eq!(margin_choice(PageMargins::WIDE), Some(cmd::MARGINS_WIDE));
        assert_eq!(
            margin_choice(PageMargins::even(ambre_core::PrintLength::new(3.0))),
            None
        );
    }

    /// `CHROME-253`: a lone cell shows the sheet first and keeps the cells; a
    /// range shows the cells first and no sheet. The content never decides.
    #[test]
    fn the_shape_of_the_selection_picks_the_sections() {
        let cell = SelectionShape::of(&ambre_core::Validated::new("B2"));
        assert_eq!(cell, SelectionShape::Cell);
        assert_eq!(cell.sections().first(), Some(&SHEET));
        assert!(
            cell.sections().contains(&CELLS),
            "a lone cell stays formattable"
        );
        let range = SelectionShape::of(&ambre_core::Validated::new("B2:D5"));
        assert_eq!(range, SelectionShape::Range);
        assert_eq!(range.sections().first(), Some(&CELLS));
        assert!(!range.sections().contains(&SHEET));
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
    /// range bar holds no clipboard entry (`CHROME-151`).
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
