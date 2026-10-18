// ambre-logic/src/threads.rs
// ============================================================================
// The threads of a channel (the Murmure threads spec). A first reply creates
// the thread document, and the Sync worker registers it with the relay, one
// per root. The link writes the root's `thread` field, so every member finds
// the thread. A member who did not write it follows the root and fetches it.
// ============================================================================

use super::*;
use ambre_bus::{ChannelId, MessageId, MurmureEvent, SegmentId, Timestamp};
use ambre_murmure::Replied;

impl LogicState {
    /// Hand the Sync worker what a reply wrote, and the view what it shows.
    /// That is the thread, the copy in the channel, and the root's new head.
    pub(crate) fn send_reply(&mut self, channel_id: ChannelId, replied: Replied, now: Timestamp) {
        let Replied { reply, copy, head } = replied;
        self.send_to_sync(SyncCommand::SendUpdate {
            document_id: reply.document,
            payload: reply.delta,
            summary: Some(reply.summary),
        });
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: reply.event,
        });
        if let Some(copy) = copy {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id: copy.document,
                payload: copy.delta,
                summary: Some(copy.summary),
            });
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: copy.event,
            });
        }
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: head,
        });
        self.close_if_full(channel_id, now);
    }

    /// The relay linked `thread` to `root` (`SRV-213`). The root names it,
    /// and it opens: this client's own for its session, or the winner's for
    /// its bytes. What waited on a lost one waits on the winner (`SRV-214`).
    pub(crate) fn thread_linked(
        &mut self,
        channel_id: ChannelId,
        segment: SegmentId,
        root: MessageId,
        thread: DocumentId,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let linked = match channel.thread_linked(segment, root, thread) {
            Ok(linked) => linked,
            Err(error) => {
                return tracing::warn!("Logic: thread of {root} not linked: {error}");
            }
        };
        if let Some(named) = linked.root {
            self.send_to_sync(SyncCommand::SendUpdate {
                document_id: named.document,
                payload: named.delta,
                summary: None,
            });
            self.send_to_ui(UiEvent::Channel {
                channel_id,
                event: named.event,
            });
        }
        if let Some(lost) = linked.replaced {
            // The replies move first: forgetting the session drops what
            // still waits on it.
            let waiting = self.murmure_waiting.remove(&lost).unwrap_or_default();
            self.murmure_waiting
                .entry(thread)
                .or_default()
                .extend(waiting);
            self.forget_murmure_session(lost);
        }
        if linked.open {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id: thread,
                doc_type: DocType::Murmure,
            });
        }
    }

    /// The creation or the registration of this client's own `thread`
    /// failed. The replies that waited on it are dropped, and the next one
    /// creates a thread again.
    pub(crate) fn thread_link_failed(
        &mut self,
        channel_id: ChannelId,
        root: MessageId,
        thread: DocumentId,
        message: &str,
    ) {
        tracing::warn!("Logic: thread of {root} on {channel_id} failed: {message}");
        if let Some(channel) = self.channels.get_mut(&channel_id)
            && channel.thread_failed(root, thread)
        {
            self.forget_murmure_session(thread);
        }
    }

    /// Open each thread a held root names, so its head reads its count.
    pub(crate) fn open_threads(&mut self, channel_id: ChannelId) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        for document_id in channel.threads_to_open() {
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id,
                doc_type: DocType::Murmure,
            });
        }
    }

    /// The thread `document_id` arrived: the view reads its replies.
    pub(crate) fn show_thread(&self, channel_id: ChannelId, document_id: DocumentId) {
        let Some(event) = self
            .channels
            .get(&channel_id)
            .and_then(|channel| channel.thread_view(document_id))
        else {
            return;
        };
        self.send_to_ui(UiEvent::Channel { channel_id, event });
    }
}

/// The events of one batch the view still needs. A segment answers with its
/// whole list, and a thread with all its replies, so only the last answer
/// of each is news.
pub(crate) fn latest_answers(events: Vec<MurmureEvent>) -> Vec<MurmureEvent> {
    let mut kept: Vec<MurmureEvent> = Vec::new();
    for event in events.into_iter().rev() {
        let seen = kept.iter().any(|later| same_answer(later, &event));
        if !seen {
            kept.push(event);
        }
    }
    kept.reverse();
    kept
}

/// Whether two events answer for the same list: one segment, or one thread.
fn same_answer(a: &MurmureEvent, b: &MurmureEvent) -> bool {
    match (a, b) {
        (
            MurmureEvent::ThreadMessages { thread: x, .. },
            MurmureEvent::ThreadMessages { thread: y, .. },
        ) => x == y,
        (MurmureEvent::ThreadMessages { .. }, _) | (_, MurmureEvent::ThreadMessages { .. }) => {
            false
        }
        _ => true,
    }
}
