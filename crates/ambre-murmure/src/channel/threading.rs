//! The threads of a channel (`features/ambre-spec-murmure-threads.md section 3`).
//!
//! The replies to one message live in one thread document (`MURMURE-105`).
//! A client creates it on the first reply, and the relay links one per root
//! (`SRV-213`, `SRV-214`). The root then names it in its `thread` field
//! (`MURMURE-107`), the one field of it another member may write
//! (`MURMURE-255`).

use crate::message::{Deletion, MessageDraft, MessageView, MurmureEvent, ReplyEcho, ThreadHead};
use ambre_bus::{Authored, Instant};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes};
use loro::LoroDoc;

use super::{
    ChannelState, Created, Held, Sent, amending, message_map, reaction_summary, read_messages,
    segment_entries, summary_of,
};
use crate::delta::{self, Arrival, Rules};
use crate::error::MurmureError;
use crate::schema::{self, DocumentKind, HeaderKey, MessageKey, Root};
use crate::session::Session;
use crate::{mention, style};

/// A thread document the channel holds, with the segment of its root.
#[derive(Debug)]
pub(super) struct Thread {
    /// The segment its root lies in, as its header names it.
    pub(super) segment: Id<Segment>,
    pub(super) held: Held,
}

/// A thread document whose bytes are still arriving.
#[derive(Debug)]
pub(super) struct Pending {
    /// The segment of its root and that root, as a held segment names them.
    /// `None` for a probe: a seek reads its header, and nothing holds it.
    named: Option<(Id<Segment>, Id<Message>)>,
    /// Its payloads, in the order they came.
    pub(super) arrivals: Vec<Arrival>,
    /// The identity issued for it, adopted once it is admitted.
    session: Option<(ambre_core::PeerId, Payload<PeerEntryBytes>)>,
}

impl Pending {
    fn of(segment: Id<Segment>, root: Id<Message>) -> Self {
        Self {
            named: Some((segment, root)),
            arrivals: Vec::new(),
            session: None,
        }
    }

    fn probe() -> Self {
        Self {
            named: None,
            arrivals: Vec::new(),
            session: None,
        }
    }
}

/// A reply on a copy whose root lies in no held segment. The channel loads
/// its older segments, one at a time, down to the one the thread names.
#[derive(Debug, Clone, Copy)]
pub(super) struct Seek {
    /// The thread document the copy names.
    thread: Id<Document>,
    /// The segment its header names, once its probe is read.
    target: Option<Id<Segment>>,
    /// The older segment the seek last asked for.
    asked: Option<Id<Segment>>,
    /// Whether its probe failed, or named no listed segment.
    trail: Trail,
}

ambre_core::define_flag! {
    /// Whether a seek for the root of a copy can still find it.
    enum Trail {
        /// The seek goes on, a segment at a time.
        Open,
        /// Its probe failed, or named no listed segment: the next step says
        /// the root is lost.
        Lost,
    }
}

/// Where a seek for the root of a copy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekStep {
    /// A held segment holds the root, and names the thread.
    Found {
        thread: Id<Document>,
        segment: Id<Segment>,
        root: Id<Message>,
    },
    /// Open this older segment, then step again.
    Load(Id<Segment>),
    /// A probe or a segment is on its way.
    Wait,
    /// The root of `thread` cannot be reached: its probe failed, or no
    /// segment down to the one the header names holds it. The seek ends,
    /// and nothing opens.
    Lost { thread: Id<Document> },
}

/// The document a reply writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadTarget {
    /// The thread document the channel holds or is loading for the root.
    Write(Id<Document>),
    /// The thread document the root names, now asked for: the reply waits
    /// until it is held.
    Open(Id<Document>),
    /// A new thread document, held here with no session. The Sync worker
    /// creates it on the relay and links it to the root (`SRV-213`).
    Create(Id<Document>),
}

/// What a link of a thread document produced.
#[derive(Debug)]
pub struct ThreadLinked {
    /// The delta that names the document in the root's `thread` field,
    /// `None` when the field named it already (`MURMURE-107`).
    pub root: Option<Sent>,
    /// Whether the document is to open: this client's own, for its session,
    /// or another member's, for its bytes.
    pub open: super::LinkedOpening,
    /// This client's own document, which lost the race to another member's
    /// (`SRV-214`). What waited on it waits on the winner.
    pub replaced: Option<Id<Document>>,
}

/// What a reply produced.
#[derive(Debug)]
pub struct Replied {
    /// The reply, written in the thread document.
    pub reply: Sent,
    /// Its copy in the active segment, when it was also sent to the channel
    /// (`MURMURE-109`).
    pub copy: Option<Sent>,
    /// The root's segment, whose thread head moved (`MURMURE-106`).
    pub head: MurmureEvent,
}

impl ChannelState {
    /// The root and the held thread document of that identifier.
    fn thread_of(&self, document_id: Id<Document>) -> Option<(Id<Message>, &Thread)> {
        self.threads
            .iter()
            .find(|(_, thread)| thread.held.id == document_id)
            .map(|(root, thread)| (*root, thread))
    }

    /// The thread documents the channel holds.
    pub fn held_threads(&self) -> Vec<Id<Document>> {
        self.threads.values().map(|thread| thread.held.id).collect()
    }

    /// Whether `document_id` is a thread document, held or arriving.
    pub(super) fn is_thread(&self, document_id: Id<Document>) -> bool {
        self.threads_pending.contains_key(&document_id) || self.thread_of(document_id).is_some()
    }

    /// The thread document the channel holds for `root`.
    pub fn thread_id(&self, root: Id<Message>) -> Option<Id<Document>> {
        self.threads.get(&root).map(|thread| thread.held.id)
    }

    /// The held segment that holds `root`: the one its thread is linked to.
    pub fn root_segment(&self, root: Id<Message>) -> Option<Id<Segment>> {
        self.root_slot(root).map(|(segment, _)| segment)
    }

    /// The held segment that holds `root`, with the thread it names: its
    /// `thread` field, else the one its amendments record (`MURMURE-484`).
    fn root_slot(&self, root: Id<Message>) -> Option<(Id<Segment>, Option<Id<Document>>)> {
        self.held_segments().into_iter().find_map(|segment| {
            let map = message_map(self.doc(segment.document_id())?, root)?;
            let named = schema::string(&map, MessageKey::Thread)
                .and_then(|s| Id::<Document>::parse_str(&s).ok())
                .or_else(|| {
                    let amendments = self.amendments.get(&segment)?;
                    amending::recorded_thread(&amendments.doc, root)
                });
            Some((segment, named))
        })
    }

    /// Where a reply to `root` writes (`MURMURE-105`).
    ///
    /// The thread the root names, held, loading or now asked for. Else this
    /// client's own thread, not linked yet. Else a new one created at `now`.
    ///
    /// # Errors
    /// [`MurmureError::NoMessage`] when no held segment holds the root.
    /// [`MurmureError::NoAmendments`] for a first reply to a root of a locked
    /// segment whose amendments the channel does not hold: they record the
    /// thread (`MURMURE-484`). [`MurmureError::Crdt`] when Loro refuses the
    /// header of a new document.
    pub fn reply_target(
        &mut self,
        root: Id<Message>,
        now: Instant<Authored>,
    ) -> Result<ThreadTarget, MurmureError> {
        let (segment, named) = self.root_slot(root).ok_or(MurmureError::NoMessage(root))?;
        if let Some(id) = named {
            if self.thread_id(root) == Some(id) || !self.ask_thread(id, segment, root) {
                return Ok(ThreadTarget::Write(id));
            }
            return Ok(ThreadTarget::Open(id));
        }
        if let Some(id) = self.thread_id(root) {
            return Ok(ThreadTarget::Write(id));
        }
        if self.amendable(segment) && !self.amendments.contains_key(&segment) {
            return Err(MurmureError::NoAmendments(segment));
        }
        let id = Id::<Document>::mint();
        let created = Created::Thread { segment, root, now };
        let doc = LoroDoc::new();
        created.write(&doc, self.channel_id)?;
        let held = Held {
            created: Some(created),
            ..Held::served(id, doc)
        };
        self.threads.insert(root, Thread { segment, held });
        Ok(ThreadTarget::Create(id))
    }

    /// The relay linked `thread` to `root` of `segment` (`SRV-213`). The
    /// root names it, so every member finds it (`MURMURE-107`).
    ///
    /// When it is not this client's own, the race went to another member
    /// (`SRV-214`). This client drops its own, and fetches the winner. Both
    /// write the same value, so the field converges (`MURMURE-108`).
    ///
    /// # Errors
    /// [`MurmureError::NoSession`] before the PeerID of the segment landed,
    /// and [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn thread_linked(
        &mut self,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    ) -> Result<ThreadLinked, MurmureError> {
        let replaced = self.thread_id(root).filter(|own| *own != thread);
        if replaced.is_some() {
            self.threads.remove(&root);
        }
        let own = self.threads.contains_key(&root);
        let open = own || self.ask_thread(thread, segment, root);
        let root = self.name_thread(segment, root, thread)?;
        Ok(ThreadLinked {
            root,
            open: super::LinkedOpening::from_bool(open),
            replaced,
        })
    }

    /// The link of this client's own `thread` for `root` failed. It is
    /// dropped, so the next reply creates one again. Returns `false` when
    /// the channel holds no such document.
    pub fn thread_failed(&mut self, root: Id<Message>, thread: Id<Document>) -> bool {
        let own = self
            .threads
            .get(&root)
            .is_some_and(|held| held.held.id == thread && held.held.session.is_none());
        if own {
            self.threads.remove(&root);
        }
        own
    }

    /// Write `thread` into the `thread` field of `root`, and nothing else of
    /// it (`MURMURE-110`). `None` when the field is set already. A locked
    /// segment takes no delta, so its amendments record it instead.
    fn name_thread(
        &mut self,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    ) -> Result<Option<Sent>, MurmureError> {
        if self.amendable(segment) {
            return self.record_thread(segment, root, thread);
        }
        let Some(held) = self.held_mut(segment.document_id()) else {
            return Ok(None);
        };
        let (doc, session) = held.session()?;
        let Some(map) = message_map(doc, root) else {
            return Ok(None);
        };
        if schema::string(&map, MessageKey::Thread).is_some() {
            return Ok(None);
        }
        map.insert(MessageKey::Thread, thread.to_string())?;
        let delta = session.export_json(doc)?;
        Ok(Some(self.changed(segment, delta)))
    }

    /// Record `thread` for `root` in the amendments of the locked `segment`
    /// (`MURMURE-484`). `None` when the root names a thread already. `None`
    /// too when the channel holds no amendments for it: the member who
    /// replied first holds them and records it.
    fn record_thread(
        &mut self,
        segment: Id<Segment>,
        root: Id<Message>,
        thread: Id<Document>,
    ) -> Result<Option<Sent>, MurmureError> {
        let held = self
            .doc(segment.document_id())
            .and_then(|doc| message_map(doc, root));
        if held.is_none_or(|map| schema::string(&map, MessageKey::Thread).is_some()) {
            return Ok(None);
        }
        let Some(amendments) = self.amendments.get_mut(&segment) else {
            return Ok(None);
        };
        let document = amendments.id;
        let (doc, session) = amendments.session()?;
        let threads = crate::schema::root_map(doc, Root::Threads);
        let key = root.to_string();
        if schema::string_value(threads.get(&key)).is_some() {
            return Ok(None);
        }
        threads.insert(&key, thread.to_string())?;
        let delta = session.export_json(doc)?;
        let mut sent = self.changed(segment, delta);
        sent.document = document;
        Ok(Some(sent))
    }

    /// The thread documents a held segment's roots name, and that the
    /// channel neither holds nor loads. Each is asked for now: open it, and
    /// [`ChannelState::older_ready`] admits it.
    pub fn threads_to_open(&mut self) -> Vec<Id<Document>> {
        let mut asked = Vec::new();
        for segment in self.held_segments() {
            let Some(doc) = self.doc(segment.document_id()) else {
                continue;
            };
            let mut named = named_threads(doc);
            if let Some(amendments) = self.amendments.get(&segment) {
                named.extend(amending::recorded_threads(&amendments.doc));
            }
            for (root, id) in named {
                if self.threads.contains_key(&root) || !self.ask_thread(id, segment, root) {
                    continue;
                }
                asked.push(id);
            }
        }
        asked
    }

    /// Keep the identity issued for a thread document still arriving.
    /// Returns `false` when no such document is arriving.
    pub(super) fn note_thread_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: Payload<PeerEntryBytes>,
    ) -> bool {
        let Some(pending) = self.threads_pending.get_mut(&document_id) else {
            return false;
        };
        pending.session = Some((peer, entry));
        true
    }

    /// The thread document `document_id` has all its bytes: check its
    /// header against its root, and hold it (`MURMURE-009`).
    ///
    /// # Errors
    /// [`MurmureError::ForeignRoot`] for a header that names another root or
    /// another segment, and every refusal of the header check.
    pub(super) fn thread_ready(
        &mut self,
        document_id: Id<Document>,
        pending: Pending,
    ) -> Result<MurmureEvent, MurmureError> {
        let doc = schema::admit_with(DocumentKind::Thread, self.channel_id, document_id, |doc| {
            delta::import_arrivals(doc, pending.arrivals, document_id, Rules::Thread)
        })?;
        let named = pending.named.ok_or(MurmureError::NotHeld(document_id))?;
        if header_root(&doc) != (Some(named.0), Some(named.1)) {
            return Err(MurmureError::ForeignRoot(document_id));
        }
        let mut held = Held::served(document_id, doc);
        if let Some((peer, entry)) = pending.session {
            held.session = Some(Session::adopt(&held.doc, peer, entry)?);
        }
        let (segment, root) = named;
        self.threads.insert(root, Thread { segment, held });
        let messages = self.read_segment(segment).unwrap_or_default();
        Ok(MurmureEvent::Messages { segment, messages })
    }

    /// Ask for `id` as the thread of `root`, a message of `segment`. A probe
    /// of it becomes this ask, so its header is checked against the root.
    /// `false` when it was on its way already.
    fn ask_thread(&mut self, id: Id<Document>, segment: Id<Segment>, root: Id<Message>) -> bool {
        match self.threads_pending.get_mut(&id) {
            Some(pending) => {
                pending.named.get_or_insert((segment, root));
                false
            }
            None => {
                self.threads_pending.insert(id, Pending::of(segment, root));
                true
            }
        }
    }

    /// The held segment whose root names `thread`, with that root.
    pub fn root_naming(&self, thread: Id<Document>) -> Option<(Id<Segment>, Id<Message>)> {
        self.held_segments().into_iter().find_map(|segment| {
            let mut named = named_threads(self.doc(segment.document_id())?);
            if let Some(amendments) = self.amendments.get(&segment) {
                named.extend(amending::recorded_threads(&amendments.doc));
            }
            named
                .into_iter()
                .find(|(_, id)| *id == thread)
                .map(|(root, _)| (segment, root))
        })
    }

    /// Look for the root of `thread`, which a copy names
    /// (the Murmure threads spec). A new seek replaces the last one.
    ///
    /// Returns the probe to open, `None` when none is to open: a held
    /// segment names the thread, or the thread is on its way already.
    pub fn seek_root(&mut self, thread: Id<Document>) -> Option<Id<Document>> {
        self.seek = Some(Seek {
            thread,
            target: None,
            asked: None,
            trail: Trail::Open,
        });
        // `is_thread` counts a thread still arriving, a probe among them.
        if self.root_naming(thread).is_some() || self.is_thread(thread) {
            return None;
        }
        self.threads_pending.insert(thread, Pending::probe());
        Some(thread)
    }

    /// Whether `document_id` is a probe still arriving: a seek reads its
    /// header, and nothing holds it.
    pub fn is_probe(&self, document_id: Id<Document>) -> bool {
        self.threads_pending
            .get(&document_id)
            .is_some_and(|pending| pending.named.is_none())
    }

    /// The probe `document_id` has all its bytes: its header names the
    /// segment the seek loads down to. The probe is then dropped.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for no such probe, every refusal of the
    /// header check, and [`MurmureError::ForeignRoot`] for a header naming no
    /// segment `meta` lists. The next step of the seek is then lost.
    pub fn probe_ready(&mut self, document_id: Id<Document>) -> Result<(), MurmureError> {
        let read = self.read_probe(document_id);
        if let Some(seek) = self.seek.as_mut().filter(|seek| seek.thread == document_id) {
            match &read {
                Ok(segment) => seek.target = Some(*segment),
                Err(_) => seek.trail = Trail::Lost,
            }
        }
        read.map(|_| ())
    }

    fn read_probe(&mut self, document_id: Id<Document>) -> Result<Id<Segment>, MurmureError> {
        if !self.is_probe(document_id) {
            return Err(MurmureError::NotHeld(document_id));
        }
        let pending = self
            .threads_pending
            .remove(&document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        self.sequences.remove(&document_id);
        let doc = schema::admit_with(DocumentKind::Thread, self.channel_id, document_id, |doc| {
            delta::import_arrivals(doc, pending.arrivals, document_id, Rules::Thread)
        })?;
        header_root(&doc)
            .0
            .filter(|segment| self.listed_as(*segment).is_some())
            .ok_or(MurmureError::ForeignRoot(document_id))
    }

    /// The next move of the seek, `None` with no seek. A found root and a
    /// lost one end it.
    ///
    /// It asks for one older segment at a time, so the list stays whole,
    /// and never one older than the segment the header names. A segment
    /// that failed is not asked for twice.
    pub fn seek_step(&mut self) -> Option<SeekStep> {
        let seek = self.seek?;
        let thread = seek.thread;
        let step = if seek.trail == Trail::Lost {
            SeekStep::Lost { thread }
        } else if let Some((segment, root)) = self.root_naming(thread) {
            SeekStep::Found {
                thread,
                segment,
                root,
            }
        } else if let Some(target) = seek.target {
            self.seek_older(target, seek.asked)
                .unwrap_or(SeekStep::Lost { thread })
        } else {
            SeekStep::Wait
        };
        match step {
            SeekStep::Found { .. } | SeekStep::Lost { .. } => self.seek = None,
            SeekStep::Load(segment) => {
                if let Some(seek) = self.seek.as_mut() {
                    seek.asked = Some(segment);
                }
            }
            SeekStep::Wait => {}
        }
        Some(step)
    }

    /// The next older segment to load down to `target`. `None` when the
    /// root is out of reach.
    fn seek_older(&mut self, target: Id<Segment>, asked: Option<Id<Segment>>) -> Option<SeekStep> {
        if self.held_segments().contains(&target) {
            return None;
        }
        if !self.pending.is_empty() {
            return Some(SeekStep::Wait);
        }
        let oldest = self.oldest_held()?;
        let older = self
            .older_than(oldest)
            .filter(|older| Some(*older) != asked)?;
        self.request_older(oldest);
        Some(SeekStep::Load(older))
    }

    /// The thread `document_id` could not open: a seek still waiting on its
    /// probe is lost.
    pub(super) fn end_seek_of(&mut self, document_id: Id<Document>) {
        if let Some(seek) = self
            .seek
            .as_mut()
            .filter(|seek| seek.thread == document_id && seek.target.is_none())
        {
            seek.trail = Trail::Lost;
        }
    }

    /// The held segment `meta` lists first: the oldest the list shows.
    fn oldest_held(&self) -> Option<Id<Segment>> {
        let held = self.held_segments();
        segment_entries(&self.meta.doc)
            .into_iter()
            .map(|(id, _)| id)
            .find(|id| held.contains(id))
    }

    /// The replies of the held thread document `document_id`, for the view.
    /// `None` for any other document.
    pub fn thread_view(&self, document_id: Id<Document>) -> Option<MurmureEvent> {
        let (root, thread) = self.thread_of(document_id)?;
        Some(MurmureEvent::ThreadMessages {
            root,
            thread: document_id,
            messages: read_messages(&thread.held.doc, self.channel_id),
        })
    }

    /// What a reaction delta in the thread of `root` produced: the delta,
    /// its summary of kind reaction, and the replies for the view
    /// (`MURMURE-485`).
    ///
    /// # Errors
    /// [`MurmureError::NoThread`] for a thread the channel does not hold.
    pub(super) fn thread_reacted(
        &self,
        root: Id<Message>,
        delta: Payload<DeltaBytes>,
    ) -> Result<Sent, MurmureError> {
        let thread = self
            .threads
            .get(&root)
            .ok_or(MurmureError::NoThread(root))?;
        let document = thread.held.id;
        let event = self
            .thread_view(document)
            .ok_or(MurmureError::NoThread(root))?;
        Ok(Sent {
            segment: thread.segment,
            document,
            delta,
            summary: reaction_summary(),
            event,
        })
    }

    /// The segment that holds `root`, read with its thread heads. A delete
    /// in its thread moves its count (`MURMURE-106`). `None` when no held
    /// segment holds it.
    pub fn root_view(&self, root: Id<Message>) -> Option<MurmureEvent> {
        let segment = self.root_segment(root)?;
        let messages = self.read_segment(segment)?;
        Some(MurmureEvent::Messages { segment, messages })
    }

    /// The root's segment of the held thread document `document_id`, read
    /// with its thread heads. `None` for any other document.
    pub(super) fn thread_answer(&self, document_id: Id<Document>) -> Option<Vec<MurmureEvent>> {
        let view = self.thread_view(document_id)?;
        let (_, thread) = self.thread_of(document_id)?;
        let segment = thread.segment;
        let mut events = vec![view];
        if let Some(messages) = self.read_segment(segment) {
            events.push(MurmureEvent::Messages { segment, messages });
        }
        Some(events)
    }

    /// Reply to `root` with `draft` (`MURMURE-105`). A reply also sent to
    /// the channel writes a copy in the active segment that names its
    /// thread (`MURMURE-109`).
    ///
    /// # Errors
    /// [`MurmureError::BadMention`] for a mention past the draft,
    /// [`MurmureError::BadStyle`] for a style it refuses, and
    /// [`MurmureError::NoThread`] before the channel holds the thread.
    /// [`MurmureError::NoSession`] before the PeerID of the thread, or of
    /// the active segment for a copy, landed. [`MurmureError::Crdt`] when
    /// Loro refuses a write or the export.
    pub fn reply(
        &mut self,
        root: Id<Message>,
        draft: &MessageDraft,
        author: Id<User>,
        now: Instant<Authored>,
        echo: ReplyEcho,
    ) -> Result<Replied, MurmureError> {
        if !mention::fits(&draft.mentions, &draft.text) {
            return Err(MurmureError::BadMention);
        }
        if !style::fits(&draft.styles, &draft.text) {
            return Err(MurmureError::BadStyle);
        }
        // Checked before the first write, so a reply never lands without
        // the copy it was sent with.
        if (echo == ReplyEcho::AlsoInChannel) && self.active.session.is_none() {
            return Err(MurmureError::NoSession(self.active.id));
        }
        let channel = self.channel_id;
        let thread = self
            .threads
            .get_mut(&root)
            .ok_or(MurmureError::NoThread(root))?;
        let segment = thread.segment;
        let document = thread.held.id;
        let (doc, session) = thread.held.session()?;
        let (id, mentions, _) = super::write_message(doc, draft, &[], author, now, channel, None)?;
        let delta = session.export_json(doc)?;
        let event = self
            .thread_view(document)
            .ok_or(MurmureError::NoThread(root))?;
        let reply = Sent {
            segment,
            document,
            delta,
            summary: summary_of(&mentions),
            event,
        };
        let copy = if echo == ReplyEcho::AlsoInChannel {
            Some(self.send_from(draft, &[], author, now, Some((document, id)))?)
        } else {
            None
        };
        let messages = self.read_segment(segment).unwrap_or_default();
        Ok(Replied {
            reply,
            copy,
            head: MurmureEvent::Messages { segment, messages },
        })
    }

    /// The copy in the channel of `reply`, a reply to `root`, with the held
    /// segment it lies in (`MURMURE-538`). `None` for a reply sent to its
    /// thread alone, or a thread the channel does not hold.
    ///
    /// The copy is written after its root. The held segments run from the
    /// active one back, so a held root leaves its copy held too.
    pub fn copy_of(
        &self,
        root: Id<Message>,
        reply: Id<Message>,
    ) -> Option<(Id<Segment>, Id<Message>)> {
        let thread = self.thread_id(root)?.to_string();
        let copy = copy_id(reply);
        self.held_segments().into_iter().find_map(|segment| {
            let map = message_map(self.doc(segment.document_id())?, copy)?;
            (schema::string(&map, MessageKey::AlsoInThread).as_deref() == Some(thread.as_str()))
                .then_some((segment, copy))
        })
    }

    /// Read the head of each thread the held threads hold over the roots of
    /// `messages` (`MURMURE-106`). A thread not held keeps no count.
    pub(super) fn overlay_threads(&self, messages: &mut [MessageView]) {
        for view in messages.iter_mut() {
            let Some(head) = view.thread.as_mut() else {
                continue;
            };
            let Some((_, thread)) = self.thread_of(head.document) else {
                continue;
            };
            *head = thread_head(head.document, &thread.held.doc, self.channel_id);
        }
    }
}

/// The identifier of the copy of `reply` in the channel: the reply's own,
/// its last bit turned (`MURMURE-538`). The turn is its own inverse, and it
/// keeps the version, the variant and the instant of the identifier.
pub(super) fn copy_id(reply: Id<Message>) -> Id<Message> {
    let mut bytes = *reply.as_bytes();
    bytes[15] ^= 1;
    Id::<Message>::from_bytes(bytes)
}

/// What a root shows of `thread`: its replies, deleted ones aside, and the
/// author of the latest one.
fn thread_head(document: Id<Document>, thread: &LoroDoc, channel: Id<Channel>) -> ThreadHead {
    let replies: Vec<MessageView> = read_messages(thread, channel)
        .into_iter()
        .filter(|reply| reply.deleted == Deletion::Live)
        .collect();
    ThreadHead {
        document,
        replies: ambre_core::Count::<crate::message::Replies>::of(replies.len()),
        last_replier: replies.last().map(|reply| reply.author),
    }
}

/// Every message of a segment whose `thread` field names a document, with
/// that document, in list order.
/// The segment and the root a thread document's header names.
fn header_root(doc: &LoroDoc) -> (Option<Id<Segment>>, Option<Id<Message>>) {
    let header = crate::schema::header(doc);
    let segment =
        schema::string(&header, HeaderKey::Segment).and_then(|s| Id::<Segment>::parse_str(&s).ok());
    let root =
        schema::string(&header, HeaderKey::Root).and_then(|s| Id::<Message>::parse_str(&s).ok());
    (segment, root)
}

fn named_threads(doc: &LoroDoc) -> Vec<(Id<Message>, Id<Document>)> {
    let mut named = Vec::new();
    crate::schema::root_list(doc, Root::Messages).for_each(|item| {
        let Some(map) = crate::schema::fields_of(item) else {
            return;
        };
        let root =
            schema::string(&map, MessageKey::Id).and_then(|s| Id::<Message>::parse_str(&s).ok());
        let thread = schema::string(&map, MessageKey::Thread)
            .and_then(|s| Id::<Document>::parse_str(&s).ok());
        if let (Some(root), Some(thread)) = (root, thread) {
            named.push((root, thread));
        }
    });
    named
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod copy_id_tests {
    use ambre_core::id::{Id, Message};
    use proptest::prelude::*;

    use super::copy_id;

    proptest! {
        /// A copy never takes the identifier of its reply, and the turn is its
        /// own inverse, so two replies never share a copy (`MURMURE-538`).
        #[test]
        fn a_copy_differs_from_its_reply_and_from_every_other_copy(
            bytes in any::<[u8; 16]>(),
            other in any::<[u8; 16]>(),
        ) {
            let reply = Id::<Message>::from_bytes(bytes);
            let copy = copy_id(reply);
            prop_assert_ne!(copy, reply);
            prop_assert_eq!(copy_id(copy), reply);
            let other = Id::<Message>::from_bytes(other);
            prop_assert_eq!(copy == copy_id(other), reply == other);
        }
    }
}
