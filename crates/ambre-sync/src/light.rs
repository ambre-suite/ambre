//! The light subscription of a document the Logic thread evicted
//! (`ambre-spec-residency.md`, `RESID-004`).
//!
//! An evicted document keeps its key, its counters and its cursors, and
//! nothing else. Its updates are opened for their activity summary alone, and
//! never reach the Logic thread: no Loro document is loaded to count them
//! (`RESID-016`).

use super::*;

use ambre_bus::{MetaId, SegmentId};
use ambre_core::activity::ActivityCounters;

/// What the Sync worker keeps of a document the Logic thread evicted.
pub(crate) struct LightSubscription {
    /// Opens the summaries. A frame sealed under another key is counted as
    /// activity alone, and the reopen reads it whole.
    doc_key: DocumentKey,
    /// The kind of the document, for the reopen.
    doc_type: DocType,
    /// Whether the relay holds the document, so the socket subscribes it.
    on_relay: bool,
    /// The highest number the evicted copy holds with no gap below it. A
    /// reopen replays only what follows it (`RESID-010`).
    horizon: Seq,
    /// The highest number read, the cursor a reconnect resumes after.
    seen: Seq,
    /// The activity read from the summaries since the eviction.
    counters: ActivityCounters,
    /// Whether a reconnect asks for what it missed. One watched from the
    /// head asks for nothing until its first delta gives it a number.
    replays: bool,
}

impl LightSubscription {
    /// The light subscription a full session leaves behind. The horizon never
    /// passes what the session itself received.
    fn of(doc: SyncDocumentState, applied: Seq) -> Self {
        Self {
            horizon: Seq(applied.0.min(doc.last_seq.0)),
            seen: doc.last_seq,
            doc_key: doc.doc_key,
            doc_type: doc.doc_type,
            on_relay: doc.server_created,
            counters: ActivityCounters::default(),
            replays: true,
        }
    }

    /// Counts one update from its summary alone (`RESID-014`, `RESID-015`).
    /// An update that does not open, or carries no summary, marks activity
    /// and counts nothing. A number already read returns `None`, and so does
    /// an update `mine` signed, which is nothing new to this user (`RESID-021`).
    fn count(
        &mut self,
        seq: Seq,
        update: &Update,
        me: Option<UserId>,
        mine: Option<&PublicKey>,
    ) -> Option<ActivityCounters> {
        if seq.0 <= self.seen.0 {
            return None;
        }
        self.seen = seq;
        self.replays = true;
        if mine == Some(&update.public_data.pub_key) {
            return None;
        }
        let summary = pipeline::open_update(update, &self.doc_key)
            .ok()
            .and_then(|body| pipeline::split_update_body(body).ok())
            .and_then(|body| body.summary);
        self.counters.advance(summary.as_ref(), me);
        Some(self.counters)
    }
}

/// Whether a reopen resumes from the copy the client holds, rather than from
/// the relay's snapshot (`RESID-010`). It resumes when that snapshot folds in
/// nothing past the horizon.
pub(crate) fn resumes(horizon: Seq, relay_covered: u64) -> bool {
    relay_covered <= horizon.0
}

impl SyncState {
    /// The Logic thread evicted a document (`RESID-006`). The full session
    /// becomes a light subscription once the relay acknowledged every frame
    /// this client sent for it (`RESID-009`).
    pub(crate) fn handle_release_document(&mut self, document_id: DocumentId, applied: Seq) {
        let Some(doc) = self.documents.get_mut(&document_id) else {
            return;
        };
        doc.released = Some(applied);
        self.settle_release(document_id);
    }

    /// Turns a released session into a light subscription, unless a frame
    /// is still unacknowledged. The echo of that frame calls it again.
    pub(crate) fn settle_release(&mut self, document_id: DocumentId) {
        let unsent_here = self
            .pending_pre_session_updates
            .iter()
            .any(|(doc, ..)| *doc == document_id);
        let Some(doc) = self.documents.get(&document_id) else {
            return;
        };
        let Some(applied) = doc.released else {
            return;
        };
        if unsent_here || doc.creating || !doc.pending_updates.is_empty() || !doc.rekey.settled() {
            tracing::debug!("[Sync] release of doc={document_id} waits on unsent frames");
            return;
        }
        let Some(doc) = self.documents.remove(&document_id) else {
            return;
        };
        self.session_nonces.remove(&document_id);
        self.light
            .insert(document_id, LightSubscription::of(doc, applied));
        tracing::info!("Sync: doc={document_id} kept as a light subscription");
        // The local copy now holds nothing the relay lacks (`RESID-019`).
        self.send_to_logic(LogicEvent::Released { document_id });
    }

    /// Counts an update of a document held as a light subscription. Returns
    /// whether the document is one.
    pub(crate) fn count_light(&mut self, seq: Seq, update: &Update) -> bool {
        let document_id = update.public_data.document_id;
        let me = self.user_id;
        let mine = self.author_pub_key();
        let Some(light) = self.light.get_mut(&document_id) else {
            return false;
        };
        if let Some(counters) = light.count(seq, update, me, mine.as_ref()) {
            self.send_to_logic(LogicEvent::Activity {
                document_id,
                counters,
            });
        }
        true
    }

    /// Counts a segment of a closed channel from its first number, as a
    /// light subscription under the key of `meta` (`SYNC-162`). A segment
    /// already held, in full or light, is left as it is. A `meta` this
    /// worker holds no key for watches nothing.
    pub(crate) fn handle_watch_segment(&mut self, segment: SegmentId, meta: MetaId) {
        self.watch_light(segment.document_id(), meta, Seq(0));
    }

    /// Counts a thread whose root is not on screen from the number after
    /// `after`, its read cursor (the Murmure threads spec). Its summaries are
    /// read, and no document is loaded.
    pub(crate) fn handle_watch_thread(&mut self, thread: DocumentId, meta: MetaId, after: Seq) {
        self.watch_light(thread, meta, after);
    }

    /// Holds `document_id` as a light subscription under the key of `meta`,
    /// and asks the relay for what follows `after`. A document already held,
    /// in full or light, is left as it is.
    fn watch_light(&mut self, document_id: DocumentId, meta: MetaId, after: Seq) {
        if self.documents.contains_key(&document_id) || self.light.contains_key(&document_id) {
            return;
        }
        let meta = meta.document_id();
        let Some(doc_key) = self
            .light
            .get(&meta)
            .map(|light| light.doc_key.clone())
            .or_else(|| self.documents.get(&meta).map(|doc| doc.doc_key.clone()))
        else {
            tracing::warn!("[Sync] no key to watch document {document_id}");
            return;
        };
        self.light.insert(
            document_id,
            LightSubscription {
                doc_key,
                doc_type: DocType::Murmure,
                on_relay: true,
                horizon: after,
                seen: after,
                counters: ActivityCounters::default(),
                replays: true,
            },
        );
        self.ws_send(&WsClientMsg::Subscribe { document_id });
        self.ws_send(&WsClientMsg::RequestMissing {
            document_id,
            after_seq: after,
        });
    }

    /// Counts a channel this session never opened, under the key of its
    /// `meta` the latest listing wrapped (the Murmure notifications spec).
    /// Each document counts after the last number beside it (`RESID-022`),
    /// and its `meta` from the next delta. A channel the listing gave no key
    /// for watches nothing.
    pub(crate) fn handle_watch_channel(
        &mut self,
        channel_id: ChannelId,
        meta: MetaId,
        documents: &[(DocumentId, Option<Seq>)],
    ) {
        let Some(doc_key) = self
            .listed_keys
            .get(&channel_id)
            .filter(|(listed, _)| *listed == meta)
            .map(|(_, key)| key.clone())
        else {
            tracing::warn!("[Sync] no listed key to watch channel {channel_id}");
            return;
        };
        for &(document_id, last) in std::iter::once(&(meta.document_id(), None)).chain(documents) {
            self.watch_unheld(document_id, &doc_key, last);
        }
    }

    /// Holds one document of a channel never opened as a light subscription.
    /// No copy of it exists, so its horizon is the first number: an open
    /// fetches it whole (`RESID-010`). The relay is asked for what follows
    /// `last`, and for nothing when there is none: the head.
    fn watch_unheld(&mut self, document_id: DocumentId, key: &DocumentKey, last: Option<Seq>) {
        if self.documents.contains_key(&document_id) || self.light.contains_key(&document_id) {
            return;
        }
        self.light.insert(
            document_id,
            LightSubscription {
                doc_key: key.clone(),
                doc_type: DocType::Murmure,
                on_relay: true,
                horizon: Seq(0),
                seen: last.unwrap_or(Seq(0)),
                counters: ActivityCounters::default(),
                replays: last.is_some(),
            },
        );
        self.ws_send(&WsClientMsg::Subscribe { document_id });
        if let Some(after_seq) = last {
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            });
        }
    }

    /// A thread took a delta (the Murmure threads spec). A thread held in
    /// full or light hears it on its own topic. Any other is named to the
    /// Logic thread, which decides what to load.
    pub(crate) fn thread_activity(&mut self, document_id: DocumentId) {
        if self.documents.contains_key(&document_id) || self.light.contains_key(&document_id) {
            return;
        }
        self.send_to_logic(LogicEvent::ThreadActivity { document_id });
    }

    /// Takes the light subscription of a document being reopened: its kind
    /// and the horizon the reopen resumes after.
    pub(crate) fn take_light(&mut self, document_id: DocumentId) -> Option<Seq> {
        self.light.remove(&document_id).map(|light| light.horizon)
    }

    /// The light subscriptions the socket subscribes and replays, with the
    /// cursor each resumes after.
    pub(crate) fn light_cursors(&self) -> Vec<(DocumentId, Seq)> {
        self.light
            .iter()
            .filter(|(_, light)| light.on_relay && light.replays)
            .map(|(id, light)| (*id, light.seen))
            .collect()
    }

    /// The light subscriptions watched from the head that took no delta
    /// yet: the socket subscribes them and asks for nothing.
    pub(crate) fn light_heads(&self) -> Vec<DocumentId> {
        self.light
            .iter()
            .filter(|(_, light)| light.on_relay && !light.replays)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The cursor one light subscription resumes after, if the relay holds it.
    pub(crate) fn light_cursor(&self, document_id: DocumentId) -> Option<Seq> {
        self.light
            .get(&document_id)
            .filter(|light| light.on_relay)
            .map(|light| light.seen)
    }

    /// The kind of a document held as a light subscription.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn light_doc_type(&self, document_id: DocumentId) -> Option<DocType> {
        self.light.get(&document_id).map(|light| light.doc_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::activity::{ActivityKind, ActivitySummary};

    const NONCE: SessionNonce = SessionNonce([6; 16]);

    /// A state with one open document, released at `applied`.
    fn released(
        applied: u64,
    ) -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        DocumentId,
        DocumentKey,
        SnapshotId,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        state.user_id = Some(UserId::from_bytes([7; 16]));
        let document_id = DocumentId::new();
        let doc_key = DocumentKey::generate();
        let mut doc = SyncDocumentState::new(document_id, doc_key.clone(), DocType::Trame, true);
        doc.last_seq = Seq(9);
        let snapshot = doc.current_snapshot_id;
        state.documents.insert(document_id, doc);
        state.session_nonces.insert(document_id, NONCE);
        state.handle_command(SyncCommand::ReleaseDocument {
            document_id,
            applied_seq: applied,
        });
        (state, rx, document_id, doc_key, snapshot)
    }

    fn update(
        document_id: DocumentId,
        snapshot_id: SnapshotId,
        doc_key: &DocumentKey,
        summary: Option<&ActivitySummary>,
    ) -> Update {
        let signing_key = MasterSeed::from_array([2; 32]).derive_signing_key();
        let public_data = UpdatePublicData {
            document_id,
            pub_key: PublicKey::from_array(signing_key.verifying_key().to_bytes()),
            key_id: doc_key.id(),
            snapshot_id,
            clock: Clock(1),
        };
        let body = pipeline::update_body(&NONCE, summary, b"delta").expect("within bounds");
        pipeline::seal_update(&body, public_data, doc_key, &signing_key).expect("seal")
    }

    fn mention() -> ActivitySummary {
        ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![UserId::from_bytes([7; 16])],
            mentions_all: false,
            anchor: None,
        }
    }

    fn drain(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> Vec<LogicEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// Holds `RESID-004`: a released document keeps its key, its counters
    /// and its cursors, and its full session is gone.
    #[test]
    fn an_evicted_document_costs_only_a_light_subscription() {
        let (state, _rx, document_id, _, _) = released(5);
        assert_eq!(state.documents_len(), 0);
        assert!(!state.session_nonces.contains_key(&document_id));
        assert_eq!(state.light_doc_type(document_id), Some(DocType::Trame));
        assert!(std::mem::size_of::<LightSubscription>() < 1024);
        assert_eq!(state.light_cursors(), vec![(document_id, Seq(9))]);
    }

    /// Holds `RESID-014`, `RESID-005` and `RESID-016`. An update of a light
    /// document moves its counters from the summary alone. Nothing reaches
    /// the Logic thread but the counters.
    #[test]
    fn a_light_update_moves_its_counters_from_the_summary() {
        let (mut state, rx, document_id, doc_key, snapshot) = released(9);
        drain(&rx);
        state.receive_update(
            Seq(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        let events = drain(&rx);
        let [LogicEvent::Activity { counters, .. }] = events.as_slice() else {
            panic!("expected the counters alone, got {events:?}");
        };
        assert_eq!((counters.content, counters.mentions), (1, 1));
        assert_eq!(state.documents_len(), 0, "counting loaded the document");
        state.receive_update(
            Seq(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert!(drain(&rx).is_empty(), "a number read twice counted twice");
    }

    /// Holds `RESID-015`: an update with no summary, or one that does not
    /// open, marks the document active and counts nothing.
    #[test]
    fn a_light_update_with_no_summary_marks_activity_alone() {
        let (mut state, rx, document_id, doc_key, snapshot) = released(9);
        drain(&rx);
        state.receive_update(Seq(10), update(document_id, snapshot, &doc_key, None));
        let stranger = DocumentKey::generate();
        state.receive_update(
            Seq(11),
            update(document_id, snapshot, &stranger, Some(&mention())),
        );
        let counters: Vec<ActivityCounters> = drain(&rx)
            .into_iter()
            .filter_map(|event| match event {
                LogicEvent::Activity { counters, .. } => Some(counters),
                _ => None,
            })
            .collect();
        let active = ActivityCounters {
            active: true,
            ..ActivityCounters::default()
        };
        assert_eq!(counters, vec![active, active]);
    }

    /// Holds `RESID-021`: an update this user signed, from this device or
    /// another, moves no counter and marks nothing.
    #[test]
    fn an_update_this_user_signed_counts_nothing() {
        let (mut state, rx, document_id, doc_key, snapshot) = released(9);
        drain(&rx);
        state.signing_key = Some(MasterSeed::from_array([2; 32]).derive_signing_key());
        state.receive_update(
            Seq(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert!(drain(&rx).is_empty(), "the user's own update counted");
        state.signing_key = Some(MasterSeed::from_array([3; 32]).derive_signing_key());
        state.receive_update(
            Seq(11),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert_eq!(
            drain(&rx).len(),
            1,
            "another member's update counted nothing"
        );
    }

    /// Holds `RESID-009`: a release waits while a frame this client sent is
    /// unacknowledged, and lands with its echo. Only then does the local
    /// store hear that the relay holds every change (`RESID-019`).
    #[test]
    fn a_release_waits_for_the_echo_of_every_frame_sent() {
        let released = |events: Vec<LogicEvent>| {
            events
                .iter()
                .any(|event| matches!(event, LogicEvent::Released { .. }))
        };
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        let document_id = DocumentId::new();
        let doc_key = DocumentKey::generate();
        let doc = SyncDocumentState::new(document_id, doc_key.clone(), DocType::Trame, true);
        let snapshot = doc.current_snapshot_id;
        state.documents.insert(document_id, doc);
        let echo = update(document_id, snapshot, &doc_key, None);
        if let Some(doc) = state.documents.get_mut(&document_id) {
            doc.rekey.record_sent(crate::rekey::SentFrame {
                clock: 1,
                key_id: doc_key.id(),
                signature: echo.signature,
                payload: b"edit".to_vec(),
            });
        }
        state.handle_command(SyncCommand::ReleaseDocument {
            document_id,
            applied_seq: 0,
        });
        assert_eq!(
            state.documents_len(),
            1,
            "an unacknowledged frame was dropped"
        );
        assert!(!released(drain(&rx)), "the store heard of a change unsent");
        state.signing_key = Some(MasterSeed::from_array([2; 32]).derive_signing_key());
        state.receive_update(Seq(1), echo);
        assert_eq!(state.documents_len(), 0);
        assert_eq!(state.light_doc_type(document_id), Some(DocType::Trame));
        assert!(released(drain(&rx)));
    }

    /// Holds `RESID-010` on the Sync side: a reopen resumes after the
    /// horizon when the relay's snapshot folds in nothing past it.
    #[test]
    fn a_reopen_resumes_after_the_horizon_it_holds() {
        let (mut state, _rx, document_id, _, _) = released(5);
        assert!(resumes(Seq(5), 5));
        assert!(resumes(Seq(5), 0));
        assert!(!resumes(Seq(5), 6), "a newer relay snapshot was skipped");
        assert_eq!(state.take_light(document_id), Some(Seq(5)));
        assert_eq!(state.take_light(document_id), None);
    }

    /// The horizon never passes what the session itself received.
    #[test]
    fn the_horizon_never_passes_what_the_session_received() {
        let (mut state, _rx, document_id, _, _) = released(40);
        assert_eq!(state.take_light(document_id), Some(Seq(9)));
    }

    /// Holds `SYNC-162`: a segment of a closed channel is watched under the
    /// key of its `meta`, and asked from its first number. Its updates count
    /// as any light document's.
    #[test]
    fn a_watched_segment_counts_from_its_first_number() {
        let (mut state, rx, meta, doc_key, _) = released(9);
        drain(&rx);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let segment = SegmentId::new();
        let meta = MetaId::from_document(meta);
        state.handle_command(SyncCommand::WatchSegment { segment, meta });
        let document_id = segment.document_id();
        let sent: Vec<WsClientMsg> = ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect();
        assert!(
            matches!(
                sent.as_slice(),
                [
                    WsClientMsg::Subscribe { document_id: a },
                    WsClientMsg::RequestMissing {
                        document_id: b,
                        after_seq: Seq(0),
                    },
                ] if *a == document_id && *b == document_id
            ),
            "expected a subscribe and a request from 0, got {sent:?}"
        );
        assert_eq!(state.light_cursor(document_id), Some(Seq(0)));
        state.receive_update(
            Seq(1),
            update(document_id, SnapshotId::new(), &doc_key, Some(&mention())),
        );
        let events = drain(&rx);
        let [LogicEvent::Activity { counters, .. }] = events.as_slice() else {
            panic!("expected the segment's counters, got {events:?}");
        };
        assert_eq!(counters.mentions, 1);

        state.handle_command(SyncCommand::WatchSegment { segment, meta });
        assert!(ws_rx.try_recv().is_err(), "a held segment is watched again");
        assert_eq!(state.light_cursor(document_id), Some(Seq(1)));
    }

    /// A watched thread is asked from its read cursor, under the key of its
    /// `meta`, and counts from its summaries alone (the Murmure threads
    /// spec). Nothing past the cursor is counted twice, and no document loads.
    #[test]
    fn a_watched_thread_counts_after_its_read_cursor() {
        let (mut state, rx, meta, doc_key, _) = released(9);
        drain(&rx);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let thread = DocumentId::new();
        let meta = MetaId::from_document(meta);
        state.handle_command(SyncCommand::WatchThread {
            thread,
            meta,
            after: Seq(4),
        });
        let sent: Vec<WsClientMsg> = ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect();
        assert!(
            matches!(
                sent.as_slice(),
                [
                    WsClientMsg::Subscribe { document_id: a },
                    WsClientMsg::RequestMissing {
                        document_id: b,
                        after_seq: Seq(4),
                    },
                ] if *a == thread && *b == thread
            ),
            "expected a subscribe and a request after the cursor, got {sent:?}"
        );
        let reply = || update(thread, SnapshotId::new(), &doc_key, Some(&mention()));
        state.receive_update(Seq(4), reply());
        assert!(drain(&rx).is_empty(), "a reply already read counted");
        state.receive_update(Seq(5), reply());
        let events = drain(&rx);
        let [
            LogicEvent::Activity {
                document_id,
                counters,
            },
        ] = events.as_slice()
        else {
            panic!("expected the thread's counters, got {events:?}");
        };
        assert_eq!((*document_id, counters.mentions), (thread, 1));
        assert_eq!(state.documents_len(), 0, "a watched thread loads nothing");
    }

    /// Holds `RESID-022`. A channel never opened is watched under the key its
    /// listing wrapped (the Murmure notifications spec). A segment with a
    /// last number asks for what follows it, and a reconnect resumes there.
    /// A document with none asks for nothing, and a reconnect replays nothing
    /// until a delta gives it a number. Every horizon is the first number, so
    /// an open fetches each document whole.
    #[test]
    fn a_channel_never_opened_is_watched_after_its_last_number() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(sink, String::new(), None);
        state.user_id = Some(UserId::from_bytes([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let sent = || -> Vec<WsClientMsg> {
            ws_rx
                .try_iter()
                .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
                .collect()
        };
        let (channel_id, meta, doc_key) =
            (ChannelId::new(), MetaId::new(), DocumentKey::generate());
        let (segment, older) = (DocumentId::new(), DocumentId::new());
        let watch = SyncCommand::WatchChannel {
            channel_id,
            meta,
            documents: vec![(segment, Some(Seq(4))), (older, None)],
        };
        state.handle_command(watch.clone());
        assert!(
            sent().is_empty(),
            "a channel with no listed key was watched"
        );

        state
            .listed_keys
            .insert(channel_id, (MetaId::new(), doc_key.clone()));
        state.handle_command(watch.clone());
        assert!(sent().is_empty(), "the key of another meta was used");

        state
            .listed_keys
            .insert(channel_id, (meta, doc_key.clone()));
        state.handle_command(watch.clone());
        let (m, s, o) = (meta.document_id(), segment, older);
        assert!(
            matches!(
                sent().as_slice(),
                [
                    WsClientMsg::Subscribe { document_id: a },
                    WsClientMsg::Subscribe { document_id: b },
                    WsClientMsg::RequestMissing {
                        document_id: c,
                        after_seq: Seq(4),
                    },
                    WsClientMsg::Subscribe { document_id: d },
                ] if (*a, *b, *c, *d) == (m, s, s, o)
            ),
            "expected three subscribes and one request after the last number"
        );
        state.handle_command(watch);
        assert!(sent().is_empty(), "a held document was watched again");

        let heads: HashSet<_> = state.light_heads().into_iter().collect();
        assert_eq!(heads, HashSet::from([m, o]));
        assert_eq!(state.light_cursors(), vec![(s, Seq(4))]);
        state.resubscribe_all();
        let requests: Vec<(DocumentId, Seq)> = sent()
            .into_iter()
            .filter_map(|msg| match msg {
                WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                } => Some((document_id, after_seq)),
                _ => None,
            })
            .collect();
        assert_eq!(
            requests,
            vec![(s, Seq(4))],
            "a reconnect resumes after the last number, and a head replays nothing"
        );

        // A delta at or below the last number was news before the answer.
        let mentioned = || update(segment, SnapshotId::new(), &doc_key, Some(&mention()));
        state.receive_update(Seq(4), mentioned());
        assert!(drain(&rx).is_empty(), "a delta the answer covered counted");
        state.receive_update(Seq(7), mentioned());
        let events = drain(&rx);
        let [
            LogicEvent::Activity {
                document_id,
                counters,
            },
        ] = events.as_slice()
        else {
            panic!("expected the segment's counters, got {events:?}");
        };
        assert_eq!((*document_id, counters.mentions), (segment, 1));
        assert_eq!(state.light_cursors(), vec![(segment, Seq(7))]);

        // The first live delta of a head gives it a cursor.
        state.receive_update(Seq(2), update(older, SnapshotId::new(), &doc_key, None));
        assert_eq!(state.light_heads(), vec![m]);
        assert_eq!(
            state.take_light(segment),
            Some(Seq(0)),
            "an open replays from the start"
        );
        assert_eq!(state.documents_len(), 0, "watching loads nothing");
    }

    /// A thread held in full or light hears its deltas on its own topic, so
    /// its signal goes no further. Any other thread is named to the Logic
    /// thread (the Murmure threads spec).
    #[test]
    fn a_thread_signal_reaches_logic_only_for_a_thread_not_held() {
        let (mut state, rx, held, _, _) = released(9);
        drain(&rx);
        let open = DocumentId::new();
        let doc = SyncDocumentState::new(open, DocumentKey::generate(), DocType::Murmure, true);
        state.documents.insert(open, doc);
        for document_id in [held, open] {
            state.handle_ws_message(WsServerMsg::ThreadActivity { document_id });
        }
        assert!(drain(&rx).is_empty(), "a held thread was signalled");

        let thread = DocumentId::new();
        state.handle_ws_message(WsServerMsg::ThreadActivity {
            document_id: thread,
        });
        let events = drain(&rx);
        assert!(
            matches!(events.as_slice(),
                [LogicEvent::ThreadActivity { document_id }] if *document_id == thread),
            "got {events:?}"
        );
    }
}
