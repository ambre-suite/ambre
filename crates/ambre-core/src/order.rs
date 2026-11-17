//! Shared sorts, so that the callers of one element type share one copy of
//! the sort.
//!
//! The standard sort links one copy per element type and per comparator, and
//! each closure is a comparator of its own. A copy weighs about 3.5 KB of
//! wasm, and the download of the Main instance is a budget
//! (`ambre-spec-budgets.md section 5`). A caller that sorts by one of the
//! orders below calls the function here instead of writing its own closure.

use std::cmp::Ordering;

use crate::{Entity, Id, Seq, Uuid, Wire};

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

/// Sorts `items` by the identifier `key` gives each one, in the order of its
/// bytes, stable: two items with an equal key keep their order. The byte
/// order is the order of the hyphenated text too.
///
/// Every caller shares one copy of the sort, whatever its element type and its
/// entity: the sort runs over the bytes of the keys.
pub fn sort_by_id<T, E: Entity<Repr = Uuid>>(items: &mut [T], key: impl Fn(&T) -> Id<E>) {
    sort_keyed(items, |item| *key(item).as_bytes());
}

/// Sorts `items` by the pair `key` gives each one: a first key, then the bytes
/// of an identifier, stable. The callers of one first-key type share one copy
/// of the sort, whatever their element type and their entity.
pub fn sort_by_key_then_id<T, K: Ord, E: Entity<Repr = Uuid>>(
    items: &mut [T],
    key: impl Fn(&T) -> (K, Id<E>),
) {
    sort_keyed(items, |item| {
        let (first, id) = key(item);
        (first, *id.as_bytes())
    });
}

/// Sorts `items` by the sequence number `key` gives each one, stable: two
/// items with an equal number keep their order.
///
/// Every caller shares one copy of the sort, whatever its element type and
/// its sequence family: the sort runs over the numbers.
pub fn sort_by_seq<T, K: Wire>(items: &mut [T], key: impl Fn(&T) -> Seq<K>) {
    sort_keyed(items, |item| key(item).get());
}

/// Sorts `items` by the key `key` gives each one, stable. The callers of one
/// key type share one copy of the sort, whatever their element type.
///
/// The sort runs over the keys and their places, and the items then move to
/// their places. The keys take one allocation per call.
fn sort_keyed<T, K: Ord>(items: &mut [T], key: impl Fn(&T) -> K) {
    let mut keyed: Vec<(K, usize)> = items
        .iter()
        .enumerate()
        .map(|(place, item)| (key(item), place))
        .collect();
    // The place breaks a tie, so the unstable sort orders as a stable one.
    keyed.sort_unstable();
    let from: Vec<usize> = keyed.into_iter().map(|(_, place)| place).collect();
    // Place `i` takes the item that stood at `from[i]`. Each place before `i`
    // already holds its item. The item `from[i]` names left its place when a
    // swap filled that place, and the walk follows it to where it went.
    for (i, &first) in from.iter().enumerate() {
        let mut source = first;
        while source < i {
            // `from` is a permutation of the places, so every place is in it.
            match from.get(source) {
                Some(&next) => source = next,
                None => break,
            }
        }
        items.swap(i, source);
    }
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

    #[test]
    fn a_sort_by_id_matches_the_stable_sort_of_the_standard_library() {
        // A small generator keeps the test free of a random source.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for len in 0..40 {
            let items: Vec<(u8, usize)> = (0..len)
                .map(|place| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    // Few distinct keys, so ties are common.
                    ((seed >> 61) as u8, place)
                })
                .collect();
            let key = |item: &(u8, usize)| Id::<crate::id::Document>::from_bytes([item.0; 16]);
            let mut ours = items.clone();
            sort_by_id(&mut ours, key);
            let mut std_sort = items;
            std_sort.sort_by_key(|item| *key(item).as_bytes());
            assert_eq!(ours, std_sort, "length {len}");
        }
    }

    #[test]
    fn a_sort_by_seq_matches_the_stable_sort_of_the_standard_library() {
        let mut seed: u64 = 0xD1B5_4A32_D192_ED03;
        for len in 0..40 {
            let items: Vec<(u64, usize)> = (0..len)
                .map(|place| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    // Few distinct numbers, so ties are common.
                    (seed >> 61, place)
                })
                .collect();
            let key = |item: &(u64, usize)| Seq::<crate::Relay>::new(item.0);
            let mut ours = items.clone();
            sort_by_seq(&mut ours, key);
            let mut std_sort = items;
            std_sort.sort_by_key(|item| item.0);
            assert_eq!(ours, std_sort, "length {len}");
        }
    }

    #[test]
    fn a_sort_by_key_then_id_matches_the_stable_sort_of_the_standard_library() {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for len in 0..40 {
            let items: Vec<(u8, u8, usize)> = (0..len)
                .map(|place| {
                    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    ((seed >> 62) as u8, (seed >> 59) as u8 & 3, place)
                })
                .collect();
            let key = |item: &(u8, u8, usize)| {
                (item.0, Id::<crate::id::Document>::from_bytes([item.1; 16]))
            };
            let mut ours = items.clone();
            sort_by_key_then_id(&mut ours, key);
            let mut std_sort = items;
            std_sort.sort_by_key(|item| (item.0, [item.1; 16]));
            assert_eq!(ours, std_sort, "length {len}");
        }
    }
}
