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
    ConversionFormat, ConversionReport, ConversionWarning, DroppedElement, FootnoteOrdinal,
    LiftedBlock, UnsupportedFeature, WarningCount,
};

use crate::doc::StoredLevel;

/// A document in flight between a format and the block tree. Held for the
/// length of one conversion and never serialized.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProseIr {
    pub meta: IrMeta,
    pub blocks: Vec<IrBlock>,
    /// The footnote bodies, in the order of their ordinals.
    pub footnotes: Vec<IrFootnote>,
}

/// What the document carries beside its blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IrMeta {
    pub title: Option<String>,
    /// The document has a page format. Every Prose document has one, and a
    /// format without pages reports it (`PROSE-IO-018`).
    pub paged: bool,
    /// The document has a header or a footer.
    pub bands: bool,
}

/// One block. Depth is the nesting, never a number.
#[derive(Clone, Debug, PartialEq)]
pub enum IrBlock {
    Paragraph {
        /// The paragraph carries a named style, or alignment, indents or
        /// spacing of its own.
        styled: bool,
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
        merged: bool,
        rows: Vec<IrTableRow>,
    },
    CodeBlock {
        lang: Option<String>,
        text: String,
    },
    Image {
        alt: String,
    },
    PageBreak,
    HRule,
    Toc,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableCell {
    /// The cell carries a fill, a border, a padding or a vertical alignment.
    pub styled: bool,
    pub blocks: Vec<IrBlock>,
}

/// One footnote body. A run points at it by ordinal.
#[derive(Clone, Debug, PartialEq)]
pub struct IrFootnote {
    pub ordinal: FootnoteOrdinal,
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

/// A run of text and what it carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IrRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub code: bool,
    pub link: Option<ambre_core::LinkUrl>,
    /// A colour, a background, a size or a family.
    pub styled: bool,
    pub comment: bool,
    pub suggestion: bool,
    pub cross_ref: bool,
    /// The ordinal of the footnote this run calls.
    pub footnote_ref: Option<FootnoteOrdinal>,
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
            count: WarningCount::new(1),
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
            count: WarningCount::new(1),
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
            count: WarningCount::new(1),
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
