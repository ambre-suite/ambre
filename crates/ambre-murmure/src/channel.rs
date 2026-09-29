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
use ambre_core::{ChannelId, DocumentId, MessageId, UserId};
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
    pub segment: DocumentId,
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
    loaded: HashMap<DocumentId, LoroDoc>,
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
        push_segment_entry(&meta.doc, active.id, now, SegmentState::Active)?;
        meta.doc.commit();
        schema::write_header(&active.doc, DocumentKind::Segment, channel_id, now.0)?;
        active.doc.commit();
        Ok(Self {
            channel_id,
            meta,
            active,
            closing: None,
            loaded: HashMap::new(),
        })
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
        meta_id: DocumentId,
        meta_bytes: &[u8],
        segments: &HashMap<DocumentId, Vec<u8>>,
    ) -> Result<Self, MurmureError> {
        let meta = Held {
            id: meta_id,
            doc: schema::admit(meta_bytes, DocumentKind::Meta, channel_id, meta_id)?,
        };
        let mut active = None;
        let mut closing = None;
        for (id, state) in segment_entries(&meta.doc) {
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
            let bytes = segments.get(&id).ok_or(MurmureError::SegmentMissing(id))?;
            let doc = schema::admit(bytes, DocumentKind::Segment, channel_id, id)?;
            *slot = Some(Held { id, doc });
        }
        Ok(Self {
            channel_id,
            meta,
            active: active.ok_or(MurmureError::NoActiveSegment(channel_id))?,
            closing,
            loaded: HashMap::new(),
        })
    }

    /// The channel this state holds.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// The identifier of the `meta` document.
    pub fn meta_id(&self) -> DocumentId {
        self.meta.id
    }

    /// The identifier of the segment new messages go to.
    pub fn active_id(&self) -> DocumentId {
        self.active.id
    }

    /// The identifier of the segment being closed, if any.
    pub fn closing_id(&self) -> Option<DocumentId> {
        self.closing.as_ref().map(|held| held.id)
    }

    /// Whether the channel holds the document of that identifier in memory.
    pub fn holds(&self, document_id: DocumentId) -> bool {
        self.doc(document_id).is_some()
    }

    /// The document of that identifier, among every one the channel holds.
    pub(crate) fn doc(&self, document_id: DocumentId) -> Option<&LoroDoc> {
        [Some(&self.meta), Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .find(|held| held.id == document_id)
            .map(|held| &held.doc)
            .or_else(|| self.loaded.get(&document_id))
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
    pub fn messages(&self, segment: DocumentId) -> Result<Vec<MessageView>, MurmureError> {
        let doc = self.doc(segment).ok_or(MurmureError::NotHeld(segment))?;
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
        Ok(Sent {
            segment: self.active.id,
            delta,
            summary: summary_of(draft),
            event: MurmureEvent::Messages {
                segment: self.active.id,
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
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        doc.import(payload)?;
        if document_id == self.meta.id {
            return Ok(Vec::new());
        }
        Ok(vec![MurmureEvent::Messages {
            segment: document_id,
            messages: read_messages(doc),
        }])
    }

    /// The final segment listed just before `segment`, the one a scroll past
    /// the oldest loaded message asks for.
    pub fn older_than(&self, segment: DocumentId) -> Option<DocumentId> {
        let entries = segment_entries(&self.meta.doc);
        let position = entries.iter().position(|(id, _)| *id == segment)?;
        entries
            .iter()
            .take(position)
            .rev()
            .find(|(_, state)| *state == SegmentState::Final)
            .map(|(id, _)| *id)
    }

    /// Load an older segment the view asked for (`MURMURE-023`).
    ///
    /// # Errors
    /// [`MurmureError::NotOlder`] when `meta` lists no final segment of that
    /// identifier, and every refusal of the header check.
    pub fn load_older(
        &mut self,
        document_id: DocumentId,
        payload: &[u8],
    ) -> Result<MurmureEvent, MurmureError> {
        let listed = segment_entries(&self.meta.doc)
            .into_iter()
            .any(|(id, state)| id == document_id && state == SegmentState::Final);
        if !listed {
            return Err(MurmureError::NotOlder(document_id));
        }
        let doc = schema::admit(payload, DocumentKind::Segment, self.channel_id, document_id)?;
        let messages = read_messages(&doc);
        self.loaded.insert(document_id, doc);
        Ok(MurmureEvent::Messages {
            segment: document_id,
            messages,
        })
    }
}

/// Append one entry to `meta.segments`.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
pub(crate) fn push_segment_entry(
    meta: &LoroDoc,
    id: DocumentId,
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
fn segment_entries(meta: &LoroDoc) -> Vec<(DocumentId, SegmentState)> {
    let list: LoroList = meta.get_list(SEGMENTS);
    let mut entries = Vec::with_capacity(list.len());
    list.for_each(|item| {
        let ValueOrContainer::Container(Container::Map(entry)) = item else {
            tracing::warn!("[Murmure] a segments entry is not a map; skipped");
            return;
        };
        let id =
            schema::string(&entry, SEGMENT_DOCUMENT).and_then(|s| DocumentId::parse_str(&s).ok());
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
