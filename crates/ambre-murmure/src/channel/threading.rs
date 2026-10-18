//! The threads of a channel (`features/ambre-spec-murmure-threads.md section 3`).
//!
//! The replies to one message live in one thread document (`MURMURE-105`).
//! A client creates it on the first reply, and the relay links one per root
//! (`SRV-213`, `SRV-214`). The root then names it in its `thread` field
//! (`MURMURE-107`), the one field of it another member may write
//! (`MURMURE-255`).

use ambre_bus::{MessageDraft, MessageView, MurmureEvent, ThreadHead, Timestamp};
use ambre_core::{DocumentId, MessageId, SegmentId, UserId};
use loro::{Container, LoroDoc, ValueOrContainer};

use super::{
    ChannelState, Created, Held, Sent, message_map, reaction_summary, read_messages, summary_of,
};
use crate::delta::{self, Arrival, Rules};
use crate::error::MurmureError;
use crate::mention;
use crate::schema::{
    self, DocumentKind, HEADER, HEADER_ROOT, HEADER_SEGMENT, MESSAGE_ID, MESSAGE_THREAD, MESSAGES,
};
use crate::session::Session;

/// A thread document the channel holds, with the segment of its root.
#[derive(Debug)]
pub(super) struct Thread {
    /// The segment its root lies in, as its header names it.
    pub(super) segment: SegmentId,
    pub(super) held: Held,
}

/// A thread document whose bytes are still arriving.
#[derive(Debug)]
pub(super) struct Pending {
    /// The segment of its root.
    segment: SegmentId,
    /// The message it holds the replies to.
    root: MessageId,
    /// Its payloads, in the order they came.
    pub(super) arrivals: Vec<Arrival>,
    /// The identity issued for it, adopted once it is admitted.
    session: Option<(u64, Vec<u8>)>,
}

impl Pending {
    fn of(segment: SegmentId, root: MessageId) -> Self {
        Self {
            segment,
            root,
            arrivals: Vec::new(),
            session: None,
        }
    }
}

/// The document a reply writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadTarget {
    /// The thread document the channel holds or is loading for the root.
    Write(DocumentId),
    /// The thread document the root names, now asked for: the reply waits
    /// until it is held.
    Open(DocumentId),
    /// A new thread document, held here with no session. The Sync worker
    /// creates it on the relay and links it to the root (`SRV-213`).
    Create(DocumentId),
}

/// What a link of a thread document produced.
#[derive(Debug)]
pub struct ThreadLinked {
    /// The delta that names the document in the root's `thread` field,
    /// `None` when the field named it already (`MURMURE-107`).
    pub root: Option<Sent>,
    /// Whether the document is to open: this client's own, for its session,
    /// or another member's, for its bytes.
    pub open: bool,
    /// This client's own document, which lost the race to another member's
    /// (`SRV-214`). What waited on it waits on the winner.
    pub replaced: Option<DocumentId>,
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
    fn thread_of(&self, document_id: DocumentId) -> Option<(MessageId, &Thread)> {
        self.threads
            .iter()
            .find(|(_, thread)| thread.held.id == document_id)
            .map(|(root, thread)| (*root, thread))
    }

    /// Whether `document_id` is a thread document, held or arriving.
    pub(super) fn is_thread(&self, document_id: DocumentId) -> bool {
        self.threads_pending.contains_key(&document_id) || self.thread_of(document_id).is_some()
    }

    /// The thread document the channel holds for `root`.
    pub fn thread_id(&self, root: MessageId) -> Option<DocumentId> {
        self.threads.get(&root).map(|thread| thread.held.id)
    }

    /// The held segment that holds `root`: the one its thread is linked to.
    pub fn root_segment(&self, root: MessageId) -> Option<SegmentId> {
        self.root_slot(root).map(|(segment, _)| segment)
    }

    /// The held segment that holds `root`, with its `thread` field.
    fn root_slot(&self, root: MessageId) -> Option<(SegmentId, Option<DocumentId>)> {
        self.held_segments().into_iter().find_map(|segment| {
            let map = message_map(self.doc(segment.document_id())?, root)?;
            let named =
                schema::string(&map, MESSAGE_THREAD).and_then(|s| DocumentId::parse_str(&s).ok());
            Some((segment, named))
        })
    }

    /// Where a reply to `root` writes (`MURMURE-105`).
    ///
    /// The thread the root names, held, loading or now asked for. Else this
    /// client's own thread, not linked yet. Else a new one created at `now`.
    ///
    /// # Errors
    /// [`MurmureError::NoMessage`] when no held segment holds the root, and
    /// [`MurmureError::RootLocked`] for a first reply to a root of a final
    /// or locked segment. [`MurmureError::Crdt`] when Loro refuses the
    /// header of a new document.
    pub fn reply_target(
        &mut self,
        root: MessageId,
        now: Timestamp,
    ) -> Result<ThreadTarget, MurmureError> {
        let (segment, named) = self.root_slot(root).ok_or(MurmureError::NoMessage(root))?;
        if let Some(id) = named {
            if self.thread_id(root) == Some(id) || self.threads_pending.contains_key(&id) {
                return Ok(ThreadTarget::Write(id));
            }
            self.threads_pending.insert(id, Pending::of(segment, root));
            return Ok(ThreadTarget::Open(id));
        }
        if let Some(id) = self.thread_id(root) {
            return Ok(ThreadTarget::Write(id));
        }
        if self.amendable(segment) {
            return Err(MurmureError::RootLocked(root));
        }
        let id = DocumentId::new();
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
        segment: SegmentId,
        root: MessageId,
        thread: DocumentId,
    ) -> Result<ThreadLinked, MurmureError> {
        let replaced = self.thread_id(root).filter(|own| *own != thread);
        if replaced.is_some() {
            self.threads.remove(&root);
        }
        let own = self.threads.contains_key(&root);
        let open = own || !self.threads_pending.contains_key(&thread);
        if !own && open {
            self.threads_pending
                .insert(thread, Pending::of(segment, root));
        }
        let root = self.name_thread(segment, root, thread)?;
        Ok(ThreadLinked {
            root,
            open,
            replaced,
        })
    }

    /// The link of this client's own `thread` for `root` failed. It is
    /// dropped, so the next reply creates one again. Returns `false` when
    /// the channel holds no such document.
    pub fn thread_failed(&mut self, root: MessageId, thread: DocumentId) -> bool {
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
    /// it (`MURMURE-110`). `None` when the field is set already, or when the
    /// segment is no longer live: the relay refuses its deltas.
    fn name_thread(
        &mut self,
        segment: SegmentId,
        root: MessageId,
        thread: DocumentId,
    ) -> Result<Option<Sent>, MurmureError> {
        if self.amendable(segment) {
            return Ok(None);
        }
        let Some(held) = self.held_mut(segment.document_id()) else {
            return Ok(None);
        };
        let (doc, session) = held.session()?;
        let Some(map) = message_map(doc, root) else {
            return Ok(None);
        };
        if schema::string(&map, MESSAGE_THREAD).is_some() {
            return Ok(None);
        }
        map.insert(MESSAGE_THREAD, thread.to_string())?;
        let delta = session.export_json(doc)?;
        Ok(Some(self.changed(segment, delta)))
    }

    /// The thread documents a held segment's roots name, and that the
    /// channel neither holds nor loads. Each is asked for now: open it, and
    /// [`ChannelState::older_ready`] admits it.
    pub fn threads_to_open(&mut self) -> Vec<DocumentId> {
        let mut asked = Vec::new();
        for segment in self.held_segments() {
            let Some(doc) = self.doc(segment.document_id()) else {
                continue;
            };
            for (root, id) in named_threads(doc) {
                if self.threads.contains_key(&root) || self.threads_pending.contains_key(&id) {
                    continue;
                }
                self.threads_pending.insert(id, Pending::of(segment, root));
                asked.push(id);
            }
        }
        asked
    }

    /// Keep the identity issued for a thread document still arriving.
    /// Returns `false` when no such document is arriving.
    pub(super) fn note_thread_session(
        &mut self,
        document_id: DocumentId,
        peer: u64,
        entry: Vec<u8>,
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
        document_id: DocumentId,
        pending: Pending,
    ) -> Result<MurmureEvent, MurmureError> {
        let doc = schema::admit_with(DocumentKind::Thread, self.channel_id, document_id, |doc| {
            delta::import_arrivals(doc, pending.arrivals, document_id, Rules::Thread)
        })?;
        let header = doc.get_map(HEADER);
        let root = schema::string(&header, HEADER_ROOT).and_then(|s| MessageId::parse_str(&s).ok());
        let segment =
            schema::string(&header, HEADER_SEGMENT).and_then(|s| SegmentId::parse_str(&s).ok());
        if root != Some(pending.root) || segment != Some(pending.segment) {
            return Err(MurmureError::ForeignRoot(document_id));
        }
        let mut held = Held::served(document_id, doc);
        if let Some((peer, entry)) = pending.session {
            held.session = Some(Session::adopt(&held.doc, peer, entry)?);
        }
        let segment = pending.segment;
        self.threads.insert(pending.root, Thread { segment, held });
        let messages = self.read_segment(segment).unwrap_or_default();
        Ok(MurmureEvent::Messages { segment, messages })
    }

    /// The replies of the held thread document `document_id`, for the view.
    /// `None` for any other document.
    pub fn thread_view(&self, document_id: DocumentId) -> Option<MurmureEvent> {
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
        root: MessageId,
        delta: Vec<u8>,
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
    pub fn root_view(&self, root: MessageId) -> Option<MurmureEvent> {
        let segment = self.root_segment(root)?;
        let messages = self.read_segment(segment)?;
        Some(MurmureEvent::Messages { segment, messages })
    }

    /// The root's segment of the held thread document `document_id`, read
    /// with its thread heads. `None` for any other document.
    pub(super) fn thread_answer(&self, document_id: DocumentId) -> Option<Vec<MurmureEvent>> {
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
    /// [`MurmureError::BadMention`] for a mention past the draft, and
    /// [`MurmureError::NoThread`] before the channel holds the thread.
    /// [`MurmureError::NoSession`] before the PeerID of the thread, or of
    /// the active segment for a copy, landed. [`MurmureError::Crdt`] when
    /// Loro refuses a write or the export.
    pub fn reply(
        &mut self,
        root: MessageId,
        draft: &MessageDraft,
        author: UserId,
        now: Timestamp,
        also_in_channel: bool,
    ) -> Result<Replied, MurmureError> {
        if !mention::fits(&draft.mentions, &draft.text) {
            return Err(MurmureError::BadMention);
        }
        // Checked before the first write, so a reply never lands without
        // the copy it was sent with.
        if also_in_channel && self.active.session.is_none() {
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
        let (_, mentions) = super::write_message(doc, draft, author, now, channel, None)?;
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
        let copy = if also_in_channel {
            Some(self.send_from(draft, author, now, Some(document))?)
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

/// What a root shows of `thread`: its replies, deleted ones aside, and the
/// author of the latest one.
fn thread_head(
    document: DocumentId,
    thread: &LoroDoc,
    channel: ambre_core::ChannelId,
) -> ThreadHead {
    let replies: Vec<MessageView> = read_messages(thread, channel)
        .into_iter()
        .filter(|reply| !reply.deleted)
        .collect();
    ThreadHead {
        document,
        replies: u32::try_from(replies.len()).unwrap_or(u32::MAX),
        last_replier: replies.last().map(|reply| reply.author),
    }
}

/// Every message of a segment whose `thread` field names a document, with
/// that document, in list order.
fn named_threads(doc: &LoroDoc) -> Vec<(MessageId, DocumentId)> {
    let mut named = Vec::new();
    doc.get_list(MESSAGES).for_each(|item| {
        let ValueOrContainer::Container(Container::Map(map)) = item else {
            return;
        };
        let root = schema::string(&map, MESSAGE_ID).and_then(|s| MessageId::parse_str(&s).ok());
        let thread =
            schema::string(&map, MESSAGE_THREAD).and_then(|s| DocumentId::parse_str(&s).ok());
        if let (Some(root), Some(thread)) = (root, thread) {
            named.push((root, thread));
        }
    });
    named
}
