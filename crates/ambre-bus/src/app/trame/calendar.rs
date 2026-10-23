// ambre-bus/src/app/trame/calendar.rs The spreadsheet calendar: the two
// serials a date travels as, and the civil date between them.
//
// A formula reads a date as a number of days from the spreadsheet epoch,
// 1899-12-30. A whole-day count is a `DaySerial`; a date with its time of day
// in the fraction is a `DateSerial`. `CivilDate` names the year, the month and
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
    pub const fn days_since(self, earlier: Self) -> i64 {
        self.0 - earlier.0
    }

    /// The proleptic-Gregorian date of this day. The inverse of
    /// [`CivilDate::serial`], after Howard Hinnant's `civil_from_days`.
    pub const fn civil(self) -> CivilDate {
        let z = self.0 - UNIX_EPOCH_SERIAL + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        CivilDate {
            year: if month <= 2 { y + 1 } else { y },
            month,
            day,
        }
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

/// A proleptic-Gregorian date by its year, its month and its day.
///
/// The month runs from 1 to 12 in a date [`DaySerial::civil`] gives. The day
/// may run past either end of its month, and [`CivilDate::serial`] counts it
/// on from the first. Day 0 is then the last day of the month before, as
/// `DATE` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i64,
    pub month: i64,
    pub day: i64,
}

impl CivilDate {
    /// The day this date names, after Howard Hinnant's `days_from_civil`.
    pub const fn serial(self) -> DaySerial {
        let Self { year, month, day } = self;
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
        if self.month >= 12 {
            Self {
                year: self.year + 1,
                month: 1,
                day: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
                day: 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn date(year: i64, month: i64, day: i64) -> CivilDate {
        CivilDate { year, month, day }
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
            assert_eq!(serial.civil().serial(), serial, "{days}");
        }
    }

    #[test]
    fn the_next_month_of_december_opens_the_next_year() {
        assert_eq!(date(2024, 12, 31).next_month_start(), date(2025, 1, 1));
        assert_eq!(date(2024, 2, 10).next_month_start(), date(2024, 3, 1));
    }

    #[test]
    fn a_date_time_falls_on_its_day_and_keeps_its_time() {
        let at = DateSerial::new(46_294.75);
        assert_eq!(at.day(), DaySerial::new(46_294));
        assert!((at.time_of_day() - 0.75).abs() < f64::EPSILON);
    }
}
