// ambre-logic/src/segments.rs
// ============================================================================
// The close of a full segment (the Murmure segments spec). A send that fills
// the active segment asks the Sync worker to close it, the relay names the
// successor, and `meta` records the answer. A member who did not write the
// close follows it from `meta`, and a closed channel watches the new segment.
// The closing segment then finalizes: the lease holder seals its shallow
// snapshot, and `meta` marks it final.
// ============================================================================

use super::*;
use ambre_bus::{
    ChannelDocument, ChannelDocumentKind, ChannelId, LeaseAnswer, SegmentId, Timestamp,
};

impl LogicState {
    /// Ask the Sync worker to close the active segment of `channel_id` once
    /// a send filled it. The engine asks once, until an answer.
    pub(crate) fn close_if_full(&mut self, channel_id: ChannelId, now: Timestamp) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let Some(closing) = channel.wants_close() else {
            return;
        };
        let meta = channel.meta_id();
        self.closing_segments.insert(channel_id, now);
        // A close that failed may still have registered its successor, so a
        // second ask names the same one (`SYNC-164`). The default is a fresh
        // identifier.
        let successor = *self.close_successors.entry(channel_id).or_default();
        self.send_to_sync(SyncCommand::CloseSegment {
            channel_id,
            meta,
            closing,
            successor,
        });
    }

    /// The relay closed `closing` into `active`. `meta` records it, and the
    /// successor opens: this client's own when `created`, the winner's
    /// otherwise.
    pub(crate) fn segment_closed(
        &mut self,
        channel_id: ChannelId,
        closing: SegmentId,
        active: SegmentId,
        created: bool,
    ) {
        let Some(now) = self.closing_segments.remove(&channel_id) else {
            return;
        };
        self.close_successors.remove(&channel_id);
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let closed = match channel.close_into(closing, active, created, now) {
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
        if created || closed.fetch.is_some() {
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
        channel_id: ChannelId,
        closing: SegmentId,
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
    pub(crate) fn drive_finalization(&mut self, channel_id: ChannelId, now: Timestamp) {
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
        let Some(closing) = channel.closing_id() else {
            return;
        };
        let covered = self
            .channel_ledgers
            .get(&closing.document_id())
            .map_or(0, snapshot_check::SeqLedger::contiguous_end);
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
        channel_id: ChannelId,
        segment: SegmentId,
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
    pub(crate) fn segment_finalized(&mut self, channel_id: ChannelId, segment: SegmentId) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let meta = match channel.mark_final(segment) {
            Ok(meta) => meta,
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

    /// A lease request or a commit of `segment` failed. A copy that
    /// `withdrawn` leaves the segment to another member; any other failure
    /// asks again later.
    pub(crate) fn segment_finalize_failed(
        &mut self,
        channel_id: ChannelId,
        segment: SegmentId,
        withdrawn: bool,
        message: &str,
    ) {
        tracing::warn!("Logic: finalization of {segment} on {channel_id} failed: {message}");
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        if withdrawn {
            channel.finalize_withdrawn(segment);
        } else {
            channel.finalize_failed(segment);
        }
    }

    /// A `meta` delta may name a successor another member created, or a
    /// closing segment another member finalized. Fetch the successor once;
    /// it becomes active when its bytes are in.
    pub(crate) fn follow_close(&mut self, channel_id: ChannelId) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        channel.settle_final();
        let Some(successor) = channel.successor_to_open() else {
            return;
        };
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id: successor.document_id(),
            doc_type: DocType::Murmure,
        });
    }

    /// The `meta` of a closed channel moved: it may name a new segment, so
    /// the channel is listed again (the Murmure unread spec).
    pub(crate) fn closed_meta_moved(&mut self, document_id: DocumentId) {
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
    pub(crate) fn watch_listed(&mut self, channel_id: ChannelId, documents: &[ChannelDocument]) {
        let Some(closed) = self.closed_channels.get_mut(&channel_id) else {
            return;
        };
        let meta = closed.meta();
        let fresh: Vec<SegmentId> = documents
            .iter()
            .filter(|d| d.kind == ChannelDocumentKind::Segment)
            .filter(|d| closed.watch(d.document_id))
            .map(|d| SegmentId::from_document(d.document_id))
            .collect();
        for segment in fresh {
            self.send_to_sync(SyncCommand::WatchSegment { segment, meta });
        }
    }
}
