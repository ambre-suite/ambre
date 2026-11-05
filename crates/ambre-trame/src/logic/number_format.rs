// ambre-trame/src/logic/number_format.rs
// The number formats that read a value as a calendar or through a pattern
// (`trame-number-formats`). They are the `Date` and `Time` families, and the
// `Custom` pattern language. `packing::format_number` dispatches here.
//
// The pattern language is the common subset of the spreadsheet formats. A
// pattern holds up to three `;`-separated sections: positive, negative, zero.
// A section holds the digit placeholders `0` and `#`, the decimal point, the
// thousands comma and `%`. It also holds quoted and escaped literals, and the
// date and time tokens `y m d h s AM/PM`. Any other character prints as it
// stands, so no pattern is refused. A mistyped pattern still renders, and the
// result shows the mistake.

use crate::formula::text::LiteralText;
use crate::message::DateParts;
use ambre_core::{Instant, SheetDateTime};

/// Month names, for `mmm` (the first three letters) and `mmmm`.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Day names, for `ddd` (the first three letters) and `dddd`. Serial 1 is a
/// Sunday, so the index is the serial modulo seven, shifted by one.
const DAYS: [&str; 7] = [
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
];

/// A day serial as an ISO calendar date, `2026-09-29` (`TRAME-107`). The ISO
/// order is the one no locale misreads; the document has no locale yet
/// (`TRAME-110`).
pub(crate) fn format_date(serial: Instant<SheetDateTime>) -> String {
    let DateParts {
        year: y,
        month: m,
        day: d,
    } = DateParts::of_day(serial.day());
    let (y, m, d) = (y.get(), m.get(), d.get());
    format!("{y:04}-{m:02}-{d:02}")
}

/// The time of day a serial carries in its fraction, `14:05:09` (`TRAME-107`).
pub(crate) fn format_time(serial: Instant<SheetDateTime>) -> String {
    let TimeOfDay {
        hours: h,
        minutes: m,
        seconds: s,
    } = hms(serial);
    format!("{h:02}:{m:02}:{s:02}")
}

/// A time of day on a clock face: hours from 0 to 23, minutes and seconds
/// from 0 to 59.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeOfDay {
    hours: u32,
    minutes: u32,
    seconds: u32,
}

/// Hours, minutes and seconds of the fraction of `serial`, rounded to the
/// second. A fraction that rounds up to midnight reads `00:00:00`.
fn hms(serial: Instant<SheetDateTime>) -> TimeOfDay {
    let seconds = (serial.time_of_day() * 86_400.0).round() as u32 % 86_400;
    TimeOfDay {
        hours: seconds / 3600,
        minutes: seconds / 60 % 60,
        seconds: seconds % 60,
    }
}

/// One lexed piece of a pattern section.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// Printed as it stands.
    Literal(LiteralText),
    /// `0`: a digit, a zero when the number has none there.
    Zero,
    /// `#`: a digit, nothing when the number has none there.
    Hash,
    /// `.`: the decimal point.
    Point,
    /// `,` between digit placeholders: group the thousands.
    Comma,
    /// `%`: the value is shown times a hundred.
    Percent,
    /// A run of one date or time letter, with its length.
    Date(DateLetter, Run),
    /// `AM/PM`: a twelve-hour clock.
    AmPm,
}

/// How many times a date or time letter repeats in a run, held at four: the
/// widths a letter prints at. A longer run prints as four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Run {
    /// `d`: the number with no padding.
    One,
    /// `dd`: the number on two digits.
    Two,
    /// `ddd`: the short name.
    Three,
    /// `dddd` or longer: the full name, or a four-digit year.
    Four,
}

impl Run {
    /// The run one letter longer, held at four.
    fn longer(self) -> Self {
        match self {
            Self::One => Self::Two,
            Self::Two => Self::Three,
            Self::Three | Self::Four => Self::Four,
        }
    }
}

/// The letter of a date or time token, in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateLetter {
    /// `y`.
    Year,
    /// `m`: the month, or the minutes beside an hour or a second.
    Month,
    /// `d`.
    Day,
    /// `h`.
    Hour,
    /// `s`.
    Second,
}

/// Render `value` through `pattern` (`TRAME-109`).
pub(crate) fn format_custom(value: f64, pattern: &str) -> String {
    let sections = split_sections(pattern);
    // One section serves every value; two split off the negatives; a third
    // serves zero. A section chosen for its sign prints the magnitude.
    let pick = |i: usize| sections.get(i).map_or("", String::as_str);
    let (section, value) = match (sections.len(), value) {
        (2.., v) if v < 0.0 => (pick(1), -v),
        (3.., v) if v == 0.0 => (pick(2), v),
        (_, v) => (pick(0), v),
    };
    let tokens = lex(section);
    if tokens.iter().any(|t| matches!(t, Token::Date(..))) {
        render_date(&tokens, Instant::<SheetDateTime>::new(value))
    } else {
        render_number(&tokens, value)
    }
}

/// Split a pattern on the `;` that sit outside a quote and are not escaped.
fn split_sections(pattern: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut quoted = false;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        let current = sections.last_mut();
        match (c, current) {
            ('"', Some(cur)) => {
                quoted = !quoted;
                cur.push(c);
            }
            ('\\', Some(cur)) => {
                cur.push(c);
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            (';', _) if !quoted => sections.push(String::new()),
            (_, Some(cur)) => cur.push(c),
            (_, None) => {}
        }
    }
    sections
}

/// Lex one section into tokens.
fn lex(section: &str) -> Vec<Token> {
    let chars: Vec<char> = section.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        let token = match c {
            '"' => {
                let tail = chars.get(i..).unwrap_or_default();
                let len = tail.iter().position(|&q| q == '"').unwrap_or(tail.len());
                let text: String = tail.iter().take(len).collect();
                i += len + 1;
                Token::Literal(LiteralText::new(text))
            }
            '\\' => {
                let escaped = chars.get(i).map(char::to_string).unwrap_or_default();
                i += 1;
                Token::Literal(LiteralText::new(escaped))
            }
            '0' => Token::Zero,
            '#' => Token::Hash,
            '.' => Token::Point,
            ',' => Token::Comma,
            '%' => Token::Percent,
            'A' | 'a' if matches!(chars.get(i..i + 4), Some(rest) if rest.iter().collect::<String>().eq_ignore_ascii_case("M/PM")) =>
            {
                i += 4;
                Token::AmPm
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let lower = c.to_ascii_lowercase();
                let letter = match lower {
                    'y' => DateLetter::Year,
                    'm' => DateLetter::Month,
                    'd' => DateLetter::Day,
                    'h' => DateLetter::Hour,
                    _ => DateLetter::Second,
                };
                let same = chars
                    .iter()
                    .skip(i)
                    .take_while(|n| n.to_ascii_lowercase() == lower)
                    .count();
                i += same;
                Token::Date(letter, (0..same).fold(Run::One, |run, _| run.longer()))
            }
            other => Token::Literal(LiteralText::new(other.to_string())),
        };
        tokens.push(token);
    }
    tokens
}

/// Render a numeric section: the placeholders take the digits, the literals
/// stay where they are.
fn render_number(tokens: &[Token], value: f64) -> String {
    let percents = tokens.iter().filter(|t| **t == Token::Percent).count();
    let value = value * 100f64.powi(percents as i32);
    let point = tokens.iter().position(|t| *t == Token::Point);
    let (int_tokens, frac_tokens) = match point {
        Some(p) => (
            tokens.get(..p).unwrap_or_default(),
            tokens.get(p + 1..).unwrap_or_default(),
        ),
        None => (tokens, &[][..]),
    };
    let is_digit = |t: &&Token| matches!(t, Token::Zero | Token::Hash);
    let frac_places = frac_tokens.iter().filter(is_digit).count();
    let thousands = if int_tokens.contains(&Token::Comma) {
        Thousands::Grouped
    } else {
        Thousands::Plain
    };
    // One rounding, at the precision the pattern shows.
    let fixed = format!("{:.*}", frac_places, value.abs());
    let (int_digits, frac_digits) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let int_digits = int_digits.trim_start_matches('0');

    let mut out = String::new();
    if value < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    out.push_str(&place_integer(int_tokens, int_digits, thousands));
    if point.is_some() {
        out.push_str(&place_fraction(frac_tokens, frac_digits));
    }
    out
}

/// How the integer part shows its thousands: a comma in the pattern groups
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thousands {
    /// A comma between each group of three digits.
    Grouped,
    /// The digits run on.
    Plain,
}

/// Fill the integer placeholders from the right. The leftmost one takes every
/// digit left over, so a pattern never truncates a number. Literals between
/// placeholders stay in place, which is how `(000) 000-0000` renders a phone
/// number. The thousands comma groups the digits when no literal sits among
/// the placeholders.
fn place_integer(tokens: &[Token], digits: &str, thousands: Thousands) -> String {
    let placeholders = tokens
        .iter()
        .filter(|t| matches!(t, Token::Zero | Token::Hash))
        .count();
    let interleaved = {
        let first = tokens
            .iter()
            .position(|t| matches!(t, Token::Zero | Token::Hash));
        let last = tokens
            .iter()
            .rposition(|t| matches!(t, Token::Zero | Token::Hash));
        match (first, last) {
            (Some(f), Some(l)) => tokens
                .get(f..=l)
                .unwrap_or_default()
                .iter()
                .any(|t| matches!(t, Token::Literal(_))),
            _ => false,
        }
    };
    let mut digits: Vec<char> = digits.chars().collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut seen = 0;
    for token in tokens.iter().rev() {
        match token {
            Token::Zero | Token::Hash => {
                seen += 1;
                let text = if seen == placeholders {
                    let rest: String = digits.drain(..).collect();
                    match (rest.is_empty(), token) {
                        (true, Token::Zero) => "0".to_owned(),
                        _ => rest,
                    }
                } else {
                    match (digits.pop(), token) {
                        (Some(d), _) => d.to_string(),
                        (None, Token::Zero) => "0".to_owned(),
                        (None, _) => String::new(),
                    }
                };
                out.push(text);
            }
            Token::Literal(text) => out.push(text.as_str().to_owned()),
            Token::Percent => out.push("%".to_owned()),
            Token::AmPm | Token::Date(..) | Token::Comma | Token::Point => {}
        }
    }
    out.reverse();
    let joined: String = out.concat();
    if thousands == Thousands::Grouped && !interleaved {
        group_thousands(&joined)
    } else {
        joined
    }
}

/// Insert a comma every three digits in the leading run of digits of `text`.
fn group_thousands(text: &str) -> String {
    let start = text
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(text.len());
    let run = text[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(text.len(), |e| start + e);
    let digits = &text[start..run];
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(d);
    }
    format!("{}{}{}", &text[..start], grouped, &text[run..])
}

/// Fill the fraction placeholders from the left. A `#` drops a trailing zero
/// that a `0` would print.
fn place_fraction(tokens: &[Token], digits: &str) -> String {
    let digits: Vec<char> = digits.chars().collect();
    // The digits a `#` may drop: the trailing zeros after the last `0`.
    let last_zero = tokens
        .iter()
        .filter(|t| matches!(t, Token::Zero | Token::Hash))
        .enumerate()
        .filter(|(_, t)| **t == Token::Zero)
        .map(|(i, _)| i + 1)
        .last()
        .unwrap_or(0);
    let keep = digits
        .iter()
        .rposition(|&d| d != '0')
        .map_or(0, |p| p + 1)
        .max(last_zero);
    let mut out = String::from(".");
    let mut place = 0;
    for token in tokens {
        match token {
            Token::Zero | Token::Hash => {
                if place < keep
                    && let Some(d) = digits.get(place)
                {
                    out.push(*d);
                }
                place += 1;
            }
            Token::Literal(text) => out.push_str(text.as_str()),
            Token::Percent => out.push('%'),
            Token::AmPm | Token::Date(..) | Token::Comma | Token::Point => {}
        }
    }
    if out == "." { String::new() } else { out }
}

/// Render a date or time section.
fn render_date(tokens: &[Token], serial: Instant<SheetDateTime>) -> String {
    let DateParts {
        year: y,
        month: mo,
        day: d,
    } = DateParts::of_day(serial.day());
    let (y, mo, d) = (y.get(), mo.get(), d.get());
    let TimeOfDay {
        hours: h,
        minutes: mi,
        seconds: s,
    } = hms(serial);
    let twelve_hour = tokens.contains(&Token::AmPm);
    let month = |run: Run| -> String {
        let name = MONTHS
            .get((mo - 1).clamp(0, 11) as usize)
            .copied()
            .unwrap_or("");
        match run {
            Run::One => mo.to_string(),
            Run::Two => format!("{mo:02}"),
            Run::Three => name.chars().take(3).collect(),
            Run::Four => name.to_owned(),
        }
    };
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Date(DateLetter::Year, run) => match run {
                Run::One | Run::Two => out.push_str(&format!("{:02}", y.rem_euclid(100))),
                Run::Three | Run::Four => out.push_str(&format!("{y:04}")),
            },
            Token::Date(DateLetter::Month, run) => {
                // `m` after an hour or before a second is the minutes.
                let after_hour = tokens
                    .iter()
                    .take(i)
                    .rev()
                    .find(|t| matches!(t, Token::Date(..)))
                    .is_some_and(|t| matches!(t, Token::Date(DateLetter::Hour, _)));
                let before_second = tokens
                    .iter()
                    .skip(i + 1)
                    .find(|t| matches!(t, Token::Date(..)))
                    .is_some_and(|t| matches!(t, Token::Date(DateLetter::Second, _)));
                if *run <= Run::Two && (after_hour || before_second) {
                    out.push_str(&pad(mi, *run));
                } else {
                    out.push_str(&month(*run));
                }
            }
            Token::Date(DateLetter::Day, run) => match run {
                Run::One | Run::Two => out.push_str(&pad(d as u32, *run)),
                Run::Three | Run::Four => {
                    let name = DAYS
                        .get(serial.day().get().rem_euclid(7) as usize)
                        .copied()
                        .unwrap_or("");
                    let shown: String = if *run == Run::Three {
                        name.chars().take(3).collect()
                    } else {
                        name.to_owned()
                    };
                    out.push_str(&shown);
                }
            },
            Token::Date(DateLetter::Hour, run) => {
                let hour = match (twelve_hour, h % 12) {
                    (true, 0) => 12,
                    (true, hour) => hour,
                    (false, _) => h,
                };
                out.push_str(&pad(hour, *run));
            }
            Token::Date(DateLetter::Second, run) => out.push_str(&pad(s, *run)),
            Token::AmPm => out.push_str(if h < 12 { "AM" } else { "PM" }),
            Token::Literal(text) => out.push_str(text.as_str()),
            Token::Zero => out.push('0'),
            Token::Hash => out.push('#'),
            Token::Point => out.push('.'),
            Token::Comma => out.push(','),
            Token::Percent => out.push('%'),
        }
    }
    out
}

/// `n` with no padding for a one-letter run, else on two digits.
fn pad(n: u32, run: Run) -> String {
    match run {
        Run::One => n.to_string(),
        Run::Two | Run::Three | Run::Four => format!("{n:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_time_render_the_calendar() {
        // 46294 is 2026-09-29; .5 is noon.
        assert_eq!(
            format_date(Instant::<SheetDateTime>::new(46294.0)),
            "2026-09-29"
        );
        assert_eq!(
            format_date(Instant::<SheetDateTime>::new(46294.75)),
            "2026-09-29"
        );
        assert_eq!(
            format_time(Instant::<SheetDateTime>::new(46294.5)),
            "12:00:00"
        );
        assert_eq!(
            format_time(Instant::<SheetDateTime>::new(0.25 + 5.0 / 86_400.0)),
            "06:00:05"
        );
    }

    #[test]
    fn custom_numeric_patterns() {
        let f = format_custom;
        assert_eq!(f(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(f(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(f(0.5, "0.0%"), "50.0%");
        assert_eq!(f(1.23456, "0.00"), "1.23");
        assert_eq!(f(3.1, "0.###"), "3.1");
        assert_eq!(f(0.25, "#.00"), ".25");
        assert_eq!(f(7.0, "000"), "007");
        assert_eq!(
            f(123456.0, "0"),
            "123456",
            "the leftmost placeholder takes every digit"
        );
        assert_eq!(f(-42.0, "0"), "-42");
    }

    #[test]
    fn custom_literals_and_interleaved_digits() {
        let f = format_custom;
        assert_eq!(f(5551234567.0, "(000) 000-0000"), "(555) 123-4567");
        assert_eq!(f(75001.0, "00000"), "75001");
        assert_eq!(f(42.0, "0 \"kg\""), "42 kg");
        assert_eq!(f(42.0, "\\$0"), "$42");
    }

    #[test]
    fn custom_sections_choose_by_sign() {
        let f = format_custom;
        assert_eq!(f(-5.0, "0;(0)"), "(5)");
        assert_eq!(f(5.0, "0;(0)"), "5");
        assert_eq!(f(0.0, "0;(0);\"zero\""), "zero");
    }

    #[test]
    fn custom_date_and_time_tokens() {
        let f = format_custom;
        let serial = 46294.0 + (14.0 * 3600.0 + 5.0 * 60.0 + 9.0) / 86_400.0;
        assert_eq!(f(serial, "dd/mm/yyyy"), "29/09/2026");
        assert_eq!(f(serial, "d mmm yy"), "29 Sep 26");
        assert_eq!(f(serial, "dddd d mmmm"), "Tuesday 29 September");
        assert_eq!(f(serial, "hh:mm:ss"), "14:05:09");
        assert_eq!(f(serial, "h:mm AM/PM"), "2:05 PM");
    }

    /// The literal part of a format is document content: the Debug form of
    /// its token holds a length alone.
    #[test]
    fn a_literal_token_prints_its_length_alone() {
        let tokens = lex(r#"0 "salary""#);
        let printed = format!("{tokens:?}");
        assert!(!printed.contains("salary"), "{printed}");
        assert!(printed.contains("6 chars"), "{printed}");
    }

    /// A run of a date letter prints at one of four widths, and a run past
    /// four prints as four.
    #[test]
    fn a_date_run_past_four_letters_prints_as_four() {
        let f = format_custom;
        let serial = 46294.0;
        assert_eq!(f(serial, "d|dd|ddd"), "29|29|Tue");
        assert_eq!(f(serial, "ddddd"), f(serial, "dddd"));
        assert_eq!(f(serial, "mmmmmm"), "September");
        assert_eq!(f(serial, "yyyyyy"), "2026");
    }
}
