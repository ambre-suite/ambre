//! nom-based formula parser (Excel-compatible syntax).
//!
//! Produces an [`Expr`] AST from a raw formula string (without the leading `=`).
//! Uses nom 8.0 with the `Parser::parse` trait method.

use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::{alpha1, alphanumeric1, char, digit1, multispace0, one_of};
use nom::combinator::{map, map_res, opt, recognize, value};
use nom::multi::{many0, many1, separated_list0};
use nom::sequence::{delimited, preceded};
use nom::Parser;

use super::types::{
    alpha_to_col, BinOpKind, CellRef, CellValue, Expr, FormulaCtx, NameTarget, RangeDep,
    UnaryOpKind,
};

// -----------------------------------------------------------------------------
// Whitespace
// -----------------------------------------------------------------------------

fn ws<'a, P, O>(mut inner: P) -> impl Parser<&'a str, Output = O, Error = nom::error::Error<&'a str>>
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
        recognize((
            opt(char('-')),
            digit1,
            opt((char('.'), digit1)),
        )),
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

fn parse_bool(input: &str) -> nom::IResult<&str, bool> {
    alt((value(true, tag("TRUE")), value(false, tag("FALSE")))).parse(input)
}

// -----------------------------------------------------------------------------
// Cell reference & range
// -----------------------------------------------------------------------------

fn col_letters(input: &str) -> nom::IResult<&str, &str> {
    recognize(many1(one_of("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))).parse(input)
}

fn row_number(input: &str) -> nom::IResult<&str, u32> {
    map_res(digit1, |s: &str| -> Result<u32, ()> {
        let n: u32 = s.parse().map_err(|_| ())?;
        Ok(n.saturating_sub(1))
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

/// Parse a range like `A1:B10`. Returns `Expr::Range`.
fn range(input: &str) -> nom::IResult<&str, Expr> {
    let (input, start) = cell_ref.parse(input)?;
    let (input, _colon) = char(':').parse(input)?;
    let (input, end) = cell_ref.parse(input)?;
    let dep = RangeDep::new(
        start.row.min(end.row),
        start.row.max(end.row),
        start.col.min(end.col),
        start.col.max(end.col),
    );
    Ok((input, Expr::Range(dep)))
}

/// Try to parse a cell reference or a range.
fn cell_or_range(input: &str) -> nom::IResult<&str, Expr> {
    if let Ok(result) = range(input) {
        return Ok(result);
    }
    map(cell_ref, Expr::CellRef).parse(input)
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

/// A sheet-qualified reference `Sheet2!A1` / `'My Sheet'!A1:B2`. Resolution of
/// the sheet name to an index happens later in [`resolve_refs`].
fn sheet_qualified(input: &str) -> nom::IResult<&str, Expr> {
    let (input, name) = sheet_name.parse(input)?;
    let (input, _) = char('!').parse(input)?;
    let (input, inner) = cell_or_range.parse(input)?;
    Ok((input, Expr::SheetRef { sheet_name: name, inner: Box::new(inner) }))
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
    Ok((input, Expr::FnCall {
        name: name.to_uppercase(),
        args,
    }))
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
        result = Expr::UnaryOp { kind: UnaryOpKind::Neg, operand: Box::new(result) };
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
            Ok((input, Expr::BinOp {
                kind: BinOpKind::Pow,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }))
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
                lhs = Expr::BinOp { kind, lhs: Box::new(lhs), rhs: Box::new(rhs) };
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
                lhs = Expr::BinOp { kind, lhs: Box::new(lhs), rhs: Box::new(rhs) };
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
            Ok((input, Expr::BinOp {
                kind,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }))
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

/// Parse a formula string (without the leading `=`) into an [`Expr`] AST.
pub fn parse_formula(input: &str) -> Result<Expr, String> {
    if input.trim().is_empty() {
        return Err("empty formula".into());
    }
    match expr.parse(input.trim()) {
        Ok(("", ast)) => Ok(ast),
        Ok((rest, _)) => Err(format!("unexpected trailing input: '{rest}'")),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
            let pos = e.input.len();
            let ctx = &input.trim()[pos.saturating_sub(10).min(pos)..pos];
            Err(format!("parse error at '{ctx}'"))
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
/// references resolve there). Returns an error string for an unknown sheet name
/// or undefined name.
pub fn resolve_refs(
    expr: Expr,
    default_sheet: u16,
    ctx: &dyn FormulaCtx,
) -> Result<Expr, String> {
    match expr {
        Expr::CellRef(mut r) => {
            r.sheet = default_sheet;
            Ok(Expr::CellRef(r))
        }
        Expr::Range(mut r) => {
            r.sheet = default_sheet;
            Ok(Expr::Range(r))
        }
        Expr::SheetRef { sheet_name, inner } => {
            let sheet = ctx
                .resolve_sheet(&sheet_name)
                .ok_or_else(|| format!("unknown sheet: {sheet_name}"))?;
            // `inner` is always a CellRef or Range straight from the parser.
            match *inner {
                Expr::CellRef(mut r) => {
                    r.sheet = sheet;
                    Ok(Expr::CellRef(r))
                }
                Expr::Range(mut r) => {
                    r.sheet = sheet;
                    Ok(Expr::Range(r))
                }
                other => resolve_refs(other, sheet, ctx),
            }
        }
        Expr::Name(name) => match ctx.resolve_name(&name) {
            Some(NameTarget::Cell(r)) => Ok(Expr::CellRef(r)),
            Some(NameTarget::Range(r)) => Ok(Expr::Range(r)),
            None => Err(format!("undefined name: {name}")),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Expr {
        parse_formula(input).unwrap_or_else(|e| panic!("parse failed: {e}"))
    }

    #[test]
    fn literal_number() {
        assert!(matches!(parse("42"), Expr::Literal(CellValue::Number(42.0))));
        assert!(matches!(parse("-5"), Expr::UnaryOp { .. }));
        assert!(matches!(parse("1.23"), Expr::Literal(CellValue::Number(v)) if (v - 1.23).abs() < 0.001));
    }

    #[test]
    fn literal_string() {
        assert!(matches!(parse(r#""hello""#), Expr::Literal(CellValue::Text(s)) if s == "hello"));
    }

    #[test]
    fn literal_bool() {
        assert!(matches!(parse("TRUE"), Expr::Literal(CellValue::Boolean(true))));
        assert!(matches!(parse("FALSE"), Expr::Literal(CellValue::Boolean(false))));
    }

    #[test]
    fn cell_reference() {
        assert!(matches!(parse("A1"), Expr::CellRef(CellRef { row: 0, col: 0, .. })));
        assert!(matches!(parse("B2"), Expr::CellRef(CellRef { row: 1, col: 1, .. })));
        assert!(matches!(parse("AA100"), Expr::CellRef(CellRef { row: 99, col: 26, .. })));
    }

    #[test]
    fn cell_range() {
        match parse("A1:B10") {
            Expr::Range(r) => {
                assert_eq!(r.row_start, 0);
                assert_eq!(r.row_end, 9);
                assert_eq!(r.col_start, 0);
                assert_eq!(r.col_end, 1);
            }
            other => panic!("expected Range, got {other:?}"),
        }
    }

    #[test]
    fn arithmetic_add() {
        match parse("A1+B2") {
            Expr::BinOp { kind: BinOpKind::Add, .. } => {}
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn arithmetic_precedence() {
        let ast = parse("1+2*3");
        match &ast {
            Expr::BinOp { kind: BinOpKind::Add, rhs, .. } => {
                assert!(matches!(**rhs, Expr::BinOp { kind: BinOpKind::Mul, .. }));
            }
            other => panic!("expected Add with Mul rhs, got {other:?}"),
        }
    }

    #[test]
    fn parens_override_precedence() {
        let ast = parse("(1+2)*3");
        match &ast {
            Expr::BinOp { kind: BinOpKind::Mul, lhs, .. } => {
                assert!(matches!(**lhs, Expr::BinOp { kind: BinOpKind::Add, .. }));
            }
            other => panic!("expected Mul with Add lhs, got {other:?}"),
        }
    }

    #[test]
    fn comparison_eq() {
        match parse("A1=B1") {
            Expr::BinOp { kind: BinOpKind::Eq, .. } => {}
            other => panic!("expected Eq, got {other:?}"),
        }
    }

    #[test]
    fn comparison_chain() {
        match parse("A1<B1") {
            Expr::BinOp { kind: BinOpKind::Lt, .. } => {}
            other => panic!("expected Lt, got {other:?}"),
        }
    }

    #[test]
    fn concatenation() {
        match parse(r#"A1&"hello""#) {
            Expr::BinOp { kind: BinOpKind::Concat, .. } => {}
            other => panic!("expected Concat, got {other:?}"),
        }
    }

    #[test]
    fn function_no_args() {
        match parse("PI()") {
            Expr::FnCall { name, args } => {
                assert_eq!(name, "PI");
                assert!(args.is_empty());
            }
            other => panic!("expected FnCall, got {other:?}"),
        }
    }

    #[test]
    fn function_single_arg() {
        match parse("SUM(A1:A10)") {
            Expr::FnCall { name, args } => {
                assert_eq!(name, "SUM");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], Expr::Range(_)));
            }
            other => panic!("expected FnCall SUM, got {other:?}"),
        }
    }

    // -- Sheet-qualified references & named ranges ----------------------------

    use super::super::types::{FormulaCtx, NameTarget, RangeDep};

    /// Test context: `Sheet2` -> index 2, name `TOTAL` -> `B1:B3` on sheet 2.
    struct TestCtx;
    impl FormulaCtx for TestCtx {
        fn resolve_sheet(&self, name: &str) -> Option<u16> {
            match name {
                "Sheet1" => Some(0),
                "Sheet2" => Some(2),
                "My Sheet" => Some(5),
                _ => None,
            }
        }
        fn resolve_name(&self, name: &str) -> Option<NameTarget> {
            match name {
                "TOTAL" => Some(NameTarget::Range(RangeDep::with_sheet(2, 0, 2, 1, 1))),
                "PIVOT" => Some(NameTarget::Cell(CellRef::with_sheet(0, 4, 0))),
                _ => None,
            }
        }
    }

    fn resolve(src: &str, default_sheet: u16) -> Result<Expr, String> {
        resolve_refs(parse(src), default_sheet, &TestCtx)
    }

    #[test]
    fn sheet_qualified_cell_parses() {
        assert!(matches!(parse("Sheet2!A1"), Expr::SheetRef { .. }));
    }

    #[test]
    fn sheet_qualified_resolves_index() {
        match resolve("Sheet2!A1", 0).unwrap() {
            Expr::CellRef(CellRef { sheet: 2, row: 0, col: 0 }) => {}
            other => panic!("expected sheet-2 cell, got {other:?}"),
        }
    }

    #[test]
    fn quoted_sheet_name_with_space() {
        match resolve("'My Sheet'!B2", 0).unwrap() {
            Expr::CellRef(CellRef { sheet: 5, row: 1, col: 1 }) => {}
            other => panic!("expected sheet-5 cell, got {other:?}"),
        }
    }

    #[test]
    fn sheet_qualified_range_resolves() {
        match resolve("Sheet2!A1:A10", 0).unwrap() {
            Expr::Range(RangeDep { sheet: 2, row_start: 0, row_end: 9, .. }) => {}
            other => panic!("expected sheet-2 range, got {other:?}"),
        }
    }

    #[test]
    fn unqualified_ref_takes_default_sheet() {
        match resolve("A1", 3).unwrap() {
            Expr::CellRef(CellRef { sheet: 3, row: 0, col: 0 }) => {}
            other => panic!("expected default-sheet cell, got {other:?}"),
        }
    }

    #[test]
    fn unknown_sheet_is_error() {
        assert!(resolve("Nope!A1", 0).is_err());
    }

    #[test]
    fn named_range_resolves() {
        match resolve("SUM(TOTAL)", 0).unwrap() {
            Expr::FnCall { name, args } => {
                assert_eq!(name, "SUM");
                assert!(matches!(
                    &args[0],
                    Expr::Range(RangeDep { sheet: 2, row_start: 0, row_end: 2, .. })
                ));
            }
            other => panic!("expected SUM(range), got {other:?}"),
        }
    }

    #[test]
    fn named_cell_in_arithmetic() {
        // PIVOT + 1 -> CellRef(sheet 0, E1) added to 1.
        match resolve("PIVOT+1", 0).unwrap() {
            Expr::BinOp { lhs, .. } => {
                assert!(matches!(*lhs, Expr::CellRef(CellRef { sheet: 0, row: 4, col: 0 })));
            }
            other => panic!("expected BinOp, got {other:?}"),
        }
    }

    #[test]
    fn undefined_name_is_error() {
        assert!(resolve("UNKNOWN", 0).is_err());
    }

    #[test]
    fn bareword_name_does_not_shadow_cell_ref() {
        // `A1` is a valid cell ref, never treated as a name.
        assert!(matches!(resolve("A1", 0).unwrap(), Expr::CellRef(_)));
    }

    #[test]
    fn function_multiple_args() {
        match parse("IF(A1>5, 1, 0)") {
            Expr::FnCall { name, args } => {
                assert_eq!(name, "IF");
                assert_eq!(args.len(), 3);
            }
            other => panic!("expected FnCall IF, got {other:?}"),
        }
    }

    #[test]
    fn power_right_associative() {
        let ast = parse("2^3^2");
        match &ast {
            Expr::BinOp { kind: BinOpKind::Pow, lhs, rhs } => {
                assert!(matches!(**lhs, Expr::Literal(CellValue::Number(2.0))));
                assert!(matches!(**rhs, Expr::BinOp { kind: BinOpKind::Pow, .. }));
            }
            other => panic!("expected right-assoc pow, got {other:?}"),
        }
    }

    #[test]
    fn empty_formula() {
        assert!(parse_formula("").is_err());
        assert!(parse_formula("   ").is_err());
    }

    #[test]
    fn invalid_formula() {
        assert!(parse_formula("=1").is_err());
        assert!(parse_formula("SUM(").is_err());
    }
}
