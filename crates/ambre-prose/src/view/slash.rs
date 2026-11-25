//! The slash menu's state, published to the chrome
//! (`features/ambre-spec-prose-slash-command.md section 3`).
//!
//! The menu is a DOM element, and this view is what knows where the caret is.
//! The slash and the query reach the document through the ordinary input
//! path (`PROSE-UI-059`). The menu remembers where the slash sits and what
//! was typed after it, and writes none of that (`PROSE-UI-087`).

use crate::events::{
    BlockInit, BlockType, HeadingLevel, ListStyle, OrderedStyle, ProseCommand, tree_id,
};
use crate::layout::run::Affinity;
use crate::message::DocChar;
use crate::ui::ProseUiState;
use crate::view::caret::Spot;
use ambre_bus::app::grid::{Col, Row};
use ambre_core::index::{Axis, IdxVec, Len, Pos, Span};
use ambre_core::{Doc, Point, Px, View};

/// The place of an item in the menu as a query filters it.
#[derive(PartialEq, Eq)]
pub enum SlashChoice {}

impl Axis for SlashChoice {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SlashChoice");
}

/// The rows of the table the menu inserts.
const TABLE_ROWS: Len<Row> = Len::<Row>::new(3);
/// The columns of the table the menu inserts.
const TABLE_COLS: Len<Col> = Len::<Col>::new(3);

/// The deepest heading a table of contents from the menu lists.
const TOC_LEVEL: HeadingLevel = HeadingLevel::H3;

/// What an item asks for. Each resolves to a command the plane declares
/// (`PROSE-UI-062`); the menu has none of its own (`PROSE-UI-088`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashCommand {
    Paragraph,
    Heading(HeadingLevel),
    List(ListStyle),
    Table,
    Code,
    Rule,
    PageBreak,
    Toc,
}

/// The item of the slash menu a command of the creation bar inserts, or
/// `None` for a command that inserts no block. The bar and the menu then
/// insert one object the same way (`PROSE-UI-062`).
#[must_use]
pub fn inserted(id: ambre_core::StaticKey<ambre_core::Commands>) -> Option<SlashCommand> {
    use crate::commands::{INSERT_SEPARATOR, INSERT_TABLE, INSERT_TEXT};
    match id {
        INSERT_TEXT => Some(SlashCommand::Paragraph),
        INSERT_TABLE => Some(SlashCommand::Table),
        INSERT_SEPARATOR => Some(SlashCommand::Rule),
        _ => None,
    }
}

/// One item of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashItem {
    pub label: ambre_commands::TitleWords,
    pub command: SlashCommand,
}

/// The twelve items, in the order the menu lists them.
pub const ITEMS: [SlashItem; 12] = [
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Text",
            fr: "Texte",
        },
        command: SlashCommand::Paragraph,
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Heading 1",
            fr: "Titre 1",
        },
        command: SlashCommand::Heading(HeadingLevel::H1),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Heading 2",
            fr: "Titre 2",
        },
        command: SlashCommand::Heading(HeadingLevel::H2),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Heading 3",
            fr: "Titre 3",
        },
        command: SlashCommand::Heading(HeadingLevel::H3),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Bullet list",
            fr: "Liste \u{e0} puces",
        },
        command: SlashCommand::List(ListStyle::Bullet),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Numbered list",
            fr: "Liste num\u{e9}rot\u{e9}e",
        },
        command: SlashCommand::List(ListStyle::Ordered(OrderedStyle::Decimal)),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Task list",
            fr: "Liste de t\u{e2}ches",
        },
        command: SlashCommand::List(ListStyle::Todo(crate::events::TodoMark::Open)),
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Table",
            fr: "Tableau",
        },
        command: SlashCommand::Table,
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Code",
            fr: "Code",
        },
        command: SlashCommand::Code,
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Separator",
            fr: "S\u{e9}parateur",
        },
        command: SlashCommand::Rule,
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Page break",
            fr: "Saut de page",
        },
        command: SlashCommand::PageBreak,
    },
    SlashItem {
        label: ambre_commands::TitleWords {
            en: "Table of contents",
            fr: "Table des mati\u{e8}res",
        },
        command: SlashCommand::Toc,
    },
];

impl SlashCommand {
    /// The one command this item sends, addressed to the block the menu
    /// recorded (`PROSE-UI-174`). A heading turns that block; every other
    /// item lands after it.
    #[must_use]
    pub fn resolve(self, origin: crate::events::BlockId) -> ProseCommand {
        let after = |init: BlockInit| ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(origin),
            init,
            style_id: None,
        };
        match self {
            Self::Paragraph => after(BlockInit::Paragraph),
            Self::Heading(level) => ProseCommand::SetHeadingLevel {
                block_id: origin,
                level: Some(level),
            },
            Self::List(style) => after(BlockInit::ListItem { style }),
            Self::Table => ProseCommand::InsertTable {
                parent_id: None,
                after_id: Some(origin),
                rows: TABLE_ROWS,
                cols: TABLE_COLS,
            },
            Self::Code => after(BlockInit::CodeBlock { lang: None }),
            Self::Rule => after(BlockInit::HRule),
            Self::PageBreak => after(BlockInit::PageBreak),
            Self::Toc => ProseCommand::InsertToc {
                parent_id: None,
                after_id: Some(origin),
                max_level: TOC_LEVEL,
                numbered: crate::events::TocNumbering::Plain,
            },
        }
    }
}

/// The items a query keeps: those whose label holds it, case folded, in the
/// menu's order.
#[must_use]
pub fn filter(query: &str) -> IdxVec<SlashChoice, SlashItem> {
    let query = query.to_lowercase();
    ITEMS
        .into_iter()
        .filter(|item| {
            ambre_commands::shown(&item.label)
                .to_lowercase()
                .contains(&query)
        })
        .collect()
}

/// The open menu, as the view keeps it. The query is what the user typed,
/// so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SlashMenu {
    /// The block the slash was typed in, and the offset of the slash
    /// (`PROSE-UI-091`).
    pub(crate) origin: Spot,
    /// What was typed after the slash.
    pub(crate) query: String,
    pub(crate) selected: Pos<SlashChoice>,
}

impl std::fmt::Debug for SlashMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlashMenu")
            .field("origin", &self.origin)
            .field("query", &ambre_core::Redacted::new(&self.query))
            .field("selected", &self.selected)
            .finish()
    }
}

impl SlashMenu {
    /// The range the slash and the query take in the origin block.
    pub(crate) fn span(&self) -> Span<DocChar> {
        let start = self.origin.1;
        Span::<DocChar>::new(
            start,
            start + Len::<DocChar>::ONE + ambre_core::char_len(&self.query).as_axis::<DocChar>(),
        )
    }
}

/// What the menu shows, rebuilt on every frame. The query is what the user
/// typed, so `Debug` prints its length alone.
#[derive(Clone, Default, PartialEq)]
pub struct SlashMenuState {
    /// Under the slash, on the canvas. `None` hides the menu.
    pub anchor: Option<Point<Px<View>>>,
    pub query: String,
    pub items: IdxVec<SlashChoice, &'static str>,
    pub selected: Pos<SlashChoice>,
}

impl std::fmt::Debug for SlashMenuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlashMenuState")
            .field("anchor", &self.anchor)
            .field("query", &ambre_core::Redacted::new(&self.query))
            .field("items", &self.items)
            .field("selected", &self.selected)
            .finish()
    }
}

impl SlashMenuState {
    #[must_use]
    pub fn visible(&self) -> bool {
        self.anchor.is_some()
    }
}

impl ProseUiState {
    /// The menu as it stands. Only the author who opened it sees it: it is
    /// view state and never reaches the document (`PROSE-UI-090`).
    #[must_use]
    pub fn slash_menu(&self) -> SlashMenuState {
        let Some(menu) = &self.slash else {
            return SlashMenuState::default();
        };
        SlashMenuState {
            anchor: self.under(menu.origin),
            query: menu.query.clone(),
            items: filter(&menu.query)
                .iter()
                .map(|item| ambre_commands::shown(&item.label))
                .collect(),
            selected: menu.selected,
        }
    }

    /// Whether a slash typed at this place opens the menu. It opens at the
    /// start of a paragraph or a heading of the flow, where a block command
    /// is legal (`PROSE-UI-058`).
    pub(crate) fn opens_menu(&self, spot: Spot) -> bool {
        if spot.1 != Pos::<DocChar>::ZERO || self.is_band(spot.0) {
            return false;
        }
        self.flow_index(&tree_id(spot.0))
            .and_then(|index| self.shaped.block(index))
            .is_some_and(|block| {
                matches!(block.block_type, BlockType::Paragraph | BlockType::Heading)
            })
    }

    /// Move the menu's selection by one, wrapping. Nothing is sent
    /// (`PROSE-UI-089`).
    pub(crate) fn step_menu(&mut self, direction: ambre_core::StepDirection) {
        let Some(menu) = &mut self.slash else {
            return;
        };
        let count = filter(&menu.query).len();
        if count.is_zero() {
            return;
        }
        let (selected, count) = (menu.selected.get(), count.get());
        menu.selected =
            Pos::<SlashChoice>::new(if direction == ambre_core::StepDirection::Forward {
                (selected + 1) % count
            } else {
                (selected + count - 1) % count
            });
    }

    /// The bottom of the line holding a place, on the canvas.
    fn under(&self, (block, offset): Spot) -> Option<Point<Px<View>>> {
        let run = self.run_of(block)?;
        let caret = run.caret_box_at(self.to_laid(block, offset), Affinity::Downstream);
        let (x, y0, y1) = (caret.x0 as f32, caret.y0 as f32, caret.y1 as f32);
        let (page, piece) = self.fragment_at(block, Px::<Doc>::new(y0))?;
        let bottom = Point::new(Px::<Doc>::new(x), Px::<Doc>::new(y1));
        Some(self.to_screen(block, page, piece, bottom))
    }
}

#[cfg(test)]
mod language_tests {
    use super::{ITEMS, filter};
    use ambre_commands::{Locale, show_locale};

    /// `SETTINGS-044`: a word of the Prose chrome shows in the interface
    /// language of the page, and the slash menu finds an item by it.
    #[test]
    fn the_slash_menu_reads_and_finds_its_items_in_the_interface_language() {
        show_locale(Locale::Fr);
        let shown: Vec<&str> = ITEMS
            .iter()
            .map(|item| ambre_commands::shown(&item.label))
            .collect();
        assert!(shown.contains(&"Titre 1"), "{shown:?}");
        assert!(
            filter("puces")
                .iter()
                .any(|item| item.label.en == "Bullet list")
        );
        let notice = crate::view::annotate::LostReply::Deleted.notice();
        assert!(notice.starts_with("Un collaborateur"), "{notice}");
        show_locale(Locale::En);
        assert!(
            filter("bullet")
                .iter()
                .any(|item| item.label.en == "Bullet list")
        );
        assert!(
            crate::view::annotate::LostReply::Deleted
                .notice()
                .starts_with("A collaborator")
        );
    }
}
