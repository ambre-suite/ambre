//! Spans and instants of time, as the relay's JSON counts them.
//!
//! The relay counts a lease in milliseconds and a token's life in seconds.
//! A bare integer lets one reach a place that expects the other, a factor
//! of a thousand off. One type per unit keeps them apart, and an instant is
//! kept apart from a span. Each is `#[serde(transparent)]`, so it reads and
//! writes the bare integer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A span of time in whole milliseconds.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Millis(pub u64);

impl Millis {
    /// The span `duration` covers, saturating past `u64::MAX` milliseconds.
    pub fn from_duration(duration: Duration) -> Self {
        Self(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
    }

    /// The span, as the standard library counts it.
    pub const fn to_duration(self) -> Duration {
        Duration::from_millis(self.0)
    }
}

/// A span of time in whole seconds.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Seconds(pub u64);

impl Seconds {
    /// The span, as the standard library counts it.
    pub const fn to_duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

/// An instant in whole seconds since the Unix epoch: the clock of a token's
/// `iat` and `exp`, and of a delete's second sweep.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixSeconds(pub u64);

impl UnixSeconds {
    /// The instant `span` after this one, saturating at `u64::MAX`.
    #[must_use]
    pub const fn plus(self, span: Seconds) -> Self {
        Self(self.0.saturating_add(span.0))
    }

    /// The span from `earlier` to this instant, zero when `earlier` is later.
    pub const fn since(self, earlier: Self) -> Seconds {
        Seconds(self.0.saturating_sub(earlier.0))
    }

    /// The instant as a Postgres `BIGINT` stores it, saturating at `i64::MAX`.
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.0).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{Millis, Seconds, UnixSeconds};
    use std::time::Duration;

    #[test]
    fn a_span_serializes_as_the_bare_integer_of_its_unit() {
        assert_eq!(serde_json::to_string(&Millis(1500)).unwrap(), "1500");
        assert_eq!(serde_json::to_string(&Seconds(900)).unwrap(), "900");
    }

    #[test]
    fn a_span_converts_to_the_same_duration() {
        assert_eq!(Millis(1500).to_duration(), Duration::from_millis(1500));
        assert_eq!(Seconds(2).to_duration(), Duration::from_secs(2));
        assert_eq!(Millis::from_duration(Duration::from_secs(3)), Millis(3000));
    }

    #[test]
    fn an_instant_moves_by_a_span_and_never_goes_below_zero() {
        assert_eq!(UnixSeconds(100).plus(Seconds(20)), UnixSeconds(120));
        assert_eq!(UnixSeconds(100).since(UnixSeconds(40)), Seconds(60));
        assert_eq!(UnixSeconds(40).since(UnixSeconds(100)), Seconds(0));
        assert_eq!(UnixSeconds(u64::MAX).to_i64_saturating(), i64::MAX);
    }
}
