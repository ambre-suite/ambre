//! How many members a document has: routing metadata the relay counts.

use std::fmt;

use crate::count::{Count, CountKind};
use crate::refused::{Refused, Refuses};

/// The members on a document, as the relay counts them. More than one means
/// the document is shared.
///
/// The wire carries the bare 32-bit count. A store counts in 64 bits;
/// [`TryFrom<i64>`] is the one checked way in from there.
pub enum Members {}

impl CountKind for Members {
    type Repr = u32;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("MemberCount");
}

impl Count<Members> {
    /// The count of a document no one shares.
    pub const ONE: Self = Self::new(1);

    /// Whether more than one member holds the document.
    #[must_use]
    pub const fn is_shared(self) -> bool {
        self.get() > 1
    }
}

/// A store's count that is not one: negative, or past 32 bits. The error
/// echoes the store's `i64`.
impl Refuses for Members {
    type Raw = i64;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("MemberCountError");

    fn message(raw: &Self::Raw, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a member count of {raw} does not fit 32 bits")
    }
}

impl TryFrom<i64> for Count<Members> {
    type Error = Refused<Members>;

    fn try_from(n: i64) -> Result<Self, Self::Error> {
        u32::try_from(n).map(Self::new).map_err(|_| Refused::new(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_reads_from_the_store_checked_and_crosses_the_wire_bare() {
        assert_eq!(Count::<Members>::try_from(3), Ok(Count::<Members>::new(3)));
        assert_eq!(Count::<Members>::try_from(-1), Err(Refused::new(-1)));
        let past = i64::from(u32::MAX) + 1;
        let refused = Count::<Members>::try_from(past).expect_err("past 32 bits");
        assert_eq!(refused.get(), past);
        assert_eq!(
            refused.to_string(),
            "a member count of 4294967296 does not fit 32 bits"
        );
        assert_eq!(format!("{refused:?}"), "MemberCountError(4294967296)");
        assert!(Count::<Members>::new(2).is_shared());
        assert!(!Count::<Members>::ONE.is_shared());
        assert_eq!(
            serde_json::to_string(&Count::<Members>::new(7)).unwrap(),
            "7"
        );
        assert_eq!(format!("{:?}", Count::<Members>::new(7)), "MemberCount(7)");
    }
}
