//! Tokens a thread mints one after another, so a reply names the request it
//! answers.
//!
//! A token is never zero. "No token" is therefore an `Option`, and never a
//! reserved value a fresh token could collide with.

/// Defines a token newtype over a non-zero `u64`.
///
/// The type has a first token and a successor, and nothing else: a token is
/// compared, never computed with. It prints as its number, for a log line.
macro_rules! define_token {
    ($(#[$meta:meta])* pub struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(std::num::NonZeroU64);

        impl $name {
            /// The first token a thread mints.
            pub const FIRST: Self = Self(std::num::NonZeroU64::MIN);

            /// The token after this one. It saturates at the last token,
            /// which a thread minting one per gesture never reaches.
            #[must_use]
            pub const fn next(self) -> Self {
                Self(self.0.saturating_add(1))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

pub(crate) use define_token;

#[cfg(test)]
mod tests {
    define_token! {
        /// A token for the test.
        pub struct Probe;
    }

    #[test]
    fn a_token_counts_up_and_saturates() {
        assert_ne!(Probe::FIRST, Probe::FIRST.next());
        assert_eq!(Probe::FIRST.next().to_string(), "2");
        let last = Probe(std::num::NonZeroU64::MAX);
        assert_eq!(last.next(), last);
    }
}
