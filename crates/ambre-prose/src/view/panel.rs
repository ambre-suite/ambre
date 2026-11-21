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
use ambre_core::{Commands, Mm, PanelSections, PrintLength, Properties, StaticKey};

use crate::commands;
use crate::doc::PageFormat;

/// The page, with no text selected.
pub const PAGE: StaticKey<PanelSections> = StaticKey::new("prose.page");
/// The text of the selection.
pub const TEXT: StaticKey<PanelSections> = StaticKey::new("prose.text");

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
            Self::Page => "Page",
            Self::Text => "Text",
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
}
