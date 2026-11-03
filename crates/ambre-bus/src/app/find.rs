//! A search and its matches (`ambre-spec-find.md`). Prose, Trame and Toile
//! each run a search. Each sends one [`FindQuery`] shape over options of its
//! own, and each reports a list with one match current in it.
//!
//! The index and the total travel side by side on the wire and into the
//! "k/n" counter. One family keeps a match index apart from a count, and
//! from every other index a search event carries.

use ambre_core::Redacted;
use ambre_core::index::{Axis, Circular, Len, Numbered, Pos, Shown};

/// A match of one search, counted from zero in the order the search
/// reports its list.
#[derive(PartialEq, Eq)]
pub enum Match {}

impl Axis for Match {
    const NAME: &'static str = "Match";
}

/// The match a counter shows a reader: its index plus one, the "k" of "k/n".
impl Numbered for Match {
    type Number = Len<Match>;

    fn number_of(pos: Pos<Match>) -> Len<Match> {
        Len::through(pos)
    }
}

/// A counter shows the number of matches to a reader.
impl Shown for Match {}

/// The match cursor wraps: a step past the last match lands on the first.
impl Circular for Match {}

/// The query of a find, as the user typed it, never empty. An empty field
/// sends no query at all, which is how the panel clears its highlight
/// (`FIND-005`). `Debug` prints the length alone, so a log carries none of
/// the text, nor of the [`FindQuery`] that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FindText(Redacted<String>);

impl FindText {
    /// The text as typed; `None` when it is empty.
    #[must_use]
    pub fn new(text: String) -> Option<Self> {
        (!text.is_empty()).then_some(Self(Redacted::new(text)))
    }

    /// The text as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
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

#[cfg(test)]
mod tests {
    use super::{FindQuery, FindText, Match};
    use ambre_core::StepDirection;
    use ambre_core::index::{Delta, Len, Pos};

    #[test]
    fn an_empty_text_is_no_query() {
        assert_eq!(FindText::new(String::new()), None);
        let text = FindText::new("a".to_owned()).expect("a text");
        let query = FindQuery::new(text, ());
        assert_eq!(query.text.as_str(), "a");
    }

    /// A query's `Debug` form holds none of the typed text.
    #[test]
    fn a_query_prints_the_length_of_its_text_alone() {
        let text = FindText::new("secret plan".to_owned()).expect("a text");
        assert_eq!(format!("{text:?}"), "FindText(11 chars)");
        let query = format!("{:?}", FindQuery::new(text, ()));
        assert_eq!(query, "FindQuery { text: FindText(11 chars), options: () }");
    }

    #[test]
    fn a_step_wraps_at_either_end_and_finds_nothing_in_an_empty_list() {
        let three = Len::<Match>::new(3);
        let (first, last) = (Pos::<Match>::ZERO, Pos::<Match>::new(2));
        let (forward, back) = (
            Delta::step(StepDirection::Forward),
            Delta::step(StepDirection::Backward),
        );
        assert_eq!(last.wrapped(forward, three), Some(first));
        assert_eq!(first.wrapped(back, three), Some(last));
        assert_eq!(first.wrapped(forward, three), Some(Pos::<Match>::new(1)));
        assert_eq!(first.wrapped(forward, Len::ZERO), None);
    }
}
