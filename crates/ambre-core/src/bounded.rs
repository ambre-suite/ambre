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
    /// Closes [`super::BoundedRepr`]: a bounded number is an `f32`, an `f64`,
    /// a `u8`, a `u16`, an `i16` or a `u32`.
    pub trait Sealed {}
}

/// The number a bounded value holds. Sealed: an `f32`, an `f64`, a `u8`, a
/// `u16`, an `i16` or a `u32`. A float refuses NaN, which lies outside every
/// pair of bounds, since it compares false with each.
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
impl sealed::Sealed for f64 {}
impl BoundedRepr for f64 {}
impl sealed::Sealed for u8 {}
impl BoundedRepr for u8 {}
impl sealed::Sealed for u16 {}
impl BoundedRepr for u16 {}
impl sealed::Sealed for i16 {}
impl BoundedRepr for i16 {}
impl sealed::Sealed for u32 {}
impl BoundedRepr for u32 {}

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
    const NAME: crate::StaticKey<crate::DebugNames>;
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

/// A quantity that serves the nearest value instead of a refusal. The
/// capability is opt-in: a quantity with a rule refuses a value outside its
/// bounds. Only a quantity whose callers may name anything takes it, such as
/// the page size of a request. A weight without it refuses instead:
///
/// ```compile_fail
/// use ambre_core::{Bounded, FontWeight};
/// let _ = Bounded::<FontWeight>::saturating_from(5000);
/// ```
pub trait Saturates: Bounds {}

impl<K: Saturates> Bounded<K>
where
    K::Repr: Ord + TryFrom<i64> + Into<i64>,
{
    /// The value nearest `wide`: below the lowest is the lowest, past the
    /// highest is the highest.
    #[must_use]
    pub fn saturating_from(wide: i64) -> Self {
        if wide <= K::MIN.into() {
            return Self::MIN;
        }
        K::Repr::try_from(wide)
            .ok()
            .filter(|raw| *raw <= K::MAX)
            .map_or(Self::MAX, Self::at)
    }
}

/// The `const` door of a quantity outside this crate, which cannot call
/// [`Bounded::at`]. A bound check in a `const` block compares a concrete
/// integer only. Rust refuses two inherent impls that differ by an
/// associated type alone (rust-lang/rust#20400). So each representation
/// that carries a door names it apart: `of` for `u32`, `of_u8` for `u8`.
impl<K: Bounds<Repr = u32>> Bounded<K> {
    /// The value `N`, checked while the crate compiles: a constant outside
    /// the bounds fails the build. A `const` item names a value through it.
    ///
    /// ```compile_fail
    /// use ambre_core::{Bounded, Bounds, Refusal};
    /// enum Page {}
    /// impl Bounds for Page {
    ///     type Repr = u32;
    ///     const MIN: u32 = 1;
    ///     const MAX: u32 = 1000;
    ///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Page");
    ///     const REFUSAL: Refusal = Refusal::new("", "");
    /// }
    /// const NONE: Bounded<Page> = Bounded::<Page>::of::<0>();
    /// ```
    ///
    /// # Panics
    ///
    /// Never at run time: the check runs in a `const` block, so a value
    /// outside the bounds is a compile error.
    #[must_use]
    pub const fn of<const N: u32>() -> Self {
        const {
            assert!(K::MIN <= N && N <= K::MAX, "a constant outside its bounds");
        }
        Self::at(N)
    }
}

/// The `const` door of a quantity over a `u8`, as [`Bounded::of`] is for a
/// `u32`.
impl<K: Bounds<Repr = u8>> Bounded<K> {
    /// The value `N`, checked while the crate compiles: a constant outside
    /// the bounds fails the build.
    ///
    /// ```compile_fail
    /// use ambre_core::{Bounded, Bounds, Refusal};
    /// enum Digits {}
    /// impl Bounds for Digits {
    ///     type Repr = u8;
    ///     const MIN: u8 = 0;
    ///     const MAX: u8 = 15;
    ///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Digits");
    ///     const REFUSAL: Refusal = Refusal::new("", "");
    /// }
    /// const TOO_MANY: Bounded<Digits> = Bounded::<Digits>::of_u8::<16>();
    /// ```
    ///
    /// The same constant inside the bounds compiles:
    ///
    /// ```
    /// use ambre_core::{Bounded, Bounds, Refusal};
    /// enum Digits {}
    /// impl Bounds for Digits {
    ///     type Repr = u8;
    ///     const MIN: u8 = 0;
    ///     const MAX: u8 = 15;
    ///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Digits");
    ///     const REFUSAL: Refusal = Refusal::new("", "");
    /// }
    /// const TWO: Bounded<Digits> = Bounded::<Digits>::of_u8::<2>();
    /// assert_eq!(TWO.get(), 2);
    /// ```
    ///
    /// # Panics
    ///
    /// Never at run time: the check runs in a `const` block, so a value
    /// outside the bounds is a compile error.
    #[must_use]
    pub const fn of_u8<const N: u8>() -> Self {
        const {
            assert!(K::MIN <= N && N <= K::MAX, "a constant outside its bounds");
        }
        Self::at(N)
    }
}

/// The `const` door of a quantity over a `u16`, as [`Bounded::of`] is for a
/// `u32`.
impl<K: Bounds<Repr = u16>> Bounded<K> {
    /// The value `N`, checked while the crate compiles: a constant outside
    /// the bounds fails the build.
    ///
    /// ```compile_fail
    /// use ambre_core::{Bounded, Bounds, Refusal};
    /// enum Code {}
    /// impl Bounds for Code {
    ///     type Repr = u16;
    ///     const MIN: u16 = 1000;
    ///     const MAX: u16 = 4999;
    ///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Code");
    ///     const REFUSAL: Refusal = Refusal::new("", "");
    /// }
    /// const LOW: Bounded<Code> = Bounded::<Code>::of_u16::<999>();
    /// ```
    ///
    /// # Panics
    ///
    /// Never at run time: the check runs in a `const` block, so a value
    /// outside the bounds is a compile error.
    #[must_use]
    pub const fn of_u16<const N: u16>() -> Self {
        const {
            assert!(K::MIN <= N && N <= K::MAX, "a constant outside its bounds");
        }
        Self::at(N)
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
        f.debug_tuple(K::NAME.as_str()).field(&self.raw).finish()
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
            .field("quantity", &K::NAME.as_str())
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
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("UnitFraction");
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
    use super::{Bounded, Bounds, OutOfRange, Refusal, Saturates, UnitFraction};

    /// A test quantity with the refused value first in its message.
    enum Grade {}
    impl Bounds for Grade {
        type Repr = u8;
        const MIN: u8 = 1;
        const MAX: u8 = 5;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Grade");
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

    /// A test quantity of 32 bits, for the `const` door, that saturates.
    enum Page {}
    impl Bounds for Page {
        type Repr = u32;
        const MIN: u32 = 1;
        const MAX: u32 = 1000;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Page");
        const REFUSAL: Refusal = Refusal::new("", " is no page size");
    }
    impl Saturates for Page {}

    /// A test quantity of a signed 16-bit count, that saturates.
    enum Shares {}
    impl Bounds for Shares {
        type Repr = i16;
        const MIN: i16 = 1;
        const MAX: i16 = i16::MAX;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Shares");
        const REFUSAL: Refusal = Refusal::new("", " is no share count");
    }
    impl Saturates for Shares {}

    /// A test quantity of 16 bits, that saturates.
    enum Weight {}
    impl Bounds for Weight {
        type Repr = u16;
        const MIN: u16 = 100;
        const MAX: u16 = 900;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Weight");
        const REFUSAL: Refusal = Refusal::new("", " is no weight");
    }
    impl Saturates for Weight {}

    impl Saturates for Grade {}

    #[test]
    fn a_const_door_checks_its_value_while_the_crate_compiles() {
        const INSIDE: Bounded<Page> = Bounded::<Page>::of::<250>();
        const LOWEST: Bounded<Page> = Bounded::<Page>::of::<1>();
        const HIGHEST: Bounded<Page> = Bounded::<Page>::of::<1000>();
        assert_eq!(INSIDE.get(), 250);
        assert_eq!(LOWEST, Bounded::MIN);
        assert_eq!(HIGHEST, Bounded::MAX);
    }

    #[test]
    fn a_saturating_door_serves_the_nearest_value_and_never_breaks_a_bound() {
        for (wide, served) in [
            (i64::MIN, 1),
            (-3, 1),
            (0, 1),
            (1, 1),
            (1000, 1000),
            (1001, 1000),
            (i64::from(u32::MAX) + 1, 1000),
            (i64::MAX, 1000),
        ] {
            assert_eq!(
                Bounded::<Page>::saturating_from(wide).get(),
                served,
                "{wide}"
            );
        }
        for (wide, served) in [
            (i64::MIN, 1),
            (i64::from(i16::MIN), 1),
            (-7, 1),
            (12, 12),
            (i64::from(i16::MAX), i16::MAX),
            (i64::from(i16::MAX) + 1, i16::MAX),
            (i64::MAX, i16::MAX),
        ] {
            assert_eq!(
                Bounded::<Shares>::saturating_from(wide).get(),
                served,
                "{wide}"
            );
        }
        for (wide, served) in [(-1, 100), (99, 100), (400, 400), (901, 900), (70_000, 900)] {
            assert_eq!(
                Bounded::<Weight>::saturating_from(wide).get(),
                served,
                "{wide}"
            );
        }
        for (wide, served) in [(-1, 1), (0, 1), (3, 3), (6, 5), (256, 5), (i64::MAX, 5)] {
            assert_eq!(
                Bounded::<Grade>::saturating_from(wide).get(),
                served,
                "{wide}"
            );
        }
    }

    /// A test quantity of an `f64` phase.
    enum Share {}
    impl Bounds for Share {
        type Repr = f64;
        const MIN: f64 = 0.0;
        const MAX: f64 = 1.0;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Share");
        const REFUSAL: Refusal = Refusal::new("", " is no share");
    }

    /// A test quantity of a 16-bit code, for the `const` door.
    enum Code {}
    impl Bounds for Code {
        type Repr = u16;
        const MIN: u16 = 1000;
        const MAX: u16 = 4999;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Code");
        const REFUSAL: Refusal = Refusal::new("", " is no code");
    }

    #[test]
    fn a_double_refuses_nan_and_keeps_the_bytes_of_its_number() {
        assert_eq!(Bounded::<Share>::new(f64::NAN), None);
        assert_eq!(Bounded::<Share>::new(1.5), None);
        let half = Bounded::<Share>::new(0.5).expect("in range");
        let bytes = postcard::to_allocvec(&half).expect("encode");
        assert_eq!(bytes, postcard::to_allocvec(&0.5_f64).expect("encode"));
        assert_eq!(
            postcard::from_bytes::<Bounded<Share>>(&bytes).ok(),
            Some(half)
        );
        let nan = postcard::to_allocvec(&f64::NAN).expect("encode");
        assert!(postcard::from_bytes::<Bounded<Share>>(&nan).is_err());
        assert_eq!(serde_json::to_string(&half).expect("encode"), "0.5");
        assert_eq!(format!("{half:?}"), "Share(0.5)");
    }

    #[test]
    fn a_16_bit_const_door_checks_its_value_while_the_crate_compiles() {
        const LOWEST: Bounded<Code> = Bounded::<Code>::of_u16::<1000>();
        const INSIDE: Bounded<Code> = Bounded::<Code>::of_u16::<4401>();
        assert_eq!(LOWEST, Bounded::MIN);
        assert_eq!(INSIDE.get(), 4401);
    }

    #[test]
    fn a_signed_count_keeps_its_bare_number_and_refuses_below_its_bound() {
        let three = Bounded::<Shares>::new(3).expect("in range");
        assert_eq!(serde_json::to_string(&three).expect("encode"), "3");
        assert_eq!(
            serde_json::from_str::<Bounded<Shares>>("3").ok(),
            Some(three)
        );
        assert!(serde_json::from_str::<Bounded<Shares>>("0").is_err());
        assert!(serde_json::from_str::<Bounded<Shares>>("-1").is_err());
        assert!(serde_json::from_str::<Bounded<Shares>>("40000").is_err());
        assert_eq!(format!("{three:?}"), "Shares(3)");
    }
}
