//! Spans of time, as the relay's JSON counts them.
//!
//! The relay counts a lease in milliseconds and a token's life in seconds.
//! A bare integer lets one reach a place that expects the other, a factor
//! of a thousand off. One type per unit keeps them apart. Each is
//! `#[serde(transparent)]`, so it reads and writes the bare integer.

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

#[cfg(test)]
mod tests {
    use super::{Millis, Seconds};
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
}
