// ambre-bus/src/app/trame/calendar.rs The spreadsheet calendar: the date
// parts between the two serials a date travels as.
//
// A formula reads a date as a number of days from the spreadsheet epoch,
// 1899-12-30. A whole-day count is an `Instant<SheetDay>`; a date with its
// time of day in the fraction is an `Instant<SheetDateTime>`. Both live in
// `ambre_core::span`, beside the other time bases. `DateParts` names the
// year, the month and the day, so no call can pass them in the wrong order.
//
// The calendar lives on the bus: a validation rule carries its date bounds
// across the seam as serials. The view parses the text once, at input.
//
// The year and the month live in `ambre-core`, which Prose shares. The day
// stays here: it runs past either end of its month, as `DATE` reads it.

use ambre_core::{Bounded, Day, Instant, SheetDay, TimeSpan};
pub use ambre_core::{Month, Year};

/// A count of months, from January of some year. It may run past December
/// or below January, as `DATE` and `EDATE` read a month out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonthCount(i64);

impl MonthCount {
    /// A count of `months`, of either sign.
    pub const fn new(months: i64) -> Self {
        Self(months)
    }

    /// The number of months.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The two counts together, held at either end of `i64`.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

/// A day of a month, counted from 1 on the first.
///
/// It may run past either end of its month. [`DateParts::serial`] counts it
/// on from the first, so day 0 is the last day of the month before, as `DATE`
/// reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayOfMonth(i64);

impl DayOfMonth {
    /// The day `day` of a month.
    pub const fn new(day: i64) -> Self {
        Self(day)
    }

    /// The day's number.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The last day of a month that holds `days` days.
    pub const fn last_of(days: TimeSpan<Day>) -> Self {
        Self(days.get())
    }
}

/// The year, the month and the day of a proleptic-Gregorian date, before the
/// day is checked against its month.
///
/// The month always runs from 1 to 12. The day may run past either end of
/// its month, and [`DateParts::serial`] counts it on from the first. So a
/// value need not name a valid date: the Prose `CivilDate` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateParts {
    pub year: Year,
    pub month: Bounded<Month>,
    pub day: DayOfMonth,
}

impl DateParts {
    /// The proleptic-Gregorian date of the day `day`. The inverse of
    /// [`DateParts::serial`], after Howard Hinnant's `civil_from_days`.
    pub const fn of_day(day: Instant<SheetDay>) -> Self {
        let z = day.get() - Instant::<SheetDay>::UNIX_EPOCH.get() + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        debug_assert!(matches!(month, 1..=12), "civil_from_days gives a month");
        Self {
            year: Year::new(if month <= 2 { y + 1 } else { y }),
            // The arithmetic keeps the month in 1 to 12; January stands in
            // for a value it never computes.
            month: match Bounded::<Month>::numbered(month) {
                Some(month) => month,
                None => Bounded::<Month>::JANUARY,
            },
            day: DayOfMonth(day),
        }
    }

    /// The date `months` months after January of `year`, on `day`. The count
    /// runs past either end of the year and carries into it, as `DATE` and
    /// `EDATE` read a month out of range. Twelve months after January 2024
    /// is January 2025.
    pub const fn of_months(year: Year, months: MonthCount, day: DayOfMonth) -> Self {
        debug_assert!(
            matches!(months.0.rem_euclid(12) + 1, 1..=12),
            "a remainder of twelve gives a month"
        );
        Self {
            year: Year::new(year.get() + months.0.div_euclid(12)),
            // The remainder keeps the month in 1 to 12. A count of months
            // past either end of a year is a `MonthCount`.
            month: match Bounded::<Month>::numbered(months.0.rem_euclid(12) + 1) {
                Some(month) => month,
                None => Bounded::<Month>::JANUARY,
            },
            day,
        }
    }

    /// The day this date names, after Howard Hinnant's `days_from_civil`.
    pub const fn serial(self) -> Instant<SheetDay> {
        let (year, month, day) = (self.year.get(), self.month.get() as i64, self.day.0);
        let yy = year - if month <= 2 { 1 } else { 0 };
        let era = if yy >= 0 { yy } else { yy - 399 } / 400;
        let yoe = yy - era * 400;
        let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let unix_days = era * 146_097 + doe - 719_468;
        Instant::new(unix_days + Instant::<SheetDay>::UNIX_EPOCH.get())
    }

    /// The first day of the month after this date's month.
    pub const fn next_month_start(self) -> Self {
        Self::of_months(
            self.year,
            MonthCount(self.month.get() as i64),
            DayOfMonth(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn date(year: i64, month: i64, day: i64) -> DateParts {
        DateParts::of_months(Year::new(year), MonthCount(month - 1), DayOfMonth(day))
    }

    #[test]
    fn the_epoch_and_a_known_day_read_their_serials() {
        assert_eq!(date(1899, 12, 30).serial(), Instant::<SheetDay>::new(0));
        assert_eq!(date(1970, 1, 1).serial(), Instant::<SheetDay>::new(25_569));
        assert_eq!(date(2026, 9, 29).serial(), Instant::<SheetDay>::new(46_294));
    }

    #[test]
    fn a_serial_reads_back_the_date_it_came_from() {
        for days in -800_000..800_000 {
            let serial = Instant::<SheetDay>::new(days);
            assert_eq!(DateParts::of_day(serial).serial(), serial, "{days}");
        }
    }

    #[test]
    fn the_next_month_of_december_opens_the_next_year() {
        assert_eq!(date(2024, 12, 31).next_month_start(), date(2025, 1, 1));
        assert_eq!(date(2024, 2, 10).next_month_start(), date(2024, 3, 1));
    }

    /// A month is numbered 1 to 12, and a count of months past either end
    /// of a year carries into the year.
    #[test]
    fn a_month_runs_from_one_to_twelve_and_a_count_carries_into_the_year() {
        for refused in [0, 13, -1] {
            assert_eq!(Bounded::<Month>::numbered(refused), None, "{refused}");
        }
        assert_eq!(
            Bounded::<Month>::numbered(12).map(Bounded::<Month>::get),
            Some(12)
        );
        let carried =
            DateParts::of_months(Year::new(2024), MonthCount::new(12), DayOfMonth::new(5));
        assert_eq!((carried.year.get(), carried.month.get()), (2025, 1));
        let back = DateParts::of_months(Year::new(2024), MonthCount::new(-1), DayOfMonth::new(5));
        assert_eq!((back.year.get(), back.month.get()), (2023, 12));
        assert_eq!(
            date(2024, 3, 1).serial() - date(2024, 2, 1).serial(),
            TimeSpan::<Day>::new(29)
        );
    }
}
