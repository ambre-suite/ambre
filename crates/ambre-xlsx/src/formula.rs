//! The two formula dialects: Excel's, as a worksheet part stores it, and
//! Trame's, as a user types it.
//!
//! The two agree on most of the grammar. Both read A1 references,
//! `Sheet!A1` and `'My Sheet'!A1`, and whole lines `A:C` and `2:5`. Both read
//! strings, numbers, booleans, function calls and the operators. They differ in seven places, and the
//! translation holds each one:
//!
//! | Excel | Trame | Translation |
//! |---|---|---|
//! | no leading `=` | a leading `=` | added on import, removed on export |
//! | `$A$1`, an anchored reference | no anchor | dropped on import, reported |
//! | `_xlfn.IFS(`, a function newer than 2007 | `IFS(` | prefix removed on import, added on export |
//! | `#VALUE!` | `#TYPE!` | renamed both ways |
//! | `#NULL!`, `#NAME?`, `#SPILL!`, `#CALC!`, `#GETTING_DATA` | none | the formula stays Excel's |
//! | `{1,2}`, `Table[Col]`, `[1]Sheet!A1`, `Sheet1:Sheet3!A1`, `A1 B1`, `(A1,B1)`, `10%`, `A1#`, `@A1` | none | the formula stays Excel's |
//! | a shared formula, stored once for a block | one formula per cell | expanded on import, each copy shifted |
//!
//! A formula that stays Excel's keeps its text byte for byte, and the export
//! writes it back unchanged.

use std::fmt::Write as _;

use ambre_core::index::{Delta, Pos};
use ambre_core::{Rule, Validated};
use serde::{Deserialize, Serialize};

use crate::cellref::{self, CellRange, Col, LAST_COL, LAST_ROW, Row};
use crate::report::{Outcome, Report, XlsxConstruct};

/// The longest formula either dialect holds: 8 192 characters, Excel's cap
/// and Trame's.
const MAX_FORMULA_CHARS: usize = 8_192;

/// The rule of a formula in Excel's dialect: no leading `=`, and at most
/// 8 192 characters.
pub enum ExcelFormula {}

impl Rule for ExcelFormula {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        if text.is_empty() || text.chars().count() > MAX_FORMULA_CHARS {
            return Err("an Excel formula holds 1 to 8192 characters");
        }
        Ok(text)
    }
}

/// The rule of a formula in Trame's dialect: a leading `=`, then at most
/// 8 192 characters.
pub enum AmbreFormula {}

impl Rule for AmbreFormula {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        let body = text
            .strip_prefix('=')
            .ok_or("a Trame formula starts with =")?;
        if body.is_empty() || body.chars().count() > MAX_FORMULA_CHARS {
            return Err("a Trame formula holds 1 to 8192 characters after its =");
        }
        Ok(text)
    }
}

/// The formula of one cell, in both dialects where both can spell it.
///
/// The Excel text is always there: the export writes it. The Trame text is
/// there when the translation reached one; a formula without it stays
/// Excel's, and Trame keeps it verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Formula {
    excel: Validated<ExcelFormula>,
    ambre: Option<Validated<AmbreFormula>>,
    /// The range a legacy array formula fills, which only Excel computes.
    array: Option<CellRange>,
}

/// Why a formula text did not become a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaError {
    /// A Trame text that does not start with `=`, or an empty one.
    NotAFormula,
    /// A text past [`MAX_FORMULA_CHARS`].
    TooLong,
}

impl Formula {
    /// The formula a worksheet part stores as `excel`, filling `array` when
    /// it is a legacy array formula. Each dialect difference the
    /// translation meets is counted in `report`.
    ///
    /// # Errors
    ///
    /// [`FormulaError::TooLong`] past the cap, and
    /// [`FormulaError::NotAFormula`] for an empty text.
    pub fn from_excel(
        excel: &str,
        array: Option<CellRange>,
        report: &mut Report,
    ) -> Result<Self, FormulaError> {
        let text = Validated::<ExcelFormula>::parse(excel.to_owned())
            .map_err(|_| too_long_or_empty(excel))?;
        let ambre = if array.is_some() {
            None
        } else {
            excel_to_ambre(excel, report)
        };
        if ambre.is_some() {
            report.record(XlsxConstruct::Formula, Outcome::Carried);
        } else {
            report.record(XlsxConstruct::ForeignFormula, Outcome::Approximated);
        }
        Ok(Self {
            excel: text,
            ambre,
            array,
        })
    }

    /// The formula a Trame user typed as `ambre`, its Excel text derived.
    ///
    /// # Errors
    ///
    /// [`FormulaError::NotAFormula`] for a text without its `=`, and
    /// [`FormulaError::TooLong`] past the cap.
    pub fn from_ambre(ambre: &str) -> Result<Self, FormulaError> {
        let text = Validated::<AmbreFormula>::parse(ambre.to_owned()).map_err(|_| {
            if ambre.starts_with('=') && ambre.len() > 1 {
                FormulaError::TooLong
            } else {
                FormulaError::NotAFormula
            }
        })?;
        let body = text.as_str().get(1..).unwrap_or_default();
        let excel = Validated::<ExcelFormula>::parse(ambre_to_excel(body))
            .map_err(|_| FormulaError::TooLong)?;
        Ok(Self {
            excel,
            ambre: Some(text),
            array: None,
        })
    }

    /// A formula kept in Excel's dialect alone: the text of a formula Trame
    /// could not run, written back as it came.
    ///
    /// # Errors
    ///
    /// [`FormulaError::TooLong`] past the cap, and
    /// [`FormulaError::NotAFormula`] for an empty text.
    pub fn verbatim(excel: &str, array: Option<CellRange>) -> Result<Self, FormulaError> {
        Ok(Self {
            excel: Validated::<ExcelFormula>::parse(excel.to_owned())
                .map_err(|_| too_long_or_empty(excel))?,
            ambre: None,
            array,
        })
    }

    /// The Excel text `excel`, kept as it came with no Trame text: a formula
    /// Trame held verbatim, written back byte for byte (`TRAME-IO-011`).
    #[must_use]
    pub const fn kept(excel: Validated<ExcelFormula>) -> Self {
        Self {
            excel,
            ambre: None,
            array: None,
        }
    }

    /// The text a worksheet part stores.
    #[must_use]
    pub fn excel(&self) -> &str {
        self.excel.as_str()
    }

    /// The text a Trame user reads, or `None` for a formula that stays
    /// Excel's.
    #[must_use]
    pub fn ambre(&self) -> Option<&str> {
        self.ambre.as_ref().map(Validated::as_str)
    }

    /// The Excel text, validated.
    #[must_use]
    pub const fn excel_text(&self) -> &Validated<ExcelFormula> {
        &self.excel
    }

    /// The Trame text, validated, or `None` for a formula that stays Excel's.
    #[must_use]
    pub const fn ambre_text(&self) -> Option<&Validated<AmbreFormula>> {
        self.ambre.as_ref()
    }

    /// The range a legacy array formula fills.
    #[must_use]
    pub const fn array(&self) -> Option<CellRange> {
        self.array
    }
}

fn too_long_or_empty(text: &str) -> FormulaError {
    if text.is_empty() {
        FormulaError::NotAFormula
    } else {
        FormulaError::TooLong
    }
}

/// The functions Excel stores behind the `_xlfn.` prefix: those added after
/// Excel 2007. An export adds the prefix to each one it meets.
const FUTURE_FUNCTIONS: &[&str] = &[
    "ACOT",
    "ACOTH",
    "AGGREGATE",
    "ARABIC",
    "ARRAYTOTEXT",
    "BASE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.DIST.RANGE",
    "BINOM.INV",
    "BITAND",
    "BITLSHIFT",
    "BITOR",
    "BITRSHIFT",
    "BITXOR",
    "BYCOL",
    "BYROW",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "COMBINA",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COT",
    "COTH",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "CSC",
    "CSCH",
    "DAYS",
    "DECIMAL",
    "DROP",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST",
    "F.DIST.RT",
    "F.INV",
    "F.INV.RT",
    "F.TEST",
    "FIELDVALUE",
    "FILTERXML",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.LINEAR",
    "FORMULATEXT",
    "GAMMA",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "GAUSS",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFNA",
    "IFS",
    "IMCOSH",
    "IMCOT",
    "IMCSC",
    "IMCSCH",
    "IMSEC",
    "IMSECH",
    "IMSINH",
    "IMTAN",
    "ISFORMULA",
    "ISOMITTED",
    "ISOWEEKNUM",
    "LAMBDA",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAKEARRAY",
    "MAP",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "MUNIT",
    "NEGBINOM.DIST",
    "NETWORKDAYS.INTL",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "NUMBERVALUE",
    "PDURATION",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "PERMUTATIONA",
    "PHI",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "REDUCE",
    "RRI",
    "SCAN",
    "SEC",
    "SECH",
    "SEQUENCE",
    "SHEET",
    "SHEETS",
    "SKEW.P",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "UNICHAR",
    "UNICODE",
    "UNIQUE",
    "VALUETOTEXT",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEIBULL.DIST",
    "WORKDAY.INTL",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "XOR",
    "Z.TEST",
];

/// The two functions Excel stores behind `_xlfn._xlws.`: they first shipped
/// in the web worksheet.
const WORKSHEET_FUNCTIONS: &[&str] = &["FILTER", "SORT"];

/// The error literals Trame reads in a formula, with Excel's spelling of
/// each.
const SHARED_ERRORS: &[(&str, &str)] = &[
    ("#REF!", "#REF!"),
    ("#DIV/0!", "#DIV/0!"),
    ("#NUM!", "#NUM!"),
    ("#N/A", "#N/A"),
    ("#VALUE!", "#TYPE!"),
];

/// The kind of one token of a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A string literal, quotes included.
    Str,
    /// A sheet prefix, its `!` included: `Sheet1!` or `'My Sheet'!`.
    Sheet,
    /// An error literal, such as `#REF!`.
    Error,
    Number,
    /// A run of name characters: a reference, a name, a function, a boolean.
    Word,
    /// A bracketed run: a structured reference or an external workbook.
    Bracket,
    /// An array constant, braces included.
    Brace,
    Open,
    Close,
    Comma,
    Space,
    /// Any other operator character.
    Op,
}

/// One token: its kind and the text it covers.
#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    kind: Kind,
    text: &'a str,
}

/// Whether `c` continues a word: a letter, a digit, or one of `_ . $ \ ?`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\' | '?')
}

/// The tokens of `text`, or `None` for a text no formula lexes: an open
/// string, an open bracket.
fn tokenize(text: &str) -> Option<Vec<Token<'_>>> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let (kind, len) = match c {
            '"' => (Kind::Str, quoted_len(rest, '"')?),
            '\'' => {
                let len = quoted_len(rest, '\'')?;
                if rest.get(len..)?.starts_with('!') {
                    (Kind::Sheet, len + 1)
                } else {
                    return None;
                }
            }
            '#' => error_len(rest).map_or((Kind::Op, 1), |len| (Kind::Error, len)),
            '[' => (Kind::Bracket, bracket_len(rest, '[', ']')?),
            '{' => (Kind::Brace, bracket_len(rest, '{', '}')?),
            '(' => (Kind::Open, 1),
            ')' => (Kind::Close, 1),
            ',' => (Kind::Comma, 1),
            c if c.is_whitespace() => (
                Kind::Space,
                rest.find(|ch: char| !ch.is_whitespace())
                    .unwrap_or(rest.len()),
            ),
            c if c.is_ascii_digit() || (c == '.' && next_is_digit(rest)) => {
                (Kind::Number, number_len(rest))
            }
            c if is_word_char(c) => {
                let len = rest
                    .find(|ch: char| !is_word_char(ch))
                    .unwrap_or(rest.len());
                if rest.get(len..)?.starts_with('!') {
                    (Kind::Sheet, len + 1)
                } else {
                    (Kind::Word, len)
                }
            }
            c => (Kind::Op, c.len_utf8()),
        };
        let (head, tail) = rest.split_at(len);
        tokens.push(Token { kind, text: head });
        rest = tail;
    }
    Some(tokens)
}

fn next_is_digit(text: &str) -> bool {
    text.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
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

/// The length of the balanced bracket run at the start of `text`.
fn bracket_len(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    for (at, c) in text.char_indices() {
        match c {
            '"' => in_string = !in_string,
            c if in_string || (c != open && c != close) => {}
            c if c == open => depth += 1,
            _ => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
        }
    }
    None
}

/// The length of the error literal at the start of `text`, if one is there.
fn error_len(text: &str) -> Option<usize> {
    crate::model::ErrorValue::ALL
        .iter()
        .map(|e| e.code())
        .chain(["#TYPE!", "#CIRCULAR!"])
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

/// The reference one word spells, if it spells one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefPart {
    Cell {
        col: Anchored<Col>,
        row: Anchored<Row>,
    },
    /// A column alone, one end of `A:C`.
    Col(Anchored<Col>),
    /// A row alone, one end of `2:5`.
    Row(Anchored<Row>),
}

/// A position and whether a `$` anchors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchored<A: ambre_core::index::Axis> {
    pos: Pos<A>,
    anchor: Anchor,
}

/// Whether a `$` anchors one part of a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Relative,
    Absolute,
}

impl Anchor {
    fn sign(self) -> &'static str {
        match self {
            Self::Relative => "",
            Self::Absolute => "$",
        }
    }
}

fn split_anchor(text: &str) -> (Anchor, &str) {
    match text.strip_prefix('$') {
        Some(rest) => (Anchor::Absolute, rest),
        None => (Anchor::Relative, text),
    }
}

fn parse_col(text: &str) -> Option<Anchored<Col>> {
    let (anchor, letters) = split_anchor(text);
    let pos = cellref::col_from_letters(letters)?;
    Some(Anchored { pos, anchor })
}

fn parse_row(text: &str) -> Option<Anchored<Row>> {
    let (anchor, digits) = split_anchor(text);
    let pos = cellref::row_from_number(digits)?;
    Some(Anchored { pos, anchor })
}

/// The cell reference `text` spells, such as `$B3`.
fn parse_cell(text: &str) -> Option<RefPart> {
    let (col_anchor, rest) = split_anchor(text);
    let split = rest.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, digits) = rest.split_at(split);
    let col = parse_col(letters)?;
    let row = parse_row(digits)?;
    Some(RefPart::Cell {
        col: Anchored {
            pos: col.pos,
            anchor: col_anchor,
        },
        row,
    })
}

impl RefPart {
    fn has_anchor(self) -> bool {
        match self {
            Self::Cell { col, row } => {
                col.anchor == Anchor::Absolute || row.anchor == Anchor::Absolute
            }
            Self::Col(col) => col.anchor == Anchor::Absolute,
            Self::Row(row) => row.anchor == Anchor::Absolute,
        }
    }

    /// The part moved by `rows` and `cols`, its anchored positions kept, or
    /// `None` when a moved position leaves the grid.
    fn shifted(self, rows: Delta<Row>, cols: Delta<Col>) -> Option<Self> {
        let col_of = |a: Anchored<Col>| -> Option<Anchored<Col>> {
            if a.anchor == Anchor::Absolute {
                return Some(a);
            }
            let pos = a.pos.checked_offset(cols)?;
            (pos <= LAST_COL).then_some(Anchored { pos, ..a })
        };
        let row_of = |a: Anchored<Row>| -> Option<Anchored<Row>> {
            if a.anchor == Anchor::Absolute {
                return Some(a);
            }
            let pos = a.pos.checked_offset(rows)?;
            (pos <= LAST_ROW).then_some(Anchored { pos, ..a })
        };
        Some(match self {
            Self::Cell { col, row } => Self::Cell {
                col: col_of(col)?,
                row: row_of(row)?,
            },
            Self::Col(col) => Self::Col(col_of(col)?),
            Self::Row(row) => Self::Row(row_of(row)?),
        })
    }

    /// The text of the part, its anchors written when `anchors` keeps them.
    fn write(self, out: &mut String, anchors: Anchors) {
        let sign = |a: Anchor| match anchors {
            Anchors::Kept => a.sign(),
            Anchors::Dropped => "",
        };
        let mut letters = String::new();
        match self {
            Self::Cell { col, row } => {
                let _ = cellref::write_col_letters(&mut letters, col.pos);
                let _ = write!(
                    out,
                    "{}{letters}{}{}",
                    sign(col.anchor),
                    sign(row.anchor),
                    u64::from(row.pos.get()) + 1
                );
            }
            Self::Col(col) => {
                let _ = cellref::write_col_letters(&mut letters, col.pos);
                let _ = write!(out, "{}{letters}", sign(col.anchor));
            }
            Self::Row(row) => {
                let _ = write!(out, "{}{}", sign(row.anchor), u64::from(row.pos.get()) + 1);
            }
        }
    }
}

/// Whether a written reference keeps its `$` anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchors {
    Dropped,
    Kept,
}

/// The reference parts of `tokens`, by token index. A word followed by `(`
/// is a function, never a reference. A column or a row alone is a reference
/// only at one end of a `:` range. The other end is then of the same kind.
fn reference_parts(tokens: &[Token<'_>]) -> Vec<Option<RefPart>> {
    let mut parts: Vec<Option<RefPart>> = tokens
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let is_call = tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Open);
            (t.kind == Kind::Word && !is_call)
                .then(|| parse_cell(t.text))
                .flatten()
        })
        .collect();
    for i in 1..tokens.len().saturating_sub(1) {
        let (Some(prev), Some(colon), Some(next)) =
            (tokens.get(i - 1), tokens.get(i), tokens.get(i + 1))
        else {
            continue;
        };
        if colon.kind != Kind::Op || colon.text != ":" {
            continue;
        }
        let line = |t: &Token<'_>| matches!(t.kind, Kind::Word | Kind::Number);
        if !line(prev) || !line(next) {
            continue;
        }
        if let (Some(a), Some(b)) = (parse_col(prev.text), parse_col(next.text)) {
            if let Some(slot) = parts.get_mut(i - 1) {
                *slot = Some(RefPart::Col(a));
            }
            if let Some(slot) = parts.get_mut(i + 1) {
                *slot = Some(RefPart::Col(b));
            }
        } else if let (Some(a), Some(b)) = (parse_row(prev.text), parse_row(next.text)) {
            if let Some(slot) = parts.get_mut(i - 1) {
                *slot = Some(RefPart::Row(a));
            }
            if let Some(slot) = parts.get_mut(i + 1) {
                *slot = Some(RefPart::Row(b));
            }
        }
    }
    parts
}

/// The text of the shared formula `master` as the cell `rows` below and
/// `cols` right of its master cell computes it (ECMA-376 part 1, 18.3.1.40).
/// A reference moved off the grid reads `#REF!`, as Excel reads it. A text
/// that does not lex is returned as it is.
#[must_use]
pub fn shift(master: &str, rows: Delta<Row>, cols: Delta<Col>) -> String {
    let Some(tokens) = tokenize(master) else {
        return master.to_owned();
    };
    let parts = reference_parts(&tokens);
    let mut out = String::with_capacity(master.len());
    for (token, part) in tokens.iter().zip(parts) {
        match part {
            Some(part) => match part.shifted(rows, cols) {
                Some(moved) => moved.write(&mut out, Anchors::Kept),
                None => out.push_str("#REF!"),
            },
            None => out.push_str(token.text),
        }
    }
    out
}

/// The Trame text of the Excel formula `excel`, or `None` when Trame's
/// dialect cannot spell it.
fn excel_to_ambre(excel: &str, report: &mut Report) -> Option<Validated<AmbreFormula>> {
    let tokens = tokenize(excel)?;
    let parts = reference_parts(&tokens);
    let mut out = String::with_capacity(excel.len() + 1);
    out.push('=');
    let mut anchored = false;
    // One entry per open parenthesis: whether it opens a function call.
    let mut calls: Vec<Call> = Vec::new();
    let significant = |i: usize, step: isize| -> Option<&Token<'_>> {
        let mut at = i.checked_add_signed(step)?;
        loop {
            let t = tokens.get(at)?;
            if t.kind != Kind::Space {
                return Some(t);
            }
            at = at.checked_add_signed(step)?;
        }
    };
    for (i, (token, part)) in tokens.iter().zip(parts.iter()).enumerate() {
        if let Some(part) = part {
            anchored |= part.has_anchor();
            part.write(&mut out, Anchors::Dropped);
            continue;
        }
        match token.kind {
            Kind::Bracket | Kind::Brace => return None,
            Kind::Error => {
                let (_, ambre) = SHARED_ERRORS
                    .iter()
                    .find(|(xl, _)| xl.eq_ignore_ascii_case(token.text))?;
                out.push_str(ambre);
            }
            Kind::Sheet => {
                // A 3D reference spans sheets: `Sheet1:Sheet3!A1`.
                let three_d = token.text.contains(':')
                    || significant(i, -1).is_some_and(|t| t.kind == Kind::Op && t.text == ":");
                if three_d {
                    return None;
                }
                out.push_str(token.text);
            }
            Kind::Word => {
                let is_call = tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Open);
                if is_call {
                    out.push_str(&strip_future_prefix(token.text)?);
                } else {
                    out.push_str(token.text);
                }
            }
            Kind::Open => {
                let after_word = i
                    .checked_sub(1)
                    .and_then(|p| tokens.get(p))
                    .is_some_and(|p| p.kind == Kind::Word);
                calls.push(if after_word {
                    Call::Function
                } else {
                    Call::Group
                });
                out.push('(');
            }
            Kind::Close => {
                calls.pop()?;
                out.push(')');
            }
            Kind::Comma => {
                // A comma outside a call's arguments is the union operator.
                if calls.last() != Some(&Call::Function) {
                    return None;
                }
                out.push(',');
            }
            Kind::Space => {
                // A space between two references is the intersection operator.
                let ends_ref = |t: Option<&Token<'_>>| {
                    t.is_some_and(|t| matches!(t.kind, Kind::Word | Kind::Close | Kind::Sheet))
                };
                let starts_ref = |t: Option<&Token<'_>>| {
                    t.is_some_and(|t| matches!(t.kind, Kind::Word | Kind::Open | Kind::Sheet))
                };
                if ends_ref(significant(i, -1)) && starts_ref(significant(i, 1)) {
                    return None;
                }
                out.push_str(token.text);
            }
            Kind::Op => {
                if matches!(token.text, "%" | "@" | "#" | ";") {
                    return None;
                }
                out.push_str(token.text);
            }
            Kind::Str | Kind::Number => out.push_str(token.text),
        }
    }
    if !calls.is_empty() {
        return None;
    }
    let formula = Validated::<AmbreFormula>::parse(out).ok()?;
    if anchored {
        report.record(XlsxConstruct::AbsoluteReference, Outcome::Approximated);
    }
    Some(formula)
}

/// What an open parenthesis opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Function,
    Group,
}

/// The function name `word` without its `_xlfn.` and `_xlws.` prefixes.
/// `None` names a function Trame cannot call by name: a user-defined one, or
/// the implicit intersection `_xlfn.SINGLE`.
fn strip_future_prefix(word: &str) -> Option<String> {
    let mut name = word;
    loop {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("_xludf.") {
            return None;
        }
        match ["_xlfn.", "_xlws."].iter().find(|p| lower.starts_with(*p)) {
            Some(prefix) => name = name.get(prefix.len()..).unwrap_or_default(),
            None => break,
        }
    }
    if name.eq_ignore_ascii_case("SINGLE") || name.eq_ignore_ascii_case("ANCHORARRAY") {
        return None;
    }
    Some(name.to_owned())
}

/// The Excel text of the Trame formula body `ambre`: each newer function
/// takes its prefix, and each error its Excel spelling. A text that does not
/// lex is returned as it is, for Excel to judge.
fn ambre_to_excel(ambre: &str) -> String {
    let Some(tokens) = tokenize(ambre) else {
        return ambre.to_owned();
    };
    let mut out = String::with_capacity(ambre.len() + 8);
    for (i, token) in tokens.iter().enumerate() {
        let is_call = tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Open);
        match token.kind {
            Kind::Word if is_call => {
                let upper = token.text.to_ascii_uppercase();
                if WORKSHEET_FUNCTIONS.contains(&upper.as_str()) {
                    out.push_str("_xlfn._xlws.");
                } else if FUTURE_FUNCTIONS.contains(&upper.as_str()) {
                    out.push_str("_xlfn.");
                }
                out.push_str(token.text);
            }
            Kind::Error => {
                let excel = SHARED_ERRORS
                    .iter()
                    .find(|(_, am)| am.eq_ignore_ascii_case(token.text))
                    .map_or("#REF!", |(xl, _)| *xl);
                out.push_str(excel);
            }
            _ => out.push_str(token.text),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ambre(excel: &str) -> Option<String> {
        excel_to_ambre(excel, &mut Report::default()).map(Validated::into_string)
    }

    #[test]
    fn a_shared_formula_shifts_its_relative_references_alone() {
        assert_eq!(shift("A1*2", Delta::new(1), Delta::new(0)), "A2*2");
        assert_eq!(shift("B$1", Delta::new(2), Delta::new(3)), "E$1");
        assert_eq!(
            shift("$A1+SUM(A1:B2)", Delta::new(1), Delta::new(1)),
            "$A2+SUM(B2:C3)"
        );
        assert_eq!(
            shift("LOG10(ABS(A1)+1)", Delta::new(4), Delta::new(0)),
            "LOG10(ABS(A5)+1)"
        );
        assert_eq!(
            shift("SUM(A:A)+SUM(1:1)", Delta::new(2), Delta::new(1)),
            "SUM(B:B)+SUM(3:3)"
        );
        assert_eq!(
            shift("'My Sheet'!A1&\"A1\"", Delta::new(1), Delta::new(0)),
            "'My Sheet'!A2&\"A1\""
        );
        assert_eq!(shift("A1", Delta::new(-1), Delta::new(0)), "#REF!");
    }

    #[test]
    fn the_dialect_differences_translate_on_import() {
        assert_eq!(ambre("SUM(A1:A3)*2").as_deref(), Some("=SUM(A1:A3)*2"));
        assert_eq!(ambre("$A$1+B$2").as_deref(), Some("=A1+B2"));
        assert_eq!(
            ambre("_xlfn.IFS(A1>0,1,TRUE,0)").as_deref(),
            Some("=IFS(A1>0,1,TRUE,0)")
        );
        assert_eq!(
            ambre("_xlfn._xlws.SORT(A1:A3)").as_deref(),
            Some("=SORT(A1:A3)")
        );
        assert_eq!(
            ambre("IF(ISERROR(A1),#VALUE!,1)").as_deref(),
            Some("=IF(ISERROR(A1),#TYPE!,1)")
        );
        assert_eq!(
            ambre("Sheet2!A1+'My Sheet'!B2").as_deref(),
            Some("=Sheet2!A1+'My Sheet'!B2")
        );
        assert_eq!(ambre("SUM( A1 , 2 )").as_deref(), Some("=SUM( A1 , 2 )"));
    }

    #[test]
    fn a_formula_trame_cannot_spell_stays_excels() {
        for excel in [
            "SUM({1,2,3})",
            "Table1[Amount]",
            "[1]Sheet1!A1",
            "SUM(Sheet1:Sheet3!A1)",
            "SUM(A1:B2 B1:C3)",
            "SUM((A1,B1))",
            "A1%",
            "_xludf.MYFN(1)",
            "_xlfn.SINGLE(A1:A3)",
            "IF(A1,#NULL!,1)",
            "A1#",
        ] {
            assert_eq!(ambre(excel), None, "{excel}");
        }
    }

    #[test]
    fn an_anchor_is_reported_once_per_formula() {
        let mut report = Report::default();
        let formula = Formula::from_excel("$A$1+$B$2", None, &mut report);
        assert!(formula.is_ok());
        assert_eq!(
            report
                .count(XlsxConstruct::AbsoluteReference, Outcome::Approximated)
                .get(),
            1
        );
    }

    #[test]
    fn a_newer_function_takes_its_prefix_on_export() {
        let Ok(formula) = Formula::from_ambre("=IFS(A1>0,XOR(TRUE,FALSE),1)+SUM(A1:A2)") else {
            panic!("the formula reads");
        };
        assert_eq!(
            formula.excel(),
            "_xlfn.IFS(A1>0,_xlfn.XOR(TRUE,FALSE),1)+SUM(A1:A2)"
        );
        assert_eq!(
            Formula::from_ambre("=1/0+#TYPE!").map(|f| f.excel().to_owned()),
            Ok("1/0+#VALUE!".to_owned())
        );
        assert_eq!(
            Formula::from_ambre("SUM(A1)"),
            Err(FormulaError::NotAFormula)
        );
    }

    #[test]
    fn an_array_formula_stays_excels_with_its_range() {
        let range = CellRange::parse("A1:A3");
        let Ok(formula) = Formula::from_excel("B1:B3*2", range, &mut Report::default()) else {
            panic!("the formula reads");
        };
        assert_eq!((formula.ambre(), formula.array()), (None, range));
    }

    /// A small Trame formula: references, whole lines, calls, strings,
    /// errors and operators, as a user types them.
    fn trame_formula() -> impl Strategy<Value = String> {
        let leaf = prop_oneof![
            (0u32..30, 0u32..60).prop_map(|(c, r)| {
                let mut s = String::new();
                let _ = cellref::write_col_letters(&mut s, Pos::new(c));
                format!("{s}{}", r + 1)
            }),
            (0u32..1000).prop_map(|n| n.to_string()),
            "[a-z ]{0,6}".prop_map(|s| format!("\"{s}\"")),
            Just("#REF!".to_owned()),
            Just("#TYPE!".to_owned()),
            Just("TRUE".to_owned()),
            Just("A:C".to_owned()),
            Just("2:5".to_owned()),
            Just("'My Sheet'!B2".to_owned()),
        ];
        leaf.prop_recursive(4, 24, 3, |inner| {
            prop_oneof![
                (
                    inner.clone(),
                    prop::sample::select(vec!["+", "-", "*", "/", "&", "<>", "^"]),
                    inner.clone()
                )
                    .prop_map(|(a, op, b)| format!("{a}{op}{b}")),
                (
                    prop::sample::select(vec!["SUM", "IFS", "XOR", "CONCAT", "IF", "TEXTJOIN"]),
                    prop::collection::vec(inner, 1..3)
                )
                    .prop_map(|(f, args)| format!("{f}({})", args.join(","))),
            ]
        })
        .prop_map(|body| format!("={body}"))
    }

    proptest! {
        #[test]
        fn a_trame_formula_survives_the_round_trip_through_excel(text in trame_formula()) {
            let Ok(formula) = Formula::from_ambre(&text) else {
                return Err(TestCaseError::fail("a generated formula reads"));
            };
            let mut report = Report::default();
            let Ok(back) = Formula::from_excel(formula.excel(), None, &mut report) else {
                return Err(TestCaseError::fail("the Excel text reads back"));
            };
            prop_assert_eq!(back.ambre(), Some(text.as_str()));
        }

        #[test]
        fn a_shift_there_and_back_restores_the_formula(text in trame_formula(), rows in 0i64..50, cols in 0i64..20) {
            let body = text.get(1..).unwrap_or_default();
            let there = shift(body, Delta::new(rows), Delta::new(cols));
            prop_assert_eq!(shift(&there, Delta::new(-rows), Delta::new(-cols)), body);
        }

        #[test]
        fn any_text_lexes_or_is_refused_without_a_panic(text in "\\PC{0,40}") {
            let _ = excel_to_ambre(&text, &mut Report::default());
            let _ = shift(&text, Delta::new(3), Delta::new(-2));
            let _ = ambre_to_excel(&text);
        }
    }
}
