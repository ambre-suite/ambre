//! A tally: a running count of one kind of thing, held at the top rather
//! than wrapping.
//!
//! A tally is not an index. It addresses nothing, it only goes up, and it is
//! read or printed. A record that keeps several tallies side by side gives
//! each its own marker, so one cannot be added into another:
//!
//! ```compile_fail
//! use ambre_core::{Tally, TallyKind};
//! enum Apples {}
//! impl TallyKind for Apples {
//!     type Repr = u32;
//!     const NAME: &'static str = "AppleCount";
//! }
//! enum Pears {}
//! impl TallyKind for Pears {
//!     type Repr = u32;
//!     const NAME: &'static str = "PearCount";
//! }
//! let _ = Tally::<Apples>::ZERO.plus(Tally::<Pears>::ZERO);
//! ```
//!
//! The marker picks the width of the count, [`TallyKind::Repr`]: a `u32`
//! for most tallies, a `u64` for one that runs past 32 bits. Serde writes the
//! bare number at that width, so a record keeps its wire bytes.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::span::StoredAsI64;

mod sealed {
    /// Closes [`super::TallyRepr`]: a tally is a `u32` or a `u64`.
    pub trait Sealed {}
}

/// The number a tally holds. Sealed: a `u32` or a `u64`.
pub trait TallyRepr:
    sealed::Sealed
    + Copy
    + Ord
    + Hash
    + fmt::Debug
    + fmt::Display
    + Serialize
    + DeserializeOwned
    + 'static
{
    /// Nothing counted.
    const ZERO: Self;

    /// The number `n`, held at the top of the width.
    fn saturating_from_u64(n: u64) -> Self;

    /// The number `len`, held at the top of the width.
    fn saturating_from_usize(len: usize) -> Self;
}

impl sealed::Sealed for u32 {}
impl TallyRepr for u32 {
    const ZERO: Self = 0;

    fn saturating_from_u64(n: u64) -> Self {
        Self::try_from(n).unwrap_or(Self::MAX)
    }

    fn saturating_from_usize(len: usize) -> Self {
        Self::try_from(len).unwrap_or(Self::MAX)
    }
}

impl sealed::Sealed for u64 {}
impl TallyRepr for u64 {
    const ZERO: Self = 0;

    fn saturating_from_u64(n: u64) -> Self {
        n
    }

    fn saturating_from_usize(len: usize) -> Self {
        Self::try_from(len).unwrap_or(Self::MAX)
    }
}

/// The marker of one kind of tally.
///
/// The marker is an uninhabited type: it names what is counted and holds no
/// value. The trait is open, because the tallies live in the crates that
/// count them.
pub trait TallyKind: 'static {
    /// The number a tally of this kind holds, and the width serde writes.
    type Repr: TallyRepr;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A tally of `K` that a tally of `J` gives one for one: each thing `J`
/// counted is one thing of `K`. The two count at one width.
///
/// A capability that each pair opts in to. A count of messages gives a count
/// of unread messages. A count of apples gives no count of pears.
///
/// ```compile_fail
/// use ambre_core::{Tally, TallyKind};
/// enum Apples {}
/// impl TallyKind for Apples {
///     type Repr = u32;
///     const NAME: &'static str = "AppleCount";
/// }
/// enum Pears {}
/// impl TallyKind for Pears {
///     type Repr = u32;
///     const NAME: &'static str = "PearCount";
/// }
/// let _ = Tally::<Pears>::recount(Tally::<Apples>::ZERO);
/// ```
///
/// The same recount compiles once the pair opts in:
///
/// ```
/// use ambre_core::{Recounts, Tally, TallyKind};
/// enum Apples {}
/// impl TallyKind for Apples {
///     type Repr = u32;
///     const NAME: &'static str = "AppleCount";
/// }
/// enum Pears {}
/// impl TallyKind for Pears {
///     type Repr = u32;
///     const NAME: &'static str = "PearCount";
/// }
/// impl Recounts<Apples> for Pears {}
/// assert_eq!(Tally::<Pears>::recount(Tally::<Apples>::new(3)).get(), 3);
/// ```
pub trait Recounts<J: TallyKind>: TallyKind<Repr = J::Repr> {}

/// A tally a sum is divided by, for an average.
///
/// A capability that a 64-bit kind opts in to: the division reads the count
/// as an `f64`, which holds every count below 2^53 exactly. A tally that is
/// never a divisor has no `as_f64`.
///
/// ```compile_fail,E0599
/// use ambre_core::{Tally, TallyKind};
/// enum Rows {}
/// impl TallyKind for Rows {
///     type Repr = u64;
///     const NAME: &'static str = "RowCount";
/// }
/// let _ = Tally::<Rows>::new(3).as_f64();
/// ```
pub trait Divisor: TallyKind<Repr = u64> {}

/// A 64-bit tally that a store keeps in a signed 64-bit column, through
/// [`StoredAsI64`].
///
/// A capability that each kind opts in to, so a tally that never reaches a
/// store has no store spelling.
pub trait TallyColumn: TallyKind<Repr = u64> {}

/// A running count of the things `K` names, held at the top rather than
/// wrapping.
///
/// `R` is always `K::Repr`, which the default fills in: write `Tally<K>`.
/// The parameter exists so that the arithmetic of each width is a `const
/// fn`. rustc refuses two inherent impls told apart by an associated type
/// alone (rust-lang/rust#20400), and accepts two told apart by a parameter.
/// The field still names `K::Repr`, the representation the primitive gate
/// reads.
///
/// Serde sees the bare number, so a record keeps its wire bytes.
pub struct Tally<K: TallyKind<Repr = R>, R: TallyRepr = <K as TallyKind>::Repr> {
    raw: <K as TallyKind>::Repr,
    kind: PhantomData<fn() -> (K, R)>,
}

impl<K: TallyKind> Tally<K> {
    /// Nothing counted.
    pub const ZERO: Self = Self::new(K::Repr::ZERO);

    /// A tally of `n`.
    #[must_use]
    pub const fn new(n: K::Repr) -> Self {
        Self {
            raw: n,
            kind: PhantomData,
        }
    }

    /// The count itself.
    #[must_use]
    pub const fn get(self) -> K::Repr {
        self.raw
    }

    /// A tally of `len` things, held at the top of the width.
    #[must_use]
    pub fn of_len(len: usize) -> Self {
        Self::new(K::Repr::saturating_from_usize(len))
    }

    /// A tally of `n` things, held at the top of the width. For the number
    /// of a series, whose gaps a tally counts.
    pub(crate) fn saturating_from_u64(n: u64) -> Self {
        Self::new(K::Repr::saturating_from_u64(n))
    }

    /// The things a tally of `J` counted, each one counted again as `K`.
    #[must_use]
    pub const fn recount<J: TallyKind>(other: Tally<J>) -> Self
    where
        K: Recounts<J>,
    {
        Self::new(other.raw)
    }
}

impl<K: TallyKind<Repr = u32>> Tally<K, u32> {
    /// Whether nothing was counted.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.raw == 0
    }

    /// The tally one more, held at the top.
    #[must_use]
    pub const fn incremented(self) -> Self {
        Self::new(self.raw.saturating_add(1))
    }

    /// The two tallies together, held at the top.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self::new(self.raw.saturating_add(other.raw))
    }

    /// What is left of this tally past `other`, held at zero.
    #[must_use]
    pub const fn minus(self, other: Self) -> Self {
        Self::new(self.raw.saturating_sub(other.raw))
    }
}

impl<K: TallyKind<Repr = u64>> Tally<K, u64> {
    /// Whether nothing was counted.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.raw == 0
    }

    /// The tally one more, held at the top.
    #[must_use]
    pub const fn incremented(self) -> Self {
        Self::new(self.raw.saturating_add(1))
    }

    /// The two tallies together, held at the top.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self::new(self.raw.saturating_add(other.raw))
    }

    /// What is left of this tally past `other`, held at zero.
    #[must_use]
    pub const fn minus(self, other: Self) -> Self {
        Self::new(self.raw.saturating_sub(other.raw))
    }
}

impl<K: Divisor> Tally<K, u64> {
    /// The count as the divisor of a sum, for an average. A count past 2^53
    /// rounds to the nearest `f64`.
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        self.raw as f64
    }
}

/// The count as a signed 64-bit column holds it. A count past `i64::MAX`
/// saturates, and a negative column, which no `COUNT` returns, reads as
/// zero.
impl<K: TallyColumn> StoredAsI64 for Tally<K, u64> {
    fn stored_count(self) -> u64 {
        self.raw
    }

    fn from_stored_count(count: u64) -> Self {
        Self::new(count)
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: TallyKind> Clone for Tally<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: TallyKind> Copy for Tally<K> {}

impl<K: TallyKind> PartialEq for Tally<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: TallyKind> Eq for Tally<K> {}

impl<K: TallyKind> PartialOrd for Tally<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: TallyKind> Ord for Tally<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: TallyKind> Hash for Tally<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K: TallyKind> Default for Tally<K> {
    fn default() -> Self {
        Self::ZERO
    }
}

/// `NAME(n)`, as the derived `Debug` of the hand-written type printed it.
impl<K: TallyKind> fmt::Debug for Tally<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number.
impl<K: TallyKind> fmt::Display for Tally<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

/// The bare number, at the width of the marker.
impl<K: TallyKind> Serialize for Tally<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl<'de, K: TallyKind> Deserialize<'de> for Tally<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        K::Repr::deserialize(deserializer).map(Self::new)
    }
}

/// How many elements a selection, a board or an import holds.
pub enum Elements {}

impl TallyKind for Elements {
    type Repr = u32;
    const NAME: &'static str = "ElementCount";
}

/// How many cells a selection or a sheet holds of some kind. A sheet's cells
/// run past 32 bits, rows times columns, so the tally is 64 bits wide.
pub enum Cells {}

impl TallyKind for Cells {
    type Repr = u64;
    const NAME: &'static str = "CellCount";
}

/// An aggregate divides a sum of cells by their count.
impl Divisor for Cells {}

#[cfg(test)]
mod tests {
    use super::{Cells, Recounts, Tally, TallyKind};

    /// A test tally.
    enum Seen {}
    impl TallyKind for Seen {
        type Repr = u32;
        const NAME: &'static str = "Seen";
    }

    /// A test tally recounted from [`Seen`].
    enum Kept {}
    impl TallyKind for Kept {
        type Repr = u32;
        const NAME: &'static str = "Kept";
    }
    impl Recounts<Seen> for Kept {}

    #[test]
    fn a_recount_keeps_the_number_and_zero_reads_as_zero() {
        assert_eq!(Tally::<Kept>::recount(Tally::<Seen>::new(9)).get(), 9);
        assert!(Tally::<Kept>::ZERO.is_zero());
        assert!(!Tally::<Kept>::new(1).is_zero());
    }

    #[test]
    fn a_tally_holds_at_the_top_and_crosses_the_wire_bare() {
        assert_eq!(Tally::<Seen>::ZERO.incremented().get(), 1);
        assert_eq!(Tally::<Seen>::new(u32::MAX).incremented().get(), u32::MAX);
        assert_eq!(
            Tally::<Seen>::new(2)
                .plus(Tally::<Seen>::new(3))
                .to_string(),
            "5"
        );
        assert_eq!(
            Tally::<Seen>::new(5).minus(Tally::<Seen>::new(3)),
            Tally::<Seen>::new(2)
        );
        assert_eq!(
            Tally::<Seen>::new(3).minus(Tally::<Seen>::new(5)),
            Tally::<Seen>::ZERO
        );
        assert_eq!(Tally::<Seen>::of_len(usize::MAX).get(), u32::MAX);
        assert_eq!(format!("{:?}", Tally::<Seen>::new(4)), "Seen(4)");
        let bytes = postcard::to_allocvec(&Tally::<Seen>::new(7)).expect("a tally encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec(&7_u32).expect("a number encodes")
        );
    }

    /// The hand-written `CellCount` a `Tally<Cells>` replaced: a
    /// transparent `u64`, as it derived its serde.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct CellCount(u64);

    /// The width of `Cells` is the one of the type it replaced: postcard and
    /// JSON write the bytes `CellCount` wrote, and read them back.
    #[test]
    fn a_wide_tally_writes_the_bytes_of_the_type_it_replaced() {
        for n in [0, 300, u64::from(u32::MAX) + 1, u64::MAX] {
            let tally = Tally::<Cells>::new(n);
            let old = CellCount(n);
            let bytes = postcard::to_allocvec(&tally).expect("a tally encodes");
            assert_eq!(bytes, postcard::to_allocvec(&old).expect("encodes"));
            let back: Tally<Cells> = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, tally);
            let json = serde_json::to_string(&tally).expect("a tally encodes");
            assert_eq!(json, serde_json::to_string(&old).expect("encodes"));
            let back: Tally<Cells> = serde_json::from_str(&json).expect("decodes");
            assert_eq!(back, tally);
        }
        assert_eq!(format!("{:?}", Tally::<Cells>::new(4)), "CellCount(4)");
    }

    #[test]
    fn a_wide_tally_holds_at_its_own_top_and_divides_a_sum() {
        let top = Tally::<Cells>::new(u64::MAX);
        assert_eq!(top.incremented(), top);
        assert_eq!(top.plus(Tally::<Cells>::new(1)), top);
        assert_eq!(Tally::<Cells>::ZERO.minus(top), Tally::<Cells>::ZERO);
        assert!(Tally::<Cells>::ZERO.is_zero());
        assert_eq!(Tally::<Cells>::of_len(7).get(), 7);
        assert_eq!(Tally::<Cells>::saturating_from_u64(u64::MAX), top);
        assert_eq!(Tally::<Seen>::saturating_from_u64(u64::MAX).get(), u32::MAX);
        assert!((Tally::<Cells>::new(4).as_f64() - 4.0).abs() < f64::EPSILON);
    }
}
