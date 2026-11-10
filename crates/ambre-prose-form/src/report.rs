//! What a mapping carried and lost, in the words of the interop vocabulary
//! (`1-Foundations/ambre-spec-interop.md section 3`). The Word mapping runs in
//! a converter module, which names no type of the thread protocol, so its
//! report is a mirror. Prose carries each entry onto its `ConversionReport`
//! one to one.

use ambre_core::{Count, CountKind};
use ambre_docx::WordConstruct;
use serde::{Deserialize, Serialize};

use crate::UnsupportedFeature;

/// A construct a mapping names: one the form or the format has no place
/// for, or one the Word reader named in its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormConstruct {
    Feature(UnsupportedFeature),
    Word(WordConstruct),
}

/// What a mapping did with one construct, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FormOutcome {
    Carried,
    Approximated,
    Dropped,
}

/// How many times one construct met one outcome.
pub enum FormOccurrences {}

impl CountKind for FormOccurrences {
    type Repr = u32;
    const NAME: &'static str = "FormOccurrences";
}

/// One line of a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormEntry {
    pub construct: FormConstruct,
    pub outcome: FormOutcome,
    pub count: Count<FormOccurrences>,
}

/// What one mapping did, one entry per construct and outcome, in the order
/// it first met them (`INTEROP-003`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormReport {
    entries: Vec<FormEntry>,
}

impl FormReport {
    /// The entries, in the order the mapping first met them.
    #[must_use]
    pub fn entries(&self) -> &[FormEntry] {
        &self.entries
    }

    /// Count `count` more `construct` with `outcome`. A zero count records
    /// nothing.
    pub fn record(&mut self, construct: FormConstruct, outcome: FormOutcome, count: u32) {
        if count == 0 {
            return;
        }
        match self
            .entries
            .iter_mut()
            .find(|e| e.construct == construct && e.outcome == outcome)
        {
            Some(entry) => entry.count = Count::new(entry.count.get().saturating_add(count)),
            None => self.entries.push(FormEntry {
                construct,
                outcome,
                count: Count::new(count),
            }),
        }
    }

    /// A construct the format has no syntax for, dropped (`PROSE-IO-004`).
    pub fn unsupported(&mut self, feature: UnsupportedFeature) {
        self.record(FormConstruct::Feature(feature), FormOutcome::Dropped, 1);
    }

    /// A construct emitted as the nearest thing the format holds
    /// (`PROSE-IO-006`).
    pub fn approximated(&mut self, feature: UnsupportedFeature) {
        self.record(
            FormConstruct::Feature(feature),
            FormOutcome::Approximated,
            1,
        );
    }

    /// How many `construct` met `outcome`.
    #[must_use]
    pub fn count(&self, construct: FormConstruct, outcome: FormOutcome) -> Count<FormOccurrences> {
        self.entries
            .iter()
            .find(|e| e.construct == construct && e.outcome == outcome)
            .map_or(Count::ZERO, |e| e.count)
    }
}
