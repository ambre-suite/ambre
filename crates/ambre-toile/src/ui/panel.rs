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
use ambre_core::{Count, PanelSections, Properties, StaticKey};

use crate::message::Elements;

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
            Self::Board => "Document",
            Self::Shape => "Selection",
            Self::Connector => "Selection",
        }
    }

    /// The subtitle of the panel for this kind (`CHROME-241`): the elements
    /// the selection holds, as the Logic thread last counted them. A count
    /// not read yet shows nothing.
    #[must_use]
    pub fn subtitle(self, count: Option<Count<Elements>>) -> String {
        match self {
            Self::Board => ambre_commands::NOTHING_SELECTED.to_owned(),
            Self::Connector => "1 connector".to_owned(),
            Self::Shape => count.map_or_else(String::new, |n| {
                let n = n.get();
                format!("{n} {}", if n == 1 { "element" } else { "elements" })
            }),
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

    /// The subtitle counts what the selection holds, and reads nothing
    /// selected over the board alone (`CHROME-241`).
    #[test]
    fn the_subtitle_counts_the_selection() {
        assert_eq!(PanelKind::Board.subtitle(None), "nothing selected");
        assert_eq!(PanelKind::Shape.subtitle(Some(Count::new(1))), "1 element");
        assert_eq!(PanelKind::Shape.subtitle(Some(Count::new(4))), "4 elements");
        assert_eq!(PanelKind::Shape.subtitle(None), "");
        assert_eq!(PanelKind::Connector.subtitle(None), "1 connector");
    }

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

#[cfg(test)]
mod menu_tests {
    use ambre_commands::Reach;
    use ambre_core::Count;

    use super::*;
    use crate::message::Elements;
    use crate::ui::menu::{ClickSite, CtxScope, FrameHeld, GroupHeld, LockMix, context_menu_ids};

    /// Every scope the board's menu knows, each condition on and off.
    pub(crate) fn scopes() -> Vec<CtxScope> {
        let mut out = vec![CtxScope::default()];
        for locks in [LockMix::Unlocked, LockMix::Locked, LockMix::Both] {
            for count in [1, 2] {
                out.push(CtxScope {
                    site: ClickSite::Selection,
                    count: Count::<Elements>::new(count),
                    groups: GroupHeld::Group,
                    locks,
                    framed: FrameHeld::Framed,
                    ..CtxScope::default()
                });
            }
        }
        out
    }

    /// The board's context menu lists its clipboard entries first, in the
    /// order cut, copy, paste, then commands of the panel alone
    /// (`CHROME-179`, `CHROME-182`).
    #[test]
    fn the_board_menu_opens_on_the_clipboard_then_reaches_the_panel() {
        for scope in scopes() {
            let menu: Vec<Reach> = context_menu_ids(&scope)
                .into_iter()
                .flatten()
                .map(Reach::Command)
                .collect();
            assert_eq!(
                ambre_commands::menu_defects(&menu, crate::ui::commands::COMMANDS, PROPERTIES),
                vec![],
                "{scope:?}"
            );
        }
    }

    /// Cut, copy and paste live in the context menu (`CHROME-178`).
    #[test]
    fn the_clipboard_homes_in_the_menu() {
        use ambre_commands::{Home, shared};
        for id in [shared::CUT, shared::COPY, shared::PASTE] {
            let command =
                ambre_commands::find(crate::ui::commands::COMMANDS, id).expect("declared");
            assert_eq!(command.home, Home::ContextMenu, "{id:?}");
        }
    }

    /// The board formats its text through the properties of the text section
    /// alone: no command of the format group lacks a control there
    /// (`CHROME-166`).
    #[test]
    fn the_board_formats_through_the_text_section() {
        let format = crate::ui::commands::COMMANDS
            .iter()
            .filter(|c| c.category.as_str() == "format")
            .count();
        assert_eq!(format, 0, "the board's formatting is properties");
        for id in [
            BOLD,
            ITALIC,
            UNDERLINE,
            STRIKETHROUGH,
            FONT,
            SIZE,
            TEXT_COLOUR,
        ] {
            assert_eq!(property(id).id, id, "{id:?} is declared");
        }
    }
}
