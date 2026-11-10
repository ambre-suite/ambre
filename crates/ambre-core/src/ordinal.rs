//! A one-based ordinal: the number a reader sees beside an item.
//!
//! An ordinal is not an index. It starts at one, it has no zero, and it is
//! printed rather than used to address a slice. A page, a footnote and an
//! ordered list item each carry one. Each is its own marker, so one cannot
//! stand in for another:
//!
//! ```compile_fail
//! use ambre_core::{Ordinal, OrdinalKind};
//! enum Page {}
//! impl OrdinalKind for Page {
//!     const NAME: &'static str = "PageNumber";
//! }
//! enum Note {}
//! impl OrdinalKind for Note {
//!     const NAME: &'static str = "NoteOrdinal";
//! }
//! let _: Ordinal<Page> = Ordinal::<Note>::FIRST;
//! ```
//!
//! The inner value is a `NonZeroU32`. So an `Option` of an ordinal costs no
//! more than the ordinal, and a zero cannot pose as "no ordinal".

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;

/// The marker of one kind of ordinal.
///
/// The marker is an uninhabited type: it names what is numbered and holds no
/// value. The trait is open, because the ordinals live in the crates that
/// number them.
pub trait OrdinalKind: 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// The one-based number of an item that `K` names.
pub struct Ordinal<K: OrdinalKind> {
    raw: NonZeroU32,
    kind: PhantomData<fn() -> K>,
}

impl<K: OrdinalKind> Ordinal<K> {
    /// The first item's ordinal: one.
    pub const FIRST: Self = Self::at(NonZeroU32::MIN);

    /// The ordinal `raw`, for this module alone.
    const fn at(raw: NonZeroU32) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The ordinal `value`, `None` for zero, which numbers nothing.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self::at(value)),
            None => None,
        }
    }

    /// The number a reader sees.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.raw.get()
    }

    /// The ordinal of the item after this one, saturating at the top.
    #[must_use]
    pub const fn next(self) -> Self {
        Self::at(self.raw.saturating_add(1))
    }

    /// The ordinal of the item at the 0-based `position`, saturating at the
    /// top.
    #[must_use]
    pub fn from_position(position: usize) -> Self {
        let position = u32::try_from(position).unwrap_or(u32::MAX);
        Self::at(NonZeroU32::MIN.saturating_add(position))
    }

    /// The 0-based position of the item this ordinal numbers: the inverse of
    /// [`from_position`](Self::from_position). It never underflows, since an
    /// ordinal has no zero.
    #[must_use]
    pub const fn position(self) -> u32 {
        self.raw.get() - 1
    }
}

impl<K: OrdinalKind + crate::index::Axis> Ordinal<K> {
    /// The position this ordinal numbers, on the axis its marker also
    /// counts: the inverse of [`Pos::number`](crate::index::Pos::number).
    #[must_use]
    pub const fn index(self) -> crate::index::Pos<K> {
        crate::index::Pos::new(self.position())
    }

    /// The ordinal of the item at `pos`, saturating at the top: the inverse
    /// of [`index`](Self::index), and the typed form of
    /// [`from_position`](Self::from_position).
    #[must_use]
    pub const fn of_index(pos: crate::index::Pos<K>) -> Self {
        Self::at(NonZeroU32::MIN.saturating_add(pos.get()))
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: OrdinalKind> Clone for Ordinal<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: OrdinalKind> Copy for Ordinal<K> {}

impl<K: OrdinalKind> PartialEq for Ordinal<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: OrdinalKind> Eq for Ordinal<K> {}

impl<K: OrdinalKind> PartialOrd for Ordinal<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: OrdinalKind> Ord for Ordinal<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: OrdinalKind> Hash for Ordinal<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME(n)`, as the derived `Debug` of the hand-written type printed it.
impl<K: OrdinalKind> fmt::Debug for Ordinal<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number.
impl<K: OrdinalKind> fmt::Display for Ordinal<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Ordinal, OrdinalKind};

    /// A test ordinal.
    enum Nth {}
    impl OrdinalKind for Nth {
        const NAME: &'static str = "Nth";
    }

    #[test]
    fn an_ordinal_counts_from_one_and_has_no_zero() {
        assert_eq!(Ordinal::<Nth>::new(0), None);
        assert_eq!(Ordinal::<Nth>::FIRST.get(), 1);
        assert_eq!(Ordinal::<Nth>::from_position(0), Ordinal::FIRST);
        assert_eq!(Ordinal::<Nth>::from_position(2).to_string(), "3");
        assert_eq!(Ordinal::<Nth>::from_position(2).position(), 2);
        assert_eq!(Ordinal::<Nth>::FIRST.next().get(), 2);
        assert_eq!(Ordinal::<Nth>::from_position(usize::MAX).get(), u32::MAX);
        assert_eq!(
            Ordinal::<Nth>::from_position(usize::MAX).next().get(),
            u32::MAX
        );
        assert_eq!(format!("{:?}", Ordinal::<Nth>::FIRST), "Nth(1)");
    }

    /// A test axis that a reader also numbers.
    #[derive(PartialEq, Eq)]
    enum Shelf {}
    impl OrdinalKind for Shelf {
        const NAME: &'static str = "ShelfNumber";
    }
    impl crate::index::Axis for Shelf {
        const NAME: &'static str = "Shelf";
    }

    #[test]
    fn an_ordinal_maps_back_to_the_position_it_numbers() {
        let third = Ordinal::<Shelf>::from_position(2);
        assert_eq!(third.index(), crate::index::Pos::<Shelf>::new(2));
        assert_eq!(Ordinal::<Shelf>::FIRST.index(), crate::index::Pos::new(0));
    }

    #[test]
    fn a_position_numbers_as_its_bare_count_does_and_saturates_at_the_top() {
        for raw in [0, 1, 2, 41, u32::MAX - 1, u32::MAX] {
            let pos = crate::index::Pos::<Shelf>::new(raw);
            assert_eq!(
                Ordinal::<Shelf>::of_index(pos),
                Ordinal::<Shelf>::from_position(pos.to_usize())
            );
        }
        let third = crate::index::Pos::<Shelf>::new(2);
        assert_eq!(Ordinal::<Shelf>::of_index(third).index(), third);
    }
}

/// The number a reader sees, as `#[serde(transparent)]` writes it.
impl<K: OrdinalKind> Serialize for Ordinal<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.get().serialize(serializer)
    }
}

/// Read through [`Ordinal::new`], so a decoded zero, which numbers nothing,
/// is refused.
impl<'de, K: OrdinalKind> Deserialize<'de> for Ordinal<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u32::deserialize(deserializer)?;
        Self::new(value).ok_or_else(|| serde::de::Error::custom("an ordinal is never zero"))
    }
}
