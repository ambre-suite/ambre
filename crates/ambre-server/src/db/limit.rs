//! How many rows one query may return.

/// The most rows one query returns: never zero, never past [`Self::MAX`]
/// (`SRV-075`).
///
/// A request names any number, and [`Self::clamped`] brings it into range
/// instead of refusing it. A pass of the relay names its own as a constant,
/// through [`Self::saturating`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct QueryLimit(u16);

impl QueryLimit {
    /// The smallest page: one row.
    pub const MIN: Self = Self(1);
    /// The cap the relay declares for one page.
    pub const MAX: Self = Self(1000);
    /// The page a request that names no limit is served.
    pub const DEFAULT: Self = Self(100);

    /// `n` brought into range: zero is one, and past the cap is the cap.
    pub const fn saturating(n: u16) -> Self {
        if n < Self::MIN.0 {
            Self::MIN
        } else if n > Self::MAX.0 {
            Self::MAX
        } else {
            Self(n)
        }
    }

    /// The number a request asked for, brought into range as
    /// [`Self::saturating`] does. Zero or below is one row.
    pub fn clamped(n: i64) -> Self {
        if n < 1 {
            return Self::MIN;
        }
        u16::try_from(n).map_or(Self::MAX, Self::saturating)
    }

    /// The count, as the store binds it.
    pub fn as_i64(self) -> i64 {
        i64::from(self.0)
    }

    /// The count, to compare a page against.
    pub fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

/// A request's `limit` reads as any integer, clamped (`SRV-075`). A word
/// that is not an integer is still refused.
impl<'de> serde::Deserialize<'de> for QueryLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <i64 as serde::Deserialize>::deserialize(deserializer).map(Self::clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_limit_is_never_zero_and_never_past_the_cap() {
        for (asked, served) in [
            (i64::MIN, 1),
            (-3, 1),
            (0, 1),
            (1, 1),
            (250, 250),
            (1000, 1000),
            (1001, 1000),
            (70_000, 1000),
            (i64::MAX, 1000),
        ] {
            assert_eq!(QueryLimit::clamped(asked).as_i64(), served, "asked {asked}");
        }
        assert_eq!(QueryLimit::saturating(0), QueryLimit::MIN);
        assert_eq!(QueryLimit::saturating(u16::MAX), QueryLimit::MAX);
        let read: QueryLimit = serde_json::from_str("5000").expect("an integer reads");
        assert_eq!(read, QueryLimit::MAX);
        assert!(serde_json::from_str::<QueryLimit>("\"many\"").is_err());
    }
}
