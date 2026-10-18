//! The relay's sequence number of a document.
//!
//! The relay numbers each update of a document as it stores it, from 1 up. A
//! read cursor, a snapshot horizon and the place of a message are all such a
//! number, so they share one type. None of them takes the place of a count.

use serde::{Deserialize, Serialize};

/// A number the relay gave one update of a document. Zero is "before the
/// first update": the start of a read cursor, or no number recorded.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Seq(pub u64);

impl Seq {
    /// Before the first update of a document.
    pub const ZERO: Self = Self(0);

    /// The number just before this one, `ZERO` at the start.
    #[must_use]
    pub const fn before(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::Seq;

    #[test]
    fn the_number_before_the_start_is_the_start() {
        assert_eq!(Seq(5).before(), Seq(4));
        assert_eq!(Seq::ZERO.before(), Seq::ZERO);
    }

    #[test]
    fn a_number_serializes_as_the_bare_integer() {
        assert_eq!(serde_json::to_string(&Seq(42)).unwrap(), "42");
    }
}
