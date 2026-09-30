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
    Timestamp,
};
use ambre_core::{ChannelId, DocumentId, MessageId, MetaId, SegmentId, UserId};
use loro::{Container, ExportMode, LoroDoc, LoroList, LoroMap, LoroText, ValueOrContainer};

use crate::error::MurmureError;
use crate::schema::{
    self, CHANNEL, CHANNEL_NAME, CHANNEL_TYPE, ChannelType, DocumentKind, MESSAGE_AUTHOR,
    MESSAGE_CONTENT, MESSAGE_ID, MESSAGE_TS, MESSAGES, SEGMENT_DOCUMENT, SEGMENT_FROM,
    SEGMENT_STATE, SEGMENTS, SegmentState,
};

/// One document the channel holds, with the identifier the Sync worker routes
/// it by.
#[derive(Debug)]
struct Held {
    id: DocumentId,
    doc: LoroDoc,
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
}

impl ChannelState {
    /// Create a new channel: its `meta` document and its first active segment.
    ///
    /// `name` stays absent when `None`, never an empty string (`MURMURE-018`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    pub fn create(
        channel_id: ChannelId,
        channel_type: ChannelType,
        name: Option<&str>,
        now: Timestamp,
    ) -> Result<Self, MurmureError> {
        let meta = Held {
            id: DocumentId::new(),
            doc: LoroDoc::new(),
        };
        let active = Held {
            id: DocumentId::new(),
            doc: LoroDoc::new(),
        };
        schema::write_header(&meta.doc, DocumentKind::Meta, channel_id, now.0)?;
        let channel = meta.doc.get_map(CHANNEL);
        channel.insert(CHANNEL_TYPE, channel_type.as_str())?;
        if let Some(name) = name {
            channel.insert(CHANNEL_NAME, name)?;
        }
        push_segment_entry(
            &meta.doc,
            SegmentId::from_document(active.id),
            now,
            SegmentState::Active,
        )?;
        meta.doc.commit();
        schema::write_header(&active.doc, DocumentKind::Segment, channel_id, now.0)?;
        active.doc.commit();
        Ok(Self::assemble(channel_id, meta, active, None))
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
        let meta = Held {
            id: meta_id,
            doc: schema::admit(meta_bytes, DocumentKind::Meta, channel_id, meta_id)?,
        };
        let live = live_segments(&meta.doc, channel_id)?;
        let admit_live = |segment: SegmentId| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let bytes = segments
                .get(&segment)
                .ok_or(MurmureError::SegmentMissing(id))?;
            let doc = schema::admit(bytes, DocumentKind::Segment, channel_id, id)?;
            Ok(Held { id, doc })
        };
        let active = admit_live(live.active)?;
        let closing = live.closing.map(admit_live).transpose()?;
        Ok(Self::assemble(channel_id, meta, active, closing))
    }

    /// Open a channel from documents whose bytes arrived as several payloads
    /// each: a snapshot, then a replay (`MURMURE-022`).
    ///
    /// `segments` holds the live segments `meta` lists, and the same checks
    /// as [`ChannelState::open`] run on each.
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::open`].
    pub(crate) fn open_from_payloads(
        channel_id: ChannelId,
        meta: (DocumentId, LoroDoc),
        segments: &HashMap<DocumentId, Vec<Vec<u8>>>,
    ) -> Result<Self, MurmureError> {
        let (meta_id, meta_doc) = meta;
        let live = live_segments(&meta_doc, channel_id)?;
        let admit_live = |segment: SegmentId| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let payloads = segments.get(&id).ok_or(MurmureError::SegmentMissing(id))?;
            let doc =
                schema::admit_all(&borrowed(payloads), DocumentKind::Segment, channel_id, id)?;
            Ok(Held { id, doc })
        };
        let active = admit_live(live.active)?;
        let closing = live.closing.map(admit_live).transpose()?;
        let meta = Held {
            id: meta_id,
            doc: meta_doc,
        };
        Ok(Self::assemble(channel_id, meta, active, closing))
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

    /// Every operation of one held document, as one Loro update: the first
    /// payload the relay receives for a channel just created.
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
        [Some(&self.meta), Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .find(|held| held.id == document_id)
            .map(|held| &held.doc)
            .or_else(|| self.loaded.get(&SegmentId::from_document(document_id)))
    }

    /// The full state of one held document, for persistence.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for an identifier the channel does not hold,
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn snapshot(&self, document_id: DocumentId) -> Result<Vec<u8>, MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        Ok(doc.export(ExportMode::Snapshot)?)
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
    pub fn send(
        &mut self,
        draft: &MessageDraft,
        author: UserId,
        now: Timestamp,
    ) -> Result<Sent, MurmureError> {
        let doc = &self.active.doc;
        let before = doc.oplog_vv();
        let id = MessageId::new();
        let message = doc.get_list(MESSAGES).push_container(LoroMap::new())?;
        message.insert(MESSAGE_ID, id.to_string())?;
        message.insert(MESSAGE_AUTHOR, author.to_string())?;
        message.insert(MESSAGE_TS, schema::ts_value(now.0))?;
        // The body is a char CRDT, so two peers typing in it keep both texts
        // (`MURMURE-004`, `MURMURE-005`).
        let content = message.insert_container(MESSAGE_CONTENT, LoroText::new())?;
        content.insert(0, &draft.text)?;
        doc.commit();
        let delta = doc.export(ExportMode::updates(&before))?;
        let segment = self.active_id();
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
    /// with nothing until a feature reads it.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a document the channel does not hold, and
    /// [`MurmureError::Crdt`] for bytes Loro refuses.
    pub fn apply_remote(
        &mut self,
        document_id: DocumentId,
        payload: &[u8],
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        // An older segment still arriving keeps its bytes until it is ready.
        if let Some(payloads) = self.pending.get_mut(&SegmentId::from_document(document_id)) {
            payloads.push(payload.to_vec());
            return Ok(Vec::new());
        }
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        doc.import(payload)?;
        if document_id == self.meta.id {
            return Ok(Vec::new());
        }
        // Every document but `meta` is a segment: a channel holds no thread
        // document yet.
        Ok(vec![MurmureEvent::Messages {
            segment: SegmentId::from_document(document_id),
            messages: read_messages(doc),
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
        self.load_older_from(segment, &borrowed(&payloads))
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
