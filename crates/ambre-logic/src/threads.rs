// ambre-logic/src/threads.rs
// ============================================================================
// The threads of a channel (the Murmure threads spec). A first reply creates
// the thread document, and the Sync worker registers it with the relay, one
// per root. The link writes the root's `thread` field, so every member finds
// the thread. A member who did not write it follows the root and fetches it.
// ============================================================================

use super::*;
use ambre_bus::{Authored, Instant, Mentions, MurmureEvent, Relay, Seq};
use ambre_murmure::{ChannelState, Replied};

impl LogicState {
    /// Hand the Sync worker what a reply wrote, and the view what it shows.
    /// That is the thread, the copy in the channel, and the root's new head.
    pub(crate) fn send_reply(
        &mut self,
        channel_id: Id<Channel>,
        replied: Replied,
        now: Instant<Authored>,
    ) {
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
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
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
        if linked.open == ambre_murmure::LinkedOpening::Open {
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
        channel_id: Id<Channel>,
        root: Id<Message>,
        thread: Id<Document>,
        message: &str,
    ) {
        tracing::warn!("Logic: thread of {root} on {channel_id} failed: {message}");
        if let Some(channel) = self.channels.get_mut(&channel_id)
            && channel.thread_failed(root, thread)
        {
            self.forget_murmure_session(thread);
        }
    }

    /// Open each thread a held root names, so its head reads its count. A
    /// thread counted from its summaries until now counts from its marks.
    pub(crate) fn open_threads(&mut self, channel_id: Id<Channel>) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        for document_id in channel.threads_to_open() {
            self.watched_threads.remove(&document_id);
            self.send_to_sync(SyncCommand::OpenDocument {
                document_id,
                doc_type: DocType::Murmure,
            });
        }
    }

    /// The relay signalled a delta on `thread`, a thread this client does
    /// not hold in full or light (the Murmure threads spec). A thread its
    /// channel holds or loads hears the delta on its own topic. Any other is
    /// watched from its summaries, once the activity answer names its
    /// channel. Nothing loads it: its root is not on screen.
    pub(crate) fn thread_activity(&mut self, thread: Id<Document>) {
        if self.follows_thread(thread) {
            return;
        }
        if let Some(&channel_id) = self.document_channels.get(&thread) {
            return self.watch_thread(channel_id, thread);
        }
        // One answer places every thread signalled while it is asked.
        if self.unplaced_threads.is_empty() {
            self.send_to_sync(SyncCommand::FetchActivity);
        }
        self.unplaced_threads.insert(thread);
    }

    /// The activity answer named the channels: watch each signalled thread
    /// it places. A thread it does not name is no longer this user's.
    pub(crate) fn place_threads(&mut self) {
        for thread in std::mem::take(&mut self.unplaced_threads) {
            if let Some(&channel_id) = self.document_channels.get(&thread)
                && !self.follows_thread(thread)
            {
                self.watch_thread(channel_id, thread);
            }
        }
    }

    /// Whether this client holds, loads or watches `thread`.
    fn follows_thread(&self, thread: Id<Document>) -> bool {
        self.watched_threads.contains_key(&thread)
            || self.channels.values().any(|channel| channel.holds(thread))
    }

    /// Count `thread` as a light subscription from its read cursor, under
    /// the key of its channel's `meta`. A channel this session neither
    /// opened nor closed shows the relay's mark alone, as its segments do.
    fn watch_thread(&mut self, channel_id: Id<Channel>, thread: Id<Document>) {
        let Some(meta) = self
            .channels
            .get(&channel_id)
            .map(ChannelState::meta_id)
            .or_else(|| self.closed_channels.get(&channel_id).map(|c| c.meta()))
        else {
            return;
        };
        let after = self
            .read_cursors
            .get(&thread)
            .map_or(Seq::<Relay>::new(0), |cursor| cursor.read);
        self.watched_threads.insert(
            thread,
            WatchedThread {
                channel_id,
                mentions: ambre_core::Count::<Mentions>::ZERO,
            },
        );
        self.send_to_sync(SyncCommand::WatchThread {
            thread,
            meta,
            after,
        });
    }

    /// Count the activity of a watched thread for its channel: its
    /// mentions, never its replies, which no unread count holds yet.
    /// Returns whether `thread` is watched.
    pub(crate) fn count_thread(
        &mut self,
        thread: Id<Document>,
        counters: ambre_core::activity::ActivityCounters,
    ) -> bool {
        let Some(watched) = self.watched_threads.get_mut(&thread) else {
            return false;
        };
        let named = ambre_core::Count::<Mentions>::recount(counters.mentions) > watched.mentions;
        watched.mentions = ambre_core::Count::<Mentions>::recount(counters.mentions);
        let channel_id = watched.channel_id;
        let mark = match self.closed_channels.get(&channel_id) {
            Some(closed) => closed.mark(channel_id, self.thread_mentions(channel_id)),
            None => self.counted(channel_id),
        };
        if let Some(mark) = mark {
            self.send_to_ui(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
        }
        // A reply that names the user is a mention on its channel.
        if named {
            self.send_to_ui(UiEvent::NewMessage {
                channel_id,
                mention: ambre_bus::MentionState::Mentioned,
                message: None,
            });
        }
        true
    }

    /// The mentions the watched threads of `channel_id` counted.
    pub(crate) fn thread_mentions(&self, channel_id: Id<Channel>) -> ambre_core::Count<Mentions> {
        mentions_of(&self.watched_threads, channel_id)
    }

    /// End the light subscription of each thread of `channel_id` watched.
    pub(crate) fn end_watched_threads(&mut self, channel_id: Id<Channel>) {
        let ended: Vec<Id<Document>> = self
            .watched_threads
            .iter()
            .filter(|(_, watched)| watched.channel_id == channel_id)
            .map(|(thread, _)| *thread)
            .collect();
        for document_id in ended {
            self.watched_threads.remove(&document_id);
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
    }

    /// The thread `document_id` arrived: the view reads its replies.
    pub(crate) fn show_thread(&self, channel_id: Id<Channel>, document_id: Id<Document>) {
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

/// A thread this client does not hold, counted from its summaries for the
/// mentions of its replies (the Murmure threads spec).
#[derive(Debug)]
pub(crate) struct WatchedThread {
    /// The channel the thread counts on.
    channel_id: Id<Channel>,
    /// The replies past the read cursor that mention this user.
    mentions: ambre_core::Count<Mentions>,
}

/// The mentions the watched threads of `channel_id` counted.
pub(crate) fn mentions_of(
    watched: &HashMap<Id<Document>, WatchedThread>,
    channel_id: Id<Channel>,
) -> ambre_core::Count<Mentions> {
    watched
        .values()
        .filter(|thread| thread.channel_id == channel_id)
        .fold(ambre_core::Count::<Mentions>::ZERO, |sum, thread| {
            sum.plus(thread.mentions)
        })
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
