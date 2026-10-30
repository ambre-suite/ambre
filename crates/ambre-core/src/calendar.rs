//! The year and the month of a proleptic-Gregorian date.
//!
//! Prose draws a date field from them, and Trame counts a date serial with
//! them. One type each, so a year never stands for a month. The day stays in
//! each application. Prose bounds it to a month. Trame lets it run past
//! either end of its month, as `DATE` reads it.

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
/// value builds one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month(u8);

impl Month {
    /// The first month of the year.
    pub const JANUARY: Self = Self(1);

    /// The month numbered `month`, `None` outside 1 to 12.
    #[must_use]
    pub const fn new(month: i64) -> Option<Self> {
        if matches!(month, 1..=12) {
            // The range check above keeps the cast exact.
            Some(Self(month as u8))
        } else {
            None
        }
    }

    /// The month's number, from 1 to 12.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for Year {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::fmt::Display for Month {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Month, Year};

    #[test]
    fn a_month_is_numbered_one_to_twelve_and_nothing_else_builds_one() {
        for refused in [i64::MIN, -1, 0, 13, 256 + 1, i64::MAX] {
            assert_eq!(Month::new(refused), None, "{refused}");
        }
        assert_eq!(Month::new(12).map(Month::get), Some(12));
        assert_eq!(Month::new(1), Some(Month::JANUARY));
    }

    #[test]
    fn a_year_and_a_month_print_their_number() {
        assert_eq!(Year::new(-44).to_string(), "-44");
        assert_eq!(format!("{:02}", Month::JANUARY), "01");
    }
}
