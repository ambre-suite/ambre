// ambre-murmure/src/logic/runtime/threads.rs
// ============================================================================
// The threads of a channel (the Murmure threads spec). A first reply creates
// the thread document, and the Sync worker registers it with the relay, one
// per root. The link writes the root's `thread` field, so every member finds
// the thread. A member who did not write it follows the root and fetches it.
// ============================================================================

use super::*;
use crate::message::MurmureEvent;
use crate::{ChannelState, Replied};
use ambre_bus::{Authored, Instant, Mentions, Relay, Seq};

impl<D> Runtime<D> {
    /// Hand the Sync worker what a reply wrote, and the view what it shows.
    /// That is the thread, the copy in the channel, and the root's new head.
    pub(crate) fn send_reply(
        &mut self,
        channel_id: Id<Channel>,
        replied: Replied,
        now: Instant<Authored>,
    ) {
        let Replied {
            reply,
            copy,
            head,
            proof,
        } = replied;
        self.ask(SyncCommand::SendUpdate {
            document_id: reply.document,
            payload: reply.delta,
            summary: Some(reply.summary),
        });
        self.emit(UiEvent::Channel {
            channel_id,
            event: reply.event,
        });
        if let Some(copy) = copy {
            self.ask(SyncCommand::SendUpdate {
                document_id: copy.document,
                payload: copy.delta,
                summary: Some(copy.summary),
            });
            self.emit(UiEvent::Channel {
                channel_id,
                event: copy.event,
            });
        }
        self.emit(UiEvent::Channel {
            channel_id,
            event: head,
        });
        // A rotation of the link key leaves the root's receipts counting as
        // none. The Sync worker reads them under its key, and asks the relay
        // for a fresh one only then (`MURMURE-741`).
        if let Some(proof) = proof {
            self.ask(SyncCommand::FreshenReceipt {
                channel_id,
                segment: proof.segment,
                root: proof.root,
                thread: proof.thread,
                receipts: proof.receipts,
            });
        }
        self.close_if_full(channel_id, now);
    }

    /// The relay answered a `receipt` of the link of `thread`, which a root
    /// names already. The segment of the root, or its amendments, keep it
    /// under this member's key, unless the root holds it (`MURMURE-741`).
    fn fresh_receipt(
        &mut self,
        channel_id: Id<Channel>,
        at: (Id<Segment>, Id<Message>),
        thread: Id<Document>,
        receipt: ambre_core::FixedBytes<ambre_core::Signature>,
    ) {
        let Some(by) = self.my_user_id else {
            return;
        };
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        match channel.write_fresh_receipt(at, thread, receipt, by) {
            Ok(Some(written)) => self.ask(SyncCommand::SendUpdate {
                document_id: written.document,
                payload: written.delta,
                summary: None,
            }),
            Ok(None) => {}
            Err(error) => tracing::warn!("Logic: fresh receipt of {thread} not written: {error}"),
        }
    }

    /// The relay linked `thread` to `root` (`SRV-213`). The root names it,
    /// and it opens: this client's own for its session, or the winner's for
    /// its bytes. What waited on a lost one waits on the winner (`SRV-214`).
    /// The root keeps the relay's `receipt` of the link (`MURMURE-732`).
    pub fn thread_linked(
        &mut self,
        channel_id: Id<Channel>,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
        receipt: Option<ambre_core::FixedBytes<ambre_core::Signature>>,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        let meta_id = channel.meta_id();
        // This client's own thread holds no update on the relay yet. A
        // member who read the link now would open an empty thread. So the
        // root names the thread once the thread went up.
        if channel.thread_id(root) == Some(thread) && !channel.identified(thread) {
            self.unnamed_roots.insert(
                thread,
                UnnamedRoot {
                    channel_id,
                    segment,
                    root,
                    receipt,
                },
            );
            return self.ask(super::channels::open_in_channel(meta_id, thread));
        }
        let linked = match channel.thread_linked(segment, root, thread, receipt) {
            Ok(linked) => linked,
            Err(error) => {
                return tracing::warn!("Logic: thread of {root} not linked: {error}");
            }
        };
        if let Some(named) = linked.root {
            self.send_named(channel_id, named);
        } else if let Some(receipt) = receipt {
            // A root that names the thread already takes the receipt as a
            // fresh one: the answer of a fresh receipt request (`SYNC-334`).
            self.fresh_receipt(channel_id, (segment, root), thread, receipt);
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
        if linked.open == crate::LinkedOpening::Open {
            self.ask(super::channels::open_in_channel(meta_id, thread));
        }
    }

    /// This client's own `thread` went up whole, its waiting replies after
    /// it: its root now names it (the Murmure threads spec). A link on the open thread
    /// only writes the root, so its answer to open is dropped.
    pub fn name_root(&mut self, thread: Id<Document>) {
        let Some(UnnamedRoot {
            channel_id,
            segment,
            root,
            receipt,
        }) = self.unnamed_roots.remove(&thread)
        else {
            return;
        };
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            return;
        };
        match channel.thread_linked(segment, root, thread, receipt) {
            Ok(linked) => {
                if let Some(named) = linked.root {
                    self.send_named(channel_id, named);
                }
            }
            Err(error) => tracing::warn!("Logic: thread of {root} not named: {error}"),
        }
    }

    /// Send the delta that names a thread in its root, and show the root.
    fn send_named(&self, channel_id: Id<Channel>, named: crate::Sent) {
        self.ask(SyncCommand::SendUpdate {
            document_id: named.document,
            payload: named.delta,
            summary: None,
        });
        self.emit(UiEvent::Channel {
            channel_id,
            event: named.event,
        });
    }

    /// The creation or the registration of this client's own `thread`
    /// failed. The replies that waited on it are dropped, and the next one
    /// creates a thread again.
    pub fn thread_link_failed(
        &mut self,
        channel_id: Id<Channel>,
        root: Id<Message>,
        thread: Id<Document>,
        failure: ambre_bus::ApiFailure,
    ) {
        tracing::warn!("Logic: thread of {root} on {channel_id} failed: {failure}");
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
        let meta = channel.meta_id();
        for document_id in channel.threads_to_open() {
            self.watched_threads.remove(&document_id);
            self.ask(super::channels::open_in_channel(meta, document_id));
        }
    }

    /// The relay signalled a delta on `thread`, a thread this client does
    /// not hold in full or light (the Murmure threads spec). A thread its
    /// channel holds or loads hears the delta on its own topic. Any other is
    /// watched from its summaries, once the activity answer names its
    /// channel. Nothing loads it: its root is not on screen.
    pub fn thread_activity(&mut self, thread: Id<Document>) {
        if self.follows_thread(thread) {
            return;
        }
        if let Some(&channel_id) = self.document_channels.get(&thread) {
            return self.watch_thread(channel_id, thread);
        }
        // One answer places every thread signalled while it is asked.
        if self.unplaced_threads.is_empty() {
            self.ask(SyncCommand::FetchActivity);
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
        self.ask(SyncCommand::WatchThread {
            thread,
            meta,
            after,
        });
    }

    /// Count the activity of a watched thread for its channel: its
    /// mentions, never its replies, which no unread count holds yet.
    /// Returns whether `thread` is watched.
    pub fn count_thread(
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
            self.emit(UiEvent::ChannelActivity {
                channels: vec![mark],
            });
        }
        // A reply that names the user is a mention on its channel.
        if named {
            self.emit(UiEvent::NewMessage {
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
            self.ask(SyncCommand::CloseDocument { document_id });
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
        self.emit(UiEvent::Channel { channel_id, event });
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

/// A root whose own thread the relay linked, waiting for that thread to go
/// up before it names it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnnamedRoot {
    channel_id: Id<Channel>,
    segment: Id<Segment>,
    root: Id<Message>,
    /// The relay's receipt of the link, which the root takes with its name.
    receipt: Option<ambre_core::FixedBytes<ambre_core::Signature>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::id::{Document, Message, Segment};

    /// One batch answers once per list: the last answer of the segment, and
    /// the last of each thread, in their order.
    #[test]
    fn a_batch_keeps_the_last_answer_of_each_list() {
        let segment = Id::<Segment>::mint();
        let (first, second) = (Id::<Document>::mint(), Id::<Document>::mint());
        let root = Id::<Message>::mint();
        let list = || MurmureEvent::Messages {
            segment,
            messages: Vec::new(),
        };
        let replies = |thread: Id<Document>| MurmureEvent::ThreadMessages {
            root,
            thread,
            messages: Vec::new(),
        };
        let kept = latest_answers(vec![
            list(),
            replies(first),
            replies(second),
            list(),
            replies(first),
        ]);
        assert_eq!(kept.len(), 3, "{kept:?}");
        assert!(
            matches!(&kept[0], MurmureEvent::ThreadMessages { thread, .. } if *thread == second)
        );
        assert!(matches!(&kept[1], MurmureEvent::Messages { .. }));
        assert!(
            matches!(&kept[2], MurmureEvent::ThreadMessages { thread, .. } if *thread == first)
        );
    }
}
