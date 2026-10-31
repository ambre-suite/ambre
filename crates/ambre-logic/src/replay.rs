// ambre-logic/src/replay.rs
// ============================================================================
// A history replay imported in slices (`SYNC-131`).
//
// A reopen after a long absence replays every delta past the local copy. One
// import of all of them would hold the Logic thread for many frames. The
// accepted deltas queue on their document instead. Each pass of the loop
// imports one slice, then the loop comes back unasked, as it does for a board
// import (`ARCH-030`).
// ============================================================================

use super::*;

use std::collections::VecDeque;

/// The payload bytes one pass imports. A guess, held by no measure yet. On a
/// slow phone a Loro import of this much takes a few milliseconds. That
/// leaves the frame of `ambre-spec-budgets.md` section 2 room to paint. A
/// slice always takes one delta, whatever its size.
pub(crate) const REPLAY_SLICE_BYTES: usize = 64 << 10;

/// The deltas of a replay not yet imported, oldest first, with their seq.
pub(crate) type ReplayQueue = VecDeque<(Seq, ambre_core::Payload<ambre_core::DeltaBytes>)>;

impl LogicState {
    /// Queue the accepted deltas of a replay, and import its first slice now.
    pub(crate) fn replay(
        &mut self,
        document_id: Id<Document>,
        accepted: Vec<(Seq, ambre_core::Payload<ambre_core::DeltaBytes>)>,
    ) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        doc.replay.extend(accepted);
        self.resume_replay(document_id);
    }

    /// The documents a replay is still being imported into.
    pub(crate) fn docs_replaying(&self) -> Vec<Id<Document>> {
        self.docs
            .iter()
            .filter(|(_, doc)| !doc.replay.is_empty())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether a replay is still being imported into the document.
    pub(crate) fn replaying(&self, document_id: Id<Document>) -> bool {
        self.docs
            .get(&document_id)
            .is_some_and(|doc| !doc.replay.is_empty())
    }

    /// Import the next slice of a document's replay. The last slice runs
    /// what the replay's end held back.
    pub(crate) fn resume_replay(&mut self, document_id: Id<Document>) {
        let Some(doc) = self.docs.get_mut(&document_id) else {
            return;
        };
        let slice = take_slice(&mut doc.replay);
        if slice.is_empty() {
            return;
        }
        let done = doc.replay.is_empty();
        let (seqs, payloads): (Vec<Seq>, Vec<ambre_core::Payload<ambre_core::DeltaBytes>>) =
            slice.into_iter().unzip();
        self.merge_batch(document_id, &payloads);
        let applied: Vec<(Seq, &ambre_core::Payload<ambre_core::DeltaBytes>)> =
            seqs.into_iter().zip(payloads.iter()).collect();
        self.track_waiting(document_id, &applied);
        if done {
            self.replay_landed(document_id);
        }
    }

    /// Import what is left of a document's replay, now. Called before what
    /// reads the whole document: a close, an export, a live delta.
    pub(crate) fn settle_replay(&mut self, document_id: Id<Document>) {
        while self.replaying(document_id) {
            self.resume_replay(document_id);
        }
    }

    /// The document is ready: what reads the whole replay runs now, or once
    /// its last slice lands.
    pub(crate) fn ready(&mut self, document_id: Id<Document>) {
        if self.replaying(document_id) {
            if let Some(doc) = self.docs.get_mut(&document_id) {
                doc.ready_held = true;
            }
            return;
        }
        self.after_replay(document_id);
    }

    /// The last slice landed: run what a ready document held back.
    fn replay_landed(&mut self, document_id: Id<Document>) {
        let held = self
            .docs
            .get_mut(&document_id)
            .is_some_and(|doc| std::mem::take(&mut doc.ready_held));
        if held {
            self.after_replay(document_id);
        }
    }

    /// Every delta the relay kept is imported. A delta still waiting now waits
    /// for ever (`AUTHOR-025`). What the relay lacks of an earlier session is
    /// known (`AUTHOR-028`).
    fn after_replay(&mut self, document_id: Id<Document>) {
        self.report_stalled(document_id);
        self.send_unsent(document_id);
    }
}

/// Take the oldest deltas up to [`REPLAY_SLICE_BYTES`], and at least one.
fn take_slice(queue: &mut ReplayQueue) -> Vec<(Seq, ambre_core::Payload<ambre_core::DeltaBytes>)> {
    let mut bytes = 0usize;
    let mut slice = Vec::new();
    while let Some((_, payload)) = queue.front() {
        let next = bytes.saturating_add(payload.len());
        if !slice.is_empty() && next > REPLAY_SLICE_BYTES {
            break;
        }
        bytes = next;
        if let Some(delta) = queue.pop_front() {
            slice.push(delta);
        }
    }
    slice
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::TrameCommand;
    use ambre_bus::{Col, Row};

    /// A delta a session of `peer` wrote, carrying `bytes` of cell text.
    fn written_by(peer: ambre_core::PeerId, bytes: usize) -> SignedDelta {
        let mut author = TrameState::new();
        assert!(author.adopt_session(peer, crate::test_entry(peer)));
        let payload = author
            .handle_command(TrameCommand::SetCellText {
                row: Row::new(0),
                col: Col::new(0),
                text: "x".repeat(bytes),
            })
            .update
            .expect("an edit writes a delta");
        SignedDelta {
            seq: ambre_core::Seq(peer.0),
            payload,
            nonce: FixedBytes::from_array([0; 16]),
            peer,
            user: None,
            summary: None,
        }
    }

    /// `SYNC-131`: a replay larger than a slice lands over several passes,
    /// each importing at most a slice. The loop comes back unasked, and what
    /// reads the whole replay waits for its last slice.
    #[test]
    fn a_long_replay_lands_in_slices_and_yields_between_them() {
        let (to_ui, ui) = test_link();
        let (to_persist, persist) = test_link();
        std::mem::forget(persist);
        let mut state = LogicState::new(to_ui, test_link().0, to_persist);
        let document_id = Id::<Document>::mint();
        state.open_with_identity(document_id, DocType::Trame);
        let deltas: Vec<SignedDelta> = (1..=8)
            .map(|peer| written_by(ambre_core::PeerId(peer), 20 << 10))
            .collect();
        let (from_ui_tx, from_ui) = ambre_bus::conduit::to_worker(
            ambre_bus::conduit::ChannelCapacity::new(64),
            &Doorbell::new(),
        );
        let (from_sync_tx, from_sync) = ambre_bus::conduit::to_worker(
            ambre_bus::conduit::ChannelCapacity::new(64),
            &Doorbell::new(),
        );
        drop(from_ui_tx);
        let queued = from_sync_tx.try_send(LogicEvent::RemoteUpdateBatch {
            document_id,
            deltas,
        });
        assert!(queued.is_ok());
        let queued = from_sync_tx.try_send(LogicEvent::DocumentReady {
            document_id,
            role: Role::Editor,
        });
        assert!(queued.is_ok());
        state.drain_pass(&from_ui, &from_sync);
        assert!(state.has_pending_work(), "the loop comes back unasked");
        assert!(state.docs[&document_id].ready_held, "the end waits");
        let mut passes = 1;
        while state.has_pending_work() {
            let left = state.docs[&document_id].replay.len();
            state.drain_pass(&from_ui, &from_sync);
            let took = left - state.docs[&document_id].replay.len();
            assert!(took * (20 << 10) <= REPLAY_SLICE_BYTES.max(20 << 10));
            passes += 1;
        }
        assert!(passes >= 3, "the replay took {passes} passes");
        assert!(!state.docs[&document_id].ready_held, "the end ran");
        let ready = ui
            .try_iter()
            .any(|event| matches!(event, UiEvent::DocumentReady { .. }));
        assert!(ready, "the view learns the role at once");
    }

    /// A slice always takes one delta, even one past the slice size.
    #[test]
    fn a_slice_takes_one_delta_whatever_its_size() {
        let mut queue: ReplayQueue = [
            (ambre_core::Seq(1), vec![0; REPLAY_SLICE_BYTES * 2].into()),
            (ambre_core::Seq(2), vec![0; 1].into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(take_slice(&mut queue).len(), 1);
        assert_eq!(take_slice(&mut queue).len(), 1);
        assert!(take_slice(&mut queue).is_empty());
    }
}
