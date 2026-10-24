//! A one-based ordinal: the number a reader sees beside an item.
//!
//! An ordinal is not an index. It starts at one, it has no zero, and it is
//! printed rather than used to address a slice. A page, a footnote and an
//! ordered list item each carry one. Each gets its own type, so one cannot
//! stand in for another.
//!
//! The inner value is a `NonZeroU32`. So an `Option` of an ordinal costs no
//! more than the ordinal, and a zero cannot pose as "no ordinal".

/// Defines a one-based ordinal type over `NonZeroU32`.
///
/// The type gets `FIRST`, `get`, a saturating `next`, and a fallible `new`
/// that refuses zero. `from_position` names the item at a 0-based position,
/// and `Display` prints the bare number.
#[macro_export]
macro_rules! define_ordinal {
    ($(#[$meta:meta])* pub struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(::core::num::NonZeroU32);

        impl $name {
            /// The first item's ordinal: one.
            pub const FIRST: Self = Self(::core::num::NonZeroU32::MIN);

            /// The ordinal `value`, `None` for zero, which numbers nothing.
            pub const fn new(value: u32) -> Option<Self> {
                match ::core::num::NonZeroU32::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            /// The number a reader sees.
            pub const fn get(self) -> u32 {
                self.0.get()
            }

            /// The ordinal of the item after this one, saturating at the top.
            #[must_use]
            pub const fn next(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// The ordinal of the item at the 0-based `position`, saturating
            /// at the top.
            pub fn from_position(position: usize) -> Self {
                let position = u32::try_from(position).unwrap_or(u32::MAX);
                Self(::core::num::NonZeroU32::MIN.saturating_add(position))
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    define_ordinal! {
        /// A test ordinal.
        pub struct Nth;
    }

    #[test]
    fn an_ordinal_counts_from_one_and_has_no_zero() {
        assert_eq!(Nth::new(0), None);
        assert_eq!(Nth::FIRST.get(), 1);
        assert_eq!(Nth::from_position(0), Nth::FIRST);
        assert_eq!(Nth::from_position(2).to_string(), "3");
        assert_eq!(Nth::FIRST.next().get(), 2);
        assert_eq!(Nth::from_position(usize::MAX).get(), u32::MAX);
        assert_eq!(Nth::from_position(usize::MAX).next().get(), u32::MAX);
    }
}
