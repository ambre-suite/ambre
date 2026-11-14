//! The year and the month of a proleptic-Gregorian date.
//!
//! Prose draws a date field from them, and Trame counts a date serial with
//! them. One type each, so a year never stands for a month. The day stays in
//! each application. Prose bounds it to a month. Trame lets it run past
//! either end of its month, as `DATE` reads it.

use crate::bounded::{Bounded, Bounds, Refusal};

/// A proleptic-Gregorian year. Year 0 is 1 BC, and any year reads as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Year(i64);

impl Year {
    /// The year `year`.
    #[must_use]
    pub const fn new(year: i64) -> Self {
        Self(year)
    }

    /// The year as a number, for a date count or the text a date prints.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A month of the year, from 1 for January to 12 for December. No other
/// value builds a [`Bounded<Month>`], and a decode refuses one.
pub enum Month {}

impl Bounds for Month {
    type Repr = u8;
    const MIN: u8 = 1;
    const MAX: u8 = 12;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Month");
    const REFUSAL: Refusal = Refusal::new("a month lies in 1..=12, not ", "");
}

impl Bounded<Month> {
    /// The first month of the year.
    pub const JANUARY: Self = Self::MIN;

    /// The month numbered `month`, `None` outside 1 to 12. A date count
    /// reckons the month in an `i64`, and a `const fn` builds it through
    /// this door.
    #[must_use]
    pub const fn numbered(month: i64) -> Option<Self> {
        if matches!(month, 1..=12) {
            // The range check above keeps the cast exact.
            Some(Self::at(month as u8))
        } else {
            None
        }
    }
}

impl std::fmt::Display for Year {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Month, Year};
    use crate::Bounded;

    #[test]
    fn a_month_is_numbered_one_to_twelve_and_nothing_else_builds_one() {
        for refused in [i64::MIN, -1, 0, 13, 256 + 1, i64::MAX] {
            assert_eq!(Bounded::<Month>::numbered(refused), None, "{refused}");
        }
        assert_eq!(
            Bounded::<Month>::numbered(12).map(Bounded::<Month>::get),
            Some(12)
        );
        assert_eq!(
            Bounded::<Month>::numbered(1),
            Some(Bounded::<Month>::JANUARY)
        );
        assert_eq!(Bounded::<Month>::new(13), None);
    }

    #[test]
    fn a_year_and_a_month_print_their_number() {
        assert_eq!(Year::new(-44).to_string(), "-44");
        assert_eq!(format!("{:02}", Bounded::<Month>::JANUARY), "01");
        assert_eq!(format!("{:?}", Bounded::<Month>::JANUARY), "Month(1)");
    }
}
