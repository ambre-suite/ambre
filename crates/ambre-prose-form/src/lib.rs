//! Prose's intermediate form: a document in flight between a format and the
//! block tree (`4-Prose/features/ambre-spec-prose-import-export.md section 3`).
//!
//! The form lives in a crate of its own so that the Office converter module
//! can link it. A module links no application crate. So the Word mapping,
//! behind the `word` feature, runs in the module and never in the Main
//! instance, whose download is held to its ceiling. Prose re-exports every
//! item at its old path, and its readers, writers, walk and builder meet the
//! form there.
//!
//! The form crosses the module boundary as postcard bytes ([`FormBytes`]).
//! It nests a list item and a table cell, and each reader caps the nesting,
//! so postcard recurses a bounded depth.

pub mod clip;
mod kinds;
pub mod link;
mod report;
#[cfg(feature = "word")]
pub mod word_read;
#[cfg(feature = "word")]
pub mod word_write;
#[cfg(feature = "word")]
pub mod zone;

pub use ambre_docx::WordConstruct;
pub use kinds::*;
pub use report::{FormConstruct, FormEntry, FormOccurrences, FormOutcome, FormReport};

use ambre_core::index::{Axis, Pos};
use ambre_core::{
    Authored, DisplayName, Instant, Ordinal, Payload, PayloadKind, Slant, Strikethrough, Underline,
    Validated, Weight,
};
use serde::{Deserialize, Serialize};

ambre_core::define_flag! {
    /// Whether a paragraph, a cell or a run carries a look of its own that
    /// a format may not keep.
    pub enum OwnStyle {
        /// It carries no look of its own.
        Plain,
        /// It carries a look of its own.
        Styled,
    }

    /// Whether a table merges two or more cells into one.
    pub enum Merging {
        /// Every cell stands alone.
        Unmerged,
        /// Two or more cells are one.
        Merged,
    }

    /// Whether a document is laid out on pages.
    pub enum Paging {
        /// The text flows with no page format.
        Flow,
        /// The document has a page format.
        Paged,
    }

    /// Whether a document draws a header or a footer.
    pub enum Banding {
        /// Neither band is on.
        Bare,
        /// A header or a footer is on.
        Banded,
    }

    /// Whether a run is a cross reference.
    pub enum Referencing {
        /// The run is text.
        Text,
        /// The run shows a cross reference.
        Reference,
    }
}

/// A document in flight between a format and the block tree. Held for the
/// length of one conversion and never serialized.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProseIr {
    pub meta: IrMeta,
    pub blocks: Vec<IrBlock>,
    /// The footnote bodies, in the order of their ordinals.
    pub footnotes: Vec<IrFootnote>,
    /// The comment threads, out of band from the blocks: a run names the
    /// threads it lies under by their place here.
    pub comments: Vec<IrComment>,
    /// The suggestions, out of band from the blocks: a run names the one it
    /// proposes by its place here.
    pub suggestions: Vec<IrSuggestion>,
}

impl ProseIr {
    /// The same form without its review: no thread and no suggestion. Each
    /// run keeps the text as it stood before the review. A suggested
    /// insertion is left out, and a suggested deletion stays as plain text
    /// (`PROSE-IO-081`).
    #[must_use]
    pub fn without_review(mut self) -> Self {
        self.comments.clear();
        self.suggestions.clear();
        // A walk over the blocks with its own stack: a list nests.
        let mut stack: Vec<&mut IrBlock> = self.blocks.iter_mut().collect();
        while let Some(block) = stack.pop() {
            match block {
                IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => {
                    settle(runs);
                }
                IrBlock::ListItem { runs, children, .. } => {
                    settle(runs);
                    stack.extend(children.iter_mut());
                }
                IrBlock::Table { rows, .. } => stack.extend(
                    rows.iter_mut()
                        .flat_map(|row| row.cells.iter_mut())
                        .flat_map(|cell| cell.blocks.iter_mut()),
                ),
                IrBlock::CodeBlock { .. }
                | IrBlock::Image { .. }
                | IrBlock::PageBreak
                | IrBlock::HRule
                | IrBlock::Toc => {}
            }
        }
        for note in &mut self.footnotes {
            settle(&mut note.runs);
        }
        self
    }
}

/// The runs as they stood before the review: no thread, no insertion, and
/// every deletion back as text.
fn settle(runs: &mut Vec<IrRun>) {
    runs.retain(|run| run.insertion.is_none());
    for run in runs {
        run.comments.clear();
        run.deletion = None;
    }
}

/// The places of [`ProseIr::comments`]: a [`Pos<IrThread>`] names a thread
/// of one form, never one of the document. The builder mints the
/// document's id of each.
#[derive(PartialEq, Eq)]
pub enum IrThread {}

impl Axis for IrThread {
    const NAME: &'static str = "IrThread";
}

/// The places of [`ProseIr::suggestions`], as [`IrThread`] is for threads.
#[derive(PartialEq, Eq)]
pub enum IrChange {}

impl Axis for IrChange {
    const NAME: &'static str = "IrChange";
}

/// A comment thread. The commented range is the runs that name it, never a
/// field here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrComment {
    pub resolved: ambre_core::ThreadState,
    /// The opening body first, then the replies.
    pub replies: Vec<IrReply>,
}

/// One body of a thread. The author is the name the file gives, and the
/// text is content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrReply {
    pub author: Option<Validated<DisplayName>>,
    /// The time the file gives, or `None` when it gives none. The builder
    /// then dates the reply at the import, and the form keeps that the date
    /// is unknown.
    pub created_at: Option<Instant<Authored>>,
    pub text: String,
}

impl std::fmt::Debug for IrReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrReply")
            .field("author", &self.author)
            .field("created_at", &self.created_at)
            .field("text", &ambre_core::Redacted::new(&self.text))
            .finish()
    }
}

/// A suggestion: the edit it proposes and who proposed it. The text it
/// covers is the runs that name it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrSuggestion {
    pub kind: SuggestionKind,
    pub author: Option<Validated<DisplayName>>,
    /// The time the file gives. Prose stores none, so an export writes none.
    pub date: Option<Instant<Authored>>,
}

/// What the document carries beside its blocks. The title is content, so
/// `Debug` prints its length alone.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrMeta {
    pub title: Option<String>,
    /// The document has a page format. Every Prose document has one, and a
    /// format without pages reports it (`PROSE-IO-018`).
    pub paged: Paging,
    /// The document has a header or a footer.
    pub bands: Banding,
}

impl std::fmt::Debug for IrMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrMeta")
            .field("title", &self.title.as_ref().map(ambre_core::Redacted::new))
            .field("paged", &self.paged)
            .field("bands", &self.bands)
            .finish()
    }
}

/// One block. Depth is the nesting, never a number. The text of a code block
/// and the alternative text of an image are content. So `Debug` prints their
/// length alone, and a run prints its own the same way.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum IrBlock {
    Paragraph {
        /// The paragraph carries a named style, or alignment, indents or
        /// spacing of its own.
        styled: OwnStyle,
        runs: Vec<IrRun>,
    },
    /// The level is the stored one, unclamped (`PROSE-FEAT-091`).
    Heading {
        level: StoredLevel,
        runs: Vec<IrRun>,
    },
    ListItem {
        style: IrListStyle,
        runs: Vec<IrRun>,
        children: Vec<IrBlock>,
    },
    Table {
        /// Two or more cells are merged into one.
        merged: Merging,
        rows: Vec<IrTableRow>,
    },
    CodeBlock {
        lang: Option<Validated<CodeLang>>,
        text: String,
    },
    Image {
        alt: String,
    },
    PageBreak,
    HRule,
    Toc,
}

impl std::fmt::Debug for IrBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::Redacted;
        match self {
            Self::Paragraph { styled, runs } => f
                .debug_struct("Paragraph")
                .field("styled", styled)
                .field("runs", runs)
                .finish(),
            Self::Heading { level, runs } => f
                .debug_struct("Heading")
                .field("level", level)
                .field("runs", runs)
                .finish(),
            Self::ListItem {
                style,
                runs,
                children,
            } => f
                .debug_struct("ListItem")
                .field("style", style)
                .field("runs", runs)
                .field("children", children)
                .finish(),
            Self::Table { merged, rows } => f
                .debug_struct("Table")
                .field("merged", merged)
                .field("rows", rows)
                .finish(),
            Self::CodeBlock { lang, text } => f
                .debug_struct("CodeBlock")
                .field(
                    "lang",
                    &lang.as_ref().map(|lang| Redacted::new(lang.as_str())),
                )
                .field("text", &Redacted::new(text))
                .finish(),
            Self::Image { alt } => f
                .debug_struct("Image")
                .field("alt", &Redacted::new(alt))
                .finish(),
            Self::PageBreak => f.write_str("PageBreak"),
            Self::HRule => f.write_str("HRule"),
            Self::Toc => f.write_str("Toc"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IrTableCell {
    /// The cell carries a fill, a border, a padding or a vertical alignment.
    pub styled: OwnStyle,
    pub blocks: Vec<IrBlock>,
}

/// One footnote body. A run points at it by ordinal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IrFootnote {
    pub ordinal: Ordinal<FootnoteCall>,
    pub runs: Vec<IrRun>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrListStyle {
    Bullet,
    Ordered(IrOrderedStyle),
    Todo(TodoMark),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrOrderedStyle {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// A run of text and what it carries. The text is content, so `Debug`
/// prints its length alone.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrRun {
    pub text: String,
    pub bold: Weight,
    pub italic: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    pub superscript: Superscript,
    pub subscript: Subscript,
    pub code: InlineCode,
    pub link: Option<ambre_core::LinkUrl>,
    /// A colour, a background, a size or a family.
    pub styled: OwnStyle,
    /// The threads whose anchor covers the run.
    pub comments: Vec<Pos<IrThread>>,
    /// The suggested insertion the run is part of.
    pub insertion: Option<Pos<IrChange>>,
    /// The suggested deletion the run is part of.
    pub deletion: Option<Pos<IrChange>>,
    pub cross_ref: Referencing,
    /// The ordinal of the footnote this run calls.
    pub footnote_ref: Option<Ordinal<FootnoteCall>>,
}

impl IrRun {
    /// A run of plain text.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for IrRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IrRun")
            .field("text", &ambre_core::Redacted::new(&self.text))
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("underline", &self.underline)
            .field("strikethrough", &self.strikethrough)
            .field("superscript", &self.superscript)
            .field("subscript", &self.subscript)
            .field("code", &self.code)
            .field("link", &self.link)
            .field("styled", &self.styled)
            .field("comments", &self.comments)
            .field("insertion", &self.insertion)
            .field("deletion", &self.deletion)
            .field("cross_ref", &self.cross_ref)
            .field("footnote_ref", &self.footnote_ref)
            .finish()
    }
}

/// The role of the form, or of a report of the mapping, as it crosses the
/// module boundary: postcard bytes. `Payload<FormBytes>`.
pub enum FormBytes {}

impl PayloadKind for FormBytes {
    const NAME: &'static str = "FormBytes";
}

/// Why bytes that crossed the module boundary hold no form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the converter answered bytes that hold no form")]
pub struct FormDecodeError;

/// The postcard bytes of `value`, one of the shapes that cross.
///
/// # Errors
///
/// [`FormDecodeError`] when postcard cannot write the value, which a value
/// built from these types never triggers.
pub fn encode<T: Serialize>(value: &T) -> Result<Payload<FormBytes>, FormDecodeError> {
    postcard::to_allocvec(value)
        .map(Payload::new)
        .map_err(|_| FormDecodeError)
}

/// The value `bytes` hold.
///
/// # Errors
///
/// [`FormDecodeError`] for bytes that decode to no value of `T`.
pub fn decode<T: for<'de> Deserialize<'de>>(
    bytes: &Payload<FormBytes>,
) -> Result<T, FormDecodeError> {
    postcard::from_bytes(bytes.as_bytes()).map_err(|_| FormDecodeError)
}
