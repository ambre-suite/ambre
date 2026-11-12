//! The amendments of a locked segment
//! (`features/ambre-spec-murmure-messages.md section 3`).
//!
//! The relay refuses every delta of a locked segment (`SRV-115`). A late edit
//! or delete of one of its messages goes to the segment's one amendments
//! document instead, which the relay links to it (`SRV-201`). A member reads
//! each amendment over the message it names. The document holds entries only
//! for messages of its own segment, each written by their author
//! (`MURMURE-013`, `MURMURE-253`).

use crate::message::{Deletion, MessageDraft, MessageView, MurmureEvent, ThreadHead};
use ambre_bus::{Authored, Instant};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes, Relay, Seq};
use loro::{
    Container, LoroDoc, LoroList, LoroMap, LoroText, LoroValue, UpdateOptions, ValueOrContainer,
};

use super::{ChannelState, Created, Held, Sent};
use crate::logic::delta::{self, Arrival, Rules, Signer};
use crate::logic::error::MurmureError;
use crate::logic::schema::{
    self, AmendmentKey, DeletedBy, DocumentKind, Fields, HeaderKey, Mark, Root, SegmentKey,
};
use crate::logic::sequence::Sequence;
use crate::logic::session::Session;
use crate::logic::{mention, style};

/// An amendments document whose bytes are still arriving.
#[derive(Debug)]
pub(super) struct Pending {
    /// The locked segment it amends.
    pub(super) segment: Id<Segment>,
    /// Its payloads, in the order they came.
    pub(super) arrivals: Vec<Arrival>,
    /// The identity issued for it, adopted once it is admitted.
    session: Option<(ambre_core::PeerId, Payload<PeerEntryBytes>)>,
}

impl Pending {
    fn of(segment: Id<Segment>) -> Self {
        Self {
            segment,
            arrivals: Vec::new(),
            session: None,
        }
    }
}

#[cfg(test)]
impl ChannelState {
    /// Wait for the bytes of `amendments` of `segment`, though `meta` marks
    /// them folded: the schema belts read them as a second peer does.
    pub(crate) fn expect_amendments(&mut self, segment: Id<Segment>, amendments: Id<Document>) {
        self.amendments_pending
            .insert(amendments, Pending::of(segment));
    }
}

/// A change of one message of a locked segment.
#[derive(Debug, Clone, Copy)]
pub(super) enum Change<'a> {
    /// Its body becomes the draft.
    Edit(&'a MessageDraft),
    /// It reads deleted, by its author.
    Delete,
}

/// The document an edit or a delete of a message writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTarget {
    /// The live segment itself, or the amendments document the channel
    /// holds or is loading for its locked segment.
    Write(Id<Document>),
    /// The amendments document `meta` names for the locked segment, now
    /// asked for: the change waits until it is held.
    Open(Id<Document>),
    /// A new amendments document, held here with no session. The Sync worker
    /// creates it on the relay and links it to the segment (`SRV-201`).
    Create(Id<Document>),
}

/// What a link of an amendments document produced.
#[derive(Debug)]
pub struct Linked {
    /// The `meta` delta that names the document in the segment's entry,
    /// `None` when the entry named it already.
    pub meta: Option<Payload<DeltaBytes>>,
    /// Whether the document is to open: this client's own, for its session,
    /// or another member's, for its bytes.
    pub open: super::LinkedOpening,
    /// This client's own document, which lost the race to another member's
    /// (`SRV-202`). What waited on it waits on the winner.
    pub replaced: Option<Id<Document>>,
}

impl ChannelState {
    /// Whether a change of a message of `segment` goes to an amendment: the
    /// segment is final, or the relay locked it while it was closing. A
    /// segment `meta` names amendments for was locked by then.
    pub(super) fn amendable(&self, segment: Id<Segment>) -> bool {
        self.loaded.contains_key(&segment)
            || self.locked.contains(&segment)
            || named_amendments(&self.meta.doc, segment).is_some()
    }

    /// The amendments document the channel holds for `segment`.
    #[cfg(test)]
    pub(crate) fn amendments_id(&self, segment: Id<Segment>) -> Option<Id<Document>> {
        self.amendments.get(&segment).map(|held| held.id)
    }

    /// The locked segment the held amendments document `document_id` amends.
    pub(super) fn amended_by(&self, document_id: Id<Document>) -> Option<Id<Segment>> {
        self.amendments
            .iter()
            .find(|(_, held)| held.id == document_id)
            .map(|(segment, _)| *segment)
    }

    /// Where an edit or a delete of a message of `segment` writes
    /// (`MURMURE-073`, `MURMURE-088`).
    ///
    /// A live segment takes the change itself. A locked one takes it in its
    /// amendments document. That is the one held or loading, else the one
    /// `meta` names, else a new one created at `now`.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the header of a new
    /// document.
    pub fn change_target(
        &mut self,
        segment: Id<Segment>,
        now: Instant<Authored>,
    ) -> Result<ChangeTarget, MurmureError> {
        if !self.amendable(segment) {
            return Ok(ChangeTarget::Write(segment.document_id()));
        }
        if let Some(held) = self.amendments.get(&segment) {
            return Ok(ChangeTarget::Write(held.id));
        }
        if let Some(id) = self.loading_amendments(segment) {
            return Ok(ChangeTarget::Write(id));
        }
        if let Some(id) = named_amendments(&self.meta.doc, segment) {
            self.amendments_pending.insert(id, Pending::of(segment));
            return Ok(ChangeTarget::Open(id));
        }
        let id = Id::<Document>::mint();
        let created = Created::Amendments { segment, now };
        let doc = LoroDoc::new();
        created.write(&doc, self.channel_id)?;
        let held = Held {
            created: Some(created),
            ..Held::served(id, doc)
        };
        self.amendments.insert(segment, held);
        Ok(ChangeTarget::Create(id))
    }

    /// The amendments document still arriving for `segment`.
    fn loading_amendments(&self, segment: Id<Segment>) -> Option<Id<Document>> {
        self.amendments_pending
            .iter()
            .find(|(_, pending)| pending.segment == segment)
            .map(|(id, _)| *id)
    }

    /// The relay linked `amendments` to `segment`: `meta` names it in the
    /// segment's entry, so every member finds it (`SRV-201`).
    ///
    /// When it is not this client's own, the race went to another member
    /// (`SRV-202`). This client drops its own, and fetches the winner.
    ///
    /// # Errors
    /// [`MurmureError::NoSession`] before the PeerID of `meta` landed, and
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    pub fn amendments_linked(
        &mut self,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) -> Result<Linked, MurmureError> {
        let replaced = self
            .amendments
            .get(&segment)
            .map(|held| held.id)
            .filter(|own| *own != amendments);
        if replaced.is_some() {
            self.amendments.remove(&segment);
        }
        let own = self.amendments.contains_key(&segment);
        let open = own || !self.amendments_pending.contains_key(&amendments);
        if !own && open {
            self.amendments_pending
                .insert(amendments, Pending::of(segment));
        }
        let meta = self.name_amendments(segment, amendments)?;
        Ok(Linked {
            meta,
            open: super::LinkedOpening::from_bool(open),
            replaced,
        })
    }

    /// The link of this client's own `amendments` to `segment` failed. It is
    /// dropped, so the next change creates one again. Returns `false` when
    /// the channel holds no such document.
    pub fn amendments_failed(&mut self, segment: Id<Segment>, amendments: Id<Document>) -> bool {
        let own = self
            .amendments
            .get(&segment)
            .is_some_and(|held| held.id == amendments && held.session.is_none());
        if own {
            self.amendments.remove(&segment);
        }
        own
    }

    /// Write `amendments` into the entry of `segment` in `meta`, the first
    /// one in list order. `None` when it names it already, or lists no such
    /// segment.
    fn name_amendments(
        &mut self,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) -> Result<Option<Payload<DeltaBytes>>, MurmureError> {
        let Some(entry) = segment_entry(&self.meta.doc, segment) else {
            return Ok(None);
        };
        let id = amendments.to_string();
        if schema::string(&entry, SegmentKey::Amendments).as_deref() == Some(id.as_str()) {
            return Ok(None);
        }
        let (doc, session) = self.meta.session()?;
        entry.insert(SegmentKey::Amendments, id)?;
        Ok(Some(session.export(doc)?))
    }

    /// The amendments documents `meta` names for a segment the channel
    /// holds, and that it neither holds nor loads. Each is asked for now:
    /// open it, and [`ChannelState::older_ready`] admits it. A document two
    /// entries name is asked for the first one alone.
    pub fn amendments_to_open(&mut self) -> Vec<Id<Document>> {
        let mut asked = Vec::new();
        for (segment, id) in amendments_entries(&self.meta.doc) {
            if self.doc(segment.document_id()).is_none()
                || self.amendments.contains_key(&segment)
                || self.loading_amendments(segment).is_some()
                || self.is_amendments(id)
            {
                continue;
            }
            self.amendments_pending.insert(id, Pending::of(segment));
            asked.push(id);
        }
        asked
    }

    /// Keep the identity issued for an amendments document still arriving.
    /// Returns `false` when no such document is arriving.
    pub(super) fn note_amendments_session(
        &mut self,
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: Payload<PeerEntryBytes>,
    ) -> bool {
        let Some(pending) = self.amendments_pending.get_mut(&document_id) else {
            return false;
        };
        pending.session = Some((peer, entry));
        true
    }

    /// The amendments document `document_id` has all its bytes: check it
    /// against its locked segment, and hold it.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when the channel no longer holds the
    /// segment, [`MurmureError::ForeignSegment`] for a header that names
    /// another one, and every refusal of the header check.
    pub(super) fn amendments_ready(
        &mut self,
        document_id: Id<Document>,
        pending: Pending,
    ) -> Result<MurmureEvent, MurmureError> {
        let segment = pending.segment;
        let segment_doc = self
            .doc(segment.document_id())
            .ok_or(MurmureError::NotHeld(segment.document_id()))?;
        let rules = Rules::Amendments {
            segment: segment_doc,
        };
        let doc = schema::admit_with(
            DocumentKind::Amendments,
            self.channel_id,
            document_id,
            |doc| delta::import_arrivals(doc, pending.arrivals, document_id, rules),
        )?;
        let named = schema::string(&crate::logic::schema::header(&doc), HeaderKey::Segment)
            .and_then(|s| Id::<Segment>::parse_str(&s).ok());
        if named != Some(segment) {
            return Err(MurmureError::ForeignSegment(document_id));
        }
        let mut held = Held::served(document_id, doc);
        if let Some((peer, entry)) = pending.session {
            held.session = Some(Session::adopt(&held.doc, peer, entry)?);
        }
        self.amendments.insert(segment, held);
        let messages = self.read_segment(segment).unwrap_or_default();
        Ok(MurmureEvent::Messages { segment, messages })
    }

    /// Whether `document_id` is an amendments document, held or arriving.
    pub(super) fn is_amendments(&self, document_id: Id<Document>) -> bool {
        self.amendments_pending.contains_key(&document_id) || self.amended_by(document_id).is_some()
    }

    /// Import a signed amendments delta numbered `seq` once its check passed
    /// against the locked segment (step 6 of the authorship check). A
    /// document still arriving keeps it until it is admitted.
    ///
    /// A refused delta still counts as delivered, so a fold names how far
    /// the relay's copy reaches.
    ///
    /// # Errors
    /// [`MurmureError::Refused`] for a delta the check refuses, which
    /// changes nothing, and [`MurmureError::Crdt`] for bytes Loro refuses.
    pub(super) fn apply_amendment(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: &Payload<DeltaBytes>,
        signer: Signer,
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        let sequence = self.sequences.entry(document_id).or_default();
        sequence.passed(seq);
        let update = delta::read(payload)?;
        let (_, end) = delta::range(&update);
        if let Some(pending) = self.amendments_pending.get_mut(&document_id) {
            sequence.note_bare_range(seq, &end);
            pending.arrivals.push(Arrival::Signed { update, signer });
            return Ok(Vec::new());
        }
        let segment = self
            .amended_by(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let segment_doc = self
            .doc(segment.document_id())
            .ok_or(MurmureError::NotHeld(segment.document_id()))?;
        let doc = self
            .amendments
            .get(&segment)
            .map(|held| &held.doc)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let rules = Rules::Amendments {
            segment: segment_doc,
        };
        delta::verdict(doc, rules, &update, signer)?;
        doc.import_json_updates(update)?;
        self.sequences
            .entry(document_id)
            .or_default()
            .note_bare_range(seq, &end);
        self.answer(
            document_id,
            super::Matching::Skip,
            None::<fn(&mut Sequence)>,
        )
    }

    /// Write `change` of `message` as an amendment of the locked `segment`
    /// (`MURMURE-073`, `MURMURE-088`).
    ///
    /// The author is the one the segment holds, read through the amendments
    /// already there. `None` when an edit leaves the body as it reads.
    ///
    /// # Errors
    /// - [`MurmureError::NotHeld`]: the channel holds no such segment.
    /// - [`MurmureError::NoMessage`]: the segment holds no such message.
    /// - [`MurmureError::NotAuthor`]: another user wrote it.
    /// - [`MurmureError::Deleted`]: it reads deleted.
    /// - [`MurmureError::NoAmendments`]: the channel holds no amendments
    ///   document for the segment yet.
    /// - [`MurmureError::NoSession`]: its PeerID has not landed.
    /// - [`MurmureError::Crdt`]: Loro refused a write or the export.
    pub(super) fn amend(
        &mut self,
        segment: Id<Segment>,
        message: Id<Message>,
        user: Id<User>,
        change: Change<'_>,
    ) -> Result<Option<Sent>, MurmureError> {
        let view = self
            .read_segment(segment)
            .ok_or(MurmureError::NotHeld(segment.document_id()))?
            .into_iter()
            .find(|view| view.id == message)
            .ok_or(MurmureError::NoMessage(message))?;
        // The check refuses an amendment of another author at every
        // receiver, so a local one is refused before it is written
        // (`MURMURE-253`).
        if view.author != user {
            return Err(MurmureError::NotAuthor(message));
        }
        if view.deleted == Deletion::Deleted {
            return Err(MurmureError::Deleted(message));
        }
        if let Change::Edit(draft) = change
            && view.text == draft.text
            && view.mentions == draft.mentions
        {
            return Ok(None);
        }
        let channel = self.channel_id;
        let held = self
            .amendments
            .get_mut(&segment)
            .ok_or(MurmureError::NoAmendments(segment))?;
        let amendments = held.id;
        let (doc, session) = held.session()?;
        let root = crate::logic::schema::root_map(doc, Root::Amendments);
        let key = message.to_string();
        let entry = Fields::<AmendmentKey>::new(match root.get(&key) {
            Some(ValueOrContainer::Container(Container::Map(entry))) => entry,
            _ => root.insert_container(&key, LoroMap::new())?,
        });
        match change {
            Change::Edit(draft) => write_edit(doc, &entry, draft, channel)?,
            Change::Delete => {
                if let Some(content) = entry_content(&entry) {
                    let len = content.len_unicode();
                    if len > 0 {
                        content.delete(0, len)?;
                    }
                }
                entry.insert(AmendmentKey::Deleted, true)?;
                entry.insert(AmendmentKey::By, DeletedBy::Author.as_str())?;
            }
        }
        let delta = session.export_json(doc)?;
        let mut sent = self.changed(segment, delta);
        sent.document = amendments;
        Ok(Some(sent))
    }
}

/// Write `draft` as the edited body of one amendment entry. The body takes
/// the difference alone, and its marks are written again from the draft.
fn write_edit(
    doc: &LoroDoc,
    entry: &Fields<AmendmentKey>,
    draft: &MessageDraft,
    channel: Id<Channel>,
) -> Result<(), MurmureError> {
    // A mergeable child would escape the authorship check, which owns a
    // container by the operation that created it.
    let content = match entry_content(entry) {
        Some(content) => content,
        None => entry.insert_container(AmendmentKey::Content, LoroText::new())?,
    };
    let len = content.len_unicode();
    if len > 0 {
        content.unmark(0..len, Mark::Mention.as_str())?;
    }
    style::clear(&content)?;
    content
        .update(&draft.text, UpdateOptions::default())
        .map_err(|error| MurmureError::Crdt(error.to_string()))?;
    mention::write(doc, &content, &draft.mentions, channel)?;
    style::write(doc, &content, &draft.styles)
}

/// The edited body of one amendment entry, `None` when it has none.
pub(super) fn entry_content(entry: &Fields<AmendmentKey>) -> Option<LoroText> {
    match entry.get(AmendmentKey::Content) {
        Some(ValueOrContainer::Container(Container::Text(content))) => Some(content),
        _ => None,
    }
}

/// Read each amendment of `amendments` over the message it names. A delete
/// empties the body and marks the message deleted. An edit replaces the body
/// and marks it edited. A message deleted in its segment stays deleted.
pub(super) fn overlay(amendments: &LoroDoc, messages: &mut [MessageView], channel: Id<Channel>) {
    overlay_named_threads(amendments, messages);
    let root = crate::logic::schema::root_map(amendments, Root::Amendments);
    if root.is_empty() {
        return;
    }
    for view in messages
        .iter_mut()
        .filter(|view| view.deleted == Deletion::Live)
    {
        let Some(entry) = root
            .get(&view.id.to_string())
            .and_then(crate::logic::schema::fields_of)
        else {
            continue;
        };
        if schema::flag(&entry, AmendmentKey::Deleted) {
            view.text.clear();
            view.mentions.clear();
            view.styles.clear();
            view.attachments.clear();
            view.deleted = crate::message::Deletion::Deleted;
        } else if let Some(content) = entry_content(&entry) {
            view.text = content.to_string();
            view.mentions = mention::read(&content, channel);
            view.styles = style::read(&content);
            view.edited = crate::message::Revision::Edited;
        }
    }
}

/// Read each thread `amendments` records over the root that names none, a
/// deleted one included (`MURMURE-484`). Its count waits for the thread.
fn overlay_named_threads(amendments: &LoroDoc, messages: &mut [MessageView]) {
    let threads = crate::logic::schema::root_map(amendments, Root::Threads);
    if threads.is_empty() {
        return;
    }
    for view in messages.iter_mut().filter(|view| view.thread.is_none()) {
        view.thread = recorded_thread(amendments, view.id).map(|document| ThreadHead {
            document,
            replies: ambre_core::Count::<crate::message::Replies>::ZERO,
            last_replier: None,
        });
    }
}

/// The thread `amendments` records for `root`, `None` when it records none.
pub(super) fn recorded_thread(amendments: &LoroDoc, root: Id<Message>) -> Option<Id<Document>> {
    let threads = crate::logic::schema::root_map(amendments, Root::Threads);
    Id::<Document>::parse_str(&schema::string_value(threads.get(&root.to_string()))?).ok()
}

/// Every root `amendments` records a thread for, with that thread.
pub(super) fn recorded_threads(amendments: &LoroDoc) -> Vec<(Id<Message>, Id<Document>)> {
    let mut recorded = Vec::new();
    amendments
        .get_map(Root::Threads.as_str())
        .for_each(|key, value| {
            let ValueOrContainer::Value(LoroValue::String(value)) = value else {
                return;
            };
            if let (Ok(root), Ok(thread)) = (
                Id::<Message>::parse_str(key),
                Id::<Document>::parse_str(&value),
            ) {
                recorded.push((root, thread));
            }
        });
    recorded
}

/// The first entry of `meta.segments` that lists `segment`.
pub(super) fn segment_entry(meta: &LoroDoc, segment: Id<Segment>) -> Option<Fields<SegmentKey>> {
    let wanted = segment.to_string();
    let list: LoroList = crate::logic::schema::root_list(meta, Root::Segments);
    (0..list.len()).find_map(|index| {
        let entry = list.get(index).and_then(schema::fields_of::<SegmentKey>)?;
        (schema::string(&entry, SegmentKey::Document).as_deref() == Some(wanted.as_str()))
            .then_some(entry)
    })
}

/// The amendments document `meta` names for `segment`, `None` when it names
/// none, or only the one a fold erased.
pub(super) fn named_amendments(meta: &LoroDoc, segment: Id<Segment>) -> Option<Id<Document>> {
    live_amendments(&segment_entry(meta, segment)?)
}

/// The amendments document a segment entry names, unless a fold erased it.
fn live_amendments(entry: &Fields<SegmentKey>) -> Option<Id<Document>> {
    let named = schema::string(entry, SegmentKey::Amendments)?;
    if schema::string(entry, SegmentKey::Folded).as_deref() == Some(named.as_str()) {
        return None;
    }
    Id::<Document>::parse_str(&named).ok()
}

/// The amendments document `meta` marks folded for `segment`.
pub(crate) fn folded_amendments(meta: &LoroDoc, segment: Id<Segment>) -> Option<Id<Document>> {
    let entry = segment_entry(meta, segment)?;
    Id::<Document>::parse_str(&schema::string(&entry, SegmentKey::Folded)?).ok()
}

/// Every segment `meta` names an amendments document for, with that
/// document, in list order.
fn amendments_entries(meta: &LoroDoc) -> Vec<(Id<Segment>, Id<Document>)> {
    let list: LoroList = crate::logic::schema::root_list(meta, Root::Segments);
    let mut entries: Vec<(Id<Segment>, Id<Document>)> = Vec::new();
    list.for_each(|item| {
        let Some(entry) = crate::logic::schema::fields_of(item) else {
            return;
        };
        let segment = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| Id::<Segment>::parse_str(&s).ok());
        let amendments = live_amendments(&entry);
        if let (Some(segment), Some(amendments)) = (segment, amendments)
            && !entries.iter().any(|(listed, _)| *listed == segment)
        {
            entries.push((segment, amendments));
        }
    });
    entries
}
