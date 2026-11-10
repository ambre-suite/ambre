//! What a read carried, approximated and dropped, construct by construct.
//!
//! The words are those of the interop vocabulary
//! (`1-Foundations/ambre-spec-interop.md section 3`): a construct, an outcome
//! and a count per construct and outcome (`INTEROP-003`). The codec cannot
//! name the bus type: the converter module links no crate of the thread
//! protocol, so `deny.toml` keeps `ambre-bus` out of every codec. The report
//! is a mirror, and Prose's mapping carries each entry onto its
//! `ConversionReport` one to one.

use ambre_core::{Count, CountKind};
use serde::{Deserialize, Serialize};

use crate::construct::WordConstruct;

/// What a read did with one construct, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Outcome {
    /// The construct arrived whole.
    Carried,
    /// The construct arrived as the nearest thing the model can express.
    Approximated,
    /// The construct did not arrive. Its text stays where it has any.
    Dropped,
}

/// How many times one construct met one outcome.
pub enum Occurrences {}

impl CountKind for Occurrences {
    type Repr = u32;
    const NAME: &'static str = "WordOccurrences";
}

/// One line of a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEntry {
    pub construct: WordConstruct,
    pub outcome: Outcome,
    pub count: Count<Occurrences>,
}

/// What one read did, one entry per construct and outcome, in the order the
/// reader first met them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordReport {
    entries: Vec<ReportEntry>,
}

impl WordReport {
    /// The entries, in the order the reader first met them.
    #[must_use]
    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }

    /// Count one more `construct` with `outcome`.
    pub fn record(&mut self, construct: WordConstruct, outcome: Outcome) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.construct == construct && e.outcome == outcome)
        {
            Some(entry) => entry.count = entry.count.incremented(),
            None => self.entries.push(ReportEntry {
                construct,
                outcome,
                count: Count::new(1),
            }),
        }
    }

    /// How many `construct` met `outcome`.
    #[must_use]
    pub fn count(&self, construct: WordConstruct, outcome: Outcome) -> Count<Occurrences> {
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
        let mut report = WordReport::default();
        report.record(WordConstruct::Drawing, Outcome::Dropped);
        report.record(WordConstruct::Comment, Outcome::Carried);
        report.record(WordConstruct::Drawing, Outcome::Dropped);
        assert_eq!(report.entries().len(), 2);
        assert_eq!(
            report.count(WordConstruct::Drawing, Outcome::Dropped),
            Count::new(2)
        );
        assert!(!report.is_lossless());
    }
}
