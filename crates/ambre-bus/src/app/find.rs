//! A search and its matches (`ambre-spec-find.md`). Prose, Trame and Toile
//! each run a search. Each sends one [`FindQuery`] shape over options of its
//! own, and each reports a list with one match current in it.
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

/// The query of a find, as the user typed it, never empty. An empty field
/// sends no query at all, which is how the panel clears its highlight
/// (`FIND-005`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FindText(String);

impl FindText {
    /// The text as typed; `None` when it is empty.
    #[must_use]
    pub fn new(text: String) -> Option<Self> {
        (!text.is_empty()).then_some(Self(text))
    }

    /// The text as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One find: its text and the refinements it applies (`ambre-spec-find.md`
/// section 3). The shape is one; each application declares its own options,
/// and every set opens on the case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FindQuery<O> {
    pub text: FindText,
    pub options: O,
}

impl<O> FindQuery<O> {
    /// The query for `text` under `options`.
    #[must_use]
    pub fn new(text: FindText, options: O) -> Self {
        Self { text, options }
    }
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
    use super::{FindQuery, FindText, MatchCount, MatchIndex};
    use ambre_core::StepDirection;

    #[test]
    fn an_empty_text_is_no_query() {
        assert_eq!(FindText::new(String::new()), None);
        let text = FindText::new("a".to_owned()).expect("a text");
        let query = FindQuery::new(text, ());
        assert_eq!(query.text.as_str(), "a");
    }

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
