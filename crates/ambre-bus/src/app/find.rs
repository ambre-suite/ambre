//! The matches of an open search: which one is current, and how many there
//! are. Prose, Trame and Toile each run a search, and each reports a list
//! with one match current in it.
//!
//! The index and the total travel side by side on the wire and into the
//! "k/n" counter. One family keeps a match index apart from a count, and
//! from every other index a search event carries.

use ambre_core::StepDirection;

ambre_core::define_index! {
    /// A match of one search, counted from zero in the order the search
    /// reports its list.
    pub struct MatchIndex(u32);
    /// A number of matches.
    pub struct MatchCount;
    /// The signed gap between two matches.
    pub struct MatchDelta(i64);
    /// A run of matches.
    pub struct MatchSpan;
    /// A list indexed by match.
    pub struct MatchVec;
}

impl MatchIndex {
    /// The match one step on in `direction` among `count`, wrapping at either
    /// end. `None` when the search found nothing.
    #[must_use]
    pub fn stepped(self, direction: StepDirection, count: MatchCount) -> Option<Self> {
        let last = count.as_end().prev()?;
        Some(match direction {
            StepDirection::Backward => self.prev().unwrap_or(last),
            StepDirection::Forward if self >= last => Self::ZERO,
            StepDirection::Forward => self.next(),
        })
    }

    /// The match a counter shows a reader: its index plus one.
    #[must_use]
    pub fn ordinal(self) -> u32 {
        self.get().saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{MatchCount, MatchIndex};
    use ambre_core::StepDirection;

    #[test]
    fn a_step_wraps_at_either_end_and_finds_nothing_in_an_empty_list() {
        let three = MatchCount::new(3);
        let (first, last) = (MatchIndex::ZERO, MatchIndex::new(2));
        assert_eq!(last.stepped(StepDirection::Forward, three), Some(first));
        assert_eq!(first.stepped(StepDirection::Backward, three), Some(last));
        assert_eq!(
            first.stepped(StepDirection::Forward, three),
            Some(MatchIndex::new(1))
        );
        assert_eq!(
            first.stepped(StepDirection::Forward, MatchCount::new(0)),
            None
        );
        assert_eq!(last.ordinal(), 3);
    }
}
