//! A rank: where an item falls in an order that a person or a rule sets.
//!
//! A rank orders and does nothing else. It is never counted, added or
//! subtracted, and it addresses no slice. Zero is a rank, so it is no
//! [`Ordinal`](crate::Ordinal), and no issuer hands one out, so it is no
//! [`Seq`](crate::Seq). Two items may share a rank: the tie is the
//! caller's to break, by an id or otherwise.
//!
//! Each order is its own marker, so a rank of one never stands in for a rank
//! of another:
//!
//! ```compile_fail
//! use ambre_core::{Rank, RankKind};
//! enum Shelf {}
//! impl RankKind for Shelf {
//!     type Repr = u32;
//!     const NAME: &'static str = "ShelfRank";
//! }
//! enum Queue {}
//! impl RankKind for Queue {
//!     type Repr = u32;
//!     const NAME: &'static str = "QueueRank";
//! }
//! let _: Rank<Shelf> = Rank::<Queue>::new(0);
//! ```
//!
//! The marker picks the number a rank holds, [`RankKind::Repr`]. A `u32`
//! serves an order that starts at zero, and an `i64` one where a negative
//! rank comes first.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

mod sealed {
    /// Closes [`super::RankRepr`]: a rank is a `u32` or an `i64`.
    pub trait Sealed {}
}

/// The number a rank holds. Sealed: a `u32` or an `i64`.
pub trait RankRepr:
    sealed::Sealed + Copy + Ord + Hash + fmt::Debug + fmt::Display + 'static
{
}

impl sealed::Sealed for u32 {}
impl RankRepr for u32 {}

impl sealed::Sealed for i64 {}
impl RankRepr for i64 {}

/// The marker of one order.
///
/// The marker is an uninhabited type: it names the order and holds no value.
/// The trait is open, because an order lives in the crate that sets it.
pub trait RankKind: 'static {
    /// The number a rank of this order holds.
    type Repr: RankRepr;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// Where an item falls in the order `K` names: the smaller comes first.
pub struct Rank<K: RankKind> {
    raw: K::Repr,
    kind: PhantomData<fn() -> K>,
}

impl<K: RankKind> Rank<K> {
    /// The rank `raw`. Every number of the representation is one.
    #[must_use]
    pub const fn new(raw: K::Repr) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The number the rank holds.
    #[must_use]
    pub const fn get(self) -> K::Repr {
        self.raw
    }
}

/// An order that starts at zero, stored in a document as a signed 64-bit
/// number, the integer a Loro map holds.
impl<K: RankKind<Repr = u32>> Rank<K> {
    /// The first rank: zero.
    pub const FIRST: Self = Self::new(0);

    /// The rank right after this one, saturating at the last rank. A new
    /// item takes it to come after every item it follows.
    #[must_use]
    pub const fn next(self) -> Self {
        Self::new(self.raw.saturating_add(1))
    }

    /// The rank a document stored as `stored`, saturating at the last rank.
    #[must_use]
    pub fn from_stored(stored: u64) -> Self {
        Self::new(u32::try_from(stored).unwrap_or(u32::MAX))
    }

    /// The rank as a document stores it.
    #[must_use]
    pub fn to_stored(self) -> i64 {
        i64::from(self.raw)
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: RankKind> Clone for Rank<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: RankKind> Copy for Rank<K> {}

impl<K: RankKind> PartialEq for Rank<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: RankKind> Eq for Rank<K> {}

impl<K: RankKind> PartialOrd for Rank<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: RankKind> Ord for Rank<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: RankKind> Hash for Rank<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME(n)`, as the derived `Debug` of the hand-written type printed it.
impl<K: RankKind> fmt::Debug for Rank<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number, as the hand-written type printed it.
impl<K: RankKind> fmt::Display for Rank<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Rank, RankKind};

    /// A test order that starts at zero.
    enum Shelf {}
    impl RankKind for Shelf {
        type Repr = u32;
        const NAME: &'static str = "ShelfRank";
    }

    /// A test order where a negative rank comes first.
    enum Queue {}
    impl RankKind for Queue {
        type Repr = i64;
        const NAME: &'static str = "QueueRank";
    }

    #[test]
    fn a_rank_orders_by_its_number_and_zero_is_one() {
        assert_eq!(Rank::<Shelf>::FIRST, Rank::new(0));
        assert!(Rank::<Shelf>::FIRST < Rank::new(1));
        assert_eq!(Rank::<Shelf>::FIRST.next(), Rank::new(1));
        assert_eq!(Rank::<Shelf>::new(u32::MAX).next(), Rank::new(u32::MAX));
        assert!(Rank::<Queue>::new(-1) < Rank::new(0));
        assert_eq!(Rank::<Queue>::new(-7).get(), -7);
        assert_eq!(format!("{:?}", Rank::<Queue>::new(-2)), "QueueRank(-2)");
        assert_eq!(Rank::<Queue>::new(-2).to_string(), "-2");
    }

    #[test]
    fn a_rank_from_zero_reads_back_what_a_document_stored() {
        assert_eq!(Rank::<Shelf>::new(3).to_stored(), 3);
        assert_eq!(Rank::<Shelf>::from_stored(3), Rank::new(3));
        assert_eq!(Rank::<Shelf>::from_stored(u64::MAX), Rank::new(u32::MAX));
    }
}
