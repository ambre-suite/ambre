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
/// The paragraphs of the selection: alignment, line spacing, indents.
pub const PARAGRAPH: StaticKey<PanelSections> = StaticKey::new("prose.paragraph");
/// The kind of each paragraph of the selection.
pub const PARAGRAPH_STYLE: StaticKey<PanelSections> = StaticKey::new("prose.paragraph_style");
/// The actions over the selection: link, comment, clear the formatting.
pub const ACTIONS: StaticKey<PanelSections> = StaticKey::new("prose.actions");
/// The default fonts of the document, with no text selected.
pub const DEFAULT_STYLES: StaticKey<PanelSections> = StaticKey::new("prose.default_styles");

/// The font of the selection.
pub const FONT: StaticKey<Properties> = StaticKey::new("prose.text.font");
/// The size of the selection.
pub const SIZE: StaticKey<Properties> = StaticKey::new("prose.text.size");
/// The colour of the text of the selection.
pub const COLOUR: StaticKey<Properties> = StaticKey::new("prose.text.colour");
/// The highlight behind the text of the selection.
pub const HIGHLIGHT: StaticKey<Properties> = StaticKey::new("prose.text.highlight");
/// The alignment of the paragraphs of the selection.
pub const ALIGN: StaticKey<Properties> = StaticKey::new("prose.paragraph.align");
/// The line spacing of the paragraphs of the selection.
pub const LINE_SPACING: StaticKey<Properties> = StaticKey::new("prose.paragraph.line_spacing");
/// The left indent of the paragraphs of the selection.
pub const INDENT_LEFT: StaticKey<Properties> = StaticKey::new("prose.paragraph.indent_left");
/// The first-line indent of the paragraphs of the selection.
pub const INDENT_FIRST: StaticKey<Properties> = StaticKey::new("prose.paragraph.indent_first");
/// What each paragraph of the selection is: body, heading, quote, list.
pub const KIND: StaticKey<Properties> = StaticKey::new("prose.paragraph.kind");
/// The body font of the document.
pub const BODY_FONT: StaticKey<Properties> = StaticKey::new("prose.defaults.body_font");
/// The font of every heading of the document.
pub const HEADING_FONT: StaticKey<Properties> = StaticKey::new("prose.defaults.heading_font");

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
    FONT: FONT, "Font";
    SIZE: SIZE, "Font size";
    COLOUR: COLOUR, "Text colour";
    HIGHLIGHT: HIGHLIGHT, "Highlight";
    ALIGN: ALIGN, "Alignment";
    LINE_SPACING: LINE_SPACING, "Line spacing";
    INDENT_LEFT: INDENT_LEFT, "Left indent";
    INDENT_FIRST: INDENT_FIRST, "First-line indent";
    KIND: KIND, "Paragraph style";
    BODY_FONT: BODY_FONT, "Body font";
    HEADING_FONT: HEADING_FONT, "Heading font";
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
            Self::Page => &[PAGE, DEFAULT_STYLES],
            Self::Text => &[TEXT, PARAGRAPH, PARAGRAPH_STYLE, ACTIONS],
        }
    }

    /// The entries of the context bar for this kind (`CHROME-208`), in the
    /// mockup's order. Over a selection, they are three marks, the link,
    /// the paragraph style list and the comment. Over the page, none.
    #[must_use]
    pub fn bar(self) -> &'static [Reach] {
        match self {
            Self::Text => &[
                Reach::Command(commands::BOLD),
                Reach::Command(commands::ITALIC),
                Reach::Command(commands::UNDERLINE),
                Reach::Command(commands::LINK),
                Reach::Property(KIND),
                Reach::Command(commands::COMMENT),
            ],
            Self::Page => &[],
        }
    }
}

/// The commands the actions section shows: the link, the comment, then
/// clearing the formatting.
pub const ACTION_COMMANDS: [StaticKey<Commands>; 3] = [
    commands::LINK,
    commands::COMMENT,
    commands::CLEAR_FORMATTING,
];

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

/// What a control of the text, paragraph or default styles sections asks
/// for, until the view turns it into one command (`PROSE-UI-098`).
///
/// The panel holds no selection: the view adds the one it holds, so a
/// write always reaches the selection the panel shows.
#[derive(Clone, Debug, PartialEq)]
pub enum TextWrite {
    /// A font, or `None` for the block's own.
    Font(Option<ambre_core::FontFamily>),
    /// A size, or `None` for the block's own.
    Size(Option<ambre_core::FontSize>),
    /// A text colour, or `None` for the theme's.
    Colour(Option<ambre_core::Rgba>),
    /// A highlight, or `None` for none.
    Highlight(Option<ambre_core::Rgba>),
    /// A link address, or `None` to take the link off.
    Link(Option<ambre_core::LinkUrl>),
    /// The paragraph properties the style names, and no other.
    Paragraph(crate::events::ParagraphStyle),
    /// What every paragraph of the selection becomes.
    Kind(crate::message::ParagraphKind),
    /// A default font of the document. It needs no selection.
    DefaultFont(crate::message::DefaultFont),
}

/// The line spacings the paragraph section offers. `None` is the
/// document's own.
pub const LINE_SPACINGS: [(Option<crate::events::LineHeight>, &str); 5] = {
    use crate::events::{LineHeight, LineSpacingMultiple};
    [
        (None, "Default"),
        (Some(LineHeight::Multiple(LineSpacingMultiple::SINGLE)), "1"),
        (
            Some(LineHeight::Multiple(LineSpacingMultiple::ONE_AND_FIFTEEN)),
            "1.15",
        ),
        (
            Some(LineHeight::Multiple(LineSpacingMultiple::ONE_AND_HALF)),
            "1.5",
        ),
        (Some(LineHeight::Multiple(LineSpacingMultiple::DOUBLE)), "2"),
    ]
};

/// The paragraph kinds the paragraph style section offers, in the mockup's
/// order, with their labels.
pub const KINDS: [(crate::message::ParagraphKind, &str); 5] = [
    (crate::message::ParagraphKind::Body, "Body"),
    (
        crate::message::ParagraphKind::Heading(crate::message::HeadingLevel::H1),
        "Heading 1",
    ),
    (
        crate::message::ParagraphKind::Heading(crate::message::HeadingLevel::H2),
        "Heading 2",
    ),
    (crate::message::ParagraphKind::Quote, "Quote"),
    (
        crate::message::ParagraphKind::List(crate::events::ListStyle::Bullet),
        "List",
    ),
];

/// The paragraph kinds the floating bar's list offers, with their labels.
pub const BAR_KINDS: [(crate::message::ParagraphKind, &str); 5] = [
    (crate::message::ParagraphKind::Body, "Body"),
    (
        crate::message::ParagraphKind::Heading(crate::message::HeadingLevel::H1),
        "Heading 1",
    ),
    (
        crate::message::ParagraphKind::Heading(crate::message::HeadingLevel::H2),
        "Heading 2",
    ),
    (
        crate::message::ParagraphKind::Heading(crate::message::HeadingLevel::H3),
        "Heading 3",
    ),
    (crate::message::ParagraphKind::Quote, "Quote"),
];

/// The kind a chip of [`KINDS`] shows for a reading: every list reads as
/// the one list chip.
#[must_use]
pub fn kind_chip(
    kind: Option<crate::message::ParagraphKind>,
) -> Option<crate::message::ParagraphKind> {
    match kind {
        Some(crate::message::ParagraphKind::List(_)) => Some(crate::message::ParagraphKind::List(
            crate::events::ListStyle::Bullet,
        )),
        other => other,
    }
}

/// The list markers the paragraph section offers.
pub const LISTS: [(crate::events::ListStyle, &str); 3] = [
    (crate::events::ListStyle::Bullet, "Bulleted list"),
    (
        crate::events::ListStyle::Ordered(crate::message::OrderedStyle::Decimal),
        "Numbered list",
    ),
    (
        crate::events::ListStyle::Todo(crate::message::TodoMark::Open),
        "Checklist",
    ),
];

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
                    assert!(
                        TEXT_COMMANDS.contains(id) || ACTION_COMMANDS.contains(id),
                        "{id:?} is in a section of the panel"
                    );
                }
            }
        }
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
                TEXT_COMMANDS.contains(&command.id) || ACTION_COMMANDS.contains(&command.id),
                "{:?} is in the panel",
                command.id
            );
        }
        let bar: Vec<Reach> = commands::BAR_MARKS
            .iter()
            .map(|(id, _)| Reach::Command(*id))
            .chain([
                Reach::Command(commands::LINK),
                Reach::Property(KIND),
                Reach::Command(commands::COMMENT),
            ])
            .collect();
        assert_eq!(bar, PanelKind::Text.bar());
    }
}
