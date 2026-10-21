// ambre-canvas/src/request_registry.rs
// ============================================================================
// RequestRegistry<Tag> -- typed P3 request/response correlation on the UI thread.
//
// The registry is the sole allocator of `RequestId` values on the UI side.
// It wraps an outgoing `RestCommand` in a `LogicCommand::Rest` envelope,
// registers a pending entry with a deadline, and later resolves the matching
// `UiEvent::Rest` by id. Expired entries (whose deadline has passed) are
// silently dropped -- the caller should reissue.
//
// Generic over `RequestTag` for future type-safety: each tag maps a
// `RestCommand` variant to its expected `RestEvent` variant. No tag is
// implemented yet (no consumer wired), but the scaffolding is here per ARCH P1.
// ============================================================================

use ambre_bus::{LogicCommand, RequestId, RestCommand};
use rustc_hash::FxHashMap;
use std::marker::PhantomData;

/// Associates a `RestCommand` variant with its expected `RestEvent` response.
///
/// Each implementation fixes the default timeout and the expected event type.
/// The actual matching is done by the caller after `try_recv` confirms the
/// `RequestId` -- the tag only provides compile-time documentation of the
/// expected variant.
pub trait RequestTag {
    /// The `RestEvent` variant expected in response, for documentation only.
    type Event;
    /// Default timeout in milliseconds before the request is considered stale.
    const DEFAULT_TIMEOUT: ambre_bus::Millis;
}

/// A pending P3 request waiting for its `RestEvent` response.
struct PendingEntry {
    /// Wall-clock deadline in milliseconds (`performance.now()` domain).
    deadline_ms: ambre_bus::PageTime,
}

/// Typed P3 request/response correlator for the UI thread.
///
/// # Lifecycle
///
/// 1. **Send** -- `registry.prepare(cmd, now_ms)` allocates a fresh `RequestId`,
///    records a `PendingEntry`, and returns a `LogicCommand::Rest` envelope
///    ready to forward to the Logic thread.
/// 2. **Receive** -- when a `UiEvent::Rest { id, evt }` arrives, call
///    `registry.try_recv(id)` to check whether the id was pending. If so, the
///    caller should extract the expected variant from `evt`.
/// 3. **Expire** -- call `registry.expire(now_ms)` once per frame to drop entries
///    whose deadline has passed.
pub struct RequestRegistry<T: RequestTag> {
    pending: FxHashMap<RequestId, PendingEntry>,
    _tag: PhantomData<T>,
}

impl<T: RequestTag> RequestRegistry<T> {
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

    /// Allocate a fresh `RequestId`, register a pending entry with the default
    /// timeout, and return the `LogicCommand::Rest` envelope ready to send
    /// downstream.
    ///
    /// `now_ms` is a `performance.now()` timestamp in milliseconds.
    pub fn prepare(&mut self, cmd: RestCommand, now_ms: ambre_bus::PageTime) -> LogicCommand {
        self.prepare_with_timeout(cmd, now_ms, T::DEFAULT_TIMEOUT)
    }

    /// Like [`prepare`](Self::prepare) but with an explicit timeout override.
    pub fn prepare_with_timeout(
        &mut self,
        cmd: RestCommand,
        now_ms: ambre_bus::PageTime,
        timeout: ambre_bus::Millis,
    ) -> LogicCommand {
        let id = RequestId::next();
        self.pending.insert(
            id,
            PendingEntry {
                deadline_ms: now_ms.after(timeout.0 as f64),
            },
        );
        LogicCommand::Rest { id, cmd }
    }

    /// Consume the pending entry for `id`, returning `true` if the id was
    /// registered and had not expired. The caller should then downcast the
    /// received `RestEvent` to `<T as RequestTag>::Event`.
    ///
    /// Returns `false` for unknown, already-resolved, or expired ids.
    pub fn try_recv(&mut self, id: RequestId) -> bool {
        self.pending.remove(&id).is_some()
    }

    /// Drop all pending entries whose deadline has passed. Call once per frame
    /// before draining the inbound queue, so queued events don't match an
    /// already-expired id.
    pub fn expire(&mut self, now_ms: ambre_bus::PageTime) {
        self.pending.retain(|_id, entry| entry.deadline_ms > now_ms);
    }
}

impl<T: RequestTag> Default for RequestRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal tag for testing -- maps nothing, provides a default timeout.
    enum TestTag {}
    impl RequestTag for TestTag {
        type Event = ();
        const DEFAULT_TIMEOUT: ambre_bus::Millis = ambre_bus::Millis(5000);
    }

    type TestRegistry = RequestRegistry<TestTag>;

    #[test]
    fn prepare_allocates_unique_ids() {
        // Not wasm, but RequestId::next() compiles and runs on the host too.
        let mut reg = TestRegistry::new();
        let cmd1 = reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        let cmd2 = reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        let LogicCommand::Rest { id: id1, .. } = cmd1 else {
            panic!()
        };
        let LogicCommand::Rest { id: id2, .. } = cmd2 else {
            panic!()
        };
        assert_ne!(id1, id2);
        assert_eq!(reg.pending_count(), 2);
    }

    #[test]
    fn try_recv_matches_pending_id() {
        let mut reg = TestRegistry::new();
        let cmd = reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        let LogicCommand::Rest { id, .. } = cmd else {
            panic!()
        };
        assert!(reg.try_recv(id));
        assert!(!reg.try_recv(id)); // second call: already removed
        assert_eq!(reg.pending_count(), 0);
    }

    #[test]
    fn try_recv_rejects_unknown_id() {
        let mut reg = TestRegistry::new();
        assert!(!reg.try_recv(RequestId(999)));
    }

    #[test]
    fn expire_drops_overdue_entries() {
        let mut reg = TestRegistry::new();
        // Prepare with explicit 1000 ms timeout at t=0.
        let cmd = reg.prepare_with_timeout(
            RestCommand::ListDocuments,
            ambre_bus::PageTime(0.0),
            ambre_bus::Millis(1000),
        );
        let LogicCommand::Rest { id, .. } = cmd else {
            panic!()
        };
        assert_eq!(reg.pending_count(), 1);

        // Just before timeout -- still alive.
        reg.expire(ambre_bus::PageTime(999.9));
        assert_eq!(reg.pending_count(), 1);

        // Past deadline -- removed.
        reg.expire(ambre_bus::PageTime(1000.0));
        assert_eq!(reg.pending_count(), 0);
        assert!(!reg.try_recv(id));
    }

    #[test]
    fn clear_removes_all() {
        let mut reg = TestRegistry::new();
        reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        assert_eq!(reg.pending_count(), 2);
        reg.clear();
        assert_eq!(reg.pending_count(), 0);
    }

    #[test]
    fn concurrent_requests_independent() {
        let mut reg = TestRegistry::new();
        let cmd_a = reg.prepare(RestCommand::ListDocuments, ambre_bus::PageTime(0.0));
        let cmd_b = reg.prepare(RestCommand::Logout, ambre_bus::PageTime(0.0));
        let LogicCommand::Rest { id: id_a, .. } = cmd_a else {
            panic!()
        };
        let LogicCommand::Rest { id: id_b, .. } = cmd_b else {
            panic!()
        };

        assert_eq!(reg.pending_count(), 2);
        assert!(reg.try_recv(id_a));
        assert_eq!(reg.pending_count(), 1);
        assert!(!reg.try_recv(id_a)); // already gone
        assert!(reg.try_recv(id_b));
        assert_eq!(reg.pending_count(), 0);
    }
}
