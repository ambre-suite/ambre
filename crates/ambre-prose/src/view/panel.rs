// ambre-prose/src/view/panel.rs
// ============================================================================
// What the page's context panel shows, and what its controls write
// (`ambre-spec-shell.md` section 2, `features/ambre-spec-prose-toolbar.md`).
// Pure, so the host tier reads it.
//
// With no text selected the panel shows the page: its paper, its orientation
// and its four margins (`PROSE-UI-253`). With a selection it shows the text:
// the marks, superscript and subscript, and clearing the formatting
// (`PROSE-UI-191`). A page control writes the whole format in one command, so
// one undo takes it back (`PROSE-UI-252`).
// ============================================================================

use ambre_commands::{Locale, Property, Reach};
use ambre_core::{Commands, Count, Mm, PanelSections, PrintLength, Properties, StaticKey};

use crate::commands;
use crate::doc::PageFormat;
use crate::events::Words;

/// The page, with no text selected.
pub const PAGE: StaticKey<PanelSections> = StaticKey::new("prose.page");
/// The text of the selection.
pub const TEXT: StaticKey<PanelSections> = StaticKey::new("prose.text");
/// The suggestion at the caret, whatever the kind (`PROSE-UI-259`).
pub const SUGGESTION: StaticKey<PanelSections> = StaticKey::new("prose.suggestion");
/// The review section: suggestion mode, shown whatever is selected
/// (`PROSE-UI-262`).
pub const REVIEW: StaticKey<PanelSections> = StaticKey::new("prose.review");

/// The paper of the page.
pub const PAPER: StaticKey<Properties> = StaticKey::new("prose.page.paper");
/// The orientation of the page.
pub const ORIENTATION: StaticKey<Properties> = StaticKey::new("prose.page.orientation");
/// The top margin.
pub const MARGIN_TOP: StaticKey<Properties> = StaticKey::new("prose.page.margin_top");
/// The bottom margin.
pub const MARGIN_BOTTOM: StaticKey<Properties> = StaticKey::new("prose.page.margin_bottom");
/// The left margin.
pub const MARGIN_LEFT: StaticKey<Properties> = StaticKey::new("prose.page.margin_left");
/// The right margin.
pub const MARGIN_RIGHT: StaticKey<Properties> = StaticKey::new("prose.page.margin_right");
/// The four margins, as a preset.
pub const MARGINS: StaticKey<Properties> = StaticKey::new("prose.page.margins");

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

        /// Every property the page's panel edits, described once.
        pub const PROPERTIES: &[Property] = &[
            $(Property { id: $id, title: titles::$title_fn, icon: None },)*
        ];
    };
}

properties! {
    PAPER: PAPER, "Paper";
    ORIENTATION: ORIENTATION, "Orientation";
    MARGIN_TOP: MARGIN_TOP, "Top margin";
    MARGIN_BOTTOM: MARGIN_BOTTOM, "Bottom margin";
    MARGIN_LEFT: MARGIN_LEFT, "Left margin";
    MARGIN_RIGHT: MARGIN_RIGHT, "Right margin";
    MARGINS: MARGINS, "Margins";
}

/// The title of a property no table describes.
fn unnamed_title(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
    ambre_core::Validated::new("Property")
}

/// What [`property`] answers for an id no table describes. Every id the
/// panel names is in the table, which a wasm test holds.
const UNNAMED: Property = Property {
    id: StaticKey::new("prose.unnamed"),
    title: unnamed_title,
    icon: None,
};

/// The property of `id`, as the panel's controls take it.
#[must_use]
pub fn property(id: StaticKey<Properties>) -> &'static Property {
    PROPERTIES.iter().find(|p| p.id == id).unwrap_or(&UNNAMED)
}

/// What the selection is, as the panel reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    /// No text is selected: the panel shows the page.
    Page,
    /// Some text is selected.
    Text,
}

impl PanelKind {
    /// Every kind, for a test that walks them all.
    pub const ALL: [Self; 2] = [Self::Page, Self::Text];

    /// The header of the panel for this kind.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Page => "Document",
            Self::Text => "Selection",
        }
    }

    /// The subtitle of the panel for this kind (`CHROME-241`): the words of
    /// the selected text. A count the view could not read shows nothing.
    #[must_use]
    pub fn subtitle(self, words: Option<Count<Words>>) -> String {
        match self {
            Self::Page => ambre_commands::NOTHING_SELECTED.to_owned(),
            Self::Text => words.map_or_else(String::new, |n| {
                let n = n.get();
                format!("{n} {}", if n == 1 { "word" } else { "words" })
            }),
        }
    }

    /// The sections the panel shows for this kind.
    #[must_use]
    pub fn sections(self) -> &'static [StaticKey<PanelSections>] {
        match self {
            Self::Page => &[PAGE],
            Self::Text => &[TEXT],
        }
    }

    /// The entries of the context bar for this kind (`CHROME-208`): the
    /// marks of the format bar over a selection, nothing over the page.
    #[must_use]
    pub fn bar(self) -> &'static [Reach] {
        match self {
            Self::Text => &[
                Reach::Command(commands::BOLD),
                Reach::Command(commands::ITALIC),
                Reach::Command(commands::UNDERLINE),
                Reach::Command(commands::STRIKETHROUGH),
                Reach::Command(commands::CODE),
            ],
            Self::Page => &[],
        }
    }
}

/// The commands the text section shows, in order: the five marks of the
/// bar, superscript, subscript, then clearing the formatting.
pub const TEXT_COMMANDS: [StaticKey<Commands>; 8] = [
    commands::BOLD,
    commands::ITALIC,
    commands::UNDERLINE,
    commands::STRIKETHROUGH,
    commands::CODE,
    commands::SUPERSCRIPT,
    commands::SUBSCRIPT,
    commands::CLEAR_FORMATTING,
];

/// The commands the suggestion section shows, in order (`PROSE-UI-259`).
pub const SUGGESTION_COMMANDS: [StaticKey<Commands>; 2] =
    [commands::ACCEPT_SUGGESTION, commands::REJECT_SUGGESTION];

/// The commands the review section shows (`PROSE-UI-262`).
pub const REVIEW_COMMANDS: [StaticKey<Commands>; 1] = [commands::SUGGEST_CHANGES];

/// The sections the panel shows. First come those of `kind`, then the
/// suggestion section while a suggestion holds the caret (`PROSE-UI-259`).
/// The review section comes last, whatever is selected (`PROSE-UI-262`).
#[must_use]
pub fn sections(kind: PanelKind, at_caret: CaretSuggestion) -> Vec<StaticKey<PanelSections>> {
    let mut all = kind.sections().to_vec();
    if at_caret == CaretSuggestion::Held {
        all.push(SUGGESTION);
    }
    all.push(REVIEW);
    all
}

ambre_core::define_flag! {
    /// Whether a suggestion holds the character after the caret.
    pub enum CaretSuggestion {
        /// The caret is outside every suggestion.
        Outside,
        /// A suggestion holds the caret.
        Held,
    }
}

/// A paper the page section offers, in its portrait dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paper {
    A4,
    A5,
    Letter,
    Legal,
    /// Dimensions no named paper has.
    Other,
}

impl Paper {
    /// The named papers, in the order the select offers them.
    pub const NAMED: [Self; 4] = [Self::A4, Self::A5, Self::Letter, Self::Legal];

    /// The portrait width and height, or `None` for another paper.
    #[must_use]
    pub fn portrait(self) -> Option<(PrintLength<Mm>, PrintLength<Mm>)> {
        let mm = PrintLength::<Mm>::new;
        match self {
            Self::A4 => Some((mm(210.0), mm(297.0))),
            Self::A5 => Some((mm(148.0), mm(210.0))),
            Self::Letter => Some((mm(215.9), mm(279.4))),
            Self::Legal => Some((mm(215.9), mm(355.6))),
            Self::Other => None,
        }
    }

    /// The label a person reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::A5 => "A5",
            Self::Letter => "Letter",
            Self::Legal => "Legal",
            Self::Other => "Custom",
        }
    }

    /// The paper of a page, whatever its orientation.
    #[must_use]
    pub fn of(format: &PageFormat) -> Self {
        let (short, long) = (
            format.width.min(format.height),
            format.width.max(format.height),
        );
        let near =
            |a: PrintLength<Mm>, b: PrintLength<Mm>| (a - b).abs() < PrintLength::<Mm>::new(0.5);
        Self::NAMED
            .into_iter()
            .find(|paper| {
                paper
                    .portrait()
                    .is_some_and(|(w, h)| near(w, short) && near(h, long))
            })
            .unwrap_or(Self::Other)
    }
}

/// A set of four margins the page section offers in one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Margins {
    /// 20 mm on each edge: what a new document holds.
    Normal,
    /// 12.7 mm on each edge.
    Narrow,
    /// 25.4 mm at the top and the bottom, 50.8 mm at the sides.
    Wide,
    /// Margins no preset names: the four steppers show.
    Custom,
}

impl Margins {
    /// The presets, then custom, in the order the list offers them.
    pub const ALL: [Self; 4] = [Self::Normal, Self::Narrow, Self::Wide, Self::Custom];

    /// The top and bottom margin, and the side margin, or `None` for
    /// [`Margins::Custom`].
    #[must_use]
    pub fn bands(self) -> Option<(PrintLength<Mm>, PrintLength<Mm>)> {
        let mm = PrintLength::<Mm>::new;
        match self {
            Self::Normal => Some((mm(20.0), mm(20.0))),
            Self::Narrow => Some((mm(12.7), mm(12.7))),
            Self::Wide => Some((mm(25.4), mm(50.8))),
            Self::Custom => None,
        }
    }

    /// The label a person reads.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Narrow => "Narrow",
            Self::Wide => "Wide",
            Self::Custom => "Custom...",
        }
    }

    /// The preset the four margins of `format` match, or custom.
    #[must_use]
    pub fn of(format: &PageFormat) -> Self {
        let near =
            |a: PrintLength<Mm>, b: PrintLength<Mm>| (a - b).abs() < PrintLength::<Mm>::new(0.05);
        Self::ALL
            .into_iter()
            .find(|m| {
                m.bands().is_some_and(|(ends, sides)| {
                    near(format.margin_top, ends)
                        && near(format.margin_bottom, ends)
                        && near(format.margin_left, sides)
                        && near(format.margin_right, sides)
                })
            })
            .unwrap_or(Self::Custom)
    }
}

/// The page with the four margins of `margins`, its paper kept. `None` for
/// [`Margins::Custom`], which names no margins.
#[must_use]
pub fn with_margins(format: PageFormat, margins: Margins) -> Option<PageFormat> {
    let (ends, sides) = margins.bands()?;
    Some(PageFormat {
        margin_top: ends,
        margin_bottom: ends,
        margin_left: sides,
        margin_right: sides,
        ..format
    })
}

/// The orientation the panel shows: landscape when the page is wider than
/// tall.
#[must_use]
pub fn orientation_of(format: &PageFormat) -> crate::doc::Orientation {
    if format.width > format.height {
        crate::doc::Orientation::Landscape
    } else {
        crate::doc::Orientation::Portrait
    }
}

/// The page with `paper`, in the orientation the page has now. `None` for
/// [`Paper::Other`], which names no dimensions.
#[must_use]
pub fn with_paper(format: PageFormat, paper: Paper) -> Option<PageFormat> {
    let (w, h) = paper.portrait()?;
    let (width, height) = match orientation_of(&format) {
        crate::doc::Orientation::Portrait => (w, h),
        crate::doc::Orientation::Landscape => (h, w),
    };
    Some(PageFormat {
        width,
        height,
        ..format
    })
}

/// The page turned to `orientation`: the two dimensions swap when it
/// differs from the page's own.
#[must_use]
pub fn with_orientation(format: PageFormat, orientation: crate::doc::Orientation) -> PageFormat {
    if orientation_of(&format) == orientation {
        return format;
    }
    PageFormat {
        width: format.height,
        height: format.width,
        ..format
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The subtitle reads nothing selected over the page, and the words of
    /// a text selection (`CHROME-241`).
    #[test]
    fn the_subtitle_counts_the_selected_words() {
        assert_eq!(PanelKind::Page.subtitle(None), "nothing selected");
        assert_eq!(PanelKind::Text.subtitle(Some(Count::of(1))), "1 word");
        assert_eq!(PanelKind::Text.subtitle(Some(Count::of(4))), "4 words");
        assert_eq!(PanelKind::Text.subtitle(None), "");
    }
    use crate::doc::Orientation;

    #[test]
    fn each_bar_reaches_the_panel_and_holds_no_clipboard() {
        for kind in PanelKind::ALL {
            assert_eq!(
                ambre_commands::bar_defects(kind.bar(), commands::COMMANDS, PROPERTIES),
                vec![],
                "{kind:?}"
            );
            for entry in kind.bar() {
                if let Reach::Command(id) = entry {
                    assert!(TEXT_COMMANDS.contains(id), "{id:?} is in the text section");
                }
            }
        }
    }

    /// `PROSE-UI-262`: the review section shows over the page and over a
    /// selection, with the caret in a suggestion or not. A selection never
    /// hides the suggestion-mode control.
    #[test]
    fn the_review_section_shows_whatever_is_selected() {
        for kind in PanelKind::ALL {
            for at_caret in [CaretSuggestion::Outside, CaretSuggestion::Held] {
                let shown = sections(kind, at_caret);
                assert_eq!(shown.last(), Some(&REVIEW), "{kind:?} {at_caret:?}");
            }
        }
        let command =
            ambre_commands::find(commands::COMMANDS, commands::SUGGEST_CHANGES).expect("declared");
        assert_eq!(command.home, ambre_commands::Home::ContextPanel);
        assert_eq!(REVIEW_COMMANDS, [commands::SUGGEST_CHANGES]);
    }

    #[test]
    fn a_paper_keeps_the_orientation_and_a_turn_swaps_the_dimensions() {
        let a4 = PageFormat::default();
        assert_eq!(Paper::of(&a4), Paper::A4);
        let wide = with_orientation(a4, Orientation::Landscape);
        assert_eq!(orientation_of(&wide), Orientation::Landscape);
        assert_eq!(Paper::of(&wide), Paper::A4, "a turned A4 is still A4");
        let letter = with_paper(wide, Paper::Letter).expect("a named paper");
        assert_eq!(Paper::of(&letter), Paper::Letter);
        assert_eq!(orientation_of(&letter), Orientation::Landscape);
        assert_eq!(letter.margin_left, a4.margin_left, "the margins stay");
        assert_eq!(with_paper(a4, Paper::Other), None);
    }

    #[test]
    fn a_margin_preset_reads_back_and_keeps_the_paper() {
        let a4 = PageFormat::default();
        assert_eq!(Margins::of(&a4), Margins::Normal, "a new document");
        let wide = with_margins(a4, Margins::Wide).expect("a preset");
        assert_eq!(Margins::of(&wide), Margins::Wide);
        assert_eq!((wide.width, wide.height), (a4.width, a4.height));
        let mut odd = a4;
        odd.margin_left = PrintLength::<Mm>::new(31.0);
        assert_eq!(Margins::of(&odd), Margins::Custom);
        assert_eq!(with_margins(a4, Margins::Custom), None);
    }

    /// The page's context menu lists its clipboard entries first, in the
    /// order cut, copy, paste, then commands the text section shows
    /// (`CHROME-179`, `CHROME-182`, `CHROME-209`).
    #[test]
    fn the_page_menu_opens_on_the_clipboard_then_reaches_the_panel() {
        let menu: Vec<Reach> = commands::CONTEXT_MENU
            .into_iter()
            .map(Reach::Command)
            .collect();
        assert_eq!(
            ambre_commands::menu_defects(&menu, commands::COMMANDS, PROPERTIES),
            vec![]
        );
        for id in commands::CONTEXT_MENU {
            assert!(
                commands::MENU_CLIPBOARD.contains(&id) || TEXT_COMMANDS.contains(&id),
                "{id:?} is a control of the text section"
            );
        }
    }

    /// Cut, copy and paste live in the context menu (`CHROME-178`). Every
    /// formatting command of the page reaches the text section
    /// (`CHROME-166`). The format bar shows the declared list (`CHROME-150`).
    #[test]
    fn the_clipboard_homes_in_the_menu_and_the_panel_holds_every_formatting_command() {
        use ambre_commands::{Home, shared};
        for id in [shared::CUT, shared::COPY, shared::PASTE] {
            let command = ambre_commands::find(commands::COMMANDS, id).expect("declared");
            assert_eq!(command.home, Home::ContextMenu, "{id:?}");
        }
        for command in commands::COMMANDS
            .iter()
            .filter(|c| c.category.as_str() == "format")
        {
            assert!(
                TEXT_COMMANDS.contains(&command.id),
                "{:?} is in the panel",
                command.id
            );
        }
        let bar: Vec<Reach> = commands::FORMAT_BAR
            .iter()
            .map(|(id, _)| Reach::Command(*id))
            .collect();
        assert_eq!(bar, PanelKind::Text.bar());
    }
}
