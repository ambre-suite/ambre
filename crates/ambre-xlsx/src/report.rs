//! What a read or a write carried, approximated and dropped, construct by
//! construct.
//!
//! The words are those of the interop vocabulary
//! (`1-Foundations/ambre-spec-interop.md` section 3): a construct, an outcome
//! and a count per construct and outcome. The codec cannot name the bus type:
//! `deny.toml` keeps `ambre-bus` to the crates that speak the thread
//! protocol, and a codec speaks none. So the report is a mirror, and Trame's
//! wiring maps each entry onto its `ConversionReport` one to one. The report
//! crosses out of the converter module as postcard bytes, beside the model.

use ambre_core::Count;
use ambre_core::NarrowCount;
use serde::{Deserialize, Serialize};

/// A thing a workbook holds that a conversion can carry or lose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum XlsxConstruct {
    Sheet,
    Cell,
    Formula,
    /// A shared formula, expanded into each cell it covers.
    SharedFormula,
    /// A `$` anchor of a reference. Trame has no anchor, so the reference
    /// shifts on a paste or a fill.
    AbsoluteReference,
    /// A formula Trame's dialect cannot spell, kept verbatim for the export.
    ForeignFormula,
    Style,
    NumberFormat,
    /// An alignment Trame lacks: justify, fill, distributed.
    Alignment,
    /// A border stroke Trame lacks: hair, dash-dot, slanted.
    BorderStroke,
    DiagonalBorder,
    /// A fill pattern other than solid, read as its foreground colour.
    PatternFill,
    GradientFill,
    /// A text rotation or an indent.
    TextOrientation,
    /// A superscript, a subscript, an outline or a shadow on a font.
    FontEffect,
    /// The formatting runs of a rich text. The text stays.
    RichText,
    Merge,
    ColumnWidth,
    RowHeight,
    HiddenSheet,
    /// A sheet the user cannot show from Excel's menus, read as hidden.
    VeryHiddenSheet,
    HiddenLine,
    /// A workbook on the 1904 date system, its dates moved to 1900.
    DateSystem,
    Chart,
    Chartsheet,
    Picture,
    Shape,
    PivotTable,
    Macro,
    Comment,
    DataValidation,
    ConditionalFormat,
    Hyperlink,
    DefinedName,
    Table,
    Protection,
    FrozenPane,
    AutoFilter,
    ExternalLink,
    /// A Trame error Excel lacks, written as the nearest one.
    ErrorValue,
}

impl XlsxConstruct {
    /// The words the user reads for one of it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sheet => "sheet",
            Self::Cell => "cell",
            Self::Formula => "formula",
            Self::SharedFormula => "shared formula",
            Self::AbsoluteReference => "absolute reference",
            Self::ForeignFormula => "formula Trame cannot run",
            Self::Style => "style",
            Self::NumberFormat => "number format",
            Self::Alignment => "alignment",
            Self::BorderStroke => "border stroke",
            Self::DiagonalBorder => "diagonal border",
            Self::PatternFill => "pattern fill",
            Self::GradientFill => "gradient fill",
            Self::TextOrientation => "text rotation or indent",
            Self::FontEffect => "font effect",
            Self::RichText => "rich text",
            Self::Merge => "merged range",
            Self::ColumnWidth => "column width",
            Self::RowHeight => "row height",
            Self::HiddenSheet => "hidden sheet",
            Self::VeryHiddenSheet => "very hidden sheet",
            Self::HiddenLine => "hidden row or column",
            Self::DateSystem => "1904 date system",
            Self::Chart => "chart",
            Self::Chartsheet => "chart sheet",
            Self::Picture => "picture",
            Self::Shape => "shape",
            Self::PivotTable => "pivot table",
            Self::Macro => "macro",
            Self::Comment => "comment",
            Self::DataValidation => "data validation",
            Self::ConditionalFormat => "conditional format",
            Self::Hyperlink => "hyperlink",
            Self::DefinedName => "defined name",
            Self::Table => "table",
            Self::Protection => "protection",
            Self::FrozenPane => "frozen pane",
            Self::AutoFilter => "filter",
            Self::ExternalLink => "link to another workbook",
            Self::ErrorValue => "error value",
        }
    }
}

/// What a conversion did with one construct, worst last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Outcome {
    /// The construct arrived whole.
    Carried,
    /// The construct arrived as the nearest thing the target can express.
    Approximated,
    /// The construct did not arrive. Its text stays where it has any.
    Dropped,
}

/// How many times one construct met one outcome.
pub enum Occurrences {}

impl NarrowCount for Occurrences {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("XlsxOccurrences");
}

/// One line of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEntry {
    pub construct: XlsxConstruct,
    pub outcome: Outcome,
    pub count: Count<Occurrences>,
}

/// What one read or one write did, one entry per construct and outcome, in
/// the order the conversion first met them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Report {
    entries: Vec<ReportEntry>,
}

impl Report {
    /// The entries, in the order the conversion first met them.
    #[must_use]
    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }

    /// Count one more `construct` with `outcome`.
    pub fn record(&mut self, construct: XlsxConstruct, outcome: Outcome) {
        self.record_count(construct, outcome, Count::new(1));
    }

    /// Count `count` more `construct` with `outcome`. A zero count records
    /// nothing.
    pub fn record_count(
        &mut self,
        construct: XlsxConstruct,
        outcome: Outcome,
        count: Count<Occurrences>,
    ) {
        if count == Count::ZERO {
            return;
        }
        match self
            .entries
            .iter_mut()
            .find(|e| e.construct == construct && e.outcome == outcome)
        {
            Some(entry) => entry.count = entry.count.plus(count),
            None => self.entries.push(ReportEntry {
                construct,
                outcome,
                count,
            }),
        }
    }

    /// How many `construct` met `outcome`.
    #[must_use]
    pub fn count(&self, construct: XlsxConstruct, outcome: Outcome) -> Count<Occurrences> {
        self.entries
            .iter()
            .find(|e| e.construct == construct && e.outcome == outcome)
            .map_or(Count::ZERO, |e| e.count)
    }

    /// Whether every entry carried its construct whole.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.entries.iter().all(|e| e.outcome == Outcome::Carried)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_construct_twice_is_one_entry_with_its_count() {
        let mut report = Report::default();
        report.record(XlsxConstruct::Chart, Outcome::Dropped);
        report.record(XlsxConstruct::Merge, Outcome::Carried);
        report.record(XlsxConstruct::Chart, Outcome::Dropped);
        assert_eq!(report.entries().len(), 2);
        assert_eq!(
            report.count(XlsxConstruct::Chart, Outcome::Dropped),
            Count::new(2)
        );
        assert!(!report.is_lossless());
    }
}
