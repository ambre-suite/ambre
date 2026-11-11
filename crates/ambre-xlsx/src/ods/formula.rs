//! OpenFormula, the formula dialect of OpenDocument (ODF 1.3, part 4), to
//! and from Excel's.
//!
//! The model keeps a formula in Excel's dialect, and the translation to
//! Trame's runs once, in `crate::formula`. So the OpenDocument reader brings
//! each formula to the text Excel would store, and the writer brings it back.
//! A formula then lands in Trame the same whichever format carried it.
//!
//! | OpenFormula | Excel |
//! |---|---|
//! | `of:=` before the text | nothing |
//! | `[.A1]`, `[.A1:.B2]`, `[$Sheet2.A1]` | `A1`, `A1:B2`, `Sheet2!A1` |
//! | `[$'My Sheet'.A1]` | `'My Sheet'!A1` |
//! | `;` between arguments | `,` |
//! | `{1;2\|3;4}` | `{1,2;3,4}` |
//! | `TRUE()`, `FALSE()` | `TRUE`, `FALSE` |
//! | `COM.MICROSOFT.IFS(`, `LEGACY.CHIDIST(` | `_xlfn.IFS(`, `CHIDIST(` |
//! | `msoxl:=` before the text | the text is Excel's already |
//!
//! A formula the writer cannot spell in OpenFormula leaves under `msoxl:`,
//! Excel's own grammar, which LibreOffice reads. A formula the reader cannot
//! bring to Excel's dialect is dropped, its cached value kept.

use std::fmt::Write as _;

use crate::formula::{
    FUTURE_FUNCTIONS, Kind, WORKSHEET_FUNCTIONS, parse_cell, parse_col, parse_row, reference_parts,
    tokenize,
};
use crate::model::ErrorValue;

/// The Excel functions OpenFormula names under `COM.MICROSOFT.`, as
/// LibreOffice 24.2 writes them. Each one but `CEILING` and `FLOOR` is a
/// function Excel stores behind `_xlfn.`.
const MICROSOFT: &[&str] = &[
    "AGGREGATE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.INV",
    "CEILING",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPON.DIST",
    "F.DIST.RT",
    "F.INV.RT",
    "F.TEST",
    "FILTERXML",
    "FLOOR",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.LINEAR",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "HYPGEOM.DIST",
    "IFS",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "NEGBINOM.DIST",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANK.AVG",
    "RANK.EQ",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TEXTJOIN",
    "VAR.P",
    "VAR.S",
    "WEIBULL.DIST",
    "Z.TEST",
];

/// The Excel functions OpenFormula names under `LEGACY.`: the statistics
/// Excel 2010 renamed, kept under their old names.
const LEGACY: &[&str] = &[
    "CHIDIST",
    "CHIINV",
    "CHITEST",
    "FDIST",
    "FINV",
    "NORMSDIST",
    "NORMSINV",
    "TDIST",
];

/// The functions the two dialects spell apart: the Excel name, which takes
/// `_xlfn.`, and the OpenFormula name.
const RENAMED: &[(&str, &str)] = &[
    ("F.DIST", "FDIST"),
    ("F.INV", "FINV"),
    ("FORMULATEXT", "FORMULA"),
    ("SKEW.P", "SKEWP"),
];

/// The newer Excel functions OpenFormula has no name for. LibreOffice keeps
/// each one behind Excel's own `_xlfn.` prefix.
const UNNAMED: &[&str] = &[
    "ARRAYTOTEXT",
    "BINOM.DIST.RANGE",
    "BYCOL",
    "BYROW",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "DROP",
    "EXPAND",
    "FIELDVALUE",
    "HSTACK",
    "IMCOSH",
    "IMCOT",
    "IMCSC",
    "IMCSCH",
    "IMSEC",
    "IMSECH",
    "IMSINH",
    "IMTAN",
    "ISOMITTED",
    "LAMBDA",
    "LET",
    "MAKEARRAY",
    "MAP",
    "NETWORKDAYS.INTL",
    "RANDARRAY",
    "REDUCE",
    "SCAN",
    "SEQUENCE",
    "SORTBY",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "UNIQUE",
    "VALUETOTEXT",
    "VSTACK",
    "WORKDAY.INTL",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
];

fn holds(list: &[&str], name: &str) -> bool {
    list.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// The Excel text of the formula a cell's `table:formula` attribute holds.
/// `None` when no Excel text spells it: an unknown grammar, a reference to
/// another file, a text that does not lex.
pub(crate) fn excel_of(attribute: &str) -> Option<String> {
    let (grammar, body) = match attribute.split_once(':') {
        Some((grammar, body))
            if !grammar.is_empty() && grammar.bytes().all(|b| b.is_ascii_alphabetic()) =>
        {
            (Some(grammar), body)
        }
        _ => (None, attribute),
    };
    let body = body.strip_prefix('=').unwrap_or(body);
    match grammar {
        Some("msoxl") => (!body.is_empty()).then(|| body.to_owned()),
        Some("of" | "oooc") | None => from_open(body),
        Some(_) => None,
    }
}

/// The `table:formula` attribute of the Excel formula `excel`: OpenFormula
/// where the translation reaches it, Excel's grammar under `msoxl:`
/// otherwise.
pub(crate) fn attribute_of(excel: &str) -> String {
    to_open(excel).map_or_else(|| format!("msoxl:={excel}"), |open| format!("of:={open}"))
}

/// The Excel text of the OpenFormula text `open`.
fn from_open(open: &str) -> Option<String> {
    let mut out = String::with_capacity(open.len());
    let mut braces = 0usize;
    let mut rest = open;
    while let Some(c) = rest.chars().next() {
        let len = match c {
            '"' => {
                let len = quoted_len(rest, '"')?;
                out.push_str(rest.get(..len)?);
                len
            }
            '[' => {
                let len = bracket_len(rest)?;
                out.push_str(&reference_to_excel(rest.get(1..len - 1)?)?);
                len
            }
            '{' => {
                braces += 1;
                out.push('{');
                1
            }
            '}' => {
                braces = braces.checked_sub(1)?;
                out.push('}');
                1
            }
            ';' | '~' => {
                out.push(',');
                1
            }
            '|' if braces > 0 => {
                out.push(';');
                1
            }
            // The intersection operator, a space in Excel.
            '!' => {
                out.push(' ');
                1
            }
            '#' => {
                let len = error_len(rest).unwrap_or(1);
                out.push_str(rest.get(..len)?);
                len
            }
            c if c.is_ascii_digit() => {
                let len = number_len(rest);
                out.push_str(rest.get(..len)?);
                len
            }
            c if c.is_alphabetic() || c == '_' => {
                let len = rest
                    .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '.' | '$')))
                    .unwrap_or(rest.len());
                let word = rest.get(..len)?;
                let after = rest.get(len..)?.trim_start();
                if after.starts_with('(') {
                    let empty_call = after.get(1..)?.trim_start().starts_with(')');
                    let constant = ["TRUE", "FALSE"]
                        .into_iter()
                        .find(|b| b.eq_ignore_ascii_case(word));
                    match constant {
                        Some(constant) if empty_call => {
                            out.push_str(constant);
                            // The call's parentheses, and the spaces around them.
                            let open_at = rest.get(len..)?.find('(')? + len;
                            let close_at = rest.get(open_at..)?.find(')')? + open_at;
                            rest = rest.get(close_at + 1..)?;
                            continue;
                        }
                        _ => out.push_str(&excel_function(word)),
                    }
                } else {
                    out.push_str(word);
                }
                len
            }
            c => {
                out.push(c);
                c.len_utf8()
            }
        };
        rest = rest.get(len..)?;
    }
    (braces == 0 && !out.is_empty()).then_some(out)
}

/// The Excel name of the OpenFormula function `name`.
fn excel_function(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    if let Some(base) = upper.strip_prefix("COM.MICROSOFT.") {
        return prefixed(base);
    }
    if let Some(base) = upper.strip_prefix("LEGACY.") {
        return base.to_owned();
    }
    if upper.starts_with("_XLFN.") {
        return format!("_xlfn.{}", upper.get(6..).unwrap_or_default())
            .replace("_xlfn._XLWS.", "_xlfn._xlws.");
    }
    if let Some((excel, _)) = RENAMED.iter().find(|(_, open)| *open == upper) {
        return format!("_xlfn.{excel}");
    }
    if holds(FUTURE_FUNCTIONS, &upper) {
        return format!("_xlfn.{upper}");
    }
    name.to_owned()
}

/// `base` behind the prefix Excel stores it with, if it takes one.
fn prefixed(base: &str) -> String {
    if holds(WORKSHEET_FUNCTIONS, base) {
        format!("_xlfn._xlws.{base}")
    } else if holds(FUTURE_FUNCTIONS, base) {
        format!("_xlfn.{base}")
    } else {
        base.to_owned()
    }
}

/// The OpenFormula name of the Excel function `name`.
fn open_function(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let (base, future) = match upper
        .strip_prefix("_XLFN._XLWS.")
        .or_else(|| upper.strip_prefix("_XLFN."))
    {
        Some(base) => (base, true),
        None => (upper.as_str(), false),
    };
    if future {
        if let Some((_, open)) = RENAMED.iter().find(|(excel, _)| *excel == base) {
            return (*open).to_owned();
        }
        if holds(MICROSOFT, base) {
            return format!("COM.MICROSOFT.{base}");
        }
        if holds(UNNAMED, base) || holds(WORKSHEET_FUNCTIONS, base) {
            return name.to_owned();
        }
        return base.to_owned();
    }
    if holds(MICROSOFT, base) {
        format!("COM.MICROSOFT.{base}")
    } else if holds(LEGACY, base) {
        format!("LEGACY.{base}")
    } else {
        name.to_owned()
    }
}

/// The Excel text of the inside of one OpenFormula reference: `.A1`,
/// `$Sheet1.A1:.B2`, `'My Sheet'.A:.C`.
fn reference_to_excel(inner: &str) -> Option<String> {
    let (first, second) = split_unquoted(inner, ':');
    let first = address(first)?;
    let second = match second {
        Some(text) => Some(address(text)?),
        None => None,
    };
    if first.cell == "#REF!" || second.as_ref().is_some_and(|s| s.cell == "#REF!") {
        return Some("#REF!".to_owned());
    }
    let mut out = String::new();
    match (&first.sheet, second.as_ref().and_then(|s| s.sheet.as_ref())) {
        (Some(a), Some(b)) if a != b => {
            let joined = format!("{a}:{b}");
            if needs_quotes(a) || needs_quotes(b) {
                push_quoted(&mut out, &joined);
            } else {
                out.push_str(&joined);
            }
            out.push('!');
        }
        (Some(a), _) => {
            if needs_quotes(a) {
                push_quoted(&mut out, a);
            } else {
                out.push_str(a);
            }
            out.push('!');
        }
        (None, Some(_)) => return None,
        (None, None) => {}
    }
    out.push_str(&first.cell);
    if let Some(second) = second {
        out.push(':');
        out.push_str(&second.cell);
    }
    Some(out)
}

/// One end of an OpenFormula reference: its sheet, when it names one, and
/// its cell, column or row in Excel's spelling.
struct Address {
    sheet: Option<String>,
    cell: String,
}

/// The address `text` spells, or `None` for one that names another file or
/// no cell.
fn address(text: &str) -> Option<Address> {
    let text = text.strip_prefix('$').unwrap_or(text);
    let (sheet, cell) = if let Some(rest) = text.strip_prefix('\'') {
        let len = quoted_len(text, '\'')?;
        let name = rest.get(..len - 2)?.replace("''", "'");
        (Some(name), text.get(len..)?.strip_prefix('.')?)
    } else if let Some(cell) = text.strip_prefix('.') {
        (None, cell)
    } else {
        match text.rfind('.') {
            Some(at) => (Some(text.get(..at)?.to_owned()), text.get(at + 1..)?),
            None => (None, text),
        }
    };
    if cell.eq_ignore_ascii_case("#REF!") || sheet.as_deref() == Some("#REF!") {
        return Some(Address {
            sheet: None,
            cell: "#REF!".to_owned(),
        });
    }
    if sheet.as_deref().is_some_and(|s| s.contains('#')) {
        return None;
    }
    let readable =
        parse_cell(cell).is_some() || parse_col(cell).is_some() || parse_row(cell).is_some();
    readable.then(|| Address {
        sheet,
        cell: cell.to_owned(),
    })
}

/// Whether Excel quotes the sheet name `name` in a reference.
fn needs_quotes(name: &str) -> bool {
    let plain = name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
    !plain
        || parse_cell(name).is_some()
        || name.eq_ignore_ascii_case("R")
        || name.eq_ignore_ascii_case("C")
}

fn push_quoted(out: &mut String, name: &str) {
    out.push('\'');
    out.push_str(&name.replace('\'', "''"));
    out.push('\'');
}

/// The OpenFormula text of the Excel formula `excel`. `None` when
/// OpenFormula cannot spell it: a structured or external reference, a
/// reference across sheets, a text that does not lex.
fn to_open(excel: &str) -> Option<String> {
    let tokens = tokenize(excel)?;
    let parts = reference_parts(&tokens);
    let mut out = String::with_capacity(excel.len() + 8);
    let mut at = 0usize;
    // The next significant token at or after `from`, and its index.
    let next = |from: usize| -> Option<usize> {
        (from..tokens.len()).find(|&i| tokens.get(i).is_some_and(|t| t.kind != Kind::Space))
    };
    while let Some(token) = tokens.get(at) {
        let part = parts.get(at).copied().flatten();
        match token.kind {
            Kind::Sheet => {
                let name = sheet_name(token.text)?;
                let first = next(at + 1)?;
                parts.get(first).copied().flatten()?;
                let (cells, end) = range_at(&tokens, &parts, first)?;
                out.push_str("[$");
                push_open_sheet(&mut out, &name);
                out.push('.');
                out.push_str(&cells);
                out.push(']');
                at = end;
                continue;
            }
            _ if part.is_some() => {
                let (cells, end) = range_at(&tokens, &parts, at)?;
                out.push_str("[.");
                out.push_str(&cells);
                out.push(']');
                at = end;
                continue;
            }
            Kind::Word => {
                let is_call = tokens.get(at + 1).is_some_and(|n| n.kind == Kind::Open);
                if is_call {
                    out.push_str(&open_function(token.text));
                } else if token.text.eq_ignore_ascii_case("TRUE")
                    || token.text.eq_ignore_ascii_case("FALSE")
                {
                    out.push_str(&token.text.to_ascii_uppercase());
                    out.push_str("()");
                } else {
                    out.push_str(token.text);
                }
            }
            Kind::Comma => out.push(';'),
            Kind::Brace => out.push_str(&array_to_open(token.text)?),
            Kind::Bracket => return None,
            Kind::Op if token.text == ":" || token.text == ";" => return None,
            Kind::Str
            | Kind::Error
            | Kind::Number
            | Kind::Open
            | Kind::Close
            | Kind::Space
            | Kind::Op => out.push_str(token.text),
        }
        at += 1;
    }
    Some(out)
}

/// The OpenFormula cells of the reference that starts at token `at`, `A1`
/// or `A1:.B2`, and the index of the token after it.
fn range_at(
    tokens: &[crate::formula::Token<'_>],
    parts: &[Option<crate::formula::RefPart>],
    at: usize,
) -> Option<(String, usize)> {
    let first = tokens.get(at)?;
    parts.get(at).copied().flatten()?;
    let is_colon = tokens
        .get(at + 1)
        .is_some_and(|t| t.kind == Kind::Op && t.text == ":");
    let second = parts.get(at + 2).copied().flatten();
    match (is_colon, second, tokens.get(at + 2)) {
        (true, Some(_), Some(end)) => Some((format!("{}:.{}", first.text, end.text), at + 3)),
        _ => Some((first.text.to_owned(), at + 1)),
    }
}

/// The sheet name a sheet prefix token spells: `Sheet1!` or `'My Sheet'!`.
fn sheet_name(token: &str) -> Option<String> {
    let name = token.strip_suffix('!')?;
    if name.contains(':') {
        return None;
    }
    match name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')) {
        Some(quoted) => Some(quoted.replace("''", "'")),
        None => Some(name.to_owned()),
    }
}

/// Write `name` as an OpenFormula reference spells a sheet: bare when it is
/// a plain word, quoted otherwise.
fn push_open_sheet(out: &mut String, name: &str) {
    let plain = name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
    if plain {
        out.push_str(name);
    } else {
        push_quoted(out, name);
    }
}

/// The OpenFormula spelling of the Excel array constant `array`, braces
/// included: a column ends on `;` and a row on `|`.
fn array_to_open(array: &str) -> Option<String> {
    let mut out = String::with_capacity(array.len());
    let mut rest = array;
    while let Some(c) = rest.chars().next() {
        let len = match c {
            '"' => {
                let len = quoted_len(rest, '"')?;
                out.push_str(rest.get(..len)?);
                len
            }
            ',' => {
                out.push(';');
                1
            }
            ';' => {
                out.push('|');
                1
            }
            c => {
                let _ = write!(out, "{c}");
                c.len_utf8()
            }
        };
        rest = rest.get(len..)?;
    }
    Some(out)
}

/// The length of the quoted run at the start of `text`, a doubled quote
/// standing for one.
fn quoted_len(text: &str, quote: char) -> Option<usize> {
    let mut chars = text.char_indices().skip(1).peekable();
    while let Some((at, c)) = chars.next() {
        if c == quote {
            if chars.peek().is_some_and(|&(_, n)| n == quote) {
                chars.next();
            } else {
                return Some(at + c.len_utf8());
            }
        }
    }
    None
}

/// The length of the reference at the start of `text`, brackets included.
/// A quoted sheet name may hold a bracket.
fn bracket_len(text: &str) -> Option<usize> {
    let mut quoted = false;
    for (at, c) in text.char_indices().skip(1) {
        match c {
            '\'' => quoted = !quoted,
            ']' if !quoted => return Some(at + 1),
            _ => {}
        }
    }
    None
}

/// `text` split at its first `separator` outside quotes.
fn split_unquoted(text: &str, separator: char) -> (&str, Option<&str>) {
    let mut quoted = false;
    for (at, c) in text.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            c if c == separator && !quoted => {
                return (
                    text.get(..at).unwrap_or(text),
                    text.get(at + c.len_utf8()..),
                );
            }
            _ => {}
        }
    }
    (text, None)
}

/// The length of the error literal at the start of `text`, if one is there.
fn error_len(text: &str) -> Option<usize> {
    ErrorValue::ALL
        .iter()
        .map(|e| e.code())
        .find(|code| {
            text.get(..code.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(code))
        })
        .map(str::len)
}

/// The length of the number at the start of `text`: digits, a point, and an
/// exponent.
fn number_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut at = bytes
        .iter()
        .take_while(|b| b.is_ascii_digit() || **b == b'.')
        .count();
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(at + 1), Some(b'+' | b'-')));
        let digits = bytes
            .iter()
            .skip(at + 1 + sign)
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0 {
            at += 1 + sign + digits;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libreoffice_formulas_read_as_the_text_excel_stores() {
        for (open, excel) in [
            ("of:=SUM([.B1:.C1])", "SUM(B1:C1)"),
            (
                "of:=IF([.B1]>1;\"big\";\"small\")",
                "IF(B1>1,\"big\",\"small\")",
            ),
            ("of:=[$Hidden.A1]*2", "Hidden!A1*2"),
            ("of:=[$'My Sheet'.$A$1]+[.A:.C]", "'My Sheet'!$A$1+A:C"),
            ("of:=SUM([$Jan.A1:$Mar.B2])", "SUM(Jan:Mar!A1:B2)"),
            (
                "of:=COM.MICROSOFT.IFS([.A1]>0;1;TRUE();0)",
                "_xlfn.IFS(A1>0,1,TRUE,0)",
            ),
            (
                "of:=LEGACY.CHIDIST(1;2)+FDIST(1;2;3)",
                "CHIDIST(1,2)+_xlfn.F.DIST(1,2,3)",
            ),
            (
                "of:=IFNA(1;2)+COM.MICROSOFT.CEILING(1)",
                "_xlfn.IFNA(1,2)+CEILING(1)",
            ),
            (
                "of:=_xlfn.xlookup(1)+_xlfn._xlws.filter(1)",
                "_xlfn.XLOOKUP(1)+_xlfn._xlws.FILTER(1)",
            ),
            ("of:=SUM({1;2|3;4})", "SUM({1,2;3,4})"),
            ("of:=[.#REF!]+[$#REF!.A1]", "#REF!+#REF!"),
            ("of:=[$'It''s'.A1]&\"[.x];\"", "'It''s'!A1&\"[.x];\""),
            ("of:=[$A1.B2]", "'A1'!B2"),
            ("msoxl:=Table1[Col]", "Table1[Col]"),
            ("of:=1E+3/#DIV/0!", "1E+3/#DIV/0!"),
        ] {
            assert_eq!(excel_of(open).as_deref(), Some(excel), "{open}");
        }
    }

    #[test]
    fn a_formula_no_excel_text_spells_reads_as_none() {
        for open in [
            "of:=['file:///x.ods'#$Sheet1.A1]",
            "of:=SUM([.A1)",
            "of:=\"open",
            "of:=1}",
            "ooow:=x",
            "of:=",
        ] {
            assert_eq!(excel_of(open), None, "{open}");
        }
    }

    #[test]
    fn an_excel_formula_survives_the_trip_through_openformula() {
        for excel in [
            "SUM(B1:C1)*2",
            "IF(B1>1,\"big\",\"small\")",
            "Hidden!A1*2+'My Sheet'!$B$2:C3",
            "_xlfn.IFS(A1>0,\"pos\",TRUE,\"neg\")",
            "_xlfn.CONCAT(\"a\",B2)&FALSE",
            "_xlfn.F.DIST(1,2,3,TRUE)+CHIDIST(1,2)+CEILING(2.5)",
            "_xlfn.XLOOKUP(1,A:A,B:B)+SUM(2:3)",
            "_xlfn._xlws.SORT(A1:A3)",
            "SUM({1,2;3,4})+10%",
            "A1&\"x;y\"",
            "IF(A1,#N/A,#DIV/0!)",
            "Table1[Amount]",
            "[1]Sheet1!A1",
            "SUM(Sheet1:Sheet3!A1)",
            "SUM( A1 , 2 )",
        ] {
            let attribute = attribute_of(excel);
            assert_eq!(excel_of(&attribute).as_deref(), Some(excel), "{attribute}");
        }
        assert_eq!(attribute_of("SUM(A1:B2)"), "of:=SUM([.A1:.B2])");
        assert_eq!(attribute_of("Table1[Amount]"), "msoxl:=Table1[Amount]");
        assert_eq!(
            attribute_of("_xlfn.IFS(A1,TRUE,1)"),
            "of:=COM.MICROSOFT.IFS([.A1];TRUE();1)"
        );
    }
}
