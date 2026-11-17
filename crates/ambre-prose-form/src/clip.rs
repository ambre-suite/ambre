//! The blocks a paste writes, in the bytes this application's own copy
//! writes (`4-Prose/features/ambre-spec-prose-clipboard.md`).
//!
//! The markup module reads the HTML half of a clipboard into these blocks.
//! The Main instance thus links neither the walk nor the style parser
//! (`ambre-spec-budgets.md section 5`). Prose writes them from a copy and
//! reads them back on a paste. Every mark is checked again on that paste, so a
//! block proves nothing about the page or the copy that wrote it.

use ambre_core::{Count, Octets, Validated};
use serde::{Deserialize, Serialize};

use crate::{CodeLang, HeadingLevel, MarkKey, OrderedStyle, TodoMark};

/// One block of a selection: its type and the runs of text it covered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipBlock {
    pub kind: ClipKind,
    pub runs: Vec<ClipRun>,
}

/// The block types a copy keeps. Any other block with a body pastes as a
/// paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipKind {
    Paragraph,
    Heading(HeadingLevel),
    Bullet,
    Ordered(OrderedStyle),
    Todo(TodoMark),
    /// A code block, `None` for a plain one. The payload spells it as the
    /// text the block stores, the empty text for a plain block.
    Code(#[serde(with = "stored_lang")] Option<Validated<CodeLang>>),
}

/// The code language as the payload spells it: the stored text, so the bytes
/// stay those a payload of a bare string wrote.
mod stored_lang {
    use crate::CodeLang;
    use ambre_core::Validated;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[expect(
        clippy::ref_option,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(
        lang: &Option<Validated<CodeLang>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        lang.as_ref()
            .map_or("", Validated::as_str)
            .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Validated<CodeLang>>, D::Error> {
        String::deserialize(deserializer).map(|tag| Validated::<CodeLang>::parse(tag).ok())
    }
}

/// One item the HTML half of a paste reads into: a block, or a table whose
/// cells hold blocks (`PROSE-UI-225`). A cell holds no table, so the type
/// nests one level alone and postcard recurses a bounded depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipItem {
    Block(ClipBlock),
    Table(ClipTable),
}

impl ClipItem {
    /// The blocks the item reads as when no table may land: the item's own
    /// block, or one block or more per cell, in reading order.
    #[must_use]
    pub fn flattened(self) -> Vec<ClipBlock> {
        match self {
            Self::Block(block) => vec![block],
            Self::Table(table) => table
                .rows
                .into_iter()
                .flat_map(|row| row.cells)
                .filter_map(|cell| match cell {
                    ClipCell::Own(blocks) => Some(blocks),
                    ClipCell::Covered(_) => None,
                })
                .flatten()
                .collect(),
        }
    }
}

/// A pasted table, laid out on its grid: every row holds one cell per
/// column. A merged cell is one [`ClipCell::Own`], and each slot it covers is
/// a [`ClipCell::Covered`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipTable {
    pub rows: Vec<ClipRow>,
}

/// One row of a pasted table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipRow {
    pub cells: Vec<ClipCell>,
}

/// One slot of a pasted table's grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipCell {
    /// A cell of its own, with the blocks it holds. It may hold none.
    Own(Vec<ClipBlock>),
    /// A slot a merged cell covers.
    Covered(ClipCover),
}

/// Which way a covered slot reaches the cell that covers it. A slot in the
/// merged cell's own row points left; a slot in a row below points up. So
/// the extent of a merge reads off the grid alone, with no count stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipCover {
    Left,
    Above,
}

/// A run of text and the formatting marks it carries. Anchors stay behind:
/// a comment or a suggestion does not travel with its text. The text is
/// content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipRun {
    pub text: String,
    pub marks: Vec<(MarkKey, ClipValue)>,
}

impl std::fmt::Debug for ClipRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipRun")
            .field("text", &ambre_core::Redacted::new(&self.text))
            .field("marks", &self.marks)
            .finish()
    }
}

/// A mark's value, in the shapes the registry writes. A text value may be a
/// link or a family the user typed, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl std::fmt::Debug for ClipValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(on) => f.debug_tuple("Bool").field(on).finish(),
            Self::Int(n) => f.debug_tuple("Int").field(n).finish(),
            Self::Float(x) => f.debug_tuple("Float").field(x).finish(),
            Self::Text(text) => f
                .debug_tuple("Text")
                .field(&ambre_core::Redacted::new(text))
                .finish(),
        }
    }
}

/// The longest family name a paste keeps, in bytes.
pub const FAMILY_MAX: Count<Octets> = Count::<Octets>::new(64);

/// A link a paste keeps: one a reader follows to a page or a mail, never one
/// that runs.
#[must_use]
pub fn safe_link(href: &str) -> bool {
    let lower = href.get(..8).unwrap_or(href).to_ascii_lowercase();
    crate::link::within_href_max(Count::<Octets>::of(href.len()))
        && ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
}

/// A family name a paste keeps: not empty, within [`FAMILY_MAX`], and free of
/// a control character or one that closes a style declaration.
#[must_use]
pub fn safe_family(name: &str) -> bool {
    !name.is_empty()
        && Count::<Octets>::of(name.len()) <= FAMILY_MAX
        && name
            .chars()
            .all(|c| !c.is_control() && !matches!(c, '<' | '>' | ';' | '"'))
}
