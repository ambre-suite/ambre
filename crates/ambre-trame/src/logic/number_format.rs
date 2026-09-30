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

use crate::formula::functions::ymd_from_serial;

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
pub(crate) fn format_date(serial: f64) -> String {
    let (y, m, d) = ymd_from_serial(serial.floor() as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The time of day a serial carries in its fraction, `14:05:09` (`TRAME-107`).
pub(crate) fn format_time(serial: f64) -> String {
    let (h, m, s) = hms(serial);
    format!("{h:02}:{m:02}:{s:02}")
}

/// Hours, minutes and seconds of the fraction of `serial`, rounded to the
/// second. A fraction that rounds up to midnight reads `00:00:00`.
fn hms(serial: f64) -> (u32, u32, u32) {
    let seconds = ((serial - serial.floor()) * 86_400.0).round() as u32 % 86_400;
    (seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// One lexed piece of a pattern section.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// Printed as it stands.
    Literal(String),
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
    /// A run of one date or time letter, lower-cased, with its length.
    Date(char, usize),
    /// `AM/PM`: a twelve-hour clock.
    AmPm,
}

/// Render `value` through `pattern` (`TRAME-109`).
pub(crate) fn format_custom(value: f64, pattern: &str) -> String {
    let sections = split_sections(pattern);
    // One section serves every value; two split off the negatives; a third
    // serves zero. A section chosen for its sign prints the magnitude.
    let (section, value) = match (sections.len(), value) {
        (2.., v) if v < 0.0 => (sections[1].as_str(), -v),
        (3.., v) if v == 0.0 => (sections[2].as_str(), v),
        (_, v) => (sections[0].as_str(), v),
    };
    let tokens = lex(section);
    if tokens.iter().any(|t| matches!(t, Token::Date(..))) {
        render_date(&tokens, value)
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
                let end = chars[i..]
                    .iter()
                    .position(|&q| q == '"')
                    .map_or(chars.len(), |p| i + p);
                let text: String = chars[i..end].iter().collect();
                i = end + 1;
                Token::Literal(text)
            }
            '\\' => {
                let escaped = chars.get(i).map(char::to_string).unwrap_or_default();
                i += 1;
                Token::Literal(escaped)
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
                let letter = c.to_ascii_lowercase();
                let run = chars[i..]
                    .iter()
                    .take_while(|n| n.to_ascii_lowercase() == letter)
                    .count();
                i += run;
                Token::Date(letter, run + 1)
            }
            other => Token::Literal(other.to_string()),
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
        Some(p) => (&tokens[..p], &tokens[p + 1..]),
        None => (tokens, &[][..]),
    };
    let is_digit = |t: &&Token| matches!(t, Token::Zero | Token::Hash);
    let frac_places = frac_tokens.iter().filter(is_digit).count();
    let grouped = int_tokens.contains(&Token::Comma);
    // One rounding, at the precision the pattern shows.
    let fixed = format!("{:.*}", frac_places, value.abs());
    let (int_digits, frac_digits) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let int_digits = int_digits.trim_start_matches('0');

    let mut out = String::new();
    if value < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    out.push_str(&place_integer(int_tokens, int_digits, grouped));
    if point.is_some() {
        out.push_str(&place_fraction(frac_tokens, frac_digits));
    }
    out
}

/// Fill the integer placeholders from the right. The leftmost one takes every
/// digit left over, so a pattern never truncates a number. Literals between
/// placeholders stay in place, which is how `(000) 000-0000` renders a phone
/// number. The thousands comma groups the digits when no literal sits among
/// the placeholders.
fn place_integer(tokens: &[Token], digits: &str, grouped: bool) -> String {
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
            (Some(f), Some(l)) => tokens[f..=l].iter().any(|t| matches!(t, Token::Literal(_))),
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
            Token::Literal(text) => out.push(text.clone()),
            Token::Percent => out.push("%".to_owned()),
            Token::AmPm | Token::Date(..) | Token::Comma | Token::Point => {}
        }
    }
    out.reverse();
    let joined: String = out.concat();
    if grouped && !interleaved {
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
            Token::Literal(text) => out.push_str(text),
            Token::Percent => out.push('%'),
            Token::AmPm | Token::Date(..) | Token::Comma | Token::Point => {}
        }
    }
    if out == "." { String::new() } else { out }
}

/// Render a date or time section.
fn render_date(tokens: &[Token], serial: f64) -> String {
    let (y, mo, d) = ymd_from_serial(serial.floor() as i64);
    let (h, mi, s) = hms(serial);
    let twelve_hour = tokens.contains(&Token::AmPm);
    let month = |len: usize| -> String {
        let name = MONTHS
            .get((mo - 1).clamp(0, 11) as usize)
            .copied()
            .unwrap_or("");
        match len {
            1 => mo.to_string(),
            2 => format!("{mo:02}"),
            3 => name.chars().take(3).collect(),
            _ => name.to_owned(),
        }
    };
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Date('y', len) => match len {
                1 | 2 => out.push_str(&format!("{:02}", y.rem_euclid(100))),
                _ => out.push_str(&format!("{y:04}")),
            },
            Token::Date('m', len) => {
                // `m` after an hour or before a second is the minutes.
                let after_hour = tokens[..i]
                    .iter()
                    .rev()
                    .find(|t| matches!(t, Token::Date(..)))
                    .is_some_and(|t| matches!(t, Token::Date('h', _)));
                let before_second = tokens[i + 1..]
                    .iter()
                    .find(|t| matches!(t, Token::Date(..)))
                    .is_some_and(|t| matches!(t, Token::Date('s', _)));
                if *len <= 2 && (after_hour || before_second) {
                    out.push_str(&pad(mi, *len));
                } else {
                    out.push_str(&month(*len));
                }
            }
            Token::Date('d', len) => match len {
                1 | 2 => out.push_str(&pad(d as u32, *len)),
                _ => {
                    let name = DAYS[(serial.floor() as i64).rem_euclid(7) as usize];
                    let shown: String = if *len == 3 {
                        name.chars().take(3).collect()
                    } else {
                        name.to_owned()
                    };
                    out.push_str(&shown);
                }
            },
            Token::Date('h', len) => {
                let hour = match (twelve_hour, h % 12) {
                    (true, 0) => 12,
                    (true, hour) => hour,
                    (false, _) => h,
                };
                out.push_str(&pad(hour, *len));
            }
            Token::Date('s', len) => out.push_str(&pad(s, *len)),
            Token::Date(..) => {}
            Token::AmPm => out.push_str(if h < 12 { "AM" } else { "PM" }),
            Token::Literal(text) => out.push_str(text),
            Token::Zero => out.push('0'),
            Token::Hash => out.push('#'),
            Token::Point => out.push('.'),
            Token::Comma => out.push(','),
            Token::Percent => out.push('%'),
        }
    }
    out
}

/// `n` on at least `len` digits, `len` capped at two.
fn pad(n: u32, len: usize) -> String {
    if len >= 2 {
        format!("{n:02}")
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_time_render_the_calendar() {
        // 46294 is 2026-09-29; .5 is noon.
        assert_eq!(format_date(46294.0), "2026-09-29");
        assert_eq!(format_date(46294.75), "2026-09-29");
        assert_eq!(format_time(46294.5), "12:00:00");
        assert_eq!(format_time(0.25 + 5.0 / 86_400.0), "06:00:05");
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
}
