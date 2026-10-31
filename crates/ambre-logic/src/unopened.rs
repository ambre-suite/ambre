// ambre-logic/src/unopened.rs
// ============================================================================
// The channels of the user this session never opened. Each is counted from
// the light subscriptions of its `meta` and its segments, as a closed channel
// is. Its first message then notifies (the Murmure notifications spec).
// ============================================================================

use super::*;
use ambre_bus::{ChannelDocumentKind, ChannelMeta, DocumentActivity};

/// The segments of a channel, each with the last number the activity
/// answer gave, if any.
type Segments = Vec<(Id<Document>, Option<Seq>)>;

/// What the Logic thread knows of the channels it may watch unopened.
#[derive(Debug, Default)]
pub(crate) struct ListedChannels {
    /// The `meta` of each channel of the latest listing whose key opened,
    /// with its last number then.
    metas: HashMap<Id<Channel>, ChannelMeta>,
    /// Whether a listing answered in this session.
    listed: bool,
    /// The segments of each channel the latest activity answer named, each
    /// with its last number then, or `None` before any answer.
    segments: Option<HashMap<Id<Channel>, Segments>>,
    /// The channels watched unopened, so a listing that drops one ends it.
    watched: HashSet<Id<Channel>>,
}

impl ListedChannels {
    /// Forget a channel the user no longer reads, deleted or left, so no
    /// later answer watches it again.
    pub(crate) fn forget(&mut self, channel_id: Id<Channel>) {
        self.metas.remove(&channel_id);
        self.watched.remove(&channel_id);
    }
}

impl LogicState {
    /// The relay listed the user's channels, with the `meta` of each whose
    /// key opened. A channel the listing drops is no longer the user's, and
    /// its watch ends. A channel joined since the last listing has segments
    /// no activity answer named yet, so the activity is asked again.
    pub(crate) fn channels_listed(&mut self, metas: &[ChannelMeta]) {
        let fresh: HashMap<Id<Channel>, ChannelMeta> =
            metas.iter().map(|m| (m.channel_id, *m)).collect();
        let listed = &mut self.listed_channels;
        let joined = listed.listed && fresh.keys().any(|c| !listed.metas.contains_key(c));
        let gone: Vec<Id<Channel>> = listed
            .watched
            .iter()
            .filter(|c| !fresh.contains_key(c))
            .copied()
            .collect();
        listed.listed = true;
        listed.metas = fresh;
        for channel_id in gone {
            self.listed_channels.watched.remove(&channel_id);
            self.end_light_channel(channel_id);
        }
        if joined && self.listed_channels.segments.is_some() {
            self.send_to_sync(SyncCommand::FetchActivity);
        }
        self.watch_unheld();
    }

    /// The relay's activity answer named the segments of each channel.
    pub(crate) fn segments_listed(&mut self, documents: &[DocumentActivity]) {
        let mut segments: HashMap<Id<Channel>, Segments> = HashMap::new();
        for document in documents
            .iter()
            .filter(|d| d.kind == ChannelDocumentKind::Segment)
        {
            segments
                .entry(document.channel_id)
                .or_default()
                .push((document.document_id, document.last));
        }
        self.listed_channels.segments = Some(segments);
        self.watch_unheld();
    }

    /// Watch each listed channel this session neither holds, opens nor
    /// counts. Each segment counts after the last number the answer gave,
    /// and its `meta` after the listing's, so nothing landing since is
    /// missed (`RESID-022`). A thread is left to its own signal, which
    /// counts its mentions alone. A channel no answer names a segment of
    /// waits.
    fn watch_unheld(&mut self) {
        let Some(segments) = &self.listed_channels.segments else {
            return;
        };
        let unheld: Vec<(ChannelMeta, Segments)> = self
            .listed_channels
            .metas
            .iter()
            .filter(|(c, _)| {
                !self.channels.contains_key(c)
                    && !self.opening.contains_key(c)
                    && !self.closed_channels.contains_key(c)
            })
            .filter_map(|(c, m)| segments.get(c).map(|s| (*m, s.clone())))
            .collect();
        for (listed, documents) in unheld {
            let ChannelMeta {
                channel_id,
                meta,
                last,
            } = listed;
            let held = documents
                .iter()
                .map(|(document_id, _)| *document_id)
                .chain([meta.document_id()])
                .collect();
            self.closed_channels
                .insert(channel_id, channels::ClosedChannel::watched(meta, held));
            self.listed_channels.watched.insert(channel_id);
            self.send_to_sync(SyncCommand::WatchChannel {
                channel_id,
                meta,
                meta_last: last,
                documents,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{ChannelActivity, Seq};
    use ambre_core::activity::ActivityCounters;

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// The last number the activity answer gives every document.
    const LAST: Seq = Seq::new(3);

    /// The last number the listing gives every `meta`.
    const META_LAST: Seq = Seq::new(8);

    fn list(state: &mut LogicState, metas: &[(Id<Channel>, Id<Meta>)]) {
        state.handle_logic_event(LogicEvent::MemberChannels {
            channels: Vec::new(),
            spaces: Vec::new(),
            names: Vec::new(),
            peers: Vec::new(),
            metas: metas
                .iter()
                .map(|&(channel_id, meta)| ChannelMeta {
                    channel_id,
                    meta,
                    last: Some(META_LAST),
                })
                .collect(),
        });
    }

    fn answer(
        state: &mut LogicState,
        documents: &[(Id<Channel>, Id<Document>, ChannelDocumentKind)],
    ) {
        state.handle_logic_event(LogicEvent::ChannelActivity {
            documents: documents
                .iter()
                .map(|&(channel_id, document_id, kind)| DocumentActivity {
                    channel_id,
                    document_id,
                    kind,
                    active: ambre_bus::Activity::Quiet,
                    read: Seq::ZERO,
                    last: Some(LAST),
                })
                .collect(),
        });
    }

    /// Every channel the Sync worker was asked to watch, with its documents.
    /// Each `meta` must resume after the listing's last number.
    fn watches(sync: &Receiver<SyncCommand>) -> Vec<(Id<Channel>, Id<Meta>, Segments)> {
        drain(sync)
            .into_iter()
            .filter_map(|command| match command {
                SyncCommand::WatchChannel {
                    channel_id,
                    meta,
                    meta_last,
                    documents,
                } => {
                    assert_eq!(meta_last, Some(META_LAST), "the meta's last number");
                    Some((channel_id, meta, documents))
                }
                _ => None,
            })
            .collect()
    }

    /// A channel listed with its key, and named by the activity answer, is
    /// watched once, whichever lands first. Each segment counts after its
    /// last number, and its `meta` after the listing's (`RESID-022`). Its
    /// threads are left to their own signal, and a channel listed with no
    /// key watches nothing.
    #[test]
    fn a_channel_never_opened_is_watched_once_listed_and_answered() {
        use ChannelDocumentKind::{Segment, Thread};
        let (mut state, _ui, sync) = harness();
        let (channel, keyless, meta) = (
            Id::<Channel>::mint(),
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
        );
        let (segment, thread, other) = (
            Id::<Document>::mint(),
            Id::<Document>::mint(),
            Id::<Document>::mint(),
        );
        list(&mut state, &[(channel, meta)]);
        assert!(
            watches(&sync).is_empty(),
            "watched before its segments are known"
        );
        let documents = [
            (channel, segment, Segment),
            (channel, thread, Thread),
            (keyless, other, Segment),
        ];
        answer(&mut state, &documents);
        assert_eq!(
            watches(&sync),
            vec![(channel, meta, vec![(segment, Some(LAST))])]
        );
        answer(&mut state, &documents);
        list(&mut state, &[(channel, meta)]);
        assert!(
            watches(&sync).is_empty(),
            "a watched channel is watched again"
        );

        // The answer may land first.
        let (mut state, _ui, sync) = harness();
        answer(&mut state, &documents);
        assert!(watches(&sync).is_empty());
        list(&mut state, &[(channel, meta)]);
        assert_eq!(
            watches(&sync),
            vec![(channel, meta, vec![(segment, Some(LAST))])]
        );
    }

    /// The last number of an active document is far past its cursor. The
    /// view still gets a mark, never a count (the Murmure unread spec).
    /// It would count deltas, edits and reactions among them.
    #[test]
    fn the_last_number_counts_nothing() {
        let (mut state, ui, _sync) = harness();
        let (channel, segment) = (Id::<Channel>::mint(), Id::<Document>::mint());
        state.handle_logic_event(LogicEvent::ChannelActivity {
            documents: vec![DocumentActivity {
                channel_id: channel,
                document_id: segment,
                kind: ChannelDocumentKind::Segment,
                active: ambre_bus::Activity::Active,
                read: Seq::new(2),
                last: Some(Seq::new(50)),
            }],
        });
        let mark = ChannelActivity {
            channel_id: channel,
            active: ambre_bus::Activity::Active,
            unread: None,
            mentions: None,
        };
        let shown = drain(&ui);
        assert!(
            matches!(shown.as_slice(),
                [UiEvent::ChannelActivity { channels }] if *channels == [mark]),
            "got {shown:?}"
        );
    }

    /// The first message of a channel never opened marks it and notifies,
    /// as a closed channel's does (the Murmure notifications spec). A move
    /// of its `meta` lists it again, for a segment opened since.
    #[test]
    fn the_first_message_of_a_channel_never_opened_notifies() {
        let (mut state, ui, sync) = harness();
        let (channel, meta, segment) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            Id::<Document>::mint(),
        );
        list(&mut state, &[(channel, meta)]);
        answer(
            &mut state,
            &[(channel, segment, ChannelDocumentKind::Segment)],
        );
        drain(&ui);
        drain(&sync);
        let message = ActivityCounters {
            active: ambre_bus::Activity::Active,
            content: ambre_core::activity::ContentUpdates::new(1),
            ..ActivityCounters::default()
        };
        state.handle_logic_event(LogicEvent::Activity {
            document_id: segment,
            counters: message,
        });
        let shown = drain(&ui);
        let mark = ChannelActivity {
            channel_id: channel,
            active: ambre_bus::Activity::Active,
            unread: None,
            mentions: None,
        };
        assert!(
            matches!(shown.as_slice(), [
                UiEvent::ChannelActivity { channels },
                UiEvent::NewMessage { channel_id, mention: ambre_bus::MentionState::Unmentioned, message: None },
            ] if *channels == [mark] && *channel_id == channel),
            "got {shown:?}"
        );
        state.handle_logic_event(LogicEvent::Activity {
            document_id: meta.document_id(),
            counters: ActivityCounters {
                active: ambre_bus::Activity::Active,
                ..ActivityCounters::default()
            },
        });
        assert!(
            drain(&ui)
                .iter()
                .all(|e| !matches!(e, UiEvent::NewMessage { .. })),
            "a move of meta notified"
        );
        let sent = drain(&sync);
        assert!(
            matches!(sent.as_slice(), [SyncCommand::OpenChannel { channel_id, .. }] if *channel_id == channel),
            "got {sent:?}"
        );
    }

    /// A channel the user joins in the session reaches a later listing. Its
    /// segments are asked of the relay, then watched. A channel a listing
    /// drops is no longer the user's, and its light subscriptions end.
    #[test]
    fn a_joined_channel_is_asked_for_and_a_dropped_one_ends() {
        use ChannelDocumentKind::Segment;
        let (mut state, _ui, sync) = harness();
        let (first, joined) = (Id::<Channel>::mint(), Id::<Channel>::mint());
        let (first_meta, joined_meta) = (Id::<Meta>::mint(), Id::<Meta>::mint());
        let (segment, later) = (Id::<Document>::mint(), Id::<Document>::mint());
        list(&mut state, &[(first, first_meta)]);
        answer(&mut state, &[(first, segment, Segment)]);
        drain(&sync);

        list(&mut state, &[(first, first_meta), (joined, joined_meta)]);
        let sent = drain(&sync);
        assert!(
            matches!(sent.as_slice(), [SyncCommand::FetchActivity]),
            "got {sent:?}"
        );
        answer(
            &mut state,
            &[(first, segment, Segment), (joined, later, Segment)],
        );
        assert_eq!(
            watches(&sync),
            vec![(joined, joined_meta, vec![(later, Some(LAST))])]
        );

        list(&mut state, &[(joined, joined_meta)]);
        let mut closed: Vec<Id<Document>> = drain(&sync)
            .into_iter()
            .map(|command| match command {
                SyncCommand::CloseDocument { document_id } => document_id,
                other => panic!("expected only closes, got {other:?}"),
            })
            .collect();
        closed.sort_by_key(|id| id.to_string());
        let mut expected = vec![segment, first_meta.document_id()];
        expected.sort_by_key(|id| id.to_string());
        assert_eq!(closed, expected);
    }

    /// A channel the user deleted is forgotten: a later answer that still
    /// names it watches nothing.
    #[test]
    fn a_deleted_channel_is_not_watched_again() {
        let (mut state, _ui, sync) = harness();
        let (channel, meta, segment) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            Id::<Document>::mint(),
        );
        let documents = [(channel, segment, ChannelDocumentKind::Segment)];
        list(&mut state, &[(channel, meta)]);
        answer(&mut state, &documents);
        state.deleting.insert(channel, Id::<Space>::mint());
        state.handle_logic_event(LogicEvent::ChannelDeleted {
            channel_id: channel,
        });
        drain(&sync);
        answer(&mut state, &documents);
        assert!(watches(&sync).is_empty());
    }

    /// Opening a channel watched unopened reads no local copy: it has none,
    /// so each document is fetched whole.
    #[test]
    fn opening_a_channel_watched_unopened_fetches_it() {
        let (mut state, _ui, sync) = harness();
        let (channel, meta, segment) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            Id::<Document>::mint(),
        );
        list(&mut state, &[(channel, meta)]);
        answer(
            &mut state,
            &[(channel, segment, ChannelDocumentKind::Segment)],
        );
        drain(&sync);
        state.handle_logic_command(LogicCommand::OpenChannel {
            channel_id: channel,
        });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id: channel,
            documents: vec![ambre_bus::ChannelDocument {
                document_id: meta.document_id(),
                kind: ChannelDocumentKind::Meta,
            }],
        });
        let sent = drain(&sync);
        assert!(
            matches!(sent.as_slice(), [
                SyncCommand::OpenChannel { .. },
                SyncCommand::OpenDocument { document_id, .. },
            ] if *document_id == meta.document_id()),
            "got {sent:?}"
        );
        assert!(!state.reads_channel_copy(meta.document_id()));
    }
}
