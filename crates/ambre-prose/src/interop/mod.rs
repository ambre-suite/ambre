//! Import and export through the intermediate form
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! This module holds the intermediate form and the readers and writers over
//! it. None of them sees the document: no item here names a `LoroDoc` or the
//! engine. So a reader cannot emit a command (`PROSE-IO-009`), and a writer
//! cannot read the document (`PROSE-IO-045`). A reader and a writer meet in
//! [`ProseIr`] alone (`PROSE-IO-044`). The walk that fills the form and the
//! builder that drains it live beside the engine, in `logic::interop`.
//!
//! The form holds the constructs a landed format reads or writes. A field
//! joins it with the first reader or writer that needs it.

pub mod markdown_read;
pub mod markdown_write;

use ambre_bus::app::prose::{
    CodeLang, ConversionFormat, ConversionLosses, ConversionReport, ConversionWarning,
    DroppedElement, FootnoteCall, LiftedBlock, UnsupportedFeature,
};

use ambre_core::{Ordinal, Slant, Strikethrough, Tally, Underline, Weight};

use crate::doc::StoredLevel;
use crate::mark::{InlineCode, Subscript, Superscript};

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

    /// Whether a run lies under a comment's anchor.
    pub enum Commenting {
        /// No comment anchors on the run.
        Uncommented,
        /// A comment anchors on the run.
        Commented,
    }

    /// Whether a run is a suggested insertion or deletion.
    pub enum Suggesting {
        /// The run is settled text.
        Settled,
        /// The run is a suggestion awaiting a decision.
        Suggested,
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProseIr {
    pub meta: IrMeta,
    pub blocks: Vec<IrBlock>,
    /// The footnote bodies, in the order of their ordinals.
    pub footnotes: Vec<IrFootnote>,
}

/// What the document carries beside its blocks. The title is content, so
/// `Debug` prints its length alone.
#[derive(Clone, Default, PartialEq, Eq)]
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
            .field("title", &crate::redact::optional(self.title.as_ref()))
            .field("paged", &self.paged)
            .field("bands", &self.bands)
            .finish()
    }
}

/// One block. Depth is the nesting, never a number. The text of a code block
/// and the alternative text of an image are content. So `Debug` prints their
/// length alone, and a run prints its own the same way.
#[derive(Clone, PartialEq)]
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
        lang: Option<CodeLang>,
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
        use crate::redact::Redacted;
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
                .field("lang", &lang.as_ref().map(|lang| Redacted(lang.as_str())))
                .field("text", &Redacted(text))
                .finish(),
            Self::Image { alt } => f
                .debug_struct("Image")
                .field("alt", &Redacted(alt))
                .finish(),
            Self::PageBreak => f.write_str("PageBreak"),
            Self::HRule => f.write_str("HRule"),
            Self::Toc => f.write_str("Toc"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableCell {
    /// The cell carries a fill, a border, a padding or a vertical alignment.
    pub styled: OwnStyle,
    pub blocks: Vec<IrBlock>,
}

/// One footnote body. A run points at it by ordinal.
#[derive(Clone, Debug, PartialEq)]
pub struct IrFootnote {
    pub ordinal: Ordinal<FootnoteCall>,
    pub runs: Vec<IrRun>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrListStyle {
    Bullet,
    Ordered(IrOrderedStyle),
    Todo(crate::events::TodoMark),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrOrderedStyle {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// A run of text and what it carries. The text is content, so `Debug`
/// prints its length alone.
#[derive(Clone, Default, PartialEq, Eq)]
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
    pub comment: Commenting,
    pub suggestion: Suggesting,
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
            .field("text", &crate::redact::Redacted(&self.text))
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("underline", &self.underline)
            .field("strikethrough", &self.strikethrough)
            .field("superscript", &self.superscript)
            .field("subscript", &self.subscript)
            .field("code", &self.code)
            .field("link", &self.link)
            .field("styled", &self.styled)
            .field("comment", &self.comment)
            .field("suggestion", &self.suggestion)
            .field("cross_ref", &self.cross_ref)
            .field("footnote_ref", &self.footnote_ref)
            .finish()
    }
}

/// The warnings of one conversion, each construct once with its count, in the
/// order they were first met.
#[derive(Debug)]
pub struct Warnings {
    format: ConversionFormat,
    list: Vec<ConversionWarning>,
}

impl Warnings {
    #[must_use]
    pub fn new(format: ConversionFormat) -> Self {
        Self {
            format,
            list: Vec::new(),
        }
    }

    /// Carry on the warnings a reader already raised.
    #[must_use]
    pub fn from_report(report: ConversionReport) -> Self {
        Self {
            format: report.format,
            list: report.warnings,
        }
    }

    /// A construct the format has no syntax for (`PROSE-IO-004`).
    pub fn unsupported(&mut self, feature: UnsupportedFeature) {
        let format = self.format;
        for warning in &mut self.list {
            if let ConversionWarning::FeatureNotSupported {
                feature: known,
                count,
                ..
            } = warning
                && *known == feature
            {
                *count = count.incremented();
                return;
            }
        }
        self.list.push(ConversionWarning::FeatureNotSupported {
            feature,
            format,
            count: Tally::<ConversionLosses>::new(1),
        });
    }

    /// An element the reader does not recognise (`PROSE-IO-005`).
    pub fn dropped(&mut self, element: DroppedElement) {
        for warning in &mut self.list {
            if let ConversionWarning::ElementDropped {
                element: known,
                count,
            } = warning
                && *known == element
            {
                *count = count.incremented();
                return;
            }
        }
        self.list.push(ConversionWarning::ElementDropped {
            element,
            count: Tally::<ConversionLosses>::new(1),
        });
    }

    /// A block moved out of a parent that refuses it (`PROSE-IO-052`).
    pub fn lifted(&mut self, block: LiftedBlock) {
        for warning in &mut self.list {
            if let ConversionWarning::BlockLifted {
                block: known,
                count,
            } = warning
                && *known == block
            {
                *count = count.incremented();
                return;
            }
        }
        self.list.push(ConversionWarning::BlockLifted {
            block,
            count: Tally::<ConversionLosses>::new(1),
        });
    }

    #[must_use]
    pub fn report(self) -> ConversionReport {
        ConversionReport {
            format: self.format,
            warnings: self.list,
        }
    }
}

/// A report as one sentence the user reads (`PROSE-IO-053`). A clean
/// conversion answers `None`.
#[must_use]
pub fn report_message(report: &ConversionReport) -> Option<String> {
    let parts: Vec<String> = report
        .warnings
        .iter()
        .map(|warning| {
            let (what, count) = match warning {
                ConversionWarning::FeatureNotSupported { feature, count, .. } => {
                    (feature.label(), *count)
                }
                ConversionWarning::ElementDropped { element, count } => (element.label(), *count),
                ConversionWarning::BlockLifted { block, count } => (block.label(), *count),
            };
            if count.get() > 1 {
                format!("{what} ({count})")
            } else {
                what.to_owned()
            }
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    let lifted = report
        .warnings
        .iter()
        .all(|w| matches!(w, ConversionWarning::BlockLifted { .. }));
    let verb = if lifted { "moved" } else { "not kept" };
    Some(format!("{verb}: {}", parts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PROSE-IO-053`: a lossy report reads as one sentence naming each
    /// construct, and a clean one says nothing.
    #[test]
    fn a_report_reads_as_one_sentence() {
        let mut warnings = Warnings::new(ConversionFormat::Markdown);
        assert_eq!(
            report_message(&Warnings::new(ConversionFormat::Markdown).report()),
            None
        );
        warnings.unsupported(UnsupportedFeature::Underline);
        warnings.unsupported(UnsupportedFeature::Underline);
        warnings.dropped(DroppedElement::Html);
        assert_eq!(
            report_message(&warnings.report()).as_deref(),
            Some("not kept: underline (2), HTML")
        );
    }
}
