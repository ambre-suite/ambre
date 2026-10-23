//! Counters that tell a late answer from the current one.
//!
//! A timer and a request both carry the number they were armed or asked
//! under. The answer is live only while that number is the latest one. Each
//! kind of timer counts on its own axis, so each counter is its own type. A
//! replay ticket compared to a resend ticket does not compile, and a listing
//! counts on one width wherever it travels.

/// Defines one counter: it starts at [`ZERO`](Self::ZERO), only grows, and
/// compares to its own kind alone.
macro_rules! define_generation {
    ($($(#[$meta:meta])* $name:ident;)*) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $name(u64);

        impl $name {
            /// The count before the first one armed.
            pub(crate) const ZERO: Self = Self(0);

            /// The one after this one. It saturates, which no session
            /// reaches: one per millisecond takes half a billion years.
            #[must_use]
            pub(crate) const fn next(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// The `n`th one after [`ZERO`](Self::ZERO), for a test that
            /// names a count.
            #[cfg(test)]
            pub(crate) const fn nth(n: u64) -> Self {
                Self(n)
            }
        }
    )*};
}

define_generation! {
    /// The ticket of a replay timer, armed when the relay stopped a replay
    /// (`SYNC-153`).
    ReplayTicket;
    /// The ticket of a resend timer, armed when the relay failed to sequence
    /// a delta (`SYNC-159`).
    ResendTicket;
    /// The number of a listing asked of the relay. Two listings can cross on
    /// the way, and only the answer to the latest one counts (`SYNC-037`).
    RequestGeneration;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_counter_only_grows() {
        let first = ReplayTicket::ZERO.next();
        assert!(first > ReplayTicket::ZERO);
        assert!(first.next() > first);
        assert_eq!(ReplayTicket::default(), ReplayTicket::ZERO);
        assert_eq!(ResendTicket::default(), ResendTicket::ZERO);
        assert_eq!(ReplayTicket::nth(2), first.next());
        assert_eq!(ResendTicket::nth(1), ResendTicket::ZERO.next());
        assert_eq!(RequestGeneration::nth(1), RequestGeneration::ZERO.next());
    }
}
