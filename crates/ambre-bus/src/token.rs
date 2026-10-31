//! Tokens a thread mints one after another, so a reply names the request it
//! answers.
//!
//! A token is never zero. "No token" is therefore an `Option`, and never a
//! reserved value a fresh token could collide with. That is the difference
//! from an `ambre_core::Generation`, whose zero stands before the first one.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU64;

/// The marker of one family of tokens: an uninhabited type that names the
/// family and holds no value.
pub trait TokenKind: 'static {
    /// The name `Debug` prints, as the macro-written type printed its own.
    const NAME: &'static str;
}

/// One token of the family `K`: a non-zero `u64`.
///
/// The type has a first token and a successor, and nothing else: a token is
/// compared, never computed with. It prints as its number, for a log line.
pub struct Token<K: TokenKind> {
    raw: NonZeroU64,
    kind: PhantomData<fn() -> K>,
}

impl<K: TokenKind> Token<K> {
    /// The first token a thread mints.
    pub const FIRST: Self = Self {
        raw: NonZeroU64::MIN,
        kind: PhantomData,
    };

    /// The token after this one. It saturates at the last token, which a
    /// thread minting one per gesture never reaches.
    #[must_use]
    pub const fn next(self) -> Self {
        Self {
            raw: self.raw.saturating_add(1),
            kind: PhantomData,
        }
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: TokenKind> Clone for Token<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: TokenKind> Copy for Token<K> {}

impl<K: TokenKind> PartialEq for Token<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: TokenKind> Eq for Token<K> {}

impl<K: TokenKind> Hash for Token<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

/// `NAME(n)`, as the macro-written token printed itself.
impl<K: TokenKind> fmt::Debug for Token<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number, for a log line.
impl<K: TokenKind> fmt::Display for Token<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::{Token, TokenKind};

    /// A family for the test.
    enum Probe {}
    impl TokenKind for Probe {
        const NAME: &'static str = "Probe";
    }

    #[test]
    fn a_token_counts_up_and_saturates() {
        assert_ne!(Token::<Probe>::FIRST, Token::FIRST.next());
        assert_eq!(Token::<Probe>::FIRST.next().to_string(), "2");
        assert_eq!(format!("{:?}", Token::<Probe>::FIRST), "Probe(1)");
        let last = Token::<Probe> {
            raw: std::num::NonZeroU64::MAX,
            kind: std::marker::PhantomData,
        };
        assert_eq!(last.next(), last);
    }
}
