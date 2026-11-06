// ambre-canvas/src/request_registry.rs
// ============================================================================
// RequestRegistry<Tag, Id> -- typed P3 request/response correlation on the UI
// thread.
//
// The registry draws each request id from the allocator of its caller. It
// registers a pending entry with a deadline, and later resolves the matching
// answer by id. Expired entries (whose deadline has passed) are silently
// dropped -- the caller should reissue.
//
// The registry names no bus type: the renderer knows no thread protocol
// (`ARCH-071`). The caller injects the allocator, `ambre_bus::next_request` on
// the UI thread. It wraps the returned id in its own `LogicCommand::Rest`.
// The id is registered before `prepare` returns it, so no id the registry
// hands out goes unrecorded.
//
// Generic over `RequestTag` for future type-safety: each tag maps a
// `RestCommand` variant to its expected `RestEvent` variant. No tag is
// implemented yet (no consumer wired), but the scaffolding is here per ARCH P1.
// ============================================================================

use ambre_core::{Instant, Ms, PageLoad, TimeSpan};
use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::marker::PhantomData;

/// Associates a `RestCommand` variant with its expected `RestEvent` response.
///
/// Each implementation fixes the default timeout and the expected event type.
/// The actual matching is done by the caller after `try_recv` confirms the
/// request id -- the tag only provides compile-time documentation of the
/// expected variant.
pub trait RequestTag {
    /// The `RestEvent` variant expected in response, for documentation only.
    type Event;
    /// Default timeout in milliseconds before the request is considered stale.
    const DEFAULT_TIMEOUT: TimeSpan<Ms>;
}

/// A pending P3 request waiting for its `RestEvent` response.
struct PendingEntry {
    /// Wall-clock deadline in milliseconds (`performance.now()` domain).
    deadline: Instant<PageLoad>,
}

/// Typed P3 request/response correlator for the UI thread, over the request
/// id type `Id` (`Seq<Request>` on the UI thread).
///
/// # Lifecycle
///
/// 1. **Send** -- `let id = registry.prepare(now, ambre_bus::next_request)`
///    draws a fresh id from the allocator and records a `PendingEntry`. The
///    caller wraps `id` in a `LogicCommand::Rest` envelope and forwards it to
///    the Logic thread.
/// 2. **Receive** -- when a `UiEvent::Rest { id, evt }` arrives, call
///    `registry.try_recv(id)` to check whether the id was pending. If so, the
///    caller should extract the expected variant from `evt`.
/// 3. **Expire** -- call `registry.expire(now)` once per frame to drop entries
///    whose deadline has passed.
pub struct RequestRegistry<T: RequestTag, Id> {
    pending: FxHashMap<Id, PendingEntry>,
    _tag: PhantomData<T>,
}

impl<T: RequestTag, Id: Copy + Eq + Hash> RequestRegistry<T, Id> {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            pending: FxHashMap::default(),
            _tag: PhantomData,
        }
    }

    /// Number of pending (in-flight) requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Remove all pending entries without firing any callback.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Draw a fresh id from `next_id`, register a pending entry with the
    /// default timeout, and return the id for the caller's envelope.
    ///
    /// `now` is a `performance.now()` timestamp in milliseconds. `next_id`
    /// MUST hand out each id once, as `ambre_bus::next_request` does: a reused
    /// id replaces the pending entry it collides with.
    pub fn prepare(&mut self, now: Instant<PageLoad>, next_id: impl FnOnce() -> Id) -> Id {
        self.prepare_with_timeout(now, T::DEFAULT_TIMEOUT, next_id)
    }

    /// Like [`prepare`](Self::prepare) but with an explicit timeout override.
    pub fn prepare_with_timeout(
        &mut self,
        now: Instant<PageLoad>,
        timeout: TimeSpan<Ms>,
        next_id: impl FnOnce() -> Id,
    ) -> Id {
        let id = next_id();
        self.pending.insert(
            id,
            PendingEntry {
                deadline: now.after(timeout.to_fractional()),
            },
        );
        id
    }

    /// Consume the pending entry for `id`, returning `true` if the id was
    /// registered and had not expired. The caller should then downcast the
    /// received `RestEvent` to `<T as RequestTag>::Event`.
    ///
    /// Returns `false` for unknown, already-resolved, or expired ids.
    pub fn try_recv(&mut self, id: Id) -> bool {
        self.pending.remove(&id).is_some()
    }

    /// Drop all pending entries whose deadline has passed. Call once per frame
    /// before draining the inbound queue, so queued events don't match an
    /// already-expired id.
    pub fn expire(&mut self, now: Instant<PageLoad>) {
        self.pending.retain(|_id, entry| entry.deadline > now);
    }
}

impl<T: RequestTag, Id: Copy + Eq + Hash> Default for RequestRegistry<T, Id> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Minimal tag for testing -- maps nothing, provides a default timeout.
    enum TestTag {}
    impl RequestTag for TestTag {
        type Event = ();
        const DEFAULT_TIMEOUT: TimeSpan<Ms> = TimeSpan::<Ms>::new(5000);
    }

    type TestRegistry = RequestRegistry<TestTag, u64>;

    /// An allocator that hands out each id once, as `next_request` does.
    #[derive(Default)]
    struct Ids(Cell<u64>);

    impl Ids {
        fn next(&self) -> u64 {
            let id = self.0.get() + 1;
            self.0.set(id);
            id
        }
    }

    fn at(ms: f64) -> Instant<PageLoad> {
        Instant::<PageLoad>::new(ms)
    }

    #[test]
    fn prepare_allocates_unique_ids() {
        let ids = Ids::default();
        let mut reg = TestRegistry::new();
        let id1 = reg.prepare(at(0.0), || ids.next());
        let id2 = reg.prepare(at(0.0), || ids.next());
        assert_ne!(id1, id2);
        assert_eq!(reg.pending_count(), 2);
    }

    #[test]
    fn try_recv_matches_pending_id() {
        let ids = Ids::default();
        let mut reg = TestRegistry::new();
        let id = reg.prepare(at(0.0), || ids.next());
        assert!(reg.try_recv(id));
        assert!(!reg.try_recv(id)); // second call: already removed
        assert_eq!(reg.pending_count(), 0);
    }

    #[test]
    fn try_recv_rejects_unknown_id() {
        let mut reg = TestRegistry::new();
        assert!(!reg.try_recv(999));
    }

    #[test]
    fn expire_drops_overdue_entries() {
        let ids = Ids::default();
        let mut reg = TestRegistry::new();
        // Prepare with explicit 1000 ms timeout at t=0.
        let id = reg.prepare_with_timeout(at(0.0), TimeSpan::<Ms>::new(1000), || ids.next());
        assert_eq!(reg.pending_count(), 1);

        // Just before timeout -- still alive.
        reg.expire(at(999.9));
        assert_eq!(reg.pending_count(), 1);

        // Past deadline -- removed.
        reg.expire(at(1000.0));
        assert_eq!(reg.pending_count(), 0);
        assert!(!reg.try_recv(id));
    }

    #[test]
    fn clear_removes_all() {
        let ids = Ids::default();
        let mut reg = TestRegistry::new();
        reg.prepare(at(0.0), || ids.next());
        reg.prepare(at(0.0), || ids.next());
        assert_eq!(reg.pending_count(), 2);
        reg.clear();
        assert_eq!(reg.pending_count(), 0);
    }

    #[test]
    fn concurrent_requests_independent() {
        let ids = Ids::default();
        let mut reg = TestRegistry::new();
        let id_a = reg.prepare(at(0.0), || ids.next());
        let id_b = reg.prepare(at(0.0), || ids.next());

        assert_eq!(reg.pending_count(), 2);
        assert!(reg.try_recv(id_a));
        assert_eq!(reg.pending_count(), 1);
        assert!(!reg.try_recv(id_a)); // already gone
        assert!(reg.try_recv(id_b));
        assert_eq!(reg.pending_count(), 0);
    }
}
