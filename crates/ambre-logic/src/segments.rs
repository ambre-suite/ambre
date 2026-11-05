// ambre-logic/src/segments.rs
// ============================================================================
// The close of a full segment (the Murmure segments spec). A send that fills
// the active segment asks the Sync worker to close it, the relay names the
// successor, and `meta` records the answer. A member who did not write the
// close follows it from `meta`, and a closed channel watches the new segment.
// The closing segment then finalizes: the lease holder seals its shallow
// snapshot, and `meta` marks it final. Its amendments fold in later, the
// same way, under a fold lease.
// ============================================================================

use super::*;
use ambre_bus::{Authored, ChannelDocument, ChannelDocumentKind, FoldAnswer, Instant, LeaseAnswer};
use ambre_murmure::message::{MurmureCommand, MurmureEvent};

impl LogicState {
    /// Ask the Sync worker to close the active segment of `channel_id` once
    /// a send filled it. The engine asks once, until an answer.
    pub(crate) fn close_if_full(&mut self, channel_id: Id<Channel>, now: Instant<Authored>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let Some(closing) = channel.wants_close() else {
            return;
        };
        let meta = channel.meta_id();
        self.closing_segments.insert(channel_id, now);
        // A close that failed may still have registered its successor, so a
        // second ask names the same one (`SYNC-164`). The first ask mints a
        // fresh identifier.
        let successor = *self
            .close_successors
            .entry(channel_id)
            .or_insert_with(Id::mint);
        self.send_to_sync(SyncCommand::CloseSegment {
            channel_id,
            meta,
            closing,
            successor,
        });
    }

    /// This client rotated the key of `document_id`. When it is the `meta` of
    /// a held channel, its active segment closes at the next tick (the
    /// Murmure channels spec).
    pub(crate) fn key_rotated(&mut self, document_id: Id<Document>) {
        if let Some(channel) = self
            .channels
            .values_mut()
            .find(|channel| channel.meta_id().document_id() == document_id)
        {
            channel.key_rotated();
        }
    }

    /// The relay closed `closing` into `active`. `meta` records it, and the
    /// successor opens: this client's own when it created it, the winner's
    /// otherwise.
    pub(crate) fn segment_closed(
        &mut self,
        channel_id: Id<Channel>,
        closing: Id<Segment>,
        active: Id<Segment>,
        successor: ambre_murmure::Successor,
    ) {
        let Some(now) = self.closing_segments.remove(&channel_id) else {
            return;
        };
        self.close_successors.remove(&channel_id);
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let closed = match channel.close_into(closing, active, successor, now) {
            Ok(closed) => closed,
            Err(error) => {
                channel.close_refused();
                tracing::warn!("Logic: close of {closing} on {channel_id} refused: {error}");
                return;
            }
        };
        let meta = channel.meta_id().document_id();
        self.send_to_sync(SyncCommand::SendUpdate {
            document_id: meta,
            payload: closed.meta,
            summary: None,
        });
        // The created successor is held already, and its whole content leaves
        // as its first update once its identity lands. A fetched one is
        // pending until its bytes are in.
        if successor == ambre_murmure::Successor::Created || closed.fetch.is_some() {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id: active.document_id(),
                doc_type: DocType::Murmure,
            });
        }
    }

    /// The close of `closing` failed. The next send that finds the segment
    /// full asks again.
    pub(crate) fn segment_close_failed(
        &mut self,
        channel_id: Id<Channel>,
        closing: Id<Segment>,
        message: &str,
    ) {
        self.closing_segments.remove(&channel_id);
        tracing::warn!("Logic: close of {closing} on {channel_id} failed: {message}");
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.close_refused();
        }
    }

    /// Drive the finalization of the closing segment of `channel_id` at
    /// `now`: ask for its lease when due. The final snapshot goes to the
    /// Sync worker once this copy holds every delta up to the horizon.
    pub(crate) fn drive_finalization(&mut self, channel_id: Id<Channel>, now: Instant<Authored>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        if let Some(segment) = channel.lease_to_ask(now) {
            self.send_to_sync(SyncCommand::TakeFinalizeLease {
                channel_id,
                segment,
            });
            return;
        }
        let Some(closing) = channel.finalizing_id() else {
            return;
        };
        let covered = self.channel_ledgers.get(&closing.document_id()).map_or(
            Seq::<Relay>::ZERO,
            snapshot_check::SeqLedger::contiguous_end,
        );
        match channel.final_snapshot(now, covered) {
            Ok(Some(snapshot)) => self.send_to_sync(SyncCommand::FinalizeSegment {
                channel_id,
                segment: snapshot.segment,
                lease: snapshot.lease,
                horizon: snapshot.horizon,
                payload: snapshot.payload,
            }),
            Ok(None) => {}
            Err(error) => {
                channel.finalize_failed(closing);
                tracing::warn!("Logic: final snapshot of {closing} refused: {error}");
            }
        }
    }

    /// The relay answered the lease request for `segment`.
    pub(crate) fn finalize_lease_answered(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        answer: LeaseAnswer,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        match answer {
            LeaseAnswer::Granted {
                lease,
                horizon,
                expires_in,
            } => channel.lease_granted(segment, lease, horizon, expires_in),
            LeaseAnswer::Deferred { retry_in } => channel.lease_deferred(segment, retry_in),
        }
    }

    /// The relay accepted the final snapshot of `segment`: `meta` marks it
    /// final, and the delta leaves for the other members.
    pub(crate) fn segment_finalized(&mut self, channel_id: Id<Channel>, segment: Id<Segment>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        // A segment finalized again is final in `meta` already.
        let meta = match channel.mark_final(segment) {
            Ok(Some(meta)) => meta,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!("Logic: final mark of {segment} on {channel_id} refused: {error}");
                return;
            }
        };
        let document_id = channel.meta_id().document_id();
        self.send_to_sync(SyncCommand::SendUpdate {
            document_id,
            payload: meta,
            summary: None,
        });
    }

    /// A lease request or a commit of `segment` failed. A copy that withdrew
    /// leaves the segment to another member; any other failure asks again
    /// later.
    pub(crate) fn segment_finalize_failed(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        end: ambre_core::FinalizeEnd,
        message: &str,
    ) {
        tracing::warn!("Logic: finalization of {segment} on {channel_id} failed: {message}");
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        match end {
            ambre_core::FinalizeEnd::Withdrawn => channel.finalize_withdrawn(segment),
            ambre_core::FinalizeEnd::Failed => channel.finalize_failed(segment),
        }
    }

    /// Drive the fold of the amendments of a final segment of `channel_id` at
    /// `now`: ask for its lease when due. The fold snapshot goes to the Sync
    /// worker once this copy holds the amendments the lease names.
    pub(crate) fn drive_fold(&mut self, channel_id: Id<Channel>, now: Instant<Authored>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        if let Some(segment) = channel.fold_lease_to_ask(now) {
            return self.send_to_sync(SyncCommand::TakeFoldLease {
                channel_id,
                segment,
            });
        }
        match channel.fold_snapshot(now) {
            Ok(Some(fold)) => self.send_to_sync(SyncCommand::FoldSegment {
                channel_id,
                segment: fold.segment,
                lease: fold.lease,
                horizon: fold.horizon,
                amendments_seq: fold.amendments_seq,
                payload: fold.payload,
            }),
            Ok(None) => {}
            Err(error) => tracing::warn!("Logic: fold snapshot on {channel_id} refused: {error}"),
        }
    }

    /// The relay answered the fold lease request for `segment`.
    pub(crate) fn fold_lease_answered(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        answer: FoldAnswer,
    ) {
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.fold_answered(segment, answer);
        }
    }

    /// The relay took the fold of `segment`; another member's check settles
    /// it.
    pub(crate) fn segment_folded(&mut self, channel_id: Id<Channel>, segment: Id<Segment>) {
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.fold_committed(segment);
        }
    }

    /// A fold lease request or a fold commit of `segment` failed: it is asked
    /// again later.
    pub(crate) fn segment_fold_failed(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        message: &str,
    ) {
        tracing::warn!("Logic: fold of {segment} on {channel_id} failed: {message}");
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.fold_failed(segment);
        }
    }

    /// This client's confirmation settled the fold of the segment
    /// `document_id`, and the relay erased `amendments` (`SRV-211`). `meta`
    /// marks them folded, and the delta leaves for the other members.
    pub(crate) fn amendments_folded(
        &mut self,
        document_id: Id<Document>,
        amendments: Id<Document>,
    ) {
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return;
        };
        let segment = Id::<Segment>::from_document(document_id);
        match channel.mark_folded(segment, amendments) {
            Ok(Some(payload)) => {
                let meta = channel.meta_id().document_id();
                self.send_to_sync(SyncCommand::SendUpdate {
                    document_id: meta,
                    payload,
                    summary: None,
                });
            }
            Ok(None) => {}
            Err(error) => {
                return tracing::warn!("Logic: fold mark of {segment} refused: {error}");
            }
        }
        self.settle_folds(channel_id);
    }

    /// The relay no longer holds `document_id`. Linked amendments gone so
    /// were erased by a fold whose mark never reached this copy. `meta`
    /// marks them, and the refused change is written again (the Murmure
    /// segments spec).
    pub(crate) fn document_gone(&mut self, document_id: Id<Document>) {
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return;
        };
        match channel.amendments_gone(document_id) {
            Ok(Some(payload)) => {
                let meta = channel.meta_id().document_id();
                self.send_to_sync(SyncCommand::SendUpdate {
                    document_id: meta,
                    payload,
                    summary: None,
                });
            }
            Ok(None) => {}
            Err(error) => {
                return tracing::warn!("Logic: {document_id} gone, not settled: {error}");
            }
        }
        self.settle_folds(channel_id);
    }

    /// Fold in every amendments document `meta` marks folded, then close it.
    /// The changes of this client the relay never sequenced are written again
    /// at the next tick (the Murmure segments spec).
    pub(crate) fn settle_folds(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let settled = match channel.settle_folds() {
            Ok(settled) => settled,
            Err(error) => return tracing::warn!("Logic: folds of {channel_id} refused: {error}"),
        };
        for settled in settled {
            self.forget_murmure_session(settled.amendments);
            self.send_to_sync(SyncCommand::CloseDocument {
                document_id: settled.amendments,
            });
            if !settled.replay.is_empty() {
                self.owed_replays
                    .entry(channel_id)
                    .or_default()
                    .extend(settled.replay);
            }
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: settled.event,
            });
        }
    }

    /// A member reported the snapshot of `document_id` as false, and the relay
    /// dropped it. The view names its author and the reporter. A final
    /// snapshot dropped is finalized again by this copy when it holds the
    /// segment (the Murmure segments spec).
    pub(crate) fn snapshot_dropped(
        &mut self,
        document_id: Id<Document>,
        author: Option<Id<User>>,
        reporter: Option<Id<User>>,
        after: ambre_core::AfterDrop,
    ) {
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return;
        };
        let segment = Id::<Segment>::from_document(document_id);
        if after == ambre_core::AfterDrop::Refinalize {
            channel.refinalize(segment);
        }
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::SnapshotDropped {
                segment,
                author,
                reporter,
            },
        });
    }

    /// The relay refused deltas of `document_id` for its lock. The segment
    /// drops them, and the new messages they carried leave for the active
    /// segment (the Murmure segments spec).
    pub(crate) fn deltas_locked(
        &mut self,
        document_id: Id<Document>,
        deltas: &[ambre_core::Payload<ambre_core::DeltaBytes>],
    ) {
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return;
        };
        let relocated = match channel.relocate_locked(document_id, deltas) {
            Ok(relocated) => relocated,
            Err(error) => {
                return tracing::warn!("Logic: locked deltas of {document_id} not moved: {error}");
            }
        };
        // The segment reads as the relay holds it until the replays land.
        let segment = Id::<Segment>::from_document(document_id);
        let rewound = (!relocated.replay.is_empty())
            .then(|| channel.messages(segment).ok())
            .flatten();
        if let Some(sent) = relocated.sent {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id: sent.document,
                payload: sent.delta,
                summary: Some(sent.summary),
            });
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: sent.event,
            });
        }
        if let Some(messages) = rewound {
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: MurmureEvent::Messages { segment, messages },
            });
        }
        if !relocated.replay.is_empty() {
            self.owed_replays
                .entry(channel_id)
                .or_default()
                .extend(relocated.replay);
        }
    }

    /// Write again, as amendments stamped `now`, the edits and deletes the
    /// lock of a segment refused (the Murmure messages spec).
    pub(crate) fn replay_owed(&mut self, channel_id: Id<Channel>, now: Instant<Authored>) {
        for replay in self.owed_replays.remove(&channel_id).unwrap_or_default() {
            let command = match replay.draft {
                Some(draft) => MurmureCommand::EditMessage {
                    at: replay.segment.into(),
                    message: replay.message,
                    draft,
                    now,
                },
                None => MurmureCommand::DeleteMessage {
                    at: replay.segment.into(),
                    message: replay.message,
                    now,
                },
            };
            self.handle_channel_command(channel_id, command);
        }
    }

    /// Forget the replays a channel still owed, when it leaves memory before
    /// its next tick.
    pub(crate) fn drop_replays(&mut self, channel_id: Id<Channel>) {
        if let Some(owed) = self.owed_replays.remove(&channel_id) {
            tracing::warn!(
                "Logic: {} refused changes of {channel_id} closed unreplayed",
                owed.len()
            );
        }
    }

    /// The relay linked `amendments` to the locked `segment` (`SRV-201`).
    /// `meta` names it, and it opens: this client's own for its session, or
    /// the winner's for its bytes. What waited on a lost one waits on the
    /// winner (`SRV-202`).
    pub(crate) fn amendments_linked(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let linked = match channel.amendments_linked(segment, amendments) {
            Ok(linked) => linked,
            Err(error) => {
                return tracing::warn!("Logic: amendments of {segment} not linked: {error}");
            }
        };
        let meta = channel.meta_id().document_id();
        if let Some(payload) = linked.meta {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id: meta,
                payload,
                summary: None,
            });
        }
        if let Some(lost) = linked.replaced {
            let waiting = self.murmure_waiting.remove(&lost).unwrap_or_default();
            self.murmure_waiting
                .entry(amendments)
                .or_default()
                .extend(waiting);
        }
        if linked.open == ambre_murmure::LinkedOpening::Open {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id: amendments,
                doc_type: DocType::Murmure,
            });
        }
    }

    /// The creation or the link of this client's own `amendments` failed.
    /// The changes that waited on it are dropped, and the next one creates
    /// a document again.
    pub(crate) fn amendments_link_failed(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        amendments: Id<Document>,
        message: &str,
    ) {
        tracing::warn!("Logic: amendments of {segment} on {channel_id} failed: {message}");
        if let Some(channel) = self.channels.get_mut(&channel_id)
            && channel.amendments_failed(segment, amendments)
        {
            self.forget_murmure_session(amendments);
        }
    }

    /// Open each amendments document `meta` names for a segment the channel
    /// holds, so its messages read as amended.
    pub(crate) fn open_amendments(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        for document_id in channel.amendments_to_open() {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id,
                doc_type: DocType::Murmure,
            });
        }
    }

    /// A `meta` delta may name a successor another member created, or a
    /// closing segment another member finalized. Fetch the successor once;
    /// it becomes active when its bytes are in.
    pub(crate) fn follow_close(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        channel.settle_final();
        let successor = channel.successor_to_open();
        if let Some(successor) = successor {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id: successor.document_id(),
                doc_type: DocType::Murmure,
            });
        }
        // The delta may name the amendments another member linked, or mark
        // some folded.
        self.open_amendments(channel_id);
        // A segment delta may name a thread another member linked.
        self.open_threads(channel_id);
        self.settle_folds(channel_id);
    }

    /// The `meta` of a closed channel moved: it may name a new segment, so
    /// the channel is listed again (the Murmure unread spec).
    pub(crate) fn closed_meta_moved(&mut self, document_id: Id<Document>) {
        let Some(channel_id) = self
            .closed_channels
            .iter()
            .find(|(_, closed)| closed.meta().document_id() == document_id)
            .map(|(id, _)| *id)
        else {
            return;
        };
        if self.opening.contains_key(&channel_id) {
            return;
        }
        let listed_in = self.listing_space(channel_id);
        self.send_to_sync(SyncCommand::OpenChannel {
            channel_id,
            listed_in,
        });
    }

    /// A closed channel was listed again. Each segment it did not hold is
    /// counted from its first number, under the key of its `meta` (the
    /// Murmure unread spec).
    pub(crate) fn watch_listed(&mut self, channel_id: Id<Channel>, documents: &[ChannelDocument]) {
        let Some(closed) = self.closed_channels.get_mut(&channel_id) else {
            return;
        };
        let meta = closed.meta();
        let fresh: Vec<Id<Segment>> = documents
            .iter()
            .filter(|d| d.kind == ChannelDocumentKind::Segment)
            .filter(|d| closed.watch(d.document_id))
            .map(|d| Id::<Segment>::from_document(d.document_id))
            .collect();
        for segment in fresh {
            self.send_to_sync(SyncCommand::WatchSegment { segment, meta });
        }
    }
}
