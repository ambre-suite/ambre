//! Relative-reference shifting for clipboard paste (`trame-clipboard`). When a
//! range is copied and pasted elsewhere, every reference in a pasted formula
//! moves by the same offset as the carrying cell. That is the standard
//! "relative reference" behaviour. AMBRE has no absolute (`$A$1`) syntax, so
//! every cell/range reference is relative and shifts.
//!
//! The shift is a parse -> rewrite -> render round-trip over the stored formula
//! text. [`parse_formula`] yields the AST, and [`shift_expr`] offsets every
//! [`CellRef`]/[`RangeDep`]. A reference driven off the grid resolves to
//! `#REF!`, and [`render`] serialises back to canonical formula text. The text
//! shifted here is the *display* form the UI cached; the Logic thread
//! re-canonicalises it on write, so cross-sheet qualifiers survive.

use super::parser::parse_formula;
use super::render::render;
use super::types::{AxisSpan, CellRef, CellValue, Expr, FormulaError, LineAxis, RangeDep};

/// Re-index a stored formula's relative references by `(dr, dc)` rows/columns
/// and return the new formula text *with* its leading `=`. A formula that fails
/// to parse is returned verbatim (a paste must never silently drop user input).
pub fn shift_formula(raw: &str, dr: i64, dc: i64) -> String {
    if dr == 0 && dc == 0 {
        return raw.to_owned();
    }
    let body = raw.strip_prefix('=').unwrap_or(raw);
    match parse_formula(body) {
        Ok(expr) => format!("={}", render(&shift_expr(expr, dr, dc))),
        Err(_) => raw.to_owned(),
    }
}

/// Offset a 0-based axis index, returning `None` when the shift drives it off
/// the grid (a negative position) -- the caller turns that into `#REF!`.
fn shift_idx(v: u32, d: i64) -> Option<u32> {
    let n = v as i64 + d;
    (n >= 0).then_some(n as u32)
}

/// Offset a single cell reference; a reference pushed off the grid becomes a
/// `#REF!` error literal (matching Excel/Sheets).
fn shift_cell(mut r: CellRef, dr: i64, dc: i64) -> Expr {
    match (shift_idx(r.row, dr), shift_idx(r.col, dc)) {
        (Some(row), Some(col)) => {
            r.row = row;
            r.col = col;
            Expr::CellRef(r)
        }
        _ => Expr::Literal(CellValue::Error(FormulaError::BrokenRef)),
    }
}

/// Offset a range reference; if any endpoint leaves the grid the whole range
/// resolves to `#REF!`.
fn shift_range(mut r: RangeDep, dr: i64, dc: i64) -> Expr {
    match (
        shift_idx(r.row_start, dr),
        shift_idx(r.row_end, dr),
        shift_idx(r.col_start, dc),
        shift_idx(r.col_end, dc),
    ) {
        (Some(rs), Some(re), Some(cs), Some(ce)) => {
            r.row_start = rs;
            r.row_end = re;
            r.col_start = cs;
            r.col_end = ce;
            Expr::Range(r)
        }
        _ => Expr::Literal(CellValue::Error(FormulaError::BrokenRef)),
    }
}

/// Offset a range of whole lines along its own axis only: `A:A` pasted one row
/// down still reads `A:A`. A line driven off the grid makes it `#REF!`.
fn shift_span(mut s: AxisSpan, dr: i64, dc: i64) -> Expr {
    let d = match s.axis {
        LineAxis::Row => dr,
        LineAxis::Col => dc,
    };
    match (shift_idx(s.start, d), shift_idx(s.end, d)) {
        (Some(start), Some(end)) => {
            s.start = start;
            s.end = end;
            Expr::AxisSpan(s)
        }
        _ => Expr::Literal(CellValue::Error(FormulaError::BrokenRef)),
    }
}

/// Recursively offset every reference in `expr`. Cross-sheet references
/// (`Sheet2!A1`) shift their inner cell/range too. A relative cross-sheet ref
/// tracks the paste like any other: `=Sheet2!A1` pasted one row down reads
/// `=Sheet2!A2`. Named ranges and literals are left intact.
fn shift_expr(expr: Expr, dr: i64, dc: i64) -> Expr {
    match expr {
        Expr::CellRef(r) => shift_cell(r, dr, dc),
        Expr::Range(r) => shift_range(r, dr, dc),
        Expr::AxisSpan(s) => shift_span(s, dr, dc),
        Expr::SheetRef { sheet, inner } => Expr::SheetRef {
            sheet,
            inner: Box::new(shift_expr(*inner, dr, dc)),
        },
        Expr::BinOp { kind, lhs, rhs } => Expr::BinOp {
            kind,
            lhs: Box::new(shift_expr(*lhs, dr, dc)),
            rhs: Box::new(shift_expr(*rhs, dr, dc)),
        },
        Expr::UnaryOp { kind, operand } => Expr::UnaryOp {
            kind,
            operand: Box::new(shift_expr(*operand, dr, dc)),
        },
        Expr::FnCall { name, args } => Expr::FnCall {
            name,
            args: args.into_iter().map(|a| shift_expr(a, dr, dc)).collect(),
        },
        leaf => leaf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_cell_and_range_down_right() {
        // Pasted 2 rows down, 1 column right.
        assert_eq!(shift_formula("=A1", 2, 1), "=B3");
        assert_eq!(shift_formula("=SUM(A1:B2)", 2, 1), "=SUM(B3:C4)");
    }

    #[test]
    fn whole_lines_shift_along_their_own_axis() {
        assert_eq!(shift_formula("=SUM(A:B)", 2, 1), "=SUM(B:C)");
        assert_eq!(shift_formula("=SUM(2:3)", 2, 1), "=SUM(4:5)");
        assert_eq!(shift_formula("=SUM(A:A)", 0, -1), "=SUM(#REF!)");
    }

    #[test]
    fn no_shift_is_verbatim() {
        assert_eq!(shift_formula("=A1+B2", 0, 0), "=A1+B2");
    }

    #[test]
    fn off_grid_reference_becomes_ref_error() {
        // A1 shifted up one row leaves the grid.
        assert_eq!(shift_formula("=A1", -1, 0), "=#REF!");
    }

    #[test]
    fn keeps_operators_and_functions() {
        assert_eq!(shift_formula("=A1*2+SUM(C1:C3)", 1, 0), "=A2*2+SUM(C2:C4)");
    }

    #[test]
    fn ref_error_text_is_reparseable() {
        // The shifted text is written back to the CRDT and re-parsed by the
        // engine. If `#REF!` is not part of the grammar the pasted cell settles
        // on `#PARSE!(...)` instead of the `#REF!` the shift decided on.
        let shifted = shift_formula("=A1+B1", -1, 0);
        assert_eq!(shifted, "=#REF!+#REF!");
        let body = shifted.strip_prefix('=').unwrap();
        assert!(
            parse_formula(body).is_ok(),
            "the renderer must only emit text the parser accepts"
        );
    }

    #[test]
    fn unparseable_left_verbatim() {
        assert_eq!(
            shift_formula("=this is not (a formula", 1, 1),
            "=this is not (a formula"
        );
    }
}
