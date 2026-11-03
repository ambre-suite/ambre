//! How many members a document has: routing metadata the relay counts.

use thiserror::Error;

use crate::tally::{Tally, TallyKind};

/// The members on a document, as the relay counts them. More than one means
/// the document is shared.
///
/// The wire carries the bare 32-bit count. A store counts in 64 bits;
/// [`TryFrom<i64>`] is the one checked way in from there.
pub enum Members {}

impl TallyKind for Members {
    type Repr = u32;
    const NAME: &'static str = "MemberCount";
}

impl Tally<Members> {
    /// The count of a document no one shares.
    pub const ONE: Self = Self::new(1);

    /// Whether more than one member holds the document.
    #[must_use]
    pub const fn is_shared(self) -> bool {
        self.get() > 1
    }
}

/// A store's count that is not one: negative, or past 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("a member count of {0} does not fit 32 bits")]
pub struct MemberCountError(i64);

impl MemberCountError {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for Tally<Members> {
    type Error = MemberCountError;

    fn try_from(n: i64) -> Result<Self, Self::Error> {
        u32::try_from(n)
            .map(Self::new)
            .map_err(|_| MemberCountError(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_reads_from_the_store_checked_and_crosses_the_wire_bare() {
        assert_eq!(Tally::<Members>::try_from(3), Ok(Tally::<Members>::new(3)));
        assert_eq!(Tally::<Members>::try_from(-1), Err(MemberCountError(-1)));
        let past = i64::from(u32::MAX) + 1;
        assert_eq!(
            Tally::<Members>::try_from(past),
            Err(MemberCountError(past))
        );
        assert!(Tally::<Members>::new(2).is_shared());
        assert!(!Tally::<Members>::ONE.is_shared());
        assert_eq!(
            serde_json::to_string(&Tally::<Members>::new(7)).unwrap(),
            "7"
        );
        assert_eq!(format!("{:?}", Tally::<Members>::new(7)), "MemberCount(7)");
    }
}
