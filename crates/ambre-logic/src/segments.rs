// ambre-logic/src/segments.rs
// ============================================================================
// The close of a full segment (the Murmure segments spec). A send that fills
// the active segment asks the Sync worker to close it, the relay names the
// successor, and `meta` records the answer. A member who did not write the
// close follows it from `meta`, and a closed channel watches the new segment.
// ============================================================================

use super::*;
use ambre_bus::{ChannelDocument, ChannelDocumentKind, ChannelId, SegmentId, Timestamp};
use ambre_murmure::ChannelState;

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
        self.send_to_sync(SyncCommand::CloseSegment {
            channel_id,
            meta,
            closing,
            successor: SegmentId::new(),
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

    /// A `meta` delta may name a successor another member created. Fetch
    /// it once; it becomes active when its bytes are in.
    pub(crate) fn follow_close(&mut self, channel_id: ChannelId) {
        let Some(successor) = self
            .channels
            .get_mut(&channel_id)
            .and_then(ChannelState::successor_to_open)
        else {
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
