//! Sequences: the numbers one issuer hands out going up, each once.
//!
//! A timer, a request or a cache key carries the number it was armed, asked
//! or built under. The answer is live only while that number is the latest
//! one. The relay numbers each update of a document, an author numbers
//! its own deltas, and a probe numbers the animation frames. Each family
//! counts on its own axis, so each family is its own marker type, and a
//! [`Seq`] of one family never compares to another:
//!
//! ```compile_fail,E0308
//! use ambre_core::{Relay, Seq, SeqKind};
//! enum Author {}
//! impl SeqKind for Author {
//!     type IssueRight = ();
//!     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Clock");
//! }
//! let _: Seq<Author> = Seq::<Relay>::ZERO;
//! ```
//!
//! An [`Issuer`] is the one source of a family. It opens on the family's
//! [`SeqKind::IssueRight`]. A family whose right is `()` is open, and any
//! crate may count it on with [`Seq::next`]. A sealed family has a right with
//! a private constructor. Only the module of the right opens an issuer, so no
//! other crate forges a number of it.
//!
//! The marker carries what differs between two families:
//! * [`SeqKind::NAME`], the name `Debug` prints;
//! * [`Wire`], for a family whose number crosses a wire or a store;
//! * [`Before`], for a family that steps back to the number before;
//! * [`CountsSince`], for a family whose gap between two numbers is a count;
//! * [`SeqColumn`], for a family a store keeps in a signed 64-bit column.

use std::cmp::Ordering as CmpOrdering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::count::{Count, CountColumn, CountKind};

/// The marker of one family of sequence numbers.
///
/// The marker is an uninhabited type: it names the family and holds no
/// value. The trait is open, because the families live in the crates that
/// count them.
///
/// A sealed family keeps its right private, and its module opens the issuer:
///
/// ```
/// mod seal {
///     use ambre_core::{Issuer, SeqKind};
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl SeqKind for Ticket {
///         type IssueRight = Right;
///         const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ticket");
///     }
///     pub fn issuer() -> Issuer<Ticket> {
///         Issuer::new(Right(()))
///     }
/// }
/// let mut issuer = seal::issuer();
/// assert!(issuer.issue() > ambre_core::Seq::ZERO);
/// ```
///
/// Outside that module, the right cannot be built:
///
/// ```compile_fail,E0603
/// mod seal {
///     use ambre_core::SeqKind;
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl SeqKind for Ticket {
///         type IssueRight = Right;
///         const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ticket");
///     }
/// }
/// let _ = ambre_core::Issuer::<seal::Ticket>::new(seal::Right(()));
/// ```
///
/// And a sealed number has no successor of its own:
///
/// ```compile_fail,E0599
/// mod seal {
///     use ambre_core::SeqKind;
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl SeqKind for Ticket {
///         type IssueRight = Right;
///         const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ticket");
///     }
/// }
/// let _ = ambre_core::Seq::<seal::Ticket>::ZERO.next();
/// ```
pub trait SeqKind: 'static {
    /// What opens an [`Issuer`] of the family: `()` for an open family, or
    /// a type with a private constructor for a sealed one. It is `Copy`, so
    /// a `const` constructor can take it.
    type IssueRight: Copy;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: crate::StaticKey<crate::DebugNames>;
}

/// A family whose number crosses a wire or a store.
///
/// The number is then read back from bytes another party wrote, so the
/// family cannot be sealed: the bound asks for an open right. Only such a
/// family has a serde form, and the [`Seq::new`] and [`Seq::get`] doors.
pub trait Wire: SeqKind<IssueRight = ()> {}

/// A family whose number steps back to the one before, for the last number
/// a reader holds once it has the next one.
///
/// A capability that each family opts in to:
///
/// ```compile_fail,E0599
/// use ambre_core::{Seq, SeqKind};
/// enum Pings {}
/// impl SeqKind for Pings {
///     type IssueRight = ();
///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ping");
/// }
/// let _ = Seq::<Pings>::ZERO.before();
/// ```
pub trait Before: SeqKind {}

/// A family whose gap between two numbers counts the things it numbered, as
/// a count of [`CountsSince::Count`].
///
/// A capability that each family opts in to. A family that numbers nothing
/// countable has no `since`:
///
/// ```compile_fail,E0599
/// use ambre_core::{Seq, SeqKind};
/// enum Pings {}
/// impl SeqKind for Pings {
///     type IssueRight = ();
///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ping");
/// }
/// let _ = Seq::<Pings>::ZERO.since(Seq::<Pings>::ZERO);
/// ```
pub trait CountsSince: SeqKind {
    /// The kind of count a gap counts.
    type Count: CountKind;
}

/// A wire family a store keeps in a signed 64-bit column.
///
/// A capability that each family opts in to. Such a number writes itself
/// with [`Seq::to_i64_saturating`], and a column reads back through
/// `TryFrom<i64>`, which refuses a negative value rather than clamping it.
/// A family no store keeps has neither:
///
/// ```compile_fail,E0277
/// use ambre_core::{Seq, SeqKind, Wire};
/// enum Pings {}
/// impl SeqKind for Pings {
///     type IssueRight = ();
///     const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Ping");
/// }
/// impl Wire for Pings {}
/// let _ = Seq::<Pings>::try_from(3_i64);
/// ```
pub trait SeqColumn: Wire {}

/// One number of the family `K`. It starts at [`ZERO`](Self::ZERO), only
/// grows, and compares to its own family alone.
///
/// Its serde form, for a [`Wire`] family, is the bare `u64`.
pub struct Seq<K: SeqKind> {
    raw: u64,
    kind: PhantomData<fn() -> K>,
}

impl<K: SeqKind> Seq<K> {
    /// The number before the first one issued. No issuer hands it out, so
    /// it never matches an answer to a real request.
    pub const ZERO: Self = Self::at(0);

    /// The number `raw`, for this module alone.
    const fn at(raw: u64) -> Self {
        Self {
            raw,
            kind: PhantomData,
        }
    }

    /// The one after this one. It saturates, which no session reaches: one
    /// per millisecond takes half a billion years.
    const fn successor(self) -> Self {
        Self::at(self.raw.saturating_add(1))
    }

    /// The `n`th one after [`ZERO`](Self::ZERO), for a test that names a
    /// count. The `testing` feature reaches the tests of other crates, and
    /// a check of the manifests keeps it out of every other build.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn nth(n: u64) -> Self {
        Self::at(n)
    }
}

impl<K: SeqKind<IssueRight = ()>> Seq<K> {
    /// The one after this one, for an open family. A sealed family has no
    /// such method: its issuer alone counts on.
    #[must_use]
    pub const fn next(self) -> Self {
        self.successor()
    }
}

impl<K: Wire> Seq<K> {
    /// The number `raw`, as a wire, a store row or a peer wrote it. The
    /// constructor checks nothing: every `u64` is a number of the family.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self::at(raw)
    }

    /// The number, as a wire or a store row keeps it.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.raw
    }
}

impl<K: Before> Seq<K> {
    /// The number just before this one, [`ZERO`](Self::ZERO) at the start.
    #[must_use]
    pub const fn before(self) -> Self {
        Self::at(self.raw.saturating_sub(1))
    }
}

impl<K: CountsSince> Seq<K> {
    /// The things the family numbered after `earlier`, up to this number.
    /// Zero when this number is not past `earlier`, and held at the top of
    /// the count's width.
    #[must_use]
    pub fn since(self, earlier: Self) -> Count<K::Count> {
        Count::<K::Count>::saturating_from_u64(self.raw.saturating_sub(earlier.raw))
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
        u64::try_from(value).map(Self::at)
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
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl<K: SeqKind> Ord for Seq<K> {
    fn cmp(&self, other: &Self) -> CmpOrdering {
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

/// `NAME(n)`, as the hand-written counter printed itself.
impl<K: SeqKind> fmt::Debug for Seq<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME.as_str()).field(&self.raw).finish()
    }
}

/// The bare number.
impl<K: SeqKind> fmt::Display for Seq<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.raw, f)
    }
}

impl<K: Wire> serde::Serialize for Seq<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.raw)
    }
}

impl<'de, K: Wire> serde::Deserialize<'de> for Seq<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <u64 as serde::Deserialize>::deserialize(deserializer).map(Self::at)
    }
}

/// The one source of the family `K`, held by one owner.
///
/// It is not `Clone`: two copies would hand out one number twice.
pub struct Issuer<K: SeqKind> {
    latest: Seq<K>,
}

impl<K: SeqKind> Issuer<K> {
    /// An issuer that has handed out nothing yet. Only a holder of the
    /// family's right opens one.
    #[must_use]
    pub const fn new(_right: K::IssueRight) -> Self {
        Self { latest: Seq::ZERO }
    }

    /// A number past every number this issuer handed out.
    #[must_use = "a number issued and dropped is wasted"]
    pub const fn issue(&mut self) -> Seq<K> {
        self.latest = self.latest.successor();
        self.latest
    }

    /// The last number handed out, or [`Seq::ZERO`] before the first. An
    /// answer is live while it carries this number.
    #[must_use]
    pub const fn latest(&self) -> Seq<K> {
        self.latest
    }
}

impl<K: SeqKind<IssueRight = ()>> Default for Issuer<K> {
    fn default() -> Self {
        Self::new(())
    }
}

impl<K: SeqKind> fmt::Debug for Issuer<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Issuer")
            .field("latest", &self.latest)
            .finish()
    }
}

/// The one source of the family `K`, shared by the threads of one memory.
///
/// It lives in a `static`, so its constructor is `const`. A Web Worker has
/// its own memory, and so its own static: the family decides which instance
/// may issue (`BUS-004` for a service request).
pub struct SharedIssuer<K: SeqKind> {
    latest: AtomicU64,
    kind: PhantomData<fn() -> K>,
}

impl<K: SeqKind> SharedIssuer<K> {
    /// An issuer that has handed out nothing yet.
    #[must_use]
    pub const fn new(_right: K::IssueRight) -> Self {
        Self {
            latest: AtomicU64::new(0),
            kind: PhantomData,
        }
    }

    /// A number past every number this issuer handed out, to any thread.
    #[must_use = "a number issued and dropped is wasted"]
    pub fn issue(&self) -> Seq<K> {
        // Relaxed: the number orders nothing but itself. The closure never
        // refuses, so both arms carry the number before the update.
        let before = match self
            .latest
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |raw| {
                Some(raw.saturating_add(1))
            }) {
            Ok(raw) | Err(raw) => raw,
        };
        Seq::at(before).successor()
    }
}

impl<K: SeqKind> fmt::Debug for SharedIssuer<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedIssuer").finish_non_exhaustive()
    }
}

/// The relay's sequencer: it numbers each update of a document as it stores
/// it, from 1 up. A read cursor, a snapshot horizon and the place of a
/// message are all such a number, and zero is "before the first update".
/// None of them takes the place of a count.
pub enum Relay {}

impl SeqKind for Relay {
    type IssueRight = ();
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Seq");
}

/// The relay's numbers cross the wire to every client.
impl Wire for Relay {}

/// A reader holds the number before the next one it waits for.
impl Before for Relay {}

/// The gap between two of the relay's numbers counts the deltas it stored.
impl CountsSince for Relay {
    type Count = Deltas;
}

/// The relay stores its numbers in Postgres.
impl SeqColumn for Relay {}

/// A number of deltas of one document: the rows of its inline tail, or the
/// updates the relay numbered past a snapshot. 64 bits wide, as the relay's
/// numbers are.
pub enum Deltas {}

impl CountKind for Deltas {
    type Repr = u64;
    const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("DeltaCount");
}

/// The relay reads a `COUNT` of rows from Postgres.
impl CountColumn for Deltas {}

#[cfg(test)]
mod tests {
    use super::{Deltas, Issuer, Relay, Seq, SeqKind, SharedIssuer, Wire};
    use crate::count::Count;
    use crate::span::StoredAsI64;

    /// An open test family.
    enum Tick {}
    impl SeqKind for Tick {
        type IssueRight = ();
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Tick");
    }
    impl Wire for Tick {}

    /// A sealed test family: only this module builds its right.
    enum Ticket {}
    #[derive(Clone, Copy)]
    struct TicketRight(());
    impl SeqKind for Ticket {
        type IssueRight = TicketRight;
        const NAME: crate::StaticKey<crate::DebugNames> = crate::StaticKey::new("Ticket");
    }

    fn seq(n: u64) -> Seq<Relay> {
        Seq::<Relay>::new(n)
    }

    fn deltas(n: u64) -> Count<Deltas> {
        Count::<Deltas>::new(n)
    }

    #[test]
    fn a_counter_only_grows_and_saturates() {
        let first = Seq::<Tick>::ZERO.next();
        assert!(first > Seq::ZERO);
        assert!(first.next() > first);
        assert_eq!(Seq::<Tick>::default(), Seq::ZERO);
        assert_eq!(Seq::<Tick>::nth(2), first.next());
        let last = Seq::<Tick>::nth(u64::MAX);
        assert_eq!(last.next(), last);
    }

    #[test]
    fn an_issuer_hands_out_each_number_once_and_never_zero() {
        let mut issuer = Issuer::<Ticket>::new(TicketRight(()));
        assert_eq!(issuer.latest(), Seq::ZERO);
        let (one, two) = (issuer.issue(), issuer.issue());
        assert!(Seq::ZERO < one && one < two);
        assert_eq!(issuer.latest(), two);
        assert_eq!(issuer.issue(), Seq::nth(3));
    }

    #[test]
    fn a_shared_issuer_counts_like_an_issuer() {
        static SHARED: SharedIssuer<Tick> = SharedIssuer::new(());
        let one = SHARED.issue();
        assert_eq!(one, Seq::nth(1));
        assert_eq!(SHARED.issue(), one.next());
    }

    #[test]
    fn debug_prints_the_family_name() {
        assert_eq!(format!("{:?}", Seq::<Ticket>::nth(3)), "Ticket(3)");
        assert_eq!(format!("{:?}", seq(3)), "Seq(3)");
        assert_eq!(seq(3).to_string(), "3");
    }

    #[test]
    fn a_wire_family_round_trips_as_its_bare_number() {
        let tick = Seq::<Tick>::nth(300);
        let json = serde_json::to_string(&tick).expect("serialize");
        assert_eq!(json, "300");
        assert_eq!(serde_json::from_str::<Seq<Tick>>(&json).ok(), Some(tick));
        let bytes = postcard::to_allocvec(&tick).expect("serialize");
        assert_eq!(bytes, postcard::to_allocvec(&300_u64).expect("serialize"));
        assert_eq!(postcard::from_bytes::<Seq<Tick>>(&bytes).ok(), Some(tick));
        assert_eq!(Seq::<Tick>::new(tick.get()), tick);
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
        assert_eq!(seq(5).next(), seq(6));
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
        assert_eq!(Count::<Deltas>::from_i64(12), deltas(12));
        assert_eq!(Count::<Deltas>::from_i64(-3), deltas(0));
        assert_eq!(deltas(u64::MAX).to_i64_saturating(), i64::MAX);
        assert_eq!(format!("{:?}", deltas(2)), "DeltaCount(2)");
    }

    /// The hand-written `Seq` the generic replaced, as it derived its serde.
    #[derive(serde::Serialize)]
    #[serde(transparent)]
    struct OldSeq(u64);

    /// The hand-written `Clock` and `Counter` derived their serde without
    /// `transparent`: serde wrote them as a newtype struct.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OldClock(u64);

    /// Postcard and JSON write a newtype struct as its inside. So the bare
    /// number the generic writes is the bytes each hand-written type wrote,
    /// and each reads the other's bytes back.
    #[test]
    fn a_number_writes_the_bytes_of_the_type_it_replaced() {
        for n in [0, 1, 300, u64::MAX] {
            let new = Seq::<Tick>::new(n);
            let bytes = postcard::to_allocvec(&new).expect("a number encodes");
            assert_eq!(bytes, postcard::to_allocvec(&OldClock(n)).expect("encodes"));
            assert_eq!(bytes, postcard::to_allocvec(&OldSeq(n)).expect("encodes"));
            let back: Seq<Tick> = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, new);
            let old: OldClock = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(old.0, n);
            let json = serde_json::to_string(&new).expect("a number encodes");
            assert_eq!(json, serde_json::to_string(&OldClock(n)).expect("encodes"));
            assert_eq!(json, serde_json::to_string(&OldSeq(n)).expect("encodes"));
            let back: Seq<Tick> = serde_json::from_str(&json).expect("decodes");
            assert_eq!(back, new);
            let old: OldClock = serde_json::from_str(&json).expect("decodes");
            assert_eq!(old.0, n);
        }
    }
}
