//! The number styles of OpenDocument, to and from the format codes of
//! Excel.
//!
//! OpenDocument spells a number format as elements: a number, a literal, a
//! year, a month. Excel spells it as a code such as `#,##0.00`. The model
//! keeps the code, read through `crate::styles::number_format_of`, so a
//! format reads into the same family whichever file carried it. The reader
//! writes the code the elements spell, and the writer the elements a code
//! spells.
//!
//! A code of one section converts both ways. A code of several sections,
//! such as `[Red]0;-0`, leaves as its first section, and the report names
//! the approximation.

use std::fmt::Write as _;

use super::{Node, push_escaped};

/// The code a boolean style reads as: the code LibreOffice writes for it in
/// an xlsx file.
pub(crate) const BOOLEAN_CODE: &str = "\"TRUE\";\"TRUE\";\"FALSE\"";

/// The code the number style `style` spells, or `None` for an element that
/// is no number style.
pub(crate) fn code_of(style: &Node) -> Option<String> {
    let mut code = String::new();
    match style.name() {
        "boolean-style" => return Some(BOOLEAN_CODE.to_owned()),
        "number-style" if is_general(style) => return Some("General".to_owned()),
        "number-style" | "percentage-style" | "currency-style" | "date-style" | "time-style"
        | "text-style" => {}
        _ => return None,
    }
    let elapsed = style.attr("truncate-on-overflow") == Some("false");
    let parts = style.children();
    for (at, part) in parts.iter().enumerate() {
        let long = part.attr("style") == Some("long");
        match part.name() {
            "number" => push_number(&mut code, part),
            "scientific-number" => {
                push_integer(&mut code, part);
                push_decimals(&mut code, part);
                let digits = count(part, "min-exponent-digits").max(1);
                code.push_str("E+");
                code.push_str(&"0".repeat(digits));
            }
            "fraction" => {
                let whole = count(part, "min-integer-digits");
                if whole > 0 {
                    code.push_str(&"0".repeat(whole));
                    code.push(' ');
                } else {
                    code.push_str("# ");
                }
                code.push_str(&"?".repeat(count(part, "min-numerator-digits").max(1)));
                code.push('/');
                match part.attr("denominator-value") {
                    Some(value) => code.push_str(value),
                    None => {
                        code.push_str(&"?".repeat(count(part, "min-denominator-digits").max(1)))
                    }
                }
            }
            "text" => push_literal(&mut code, part.text()),
            "currency-symbol" => {
                code.push_str("[$");
                code.push_str(part.text());
                code.push(']');
            }
            "text-content" => code.push('@'),
            "year" => code.push_str(if long { "yyyy" } else { "yy" }),
            "month" => code.push_str(match (part.attr("textual") == Some("true"), long) {
                (true, true) => "mmmm",
                (true, false) => "mmm",
                (false, true) => "mm",
                (false, false) => "m",
            }),
            "day" => code.push_str(if long { "dd" } else { "d" }),
            "day-of-week" => code.push_str(if long { "dddd" } else { "ddd" }),
            "hours" => {
                let hours = if long { "hh" } else { "h" };
                if elapsed && at == first_of(parts, "hours") {
                    let _ = write!(code, "[{hours}]");
                } else {
                    code.push_str(hours);
                }
            }
            "minutes" => code.push_str(if long { "mm" } else { "m" }),
            "seconds" => {
                code.push_str(if long { "ss" } else { "s" });
                let places = count(part, "decimal-places");
                if places > 0 {
                    code.push('.');
                    code.push_str(&"0".repeat(places));
                }
            }
            "am-pm" => code.push_str("AM/PM"),
            _ => {}
        }
    }
    (!code.is_empty()).then_some(code)
}

/// Whether the number style `style` is the general one: a lone number with
/// no decimal places named.
fn is_general(style: &Node) -> bool {
    let parts: Vec<&Node> = style
        .children()
        .iter()
        .filter(|c| c.name() != "text-properties")
        .collect();
    matches!(parts.as_slice(), [only] if only.name() == "number"
        && only.attr("decimal-places").is_none()
        && only.attr("grouping") != Some("true")
        && count(only, "min-integer-digits") <= 1)
}

fn first_of(parts: &[Node], name: &str) -> usize {
    parts.iter().position(|p| p.name() == name).unwrap_or(0)
}

fn count(part: &Node, attr: &str) -> usize {
    part.attr(attr)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(30)
}

fn push_integer(code: &mut String, part: &Node) {
    let digits = count(part, "min-integer-digits");
    if part.attr("grouping") == Some("true") {
        code.push_str("#,##");
        code.push_str(&"0".repeat(digits.max(1)));
    } else if digits == 0 {
        code.push('#');
    } else {
        code.push_str(&"0".repeat(digits));
    }
}

fn push_decimals(code: &mut String, part: &Node) {
    let places = count(part, "decimal-places");
    if places == 0 {
        return;
    }
    let shown = part
        .attr("min-decimal-places")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(places)
        .min(places);
    code.push('.');
    code.push_str(&"0".repeat(shown));
    code.push_str(&"#".repeat(places - shown));
}

fn push_number(code: &mut String, part: &Node) {
    push_integer(code, part);
    push_decimals(code, part);
}

/// The characters a code writes bare. Any other literal is quoted.
const BARE: &str = " -/:.,()$%";

fn push_literal(code: &mut String, text: &str) {
    if !text.is_empty() && text.chars().all(|c| BARE.contains(c)) {
        code.push_str(text);
    } else {
        code.push('"');
        code.push_str(&text.replace('"', ""));
        code.push('"');
    }
}

/// One piece of a format code.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    /// A run of digit placeholders: `#,##0.00`, `0.0E+00`, `# ?/?`.
    Digits(String),
    Text(String),
    Currency(String),
    Percent,
    TextContent,
    Year(Width),
    /// A month, or a minute: the code spells both `m`.
    MonthOrMinute(usize),
    Day(Width),
    Weekday(Width),
    Hours(Width, Elapsed),
    Seconds(Width, usize),
    AmPm,
}

/// Whether a date part shows in its long form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Width {
    Short,
    Long,
}

/// Whether an hour count runs past a day: `[h]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Elapsed {
    No,
    Yes,
}

/// The number style element of `code`, named `name`. The second value says
/// whether the element only approximates the code: a second section
/// dropped, or a piece no element spells.
pub(crate) fn style_of(name: &str, code: &str) -> (String, Approximation) {
    let (section, rest) = first_section(code);
    let mut approximation = if rest {
        Approximation::Approximated
    } else {
        Approximation::Exact
    };
    let pieces = match pieces(section) {
        Some(pieces) => pieces,
        None => {
            approximation = Approximation::Approximated;
            Vec::new()
        }
    };
    let dated = pieces.iter().any(|p| {
        matches!(
            p,
            Piece::Year(_) | Piece::Day(_) | Piece::Weekday(_) | Piece::MonthOrMinute(_)
        )
    }) && pieces
        .iter()
        .any(|p| matches!(p, Piece::Year(_) | Piece::Day(_) | Piece::Weekday(_)));
    let timed = pieces
        .iter()
        .any(|p| matches!(p, Piece::Hours(..) | Piece::Seconds(..) | Piece::AmPm));
    let kind = if pieces.contains(&Piece::TextContent) {
        "text-style"
    } else if dated {
        "date-style"
    } else if timed {
        "time-style"
    } else if pieces.iter().any(|p| matches!(p, Piece::Currency(_))) {
        "currency-style"
    } else if pieces.contains(&Piece::Percent) {
        "percentage-style"
    } else {
        "number-style"
    };
    let mut out = format!("<number:{kind} style:name=\"{name}\"");
    if pieces
        .iter()
        .any(|p| matches!(p, Piece::Hours(_, Elapsed::Yes)))
    {
        out.push_str(" number:truncate-on-overflow=\"false\"");
    }
    out.push('>');
    for (at, piece) in pieces.iter().enumerate() {
        let long = |w: Width| match w {
            Width::Short => "",
            Width::Long => " number:style=\"long\"",
        };
        match piece {
            Piece::Digits(digits) => out.push_str(&digits_element(digits)),
            Piece::Text(text) => {
                out.push_str("<number:text>");
                push_escaped(&mut out, text);
                out.push_str("</number:text>");
            }
            Piece::Currency(symbol) => {
                out.push_str("<number:currency-symbol>");
                push_escaped(&mut out, symbol);
                out.push_str("</number:currency-symbol>");
            }
            Piece::Percent => out.push_str("<number:text>%</number:text>"),
            Piece::TextContent => out.push_str("<number:text-content/>"),
            Piece::Year(w) => {
                let _ = write!(out, "<number:year{}/>", long(*w));
            }
            Piece::MonthOrMinute(n) => {
                let minute = is_minute(&pieces, at);
                let (element, textual, width) = match (minute, n) {
                    (true, 1) => ("minutes", "", Width::Short),
                    (true, _) => ("minutes", "", Width::Long),
                    (false, 1) => ("month", "", Width::Short),
                    (false, 2) => ("month", "", Width::Long),
                    (false, 3) => ("month", " number:textual=\"true\"", Width::Short),
                    (false, _) => ("month", " number:textual=\"true\"", Width::Long),
                };
                let _ = write!(out, "<number:{element}{}{textual}/>", long(width));
            }
            Piece::Day(w) => {
                let _ = write!(out, "<number:day{}/>", long(*w));
            }
            Piece::Weekday(w) => {
                let _ = write!(out, "<number:day-of-week{}/>", long(*w));
            }
            Piece::Hours(w, _) => {
                let _ = write!(out, "<number:hours{}/>", long(*w));
            }
            Piece::Seconds(w, places) => {
                let decimals = if *places > 0 {
                    format!(" number:decimal-places=\"{places}\"")
                } else {
                    String::new()
                };
                let _ = write!(out, "<number:seconds{}{decimals}/>", long(*w));
            }
            Piece::AmPm => out.push_str("<number:am-pm/>"),
        }
    }
    let _ = write!(out, "</number:{kind}>");
    (out, approximation)
}

/// Whether a code converted exactly, or only its nearest form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Approximation {
    Exact,
    Approximated,
}

/// Whether the `m` run at `at` is a minute: right after an hour, or right
/// before a second.
fn is_minute(pieces: &[Piece], at: usize) -> bool {
    let significant = |range: &mut dyn Iterator<Item = usize>| -> Option<&Piece> {
        range
            .filter_map(|i| pieces.get(i))
            .find(|p| !matches!(p, Piece::Text(_)))
    };
    let before = significant(&mut (0..at).rev());
    let after = significant(&mut (at + 1..pieces.len()));
    matches!(before, Some(Piece::Hours(..))) || matches!(after, Some(Piece::Seconds(..)))
}

/// The element a run of digit placeholders spells.
fn digits_element(digits: &str) -> String {
    if let Some((whole, fraction)) = digits.split_once('/') {
        let (integer, numerator) = whole.rsplit_once(' ').unwrap_or(("", whole));
        let integer_digits = integer.matches('0').count();
        let denominator = if fraction.bytes().all(|b| b.is_ascii_digit()) && !fraction.is_empty() {
            format!(" number:denominator-value=\"{fraction}\"")
        } else {
            format!(" number:min-denominator-digits=\"{}\"", fraction.len())
        };
        return format!(
            "<number:fraction number:min-integer-digits=\"{integer_digits}\" \
             number:min-numerator-digits=\"{}\"{denominator}/>",
            numerator.len()
        );
    }
    let (mantissa, exponent) = match digits.find(['E', 'e']) {
        Some(at) => (digits.get(..at).unwrap_or(digits), digits.get(at + 2..)),
        None => (digits, None),
    };
    let (integer, decimals) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let grouping = if integer.contains(',') {
        " number:grouping=\"true\""
    } else {
        ""
    };
    let integer_digits = integer.matches('0').count();
    let places = decimals.len();
    let shown = decimals.matches('0').count();
    match exponent {
        Some(exponent) => format!(
            "<number:scientific-number number:decimal-places=\"{places}\" \
             number:min-decimal-places=\"{shown}\" number:min-integer-digits=\"{integer_digits}\"{grouping} \
             number:min-exponent-digits=\"{}\"/>",
            exponent.len()
        ),
        None => format!(
            "<number:number number:decimal-places=\"{places}\" \
             number:min-decimal-places=\"{shown}\" number:min-integer-digits=\"{integer_digits}\"{grouping}/>"
        ),
    }
}

/// The first section of `code`, and whether another follows it.
fn first_section(code: &str) -> (&str, bool) {
    let mut quoted = false;
    let mut escaped = false;
    for (at, c) in code.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => return (code.get(..at).unwrap_or(code), true),
            _ => {}
        }
    }
    (code, false)
}

/// The pieces of one section of a code, or `None` for a piece no element
/// spells: a fill, a condition, a locale tag.
fn pieces(section: &str) -> Option<Vec<Piece>> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut rest = section;
    let text = |pieces: &mut Vec<Piece>, s: &str| match pieces.last_mut() {
        Some(Piece::Text(held)) => held.push_str(s),
        _ => pieces.push(Piece::Text(s.to_owned())),
    };
    while let Some(c) = rest.chars().next() {
        let lower = c.to_ascii_lowercase();
        let run = |ch: char| {
            rest.chars()
                .take_while(|x| x.eq_ignore_ascii_case(&ch))
                .count()
        };
        let len = match c {
            '"' => {
                let end = rest.get(1..)?.find('"')? + 1;
                text(&mut pieces, rest.get(1..end)?);
                end + 1
            }
            '\\' => {
                let ch = rest.chars().nth(1)?;
                text(&mut pieces, &ch.to_string());
                1 + ch.len_utf8()
            }
            '[' => {
                let end = rest.find(']')?;
                let inner = rest.get(1..end)?;
                if let Some(symbol) = inner.strip_prefix('$') {
                    let symbol = symbol.split('-').next().unwrap_or_default();
                    pieces.push(Piece::Currency(symbol.to_owned()));
                } else if inner.eq_ignore_ascii_case("h") || inner.eq_ignore_ascii_case("hh") {
                    let width = if inner.len() > 1 {
                        Width::Long
                    } else {
                        Width::Short
                    };
                    pieces.push(Piece::Hours(width, Elapsed::Yes));
                } else {
                    return None;
                }
                end + 1
            }
            '0' | '#' | '?' => {
                let len = rest
                    .char_indices()
                    .find(|&(i, ch)| !continues_digits(rest, i, ch))
                    .map_or(rest.len(), |(i, _)| i);
                let digits = rest.get(..len)?;
                // A point or a comma that closes the run is a literal.
                let trimmed = digits.trim_end_matches([',', '.']);
                pieces.push(Piece::Digits(trimmed.to_owned()));
                trimmed.len()
            }
            '%' => {
                pieces.push(Piece::Percent);
                1
            }
            '@' => {
                pieces.push(Piece::TextContent);
                1
            }
            '_' | '*' => 1 + rest.chars().nth(1).map_or(0, char::len_utf8),
            _ if lower == 'y' => {
                let n = run('y');
                pieces.push(Piece::Year(if n > 2 { Width::Long } else { Width::Short }));
                n
            }
            _ if lower == 'm' => {
                let n = run('m');
                pieces.push(Piece::MonthOrMinute(n));
                n
            }
            _ if lower == 'd' => {
                let n = run('d');
                pieces.push(match n {
                    1 => Piece::Day(Width::Short),
                    2 => Piece::Day(Width::Long),
                    3 => Piece::Weekday(Width::Short),
                    _ => Piece::Weekday(Width::Long),
                });
                n
            }
            _ if lower == 'h' => {
                let n = run('h');
                let width = if n > 1 { Width::Long } else { Width::Short };
                pieces.push(Piece::Hours(width, Elapsed::No));
                n
            }
            _ if lower == 's' => {
                let n = run('s');
                let width = if n > 1 { Width::Long } else { Width::Short };
                let places = rest
                    .get(n..)
                    .and_then(|r| r.strip_prefix('.'))
                    .map_or(0, |r| r.chars().take_while(|&ch| ch == '0').count());
                pieces.push(Piece::Seconds(width, places));
                n + if places > 0 { 1 + places } else { 0 }
            }
            _ if rest
                .get(..5)
                .is_some_and(|r| r.eq_ignore_ascii_case("AM/PM")) =>
            {
                pieces.push(Piece::AmPm);
                5
            }
            c if c.is_alphabetic() => return None,
            c => {
                text(&mut pieces, &c.to_string());
                c.len_utf8()
            }
        };
        rest = rest.get(len..)?;
    }
    Some(pieces)
}

/// Whether the character `ch` at `at` of `code` continues a run of digit
/// placeholders. A placeholder and a separator do, and so do the space of a
/// fraction and the exponent of a scientific number.
fn continues_digits(code: &str, at: usize, ch: char) -> bool {
    let next = code.get(at + ch.len_utf8()..).unwrap_or_default();
    let before = code.get(..at).unwrap_or_default();
    match ch {
        '0'..='9' | '#' | '?' | ',' | '.' | '/' => true,
        ' ' => next.starts_with(['?', '#']),
        'E' | 'e' => next.starts_with(['+', '-']),
        '+' | '-' => before.ends_with(['E', 'e']),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The code the element `style_of` writes for `code` reads back as.
    fn trip(code: &str) -> Option<String> {
        let (xml, _) = style_of("N1", code);
        let node = super::super::tree(xml.as_bytes()).ok()?;
        code_of(&node)
    }

    #[test]
    fn a_code_of_one_section_survives_the_elements() {
        for code in [
            "0",
            "0.00",
            "#,##0.00",
            "0%",
            "0.00%",
            "0.00E+00",
            "0E+00",
            "# ?/?",
            "# ??/??",
            "yyyy-mm-dd",
            "hh:mm:ss",
            "d-mmm-yy hh:mm",
            "@",
            "0.0\" kg\"",
            "#,##0.00 [$\u{a4}]",
            "[h]:mm:ss",
            "dddd d mmmm yyyy",
            "h:mm AM/PM",
            "mm:ss.0",
        ] {
            assert_eq!(trip(code).as_deref(), Some(code), "{code}");
        }
    }

    #[test]
    fn a_second_section_is_dropped_and_named() {
        let (_, approximation) = style_of("N1", "[Red]0;-0");
        assert_eq!(approximation, Approximation::Approximated);
        let (_, approximation) = style_of("N1", "0;-0");
        assert_eq!(approximation, Approximation::Approximated);
        assert_eq!(trip("0;-0").as_deref(), Some("0"));
    }

    #[test]
    fn libreoffice_styles_read_as_excel_codes() {
        let general = "<number:number-style style:name=\"N0\"><number:number number:min-integer-digits=\"1\"/></number:number-style>";
        let boolean =
            "<number:boolean-style style:name=\"N99\"><number:boolean/></number:boolean-style>";
        let read = |xml: &str| {
            super::super::tree(xml.as_bytes())
                .ok()
                .as_ref()
                .and_then(code_of)
        };
        assert_eq!(read(general).as_deref(), Some("General"));
        assert_eq!(
            read(boolean).as_deref(),
            Some("\"TRUE\";\"TRUE\";\"FALSE\"")
        );
    }
}
