// ambre-bus/src/app/trame/calendar.rs The spreadsheet calendar: the two
// serials a date travels as, and the date parts between them.
//
// A formula reads a date as a number of days from the spreadsheet epoch,
// 1899-12-30. A whole-day count is a `DaySerial`; a date with its time of day
// in the fraction is a `DateSerial`. `DateParts` names the year, the month and
// the day, so no call can pass them in the wrong order.
//
// The calendar lives on the bus: a validation rule carries its date bounds
// across the seam as serials. The view parses the text once, at input.

/// Days from the spreadsheet epoch to the Unix epoch, 1970-01-01.
const UNIX_EPOCH_SERIAL: i64 = 25_569;

/// A whole number of days from the spreadsheet epoch, 1899-12-30.
///
/// Every count of days is a date, so the constructor takes any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DaySerial(i64);

impl DaySerial {
    /// The serial `days` after the epoch, or before it when negative.
    pub const fn new(days: i64) -> Self {
        Self(days)
    }

    /// The count of days, for the number a formula returns.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The days from `earlier` to `self`.
    pub const fn days_since(self, earlier: Self) -> DayCount {
        DayCount(self.0 - earlier.0)
    }

    /// The proleptic-Gregorian date of this day. The inverse of
    /// [`DateParts::serial`], after Howard Hinnant's `civil_from_days`.
    pub const fn parts(self) -> DateParts {
        let z = self.0 - UNIX_EPOCH_SERIAL + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        DateParts {
            year: Year(if month <= 2 { y + 1 } else { y }),
            month: Month(month),
            day: DayOfMonth(day),
        }
    }
}

/// A signed number of days between two dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayCount(i64);

impl DayCount {
    /// A count of `days`, negative when the later date comes first.
    pub const fn new(days: i64) -> Self {
        Self(days)
    }

    /// The count itself.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A proleptic-Gregorian year. Year 0 is 1 BC, and any year reads as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Year(i64);

impl Year {
    /// The year `year`.
    pub const fn new(year: i64) -> Self {
        Self(year)
    }

    /// The year itself.
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A month of the year, from 1 for January to 12 for December.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month(i64);

impl Month {
    /// The month numbered `month`, `None` outside 1 to 12. A count of months
    /// past either end of a year is a [`MonthCount`], which
    /// [`DateParts::of_months`] reads.
    pub const fn new(month: i64) -> Option<Self> {
        if matches!(month, 1..=12) {
            Some(Self(month))
        } else {
            None
        }
    }

    /// The month's number, from 1 to 12.
    pub const fn get(self) -> i64 {
        self.0
    }
}

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
    pub const fn last_of(days: DayCount) -> Self {
        Self(days.0)
    }
}

/// A date and a time of day: whole days from the epoch, and the time of day
/// in the fraction.
///
/// Every number reads as one, so the constructor takes any.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct DateSerial(f64);

impl DateSerial {
    /// The serial a cell's number reads as.
    pub const fn new(serial: f64) -> Self {
        Self(serial)
    }

    /// The number a formula returns for it.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// The day it falls on, its time of day dropped.
    pub fn day(self) -> DaySerial {
        DaySerial(self.0.floor() as i64)
    }

    /// The time of day, as a fraction of a day from 0 up to 1.
    pub fn time_of_day(self) -> f64 {
        self.0 - self.0.floor()
    }

    /// The serial of the instant `unix_days` days after the Unix epoch.
    pub fn from_unix_days(unix_days: f64) -> Self {
        Self(unix_days + UNIX_EPOCH_SERIAL as f64)
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
    pub month: Month,
    pub day: DayOfMonth,
}

impl DateParts {
    /// The date `months` months after January of `year`, on `day`. The count
    /// runs past either end of the year and carries into it, as `DATE` and
    /// `EDATE` read a month out of range. Twelve months after January 2024
    /// is January 2025.
    pub const fn of_months(year: Year, months: MonthCount, day: DayOfMonth) -> Self {
        Self {
            year: Year(year.0 + months.0.div_euclid(12)),
            month: Month(months.0.rem_euclid(12) + 1),
            day,
        }
    }

    /// The day this date names, after Howard Hinnant's `days_from_civil`.
    pub const fn serial(self) -> DaySerial {
        let (year, month, day) = (self.year.0, self.month.0, self.day.0);
        let yy = year - if month <= 2 { 1 } else { 0 };
        let era = if yy >= 0 { yy } else { yy - 399 } / 400;
        let yoe = yy - era * 400;
        let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let unix_days = era * 146_097 + doe - 719_468;
        DaySerial(unix_days + UNIX_EPOCH_SERIAL)
    }

    /// The first day of the month after this date's month.
    pub const fn next_month_start(self) -> Self {
        Self::of_months(self.year, MonthCount(self.month.0), DayOfMonth(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn date(year: i64, month: i64, day: i64) -> DateParts {
        DateParts::of_months(Year(year), MonthCount(month - 1), DayOfMonth(day))
    }

    #[test]
    fn the_epoch_and_a_known_day_read_their_serials() {
        assert_eq!(date(1899, 12, 30).serial(), DaySerial::new(0));
        assert_eq!(date(1970, 1, 1).serial(), DaySerial::new(25_569));
        assert_eq!(date(2026, 9, 29).serial(), DaySerial::new(46_294));
    }

    #[test]
    fn a_serial_reads_back_the_date_it_came_from() {
        for days in -800_000..800_000 {
            let serial = DaySerial::new(days);
            assert_eq!(serial.parts().serial(), serial, "{days}");
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
            assert_eq!(Month::new(refused), None, "{refused}");
        }
        assert_eq!(Month::new(12).map(Month::get), Some(12));
        let carried =
            DateParts::of_months(Year::new(2024), MonthCount::new(12), DayOfMonth::new(5));
        assert_eq!((carried.year.get(), carried.month.get()), (2025, 1));
        let back = DateParts::of_months(Year::new(2024), MonthCount::new(-1), DayOfMonth::new(5));
        assert_eq!((back.year.get(), back.month.get()), (2023, 12));
        assert_eq!(
            date(2024, 3, 1)
                .serial()
                .days_since(date(2024, 2, 1).serial()),
            DayCount::new(29)
        );
    }

    #[test]
    fn a_date_time_falls_on_its_day_and_keeps_its_time() {
        let at = DateSerial::new(46_294.75);
        assert_eq!(at.day(), DaySerial::new(46_294));
        assert!((at.time_of_day() - 0.75).abs() < f64::EPSILON);
    }
}
