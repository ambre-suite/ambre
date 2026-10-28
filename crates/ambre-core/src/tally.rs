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
        )]
        pub struct $name(u32);

        // Serde through core's own paths, so a crate that expands the macro
        // needs no serde dependency of its own.
        $crate::define_index!(@serde $name, u32);

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

            /// What is left of this tally past `other`, held at zero.
            #[must_use]
            pub const fn minus(self, other: Self) -> Self {
                Self(self.0.saturating_sub(other.0))
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

define_tally! {
    /// How many elements a selection, a board or an import holds.
    pub struct ElementCount;
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
    define_tally! {
        /// A test tally.
        pub struct Seen;
    }

    #[test]
    fn a_tally_holds_at_the_top_and_crosses_the_wire_bare() {
        assert_eq!(Seen::ZERO.incremented().get(), 1);
        assert_eq!(Seen::new(u32::MAX).incremented().get(), u32::MAX);
        assert_eq!(Seen::new(2).plus(Seen::new(3)).to_string(), "5");
        assert_eq!(Seen::new(5).minus(Seen::new(3)), Seen::new(2));
        assert_eq!(Seen::new(3).minus(Seen::new(5)), Seen::ZERO);
        assert_eq!(Seen::of_len(usize::MAX).get(), u32::MAX);
        let bytes = postcard::to_allocvec(&Seen::new(7)).expect("a tally encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec(&7_u32).expect("a number encodes")
        );
    }
}
