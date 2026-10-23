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

use ambre_bus::{ColDelta, RowDelta};

use super::parser::parse_formula;
use super::render::render;
use super::text::FormulaSource;
use super::types::{AxisSpan, CellRef, CellValue, Expr, FormulaError, RangeDep, SpanLines};

/// Re-index a stored formula's relative references by `(dr, dc)` rows/columns
/// and return the new formula text *with* its leading `=`. A formula that fails
/// to parse is returned verbatim (a paste must never silently drop user input).
pub fn shift_formula(raw: &FormulaSource, dr: RowDelta, dc: ColDelta) -> FormulaSource {
    if dr == RowDelta::ZERO && dc == ColDelta::ZERO {
        return raw.clone();
    }
    let body = raw.formula_body().unwrap_or(raw.as_str());
    match parse_formula(body) {
        Ok(expr) => FormulaSource::new(format!("={}", render(&shift_expr(expr, dr, dc)))),
        Err(_) => raw.clone(),
    }
}

/// The `#REF!` a reference driven off the grid becomes.
fn off_grid() -> Expr {
    Expr::Literal(CellValue::Error(FormulaError::BrokenRef))
}

/// Offset a single cell reference; a reference pushed off the grid becomes a
/// `#REF!` error literal (matching Excel/Sheets).
fn shift_cell(mut r: CellRef, dr: RowDelta, dc: ColDelta) -> Expr {
    match (r.row.checked_offset(dr), r.col.checked_offset(dc)) {
        (Some(row), Some(col)) => {
            r.row = row;
            r.col = col;
            Expr::CellRef(r)
        }
        _ => off_grid(),
    }
}

/// Offset a range reference; if any endpoint leaves the grid the whole range
/// resolves to `#REF!`.
fn shift_range(mut r: RangeDep, dr: RowDelta, dc: ColDelta) -> Expr {
    match (
        r.row_start.checked_offset(dr),
        r.row_end.checked_offset(dr),
        r.col_start.checked_offset(dc),
        r.col_end.checked_offset(dc),
    ) {
        (Some(rs), Some(re), Some(cs), Some(ce)) => {
            r.row_start = rs;
            r.row_end = re;
            r.col_start = cs;
            r.col_end = ce;
            Expr::Range(r)
        }
        _ => off_grid(),
    }
}

/// Offset a range of whole lines along its own axis only: `A:A` pasted one row
/// down still reads `A:A`. A line driven off the grid makes it `#REF!`.
fn shift_span(mut s: AxisSpan, dr: RowDelta, dc: ColDelta) -> Expr {
    let lines = match s.lines {
        SpanLines::Rows { first, last } => first
            .checked_offset(dr)
            .zip(last.checked_offset(dr))
            .map(|(first, last)| SpanLines::Rows { first, last }),
        SpanLines::Cols { first, last } => first
            .checked_offset(dc)
            .zip(last.checked_offset(dc))
            .map(|(first, last)| SpanLines::Cols { first, last }),
    };
    match lines {
        Some(lines) => {
            s.lines = lines;
            Expr::AxisSpan(s)
        }
        None => off_grid(),
    }
}

/// Recursively offset every reference in `expr`. Cross-sheet references
/// (`Sheet2!A1`) shift their inner cell/range too. A relative cross-sheet ref
/// tracks the paste like any other: `=Sheet2!A1` pasted one row down reads
/// `=Sheet2!A2`. Named ranges and literals are left intact.
fn shift_expr(expr: Expr, dr: RowDelta, dc: ColDelta) -> Expr {
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

    /// `raw` shifted by `(dr, dc)`, as text.
    fn shift(raw: &str, dr: RowDelta, dc: ColDelta) -> String {
        shift_formula(&FormulaSource::new(raw.to_owned()), dr, dc).into_string()
    }

    #[test]
    fn shifts_cell_and_range_down_right() {
        // Pasted 2 rows down, 1 column right.
        assert_eq!(shift("=A1", RowDelta::new(2), ColDelta::new(1)), "=B3");
        assert_eq!(
            shift("=SUM(A1:B2)", RowDelta::new(2), ColDelta::new(1)),
            "=SUM(B3:C4)"
        );
    }

    #[test]
    fn whole_lines_shift_along_their_own_axis() {
        assert_eq!(
            shift("=SUM(A:B)", RowDelta::new(2), ColDelta::new(1)),
            "=SUM(B:C)"
        );
        assert_eq!(
            shift("=SUM(2:3)", RowDelta::new(2), ColDelta::new(1)),
            "=SUM(4:5)"
        );
        assert_eq!(
            shift("=SUM(A:A)", RowDelta::new(0), ColDelta::new(-1)),
            "=SUM(#REF!)"
        );
    }

    #[test]
    fn no_shift_is_verbatim() {
        assert_eq!(
            shift("=A1+B2", RowDelta::new(0), ColDelta::new(0)),
            "=A1+B2"
        );
    }

    #[test]
    fn off_grid_reference_becomes_ref_error() {
        // A1 shifted up one row leaves the grid.
        assert_eq!(shift("=A1", RowDelta::new(-1), ColDelta::new(0)), "=#REF!");
    }

    #[test]
    fn keeps_operators_and_functions() {
        assert_eq!(
            shift("=A1*2+SUM(C1:C3)", RowDelta::new(1), ColDelta::new(0)),
            "=A2*2+SUM(C2:C4)"
        );
    }

    #[test]
    fn ref_error_text_is_reparseable() {
        // The shifted text is written back to the CRDT and re-parsed by the
        // engine. If `#REF!` is not part of the grammar the pasted cell settles
        // on `#PARSE!(...)` instead of the `#REF!` the shift decided on.
        let shifted = shift("=A1+B1", RowDelta::new(-1), ColDelta::new(0));
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
            shift(
                "=this is not (a formula",
                RowDelta::new(1),
                ColDelta::new(1)
            ),
            "=this is not (a formula"
        );
    }
}
