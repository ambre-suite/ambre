//! What a reader of a presentation approximated and dropped, construct by
//! construct.
//!
//! The words are those of the interop vocabulary
//! (`1-Foundations/ambre-spec-interop.md` section 3): a construct, an outcome
//! and a count per construct and outcome. The codec cannot name the bus type:
//! `deny.toml` keeps `ambre-bus` to the crates that speak the thread
//! protocol, and a codec speaks none. So the report is a mirror, and Toile's
//! map carries each entry onto its own report one to one. Every reader of a
//! presentation format writes this one report, so a deck reads the same
//! whatever file it came from.

use ambre_core::Count;
use ambre_core::NarrowCount;
use serde::{Deserialize, Serialize};

/// A thing a presentation file holds that the deck model cannot hold whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeckConstruct {
    /// A SmartArt diagram.
    SmartArt,
    Chart,
    Table,
    /// A video, which lands as its poster frame where it has one.
    Video,
    Audio,
    /// The animations of one slide.
    Animation,
    /// The transition of one slide, which the deck replaces with its own.
    Transition,
    /// An embedded object, which lands as its preview picture where it has
    /// one.
    EmbeddedObject,
    /// A group, whose members land ungrouped.
    Group,
    /// A text of several styles, which takes the style of its first run.
    RichText,
    /// A fill that is a gradient, a pattern or a picture.
    NonSolidFill,
    /// An outline the deck model lacks, which lands as its bounding box.
    PresetShape,
    /// A picture in an encoding the reader does not decode: a metafile, a
    /// TIFF or a GIF.
    UnreadPicture,
    /// An ink drawing.
    Ink,
}

impl DeckConstruct {
    /// The words the user reads for one of it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SmartArt => "SmartArt diagram",
            Self::Chart => "chart",
            Self::Table => "table",
            Self::Video => "video",
            Self::Audio => "audio clip",
            Self::Animation => "slide animation",
            Self::Transition => "slide transition",
            Self::EmbeddedObject => "embedded object",
            Self::Group => "group",
            Self::RichText => "text of several styles",
            Self::NonSolidFill => "gradient, pattern or picture fill",
            Self::PresetShape => "shape outline",
            Self::UnreadPicture => "picture in an unread format",
            Self::Ink => "ink drawing",
        }
    }
}

/// What a reader did with one construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DeckOutcome {
    /// The construct arrived as the nearest thing the deck can hold.
    Approximated,
    /// The construct did not arrive.
    Dropped,
}

/// How many times one construct met one outcome.
pub enum DeckOccurrences {}

impl NarrowCount for DeckOccurrences {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("DeckOccurrences");
}

/// One line of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckEntry {
    pub construct: DeckConstruct,
    pub outcome: DeckOutcome,
    pub count: Count<DeckOccurrences>,
}

/// What one read did, one entry per construct and outcome, in the order the
/// reader first met them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeckReport {
    entries: Vec<DeckEntry>,
}

impl DeckReport {
    /// The entries, in the order the reader first met them.
    #[must_use]
    pub fn entries(&self) -> &[DeckEntry] {
        &self.entries
    }

    /// Count one more `construct` with `outcome`.
    pub fn record(&mut self, construct: DeckConstruct, outcome: DeckOutcome) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.construct == construct && e.outcome == outcome)
        {
            Some(entry) => entry.count = entry.count.incremented(),
            None => self.entries.push(DeckEntry {
                construct,
                outcome,
                count: Count::new(1),
            }),
        }
    }

    /// How many `construct` met `outcome`.
    #[must_use]
    pub fn count(&self, construct: DeckConstruct, outcome: DeckOutcome) -> Count<DeckOccurrences> {
        self.entries
            .iter()
            .find(|e| e.construct == construct && e.outcome == outcome)
            .map_or(Count::ZERO, |e| e.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_construct_twice_is_one_entry_with_its_count() {
        let mut report = DeckReport::default();
        report.record(DeckConstruct::Chart, DeckOutcome::Dropped);
        report.record(DeckConstruct::Group, DeckOutcome::Approximated);
        report.record(DeckConstruct::Chart, DeckOutcome::Dropped);
        assert_eq!(report.entries().len(), 2);
        assert_eq!(
            report.count(DeckConstruct::Chart, DeckOutcome::Dropped),
            Count::new(2)
        );
    }
}
