// ambre-trame/src/logic/helpers.rs
// Free helpers shared across the impl submodules: name-target resolution, value
// ordering, filter predicates, and filter-op string round-trips.

use super::*;
use crate::formula::text::{fold_char, fold_cmp};

/// Convert a resolved name-definition expression into a [`NameTarget`]. Only a
/// bare cell or range is a valid named-range body; anything else (an arithmetic
/// expression, a function call) is rejected.
pub(crate) fn expr_to_name_target(expr: Expr) -> Option<NameTarget> {
    match expr {
        Expr::CellRef(r) => Some(NameTarget::Cell(r)),
        Expr::Range(r) => Some(NameTarget::Range(r)),
        _ => None,
    }
}

/// Order two cell values for sorting (`trame-sort`). Within a type the natural
/// order applies; text reads whatever its case, through the fold the formula
/// operators share (`TRAME-255`). Two texts that fold equal compare equal, so
/// a stable sort keeps their order. Across types the precedence is `Number < Text < Boolean <
/// Error < Empty`, so blanks sink to the bottom. Errors group just above them,
/// matching the spec. `Range` only ever appears on the VM stack, never in the
/// cache, so it is treated as `Empty` defensively.
pub(crate) fn compare_cell_values(a: &CellValue, b: &CellValue) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    // Type rank: lower sorts first.
    // A NaN is not a number: it ranks with the errors, so the comparator stays
    // a total order whatever the cell holds (`TRAME-181`).
    fn rank(v: &CellValue) -> u8 {
        match v {
            CellValue::Number(n) if n.is_nan() => 3,
            CellValue::Number(_) => 0,
            CellValue::Text(_) => 1,
            CellValue::Boolean(_) => 2,
            CellValue::Error(_) => 3,
            CellValue::Empty | CellValue::Range(_) => 4,
        }
    }
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) if !x.is_nan() && !y.is_nan() => {
            x.partial_cmp(y).unwrap_or(Equal)
        }
        (CellValue::Text(x), CellValue::Text(y)) => fold_cmp(x, y),
        (CellValue::Boolean(x), CellValue::Boolean(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Delete every key of a Loro map (its child containers are detached with it).
/// `LoroMap` has no `clear`, so we enumerate and delete -- used to reset the
/// filter definition / hidden-row maps before rewriting them.
pub(crate) fn clear_loro_map(map: &loro::LoroMap) {
    if let LoroValue::Map(entries) = map.get_value() {
        for key in entries.keys() {
            let _ = map.delete(key);
        }
    }
}

/// Whether a row's filtered-column `value` passes the filter (stays visible),
/// per [`FilterKind`] (`trame-shared-filter`).
pub(crate) fn filter_pass(kind: &FilterKind, value: &CellValue) -> bool {
    match kind {
        // "Filter by values": the listed values are the visible (checked) ones.
        FilterKind::Values(allowed) => {
            let text = value.to_display();
            allowed.contains(&text)
        }
        FilterKind::Condition { op, value: operand } => condition_pass(*op, value, operand),
    }
}

/// Evaluate a single [`FilterOp`] against a cell value and a comparison operand.
/// Comparisons are numeric when both sides parse as numbers. Otherwise they
/// read the display text whatever its case, through the fold the formula
/// operators share (`TRAME-256`). The unary operators ignore `operand`.
pub(crate) fn condition_pass(op: FilterOp, value: &CellValue, operand: &str) -> bool {
    use std::cmp::Ordering;
    // The substring tests need no offset back into the text, so a folded copy
    // of each side serves them.
    let folded = |s: &str| s.chars().flat_map(fold_char).collect::<String>();
    let text = value.to_display();
    // A numeric comparison applies only when *both* operands are numeric.
    let nums = match (
        value.as_number(),
        crate::formula::types::finite_number(operand),
    ) {
        (Ok(a), Some(b)) => Some((a, b)),
        _ => None,
    };
    // Only treat as numeric when the cell itself is a number (avoid "abc"->Err
    // but Empty->0.0 surprises): require the value to be a real Number.
    let order = match (matches!(value, CellValue::Number(_)), nums) {
        (true, Some((a, b))) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        _ => fold_cmp(&text, operand),
    };
    let eq = match (matches!(value, CellValue::Number(_)), nums) {
        (true, Some((a, b))) => a == b,
        _ => fold_cmp(&text, operand).is_eq(),
    };
    match op {
        FilterOp::Eq => eq,
        FilterOp::Ne => !eq,
        FilterOp::Gt => order == Ordering::Greater,
        FilterOp::Lt => order == Ordering::Less,
        FilterOp::Gte => order != Ordering::Less,
        FilterOp::Lte => order != Ordering::Greater,
        FilterOp::Contains => folded(&text).contains(&folded(operand)),
        FilterOp::NotContains => !folded(&text).contains(&folded(operand)),
        FilterOp::StartsWith => folded(&text).starts_with(&folded(operand)),
        FilterOp::IsEmpty => text.is_empty(),
        FilterOp::IsNotEmpty => !text.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::types::FormulaError;
    use std::cmp::Ordering;

    /// One value of every kind the comparator meets, a NaN among the numbers.
    fn sample() -> Vec<CellValue> {
        vec![
            CellValue::Number(f64::NAN),
            CellValue::Number(1.0),
            CellValue::Number(2.0),
            CellValue::Number(f64::NEG_INFINITY),
            CellValue::Text("a".into()),
            CellValue::Text("A".into()),
            CellValue::Text("b".into()),
            CellValue::Text("B".into()),
            CellValue::Boolean(false),
            CellValue::Boolean(true),
            CellValue::Error(FormulaError::NumError),
            CellValue::Error(FormulaError::BrokenRef),
            CellValue::Empty,
        ]
    }

    #[test]
    fn comparator_is_a_total_order_with_nan() {
        let values = sample();
        for a in &values {
            for b in &values {
                assert_eq!(
                    compare_cell_values(a, b),
                    compare_cell_values(b, a).reverse(),
                    "antisymmetry: {a:?} {b:?}"
                );
                for c in &values {
                    let ab = compare_cell_values(a, b);
                    let bc = compare_cell_values(b, c);
                    if ab != Ordering::Greater && bc != Ordering::Greater {
                        assert_ne!(
                            compare_cell_values(a, c),
                            Ordering::Greater,
                            "transitivity: {a:?} {b:?} {c:?}"
                        );
                    }
                }
            }
        }
        // A NaN orders after every number, beside the errors.
        assert_eq!(
            compare_cell_values(&CellValue::Number(f64::NAN), &CellValue::Number(2.0)),
            Ordering::Greater
        );
    }
}
