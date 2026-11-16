//! The date of a comment or a tracked change, `ST_DateTime`: an ISO 8601
//! instant such as `2009-07-24T15:05:00Z`.

use ambre_client_core::MonthDay;
use ambre_core::{Authored, Bounded, Day, Instant, Month, TimeSpan, Year};

use crate::model::WordDate;

const MS_PER_SECOND: i64 = 1000;
const SECONDS_PER_DAY: i64 = 86_400;

/// The instant `text` spells, or `None` for text that is no ISO 8601 date
/// and time, or a date before 1970. A missing offset reads as UTC, as Word
/// writes local time without one.
#[must_use]
pub fn parse(text: &str) -> Option<Instant<Authored>> {
    let text = text.trim();
    let (date, rest) = text.split_once('T').unwrap_or((text, "00:00:00"));
    let mut fields = date.splitn(3, '-');
    let date = CivilDate {
        year: Year::new(fields.next()?.parse().ok()?),
        month: Bounded::new(fields.next()?.parse().ok()?)?,
        day: Bounded::new(fields.next()?.parse().ok()?)?,
    };
    let (clock, offset_seconds) = split_offset(rest)?;
    let mut clock_fields = clock.splitn(3, ':');
    let hour: i64 = clock_fields.next()?.parse().ok()?;
    let minute: i64 = clock_fields.next()?.parse().ok()?;
    let seconds = clock_fields.next().unwrap_or("0");
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let second: i64 = whole.parse().ok()?;
    if hour > 24 || minute > 59 || second > 60 {
        return None;
    }
    let millis: i64 = format!("{fraction:0<3}")
        .get(..3)
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);
    let seconds = days_from_civil(date)
        .get()
        .checked_mul(SECONDS_PER_DAY)?
        .checked_add(hour * 3600 + minute * 60 + second)?
        .checked_sub(offset_seconds)?;
    let millis = seconds.checked_mul(MS_PER_SECOND)?.checked_add(millis)?;
    // An authored instant counts from the epoch, so a date before 1970
    // reads as none.
    u64::try_from(millis).ok().map(Instant::new)
}

/// The date an annotation carries: `w16du:dateUtc` when the part gives it,
/// beside the wall time of `w:date`. Without it, `w:date` is a wall time
/// whose `Z` Word does not mean, unless it spells a numeric offset, which
/// names the instant.
#[must_use]
pub fn word_date(date: Option<&str>, date_utc: Option<&str>) -> Option<WordDate> {
    let wall = date.and_then(parse_wall);
    match (date_utc.and_then(parse), wall) {
        (Some(utc), wall) => Some(WordDate::Zoned {
            utc,
            wall: wall.unwrap_or(utc),
        }),
        (None, Some(wall)) => {
            let explicit = date.and_then(parse).filter(|instant| *instant != wall);
            Some(match explicit {
                Some(utc) => WordDate::Zoned { utc, wall },
                None => WordDate::Floating(wall),
            })
        }
        (None, None) => None,
    }
}

/// The wall time `text` spells, any offset ignored.
fn parse_wall(text: &str) -> Option<Instant<Authored>> {
    let text = text.trim();
    let cut = text
        .find('T')
        .and_then(|t| text.get(t..)?.find(['Z', '+', '-']).map(|at| t + at))
        .unwrap_or(text.len());
    parse(text.get(..cut)?)
}

/// The instant as Word writes it: whole seconds, in UTC.
#[must_use]
pub fn format(instant: Instant<Authored>) -> String {
    let seconds = i64::try_from(instant.get())
        .unwrap_or(i64::MAX)
        .div_euclid(MS_PER_SECOND);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let CivilDate { year, month, day } = civil_from_days(TimeSpan::new(days));
    format!(
        "{:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        year.get(),
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// The clock and the offset east of UTC in seconds, from `HH:MM:SS` and an
/// optional `Z`, `+HH:MM` or `-HH:MM`.
fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(clock) = rest.strip_suffix('Z') {
        return Some((clock, 0));
    }
    match rest.rfind(['+', '-']) {
        Some(at) => {
            let (clock, offset) = rest.split_at(at);
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = offset
                .get(1..)?
                .split_once(':')
                .unwrap_or((offset.get(1..)?, "0"));
            let hours: i64 = hours.parse().ok()?;
            let minutes: i64 = minutes.parse().ok()?;
            Some((clock, sign * (hours * 3600 + minutes * 60)))
        }
        None => Some((rest, 0)),
    }
}

/// A proleptic-Gregorian date, each part checked at its door.
struct CivilDate {
    year: Year,
    month: Bounded<Month>,
    day: Bounded<MonthDay>,
}

/// The days from 1970-01-01 to a civil date: Howard Hinnant's
/// `days_from_civil`, exact over the proleptic Gregorian calendar.
fn days_from_civil(date: CivilDate) -> TimeSpan<Day> {
    let month = i64::from(date.month.get());
    let year = if month <= 2 {
        date.year.get() - 1
    } else {
        date.year.get()
    };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
        + i64::from(date.day.get())
        - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    TimeSpan::new(era * 146_097 + day_of_era - 719_468)
}

/// The civil date of a day count from 1970-01-01, the inverse of
/// [`days_from_civil`].
fn civil_from_days(days: TimeSpan<Day>) -> CivilDate {
    let days = days.get() + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    // The arithmetic keeps the month in 1 to 12 and the day in 1 to 31.
    // The first of each stands in for a value it never computes.
    CivilDate {
        year: Year::new(year_of_era + era * 400 + i64::from(month <= 2)),
        month: Bounded::<Month>::numbered(month).unwrap_or(Bounded::<Month>::JANUARY),
        day: u8::try_from(day)
            .ok()
            .and_then(Bounded::new)
            .unwrap_or(Bounded::<MonthDay>::MIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_dates_read_and_write_back() {
        let instant = parse("2009-07-24T15:05:00Z").expect("reads");
        assert_eq!(instant.get(), 1_248_447_900_000);
        assert_eq!(format(instant), "2009-07-24T15:05:00Z");
        assert_eq!(parse("2009-07-24T17:05:00+02:00"), Some(instant));
        assert_eq!(parse("2009-07-24T15:05:00"), Some(instant));
        assert_eq!(
            parse("2009-07-24T15:05:00.250Z").map(Instant::get),
            Some(1_248_447_900_250)
        );
        assert_eq!(
            format(parse("2100-02-28T23:59:59Z").expect("reads")),
            "2100-02-28T23:59:59Z"
        );
    }

    #[test]
    fn a_word_date_reads_its_utc_instant_or_floats() {
        let wall = parse("2016-05-09T16:13:00Z");
        assert_eq!(
            word_date(Some("2016-05-09T16:13:00Z"), None),
            wall.map(WordDate::Floating)
        );
        assert_eq!(
            word_date(Some("2016-05-09T16:13:00Z"), Some("2016-05-09T14:13:00Z")),
            Some(WordDate::Zoned {
                utc: parse("2016-05-09T14:13:00Z").expect("a date"),
                wall: wall.expect("a date"),
            })
        );
        assert_eq!(
            word_date(Some("2016-05-09T16:13:00+02:00"), None),
            Some(WordDate::Zoned {
                utc: parse("2016-05-09T14:13:00Z").expect("a date"),
                wall: wall.expect("a date"),
            })
        );
        assert_eq!(word_date(None, None), None);
    }

    #[test]
    fn text_that_is_no_date_reads_as_none() {
        for text in [
            "",
            "yesterday",
            "1969-12-31T23:59:59Z",
            "2009-13-01T00:00:00Z",
            "2009-07-24T25:00:00Z",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
