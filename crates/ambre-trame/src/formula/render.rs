//! AST-to-text rendering and sheet-qualifier rewriting.
//!
//! Used for two host flows around the CRDT `v` text (spec section 2.D):
//!
//! - **Canonicalization** (user input -> stored `v`): [`canonicalize`] rewrites
//!   every display-name sheet qualifier into its stable id, then [`render`]
//!   serializes back to formula text. The stored form survives rename/reorder.
//! - **Display** (stored `v` -> formula bar): [`to_display_form`] rewrites every
//!   id qualifier back into the sheet's current display name, then [`render`]
//!   produces the human-facing text.
//!
//! The renderer is precedence-aware and emits the minimum parentheses needed, so
//! the output re-parses to an equivalent AST. Whitespace is normalized (Excel
//! and Sheets reformat typed formulas the same way).

use super::types::{
    col_to_alpha, BinOpKind, CellRef, CellValue, Expr, RangeDep, SheetQualifier, UnaryOpKind,
};

// -----------------------------------------------------------------------------
// Qualifier rewriting
// -----------------------------------------------------------------------------

/// Rewrite every sheet qualifier in `expr` through `f` (post-order: `inner`
/// nodes are left untouched, only the qualifier token is mapped).
fn map_qualifiers(expr: Expr, f: &impl Fn(SheetQualifier) -> SheetQualifier) -> Expr {
    match expr {
        Expr::SheetRef { sheet, inner } => Expr::SheetRef {
            sheet: f(sheet),
            inner: Box::new(map_qualifiers(*inner, f)),
        },
        Expr::BinOp { kind, lhs, rhs } => Expr::BinOp {
            kind,
            lhs: Box::new(map_qualifiers(*lhs, f)),
            rhs: Box::new(map_qualifiers(*rhs, f)),
        },
        Expr::UnaryOp { kind, operand } => Expr::UnaryOp {
            kind,
            operand: Box::new(map_qualifiers(*operand, f)),
        },
        Expr::FnCall { name, args } => Expr::FnCall {
            name,
            args: args.into_iter().map(|a| map_qualifiers(a, f)).collect(),
        },
        leaf => leaf,
    }
}

/// Rewrite display-name qualifiers (`Sheet2!`) into stable-id form (`#s0!`).
/// A name that `name_to_id` cannot resolve is left as-is (it will surface as a
/// parse error, never silently dropping user input).
pub fn canonicalize(expr: Expr, name_to_id: &impl Fn(&str) -> Option<String>) -> Expr {
    map_qualifiers(expr, &|q| match q {
        SheetQualifier::Name(n) => match name_to_id(&n) {
            Some(id) => SheetQualifier::Id(id),
            None => SheetQualifier::Name(n),
        },
        id => id,
    })
}

/// Rewrite stable-id qualifiers (`#s0!`) back into the sheet's current display
/// name (`Data!`). An id that `id_to_name` cannot resolve (deleted sheet) is
/// left in `#id` form, which signals the dangling reference in the formula bar.
pub fn to_display_form(expr: Expr, id_to_name: &impl Fn(&str) -> Option<String>) -> Expr {
    map_qualifiers(expr, &|q| match q {
        SheetQualifier::Id(id) => match id_to_name(&id) {
            Some(name) => SheetQualifier::Name(name),
            None => SheetQualifier::Id(id),
        },
        name => name,
    })
}

// -----------------------------------------------------------------------------
// Rendering
// -----------------------------------------------------------------------------

/// Serialize an [`Expr`] AST back to formula text (without the leading `=`).
pub fn render(expr: &Expr) -> String {
    // Top-level context: the lowest binary precedence, so no outer parens.
    emit(expr, 1)
}

/// Binding precedence of a node (higher binds tighter); mirrors the parser's
/// grammar layering so the renderer can decide parenthesization.
fn precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::BinOp { kind, .. } => match kind {
            BinOpKind::Eq
            | BinOpKind::Ne
            | BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge => 1,
            BinOpKind::Concat => 2,
            BinOpKind::Add | BinOpKind::Sub => 3,
            BinOpKind::Mul | BinOpKind::Div => 4,
            BinOpKind::Pow => 5,
        },
        Expr::UnaryOp { .. } => 6,
        _ => 7,
    }
}

/// Render `expr`, wrapping it in parentheses when its precedence is below the
/// `min_prec` required by its parent context.
fn emit(expr: &Expr, min_prec: u8) -> String {
    let s = render_node(expr);
    if precedence(expr) < min_prec {
        format!("({s})")
    } else {
        s
    }
}

fn render_node(expr: &Expr) -> String {
    match expr {
        Expr::Literal(v) => render_literal(v),
        Expr::CellRef(r) => render_cell(r),
        Expr::Range(r) => render_range(r),
        Expr::Name(n) => n.clone(),
        Expr::SheetRef { sheet, inner } => {
            format!("{}!{}", render_qualifier(sheet), render_node(inner))
        }
        Expr::BinOp { kind, lhs, rhs } => {
            let p = precedence(expr);
            // All binary operators are left-associative except `^` (right).
            let (lhs_min, rhs_min) = if *kind == BinOpKind::Pow {
                (p + 1, p)
            } else {
                (p, p + 1)
            };
            format!("{}{}{}", emit(lhs, lhs_min), binop_symbol(*kind), emit(rhs, rhs_min))
        }
        Expr::UnaryOp { kind, operand } => match kind {
            UnaryOpKind::Neg => format!("-{}", emit(operand, 6)),
            // `Not` is never produced by the parser (NOT is a function); render
            // defensively as the function form.
            UnaryOpKind::Not => format!("NOT({})", render(operand)),
        },
        Expr::FnCall { name, args } => {
            let rendered: Vec<String> = args.iter().map(|a| emit(a, 1)).collect();
            format!("{}({})", name, rendered.join(","))
        }
    }
}

fn render_literal(v: &CellValue) -> String {
    match v {
        CellValue::Number(n) => {
            if *n == n.trunc() && n.is_finite() {
                format!("{n:.0}")
            } else {
                format!("{n}")
            }
        }
        // Escape embedded quotes by doubling, per the parser's string rules.
        CellValue::Text(s) => format!("\"{}\"", s.replace('"', "\"\"")),
        CellValue::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Empty => String::new(),
        // Internal-only / defensive: an error or range literal in source text.
        CellValue::Error(e) => format!("{e}"),
        CellValue::Range(_) => String::new(),
    }
}

fn render_cell(r: &CellRef) -> String {
    format!("{}{}", col_to_alpha(r.col), r.row + 1)
}

fn render_range(r: &RangeDep) -> String {
    format!(
        "{}{}:{}{}",
        col_to_alpha(r.col_start),
        r.row_start + 1,
        col_to_alpha(r.col_end),
        r.row_end + 1
    )
}

fn render_qualifier(q: &SheetQualifier) -> String {
    match q {
        SheetQualifier::Id(id) => format!("#{id}"),
        SheetQualifier::Name(name) => {
            if needs_quoting(name) {
                format!("'{name}'")
            } else {
                name.clone()
            }
        }
    }
}

/// A sheet display name must be single-quoted unless it is a plain identifier
/// (`alpha`/`_` start, then `alphanumeric`/`_`). This matches the bareword form
/// the parser accepts without quotes.
fn needs_quoting(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => true,
        Some(c) if !c.is_ascii_alphabetic() && c != '_' => true,
        _ => chars.any(|c| !c.is_ascii_alphanumeric() && c != '_'),
    }
}

fn binop_symbol(kind: BinOpKind) -> &'static str {
    match kind {
        BinOpKind::Add => "+",
        BinOpKind::Sub => "-",
        BinOpKind::Mul => "*",
        BinOpKind::Div => "/",
        BinOpKind::Pow => "^",
        BinOpKind::Eq => "=",
        BinOpKind::Ne => "<>",
        BinOpKind::Lt => "<",
        BinOpKind::Le => "<=",
        BinOpKind::Gt => ">",
        BinOpKind::Ge => ">=",
        BinOpKind::Concat => "&",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parser::parse_formula;

    /// Parse, render, and assert the round-trip yields the expected canonical text.
    fn roundtrip(src: &str) -> String {
        render(&parse_formula(src).unwrap())
    }

    #[test]
    fn precedence_minimal_parens() {
        assert_eq!(roundtrip("2+3*4"), "2+3*4");
        assert_eq!(roundtrip("(2+3)*4"), "(2+3)*4");
        assert_eq!(roundtrip("2*3+4"), "2*3+4");
        assert_eq!(roundtrip("2-(3-4)"), "2-(3-4)");
        assert_eq!(roundtrip("2-3-4"), "2-3-4");
    }

    #[test]
    fn pow_right_associative() {
        assert_eq!(roundtrip("2^3^2"), "2^3^2");
        assert_eq!(roundtrip("(2^3)^2"), "(2^3)^2");
    }

    #[test]
    fn refs_and_ranges() {
        assert_eq!(roundtrip("A1"), "A1");
        assert_eq!(roundtrip("SUM(A1:B10)"), "SUM(A1:B10)");
        assert_eq!(roundtrip("IF(A1>0,\"yes\",\"no\")"), "IF(A1>0,\"yes\",\"no\")");
    }

    #[test]
    fn string_quotes_escaped() {
        assert_eq!(roundtrip(r#""a""b""#), r#""a""b""#);
    }

    #[test]
    fn canonicalize_name_to_id() {
        let ast = parse_formula("Sheet2!A1+1").unwrap();
        let name_to_id = |n: &str| (n == "Sheet2").then(|| "s0_a1b2".to_string());
        let canon = canonicalize(ast, &name_to_id);
        assert_eq!(render(&canon), "#s0_a1b2!A1+1");
    }

    #[test]
    fn display_id_to_name() {
        let ast = parse_formula("#s0_a1b2!A1+1").unwrap();
        let id_to_name = |id: &str| (id == "s0_a1b2").then(|| "Data".to_string());
        let disp = to_display_form(ast, &id_to_name);
        assert_eq!(render(&disp), "Data!A1+1");
    }

    #[test]
    fn display_quotes_name_with_space() {
        let ast = parse_formula("#s0!A1").unwrap();
        let id_to_name = |id: &str| (id == "s0").then(|| "My Sheet".to_string());
        let disp = to_display_form(ast, &id_to_name);
        assert_eq!(render(&disp), "'My Sheet'!A1");
    }

    #[test]
    fn unresolved_name_left_intact() {
        let ast = parse_formula("Ghost!A1").unwrap();
        let canon = canonicalize(ast, &|_| None);
        assert_eq!(render(&canon), "Ghost!A1");
    }

    #[test]
    fn deleted_id_kept_in_hash_form() {
        let ast = parse_formula("#gone!A1").unwrap();
        let disp = to_display_form(ast, &|_| None);
        assert_eq!(render(&disp), "#gone!A1");
    }
}
