//! A small set of named bits: the modifier keys of a chord, the lazy scripts
//! a text asked for.
//!
//! Every such set is one generic type, [`Flags<K>`], over a marker `K` that
//! names what the bits stand for. The marker's crate names each member as a
//! constant of type `Flags<K>`, so the set operations are written once here.
//! A set of modifier keys never mixes with a set of scripts:
//!
//! ```compile_fail,E0308
//! use ambre_core::{Flags, FlagsKind};
//! #[derive(PartialEq, Eq)]
//! enum Keys {}
//! impl FlagsKind for Keys {
//!     type Repr = u8;
//!     const NAME: &'static str = "Modifiers";
//!     const ALL: u8 = 0b111;
//! }
//! #[derive(PartialEq, Eq)]
//! enum Scripts {}
//! impl FlagsKind for Scripts {
//!     type Repr = u8;
//!     const NAME: &'static str = "ScriptSet";
//!     const ALL: u8 = 0b1111;
//! }
//! let _ = Flags::<Keys>::EMPTY.with(Flags::<Scripts>::EMPTY);
//! ```
//!
//! Every operation is a `const fn`, so a `const` table builds a set. The
//! family has no serde form: no set crosses a wire or a store today.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

mod sealed {
    /// Closes [`super::FlagsRepr`]: a set is a `u8`.
    pub trait Sealed {}
}

/// The integer a set holds its bits in. Sealed: a `u8`, the one width a set
/// needs today. A wider set adds a width here and an `impl` block for it.
pub trait FlagsRepr: sealed::Sealed + Copy + Ord + Hash + fmt::Debug + 'static {}

impl sealed::Sealed for u8 {}
impl FlagsRepr for u8 {}

/// The marker of one kind of bit set.
///
/// The marker is an uninhabited type: it names what the bits stand for and
/// holds no value. The trait is open, because each set lives in the crate
/// that names its members. The marker derives `PartialEq` and `Eq`, so a
/// member constant stands in a pattern.
pub trait FlagsKind: 'static {
    /// The integer the bits are held in.
    type Repr: FlagsRepr;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;

    /// Every bit that names a member. A set holds no bit outside it: the
    /// doors refuse one, and no set operation makes one.
    const ALL: Self::Repr;
}

/// A set of the bits `K` names.
///
/// `PartialEq` and `Eq` are derived, not written by hand. A handler matches
/// a set against the member constants of its crate, and only a derived
/// equality lets a constant stand in a pattern. The derive bounds the
/// marker, so each marker derives the same two traits. Every other trait is
/// written by hand.
#[derive(PartialEq, Eq)]
pub struct Flags<K: FlagsKind> {
    bits: <K as FlagsKind>::Repr,
    kind: PhantomData<fn() -> K>,
}

impl<K: FlagsKind<Repr = u8>> Flags<K> {
    /// The set that holds no member.
    pub const EMPTY: Self = Self::admitted(0);

    /// The set that holds every member, [`FlagsKind::ALL`].
    pub const ALL: Self = Self::admitted(K::ALL);

    /// The set whose bits are `BITS`, checked against [`FlagsKind::ALL`]
    /// when the program is compiled. The marker's crate names each member
    /// through this door, one constant per bit:
    ///
    /// ```
    /// use ambre_core::{Flags, FlagsKind};
    /// #[derive(PartialEq, Eq)]
    /// enum Keys {}
    /// impl FlagsKind for Keys {
    ///     type Repr = u8;
    ///     const NAME: &'static str = "Modifiers";
    ///     const ALL: u8 = 0b11;
    /// }
    /// const SHIFT: Flags<Keys> = Flags::of::<0b10>();
    /// assert_eq!(SHIFT.get(), 2);
    /// ```
    ///
    /// A bit outside the set does not compile:
    ///
    /// ```compile_fail,E0080
    /// use ambre_core::{Flags, FlagsKind};
    /// #[derive(PartialEq, Eq)]
    /// enum Keys {}
    /// impl FlagsKind for Keys {
    ///     type Repr = u8;
    ///     const NAME: &'static str = "Modifiers";
    ///     const ALL: u8 = 0b11;
    /// }
    /// const META: Flags<Keys> = Flags::of::<0b100>();
    /// ```
    ///
    /// # Panics
    ///
    /// Never at run time. The check is a `const` assertion, so a bit outside
    /// the set is a compile error.
    #[must_use]
    pub const fn of<const BITS: u8>() -> Self {
        const { assert!(BITS & !K::ALL == 0, "a bit outside FlagsKind::ALL") };
        Self::admitted(BITS)
    }

    /// The set whose bits are `bits`, or `None` when one lies outside
    /// [`FlagsKind::ALL`]: the door for bits read at run time.
    #[must_use]
    pub const fn new(bits: u8) -> Option<Self> {
        if bits & !K::ALL == 0 {
            Some(Self::admitted(bits))
        } else {
            None
        }
    }

    /// The set of `bits`, which a caller has checked against
    /// [`FlagsKind::ALL`] or built from two sets that hold.
    const fn admitted(bits: u8) -> Self {
        Self {
            bits,
            kind: PhantomData,
        }
    }

    /// The bits of the set.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.bits
    }

    /// Whether the set holds no member.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// The union of two sets.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self::admitted(self.bits | other.bits)
    }

    /// The set with each member of `other` removed.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self::admitted(self.bits & !other.bits)
    }

    /// Whether the set holds every member of `other`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    /// Adds each member of `other` to the set.
    pub const fn insert(&mut self, other: Self) {
        self.bits |= other.bits;
    }
}

impl<K: FlagsKind> Clone for Flags<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: FlagsKind> Copy for Flags<K> {}

impl<K: FlagsKind + Eq> PartialOrd for Flags<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The order of the bits read as an integer, so a sorted list of sets is
/// the same on every peer.
impl<K: FlagsKind + Eq> Ord for Flags<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bits.cmp(&other.bits)
    }
}

impl<K: FlagsKind> Hash for Flags<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits.hash(state);
    }
}

/// The empty set.
impl<K: FlagsKind<Repr = u8>> Default for Flags<K> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// `NAME(bits)`, as the derived `Debug` of the hand-written type printed it.
impl<K: FlagsKind> fmt::Debug for Flags<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.bits).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Flags, FlagsKind};

    #[derive(PartialEq, Eq)]
    enum Keys {}
    impl FlagsKind for Keys {
        type Repr = u8;
        const NAME: &'static str = "Modifiers";
        const ALL: u8 = 0b11;
    }

    const A: Flags<Keys> = Flags::of::<1>();
    const B: Flags<Keys> = Flags::of::<{ 1 << 1 }>();
    const AB: Flags<Keys> = A.with(B);

    #[test]
    fn the_run_time_door_refuses_a_bit_outside_all() {
        assert_eq!(Flags::<Keys>::new(0b11), Some(AB));
        assert_eq!(Flags::<Keys>::new(0b100), None);
        assert_eq!(Flags::<Keys>::ALL, AB);
        assert_eq!(Flags::<Keys>::ALL.without(A), B);
    }

    #[test]
    fn the_set_operations_hold_in_a_const_table() {
        assert_eq!(AB.get(), 3);
        assert!(AB.contains(A) && AB.contains(B) && AB.contains(Flags::EMPTY));
        assert!(!A.contains(AB));
        assert_eq!(AB.without(A), B);
        assert!(Flags::<Keys>::EMPTY.is_empty() && !A.is_empty());
        assert_eq!(Flags::<Keys>::default(), Flags::EMPTY);
        let mut set = A;
        set.insert(B);
        assert_eq!(set, AB);
        assert!(A < B);
    }

    #[test]
    fn a_member_constant_stands_in_a_pattern() {
        assert!(matches!(A.with(B), AB));
        assert!(!matches!(A, Flags::<Keys>::EMPTY | B));
    }

    #[test]
    fn debug_prints_the_name_and_the_bits() {
        assert_eq!(format!("{AB:?}"), "Modifiers(3)");
    }
}
