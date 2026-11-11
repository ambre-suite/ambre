//! Shared sorts, so that the callers of one element type share one copy of
//! the sort.
//!
//! The standard sort links one copy per element type and per comparator, and
//! each closure is a comparator of its own. A copy weighs about 3.5 KB of
//! wasm, and the download of the Main instance is a budget
//! (`ambre-spec-budgets.md section 5`). A caller that sorts by one of the
//! orders below calls the function here instead of writing its own closure.

use std::cmp::Ordering;

/// Sorts `items` by their partial order, stable. A pair with no order, such
/// as a NaN and a number, counts as equal, so the sort keeps their order.
pub fn sort_partial<T: PartialOrd>(items: &mut [T]) {
    items.sort_by(partial_order);
}

/// The partial order of `a` and `b`, a pair with no order as equal.
fn partial_order<T: PartialOrd>(a: &T, b: &T) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

/// Sorts `pairs` by their first member alone, stable: two pairs with an equal
/// first member keep their order.
pub fn sort_by_first<K: Ord, V>(pairs: &mut [(K, V)]) {
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_sort_orders_numbers_and_keeps_an_unordered_value_in_place() {
        let mut values = [3.0, 1.0, 2.0];
        sort_partial(&mut values);
        assert_eq!(values, [1.0, 2.0, 3.0]);
        // A NaN is equal to every value, so the stable sort leaves it first.
        let mut with_nan = [f64::NAN, 1.0];
        sort_partial(&mut with_nan);
        assert!(with_nan[0].is_nan());
    }

    #[test]
    fn a_sort_by_first_member_is_stable() {
        let mut pairs = [(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd')];
        sort_by_first(&mut pairs);
        assert_eq!(pairs, [(1, 'b'), (1, 'd'), (2, 'a'), (2, 'c')]);
    }
}
