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
//!     const NAME: &'static str = "AppleCount";
//! }
//! enum Pears {}
//! impl TallyKind for Pears {
//!     const NAME: &'static str = "PearCount";
//! }
//! let _ = Tally::<Apples>::ZERO.plus(Tally::<Pears>::ZERO);
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// The marker of one kind of tally.
///
/// The marker is an uninhabited type: it names what is counted and holds no
/// value. The trait is open, because the tallies live in the crates that
/// count them.
pub trait TallyKind: 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A running count of the things `K` names, over a `u32`.
///
/// Serde sees the bare number, so a record keeps its wire bytes.
pub struct Tally<K: TallyKind> {
    raw: u32,
    kind: PhantomData<fn() -> K>,
}

impl<K: TallyKind> Tally<K> {
    /// Nothing counted.
    pub const ZERO: Self = Self::new(0);

    /// A tally of `n`.
    #[must_use]
    pub const fn new(n: u32) -> Self {
        Self {
            raw: n,
            kind: PhantomData,
        }
    }

    /// The count itself.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.raw
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

    /// A tally of `len` things, held at the top past 32 bits.
    #[must_use]
    pub fn of_len(len: usize) -> Self {
        Self::new(u32::try_from(len).unwrap_or(u32::MAX))
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

impl<K: TallyKind> serde::Serialize for Tally<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.raw)
    }
}

impl<'de, K: TallyKind> serde::Deserialize<'de> for Tally<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <u32 as serde::Deserialize>::deserialize(deserializer).map(Self::new)
    }
}

/// How many elements a selection, a board or an import holds.
pub enum Elements {}

impl TallyKind for Elements {
    const NAME: &'static str = "ElementCount";
}

/// How many cells a selection or a sheet holds of some kind. A sheet's cells
/// run past 32 bits, rows times columns, so the tally is 64 bits wide.
///
/// Transparent to serde: a manifest or a message carries the bare number.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct CellCount(u64);

impl CellCount {
    /// No cell.
    pub const ZERO: Self = Self(0);

    /// A tally of `n` cells.
    pub const fn new(n: u64) -> Self {
        Self(n)
    }

    /// The count itself.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Whether no cell was counted.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// The tally one more, held at the top.
    #[must_use]
    pub const fn incremented(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The count as a divisor of a sum, for an average.
    #[allow(clippy::cast_precision_loss)]
    pub const fn as_f64(self) -> f64 {
        self.0 as f64
    }
}

impl std::fmt::Display for CellCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Tally, TallyKind};

    /// A test tally.
    enum Seen {}
    impl TallyKind for Seen {
        const NAME: &'static str = "Seen";
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
}
