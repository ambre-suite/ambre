//! nom-based formula parser (Excel-compatible syntax).
//!
//! Produces an [`Expr`] AST from a raw formula string (without the leading `=`).
//! Uses nom 8.0 with the `Parser::parse` trait method.

use nom::Parser;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::{alpha1, alphanumeric1, char, digit1, multispace0, one_of};
use nom::combinator::{map, map_res, opt, recognize, value};
use nom::multi::{many0, many1, separated_list0};
use nom::sequence::{delimited, preceded};

use super::types::{
    BinOpKind, CellRef, CellValue, Expr, FormulaCtx, FormulaError, NameTarget, RangeDep,
    SheetQualifier, StoredAxis, StoredCell, UnaryOpKind, alpha_to_col,
};
use crate::model::Uid;

// -----------------------------------------------------------------------------
// Whitespace
// -----------------------------------------------------------------------------

fn ws<'a, P, O>(
    mut inner: P,
) -> impl Parser<&'a str, Output = O, Error = nom::error::Error<&'a str>>
where
    P: Parser<&'a str, Output = O, Error = nom::error::Error<&'a str>>,
{
    move |input: &'a str| {
        let (input, _) = multispace0.parse(input)?;
        let (input, val) = inner.parse(input)?;
        let (input, _) = multispace0.parse(input)?;
        Ok((input, val))
    }
}

// -----------------------------------------------------------------------------
// Literals
// -----------------------------------------------------------------------------

fn number(input: &str) -> nom::IResult<&str, f64> {
    map_res(
        recognize((opt(char('-')), digit1, opt((char('.'), digit1)))),
        |s: &str| s.parse::<f64>(),
    )
    .parse(input)
}

fn string_literal(input: &str) -> nom::IResult<&str, String> {
    let (input, _) = char('"').parse(input)?;
    let mut s = String::new();
    let mut chars = input.chars();
    let mut pos = 0;
    loop {
        match chars.next() {
            Some('"') => {
                pos += 1;
                if chars.clone().next() == Some('"') {
                    chars.next();
                    s.push('"');
                    pos += 1;
                } else {
                    break;
                }
            }
            Some(c) => {
                s.push(c);
                pos += c.len_utf8();
            }
            None => {
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Tag,
                )));
            }
        }
    }
    Ok((&input[pos..], s))
}

/// `TRUE` / `FALSE`, matched as a whole word only. A bareword that merely starts
/// with one must reach the name and sheet parsers intact. A named range
/// `TRUEVALUE` or a sheet `FALSEY` must not leave its tail as trailing input.
fn parse_bool(input: &str) -> nom::IResult<&str, bool> {
    let (rest, b) = alt((value(true, tag("TRUE")), value(false, tag("FALSE")))).parse(input)?;
    if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    Ok((rest, b))
}

// -----------------------------------------------------------------------------
// Cell reference & range
// -----------------------------------------------------------------------------

fn col_letters(input: &str) -> nom::IResult<&str, &str> {
    recognize(many1(one_of("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))).parse(input)
}

/// The 1-based row number of an A1 reference, as a 0-based index. Row 0 does not
/// exist: `A0` is rejected here so it falls through to the named-range parser,
/// where it surfaces as an undefined name. Saturating it to row 1 would silently
/// rewrite the user's `=A0` into `=A1` on the canonicalization round-trip.
fn row_number(input: &str) -> nom::IResult<&str, u32> {
    map_res(digit1, |s: &str| -> Result<u32, ()> {
        let n: u32 = s.parse().map_err(|_| ())?;
        n.checked_sub(1).ok_or(())
    })
    .parse(input)
}

fn cell_ref(input: &str) -> nom::IResult<&str, CellRef> {
    let (input, col_str) = col_letters.parse(input)?;
    let (input, row) = row_number.parse(input)?;
    let col = alpha_to_col(col_str).map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
    })?;
    Ok((input, CellRef::new(row, col)))
}

/// A line identity as the stored text writes it: lowercase hex, `_`, lowercase
/// hex (`3fa1_2`). Lowercase keeps it apart from column letters.
fn line_uid(input: &str) -> nom::IResult<&str, Uid> {
    let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    map_res(
        recognize((take_while1(hex), char('_'), take_while1(hex))),
        |s: &str| Uid::parse(s).ok_or(()),
    )
    .parse(input)
}

/// A cell in stored form, `{row|col}` (`TRAME-228`). Each axis is a line
/// identity, or the position it held when no line stood there: a 1-based row
/// number, column letters.
fn stored_cell(input: &str) -> nom::IResult<&str, StoredCell> {
    let row = alt((
        map(line_uid, StoredAxis::Line),
        map(row_number, StoredAxis::At),
    ));
    let col = alt((
        map(line_uid, StoredAxis::Line),
        map_res(col_letters, |s: &str| alpha_to_col(s).map(StoredAxis::At)),
    ));
    let (input, (_, row, _, col, _)) = (char('{'), row, char('|'), col, char('}')).parse(input)?;
    Ok((input, StoredCell { row, col }))
}

/// One corner of a reference: the stored form, or a typed `A1`.
fn endpoint(input: &str) -> nom::IResult<&str, StoredCell> {
    alt((stored_cell, map(cell_ref, StoredCell::at))).parse(input)
}

/// Parse a range like `A1:B10`. Two positional corners make an `Expr::Range`,
/// normalized so its start is above and left of its end. A corner named by a
/// line makes an `Expr::StoredRange`, ordered once its lines have positions.
fn range(input: &str) -> nom::IResult<&str, Expr> {
    let (input, start) = endpoint.parse(input)?;
    let (input, _colon) = char(':').parse(input)?;
    let (input, end) = endpoint.parse(input)?;
    let (Some(start_at), Some(end_at)) = (start.as_position(), end.as_position()) else {
        return Ok((input, Expr::StoredRange(start, end)));
    };
    let dep = RangeDep::new(
        start_at.row.min(end_at.row),
        start_at.row.max(end_at.row),
        start_at.col.min(end_at.col),
        start_at.col.max(end_at.col),
    );
    Ok((input, Expr::Range(dep)))
}

/// Try to parse a cell reference or a range.
fn cell_or_range(input: &str) -> nom::IResult<&str, Expr> {
    if let Ok(result) = range(input) {
        return Ok(result);
    }
    map(endpoint, |cell| match cell.as_position() {
        Some(at) => Expr::CellRef(at),
        None => Expr::StoredCell(cell),
    })
    .parse(input)
}

/// A sheet name preceding a `!`: a bareword identifier (`Sheet2`) or a
/// single-quoted name (`'My Sheet'`, allowing spaces). The closing `!` is not
/// consumed here.
fn sheet_name(input: &str) -> nom::IResult<&str, String> {
    if let Ok((rest, _)) = char::<&str, nom::error::Error<&str>>('\'').parse(input) {
        // Quoted: read until the closing quote (no escaping in the MVP).
        let end = rest.find('\'').ok_or_else(|| {
            nom::Err::Error(nom::error::Error::new(rest, nom::error::ErrorKind::Tag))
        })?;
        let name = rest[..end].to_string();
        return Ok((&rest[end + 1..], name));
    }
    map(identifier, |s: &str| s.to_string()).parse(input)
}

/// A stable sheet id token in canonical form: `#` followed by id characters
/// (hex digits and `_`, for example `#s0` or `#00..0_0000002a`). The id may start with
/// a digit, so a plain `identifier` parser does not fit.
fn sheet_id_token(input: &str) -> nom::IResult<&str, String> {
    let (input, _) = char('#').parse(input)?;
    let (input, id) = recognize(many1(alt((alphanumeric1, tag("_"))))).parse(input)?;
    Ok((input, id.to_string()))
}

/// The qualifier preceding a `!`: a stable id (`#s0`) or a display name
/// (`Sheet2`, `'My Sheet'`).
fn sheet_qualifier(input: &str) -> nom::IResult<&str, SheetQualifier> {
    if let Ok((rest, id)) = sheet_id_token(input) {
        return Ok((rest, SheetQualifier::Id(id)));
    }
    map(sheet_name, SheetQualifier::Name).parse(input)
}

/// A sheet-qualified reference `Sheet2!A1` / `'My Sheet'!A1:B2` / `#s0!A1`.
/// Resolution of the qualifier to an index happens later in [`resolve_refs`].
fn sheet_qualified(input: &str) -> nom::IResult<&str, Expr> {
    let (input, sheet) = sheet_qualifier.parse(input)?;
    let (input, _) = char('!').parse(input)?;
    let (input, inner) = cell_or_range.parse(input)?;
    Ok((
        input,
        Expr::SheetRef {
            sheet,
            inner: Box::new(inner),
        },
    ))
}

/// An error literal (`#REF!`, `#DIV/0!`, ...). The renderer emits these whenever a
/// rewrite invalidates a reference. That covers a paste that drives one off the
/// grid (`shift_formula`), or a sheet qualifier that no longer resolves. That text
/// is written back to the CRDT and re-parsed. The grammar therefore has to read
/// back everything the renderer writes, otherwise `=#REF!` settles on `#PARSE!`.
///
/// `#PARSE!(msg)` is deliberately absent: evaluation produces it, no AST holds it.
fn error_literal(input: &str) -> nom::IResult<&str, FormulaError> {
    preceded(
        char('#'),
        alt((
            value(FormulaError::BrokenRef, tag("REF!")),
            value(FormulaError::DivisionByZero, tag("DIV/0!")),
            value(FormulaError::CircularReference, tag("CIRCULAR!")),
            value(FormulaError::TypeError, tag("TYPE!")),
            value(FormulaError::NumError, tag("NUM!")),
            value(FormulaError::NaError, tag("N/A")),
        )),
    )
    .parse(input)
}

/// A bareword reference to a named range. Resolution happens in
/// [`resolve_refs`]; here we only capture the identifier.
fn name_ref(input: &str) -> nom::IResult<&str, Expr> {
    map(identifier, |s: &str| Expr::Name(s.to_string())).parse(input)
}

// -----------------------------------------------------------------------------
// Function call
// -----------------------------------------------------------------------------

fn identifier(input: &str) -> nom::IResult<&str, &str> {
    recognize((
        alt((alpha1, tag("_"))),
        many0(alt((alphanumeric1, tag("_")))),
    ))
    .parse(input)
}

fn function_call(input: &str) -> nom::IResult<&str, Expr> {
    let (input, name) = identifier.parse(input)?;
    let (input, _) = char('(').parse(input)?;
    let (input, args) = separated_list0(ws(char(',')), expr).parse(input)?;
    let (input, _) = ws(char(')')).parse(input)?;
    Ok((
        input,
        Expr::FnCall {
            name: name.to_uppercase(),
            args,
        },
    ))
}

// -----------------------------------------------------------------------------
// Expression grammar (precedence climbing)
// -----------------------------------------------------------------------------

/// Parse any atomic expression (literal, cell ref, range, function call, or
/// parenthesized sub-expression).
fn atom_inner(input: &str) -> nom::IResult<&str, Expr> {
    // Try each atomic parser in priority order.
    if let Ok(result) = map(number, |n| Expr::Literal(CellValue::Number(n))).parse(input) {
        return Ok(result);
    }
    if let Ok(result) = map(string_literal, |s| Expr::Literal(CellValue::Text(s))).parse(input) {
        return Ok(result);
    }
    if let Ok(result) = map(parse_bool, |b| Expr::Literal(CellValue::Boolean(b))).parse(input) {
        return Ok(result);
    }
    if let Ok(result) = function_call(input) {
        return Ok(result);
    }
    // Sheet-qualified (`Sheet2!A1`) before a plain ref: it starts like an
    // identifier but is disambiguated by the `!`.
    if let Ok(result) = sheet_qualified(input) {
        return Ok(result);
    }
    // After `sheet_qualified`: both open on `#`, and a stable sheet id followed
    // by a reference (`#s0!A1`) must not be read as an error literal.
    if let Ok(result) = map(error_literal, |e| Expr::Literal(CellValue::Error(e))).parse(input) {
        return Ok(result);
    }
    if let Ok(result) = cell_or_range(input) {
        return Ok(result);
    }
    // A bareword that is neither a function call, a cell ref, nor a range is a
    // named-range reference, resolved later against the context.
    if let Ok(result) = name_ref(input) {
        return Ok(result);
    }
    delimited(char('('), expr, char(')')).parse(input)
}

fn atom(input: &str) -> nom::IResult<&str, Expr> {
    ws(atom_inner).parse(input)
}

/// Unary minus.
fn unary(input: &str) -> nom::IResult<&str, Expr> {
    let (input, ops) = many0(preceded(multispace0, char('-'))).parse(input)?;
    let (input, operand) = atom.parse(input)?;
    let mut result = operand;
    for _ in ops.into_iter().rev() {
        result = Expr::UnaryOp {
            kind: UnaryOpKind::Neg,
            operand: Box::new(result),
        };
    }
    Ok((input, result))
}

/// `^` operator, right-associative.
fn pow_expr(input: &str) -> nom::IResult<&str, Expr> {
    let (input, lhs) = unary.parse(input)?;
    let (input, rest) = opt(ws(char('^'))).parse(input)?;
    match rest {
        Some(_) => {
            let (input, rhs) = pow_expr(input)?;
            Ok((
                input,
                Expr::BinOp {
                    kind: BinOpKind::Pow,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            ))
        }
        None => Ok((input, lhs)),
    }
}

/// `*` `/` operators.
fn mul_div(input: &str) -> nom::IResult<&str, Expr> {
    let (mut input, mut lhs) = pow_expr.parse(input)?;
    loop {
        let (rest, op) = opt(ws(alt((
            value(BinOpKind::Mul, char('*')),
            value(BinOpKind::Div, char('/')),
        ))))
        .parse(input)?;
        match op {
            Some(kind) => {
                let (rest, rhs) = pow_expr.parse(rest)?;
                lhs = Expr::BinOp {
                    kind,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                input = rest;
            }
            None => break,
        }
    }
    Ok((input, lhs))
}

/// `+` `-` operators.
fn add_sub(input: &str) -> nom::IResult<&str, Expr> {
    let (mut input, mut lhs) = mul_div.parse(input)?;
    loop {
        let (rest, op) = opt(ws(alt((
            value(BinOpKind::Add, char('+')),
            value(BinOpKind::Sub, char('-')),
        ))))
        .parse(input)?;
        match op {
            Some(kind) => {
                let (rest, rhs) = mul_div.parse(rest)?;
                lhs = Expr::BinOp {
                    kind,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                input = rest;
            }
            None => break,
        }
    }
    Ok((input, lhs))
}

/// `&` concatenation operator.
fn concat_expr(input: &str) -> nom::IResult<&str, Expr> {
    let (mut input, mut lhs) = add_sub.parse(input)?;
    loop {
        let (rest, found) = opt(ws(char('&'))).parse(input)?;
        match found {
            Some(_) => {
                let (rest, rhs) = add_sub.parse(rest)?;
                lhs = Expr::BinOp {
                    kind: BinOpKind::Concat,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                };
                input = rest;
            }
            None => break,
        }
    }
    Ok((input, lhs))
}

/// Comparison operators (`=`, `<>`, `<=`, `>=`, `<`, `>`).
fn comparison(input: &str) -> nom::IResult<&str, Expr> {
    let (input, lhs) = concat_expr.parse(input)?;
    let (input, op) = opt(ws(alt((
        value(BinOpKind::Eq, tag("=")),
        value(BinOpKind::Ne, tag("<>")),
        value(BinOpKind::Le, tag("<=")),
        value(BinOpKind::Ge, tag(">=")),
        value(BinOpKind::Lt, tag("<")),
        value(BinOpKind::Gt, tag(">")),
    ))))
    .parse(input)?;
    match op {
        Some(kind) => {
            let (input, rhs) = concat_expr.parse(input)?;
            Ok((
                input,
                Expr::BinOp {
                    kind,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            ))
        }
        None => Ok((input, lhs)),
    }
}

/// Top-level expression.
fn expr(input: &str) -> nom::IResult<&str, Expr> {
    comparison.parse(input)
}

// -----------------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------------

/// The longest formula the parser reads, in characters: Excel's own limit.
pub const MAX_FORMULA_CHARS: usize = 8_192;

/// The deepest parenthesis or call nesting it reads: Excel's own limit.
pub const MAX_FORMULA_NESTING: usize = 64;

/// The most operators it reads. Each one can add a level to the tree, and the
/// parser, the compiler and the drop all walk the tree by recursion. A release
/// build overflows a 1 MiB stack past about 256 nested levels and 2 048
/// operators, so both bounds keep a margin of four.
pub const MAX_FORMULA_OPERATORS: usize = 512;

/// Refuse, before any recursion, a formula that could nest deeper than the
/// stack holds (`TRAME-013`). One linear scan; text between double quotes is
/// skipped, and a quoted sheet name is counted, which can only over-estimate.
///
/// A stored reference between braces counts as no character: it renders as a
/// few-character `A1`, and the limit is on what the user typed. Nothing between
/// braces nests or adds an operator.
fn within_bounds(input: &str) -> Result<(), String> {
    let (mut depth, mut deepest, mut operators) = (0_usize, 0_usize, 0_usize);
    let (mut quoted, mut braced, mut chars) = (false, false, 0_usize);
    for c in input.chars() {
        if !braced {
            chars += 1;
        }
        match c {
            '"' if !braced => quoted = !quoted,
            _ if quoted => {}
            '{' => braced = true,
            '}' => braced = false,
            _ if braced => {}
            '(' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            ')' => depth = depth.saturating_sub(1),
            '+' | '-' | '*' | '/' | '^' | '&' | '=' | '<' | '>' => operators += 1,
            _ => {}
        }
    }
    if chars > MAX_FORMULA_CHARS {
        return Err(format!(
            "formula longer than {MAX_FORMULA_CHARS} characters"
        ));
    }
    if deepest > MAX_FORMULA_NESTING {
        return Err(format!(
            "formula nested deeper than {MAX_FORMULA_NESTING} levels"
        ));
    }
    if operators > MAX_FORMULA_OPERATORS {
        return Err(format!(
            "formula with more than {MAX_FORMULA_OPERATORS} operators"
        ));
    }
    Ok(())
}

/// Parse a formula string (without the leading `=`) into an [`Expr`] AST.
///
/// # Errors
///
/// A message for an empty formula, and one for trailing input the grammar does
/// not take. A parse failure names the text it stopped near.
pub fn parse_formula(input: &str) -> Result<Expr, String> {
    if input.trim().is_empty() {
        return Err("empty formula".into());
    }
    within_bounds(input)?;
    match expr.parse(input.trim()) {
        Ok(("", ast)) => Ok(ast),
        Ok((rest, _)) => Err(format!("unexpected trailing input: '{rest}'")),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            // Point at the unparsed remainder. Slice by `chars()` so a
            // multi-byte codepoint at the failure site cannot split a UTF-8
            // boundary (a byte-range slice here panics on non-ASCII input).
            let near: String = e.input.chars().take(12).collect();
            Err(format!("parse error near '{near}'"))
        }
        Err(nom::Err::Incomplete(_)) => Err("incomplete input".into()),
    }
}

// -----------------------------------------------------------------------------
// Reference resolution (context-dependent pass over the AST)
// -----------------------------------------------------------------------------

/// Rewrite the context-free AST into a fully resolved one: stamp `default_sheet`
/// onto every unqualified reference, resolve `Sheet2!ref` to a sheet index, and
/// expand named ranges. After this pass the AST contains no [`Expr::SheetRef`]
/// or [`Expr::Name`] nodes, so the compiler only deals with concrete refs.
///
/// `default_sheet` is the sheet the formula itself lives on (its unqualified
/// references resolve there). Returns an error string for an unknown sheet
/// name.
///
/// # Errors
///
/// A message for a sheet display name that resolves to nothing. A deleted sheet
/// is not an error here: it evaluates to `#REF!`, and an undefined name to
/// `#NAME?`.
pub fn resolve_refs(expr: Expr, default_sheet: u16, ctx: &dyn FormulaCtx) -> Result<Expr, String> {
    match expr {
        Expr::CellRef(mut r) => {
            r.sheet = default_sheet;
            Ok(Expr::CellRef(r))
        }
        Expr::Range(mut r) => {
            r.sheet = default_sheet;
            Ok(Expr::Range(r))
        }
        Expr::SheetRef { sheet, inner } => {
            // A display name that does not resolve is a hard parse error (user
            // typo). A canonical id that does not resolve means the sheet was
            // deleted: the reference evaluates to `#REF!`, mirroring a deleted
            // row/column (`trame-sheets`, `trame-formula-engine`).
            let resolved = match &sheet {
                SheetQualifier::Name(name) => Some(
                    ctx.resolve_sheet(name)
                        .ok_or_else(|| format!("unknown sheet: {name}"))?,
                ),
                SheetQualifier::Id(id) => ctx.resolve_sheet_id(id),
            };
            let Some(sheet_idx) = resolved else {
                return Ok(Expr::Literal(CellValue::Error(FormulaError::BrokenRef)));
            };
            // `inner` is always a CellRef or Range straight from the parser.
            match *inner {
                Expr::CellRef(mut r) => {
                    r.sheet = sheet_idx;
                    Ok(Expr::CellRef(r))
                }
                Expr::Range(mut r) => {
                    r.sheet = sheet_idx;
                    Ok(Expr::Range(r))
                }
                other => resolve_refs(other, sheet_idx, ctx),
            }
        }
        Expr::Name(name) => match ctx.resolve_name(&name) {
            Some(NameTarget::Cell(r)) => Ok(Expr::CellRef(r)),
            Some(NameTarget::Range(r)) => Ok(Expr::Range(r)),
            // An undefined name is a value, not a failure to parse. The cell
            // reads `#NAME?` until the name is defined; a reseed then
            // recompiles it (`TRAME-065`).
            None => Ok(Expr::Literal(CellValue::Error(FormulaError::NameError))),
        },
        Expr::BinOp { kind, lhs, rhs } => Ok(Expr::BinOp {
            kind,
            lhs: Box::new(resolve_refs(*lhs, default_sheet, ctx)?),
            rhs: Box::new(resolve_refs(*rhs, default_sheet, ctx)?),
        }),
        Expr::UnaryOp { kind, operand } => Ok(Expr::UnaryOp {
            kind,
            operand: Box::new(resolve_refs(*operand, default_sheet, ctx)?),
        }),
        Expr::FnCall { name, args } => {
            let args = args
                .into_iter()
                .map(|a| resolve_refs(a, default_sheet, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::FnCall { name, args })
        }
        lit @ Expr::Literal(_) => Ok(lit),
        // `unbind` gives every stored reference a position before resolution.
        // One left here names a sheet the host has no index for.
        Expr::StoredCell(_) | Expr::StoredRange(..) => {
            Ok(Expr::Literal(CellValue::Error(FormulaError::BrokenRef)))
        }
    }
}

#[cfg(test)]
mod tests;
