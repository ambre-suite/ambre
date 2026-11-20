//! The light subscription of a document the Logic thread evicted
//! (`ambre-spec-residency.md`, `RESID-004`).
//!
//! An evicted document keeps its key, its counters and its cursors, and
//! nothing else. Its updates are opened for their activity summary alone, and
//! never reach the Logic thread: no Loro document is loaded to count them
//! (`RESID-016`).

use super::*;

use ambre_core::activity::ActivityCounters;

/// How many holes in its numbers a light subscription remembers. Past it the
/// oldest is forgotten, and a delta that fills it later counts nothing. Eight
/// holes of two numbers each keep the entry within its budget row
/// (`ambre-spec-budgets.md` section 6, `RESID-004`).
const MAX_LIGHT_HOLES: Count<LightHoles> = Count::new(8);

/// The holes a light subscription remembers below its cursor.
enum LightHoles {}

impl ambre_core::NarrowCount for LightHoles {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("LightHoles");
}

/// A run of numbers a light subscription skipped: a broadcast it never
/// received (`RESID-029`, `RESID-031`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hole {
    first: Seq<Relay>,
    last: Seq<Relay>,
}

/// What a number tells a light subscription about the numbers before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrival {
    /// Read already, or below a hole it forgot: nothing to count.
    Read,
    /// The number after the cursor, or one that fills a hole.
    Counts,
    /// A number past the next one: the relay is asked for what follows
    /// `after`, the cursor before it.
    Gap { after: Seq<Relay> },
}

/// Whose summaries a light subscription counts (`RESID-032`, audit `P23`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum LightWriters {
    /// The writers a walk of the document's log granted. A summary another
    /// key signed counts nothing.
    Walked(membership_log::Writers),
    /// The log of this document, the `meta` of a channel never opened, is
    /// not walked yet. A summary counts as activity alone: no mention and
    /// no call cue.
    Awaiting(Id<Document>),
}

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
    /// The runs of numbers below `seen` it never received, oldest first, at
    /// most [`MAX_LIGHT_HOLES`]. A delta that fills one counts once
    /// (`RESID-031`).
    holes: Vec<Hole>,
    /// The first number sealed under a key this subscription lacked, since
    /// the last key it took. A new key replays from it (`RESID-024`).
    unopened_from: Option<Seq<Relay>>,
    /// The keys a frame was sealed under that the relay was asked for, so
    /// each is asked once.
    asked: HashSet<FixedBytes<KeyId>>,
    /// Whose summaries count (`RESID-032`).
    writers: LightWriters,
    /// The clocks the evicted copy applied. The reopen resumes from that
    /// copy, so a revoke that caps one of them reads the document again
    /// (`SYNC-300`).
    applied: crate::Applied,
}

impl LightSubscription {
    /// A light subscription under `doc_key` that read up to `seen`, whose
    /// summaries count when `writers` admits their signer.
    fn new(
        doc_key: DocumentKey,
        doc_type: DocType,
        horizon: Seq<Relay>,
        seen: Seq<Relay>,
        writers: LightWriters,
    ) -> Self {
        Self {
            doc_key,
            doc_type,
            on_relay: crate::Existence::OnRelay,
            horizon,
            seen,
            counters: ActivityCounters::default(),
            replays: true,
            holes: Vec::new(),
            unopened_from: None,
            asked: HashSet::new(),
            writers,
            applied: crate::Applied::default(),
        }
    }

    /// The light subscription a full session leaves behind. The horizon never
    /// passes what the session itself received.
    /// It keeps the writers the session walked (`RESID-032`).
    fn of(doc: SyncDocumentState, applied: Seq<Relay>) -> Self {
        Self {
            on_relay: doc.existence,
            applied: doc.take_applied(),
            ..Self::new(
                doc.doc_key,
                doc.doc_type,
                applied.min(doc.last_seq),
                doc.last_seq,
                LightWriters::Walked(doc.writers),
            )
        }
    }

    /// Reads the number of a broadcast against the cursor (`RESID-029`). A
    /// number past the next one leaves a hole and moves the cursor to it. One
    /// watched from the head takes its first number with no hole: nothing
    /// before it was ever owed (`RESID-030`). A number inside a hole fills it.
    fn arrive(&mut self, seq: Seq<Relay>) -> Arrival {
        if seq <= self.seen {
            return self.fill(seq);
        }
        let after = self.seen;
        let skipped = self.replays && seq.get() > after.get().saturating_add(1);
        self.seen = seq;
        self.replays = true;
        if !skipped {
            return Arrival::Counts;
        }
        self.holes.push(Hole {
            first: Seq::<Relay>::new(after.get().saturating_add(1)),
            last: Seq::<Relay>::new(seq.get().saturating_sub(1)),
        });
        if Count::of(self.holes.len()) > MAX_LIGHT_HOLES {
            self.holes.remove(0);
        }
        Arrival::Gap { after }
    }

    /// Takes `seq` out of the hole that holds it, splitting the hole in two
    /// when `seq` sits inside it.
    fn fill(&mut self, seq: Seq<Relay>) -> Arrival {
        let Some(at) = self
            .holes
            .iter()
            .position(|hole| hole.first <= seq && seq <= hole.last)
        else {
            return Arrival::Read;
        };
        let hole = self.holes.remove(at);
        let below = Hole {
            first: hole.first,
            last: Seq::<Relay>::new(seq.get().saturating_sub(1)),
        };
        let above = Hole {
            first: Seq::<Relay>::new(seq.get().saturating_add(1)),
            last: hole.last,
        };
        let parts = [below, above]
            .into_iter()
            .filter(|part| part.first <= part.last);
        self.holes.splice(at..at, parts);
        Arrival::Counts
    }

    /// Counts one update from its summary alone (`RESID-014`, `RESID-015`).
    /// An update that does not open, or carries no summary, marks activity
    /// and counts nothing. An update `mine` signed returns `None`: it is
    /// nothing new to this user (`RESID-021`).
    fn count(
        &mut self,
        seq: Seq<Relay>,
        update: &Update,
        me: Option<Id<User>>,
        mine: Option<&FixedBytes<PublicKey>>,
    ) -> Option<(ActivityCounters, Option<ambre_bus::CallCue>)> {
        let signer = &update.public_data.pub_key;
        if mine == Some(signer) {
            return None;
        }
        // A key the walk granted no writing role counts nothing: a viewer
        // and the relay inflate no mention and ring no call (`RESID-032`).
        let walked = match &self.writers {
            LightWriters::Walked(writers) if !writers.admits(signer) => return None,
            LightWriters::Walked(_) => true,
            LightWriters::Awaiting(_) => false,
        };
        if update.public_data.key_id != self.doc_key.id() {
            self.unopened_from.get_or_insert(seq);
        }
        let summary = pipeline::open_update(update, &self.doc_key)
            .ok()
            .and_then(|body| pipeline::split_update_body(body).ok())
            .and_then(|body| body.summary)
            .filter(|_| walked);
        self.counters.advance(summary.as_ref(), me);
        Some((self.counters, summary.and_then(|summary| summary.call)))
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
        if unsent_here
            || doc.creating
            || !doc.pending_updates.is_empty()
            || !doc.rekey.settled()
            || !doc.head_watch.settled()
        {
            crate::log::debug!("[Sync] release of doc={document_id} waits on unsent frames");
            return;
        }
        let Some(doc) = self.documents.remove(&document_id) else {
            return;
        };
        self.session_nonces.remove(&document_id);
        self.light
            .insert(document_id, LightSubscription::of(doc, applied));
        crate::log::info!("Sync: doc={document_id} kept as a light subscription");
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
        let arrival = light.arrive(seq);
        let counted = match arrival {
            Arrival::Read => None,
            Arrival::Counts | Arrival::Gap { .. } => light.count(seq, update, me, mine.as_ref()),
        };
        let key_id = update.public_data.key_id;
        let ask = counted.is_some() && key_id != light.doc_key.id() && light.asked.insert(key_id);
        if let Some((counters, cue)) = counted {
            self.send_to_logic(LogicEvent::Activity {
                document_id,
                counters,
            });
            // A call message of another member rings with no document loaded (the Murmure
            // calls spec, the ring). The summary is sealed, so the relay reads no cue.
            if let Some(cue) = cue {
                self.send_to_logic(LogicEvent::CallCue { document_id, cue });
            }
        }
        if ask {
            self.fetch_rotated_key(document_id);
        }
        // A broadcast lost on the way leaves a hole the relay still holds:
        // ask once for what follows the cursor before it (`RESID-029`).
        if let Arrival::Gap { after } = arrival {
            self.ws_send(&WsClientMsg::RequestMissing {
                document_id,
                after_seq: after,
            });
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
        // A segment and a thread take the writers of their channel, which
        // the `meta` holds (`SYNC-229`, `RESID-032`).
        let Some((doc_key, writers)) = self
            .light
            .get(&meta)
            .map(|light| (light.doc_key.clone(), light.writers.clone()))
            .or_else(|| {
                self.documents.get(&meta).map(|doc| {
                    let writers = LightWriters::Walked(doc.writers.clone());
                    (doc.doc_key.clone(), writers)
                })
            })
        else {
            crate::log::warning!("[Sync] no key to watch document {document_id}");
            return;
        };
        self.light.insert(
            document_id,
            LightSubscription::new(doc_key, DocType::Murmure, after, after, writers),
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
            crate::log::warning!("[Sync] no listed key to watch channel {channel_id}");
            return;
        };
        let meta_entry = (meta.document_id(), meta_last);
        for &(document_id, last) in std::iter::once(&meta_entry).chain(documents) {
            self.watch_unheld(document_id, &doc_key, last, meta.document_id());
        }
        // The log of the `meta` walks first: until then a summary counts as
        // activity alone (`RESID-032`, audit `P23`).
        self.handle_list_document_members(None, meta.document_id());
    }

    /// The listing of `document_id`, a document this session does not hold
    /// open, walked its log to `writers`. Each light subscription that
    /// awaited that log now counts the summaries of those writers alone
    /// (`RESID-032`).
    pub(crate) fn light_walked(
        &mut self,
        document_id: Id<Document>,
        writers: &membership_log::Writers,
    ) {
        for light in self.light.values_mut() {
            if light.writers == LightWriters::Awaiting(document_id) {
                light.writers = LightWriters::Walked(writers.clone());
            }
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
        meta: Id<Document>,
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
                    LightWriters::Awaiting(meta),
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
        let light = self.light.remove(&document_id)?;
        // The open that resumes from the copy checks its clocks against the
        // caps of the log it walks (`SYNC-300`).
        if !light.applied.is_empty() {
            self.resumed_clocks.insert(document_id, light.applied);
        }
        Some(light.horizon)
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
            log_head: ambre_protocol::LogHead::EMPTY,
            log_entry: None,
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
            call: None,
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
        assert_eq!(state.documents_len().get(), 0);
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
        assert_eq!(
            state.documents_len().get(),
            0,
            "counting loaded the document"
        );
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, Some(&mention())),
        );
        assert!(drain(&rx).is_empty(), "a number read twice counted twice");
    }

    /// A call cue in the summary of a light document reaches the Logic thread after its
    /// counters. No document loads (the Murmure calls spec, the ring).
    #[test]
    fn a_call_cue_of_a_light_document_reaches_logic_with_no_document_loaded() {
        let (mut state, rx, document_id, doc_key, snapshot) =
            released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
        drain(&rx);
        let cue = ambre_bus::CallCue {
            call: Id::<ambre_core::id::Call>::from_bytes([3; 16]),
            kind: ambre_bus::CueKind::Ring,
            at: ambre_bus::Instant::<ambre_bus::Authored>::new(42),
        };
        let ring = ActivitySummary {
            call: Some(cue),
            ..mention()
        };
        state.receive_update(
            Seq::<Relay>::new(10),
            update(document_id, snapshot, &doc_key, Some(&ring)),
        );
        let events = drain(&rx);
        let [
            LogicEvent::Activity { .. },
            LogicEvent::CallCue {
                document_id: cued,
                cue: heard,
            },
        ] = events.as_slice()
        else {
            panic!("expected the counters then the cue, got {events:?}");
        };
        assert_eq!((*cued, *heard), (document_id, cue));
        assert_eq!(
            state.documents_len().get(),
            0,
            "the cue loaded the document"
        );
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
            state.documents_len().get(),
            1,
            "an unacknowledged frame was dropped"
        );
        assert!(!released(drain(&rx)), "the store heard of a change unsent");
        state.signing_key = Some(MasterSeed::from_array([2; 32]).derive_signing_key());
        state.receive_update(Seq::<Relay>::new(1), echo);
        assert_eq!(state.documents_len().get(), 0);
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
            assert_eq!(
                state.documents_len().get(),
                1,
                "a frame not yet sent was dropped"
            );
        }
        let (mut state, _rx, document_id) = open_one(crate::Existence::OnRelay);
        state.pending_pre_session_updates.push((
            Id::<Document>::mint(),
            ambre_core::Payload::new(b"edit".to_vec()),
            None,
        ));
        release(&mut state, document_id);
        assert_eq!(
            state.documents_len().get(),
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

    /// Holds `SYNC-304` at a reopen. The evicted session applied Bob's
    /// clock 2, and a revoke capped him at 1 while the document was light.
    /// The reopen resumes from the local copy, so the open that walks the
    /// cap reads the document again. A walk with no cap reads nothing.
    #[test]
    fn a_reopen_past_a_revoke_cap_reads_the_document_again() {
        use crate::membership_log::Writers;
        let bob = FixedBytes::<PublicKey>::from_array([0x52; 32]);
        for (cap, reread) in [(Some(1), true), (Some(2), false), (None, false)] {
            let (mut state, rx, document_id, doc_key, snapshot_id) =
                released(ambre_bus::Seq::<ambre_bus::Relay>::new(9));
            // The clocks the evicted session applied travel with its light
            // subscription.
            if let Some(light) = state.light.get_mut(&document_id) {
                light.applied.clocks.insert(bob, Seq::<DeltaAuthor>::new(2));
            }
            assert_eq!(state.take_light(document_id), Some(Seq::<Relay>::new(9)));
            drain(&rx);
            let writers = Writers::capped(
                HashSet::from([bob]),
                cap.map(|cap| (bob, Seq::<DeltaAuthor>::new(cap)))
                    .into_iter()
                    .collect(),
            );
            state.handle_sync_event(SyncEvent::OpenDocumentReady {
                document_id,
                doc_key,
                snapshot_payload: None,
                snapshot_id,
                covered_seq: Seq::<Relay>::new(9),
                held_snapshot: None,
                doc_type: DocType::Trame,
                existence: crate::Existence::OnRelay,
                signing_keys: Vec::new(),
                role: ambre_bus::Role::Owner,
                rotation_owed: ambre_core::RotationDebt::Settled,
                links: Vec::new(),
                writers,
                departed: None,
            });
            let read_again = drain(&rx).iter().any(
                |e| matches!(e, LogicEvent::DocumentReread { document_id: d } if *d == document_id),
            );
            assert_eq!(read_again, reread, "cap {cap:?}");
            assert!(!state.resumed_clocks.contains_key(&document_id));
        }
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
        assert_eq!(
            state.documents_len().get(),
            0,
            "a watched thread loads nothing"
        );
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
        state.light_walked(m, &membership_log::Writers::Unchecked);

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
        assert_eq!(state.documents_len().get(), 0, "watching loads nothing");
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
        state.light_walked(m, &membership_log::Writers::Unchecked);
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
            departed: None,
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
        state.light_walked(m, &membership_log::Writers::Unchecked);
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
            departed: None,
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

    /// The replay asks a mock socket received, as `(document, after)`.
    fn asks(
        ws_rx: &ambre_bus::conduit::Receiver<Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<(Id<Document>, Seq<Relay>)> {
        ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .filter_map(|msg| match msg {
                WsClientMsg::RequestMissing {
                    document_id,
                    after_seq,
                } => Some((document_id, after_seq)),
                _ => None,
            })
            .collect()
    }

    /// The mention counter of each `LogicEvent::Activity` drained.
    fn mentions(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> Vec<u32> {
        drain(rx)
            .into_iter()
            .filter_map(|event| match event {
                LogicEvent::Activity { counters, .. } => Some(counters.mentions.get()),
                _ => None,
            })
            .collect()
    }

    /// Holds `RESID-029` and `RESID-031`. A broadcast past the next number asks the relay
    /// once for what follows the cursor before it. The replay counts each
    /// missing delta once, and a number already read counts nothing.
    #[test]
    fn a_lost_broadcast_is_asked_for_and_counted_once() {
        let (mut state, rx, document_id, doc_key, snapshot) = released(Seq::<Relay>::new(9));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        drain(&rx);
        let deliver = |state: &mut SyncState, seq: u64| {
            state.receive_update(
                Seq::<Relay>::new(seq),
                update(document_id, snapshot, &doc_key, Some(&mention())),
            );
        };
        deliver(&mut state, 10);
        assert!(asks(&ws_rx).is_empty(), "the next number asked for a gap");
        deliver(&mut state, 13);
        assert_eq!(asks(&ws_rx), vec![(document_id, Seq::<Relay>::new(10))]);
        deliver(&mut state, 14);
        deliver(&mut state, 13);
        assert!(asks(&ws_rx).is_empty(), "one gap asked twice");
        assert_eq!(mentions(&rx), vec![1, 2, 3], "a number read twice counted");

        // The replay the ask brings back fills the hole and nothing else.
        for seq in [11, 12, 13, 14, 12] {
            deliver(&mut state, seq);
        }
        assert_eq!(
            mentions(&rx),
            vec![4, 5],
            "the hole counted other than once"
        );
        assert!(asks(&ws_rx).is_empty(), "the replay asked again");

        // A delta inside a hole splits it, and each part still counts once.
        deliver(&mut state, 20);
        state.pass_replay_window();
        for seq in [17, 16, 17, 15, 18, 19, 20] {
            deliver(&mut state, seq);
        }
        assert_eq!(mentions(&rx), vec![6, 7, 8, 9, 10, 11]);

        // A relay that skips numbers at every delta leaves a bounded entry.
        for seq in (0..40).map(|n| 30 + 2 * n) {
            deliver(&mut state, seq);
        }
        let held = state.light.get(&document_id).map(|light| light.holes.len());
        assert_eq!(held.map(Count::<LightHoles>::of), Some(MAX_LIGHT_HOLES));
    }

    /// Holds `RESID-030`: a document watched from the head takes its first
    /// number with no ask, and asks for a gap after it.
    #[test]
    fn a_document_watched_from_the_head_asks_for_nothing_before_its_first_delta() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel_id, meta, doc_key) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            DocumentKey::generate(),
        );
        let segment = Id::<Document>::mint();
        state
            .listed_keys
            .insert(channel_id, (meta, doc_key.clone()));
        state.handle_command(SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: Some(Seq::<Relay>::new(1)),
            documents: vec![(segment, None)],
        });
        state.light_walked(meta.document_id(), &membership_log::Writers::Unchecked);
        let watched = asks(&ws_rx);
        assert!(
            watched
                .iter()
                .all(|(document_id, _)| *document_id != segment),
            "a document watched from the head asked for its history"
        );
        let snapshot = Id::<Snapshot>::mint();
        let deliver = |state: &mut SyncState, seq: u64| {
            state.receive_update(
                Seq::<Relay>::new(seq),
                update(segment, snapshot, &doc_key, Some(&mention())),
            );
        };
        deliver(&mut state, 500);
        assert!(asks(&ws_rx).is_empty(), "the first number asked for a gap");
        assert_eq!(mentions(&rx), vec![1]);
        deliver(&mut state, 503);
        assert_eq!(asks(&ws_rx), vec![(segment, Seq::<Relay>::new(500))]);
    }

    /// `RESID-032`, audit `P23`: a channel never opened counts a summary as
    /// activity alone until the log of its `meta` walks. Once it walks, the
    /// summary of a key it granted no writing role counts nothing, and the
    /// summary of a writer counts its mention. Before the fix, any update
    /// that opened counted its mention and rang its call.
    #[test]
    fn a_light_subscription_counts_the_summaries_of_writers_alone() {
        let (sink, rx) = LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        state.user_id = Some(Id::<User>::from_bytes([7; 16]));
        let (ws_tx, _ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let (channel_id, meta, doc_key) = (
            Id::<Channel>::mint(),
            Id::<Meta>::mint(),
            DocumentKey::generate(),
        );
        state
            .listed_keys
            .insert(channel_id, (meta, doc_key.clone()));
        state.handle_command(SyncCommand::WatchChannel {
            channel_id,
            meta,
            meta_last: Some(Seq::<Relay>::new(1)),
            documents: Vec::new(),
        });
        let m = meta.document_id();
        let counted = |rx: &ambre_bus::conduit::Receiver<LogicEvent>| -> Vec<(u32, u32)> {
            drain(rx)
                .into_iter()
                .filter_map(|event| match event {
                    LogicEvent::Activity { counters, .. } => {
                        Some((counters.content.get(), counters.mentions.get()))
                    }
                    _ => None,
                })
                .collect()
        };
        let mentioned = || update(m, Id::<Snapshot>::mint(), &doc_key, Some(&mention()));
        state.receive_update(Seq::<Relay>::new(2), mentioned());
        assert_eq!(
            counted(&rx),
            vec![(0, 0)],
            "an unwalked log counts activity alone"
        );

        // The signer of `update` is seed 2. The walk grants another key.
        let other = MasterSeed::from_array([3; 32]).derive_signing_key();
        let granted = membership_log::Writers::granted(HashSet::from([FixedBytes::from_array(
            other.verifying_key().to_bytes(),
        )]));
        state.light_walked(m, &granted);
        state.receive_update(Seq::<Relay>::new(3), mentioned());
        assert!(counted(&rx).is_empty(), "a non-writer's summary counted");

        let writer = MasterSeed::from_array([2; 32]).derive_signing_key();
        let granted = membership_log::Writers::granted(HashSet::from([FixedBytes::from_array(
            writer.verifying_key().to_bytes(),
        )]));
        state.light.get_mut(&m).expect("watched").writers = LightWriters::Walked(granted);
        state.receive_update(Seq::<Relay>::new(4), mentioned());
        assert_eq!(counted(&rx), vec![(1, 1)], "a writer's summary counts");
    }
}
