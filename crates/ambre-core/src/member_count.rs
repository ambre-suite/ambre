//! How many members a document has: routing metadata the relay counts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The number of members on a document, as the relay counts them. More than
/// one means the document is shared.
///
/// Transparent to serde, so the wire carries the bare 32-bit count. A store
/// counts in 64 bits; [`TryFrom<i64>`] is the one checked way in from there.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MemberCount(u32);

impl MemberCount {
    /// The count of a document no one shares.
    pub const ONE: Self = Self(1);

    /// A count of `n` members.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }

    /// The count itself.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Whether more than one member holds the document.
    pub const fn is_shared(self) -> bool {
        self.0 > 1
    }
}

/// A store's count that is not one: negative, or past 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("a member count of {0} does not fit 32 bits")]
pub struct MemberCountError(pub i64);

impl TryFrom<i64> for MemberCount {
    type Error = MemberCountError;

    fn try_from(n: i64) -> Result<Self, Self::Error> {
        u32::try_from(n).map(Self).map_err(|_| MemberCountError(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_reads_from_the_store_checked_and_crosses_the_wire_bare() {
        assert_eq!(MemberCount::try_from(3), Ok(MemberCount::new(3)));
        assert_eq!(MemberCount::try_from(-1), Err(MemberCountError(-1)));
        let past = i64::from(u32::MAX) + 1;
        assert_eq!(MemberCount::try_from(past), Err(MemberCountError(past)));
        assert!(MemberCount::new(2).is_shared());
        assert!(!MemberCount::ONE.is_shared());
        assert_eq!(serde_json::to_string(&MemberCount::new(7)).unwrap(), "7");
    }
}
