// ambre-toile/src/ui/panel.rs
// ============================================================================
// What the board's context panel shows, and what the context bar repeats
// (`ambre-spec-shell.md` section 2). Pure, so the host tier reads it.
//
// The kind of the selection picks the sections. Nothing selected shows the
// board. Elements show a shape's style and arrangement. An arrow shows a
// connector's style. The bar is a declared list per kind, each entry a control the
// panel shows for that kind (`CHROME-208`).
// ============================================================================

use ambre_commands::{Locale, Property, Reach};
use ambre_core::{PanelSections, Properties, StaticKey};

use super::menu;
use super::style::{StyleModel, Styled};

/// The board, with nothing selected.
pub const BOARD: StaticKey<PanelSections> = StaticKey::new("toile.board");
/// The fill, the stroke and the opacity of the selected shapes.
pub const SHAPE: StaticKey<PanelSections> = StaticKey::new("toile.shape");
/// The text of the selected shapes.
pub const TEXT: StaticKey<PanelSections> = StaticKey::new("toile.text");
/// The order, the grouping, the lock and the alignment of the selection.
pub const ARRANGE: StaticKey<PanelSections> = StaticKey::new("toile.arrange");
/// The stroke of the selected connector.
pub const CONNECTOR: StaticKey<PanelSections> = StaticKey::new("toile.connector");

/// The fill of a shape.
pub const FILL: StaticKey<Properties> = StaticKey::new("toile.style.fill");
/// The stroke colour of a shape or a connector.
pub const STROKE: StaticKey<Properties> = StaticKey::new("toile.style.stroke");
/// The stroke width of a shape or a connector.
pub const STROKE_WIDTH: StaticKey<Properties> = StaticKey::new("toile.style.stroke_width");
/// The opacity of an element.
pub const OPACITY: StaticKey<Properties> = StaticKey::new("toile.style.opacity");
/// The colour of a text.
pub const TEXT_COLOUR: StaticKey<Properties> = StaticKey::new("toile.style.text_colour");
/// The font family of a text.
pub const FONT: StaticKey<Properties> = StaticKey::new("toile.style.font");
/// The size of a text.
pub const SIZE: StaticKey<Properties> = StaticKey::new("toile.style.size");
/// The weight of a text.
pub const BOLD: StaticKey<Properties> = StaticKey::new("toile.style.bold");
/// The slant of a text.
pub const ITALIC: StaticKey<Properties> = StaticKey::new("toile.style.italic");
/// The underline of a text.
pub const UNDERLINE: StaticKey<Properties> = StaticKey::new("toile.style.underline");
/// The strikethrough of a text.
pub const STRIKETHROUGH: StaticKey<Properties> = StaticKey::new("toile.style.strikethrough");

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

        /// Every property the board's panel edits, described once.
        pub const PROPERTIES: &[Property] = &[
            $(Property { id: $id, title: titles::$title_fn, icon: None },)*
        ];
    };
}

properties! {
    FILL: FILL, "Fill";
    STROKE: STROKE, "Stroke";
    STROKE_WIDTH: STROKE_WIDTH, "Stroke width";
    OPACITY: OPACITY, "Opacity";
    TEXT_COLOUR: TEXT_COLOUR, "Text colour";
    FONT: FONT, "Font";
    SIZE: SIZE, "Size";
    BOLD: BOLD, "Bold";
    ITALIC: ITALIC, "Italic";
    UNDERLINE: UNDERLINE, "Underline";
    STRIKETHROUGH: STRIKETHROUGH, "Strikethrough";
}

/// The title of a property no table describes.
fn unnamed_title(_: &Locale) -> ambre_core::Validated<ambre_commands::CommandTitle> {
    ambre_core::Validated::new("Property")
}

/// What [`property`] answers for an id no table describes. Every id the
/// panel names is in the table, which a test holds.
const UNNAMED: Property = Property {
    id: StaticKey::new("toile.unnamed"),
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
    /// Nothing is selected: the panel shows the board.
    Board,
    /// One or more elements.
    Shape,
    /// One arrow.
    Connector,
}

impl PanelKind {
    /// Every kind, for a test that walks them all.
    pub const ALL: [Self; 3] = [Self::Board, Self::Shape, Self::Connector];

    /// The kind of the selection the style model describes.
    #[must_use]
    pub fn of(model: Option<&StyleModel>) -> Self {
        match model.map(|m| m.kind) {
            None => Self::Board,
            Some(Styled::Elements) => Self::Shape,
            Some(Styled::Arrow) => Self::Connector,
        }
    }

    /// The header of the panel for this kind.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Board => "Board",
            Self::Shape => "Selection",
            Self::Connector => "Connector",
        }
    }

    /// The sections the panel shows for this kind, in order.
    #[must_use]
    pub fn sections(self) -> &'static [StaticKey<PanelSections>] {
        match self {
            Self::Board => &[BOARD],
            Self::Shape => &[SHAPE, TEXT, ARRANGE],
            Self::Connector => &[CONNECTOR],
        }
    }

    /// The entries of the context bar for this kind (`CHROME-208`): the
    /// fill, the text colour, duplicate, delete, and the lock.
    #[must_use]
    pub fn bar(self) -> &'static [Reach] {
        match self {
            Self::Shape => &[
                Reach::Property(FILL),
                Reach::Property(TEXT_COLOUR),
                Reach::Command(menu::DUPLICATE),
                Reach::Command(menu::DELETE),
                Reach::Command(menu::LOCK),
                Reach::Command(menu::UNLOCK),
            ],
            Self::Board | Self::Connector => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_bar_reaches_the_panel_and_holds_no_clipboard() {
        for kind in PanelKind::ALL {
            assert_eq!(
                ambre_commands::bar_defects(
                    kind.bar(),
                    super::super::commands::COMMANDS,
                    PROPERTIES
                ),
                vec![],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn no_selection_shows_the_board() {
        assert_eq!(PanelKind::of(None), PanelKind::Board);
        assert_eq!(PanelKind::Board.sections(), &[BOARD]);
    }
}
