//! Counters that tell a late answer from the current one.
//!
//! A timer, a request or a cache key carries the number it was armed, asked
//! or built under. The answer is live only while that number is the latest
//! one. Each family counts on its own axis, so each family is its own
//! marker type. A [`Generation`] of one family never compares to another.
//!
//! An [`Issuer`] is the one source of a family. It opens on the family's
//! [`GenerationKind::IssueRight`]. A family whose right is `()` is open, and
//! any crate may count it on with [`Generation::next`]. A sealed family has
//! a right with a private constructor. Only the module of the right opens
//! an issuer, so no other crate forges a generation of it.

use std::cmp::Ordering as CmpOrdering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// The marker of one family of generations.
///
/// The marker is an uninhabited type: it names the family and holds no
/// value. The trait is open, because the families live in the crates that
/// count them.
///
/// A sealed family keeps its right private, and its module opens the issuer:
///
/// ```
/// mod seal {
///     use ambre_core::{GenerationKind, Issuer};
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl GenerationKind for Ticket {
///         type IssueRight = Right;
///         const NAME: &'static str = "Ticket";
///     }
///     pub fn issuer() -> Issuer<Ticket> {
///         Issuer::new(Right(()))
///     }
/// }
/// let mut issuer = seal::issuer();
/// assert!(issuer.issue() > ambre_core::Generation::ZERO);
/// ```
///
/// Outside that module, the right cannot be built:
///
/// ```compile_fail,E0603
/// mod seal {
///     use ambre_core::GenerationKind;
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl GenerationKind for Ticket {
///         type IssueRight = Right;
///         const NAME: &'static str = "Ticket";
///     }
/// }
/// let _ = ambre_core::Issuer::<seal::Ticket>::new(seal::Right(()));
/// ```
///
/// And a sealed generation has no successor of its own:
///
/// ```compile_fail,E0599
/// mod seal {
///     use ambre_core::GenerationKind;
///     pub enum Ticket {}
///     #[derive(Clone, Copy)]
///     pub struct Right(());
///     impl GenerationKind for Ticket {
///         type IssueRight = Right;
///         const NAME: &'static str = "Ticket";
///     }
/// }
/// let _ = ambre_core::Generation::<seal::Ticket>::ZERO.next();
/// ```
pub trait GenerationKind: 'static {
    /// What opens an [`Issuer`] of the family: `()` for an open family, or
    /// a type with a private constructor for a sealed one. It is `Copy`, so
    /// a `const` constructor can take it.
    type IssueRight: Copy;

    /// The name `Debug` prints, as the hand-written type printed its own.
    const NAME: &'static str;
}

/// A family whose number crosses a wire or a store.
///
/// The number is then read back from bytes another party wrote, so the
/// family cannot be sealed: the bound asks for an open right. Only such a
/// family has a serde form and the [`Generation::from_stored`] read.
pub trait Wire: GenerationKind<IssueRight = ()> {}

/// One number of the family `K`. It starts at [`ZERO`](Self::ZERO), only
/// grows, and compares to its own family alone.
///
/// Its serde form, for a [`Wire`] family, is the bare `u64`.
pub struct Generation<K: GenerationKind> {
    raw: u64,
    kind: PhantomData<fn() -> K>,
}

impl<K: GenerationKind> Generation<K> {
    /// The count before the first one issued. No issuer hands it out, so it
    /// never matches an answer to a real request.
    pub const ZERO: Self = Self::at(0);

    /// The generation `raw`, for this module alone.
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

impl<K: GenerationKind<IssueRight = ()>> Generation<K> {
    /// The one after this one, for an open family. A sealed family has no
    /// such method: its issuer alone counts on.
    #[must_use]
    pub const fn next(self) -> Self {
        self.successor()
    }
}

impl<K: Wire> Generation<K> {
    /// The generation a store row holds, read back after a reload.
    #[must_use]
    pub const fn from_stored(raw: u64) -> Self {
        Self::at(raw)
    }

    /// The number a store row keeps.
    #[must_use]
    pub const fn stored(self) -> u64 {
        self.raw
    }
}

// By hand, every impl below: a derive would bound the marker, which has no
// value to clone, compare or hash.

impl<K: GenerationKind> Clone for Generation<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: GenerationKind> Copy for Generation<K> {}

impl<K: GenerationKind> PartialEq for Generation<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K: GenerationKind> Eq for Generation<K> {}

impl<K: GenerationKind> PartialOrd for Generation<K> {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl<K: GenerationKind> Ord for Generation<K> {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.raw.cmp(&other.raw)
    }
}

impl<K: GenerationKind> Hash for Generation<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K: GenerationKind> Default for Generation<K> {
    fn default() -> Self {
        Self::ZERO
    }
}

/// `NAME(n)`, as the hand-written counter printed itself.
impl<K: GenerationKind> fmt::Debug for Generation<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(K::NAME).field(&self.raw).finish()
    }
}

impl<K: Wire> serde::Serialize for Generation<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.raw)
    }
}

impl<'de, K: Wire> serde::Deserialize<'de> for Generation<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <u64 as serde::Deserialize>::deserialize(deserializer).map(Self::at)
    }
}

/// The one source of the family `K`, held by one owner.
///
/// It is not `Clone`: two copies would hand out one number twice.
pub struct Issuer<K: GenerationKind> {
    latest: Generation<K>,
}

impl<K: GenerationKind> Issuer<K> {
    /// An issuer that has handed out nothing yet. Only a holder of the
    /// family's right opens one.
    #[must_use]
    pub const fn new(_right: K::IssueRight) -> Self {
        Self {
            latest: Generation::ZERO,
        }
    }

    /// A number past every number this issuer handed out.
    #[must_use = "a generation issued and dropped wastes a number"]
    pub const fn issue(&mut self) -> Generation<K> {
        self.latest = self.latest.successor();
        self.latest
    }

    /// The last number handed out, or [`Generation::ZERO`] before the first.
    /// An answer is live while it carries this number.
    #[must_use]
    pub const fn latest(&self) -> Generation<K> {
        self.latest
    }
}

impl<K: GenerationKind<IssueRight = ()>> Default for Issuer<K> {
    fn default() -> Self {
        Self::new(())
    }
}

impl<K: GenerationKind> fmt::Debug for Issuer<K> {
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
pub struct SharedIssuer<K: GenerationKind> {
    latest: AtomicU64,
    kind: PhantomData<fn() -> K>,
}

impl<K: GenerationKind> SharedIssuer<K> {
    /// An issuer that has handed out nothing yet.
    #[must_use]
    pub const fn new(_right: K::IssueRight) -> Self {
        Self {
            latest: AtomicU64::new(0),
            kind: PhantomData,
        }
    }

    /// A number past every number this issuer handed out, to any thread.
    #[must_use = "a generation issued and dropped wastes a number"]
    pub fn issue(&self) -> Generation<K> {
        // Relaxed: the number orders nothing but itself. The closure never
        // refuses, so both arms carry the number before the update.
        let before = match self
            .latest
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |raw| {
                Some(raw.saturating_add(1))
            }) {
            Ok(raw) | Err(raw) => raw,
        };
        Generation::at(before).successor()
    }
}

impl<K: GenerationKind> fmt::Debug for SharedIssuer<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedIssuer").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{Generation, GenerationKind, Issuer, SharedIssuer, Wire};

    /// An open test family.
    enum Tick {}
    impl GenerationKind for Tick {
        type IssueRight = ();
        const NAME: &'static str = "Tick";
    }
    impl Wire for Tick {}

    /// A sealed test family: only this module builds its right.
    enum Ticket {}
    #[derive(Clone, Copy)]
    struct TicketRight(());
    impl GenerationKind for Ticket {
        type IssueRight = TicketRight;
        const NAME: &'static str = "Ticket";
    }

    #[test]
    fn a_counter_only_grows_and_saturates() {
        let first = Generation::<Tick>::ZERO.next();
        assert!(first > Generation::ZERO);
        assert!(first.next() > first);
        assert_eq!(Generation::<Tick>::default(), Generation::ZERO);
        assert_eq!(Generation::<Tick>::nth(2), first.next());
        let last = Generation::<Tick>::nth(u64::MAX);
        assert_eq!(last.next(), last);
    }

    #[test]
    fn an_issuer_hands_out_each_number_once_and_never_zero() {
        let mut issuer = Issuer::<Ticket>::new(TicketRight(()));
        assert_eq!(issuer.latest(), Generation::ZERO);
        let (one, two) = (issuer.issue(), issuer.issue());
        assert!(Generation::ZERO < one && one < two);
        assert_eq!(issuer.latest(), two);
        assert_eq!(issuer.issue(), Generation::nth(3));
    }

    #[test]
    fn a_shared_issuer_counts_like_an_issuer() {
        static SHARED: SharedIssuer<Tick> = SharedIssuer::new(());
        let one = SHARED.issue();
        assert_eq!(one, Generation::nth(1));
        assert_eq!(SHARED.issue(), one.next());
    }

    #[test]
    fn debug_prints_the_family_name() {
        assert_eq!(format!("{:?}", Generation::<Ticket>::nth(3)), "Ticket(3)");
    }

    #[test]
    fn a_wire_family_round_trips_as_its_bare_number() {
        let tick = Generation::<Tick>::nth(300);
        let json = serde_json::to_string(&tick).expect("serialize");
        assert_eq!(json, "300");
        assert_eq!(
            serde_json::from_str::<Generation<Tick>>(&json).ok(),
            Some(tick)
        );
        let bytes = postcard::to_allocvec(&tick).expect("serialize");
        assert_eq!(bytes, postcard::to_allocvec(&300_u64).expect("serialize"));
        assert_eq!(
            postcard::from_bytes::<Generation<Tick>>(&bytes).ok(),
            Some(tick)
        );
        assert_eq!(Generation::<Tick>::from_stored(tick.stored()), tick);
    }
}
