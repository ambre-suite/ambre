// ambre-logic/src/channels.rs
// ============================================================================
// Murmure channels: route a channel command to its engine, and a remote delta
// to the channel that holds its document. A channel is a set of documents,
// never an entry of `docs` (the Murmure engine spec).
// ============================================================================

use super::*;
use ambre_bus::{
    ChannelActivity, ChannelDocument, ChannelDocumentKind, ChannelId, DocumentActivity, MetaId,
    MurmureCommand, MurmureEvent,
};
#[cfg(test)]
use ambre_murmure::ChannelState;
use ambre_murmure::{Opening, OpeningChannel};

impl LogicState {
    /// Ask the relay for the documents of a channel (the Murmure engine spec).
    /// A channel already open or opening is left as it is.
    pub(crate) fn open_channel(&mut self, channel_id: ChannelId) {
        if self.channels.contains_key(&channel_id) || self.opening.contains_key(&channel_id) {
            return;
        }
        self.opening.insert(channel_id, None);
        // A channel a catalogue lists is public, and its log admits a member
        // who joined alone. Any other channel's log admits only invites
        // (`SYNC-106`).
        let self_join = self.lists_channel(channel_id);
        self.send_to_sync(SyncCommand::OpenChannel {
            channel_id,
            self_join,
        });
    }

    /// Drop a channel, open or opening, and end the session of each of its
    /// documents.
    pub(crate) fn close_channel(&mut self, channel_id: ChannelId) {
        let documents = match (
            self.channels.remove(&channel_id),
            self.opening.remove(&channel_id),
        ) {
            (Some(channel), _) => channel.document_ids(),
            (None, Some(Some(opening))) => opening.document_ids(),
            (None, _) => Vec::new(),
        };
        for document_id in documents {
            self.channel_ledgers.remove(&document_id);
            self.send_to_sync(SyncCommand::CloseDocument { document_id });
        }
    }

    /// The relay listed the documents of a channel: open its `meta` first,
    /// which names the live segments (the Murmure engine spec).
    pub(crate) fn channel_listed(&mut self, channel_id: ChannelId, documents: &[ChannelDocument]) {
        // A listing for a channel closed meanwhile, or listed twice, is stale.
        let Some(slot @ None) = self.opening.get_mut(&channel_id) else {
            return;
        };
        let Some(meta) = documents
            .iter()
            .find(|d| d.kind == ChannelDocumentKind::Meta)
        else {
            self.opening.remove(&channel_id);
            self.channel_open_failed(channel_id, "the channel has no meta document".into());
            return;
        };
        *slot = Some(OpeningChannel::new(
            channel_id,
            MetaId::from_document(meta.document_id),
        ));
        self.send_to_sync(SyncCommand::OpenDocument {
            document_id: meta.document_id,
            doc_type: DocType::Murmure,
        });
    }

    /// Tell the view a channel could not open. Nothing of it is held.
    pub(crate) fn channel_open_failed(&self, channel_id: ChannelId, message: String) {
        tracing::warn!("Logic: channel {channel_id} failed to open: {message}");
        self.send_to_ui(UiEvent::Channel {
            channel_id,
            event: MurmureEvent::OpenFailed { message },
        });
    }

    /// A document of a channel has all its bytes. Returns `false` when no
    /// channel, open or opening, asked for it.
    pub(crate) fn channel_document_ready(&mut self, document_id: DocumentId) -> bool {
        if let Some((&channel_id, channel)) =
            self.channels.iter_mut().find(|(_, c)| c.holds(document_id))
        {
            // An older segment the view asked for. A second
            // ready for a live document, after a reconnect, changes nothing.
            if channel.is_pending(document_id) {
                match channel.older_ready(document_id) {
                    Ok(event) => self.send_to_ui(UiEvent::Channel { channel_id, event }),
                    Err(error) => {
                        tracing::warn!("Logic: older segment {document_id} refused: {error}");
                        self.send_to_sync(SyncCommand::CloseDocument { document_id });
                    }
                }
            }
            return true;
        }
        let Some((&channel_id, opening)) = self.opening.iter_mut().find_map(|(id, o)| {
            o.as_mut()
                .filter(|o| o.expects(document_id))
                .map(|o| (id, o))
        }) else {
            return false;
        };
        match opening.ready(document_id) {
            Ok(Opening::Waiting) => {}
            Ok(Opening::OpenSegments(segments)) => {
                for segment in segments {
                    self.send_to_sync(SyncCommand::OpenDocument {
                        document_id: segment.document_id(),
                        doc_type: DocType::Murmure,
                    });
                }
            }
            Ok(Opening::Open(state)) => {
                self.opening.remove(&channel_id);
                // The closing segment first, so the view draws the history in
                // order.
                for segment in [state.closing_id(), Some(state.active_id())]
                    .into_iter()
                    .flatten()
                {
                    match state.messages(segment) {
                        Ok(messages) => self.send_to_ui(UiEvent::Channel {
                            channel_id,
                            event: MurmureEvent::Messages { segment, messages },
                        }),
                        Err(error) => tracing::warn!("Logic: segment {segment}: {error}"),
                    }
                }
                self.channels.insert(channel_id, *state);
            }
            Err(error) => self.end_opening(channel_id, error.to_string()),
        }
        true
    }

    /// A document of a channel could not open. Returns `false` when no
    /// channel asked for it.
    pub(crate) fn channel_document_failed(
        &mut self,
        document_id: DocumentId,
        message: &str,
    ) -> bool {
        if let Some(channel) = self.channels.values_mut().find(|c| c.holds(document_id)) {
            // An older segment is given up, so the view can ask again. A live
            // document of an open channel keeps what it holds.
            channel.cancel_older(document_id);
            tracing::warn!("Logic: channel document {document_id} failed to open: {message}");
            return true;
        }
        let Some(channel_id) = self.opening.iter().find_map(|(id, o)| {
            o.as_ref()
                .is_some_and(|o| o.expects(document_id))
                .then_some(*id)
        }) else {
            return false;
        };
        self.end_opening(channel_id, message.to_owned());
        true
    }

    /// Drop an opening that failed, end the session of each document it asked
    /// for, and tell the view.
    fn end_opening(&mut self, channel_id: ChannelId, message: String) {
        if let Some(Some(opening)) = self.opening.remove(&channel_id) {
            for document_id in opening.document_ids() {
                self.send_to_sync(SyncCommand::CloseDocument { document_id });
            }
        }
        self.channel_open_failed(channel_id, message);
    }

    /// The snapshot of a channel document, for the relay's compaction, with
    /// the last delta it folds in (`SRV-083`).
    pub(crate) fn channel_snapshot(&self, document_id: DocumentId) -> Option<(Vec<u8>, u64)> {
        let channel = self.channels.values().find(|c| c.holds(document_id))?;
        let payload = channel
            .snapshot(document_id)
            .map_err(|e| tracing::warn!("Logic: snapshot of {document_id} failed: {e}"))
            .ok()?;
        let covered = self
            .channel_ledgers
            .get(&document_id)
            .map_or(0, snapshot_check::SeqLedger::contiguous_end);
        Some((payload, covered))
    }

    /// Note the relay's number of a delta for a channel or a space document,
    /// open or opening. A document neither holds is not noted.
    pub(crate) fn record_channel_seq(
        &mut self,
        document_id: DocumentId,
        seq: u64,
        payload: Option<&[u8]>,
    ) {
        let space = ambre_bus::SpaceId::from_document(document_id);
        let held = self.channels.values().any(|c| c.holds(document_id))
            || self.opening.values().flatten().any(|o| o.expects(document_id))
            || self.spaces.contains_key(&space)
            || self.space_opening.contains_key(&space);
        if held {
            self.channel_ledgers
                .entry(document_id)
                .or_default()
                .record(seq, payload);
        }
    }

    /// Run one command on an open channel, and emit what it produced
    /// (`BUS-027`).
    ///
    /// The delta leaves for the Sync worker under the document it names, with
    /// its activity summary (the Murmure messages spec). The event goes to the view.
    pub(crate) fn handle_channel_command(
        &mut self,
        channel_id: ChannelId,
        command: MurmureCommand,
    ) {
        let Some(channel) = self.channels.get_mut(&channel_id) else {
            tracing::warn!("Logic: command for channel {channel_id}, which is not open; dropped");
            return;
        };
        match command {
            MurmureCommand::SendMessage { draft, now } => {
                // The author is this session's user. Before `SessionReady`
                // there is none, and a message with no author is refused
                // rather than written under a placeholder (the Murmure schema).
                let Some(author) = self.my_user_id else {
                    tracing::warn!(
                        "Logic: message for channel {channel_id} before the session; dropped"
                    );
                    return;
                };
                match channel.send(&draft, author, now) {
                    Ok(sent) => {
                        self.send_to_sync(SyncCommand::SendUpdate {
                            document_id: sent.segment.document_id(),
                            payload: sent.delta,
                            summary: Some(sent.summary),
                        });
                        self.send_to_ui(UiEvent::Channel {
                            channel_id,
                            event: sent.event,
                        });
                    }
                    Err(error) => {
                        tracing::warn!("Logic: send on channel {channel_id} refused: {error}");
                    }
                }
            }
            MurmureCommand::LoadOlder { segment } => {
                // Nothing to ask for when the segment is the oldest, or the
                // one before it is loaded or on its way.
                if let Some(older) = channel.request_older(segment) {
                    self.send_to_sync(SyncCommand::OpenDocument {
                        document_id: older.document_id(),
                        doc_type: DocType::Murmure,
                    });
                }
            }
            MurmureCommand::MarkRead => {
                // The latest message is on screen: every segment before it is
                // read too, loaded or not (the Murmure unread spec). A relay record, so
                // a viewer moves its cursor as a writer does.
                let live = [Some(channel.active_id()), channel.closing_id()]
                    .into_iter()
                    .flatten()
                    .collect();
                let finals = channel.final_segments();
                self.send_to_sync(SyncCommand::MarkRead { live, finals });
                self.send_to_ui(UiEvent::Activity {
                    channels: vec![ChannelActivity {
                        channel_id,
                        active: false,
                    }],
                });
            }
        }
    }

    /// Hand the view the relay's activity answer, one mark per channel: a
    /// channel is active when any of its documents is (the Murmure
    /// unread spec).
    ///
    /// Opens nothing: the answer is a yes or no per document,
    /// and a mark needs no more.
    pub(crate) fn channel_activity(&self, documents: &[DocumentActivity]) {
        let mut channels: Vec<ChannelActivity> = Vec::new();
        let mut index: HashMap<ChannelId, usize> = HashMap::new();
        for document in documents {
            match index.get(&document.channel_id) {
                Some(&i) => {
                    if let Some(channel) = channels.get_mut(i) {
                        channel.active |= document.active;
                    }
                }
                None => {
                    index.insert(document.channel_id, channels.len());
                    channels.push(ChannelActivity {
                        channel_id: document.channel_id,
                        active: document.active,
                    });
                }
            }
        }
        self.send_to_ui(UiEvent::Activity { channels });
    }

    /// Apply remote payloads to the channel that holds `document_id`
    /// (the Murmure engine spec). Returns `false` when no open channel holds it, so the
    /// caller reports the drop as it does for a document.
    pub(crate) fn apply_channel_remote(
        &mut self,
        document_id: DocumentId,
        payloads: &[&[u8]],
    ) -> bool {
        // A channel still opening keeps the bytes until the document is ready.
        if let Some(opening) = self
            .opening
            .values_mut()
            .flatten()
            .find(|o| o.expects(document_id))
        {
            for payload in payloads {
                opening.receive(document_id, payload);
            }
            return true;
        }
        let Some((&channel_id, channel)) = self
            .channels
            .iter_mut()
            .find(|(_, channel)| channel.holds(document_id))
        else {
            return false;
        };
        let mut events = Vec::new();
        for payload in payloads {
            match channel.apply_remote(document_id, payload) {
                Ok(produced) => events.extend(produced),
                Err(error) => {
                    tracing::warn!("Logic: delta for {document_id} refused: {error}");
                }
            }
        }
        // One segment answers with its whole list, so only the last answer of
        // a batch is news to the view.
        if let Some(event) = events.pop() {
            self.send_to_ui(UiEvent::Channel { channel_id, event });
        }
        true
    }

    /// Hold an open channel, as the staged open does, for a test that needs
    /// one without the relay's round trips.
    #[cfg(test)]
    pub(crate) fn hold_channel(&mut self, channel: ChannelState) {
        self.channels.insert(channel.channel_id(), channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::{
        ActivityKind, MentionTarget, MessageDraft, MurmureEvent, RestEvent, SegmentId, Timestamp,
    };
    use ambre_murmure::ChannelType;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn harness() -> (LogicState, Receiver<UiEvent>, Receiver<SyncCommand>) {
        let (to_ui, ui_rx) = test_link();
        let (to_sync, sync_rx) = test_link();
        let (to_persist, persist_rx) = test_link();
        std::mem::forget(persist_rx);
        (LogicState::new(to_ui, to_sync, to_persist), ui_rx, sync_rx)
    }

    fn signed_in(state: &mut LogicState, user: UserId) {
        state.handle_logic_event(LogicEvent::Rest {
            id: RequestId(0),
            evt: RestEvent::SessionReady {
                user_id: user,
                display_name: "alice".to_owned(),
                email: "alice@example.com".to_owned(),
            },
        });
    }

    fn open_channel(state: &mut LogicState) -> (ChannelId, SegmentId) {
        let channel_id = ChannelId::new();
        let channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = channel.active_id();
        state.hold_channel(channel);
        (channel_id, active)
    }

    fn send(text: &str, mentions: Vec<MentionTarget>) -> MurmureCommand {
        MurmureCommand::SendMessage {
            draft: MessageDraft {
                text: text.to_owned(),
                mentions,
            },
            now: NOW,
        }
    }

    #[test]
    fn a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event() {
        let (mut state, ui, sync) = harness();
        let alice = UserId::new();
        signed_in(&mut state, alice);
        let _ = std::iter::from_fn(|| ui.try_recv().ok()).count();
        let (channel_id, active) = open_channel(&mut state);
        let bob = UserId::new();

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("hi @bob", vec![MentionTarget::User(bob)]),
        });

        let sent: Vec<SyncCommand> = std::iter::from_fn(|| sync.try_recv().ok()).collect();
        let [
            SyncCommand::SendUpdate {
                document_id,
                payload,
                summary: Some(summary),
            },
        ] = sent.as_slice()
        else {
            panic!("one update with a summary, got {sent:?}");
        };
        assert_eq!(*document_id, active.document_id());
        assert!(!payload.is_empty());
        assert_eq!(summary.kind, ActivityKind::Content);
        assert_eq!(summary.mentions, [bob]);

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("one channel event, got {shown:?}");
        };
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, active);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].author, alice);
        assert_eq!(messages[0].text, "hi @bob");
    }

    #[test]
    fn a_message_before_the_session_is_dropped() {
        let (mut state, ui, sync) = harness();
        let (channel_id, _) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("too early", Vec::new()),
        });
        assert!(sync.try_recv().is_err());
        assert!(ui.try_recv().is_err());
    }

    #[test]
    fn a_remote_delta_reaches_the_channel_that_holds_its_document() {
        let (mut alice, _, alice_sync) = harness();
        signed_in(&mut alice, UserId::new());
        let channel_id = ChannelId::new();
        let original =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        let active = original.active_id();
        let segments = std::collections::HashMap::from([(
            active,
            original.snapshot(active.document_id()).expect("active"),
        )]);
        let meta = original
            .snapshot(original.meta_id().document_id())
            .expect("meta");
        let copy =
            ChannelState::open(channel_id, original.meta_id(), &meta, &segments).expect("open");
        alice.hold_channel(original);
        let (mut bob, bob_ui, _) = harness();
        bob.hold_channel(copy);

        alice.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: send("over the wire", Vec::new()),
        });
        let Ok(SyncCommand::SendUpdate {
            document_id,
            payload,
            ..
        }) = alice_sync.try_recv()
        else {
            panic!("alice sends one update");
        };
        // The PeerID Sync would derive from alice's signer.
        let peer = *loro::LoroDoc::decode_import_blob_meta(&payload, false)
            .expect("an update")
            .partial_end_vv
            .keys()
            .next()
            .expect("one peer");
        bob.handle_logic_event(LogicEvent::RemoteUpdate {
            document_id,
            delta: ambre_bus::SignedDelta {
                seq: 1,
                payload,
                nonce: ambre_core::SessionNonce([0; 16]),
                peer,
                user: None,
            },
        });

        let Ok(UiEvent::Channel {
            event: MurmureEvent::Messages { messages, .. },
            ..
        }) = bob_ui.try_recv()
        else {
            panic!("bob's view hears of the message");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "over the wire");
    }

    #[test]
    fn closing_a_channel_drops_its_documents() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert!(state.channels.is_empty());
        assert!(!state.apply_channel_remote(active.document_id(), &[&[]]));
        let closed = sent(&sync);
        assert!(closed.contains(&Sent::Close(active.document_id())));
        assert!(closed.iter().all(|c| matches!(c, Sent::Close(_))));
    }

    // --- The staged open over the relay ---

    fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// A channel as the relay stores it: one message in its active segment.
    fn stored_channel() -> (ChannelId, ChannelState) {
        let channel_id = ChannelId::new();
        let mut channel =
            ChannelState::create(channel_id, ChannelType::Public, None, NOW).expect("create");
        channel
            .send(
                &MessageDraft {
                    text: "stored".to_owned(),
                    mentions: Vec::new(),
                },
                UserId::new(),
                NOW,
            )
            .expect("send");
        (channel_id, channel)
    }

    fn listing(channel: &ChannelState) -> Vec<ChannelDocument> {
        vec![
            ChannelDocument {
                document_id: channel.meta_id().document_id(),
                kind: ChannelDocumentKind::Meta,
            },
            ChannelDocument {
                document_id: channel.active_id().document_id(),
                kind: ChannelDocumentKind::Segment,
            },
        ]
    }

    /// The relay serves a document: its snapshot, then its ready signal.
    fn serve(state: &mut LogicState, stored: &ChannelState, document_id: DocumentId) {
        state.handle_logic_event(LogicEvent::RemoteSnapshot {
            document_id,
            payload: stored.snapshot(document_id).expect("snapshot"),
        });
        state.handle_logic_event(LogicEvent::DocumentReady {
            document_id,
            role: ambre_bus::Role::Editor,
        });
    }

    /// What Logic asked the Sync worker, reduced to the calls these tests
    /// follow: `SyncCommand` carries secrets, so it has no `PartialEq`.
    #[derive(Debug, PartialEq)]
    enum Sent {
        List(ChannelId),
        Open(DocumentId),
        Close(DocumentId),
        Other,
    }

    fn sent(rx: &Receiver<SyncCommand>) -> Vec<Sent> {
        drain(rx)
            .into_iter()
            .map(|c| match c {
                SyncCommand::OpenChannel { channel_id, .. } => Sent::List(channel_id),
                SyncCommand::OpenDocument {
                    document_id,
                    doc_type: DocType::Murmure,
                } => Sent::Open(document_id),
                SyncCommand::CloseDocument { document_id } => Sent::Close(document_id),
                _ => Sent::Other,
            })
            .collect()
    }

    #[test]
    fn a_channel_opens_its_meta_then_its_live_segments_then_shows_them() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();

        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::List(channel_id)]);

        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert_eq!(
            sent(&sync),
            [Sent::Open(stored.meta_id().document_id())],
            "meta first"
        );

        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Open(stored.active_id().document_id())]);
        assert!(drain(&ui).is_empty(), "nothing of the channel shows yet");

        serve(&mut state, &stored, stored.active_id().document_id());
        let shown = drain(&ui);
        let [
            UiEvent::Channel {
                channel_id: shown_channel,
                event: MurmureEvent::Messages { segment, messages },
            },
        ] = shown.as_slice()
        else {
            panic!("the active segment's messages, got {shown:?}");
        };
        assert_eq!(*shown_channel, channel_id);
        assert_eq!(*segment, stored.active_id());
        assert_eq!(messages[0].text, "stored");
        assert!(state.channels.contains_key(&channel_id));
        assert!(state.opening.is_empty());
    }

    /// A channel no catalogue of this client lists admits invites alone, so
    /// its member list counts no self-join.
    #[test]
    fn a_channel_no_catalogue_lists_admits_no_self_join() {
        let (mut state, _, sync) = harness();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        assert!(matches!(
            drain(&sync).as_slice(),
            [SyncCommand::OpenChannel {
                self_join: false,
                ..
            }]
        ));
    }

    #[test]
    fn a_refused_listing_tells_the_view_and_holds_nothing() {
        let (mut state, ui, _) = harness();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelOpenFailed {
            channel_id,
            message: "403".to_owned(),
        });
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed { .. },
                ..
            }]
        ));
        assert!(state.opening.is_empty());
    }

    #[test]
    fn a_document_that_fails_to_open_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);

        state.handle_logic_event(LogicEvent::DocumentOpenFailed {
            document_id: stored.meta_id().document_id(),
            message: "cannot unwrap document key".to_owned(),
        });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(
            matches!(
                drain(&ui).as_slice(),
                [UiEvent::Channel {
                    event: MurmureEvent::OpenFailed { .. },
                    ..
                }]
            ),
            "the channel fails, and no bare auth error"
        );
        assert!(state.opening.is_empty());
    }

    #[test]
    fn a_meta_of_another_channel_ends_the_opening() {
        let (mut state, ui, sync) = harness();
        let (_, stored) = stored_channel();
        let channel_id = ChannelId::new();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        serve(&mut state, &stored, stored.meta_id().document_id());
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        assert!(matches!(
            drain(&ui).as_slice(),
            [UiEvent::Channel {
                event: MurmureEvent::OpenFailed { .. },
                ..
            }]
        ));
        assert!(state.channels.is_empty());
    }

    #[test]
    fn closing_a_channel_still_opening_ends_its_documents() {
        let (mut state, _, sync) = harness();
        let (channel_id, stored) = stored_channel();
        state.handle_logic_command(LogicCommand::OpenChannel { channel_id });
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        let _ = drain(&sync);
        state.handle_logic_command(LogicCommand::CloseChannel { channel_id });
        assert_eq!(sent(&sync), [Sent::Close(stored.meta_id().document_id())]);
        // A listing that lands after the close is stale.
        state.handle_logic_event(LogicEvent::ChannelDocuments {
            channel_id,
            documents: listing(&stored),
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn asking_for_the_segment_before_the_oldest_asks_nothing() {
        let (mut state, _, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);
        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::LoadOlder { segment: active },
        });
        assert!(drain(&sync).is_empty());
    }

    #[test]
    fn marking_a_channel_read_moves_its_live_segments_and_clears_its_mark() {
        let (mut state, ui, sync) = harness();
        let (channel_id, active) = open_channel(&mut state);

        state.handle_logic_command(LogicCommand::Channel {
            channel_id,
            command: MurmureCommand::MarkRead,
        });

        let sent = drain(&sync);
        let [SyncCommand::MarkRead { live, finals }] = sent.as_slice() else {
            panic!("one mark read, got {sent:?}");
        };
        assert_eq!(*live, [active]);
        assert!(finals.is_empty());
        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [UiEvent::Activity { channels }] = shown.as_slice() else {
            panic!("one activity event, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [ChannelActivity {
                channel_id,
                active: false
            }]
        );
    }

    #[test]
    fn an_activity_answer_marks_each_channel_once_and_opens_nothing() {
        let (mut state, ui, sync) = harness();
        let (quiet, busy) = (ChannelId::new(), ChannelId::new());
        let entry = |channel_id, active| DocumentActivity {
            channel_id,
            document_id: DocumentId::new(),
            active,
        };

        state.handle_logic_event(LogicEvent::Activity {
            documents: vec![
                entry(busy, false),
                entry(quiet, false),
                entry(busy, true),
                entry(quiet, false),
            ],
        });

        let shown: Vec<UiEvent> = std::iter::from_fn(|| ui.try_recv().ok()).collect();
        let [UiEvent::Activity { channels }] = shown.as_slice() else {
            panic!("one activity event, got {shown:?}");
        };
        assert_eq!(
            *channels,
            [
                ChannelActivity {
                    channel_id: busy,
                    active: true
                },
                ChannelActivity {
                    channel_id: quiet,
                    active: false
                },
            ]
        );
        assert!(
            drain(&sync).is_empty(),
            "an activity answer opens no document"
        );
    }

    #[test]
    fn the_activity_commands_reach_the_sync_worker() {
        let (mut state, _, sync) = harness();
        state.handle_logic_command(LogicCommand::FetchActivity);
        state.handle_logic_command(LogicCommand::MarkAllRead);
        let sent = drain(&sync);
        assert!(
            matches!(
                sent.as_slice(),
                [SyncCommand::FetchActivity, SyncCommand::MarkAllRead]
            ),
            "got {sent:?}"
        );
    }
}
