//! Counters that tell a late answer from the current one.
//!
//! A timer, a request or a cache key carries the number it was armed, asked
//! or built under. The answer is live only while that number is the latest
//! one. Each kind counts on its own axis, so each counter is its own type.
//! A ticket of one kind compared to a ticket of another does not compile.

/// Defines one counter per item: it starts at `ZERO`, only grows, and
/// compares to its own kind alone.
///
/// Each item names its visibility, so a crate keeps a counter of its own
/// private. `nth` builds the `n`th counter for a test that names a count.
#[macro_export]
macro_rules! define_generation {
    ($($(#[$meta:meta])* $vis:vis struct $name:ident;)*) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name(u64);

        impl $name {
            /// The count before the first one armed.
            pub const ZERO: Self = Self(0);

            /// The one after this one. It saturates, which no session
            /// reaches: one per millisecond takes half a billion years.
            #[must_use]
            pub const fn next(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// The `n`th one after [`ZERO`](Self::ZERO), for a test that
            /// names a count.
            #[cfg(test)]
            pub const fn nth(n: u64) -> Self {
                Self(n)
            }
        }
    )*};
}

#[cfg(test)]
mod tests {
    define_generation! {
        /// A test counter.
        struct Tick;
        /// Another test counter.
        struct Tock;
    }

    #[test]
    fn a_counter_only_grows() {
        let first = Tick::ZERO.next();
        assert!(first > Tick::ZERO);
        assert!(first.next() > first);
        assert_eq!(Tick::default(), Tick::ZERO);
        assert_eq!(Tick::nth(2), first.next());
        assert_eq!(Tock::nth(1), Tock::ZERO.next());
        assert_eq!(Tick::nth(u64::MAX).next(), Tick::nth(u64::MAX));
    }
}
