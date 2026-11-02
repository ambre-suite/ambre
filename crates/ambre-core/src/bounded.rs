//! A number held between two bounds.
//!
//! Every bounded number is one generic type, [`Bounded<K>`], over a marker
//! `K` that names its quantity. The marker carries the two bounds as
//! constants, since a float cannot be a const generic on stable. An opacity
//! never stands in for a fraction of a side, though both lie in `0..=1`:
//!
//! ```compile_fail
//! use ambre_core::{Bounded, Opacity, UnitFraction};
//! let _: Bounded<Opacity> = Bounded::<UnitFraction>::MAX;
//! ```
//!
//! A value enters through [`Bounded::new`] or [`Bounded::try_new`] alone. A
//! decode goes through the same check. A value outside the bounds, NaN
//! included, is refused with an [`OutOfRange`]. The wire form is the bare
//! number.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::space::Board;
use crate::unit::Px;

mod sealed {
    /// Closes [`super::BoundedRepr`]: a bounded number is an `f32`, a `u8` or
    /// a `u16`.
    pub trait Sealed {}
}

/// The number a bounded value holds. Sealed: an `f32`, a `u8` or a `u16`.
pub trait BoundedRepr:
    sealed::Sealed
    + Copy
    + PartialOrd
    + fmt::Debug
    + fmt::Display
    + Serialize
    + DeserializeOwned
    + 'static
{
}

impl sealed::Sealed for f32 {}
impl BoundedRepr for f32 {}
impl sealed::Sealed for u8 {}
impl BoundedRepr for u8 {}
impl sealed::Sealed for u16 {}
impl BoundedRepr for u16 {}

/// The words a refusal prints around the refused value, so each quantity
/// keeps the message a person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    before: &'static str,
    after: &'static str,
}

impl Refusal {
    /// The words `before` and `after` the refused value.
    #[must_use]
    pub const fn new(before: &'static str, after: &'static str) -> Self {
        Self { before, after }
    }
}

/// The marker of one bounded quantity.
///
/// The marker is an uninhabited type: it names the quantity and holds no
/// value. The trait is open, because a quantity may live in the crate that
/// reads it.
pub trait Bounds: 'static {
    /// The number a value of the quantity holds.
    type Repr: BoundedRepr;
    /// The lowest value, admitted.
    const MIN: Self::Repr;
    /// The highest value, admitted.
    const MAX: Self::Repr;
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
    /// The words a refused value prints with.
    const REFUSAL: Refusal;
}

/// A quantity whose value without a choice is not its lowest one.
pub trait Defaulted: Bounds + Sized {
    /// The value `Default` gives.
    const DEFAULT: Bounded<Self>;
}

/// A number of the quantity `K`, from `K::MIN` to `K::MAX`.
pub struct Bounded<K: Bounds> {
    raw: K::Repr,
    kind: PhantomData<fn() -> K>,
}

impl<K: Bounds> Bounded<K> {
    /// The lowest value.
    pub const MIN: Self = Self::at(K::MIN);

    /// The highest value.
    pub const MAX: Self = Self::at(K::MAX);

    /// The value `raw`, unchecked: the module of each marker alone names a
    /// constant through it.
    pub(crate) const fn at(raw: K::Repr) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The value `raw`, or `None` outside the bounds. A NaN lies outside.
    #[must_use]
    pub fn new(raw: K::Repr) -> Option<Self> {
        (K::MIN <= raw && raw <= K::MAX).then_some(Self::at(raw))
    }

    /// The value `raw`.
    ///
    /// # Errors
    ///
    /// [`OutOfRange`] outside the bounds, carrying `raw`.
    pub fn try_new(raw: K::Repr) -> Result<Self, OutOfRange<K>> {
        Self::new(raw).ok_or(OutOfRange { raw })
    }

    /// The number itself.
    #[must_use]
    pub const fn get(self) -> K::Repr {
        self.raw
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: Bounds> Clone for Bounded<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Bounds> Copy for Bounded<K> {}

impl<K: Bounds> PartialEq for Bounded<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

/// Total for an integer. A float is never NaN here, since every constructor
/// refuses it, but the representation alone decides.
impl<K: Bounds> Eq for Bounded<K> where K::Repr: Eq {}

impl<K: Bounds> PartialOrd for Bounded<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.raw.partial_cmp(&other.raw)
    }
}

impl<K: Bounds> Ord for Bounded<K>
where
    K::Repr: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: Bounds> Hash for Bounded<K>
where
    K::Repr: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K: Defaulted> Default for Bounded<K> {
    fn default() -> Self {
        K::DEFAULT
    }
}

/// `NAME(n)`, as the derived `Debug` of the hand-written type printed it.
impl<K: Bounds> fmt::Debug for Bounded<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number, without a unit.
impl<K: Bounds> fmt::Display for Bounded<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

impl<K: Bounds> Serialize for Bounded<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

/// The bare number, refused outside the bounds with the message of
/// [`OutOfRange`].
impl<'de, K: Bounds> Deserialize<'de> for Bounded<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = K::Repr::deserialize(deserializer)?;
        Self::try_new(raw).map_err(serde::de::Error::custom)
    }
}

/// The error of a number outside the bounds of the quantity `K`.
pub struct OutOfRange<K: Bounds> {
    raw: K::Repr,
}

impl<K: Bounds> OutOfRange<K> {
    /// The refused number, as the error echoes it.
    #[must_use]
    pub const fn refused(&self) -> K::Repr {
        self.raw
    }
}

impl<K: Bounds> Clone for OutOfRange<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Bounds> Copy for OutOfRange<K> {}

impl<K: Bounds> PartialEq for OutOfRange<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: Bounds> fmt::Debug for OutOfRange<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutOfRange")
            .field("quantity", &K::NAME)
            .field("refused", &self.raw)
            .finish()
    }
}

/// The words of the quantity around the refused number.
impl<K: Bounds> fmt::Display for OutOfRange<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", K::REFUSAL.before, self.raw, K::REFUSAL.after)
    }
}

impl<K: Bounds> std::error::Error for OutOfRange<K> {}

/// A fraction from 0 to 1: of a box's side, from its top or left edge to the
/// other, or of a pen's full pressure. Anything else, NaN included, is
/// refused.
pub enum UnitFraction {}

impl Bounds for UnitFraction {
    type Repr = f32;
    const MIN: f32 = 0.0;
    const MAX: f32 = 1.0;
    const NAME: &'static str = "UnitFraction";
    const REFUSAL: Refusal = Refusal::new("a fraction of a side lies in 0..=1, not ", "");
}

impl Bounded<UnitFraction> {
    /// The middle of the side.
    pub const HALF: Self = Self::at(0.5);

    /// The whole of it: the far edge of a side, a full pressure.
    pub const ONE: Self = Self::MAX;

    /// The point this fraction of the way from `start` to `end`, on a board.
    #[must_use]
    pub fn along(self, start: Px<Board>, end: Px<Board>) -> Px<Board> {
        start + (end - start) * f64::from(self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::{Bounded, Bounds, OutOfRange, Refusal, UnitFraction};

    /// A test quantity with the refused value first in its message.
    enum Grade {}
    impl Bounds for Grade {
        type Repr = u8;
        const MIN: u8 = 1;
        const MAX: u8 = 5;
        const NAME: &'static str = "Grade";
        const REFUSAL: Refusal = Refusal::new("", " is no grade from 1 to 5");
    }

    #[test]
    fn a_bounded_value_admits_both_bounds_and_refuses_past_them() {
        assert_eq!(Bounded::<Grade>::new(1), Some(Bounded::MIN));
        assert_eq!(Bounded::<Grade>::new(5), Some(Bounded::MAX));
        assert_eq!(Bounded::<Grade>::new(0), None);
        assert_eq!(
            Bounded::<Grade>::try_new(6).map_err(|refused| refused.to_string()),
            Err("6 is no grade from 1 to 5".to_owned())
        );
        assert_eq!(Bounded::<UnitFraction>::new(f32::NAN), None);
        assert_eq!(
            format!("{:?}", Bounded::<UnitFraction>::HALF),
            "UnitFraction(0.5)"
        );
    }

    #[test]
    fn a_decode_goes_through_the_check_and_keeps_the_bare_number() {
        let half = serde_json::to_string(&Bounded::<UnitFraction>::HALF).expect("encode");
        assert_eq!(half, "0.5");
        assert_eq!(
            serde_json::from_str::<Bounded<UnitFraction>>(&half).ok(),
            Some(Bounded::HALF)
        );
        let refused = serde_json::from_str::<Bounded<UnitFraction>>("1.5")
            .expect_err("past the bound")
            .to_string();
        assert!(refused.starts_with("a fraction of a side lies in 0..=1, not 1.5"));
        let error: OutOfRange<Grade> = Bounded::<Grade>::try_new(9).expect_err("past");
        assert_eq!(error.refused(), 9);
    }
}
