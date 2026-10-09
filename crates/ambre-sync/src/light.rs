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
    on_relay: bool,
    /// The highest number the evicted copy holds with no gap below it. A
    /// reopen replays only what follows it (`RESID-010`).
    horizon: Seq,
    /// The highest number read, the cursor a reconnect resumes after.
    seen: Seq,
    /// The activity read from the summaries since the eviction.
    counters: ActivityCounters,
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
        }
    }

    /// Counts one update from its summary alone (`RESID-014`, `RESID-015`).
    /// An update that does not open, or carries no summary, marks activity
    /// and counts nothing. A number already read returns `None`.
    fn count(&mut self, seq: Seq, update: &Update, me: Option<UserId>) -> Option<ActivityCounters> {
        if seq.0 <= self.seen.0 {
            return None;
        }
        self.seen = seq;
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
            .any(|(doc, _)| *doc == document_id);
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
        let Some(light) = self.light.get_mut(&document_id) else {
            return false;
        };
        if let Some(counters) = light.count(seq, update, me) {
            self.send_to_logic(LogicEvent::Activity {
                document_id,
                counters,
            });
        }
        true
    }

    /// Takes the light subscription of a document being reopened: its kind
    /// and the horizon the reopen resumes after.
    pub(crate) fn take_light(&mut self, document_id: DocumentId) -> Option<Seq> {
        self.light.remove(&document_id).map(|light| light.horizon)
    }

    /// The light subscriptions the socket subscribes, with the cursor each
    /// resumes after.
    pub(crate) fn light_cursors(&self) -> Vec<(DocumentId, Seq)> {
        self.light
            .iter()
            .filter(|(_, light)| light.on_relay)
            .map(|(id, light)| (*id, light.seen))
            .collect()
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
        if let Some(doc) = state.documents.get_mut(&document_id) {
            doc.rekey.record_sent(1, doc_key.id(), b"edit".to_vec());
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
        let echo = update(document_id, snapshot, &doc_key, None);
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
}
