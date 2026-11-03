//! A series number: the numbers one issuer hands out going up, each once.
//!
//! Every series is one generic type, [`Seq<K>`], over a marker `K` that names
//! the issuer. The relay's number of an update never stands in for an
//! author's clock:
//!
//! ```compile_fail,E0308
//! use ambre_core::seq::{Relay, Seq, SeqKind};
//! enum AuthorKey {}
//! impl SeqKind for AuthorKey {
//!     const NAME: &'static str = "Clock";
//! }
//! let _: Seq<AuthorKey> = Seq::<Relay>::ZERO;
//! ```
//!
//! The marker carries what differs between two series:
//! * [`SeqKind::NAME`], the name `Debug` prints;
//! * [`CountsSince`], for a series whose gap between two numbers is a tally;
//! * [`SeqColumn`], for a series a store keeps in a signed 64-bit column.
//!
//! A number is a `u64`, and its wire form is the bare number: `Seq<K>` is
//! transparent to serde.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::tally::{Tally, TallyColumn, TallyKind};

/// The marker of one issuer of a series.
///
/// The marker is an uninhabited type: it names the issuer and holds no
/// value. The trait is open, because a series may live in the crate that
/// issues it.
pub trait SeqKind: 'static {
    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A series whose gap between two numbers counts the things it numbered, as
/// a tally of [`CountsSince::Tally`].
///
/// A capability that each series opts in to. A series that numbers nothing
/// countable has no `since`:
///
/// ```compile_fail,E0599
/// use ambre_core::seq::{Seq, SeqKind};
/// enum Pings {}
/// impl SeqKind for Pings {
///     const NAME: &'static str = "Ping";
/// }
/// let _ = Seq::<Pings>::new(3).since(Seq::<Pings>::ZERO);
/// ```
pub trait CountsSince: SeqKind {
    /// The kind of tally a gap counts.
    type Tally: TallyKind;
}

/// A series a store keeps in a signed 64-bit column.
///
/// A capability that each series opts in to. Such a number writes itself
/// with [`Seq::to_i64_saturating`], and a column reads back through
/// `TryFrom<i64>`, which refuses a negative value rather than clamping it.
/// A series no store keeps has neither:
///
/// ```compile_fail,E0277
/// use ambre_core::seq::{Seq, SeqKind};
/// enum Pings {}
/// impl SeqKind for Pings {
///     const NAME: &'static str = "Ping";
/// }
/// let _ = Seq::<Pings>::try_from(3_i64);
/// ```
pub trait SeqColumn: SeqKind {}

/// One number of the series `K`. Zero is "before the first number": the
/// start of a cursor, or no number recorded.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Seq<K: SeqKind> {
    raw: u64,
    #[serde(skip)]
    kind: PhantomData<fn() -> K>,
}

impl<K: SeqKind> Seq<K> {
    /// Before the first number of the series.
    pub const ZERO: Self = Self::new(0);

    /// Wraps a raw value. The constructor checks nothing.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self {
            raw: value,
            kind: PhantomData,
        }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.raw
    }

    /// The number just before this one, `ZERO` at the start.
    #[must_use]
    pub const fn before(self) -> Self {
        Self::new(self.raw.saturating_sub(1))
    }

    /// The number just after this one: the next one the issuer hands out,
    /// held at the top.
    #[must_use]
    pub const fn after(self) -> Self {
        Self::new(self.raw.saturating_add(1))
    }
}

impl<K: CountsSince> Seq<K> {
    /// The things the issuer numbered after `earlier`, up to this number.
    /// Zero when this number is not past `earlier`, and held at the top of
    /// the tally's width.
    #[must_use]
    pub fn since(self, earlier: Self) -> Tally<K::Tally> {
        Tally::<K::Tally>::saturating_from_u64(self.raw.saturating_sub(earlier.raw))
    }
}

impl<K: SeqColumn> Seq<K> {
    /// The number as a signed 64-bit column holds it. A number past
    /// `i64::MAX` saturates: an issuer counts far below it.
    #[must_use]
    pub fn to_i64_saturating(self) -> i64 {
        i64::try_from(self.raw).unwrap_or(i64::MAX)
    }
}

/// A signed 64-bit column read back. A negative value is no issuer's number:
/// it is refused, never clamped.
impl<K: SeqColumn> TryFrom<i64> for Seq<K> {
    type Error = std::num::TryFromIntError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u64::try_from(value).map(Self::new)
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: SeqKind> Clone for Seq<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: SeqKind> Copy for Seq<K> {}

impl<K: SeqKind> PartialEq for Seq<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: SeqKind> Eq for Seq<K> {}

impl<K: SeqKind> PartialOrd for Seq<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: SeqKind> Ord for Seq<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: SeqKind> Hash for Seq<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K: SeqKind> Default for Seq<K> {
    fn default() -> Self {
        Self::ZERO
    }
}

/// `NAME(n)`, as the derived `Debug` of the hand-written type printed it.
impl<K: SeqKind> fmt::Debug for Seq<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

/// The bare number.
impl<K: SeqKind> fmt::Display for Seq<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

/// The relay's sequencer: it numbers each update of a document as it stores
/// it, from 1 up. A read cursor, a snapshot horizon and the place of a
/// message are all such a number. None of them takes the place of a count.
pub enum Relay {}

impl SeqKind for Relay {
    const NAME: &'static str = "Seq";
}

/// The gap between two of the relay's numbers counts the deltas it stored.
impl CountsSince for Relay {
    type Tally = Deltas;
}

/// The relay stores its numbers in Postgres.
impl SeqColumn for Relay {}

/// A number of deltas of one document: the rows of its inline tail, or the
/// updates the relay numbered past a snapshot. 64 bits wide, as the relay's
/// numbers are.
pub enum Deltas {}

impl TallyKind for Deltas {
    type Repr = u64;
    const NAME: &'static str = "DeltaCount";
}

/// The relay reads a `COUNT` of rows from Postgres.
impl TallyColumn for Deltas {}

#[cfg(test)]
mod tests {
    use super::{Deltas, Relay, Seq, SeqKind};
    use crate::span::StoredAsI64;
    use crate::tally::Tally;

    fn seq(n: u64) -> Seq<Relay> {
        Seq::<Relay>::new(n)
    }

    fn deltas(n: u64) -> Tally<Deltas> {
        Tally::<Deltas>::new(n)
    }

    /// The history past a horizon counts the deltas after it, and a horizon
    /// ahead of the number reads as no history rather than wrapping.
    #[test]
    fn the_deltas_since_a_number_saturate_at_zero() {
        assert_eq!(seq(532).since(seq(32)), deltas(500));
        assert_eq!(seq(7).since(seq(7)), deltas(0));
        assert_eq!(seq(3).since(seq(9)), deltas(0));
    }

    #[test]
    fn the_number_before_the_start_is_the_start() {
        assert_eq!(seq(5).before(), seq(4));
        assert_eq!(Seq::<Relay>::ZERO.before(), Seq::<Relay>::ZERO);
    }

    #[test]
    fn a_signed_column_round_trips_and_refuses_a_negative() {
        assert_eq!(Seq::<Relay>::try_from(9_i64), Ok(seq(9)));
        assert!(Seq::<Relay>::try_from(-1_i64).is_err());
        assert_eq!(seq(9).to_i64_saturating(), 9);
        assert_eq!(seq(u64::MAX).to_i64_saturating(), i64::MAX);
    }

    /// A delta count reads a `COUNT` back: a negative one, which no `COUNT`
    /// returns, reads as zero, and a count past `i64::MAX` saturates.
    #[test]
    fn a_delta_count_round_trips_through_a_signed_column() {
        assert_eq!(Tally::<Deltas>::from_i64(12), deltas(12));
        assert_eq!(Tally::<Deltas>::from_i64(-3), deltas(0));
        assert_eq!(deltas(u64::MAX).to_i64_saturating(), i64::MAX);
        assert_eq!(format!("{:?}", deltas(2)), "DeltaCount(2)");
    }

    #[test]
    fn the_number_after_a_number_is_the_next_one() {
        assert_eq!(seq(5).after(), seq(6));
        assert_eq!(seq(u64::MAX).after(), seq(u64::MAX));
    }

    /// The hand-written `Seq` the generic replaced, as it derived its serde.
    #[derive(serde::Serialize)]
    #[serde(transparent)]
    struct OldSeq(u64);

    /// The hand-written `Clock` and `Counter` derived their serde without
    /// `transparent`: serde wrote them as a newtype struct.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OldClock(u64);

    /// A test issuer, for the shape the generic writes.
    enum Issuer {}
    impl SeqKind for Issuer {
        const NAME: &'static str = "Clock";
    }

    /// Postcard and JSON write a newtype struct as its inside, so the
    /// transparent generic writes the bytes each hand-written type wrote,
    /// and reads them back.
    #[test]
    fn a_number_writes_the_bytes_of_the_type_it_replaced() {
        for n in [0, 1, 300, u64::MAX] {
            let new = Seq::<Issuer>::new(n);
            let bytes = postcard::to_allocvec(&new).expect("a number encodes");
            assert_eq!(bytes, postcard::to_allocvec(&OldClock(n)).expect("encodes"));
            assert_eq!(bytes, postcard::to_allocvec(&OldSeq(n)).expect("encodes"));
            let back: Seq<Issuer> = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, new);
            let json = serde_json::to_string(&new).expect("a number encodes");
            assert_eq!(json, serde_json::to_string(&OldClock(n)).expect("encodes"));
            assert_eq!(json, serde_json::to_string(&OldSeq(n)).expect("encodes"));
            let back: Seq<Issuer> = serde_json::from_str(&json).expect("decodes");
            assert_eq!(back, new);
            let old: OldClock = serde_json::from_str(&json).expect("decodes");
            assert_eq!(old.0, n);
        }
        assert_eq!(serde_json::to_string(&seq(42)).expect("encodes"), "42");
        assert_eq!(format!("{:?}", Seq::<Issuer>::new(3)), "Clock(3)");
        assert_eq!(format!("{:?}", seq(3)), "Seq(3)");
    }
}
