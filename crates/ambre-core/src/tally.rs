//! A tally: a running count of one kind of thing, held at the top rather
//! than wrapping.
//!
//! A tally is not an index. It addresses nothing, it only goes up, and it is
//! read or printed. A record that keeps several tallies side by side gives
//! each its own type, so one cannot be added into another.

/// Defines one tally type per item, each over a `u32`.
///
/// Each type gets `ZERO`, `new`, `get`, a saturating `incremented` and
/// `plus`, a saturating `of_len` from a `usize`, and `Display` as the bare
/// number. Serde sees the bare number, so a record keeps its wire bytes.
#[macro_export]
macro_rules! define_tally {
    ($($(#[$meta:meta])* pub struct $name:ident;)+) => {$(
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
            ::serde::Serialize, ::serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// Nothing counted.
            pub const ZERO: Self = Self(0);

            /// A tally of `n`.
            pub const fn new(n: u32) -> Self {
                Self(n)
            }

            /// The count itself.
            pub const fn get(self) -> u32 {
                self.0
            }

            /// The tally one more, held at the top.
            #[must_use]
            pub const fn incremented(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// The two tallies together, held at the top.
            #[must_use]
            pub const fn plus(self, other: Self) -> Self {
                Self(self.0.saturating_add(other.0))
            }

            /// A tally of `len` things, held at the top past 32 bits.
            pub fn of_len(len: usize) -> Self {
                Self(u32::try_from(len).unwrap_or(u32::MAX))
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    )+};
}

#[cfg(test)]
mod tests {
    define_tally! {
        /// A test tally.
        pub struct Seen;
    }

    #[test]
    fn a_tally_holds_at_the_top_and_crosses_the_wire_bare() {
        assert_eq!(Seen::ZERO.incremented().get(), 1);
        assert_eq!(Seen::new(u32::MAX).incremented().get(), u32::MAX);
        assert_eq!(Seen::new(2).plus(Seen::new(3)).to_string(), "5");
        assert_eq!(Seen::of_len(usize::MAX).get(), u32::MAX);
        let bytes = postcard::to_allocvec(&Seen::new(7)).expect("a tally encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec(&7_u32).expect("a number encodes")
        );
    }
}
