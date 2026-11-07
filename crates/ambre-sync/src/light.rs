//! The light subscription of a document the Logic thread evicted
//! (`ambre-spec-residency.md`, `RESID-004`).
//!
//! An evicted document keeps its key, its counters and its cursors, and
//! nothing else. Its updates are opened for their activity summary alone, and
//! never reach the Logic thread: no Loro document is loaded to count them
//! (`RESID-016`).

use super::*;

use ambre_core::activity::ActivityCounters;

/// What the Sync worker keeps of a document the Logic thread evicted.
pub(crate) struct LightSubscription {
    /// Opens the summaries. A frame sealed under another key is counted as
    /// activity alone, and the reopen reads it whole.
    doc_key: DocumentKey,
    /// The kind of the document, for the reopen.
    doc_type: DocType,
    /// Whether the relay holds the document, so the socket subscribes it.
    on_relay: crate::Existence,
    /// The highest number the evicted copy holds with no gap below it. A
    /// reopen replays only what follows it (`RESID-010`).
    horizon: Seq<Relay>,
    /// The highest number read, the cursor a reconnect resumes after.
    seen: Seq<Relay>,
    /// The activity read from the summaries since the eviction.
    counters: ActivityCounters,
    /// Whether a reconnect asks for what it missed. One watched from the
    /// head asks for nothing until its first delta gives it a number.
    replays: bool,
    /// The first number sealed under a key this subscription lacked, since
    /// the last key it took. A new key replays from it (`RESID-024`).
    unopened_from: Option<Seq<Relay>>,
    /// The keys a frame was sealed under that the relay was asked for, so
    /// each is asked once.
    asked: HashSet<FixedBytes<KeyId>>,
}

impl LightSubscription {
    /// A light subscription under `doc_key` that read up to `seen`.
    fn new(doc_key: DocumentKey, doc_type: DocType, horizon: Seq<Relay>, seen: Seq<Relay>) -> Self {
        Self {
            doc_key,
            doc_type,
            on_relay: crate::Existence::OnRelay,
            horizon,
            seen,
            counters: ActivityCounters::default(),
            replays: true,
            unopened_from: None,
            asked: HashSet::new(),
        }
    }

    /// The light subscription a full session leaves behind. The horizon never
    /// passes what the session itself received.
    fn of(doc: SyncDocumentState, applied: Seq<Relay>) -> Self {
        Self {
            on_relay: doc.existence,
            ..Self::new(
                doc.doc_key,
                doc.doc_type,
                applied.min(doc.last_seq),
                doc.last_seq,
            )
        }
    }

    /// Counts one update from its summary alone (`RESID-014`, `RESID-015`).
    /// An update that does not open, or carries no summary, marks activity
    /// and counts nothing. A number already read returns `None`, and so does
    /// an update `mine` signed, which is nothing new to this user (`RESID-021`).
    fn count(
        &mut self,
        seq: Seq<Relay>,
        update: &Update,
        me: Option<Id<User>>,
        mine: Option<&FixedBytes<PublicKey>>,
    ) -> Option<ActivityCounters> {
        if seq.get() <= self.seen.get() {
            return None;
        }
        self.seen = seq;
        self.replays = true;
        if mine == Some(&update.public_data.pub_key) {
            return None;
        }
        if update.public_data.key_id != self.doc_key.id() {
            self.unopened_from.get_or_insert(seq);
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
pub(crate) fn resumes(horizon: Seq<Relay>, relay_covered: Seq<Relay>) -> bool {
    relay_covered <= horizon
}

impl SyncState {
    /// The Logic thread evicted a document (`RESID-006`). The full session
    /// becomes a light subscription once the relay acknowledged every frame
    /// this client sent for it (`RESID-009`).
    pub(crate) fn handle_release_document(
        &mut self,
        document_id: Id<Document>,
        applied: Seq<Relay>,
    ) {
        let Some(doc) = self.documents.get_mut(&document_id) else {
            return;
        };
        doc.released = Some(applied);
        self.settle_release(document_id);
    }

    /// Turns a released session into a light subscription, unless a frame
    /// is still unacknowledged. The echo of that frame calls it again.
    pub(crate) fn settle_release(&mut self, document_id: Id<Document>) {
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
    /// whether the document is one. A frame sealed under a key it lacks asks
    /// the relay for the document's current key, once per key. A rotation
    /// notice may have been lost while the socket was down (`RESID-024`).
    pub(crate) fn count_light(&mut self, seq: Seq<Relay>, update: &Update) -> bool {
        let document_id = update.public_data.document_id;
        let me = self.user_id;
        let mine = self.author_pub_key();
        let Some(light) = self.light.get_mut(&document_id) else {
            return false;
        };
        let counted = light.count(seq, update, me, mine.as_ref());
        let key_id = update.public_data.key_id;
        let ask = counted.is_some() && key_id != light.doc_key.id() && light.asked.insert(key_id);
        if let Some(counters) = counted {
            self.send_to_logic(LogicEvent::Activity {
                document_id,
                counters,
            });
        }
        if ask {
            self.fetch_rotated_key(document_id);
        }
        true
    }

    /// A rotation of a document held as a light subscription gave it a new
    /// key (`RESID-023`). Its next summaries open under it, and so do those
    /// of a document watched later under its key. The listed key of a
    /// channel whose `meta` it is follows.
    ///
    /// The frames read under a key it lacked counted as activity alone. They
    /// are asked again, so their summaries count under the new key
    /// (`RESID-024`). A key it already held replays nothing.
    pub(crate) fn rekey_light(&mut self, document_id: Id<Document>, new_key: DocumentKey) {
        let Some(light) = self.light.get_mut(&document_id) else {
            return;
        };
        let unopened = light.unopened_from.take();
        if light.doc_key.id() == new_key.id() {
            return;
        }
        light.doc_key = new_key.clone();
        // A frame is counted once its number passes `seen`, so the cursor
        // steps back below the first frame that did not open.
        let replay = unopened.map(|from| Seq::<Relay>::new(from.get().saturating_sub(1)));
        if let Some(after) = replay {
            light.seen = after;
        }
        for (meta, key) in self.listed_keys.values_mut() {
            if meta.document_id() == document_id {
                *key = new_key.clone();
            }
        }
        if let Some(after_seq) = replay {
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq,
            });
        }
    }

    /// Counts a segment of a closed channel from its first number, as a
    /// light subscription under the key of `meta` (`SYNC-162`). A segment
    /// already held, in full or light, is left as it is. A `meta` this
    /// worker holds no key for watches nothing.
    pub(crate) fn handle_watch_segment(&mut self, segment: Id<Segment>, meta: Id<Meta>) {
        self.watch_light(segment.document_id(), meta, Seq::<Relay>::new(0));
    }

    /// Counts a thread whose root is not on screen from the number after
    /// `after`, its read cursor (the Murmure threads spec). Its summaries are
    /// read, and no document is loaded.
    pub(crate) fn handle_watch_thread(
        &mut self,
        thread: Id<Document>,
        meta: Id<Meta>,
        after: Seq<Relay>,
    ) {
        self.watch_light(thread, meta, after);
    }

    /// Holds `document_id` as a light subscription under the key of `meta`,
    /// and asks the relay for what follows `after`. A document already held,
    /// in full or light, is left as it is.
    fn watch_light(&mut self, document_id: Id<Document>, meta: Id<Meta>, after: Seq<Relay>) {
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
            LightSubscription::new(doc_key, DocType::Murmure, after, after),
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
        channel_id: Id<Channel>,
        (meta, meta_last): (Id<Meta>, Option<Seq<Relay>>),
        documents: &[(Id<Document>, Option<Seq<Relay>>)],
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
        let meta_entry = (meta.document_id(), meta_last);
        for &(document_id, last) in std::iter::once(&meta_entry).chain(documents) {
            self.watch_unheld(document_id, &doc_key, last);
        }
    }

    /// Holds one document of a channel never opened as a light subscription.
    /// No copy of it exists, so its horizon is the first number: an open
    /// fetches it whole (`RESID-010`). The relay is asked for what follows
    /// `last`, and for nothing when there is none: the head.
    fn watch_unheld(
        &mut self,
        document_id: Id<Document>,
        key: &DocumentKey,
        last: Option<Seq<Relay>>,
    ) {
        if self.documents.contains_key(&document_id) || self.light.contains_key(&document_id) {
            return;
        }
        self.light.insert(
            document_id,
            LightSubscription {
                replays: last.is_some(),
                ..LightSubscription::new(
                    key.clone(),
                    DocType::Murmure,
                    Seq::<Relay>::new(0),
                    last.unwrap_or(Seq::<Relay>::new(0)),
                )
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
    pub(crate) fn thread_activity(&mut self, document_id: Id<Document>) {
        if self.documents.contains_key(&document_id) || self.light.contains_key(&document_id) {
            return;
        }
        self.send_to_logic(LogicEvent::ThreadActivity { document_id });
    }

    /// Takes the light subscription of a document being reopened: its kind
    /// and the horizon the reopen resumes after.
    pub(crate) fn take_light(&mut self, document_id: Id<Document>) -> Option<Seq<Relay>> {
        self.light.remove(&document_id).map(|light| light.horizon)
    }

    /// The light subscriptions the socket subscribes and replays, with the
    /// cursor each resumes after.
    pub(crate) fn light_cursors(&self) -> Vec<(Id<Document>, Seq<Relay>)> {
        self.light
            .iter()
            .filter(|(_, light)| light.on_relay == crate::Existence::OnRelay && light.replays)
            .map(|(id, light)| (*id, light.seen))
            .collect()
    }

    /// The light subscriptions watched from the head that took no delta
    /// yet: the socket subscribes them and asks for nothing.
    pub(crate) fn light_heads(&self) -> Vec<Id<Document>> {
        self.light
            .iter()
            .filter(|(_, light)| light.on_relay == crate::Existence::OnRelay && !light.replays)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The cursor one light subscription resumes after, if the relay holds it.
    pub(crate) fn light_cursor(&self, document_id: Id<Document>) -> Option<Seq<Relay>> {
        self.light
            .get(&document_id)
            .filter(|light| light.on_relay == crate::Existence::OnRelay)
            .map(|light| light.seen)
    }

    /// The kind of a document held as a light subscription.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only a test reads the kind of a light document")
    )]
    pub(crate) fn light_doc_type(&self, document_id: Id<Document>) -> Option<DocType> {
        self.light.get(&document_id).map(|light| light.doc_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::activity::{ActivityKind, ActivitySummary};

    const NONCE: FixedBytes<SessionNonce> = FixedBytes::from_array([6; 16]);

    /// A state with one open document, released at `applied`.
    fn released(
        applied: Seq<Relay>,
    ) -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        Id<Document>,
        DocumentKey,
        Id<Snapshot>,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let mut doc = SyncDocumentState::new(
            document_id,
            doc_key.clone(),
            DocType::Trame,
            crate::Existence::OnRelay,
        );
        doc.last_seq = Seq::<Relay>::new(9);
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
        document_id: Id<Document>,
        snapshot_id: Id<Snapshot>,
        doc_key: &DocumentKey,
        summary: Option<&ActivitySummary>,
    ) -> Update {
        let signing_key = MasterSeed::from_array([2; 32]).derive_signing_key();
        let public_data = UpdatePublicData {
            document_id,
            pub_key: FixedBytes::from_array(signing_key.verifying_key().to_bytes()),
            key_id: doc_key.id(),
            snapshot_id,
            clock: Seq::<DeltaAuthor>::new(1),
        };
        let body = pipeline::update_body(&NONCE, summary, &b"delta".to_vec().into())
            .expect("within bounds");
        pipeline::seal_update(&body, public_data, doc_key, &signing_key).expect("seal")
    }

    fn mention() -> ActivitySummary {
        ActivitySummary {
            kind: ActivityKind::Content,
            mentions: vec![Id::<User>::from_bytes([7; 16])],
            mentions_all: ambre_bus::MentionScope::Named,
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
        let (state, _rx, document_id, _, _) = released(ambre_bus::Seq::<ambre_bus::Relay>::new(5));
        assert_eq!(state.documents_len(), 0);
        assert!(!state.session_nonces.contains_key(&document_id));
        assert_eq!(state.light_doc_type(document_id), Some(DocType::Trame));
        assert!(std::mem::size_of::<LightSubscription>() < 1024);
        assert_eq!(
            state.light_cursors(),
            vec![(document_id, Seq::<Relay>::new(9))]
        );
    }

    /// Holds `RESID-014`, `RESID-005` and `RESID-016`. An update of a light
    /// document moves its counters from the summary alone. Nothing reaches
    /// the Logic thread but the counters.
    #[test]
    fn a_light_update_moves_its_counters_from_the_summary() {
        let (mut state, rx, document_id, doc_key, snapshot) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        let events = drain(&rx);
        let [LogicEvent::Activity { counters, .. }] = events.as_slice() else {
            panic!("expected the counters alone, got {events:?}");
        };
        assert_eq!((counters.content.get(), counters.mentions.get()), (1, 1));
        assert_eq!(state.documents_len(), 0, "counting loaded the document");
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert!(drain(&rx).is_empty(), "a number read twice counted twice");
    }

    /// Holds `RESID-015`: an update with no summary, or one that does not
    /// open, marks the document active and counts nothing.
    #[test]
    fn a_light_update_with_no_summary_marks_activity_alone() {
        let (mut state, rx, document_id, doc_key, snapshot) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, None),
        );
        let stranger = DocumentKey::generate();
        state.receive_update(
            Seq::<Relay>::new(11),
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
            active: ambre_bus::Activity::Active,
            ..ActivityCounters::default()
        };
        assert_eq!(counters, vec![active, active]);
    }

    /// Holds `RESID-021`: an update this user signed, from this device or
    /// another, moves no counter and marks nothing.
    #[test]
    fn an_update_this_user_signed_counts_nothing() {
        let (mut state, rx, document_id, doc_key, snapshot) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        state.signing_key = Some(MasterSeed::from_array([2; 32]).derive_signing_key());
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert!(drain(&rx).is_empty(), "the user's own update counted");
        state.signing_key = Some(MasterSeed::from_array([3; 32]).derive_signing_key());
        state.receive_update(
            Seq::<Relay>::new(11),
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
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let doc_key = DocumentKey::generate();
        let doc = SyncDocumentState::new(
            document_id,
            doc_key.clone(),
            DocType::Trame,
            crate::Existence::OnRelay,
        );
        let snapshot = doc.current_snapshot_id;
        state.documents.insert(document_id, doc);
        let echo = update(document_id, snapshot, &doc_key, None);
        if let Some(doc) = state.documents.get_mut(&document_id) {
            doc.rekey.record_sent(crate::rekey::SentFrame {
                clock: ambre_protocol::messages::Seq::<ambre_protocol::messages::DeltaAuthor>::new(
                    1,
                ),
                key_id: doc_key.id(),
                signature: echo.signature,
                payload: crate::pipeline::UpdateFrame::from_bytes(b"edit".to_vec()),
            });
        }
        state.handle_command(SyncCommand::ReleaseDocument {
            document_id,
            applied_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(0),
        });
        assert_eq!(
            state.documents_len(),
            1,
            "an unacknowledged frame was dropped"
        );
        assert!(!released(drain(&rx)), "the store heard of a change unsent");
        state.signing_key = Some(MasterSeed::from_array([2; 32]).derive_signing_key());
        state.receive_update(Seq::<Relay>::new(1), echo);
        assert_eq!(state.documents_len(), 0);
        assert_eq!(state.light_doc_type(document_id), Some(DocType::Trame));
        assert!(released(drain(&rx)));
    }

    /// A state holding one open document the relay holds as `existence`.
    fn open_one(
        existence: crate::Existence,
    ) -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        Id<Document>,
    ) {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let document_id = Id::<Document>::mint();
        let doc = SyncDocumentState::new(
            document_id,
            DocumentKey::generate(),
            DocType::Trame,
            existence,
        );
        state.documents.insert(document_id, doc);
        (state, rx, document_id)
    }

    fn release(state: &mut SyncState, document_id: Id<Document>) {
        state.handle_command(SyncCommand::ReleaseDocument {
            document_id,
            applied_seq: ambre_bus::Seq::<ambre_bus::Relay>::new(0),
        });
    }

    /// Holds `RESID-009`: a release waits on each frame not yet sent: an
    /// update queued before the session, a creation in flight, or a frame
    /// pending. An update queued for another document holds nothing.
    #[test]
    fn a_release_waits_on_every_frame_not_yet_sent() {
        let blockers: [fn(&mut SyncState, Id<Document>); 3] = [
            |state, document_id| {
                state.pending_pre_session_updates.push((
                    document_id,
                    ambre_core::Payload::new(b"edit".to_vec()),
                    None,
                ));
            },
            |state, document_id| {
                if let Some(doc) = state.documents.get_mut(&document_id) {
                    doc.creating = true;
                }
            },
            |state, document_id| {
                if let Some(doc) = state.documents.get_mut(&document_id) {
                    doc.pending_updates
                        .push(crate::pipeline::UpdateFrame::from_bytes(b"edit".to_vec()));
                }
            },
        ];
        for block in blockers {
            let (mut state, _rx, document_id) = open_one(crate::Existence::OnRelay);
            block(&mut state, document_id);
            release(&mut state, document_id);
            assert_eq!(state.documents_len(), 1, "a frame not yet sent was dropped");
        }
        let (mut state, _rx, document_id) = open_one(crate::Existence::OnRelay);
        state.pending_pre_session_updates.push((
            Id::<Document>::mint(),
            ambre_core::Payload::new(b"edit".to_vec()),
            None,
        ));
        release(&mut state, document_id);
        assert_eq!(
            state.documents_len(),
            0,
            "another document held the release"
        );
    }

    /// `RESID-028`: a document released before the relay holds it keeps a
    /// light subscription the socket never subscribes.
    #[test]
    fn a_local_document_released_is_never_subscribed() {
        let (mut state, _rx, document_id) = open_one(crate::Existence::LocalOnly);
        release(&mut state, document_id);
        assert_eq!(state.light_doc_type(document_id), Some(DocType::Trame));
        assert_eq!(state.light_cursor(document_id), None);
        assert!(state.light_cursors().is_empty());
        assert!(state.light_heads().is_empty());
    }

    /// Holds `RESID-010` on the Sync side: a reopen resumes after the
    /// horizon when the relay's snapshot folds in nothing past it.
    #[test]
    fn a_reopen_resumes_after_the_horizon_it_holds() {
        let (mut state, _rx, document_id, _, _) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(5));
        assert!(resumes(
            Seq::<Relay>::new(5),
            ambre_bus::Seq::<ambre_bus::Relay>::new(5)
        ));
        assert!(resumes(
            Seq::<Relay>::new(5),
            ambre_bus::Seq::<ambre_bus::Relay>::new(0)
        ));
        assert!(
            !resumes(
                Seq::<Relay>::new(5),
                ambre_bus::Seq::<ambre_bus::Relay>::new(6)
            ),
            "a newer relay snapshot was skipped"
        );
        assert_eq!(state.take_light(document_id), Some(Seq::<Relay>::new(5)));
        assert_eq!(state.take_light(document_id), None);
    }

    /// The horizon never passes what the session itself received.
    #[test]
    fn the_horizon_never_passes_what_the_session_received() {
        let (mut state, _rx, document_id, _, _) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(40));
        assert_eq!(state.take_light(document_id), Some(Seq::<Relay>::new(9)));
    }

    /// Holds `SYNC-162`: a segment of a closed channel is watched under the
    /// key of its `meta`, and asked from its first number. Its updates count
    /// as any light document's.
    #[test]
    fn a_watched_segment_counts_from_its_first_number() {
        let (mut state, rx, meta, doc_key, _) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let segment = Id::<Segment>::mint();
        let meta = Id::<Meta>::from_document(meta);
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
                        after_seq,
                    },
                ] if *a == document_id && *b == document_id && after_seq.get() == 0
            ),
            "expected a subscribe and a request from 0, got {sent:?}"
        );
        assert_eq!(state.light_cursor(document_id), Some(Seq::<Relay>::new(0)));
        state.receive_update(
            Seq::<Relay>::new(1),
            update(
                document_id,
                Id::<Snapshot>::mint(),
                &doc_key,
                Some(&mention()),
            ),
        );
        let events = drain(&rx);
        let [LogicEvent::Activity { counters, .. }] = events.as_slice() else {
            panic!("expected the segment's counters, got {events:?}");
        };
        assert_eq!(counters.mentions.get(), 1);

        state.handle_command(SyncCommand::WatchSegment { segment, meta });
        assert!(ws_rx.try_recv().is_err(), "a held segment is watched again");
        assert_eq!(state.light_cursor(document_id), Some(Seq::<Relay>::new(1)));
    }

    /// A watched thread is asked from its read cursor, under the key of its
    /// `meta`, and counts from its summaries alone (the Murmure threads
    /// spec). Nothing past the cursor is counted twice, and no document loads.
    #[test]
    fn a_watched_thread_counts_after_its_read_cursor() {
        let (mut state, rx, meta, doc_key, _) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let thread = Id::<Document>::mint();
        let meta = Id::<Meta>::from_document(meta);
        state.handle_command(SyncCommand::WatchThread {
            thread,
            meta,
            after: Seq::<Relay>::new(4),
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
                        after_seq,
                    },
                ] if *a == thread && *b == thread && after_seq.get() == 4
            ),
            "expected a subscribe and a request after the cursor, got {sent:?}"
        );
        let reply = || update(thread, Id::<Snapshot>::mint(), &doc_key, Some(&mention()));
        state.receive_update(Seq::<Relay>::new(4), reply());
        assert!(drain(&rx).is_empty(), "a reply already read counted");
        state.receive_update(Seq::<Relay>::new(5), reply());
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
        assert_eq!((*document_id, counters.mentions.get()), (thread, 1));
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
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let sent = || -> Vec<WsClientMsg> {
            ws_rx
                .try_iter()
                .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
                .collect()
        };
        let (channel_id, meta, doc_key) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            DocumentKey::generate(),
        );
        let (segment, older) = (Id::<Document>::mint(), Id::<Document>::mint());
        let watch = SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: None,
            documents: vec![(segment, Some(Seq::<Relay>::new(4))), (older, None)],
        };
        state.handle_command(watch.clone());
        assert!(
            sent().is_empty(),
            "a channel with no listed key was watched"
        );

        state
            .listed_keys
            .insert(channel_id, (Id::<Meta>::mint(), doc_key.clone()));
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
                        after_seq,
                    },
                    WsClientMsg::Subscribe { document_id: d },
                ] if (*a, *b, *c, *d) == (m, s, s, o) && after_seq.get() == 4
            ),
            "expected three subscribes and one request after the last number"
        );
        state.handle_command(watch);
        assert!(sent().is_empty(), "a held document was watched again");

        let heads: HashSet<_> = state.light_heads().into_iter().collect();
        assert_eq!(heads, HashSet::from([m, o]));
        assert_eq!(state.light_cursors(), vec![(s, Seq::<Relay>::new(4))]);
        state.resubscribe_all();
        let requests: Vec<(Id<Document>, Seq<Relay>)> = sent()
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
            vec![(s, Seq::<Relay>::new(4))],
            "a reconnect resumes after the last number, and a head replays nothing"
        );

        // A delta at or below the last number was news before the answer.
        let mentioned = || update(segment, Id::<Snapshot>::mint(), &doc_key, Some(&mention()));
        state.receive_update(Seq::<Relay>::new(4), mentioned());
        assert!(drain(&rx).is_empty(), "a delta the answer covered counted");
        state.receive_update(Seq::<Relay>::new(7), mentioned());
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
        assert_eq!((*document_id, counters.mentions.get()), (segment, 1));
        assert_eq!(state.light_cursors(), vec![(segment, Seq::<Relay>::new(7))]);

        // The first live delta of a head gives it a cursor.
        state.receive_update(
            Seq::<Relay>::new(2),
            update(older, Id::<Snapshot>::mint(), &doc_key, None),
        );
        assert_eq!(state.light_heads(), vec![m]);
        assert_eq!(
            state.take_light(segment),
            Some(Seq::<Relay>::new(0)),
            "an open replays from the start"
        );
        assert_eq!(state.documents_len(), 0, "watching loads nothing");
    }

    /// Holds `RESID-022` for the `meta`. A `meta` the listing gave a last
    /// number asks for what follows it, and a reconnect resumes there.
    #[test]
    fn the_meta_of_a_channel_never_opened_is_watched_after_its_last_number() {
        let mut state = SyncState::new(
            LogicSink::channel().0,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
        state
            .listed_keys
            .insert(channel_id, (meta, DocumentKey::generate()));
        state.handle_command(SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: Some(Seq::<Relay>::new(5)),
            documents: Vec::new(),
        });
        let sent: Vec<WsClientMsg> = ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .collect();
        let m = meta.document_id();
        assert!(
            matches!(
                sent.as_slice(),
                [
                    WsClientMsg::Subscribe { document_id: a },
                    WsClientMsg::RequestMissing {
                        document_id: b,
                        after_seq,
                    },
                ] if (*a, *b) == (m, m) && after_seq.get() == 5
            ),
            "expected a subscribe and a request after the last number, got {sent:?}"
        );
        assert!(state.light_heads().is_empty());
        assert_eq!(state.light_cursors(), vec![(m, Seq::<Relay>::new(5))]);
    }

    /// Holds `RESID-023`. A rotation of a watched `meta` gives it the new
    /// key, and its summaries under that key count. So do those of a
    /// segment watched after under it. The listed key follows.
    #[test]
    fn a_rotation_rekeys_a_channel_watched_unopened() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
        let (old_key, new_key) = (DocumentKey::generate(), DocumentKey::generate());
        state
            .listed_keys
            .insert(channel_id, (meta, old_key.clone()));
        state.handle_command(SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: Some(Seq::<Relay>::new(1)),
            documents: Vec::new(),
        });
        let m = meta.document_id();
        let mentions = |rx: &ambre_bus::conduit::Receiver<LogicEvent>| -> Vec<(Id<Document>, u32)> {
            drain(rx)
                .into_iter()
                .filter_map(|event| match event {
                    LogicEvent::Activity {
                        document_id,
                        counters,
                    } => Some((document_id, counters.mentions.get())),
                    _ => None,
                })
                .collect()
        };
        let mentioned =
            |doc, key: &DocumentKey| update(doc, Id::<Snapshot>::mint(), key, Some(&mention()));
        state.receive_update(Seq::<Relay>::new(2), mentioned(m, &new_key));
        assert_eq!(
            mentions(&rx),
            vec![(m, 0)],
            "the new key opened before the rotation"
        );

        state.handle_sync_event(SyncEvent::KeyRotated {
            document_id: m,
            new_doc_key: new_key.clone(),
            links: Vec::new(),
        });
        let listed = state.listed_keys.get(&channel_id).map(|(_, key)| key.id());
        assert_eq!(listed, Some(new_key.id()), "the listed key follows");
        state.receive_update(Seq::<Relay>::new(3), mentioned(m, &new_key));
        assert_eq!(mentions(&rx), vec![(m, 1)]);

        let segment = Id::<Segment>::mint();
        state.handle_command(SyncCommand::WatchSegment { segment, meta });
        state.receive_update(
            Seq::<Relay>::new(1),
            mentioned(segment.document_id(), &new_key),
        );
        assert_eq!(mentions(&rx), vec![(segment.document_id(), 1)]);
    }

    /// Holds `RESID-024`. A rotation missed offline leaves a watched `meta`
    /// under its old key. A frame under the new key marks activity and asks
    /// for the key once. The key, once taken, replays from that frame, which
    /// counts.
    #[test]
    fn a_frame_under_a_missing_key_fetches_it_and_replays() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel_id, meta) = (Id::<Channel>::mint(), Id::<Meta>::mint());
        let (old_key, new_key) = (DocumentKey::generate(), DocumentKey::generate());
        state.listed_keys.insert(channel_id, (meta, old_key));
        state.handle_command(SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: Some(Seq::<Relay>::new(1)),
            documents: Vec::new(),
        });
        let m = meta.document_id();
        let requests = || -> Vec<Seq<Relay>> {
            ws_rx
                .try_iter()
                .filter_map(|wire| match ambre_protocol::ws::decode_client_msg(&wire) {
                    Ok(WsClientMsg::RequestMissing { after_seq, .. }) => Some(after_seq),
                    _ => None,
                })
                .collect()
        };
        let mentions = || -> Vec<u32> {
            drain(&rx)
                .into_iter()
                .filter_map(|event| match event {
                    LogicEvent::Activity { counters, .. } => Some(counters.mentions.get()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(requests(), vec![Seq::<Relay>::new(1)]);
        let mentioned = || update(m, Id::<Snapshot>::mint(), &new_key, Some(&mention()));
        state.receive_update(Seq::<Relay>::new(2), mentioned());
        state.receive_update(Seq::<Relay>::new(3), mentioned());
        assert_eq!(mentions(), vec![0, 0], "a key it lacks counts nothing");
        let asked: Vec<_> = state.light[&m].asked.iter().copied().collect();
        assert_eq!(asked, vec![new_key.id()], "each key is asked once");

        let rotated = |key: &DocumentKey| SyncEvent::KeyRotated {
            document_id: m,
            new_doc_key: key.clone(),
            links: Vec::new(),
        };
        state.pass_replay_window();
        state.handle_sync_event(rotated(&new_key));
        assert_eq!(
            requests(),
            vec![Seq::<Relay>::new(1)],
            "replayed from the first unopened frame"
        );
        state.receive_update(Seq::<Relay>::new(2), mentioned());
        state.receive_update(Seq::<Relay>::new(3), mentioned());
        assert_eq!(mentions(), vec![1, 2]);
        state.handle_sync_event(rotated(&new_key));
        assert!(requests().is_empty(), "a key already held replays nothing");
    }

    /// A thread held in full or light hears its deltas on its own topic, so
    /// its signal goes no further. Any other thread is named to the Logic
    /// thread (the Murmure threads spec).
    #[test]
    fn a_thread_signal_reaches_logic_only_for_a_thread_not_held() {
        let (mut state, rx, held, _, _) = released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        let open = Id::<Document>::mint();
        let doc = SyncDocumentState::new(
            open,
            DocumentKey::generate(),
            DocType::Murmure,
            crate::Existence::OnRelay,
        );
        state.documents.insert(open, doc);
        for document_id in [held, open] {
            state.handle_ws_message(WsServerMsg::ThreadActivity { document_id });
        }
        assert!(drain(&rx).is_empty(), "a held thread was signalled");

        let thread = Id::<Document>::mint();
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
