//! The state of an open channel on the Logic thread
//! (`features/ambre-spec-murmure-engine.md section 3`).
//!
//! A channel is a set of Loro documents, never one (`MURMURE-021`). It holds
//! its `meta` document, its active segment and the segment being closed. It
//! also holds the older segments the view asked for. Each document keeps its own identifier, so a
//! remote delta reaches the one it names and no other (`MURMURE-024`).

use std::collections::HashMap;

use ambre_bus::{
    ActivityKind, ActivitySummary, MentionTarget, MessageDraft, MessageView, MurmureEvent,
    Timestamp, UnreadCount,
};
use ambre_core::{ChannelId, DocumentId, MessageId, MetaId, SegmentId, Seq, UserId};
use loro::{Container, ExportMode, LoroDoc, LoroList, LoroMap, LoroText, ValueOrContainer};

use crate::error::MurmureError;
use crate::schema::{
    self, CHANNEL, CHANNEL_NAME, CHANNEL_TYPE, ChannelType, DocumentKind, MESSAGE_AUTHOR,
    MESSAGE_CONTENT, MESSAGE_ID, MESSAGE_TS, MESSAGES, SEGMENT_DOCUMENT, SEGMENT_FROM,
    SEGMENT_STATE, SEGMENTS, SegmentState,
};
use crate::sequence::{self, Order, Position, Sequence};
use crate::session::Session;

/// One document the channel holds, with the identifier the Sync worker routes
/// it by.
#[derive(Debug)]
struct Held {
    id: DocumentId,
    doc: LoroDoc,
    /// The PeerID this session writes the document under. `None` until the
    /// Sync worker issues it, and nothing is written before (`AUTHOR-029`).
    session: Option<Session>,
    /// What a document created here holds, written again under its PeerID
    /// once the session lands. `None` for a document the relay served.
    created: Option<Created>,
}

impl Held {
    /// A document the relay served, with no session yet.
    fn served(id: DocumentId, doc: LoroDoc) -> Self {
        Self {
            id,
            doc,
            session: None,
            created: None,
        }
    }

    /// The session this document writes under.
    ///
    /// # Errors
    /// [`MurmureError::NoSession`] before the Sync worker issued it.
    fn session(&mut self) -> Result<(&LoroDoc, &mut Session), MurmureError> {
        let session = self
            .session
            .as_mut()
            .ok_or(MurmureError::NoSession(self.id))?;
        Ok((&self.doc, session))
    }
}

/// The content of a document this client created. It is written at once, so
/// the channel reads before its sessions land. It is written again from
/// scratch under the PeerID of its session: the first write never travels.
#[derive(Debug, Clone)]
enum Created {
    /// The `meta` document, listing the first active segment.
    Meta {
        channel_type: ChannelType,
        name: Option<String>,
        active: SegmentId,
        now: Timestamp,
    },
    /// The first active segment, which holds no message yet.
    Segment { now: Timestamp },
}

impl Created {
    /// Write this content into `doc`, and commit it.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    fn write(&self, doc: &LoroDoc, channel_id: ChannelId) -> Result<(), MurmureError> {
        match self {
            Self::Meta {
                channel_type,
                name,
                active,
                now,
            } => {
                schema::write_header(doc, DocumentKind::Meta, channel_id, now.0)?;
                let channel = doc.get_map(CHANNEL);
                channel.insert(CHANNEL_TYPE, channel_type.as_str())?;
                if let Some(name) = name {
                    channel.insert(CHANNEL_NAME, name.as_str())?;
                }
                push_segment_entry(doc, *active, *now, SegmentState::Active)?;
            }
            Self::Segment { now } => {
                schema::write_header(doc, DocumentKind::Segment, channel_id, now.0)?;
            }
        }
        doc.commit();
        Ok(())
    }
}

/// What a sent message produced: the delta to hand the Sync worker, its
/// activity summary, and the event to hand the view.
#[derive(Debug)]
pub struct Sent {
    /// The segment the message went to, the address of the delta.
    pub segment: SegmentId,
    /// Every operation of the send, as a Loro update.
    pub delta: Vec<u8>,
    /// What the delta holds, readable with no document (`MURMURE-081`).
    pub summary: ActivitySummary,
    /// The new message, for the view.
    pub event: MurmureEvent,
}

/// What a name change produced: the delta to hand the Sync worker under
/// `meta`, and the event to hand the view.
#[derive(Debug)]
pub struct Named {
    /// The document the name lives in, the address of the delta.
    pub meta: MetaId,
    /// Every operation of the change, as a Loro update. Empty of operations
    /// when the name was already so.
    pub delta: Vec<u8>,
    /// The name now, for the view.
    pub event: MurmureEvent,
}

/// An open channel: its documents, each apart (`MURMURE-021`).
///
/// No thread document yet: threads arrive with their Story, and the spec's
/// `threads` field with them.
#[derive(Debug)]
pub struct ChannelState {
    channel_id: ChannelId,
    meta: Held,
    active: Held,
    closing: Option<Held>,
    /// Final segments, loaded only on the view's demand (`MURMURE-023`).
    loaded: HashMap<SegmentId, LoroDoc>,
    /// Final segments the view asked for, whose bytes are still arriving.
    /// Nothing reads them until [`ChannelState::older_ready`] admits them.
    pending: HashMap<SegmentId, Vec<Vec<u8>>>,
    /// The relay's number of each message, per segment
    /// (`features/ambre-spec-murmure-unread.md section 3`). A segment that
    /// came with no number has no entry.
    sequences: HashMap<DocumentId, Sequence>,
}

impl ChannelState {
    /// Create a new channel: its `meta` document and its first active segment.
    ///
    /// `name` stays absent when `None`, never an empty string (`MURMURE-018`).
    ///
    /// The channel reads at once. Each document is written again under its
    /// PeerID when its session lands, and only then does it leave
    /// ([`ChannelState::adopt_session`]).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    pub fn create(
        channel_id: ChannelId,
        channel_type: ChannelType,
        name: Option<&str>,
        now: Timestamp,
    ) -> Result<Self, MurmureError> {
        let active_id = DocumentId::new();
        let created = |id: DocumentId, created: Created| -> Result<Held, MurmureError> {
            let doc = LoroDoc::new();
            created.write(&doc, channel_id)?;
            Ok(Held {
                created: Some(created),
                ..Held::served(id, doc)
            })
        };
        let meta = created(
            DocumentId::new(),
            Created::Meta {
                channel_type,
                name: name.map(str::to_owned),
                active: SegmentId::from_document(active_id),
                now,
            },
        )?;
        let active = created(active_id, Created::Segment { now })?;
        Ok(Self::assemble(channel_id, meta, active, None))
    }

    /// Write `document_id` under the PeerID the Sync worker issued for this
    /// session, with its registry entry (`AUTHOR-002`, `AUTHOR-009`).
    ///
    /// A document created here is written again from scratch under it, and
    /// its whole content returns: the first update, which creates it on the
    /// relay. A document the relay served returns nothing: its entry rides
    /// with its first delta. An older segment is final and writes nothing, so
    /// it takes no entry. The same identity issued twice changes nothing.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a document the channel does not hold,
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn adopt_session(
        &mut self,
        document_id: DocumentId,
        peer: u64,
        entry: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, MurmureError> {
        let channel_id = self.channel_id;
        let Some(held) = self.held_mut(document_id) else {
            return if self.holds(document_id) {
                Ok(None)
            } else {
                Err(MurmureError::NotHeld(document_id))
            };
        };
        if held.session.map(Session::peer) == Some(peer) {
            return Ok(None);
        }
        let Some(created) = &held.created else {
            held.session = Some(Session::adopt(&held.doc, peer, entry)?);
            return Ok(None);
        };
        let doc = LoroDoc::new();
        let mut session = Session::adopt(&doc, peer, entry)?;
        created.write(&doc, channel_id)?;
        let first = session.export(&doc)?;
        held.doc = doc;
        held.session = Some(session);
        held.created = None;
        Ok(Some(first))
    }

    /// This channel with every live document under a fresh PeerID, as the
    /// Sync worker would issue them.
    #[cfg(test)]
    pub(crate) fn identify(mut self) -> Self {
        let ids: Vec<DocumentId> = self.live().map(|held| held.id).collect();
        for id in ids {
            let peer = crate::session::next_peer();
            self.adopt_session(id, peer, peer.to_le_bytes().to_vec())
                .expect("adopt");
        }
        self
    }

    /// Whether this session may write `document_id`: its PeerID landed.
    pub fn identified(&self, document_id: DocumentId) -> bool {
        self.live()
            .any(|held| held.id == document_id && held.session.is_some())
    }

    /// The live documents: `meta`, the active segment and the one being
    /// closed.
    fn live(&self) -> impl Iterator<Item = &Held> {
        [Some(&self.meta), Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
    }

    /// The live document of that identifier, to write it.
    fn held_mut(&mut self, document_id: DocumentId) -> Option<&mut Held> {
        [
            Some(&mut self.meta),
            Some(&mut self.active),
            self.closing.as_mut(),
        ]
        .into_iter()
        .flatten()
        .find(|held| held.id == document_id)
    }

    /// A channel of these documents, each already admitted.
    fn assemble(channel_id: ChannelId, meta: Held, active: Held, closing: Option<Held>) -> Self {
        Self {
            channel_id,
            meta,
            active,
            closing,
            loaded: HashMap::new(),
            pending: HashMap::new(),
            sequences: HashMap::new(),
        }
    }

    /// Open a channel from stored bytes: its `meta` document, then the live
    /// segments `meta` lists (`MURMURE-022`).
    ///
    /// A final segment is never loaded here, even when `segments` holds its
    /// bytes (`MURMURE-028`). Every document passes the header check first
    /// (`MURMURE-019`, `MURMURE-027`).
    ///
    /// # Errors
    /// Every refusal of the header check, [`MurmureError::NoActiveSegment`],
    /// and [`MurmureError::SegmentMissing`] for a live segment with no bytes.
    pub fn open(
        channel_id: ChannelId,
        meta_id: MetaId,
        meta_bytes: &[u8],
        segments: &HashMap<SegmentId, Vec<u8>>,
    ) -> Result<Self, MurmureError> {
        let meta_id = meta_id.document_id();
        let meta = Held::served(
            meta_id,
            schema::admit(meta_bytes, DocumentKind::Meta, channel_id, meta_id)?,
        );
        let live = live_segments(&meta.doc, channel_id)?;
        let admit_live = |segment: SegmentId| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let bytes = segments
                .get(&segment)
                .ok_or(MurmureError::SegmentMissing(id))?;
            let doc = schema::admit(bytes, DocumentKind::Segment, channel_id, id)?;
            Ok(Held::served(id, doc))
        };
        let active = admit_live(live.active)?;
        let closing = live.closing.map(admit_live).transpose()?;
        Ok(Self::assemble(channel_id, meta, active, closing))
    }

    /// Open a channel from documents whose bytes arrived as several payloads
    /// each: a snapshot, then a replay (`MURMURE-022`).
    ///
    /// `segments` holds the live segments `meta` lists, and the same checks
    /// as [`ChannelState::open`] run on each. `sequences` holds what the
    /// relay numbered of each segment, matched here against its messages.
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::open`].
    pub(crate) fn open_from_payloads(
        channel_id: ChannelId,
        meta: (DocumentId, LoroDoc),
        segments: &HashMap<DocumentId, Vec<Vec<u8>>>,
        sequences: HashMap<DocumentId, Sequence>,
    ) -> Result<Self, MurmureError> {
        let (meta_id, meta_doc) = meta;
        let live = live_segments(&meta_doc, channel_id)?;
        let admit_live = |segment: SegmentId| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let payloads = segments.get(&id).ok_or(MurmureError::SegmentMissing(id))?;
            let doc =
                schema::admit_all(&borrowed(payloads), DocumentKind::Segment, channel_id, id)?;
            Ok(Held::served(id, doc))
        };
        let active = admit_live(live.active)?;
        let closing = live.closing.map(admit_live).transpose()?;
        let meta = Held::served(meta_id, meta_doc);
        let mut state = Self::assemble(channel_id, meta, active, closing);
        for (document_id, mut sequence) in sequences {
            // Only a held segment keeps its numbers: `meta` has no message,
            // and a segment no longer live was not admitted.
            let Some(doc) = state.doc(document_id).filter(|_| document_id != meta_id) else {
                continue;
            };
            sequence.resolve(doc);
            state.sequences.insert(document_id, sequence);
        }
        Ok(state)
    }

    /// Who may read the channel, as its `meta` document states it. `None`
    /// for a word no build wrote.
    pub fn channel_type(&self) -> Option<ChannelType> {
        schema::string(&self.meta.doc.get_map(CHANNEL), CHANNEL_TYPE)
            .and_then(|word| ChannelType::parse(&word))
    }

    /// The name of the channel, absent when it has none (`MURMURE-018`).
    pub fn name(&self) -> Option<String> {
        schema::string(&self.meta.doc.get_map(CHANNEL), CHANNEL_NAME)
    }

    /// Name a direct message in its `meta`, or clear its name with `None`
    /// (`MURMURE-054`). A name of blanks alone clears it too. A cleared name
    /// is an absent key, never an empty string (`MURMURE-055`). The engine
    /// stores what the member typed, trimmed, and composes nothing
    /// (`MURMURE-016`).
    ///
    /// # Errors
    /// [`MurmureError::NotDirect`] for any other channel: a public channel's
    /// name lives in its catalogue too. [`MurmureError::Crdt`] when Loro
    /// refuses a write or the export. [`MurmureError::NoSession`] before the
    /// PeerID of `meta` landed.
    pub fn name_direct(&mut self, name: Option<&str>) -> Result<Named, MurmureError> {
        if self.channel_type() != Some(ChannelType::Direct) {
            return Err(MurmureError::NotDirect(self.channel_id));
        }
        let meta = self.meta_id();
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let (doc, session) = self.meta.session()?;
        let channel = doc.get_map(CHANNEL);
        match name {
            Some(name) => channel.insert(CHANNEL_NAME, name)?,
            None if channel.get(CHANNEL_NAME).is_some() => channel.delete(CHANNEL_NAME)?,
            None => {}
        }
        Ok(Named {
            meta,
            delta: session.export(doc)?,
            event: MurmureEvent::Named {
                name: name.map(str::to_owned),
            },
        })
    }

    /// Every operation of one held document, as one Loro update.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for an identifier the channel does not hold,
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn export_all(&self, document_id: DocumentId) -> Result<Vec<u8>, MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        Ok(doc.export(ExportMode::all_updates())?)
    }

    /// The channel this state holds.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// The identifier of the `meta` document.
    pub fn meta_id(&self) -> MetaId {
        MetaId::from_document(self.meta.id)
    }

    /// The identifier of the segment new messages go to.
    pub fn active_id(&self) -> SegmentId {
        SegmentId::from_document(self.active.id)
    }

    /// The identifier of the segment being closed, if any.
    pub fn closing_id(&self) -> Option<SegmentId> {
        self.closing
            .as_ref()
            .map(|held| SegmentId::from_document(held.id))
    }

    /// Whether the channel holds the document of that identifier in memory,
    /// or is loading it on the view's demand.
    pub fn holds(&self, document_id: DocumentId) -> bool {
        self.doc(document_id).is_some() || self.is_pending(document_id)
    }

    /// Every document the channel holds or is loading, so a close can end
    /// the session of each.
    pub fn document_ids(&self) -> Vec<DocumentId> {
        [Some(&self.meta), Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .map(|held| held.id)
            .chain(self.loaded.keys().map(SegmentId::document_id))
            .chain(self.pending.keys().map(SegmentId::document_id))
            .collect()
    }

    /// The document of that identifier, among every one the channel holds.
    pub(crate) fn doc(&self, document_id: DocumentId) -> Option<&LoroDoc> {
        self.live()
            .find(|held| held.id == document_id)
            .map(|held| &held.doc)
            .or_else(|| self.loaded.get(&SegmentId::from_document(document_id)))
    }

    /// The full state of one held document, for persistence.
    ///
    /// A segment carries the numbers this member knows of its messages,
    /// framed before the Loro bytes. A member opening from it counts its
    /// unread messages (`MURMURE-113`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for an identifier the channel does not hold,
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn snapshot(&self, document_id: DocumentId) -> Result<Vec<u8>, MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let loro = doc.export(ExportMode::Snapshot)?;
        Ok(match self.sequences.get(&document_id) {
            Some(sequence) => sequence.frame(loro),
            None => loro,
        })
    }

    /// Where `message` of `segment` stands in the relay's order, `None` for
    /// a segment the channel does not hold.
    pub fn position(&self, segment: SegmentId, message: MessageId) -> Option<Position> {
        let document_id = segment.document_id();
        self.doc(document_id)?;
        Some(
            self.sequences
                .get(&document_id)
                .map_or(Position::Unknown, |s| s.position(message)),
        )
    }

    /// The highest number the relay delivered for `document_id`, `Seq::ZERO`
    /// for none.
    pub fn delivered(&self, document_id: DocumentId) -> Seq {
        self.sequences
            .get(&document_id)
            .map_or(Seq::ZERO, Sequence::delivered)
    }

    /// The messages of `segment` past the cursor `read`, written by someone
    /// other than `me` (`MURMURE-113`, `MURMURE-120`).
    ///
    /// A message known only to lie in a snapshot counts when the snapshot
    /// reaches past the cursor. One not numbered yet counts too: the count
    /// may run over, never under. A reaction or an edit creates no message,
    /// so it never counts.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a segment the channel does not hold.
    pub fn unread(
        &self,
        segment: SegmentId,
        read: Seq,
        me: UserId,
    ) -> Result<UnreadCount, MurmureError> {
        let document_id = segment.document_id();
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let sequence = self.sequences.get(&document_id);
        let mut count = UnreadCount::ZERO;
        doc.get_list(MESSAGES).for_each(|item| {
            let Some((id, author)) = identity_of(item) else {
                return;
            };
            if author == me {
                return;
            }
            let past = match sequence.map_or(Position::Unknown, |s| s.position(id)) {
                Position::Exact(seq) | Position::AtMost(seq) => seq > read,
                Position::Unknown => true,
            };
            if past {
                count = count.plus(UnreadCount(1));
            }
        });
        Ok(count)
    }

    /// Every segment the channel holds in memory: the live ones, then the
    /// older ones loaded on demand.
    pub fn held_segments(&self) -> Vec<SegmentId> {
        [Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .map(|held| SegmentId::from_document(held.id))
            .chain(self.loaded.keys().copied())
            .collect()
    }

    /// The segments `meta` lists after `segment`, in list order: the ones
    /// whose messages all come later.
    pub fn segments_after(&self, segment: SegmentId) -> Vec<SegmentId> {
        segment_entries(&self.meta.doc)
            .into_iter()
            .map(|(id, _)| id)
            .skip_while(|id| *id != segment)
            .skip(1)
            .collect()
    }

    /// The messages of one held segment, in list order.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for an identifier the channel does not hold.
    pub fn messages(&self, segment: SegmentId) -> Result<Vec<MessageView>, MurmureError> {
        let document_id = segment.document_id();
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        Ok(read_messages(doc))
    }

    /// Append a message to the active segment (`MURMURE-080`).
    ///
    /// The identifier is minted here, by the sending peer (`MURMURE-002`). The
    /// send writes one new message map and nothing else (`MURMURE-083`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    /// [`MurmureError::NoSession`] before the PeerID of the active segment
    /// landed.
    pub fn send(
        &mut self,
        draft: &MessageDraft,
        author: UserId,
        now: Timestamp,
    ) -> Result<Sent, MurmureError> {
        let segment = self.active_id();
        let (doc, session) = self.active.session()?;
        let id = MessageId::new();
        let message = doc.get_list(MESSAGES).push_container(LoroMap::new())?;
        message.insert(MESSAGE_ID, id.to_string())?;
        message.insert(MESSAGE_AUTHOR, author.to_string())?;
        message.insert(MESSAGE_TS, schema::ts_value(now.0))?;
        // The body is a char CRDT, so two peers typing in it keep both texts
        // (`MURMURE-004`, `MURMURE-005`).
        let content = message.insert_container(MESSAGE_CONTENT, LoroText::new())?;
        content.insert(0, &draft.text)?;
        let delta = session.export(doc)?;
        Ok(Sent {
            segment,
            delta,
            summary: summary_of(draft),
            event: MurmureEvent::Messages {
                segment,
                messages: vec![MessageView {
                    id,
                    author,
                    ts: now,
                    text: draft.text.clone(),
                }],
            },
        })
    }

    /// Import a remote delta into the one document it names (`MURMURE-024`).
    ///
    /// A segment answers with its whole message list: the view keys messages
    /// by identifier, so an unchanged one is replaced by itself. `meta` answers
    /// with the name it holds (`MURMURE-054`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a document the channel does not hold, and
    /// [`MurmureError::Crdt`] for bytes Loro refuses.
    pub fn apply_remote(
        &mut self,
        document_id: DocumentId,
        payload: &[u8],
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        self.apply_remote_at(document_id, None, payload)
    }

    /// [`ChannelState::apply_remote`], with where the relay placed the
    /// payload. A segment's messages learn their number from it
    /// (`features/ambre-spec-murmure-unread.md section 3`).
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::apply_remote`].
    pub fn apply_remote_at(
        &mut self,
        document_id: DocumentId,
        order: Option<Order>,
        payload: &[u8],
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        // An older segment still arriving keeps its bytes until it is ready.
        // Its numbers are noted now and matched once it is admitted.
        if let Some(payloads) = self.pending.get_mut(&SegmentId::from_document(document_id)) {
            payloads.push(payload.to_vec());
            if let Some(order) = order {
                self.sequences
                    .entry(document_id)
                    .or_default()
                    .note(order, payload);
            }
            return Ok(Vec::new());
        }
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        doc.import(sequence::loro_part(payload))?;
        if document_id == self.meta.id {
            // The name is the one field of `meta` a view shows. It is read
            // whole, so a delta that touched something else repeats it.
            return Ok(vec![MurmureEvent::Named { name: self.name() }]);
        }
        // Every document but `meta` is a segment: a channel holds no thread
        // document yet.
        let messages = read_messages(doc);
        // Only an update needs the messages matched; a snapshot's numbers
        // ride in its table.
        let created = matches!(order, Some(Order::Update(_))).then(|| sequence::created(doc));
        if let Some(order) = order {
            let sequence = self.sequences.entry(document_id).or_default();
            sequence.note(order, payload);
            if let Some(created) = created {
                sequence.resolve_created(&created);
            }
        }
        Ok(vec![MurmureEvent::Messages {
            segment: SegmentId::from_document(document_id),
            messages,
        }])
    }

    /// The final segment listed just before `segment`, the one a scroll past
    /// the oldest loaded message asks for.
    pub fn older_than(&self, segment: SegmentId) -> Option<SegmentId> {
        let entries = segment_entries(&self.meta.doc);
        let position = entries.iter().position(|(id, _)| *id == segment)?;
        entries
            .iter()
            .take(position)
            .rev()
            .find(|(_, state)| *state == SegmentState::Final)
            .map(|(id, _)| *id)
    }

    /// Every final segment `meta` lists, in list order, loaded or not.
    ///
    /// Reaching the latest message reads them all (`MURMURE-115`). Their
    /// content no longer moves, so the relay may advance their cursors to
    /// their last number with no message skipped.
    pub fn final_segments(&self) -> Vec<SegmentId> {
        segment_entries(&self.meta.doc)
            .into_iter()
            .filter(|(_, state)| *state == SegmentState::Final)
            .map(|(id, _)| id)
            .collect()
    }

    /// Ask for the final segment listed before `segment`, the one a scroll
    /// past the oldest loaded message needs (`MURMURE-023`).
    ///
    /// Returns the document to open, or `None` when there is no older segment,
    /// or when it is already loaded or on its way. Its bytes then go to
    /// [`ChannelState::apply_remote`], and [`ChannelState::older_ready`] reads
    /// them once they are all in.
    pub fn request_older(&mut self, segment: SegmentId) -> Option<SegmentId> {
        let older = self.older_than(segment)?;
        if self.loaded.contains_key(&older) || self.pending.contains_key(&older) {
            return None;
        }
        self.pending.insert(older, Vec::new());
        Some(older)
    }

    /// Whether `document_id` is an older segment still arriving.
    ///
    /// Takes the routing address, which names no kind: a document this
    /// channel did not ask for is simply not pending.
    pub fn is_pending(&self, document_id: DocumentId) -> bool {
        self.pending
            .contains_key(&SegmentId::from_document(document_id))
    }

    /// Give up an older segment that could not open, so the view can ask
    /// again. Returns `false` when none of that identifier was asked for.
    pub fn cancel_older(&mut self, document_id: DocumentId) -> bool {
        self.sequences.remove(&document_id);
        self.pending
            .remove(&SegmentId::from_document(document_id))
            .is_some()
    }

    /// The older segment `document_id` has all its bytes: check it and hold
    /// it (`MURMURE-023`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when no older segment of that identifier was
    /// asked for, and every refusal of [`ChannelState::load_older`]. A refused
    /// segment is dropped, so the view can ask again.
    pub fn older_ready(&mut self, document_id: DocumentId) -> Result<MurmureEvent, MurmureError> {
        let segment = SegmentId::from_document(document_id);
        let payloads = self
            .pending
            .remove(&segment)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let loaded = self.load_older_from(segment, &borrowed(&payloads));
        if loaded.is_err() {
            self.sequences.remove(&document_id);
        }
        loaded
    }

    /// Load an older segment the view asked for (`MURMURE-023`).
    ///
    /// # Errors
    /// [`MurmureError::NotOlder`] when `meta` lists no final segment of that
    /// identifier, and every refusal of the header check.
    pub fn load_older(
        &mut self,
        segment: SegmentId,
        payload: &[u8],
    ) -> Result<MurmureEvent, MurmureError> {
        self.load_older_from(segment, &[payload])
    }

    fn load_older_from(
        &mut self,
        segment: SegmentId,
        payloads: &[&[u8]],
    ) -> Result<MurmureEvent, MurmureError> {
        let document_id = segment.document_id();
        let listed = segment_entries(&self.meta.doc)
            .into_iter()
            .any(|(id, state)| id == segment && state == SegmentState::Final);
        if !listed {
            return Err(MurmureError::NotOlder(document_id));
        }
        let doc = schema::admit_all(
            payloads,
            DocumentKind::Segment,
            self.channel_id,
            document_id,
        )?;
        let messages = read_messages(&doc);
        if let Some(sequence) = self.sequences.get_mut(&document_id) {
            sequence.resolve(&doc);
        }
        self.loaded.insert(segment, doc);
        Ok(MurmureEvent::Messages { segment, messages })
    }
}

/// The payloads of one document, borrowed for [`schema::admit_all`].
fn borrowed(payloads: &[Vec<u8>]) -> Vec<&[u8]> {
    payloads.iter().map(Vec::as_slice).collect()
}

/// The live segments `meta` lists: the active one, and the one being closed.
pub(crate) struct LiveSegments {
    pub(crate) active: SegmentId,
    pub(crate) closing: Option<SegmentId>,
}

/// The live segments a `meta` document lists (`MURMURE-022`). A final
/// segment is never among them (`MURMURE-028`).
///
/// # Errors
/// [`MurmureError::NoActiveSegment`] when `meta` lists no active segment.
pub(crate) fn live_segments(
    meta: &LoroDoc,
    channel_id: ChannelId,
) -> Result<LiveSegments, MurmureError> {
    let mut active = None;
    let mut closing = None;
    for (id, state) in segment_entries(meta) {
        let slot = match state {
            SegmentState::Active => &mut active,
            SegmentState::Closing => &mut closing,
            SegmentState::Final => continue,
        };
        if slot.is_some() {
            // Two live segments of one state is the fork `MURMURE-010`
            // forbids. The first in list order wins, the same on every
            // peer, and the warn keeps the fork visible.
            tracing::warn!("[Murmure] channel {channel_id} lists a second {state:?} segment");
            continue;
        }
        *slot = Some(id);
    }
    Ok(LiveSegments {
        active: active.ok_or(MurmureError::NoActiveSegment(channel_id))?,
        closing,
    })
}

/// Append one entry to `meta.segments`.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn push_segment_entry(
    meta: &LoroDoc,
    id: SegmentId,
    from: Timestamp,
    state: SegmentState,
) -> Result<(), MurmureError> {
    let entry = meta.get_list(SEGMENTS).push_container(LoroMap::new())?;
    entry.insert(SEGMENT_DOCUMENT, id.to_string())?;
    entry.insert(SEGMENT_FROM, schema::ts_value(from.0))?;
    entry.insert(SEGMENT_STATE, state.as_str())?;
    Ok(())
}

/// The segments `meta` lists, in list order. A malformed entry is skipped
/// with a warn: one bad entry must not close the channel.
fn segment_entries(meta: &LoroDoc) -> Vec<(SegmentId, SegmentState)> {
    let list: LoroList = meta.get_list(SEGMENTS);
    let mut entries = Vec::with_capacity(list.len());
    list.for_each(|item| {
        let ValueOrContainer::Container(Container::Map(entry)) = item else {
            tracing::warn!("[Murmure] a segments entry is not a map; skipped");
            return;
        };
        let id =
            schema::string(&entry, SEGMENT_DOCUMENT).and_then(|s| SegmentId::parse_str(&s).ok());
        let state = schema::string(&entry, SEGMENT_STATE).and_then(|s| SegmentState::parse(&s));
        match (id, state) {
            (Some(id), Some(state)) => entries.push((id, state)),
            _ => tracing::warn!("[Murmure] a segments entry is malformed; skipped"),
        }
    });
    entries
}

/// The messages of a segment, in list order. A malformed message is skipped
/// with a warn rather than failing the whole segment.
fn read_messages(doc: &LoroDoc) -> Vec<MessageView> {
    let list = doc.get_list(MESSAGES);
    let mut messages = Vec::with_capacity(list.len());
    list.for_each(|item| match read_message(item) {
        Some(view) => messages.push(view),
        None => tracing::warn!("[Murmure] a message is malformed; skipped"),
    });
    messages
}

/// The identifier and the author of one message, with no body read: what a
/// count needs. `None` when either is absent or unreadable.
fn identity_of(item: ValueOrContainer) -> Option<(MessageId, UserId)> {
    let ValueOrContainer::Container(Container::Map(map)) = item else {
        return None;
    };
    let id = MessageId::parse_str(&schema::string(&map, MESSAGE_ID)?).ok()?;
    let author = UserId::parse_str(&schema::string(&map, MESSAGE_AUTHOR)?).ok()?;
    Some((id, author))
}

/// One message, or `None` when a field is absent or unreadable.
fn read_message(item: ValueOrContainer) -> Option<MessageView> {
    let ValueOrContainer::Container(Container::Map(map)) = item else {
        return None;
    };
    let id = MessageId::parse_str(&schema::string(&map, MESSAGE_ID)?).ok()?;
    let author = UserId::parse_str(&schema::string(&map, MESSAGE_AUTHOR)?).ok()?;
    let ts = Timestamp(schema::unsigned(&map, MESSAGE_TS)?);
    let Some(ValueOrContainer::Container(Container::Text(content))) = map.get(MESSAGE_CONTENT)
    else {
        return None;
    };
    Some(MessageView {
        id,
        author,
        ts,
        text: content.to_string(),
    })
}

/// The activity summary of a new message (`MURMURE-081`): kind content, each
/// named user once in typing order, and whether the channel itself is named.
fn summary_of(draft: &MessageDraft) -> ActivitySummary {
    let mut mentions: Vec<UserId> = Vec::new();
    let mut mentions_all = false;
    for target in &draft.mentions {
        match target {
            MentionTarget::User(user) if !mentions.contains(user) => mentions.push(*user),
            MentionTarget::User(_) => {}
            MentionTarget::Channel => mentions_all = true,
        }
    }
    ActivitySummary {
        kind: ActivityKind::Content,
        mentions,
        mentions_all,
        anchor: None,
    }
}
