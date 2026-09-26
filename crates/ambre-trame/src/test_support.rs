// ambre-trame/src/test_support.rs
// ============================================================================
// MockSyncHandle -- a test double for the Sync thread.
//
// Controller tests exercise `TrameState` against a realistic collaboration loop
// without a network. A `MockSyncHandle` plays the Sync worker, capturing the
// `SyncCommand`s a controller emits through the `ambre_bus::CommandSink` seam.
// It relays the captured CRDT deltas to a peer controller's `apply_remote`.
//
// In production the Logic dispatcher (`ambre-logic::LogicState`) reads a
// `TrameOutcome` and pushes `SyncCommand::SendUpdate` / `SendEphemeral` onto the
// Logic->Sync queue. `TrameState` is a typed `DocState` variant that holds no
// transport handle of its own (see `trame-engine` and
// `ambre-spec-message-bus.md`). `MockSyncHandle::absorb` reproduces that exact
// relay step over the `TrameOutcome` API, so tests drive the same `SyncCommand`s
// through the same `CommandSink` seam the dispatcher emits.
// ============================================================================

use std::cell::RefCell;
use std::rc::Rc;

use ambre_bus::{CommandSink, DocumentId, SyncCommand, TrameEvent};

use crate::{TrameOutcome, TrameState};

/// In-memory stand-in for the Sync thread.
///
/// Implements [`CommandSink`], so a controller (or, today, [`Self::absorb`])
/// pushes `SyncCommand`s into it; tests then inspect them or relay the CRDT
/// deltas to a peer. Cheap to [`Clone`] -- clones share one buffer (`Rc`),
/// matching how a single Sync handle is shared by reference in production.
#[derive(Clone, Default)]
pub struct MockSyncHandle {
    sent: Rc<RefCell<Vec<SyncCommand>>>,
}

impl CommandSink for MockSyncHandle {
    fn send(&self, cmd: SyncCommand) {
        self.sent.borrow_mut().push(cmd);
    }
}

impl MockSyncHandle {
    /// A fresh handle with an empty capture buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a controller outcome through the sink exactly as the Logic
    /// dispatcher does in production: a CRDT delta becomes
    /// [`SyncCommand::SendUpdate`], an ephemeral payload becomes
    /// [`SyncCommand::SendEphemeral`]. Both are tagged with `document_id`.
    pub fn absorb(&self, document_id: DocumentId, outcome: &TrameOutcome) {
        if let Some(payload) = &outcome.update {
            self.send(SyncCommand::SendUpdate {
                document_id,
                payload: payload.clone(),
            });
        }
        if let Some(payload) = &outcome.ephemeral {
            self.send(SyncCommand::SendEphemeral {
                document_id,
                payload: payload.clone(),
            });
        }
    }

    /// Number of commands captured so far.
    pub fn len(&self) -> usize {
        self.sent.borrow().len()
    }

    /// Whether anything has been captured.
    pub fn is_empty(&self) -> bool {
        self.sent.borrow().is_empty()
    }

    /// Drain and return every captured command, leaving the buffer empty.
    pub fn drain(&self) -> Vec<SyncCommand> {
        std::mem::take(&mut *self.sent.borrow_mut())
    }

    /// Drain only the CRDT-update payloads (`SendUpdate`), in capture order.
    /// Other captured commands are dropped -- this is the relay payload a
    /// peer needs.
    pub fn drain_update_payloads(&self) -> Vec<Vec<u8>> {
        self.drain()
            .into_iter()
            .filter_map(|cmd| match cmd {
                SyncCommand::SendUpdate { payload, .. } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// Deliver every pending CRDT delta to `peer` via [`TrameState::apply_remote`],
    /// draining the buffer. Returns the peer's flattened display events, so a
    /// test can assert on conflict detection or viewport resets triggered by the
    /// merge. This is the "B receives A's edits" half of a two-peer loop.
    pub fn deliver_updates_to(&self, peer: &mut TrameState) -> Vec<TrameEvent> {
        self.drain_update_payloads()
            .iter()
            .flat_map(|payload| peer.apply_remote(payload))
            .collect()
    }
}
