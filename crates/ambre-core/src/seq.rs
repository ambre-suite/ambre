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

    /// The number as a signed 64-bit column holds it. A number past
    /// `i64::MAX` saturates: the relay counts far below it.
    #[must_use]
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }

    /// The number just after this one: the next update the relay would number.
    #[must_use]
    pub const fn after(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// A signed 64-bit column read back. A negative value is no relay's number.
impl std::fmt::Display for Seq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<i64> for Seq {
    type Error = std::num::TryFromIntError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u64::try_from(value).map(Self)
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
    fn a_signed_column_round_trips_and_refuses_a_negative() {
        assert_eq!(Seq::try_from(9_i64), Ok(Seq(9)));
        assert!(Seq::try_from(-1_i64).is_err());
        assert_eq!(Seq(9).to_i64_saturating(), 9);
        assert_eq!(Seq(u64::MAX).to_i64_saturating(), i64::MAX);
    }

    #[test]
    fn the_number_after_a_number_is_the_next_one() {
        assert_eq!(Seq(5).after(), Seq(6));
        assert_eq!(Seq(u64::MAX).after(), Seq(u64::MAX));
    }

    #[test]
    fn a_number_serializes_as_the_bare_integer() {
        assert_eq!(serde_json::to_string(&Seq(42)).unwrap(), "42");
    }
}
