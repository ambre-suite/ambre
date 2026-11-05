//! The state of an open channel on the Logic thread
//! (`features/ambre-spec-murmure-engine.md section 3`).
//!
//! A channel is a set of Loro documents, never one (`MURMURE-021`). It holds
//! its `meta` document, its active segment and the segment being closed. It
//! also holds the older segments the view asked for. Each document keeps its own identifier, so a
//! remote delta reaches the one it names and no other (`MURMURE-024`).

use ambre_core::Count;
use ambre_core::Validated;
use std::collections::{HashMap, HashSet};

use crate::message::{
    Attachment, ChannelMatches, Deletion, Mention, MentionTarget, MessageAt, MessageDraft,
    MessageView, MurmureEvent, Notice, Revision, SearchMatch, StyleMark, ThreadHead, Unreadable,
};
use ambre_bus::{
    ActivityKind, ActivitySummary, Authored, ContentHash, Emoji, Instant, Mentions, Messages,
    Unread,
};
use ambre_core::id::{Channel, Document, Id, Message, Meta, Segment, User};
use ambre_core::{
    DeltaBytes, LoroPayload, LoroPayloadRef, Payload, PeerEntryBytes, Relay, Seq, SnapshotBytes,
};
use loro::{
    Container, ExportMode, LoroDoc, LoroList, LoroMap, LoroText, UpdateOptions, ValueOrContainer,
};

use crate::delta::{self, Arrival, Refusal, Rules, Signer};
use crate::error::MurmureError;
use crate::schema::{
    self, ChannelKey, ChannelType, DocumentKind, Fields, Mark, MessageKey, NoticeKind, Root,
    SegmentKey, SegmentState,
};
use crate::search::Query;
use crate::sequence::{self, Order, Position, Sequence};
use crate::session::Session;
use crate::{attachment, mention, reaction, style};

mod amending;
mod closing;
mod confirming;
mod finalizing;
mod folding;
mod relocating;
mod threading;

pub use amending::{ChangeTarget, Linked};
pub use closing::{Closed, SEGMENT_MESSAGES, SEGMENT_SNAPSHOT_SIZE, SUCCESSOR_RETRY, Successor};
pub use finalizing::{FINALIZE_RETRY, FinalSnapshot};
pub use folding::{FOLD_DELAY, FoldSnapshot, Settled};
pub use relocating::{Relocated, Replay};
pub use threading::{Replied, SeekStep, ThreadLinked, ThreadTarget};

#[cfg(test)]
pub(crate) use {amending::folded_amendments, folding::header_fold};

ambre_core::define_flag! {
    /// Whether a linked document is to open: this client's own, for its
    /// session, or another member's, for its bytes.
    pub enum LinkedOpening {
        /// The document is not opened: another link of it already asked.
        Skip,
        /// The document is opened.
        Open,
    }

    /// Whether the active segment of a channel asked to close.
    enum CloseAsk {
        /// It did not, or the relay refused: the next send may ask.
        Idle,
        /// It asked, and waits for the answer.
        Asked,
    }

    /// Whether a key rotation owes the close of the active segment.
    enum Rotation {
        /// The active segment was opened under the current key.
        Settled,
        /// A rotation retired its key: it closes at the next ask, whatever
        /// it holds (`MURMURE-060`).
        OwesClose,
    }
}

/// One document the channel holds, with the identifier the Sync worker routes
/// it by.
#[derive(Debug)]
struct Held {
    id: Id<Document>,
    doc: LoroDoc,
    /// The PeerID this session writes the document under. `None` until the
    /// Sync worker issues it, and nothing is written before (`AUTHOR-029`).
    session: Option<Session>,
    /// What a document created here holds, written again under its PeerID
    /// once the session lands. `None` for a document the relay served.
    created: Option<Created>,
    /// The message count at which an active segment next measures its
    /// snapshot against the byte bound. Zero measures at the first ask.
    probe_at: Count<Messages>,
}

impl Held {
    /// A document the relay served, with no session yet.
    fn served(id: Id<Document>, doc: LoroDoc) -> Self {
        Self {
            id,
            doc,
            session: None,
            created: None,
            probe_at: Count::<Messages>::ZERO,
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
        name: Option<ambre_core::Validated<ambre_core::Name>>,
        active: Id<Segment>,
        now: Instant<Authored>,
    },
    /// The first active segment, which holds no message yet.
    Segment { now: Instant<Authored> },
    /// The amendments document of a locked segment, which holds no
    /// amendment yet.
    Amendments {
        segment: Id<Segment>,
        now: Instant<Authored>,
    },
    /// The thread document of a root, which holds no reply yet.
    Thread {
        segment: Id<Segment>,
        root: Id<Message>,
        now: Instant<Authored>,
    },
}

impl Created {
    /// Write this content into `doc`, and commit it.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write.
    fn write(&self, doc: &LoroDoc, channel_id: Id<Channel>) -> Result<(), MurmureError> {
        match self {
            Self::Meta {
                channel_type,
                name,
                active,
                now,
            } => {
                schema::write_header(
                    doc,
                    DocumentKind::Meta,
                    channel_id,
                    Instant::<Authored>::new(now.get()),
                )?;
                let channel = crate::schema::channel(doc);
                channel.insert(ChannelKey::Type, channel_type.as_str())?;
                if let Some(name) = name {
                    channel.insert(ChannelKey::Name, name.as_str())?;
                }
                push_segment_entry(doc, *active, *now, SegmentState::Active)?;
            }
            Self::Segment { now } => {
                schema::write_header(
                    doc,
                    DocumentKind::Segment,
                    channel_id,
                    Instant::<Authored>::new(now.get()),
                )?;
            }
            Self::Amendments { segment, now } => {
                schema::write_header(
                    doc,
                    DocumentKind::Amendments,
                    channel_id,
                    Instant::<Authored>::new(now.get()),
                )?;
                crate::schema::header(doc)
                    .insert(schema::HeaderKey::Segment, segment.to_string())?;
            }
            Self::Thread { segment, root, now } => {
                schema::write_header(
                    doc,
                    DocumentKind::Thread,
                    channel_id,
                    Instant::<Authored>::new(now.get()),
                )?;
                let header = crate::schema::header(doc);
                header.insert(schema::HeaderKey::Segment, segment.to_string())?;
                header.insert(schema::HeaderKey::Root, root.to_string())?;
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
    /// The segment the message went to, or the locked segment it amends.
    pub segment: Id<Segment>,
    /// The document the delta goes to: the segment, or its amendments
    /// document.
    pub document: Id<Document>,
    /// Every operation of the send, as a Loro JSON update (`MURMURE-223`).
    pub delta: Payload<DeltaBytes>,
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
    pub meta: Id<Meta>,
    /// Every operation of the change, as a Loro update. Empty of operations
    /// when the name was already so.
    pub delta: Payload<DeltaBytes>,
    /// The name now, for the view.
    pub event: MurmureEvent,
}

/// Whether an answer matches the messages of a segment against the numbers
/// the relay gave them, or leaves them as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matching {
    Skip,
    Resolve,
}

/// An open channel: its documents, each apart (`MURMURE-021`).
#[derive(Debug)]
pub struct ChannelState {
    channel_id: Id<Channel>,
    meta: Held,
    active: Held,
    closing: Option<Held>,
    /// Final segments, loaded only on the view's demand (`MURMURE-023`).
    loaded: HashMap<Id<Segment>, LoroDoc>,
    /// Final segments the view asked for, whose bytes are still arriving.
    /// Nothing reads them until [`ChannelState::older_ready`] admits them.
    pending: HashMap<Id<Segment>, Vec<Arrival>>,
    /// The relay's number of each message, per segment
    /// (`features/ambre-spec-murmure-unread.md section 3`). A segment that
    /// came with no number has no entry.
    sequences: HashMap<Id<Document>, Sequence>,
    /// The messages past which the active segment asks to close.
    segment_size: Count<Messages>,
    /// The snapshot bytes past which the active segment asks to close.
    segment_snapshot_size: ambre_core::Count<ambre_core::Octets>,
    /// Whether the active segment asked to close and waits for the answer.
    close_asked: CloseAsk,
    /// Whether a rotation retired the key the active segment was opened
    /// under, so it closes whatever it holds.
    key_rotated: Rotation,
    /// The successor another client created, whose bytes are in `pending`.
    successor: Option<Id<Segment>>,
    /// The identity issued for that successor, adopted once it is admitted.
    successor_session: Option<(ambre_core::PeerId, Payload<PeerEntryBytes>)>,
    /// The successor that failed to open, fetched again on a later tick.
    successor_retry: Option<closing::Retry>,
    /// The finalization of the closing segment, as this client drives it.
    finalizing: Option<finalizing::Finalizing>,
    /// A final segment whose final snapshot a report dropped, finalized again
    /// before the closing one.
    refinalizing: Option<Id<Segment>>,
    /// The fold of the amendments of a final segment, as this client drives
    /// it.
    folding: Option<folding::Folding>,
    /// The amendments document of each locked segment it holds, by that
    /// segment (`MURMURE-013`).
    amendments: HashMap<Id<Segment>, Held>,
    /// The amendments documents whose bytes are still arriving.
    amendments_pending: HashMap<Id<Document>, amending::Pending>,
    /// The live segments the relay locked: a change of one of their
    /// messages goes to an amendment (`MURMURE-073`).
    locked: HashSet<Id<Segment>>,
    /// The thread document of each root it holds, by that root
    /// (`MURMURE-105`).
    threads: HashMap<Id<Message>, threading::Thread>,
    /// The thread documents whose bytes are still arriving.
    threads_pending: HashMap<Id<Document>, threading::Pending>,
    /// The root a reply on a copy looks for, down the older segments.
    seek: Option<threading::Seek>,
    /// The band of the closing segment the channel opened without, until
    /// the caller takes it (`MURMURE-502`).
    closing_band: Option<MurmureEvent>,
    /// The last message a remote delta brought to a segment, until the
    /// caller takes it (`MURMURE-507`).
    arrived: Option<(Id<Segment>, Id<Message>)>,
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
    /// [`MurmureError::Crdt`] when Loro refuses a write. A name past its
    /// cap makes no [`Name`](ambre_core::Name), so it never reaches here
    /// (`MURMURE-579`).
    pub fn create(
        channel_id: Id<Channel>,
        channel_type: ChannelType,
        name: Option<&ambre_core::Validated<ambre_core::Name>>,
        now: Instant<Authored>,
    ) -> Result<Self, MurmureError> {
        let active_id = Id::<Document>::mint();
        let created = |id: Id<Document>, created: Created| -> Result<Held, MurmureError> {
            let doc = LoroDoc::new();
            created.write(&doc, channel_id)?;
            Ok(Held {
                created: Some(created),
                ..Held::served(id, doc)
            })
        };
        let meta = created(
            Id::<Document>::mint(),
            Created::Meta {
                channel_type,
                name: name.cloned(),
                active: Id::<Segment>::from_document(active_id),
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
        document_id: Id<Document>,
        peer: ambre_core::PeerId,
        entry: Payload<PeerEntryBytes>,
    ) -> Result<Option<Payload<DeltaBytes>>, MurmureError> {
        let channel_id = self.channel_id;
        let Some(held) = self.held_mut(document_id) else {
            let segment = Id::<Segment>::from_document(document_id);
            if self.note_successor_session(segment, peer, entry.clone())
                || self.note_amendments_session(document_id, peer, entry.clone())
                || self.note_thread_session(document_id, peer, entry)
            {
                return Ok(None);
            }
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
        // A segment's first update is a delta like any other, so it travels
        // as JSON, and so does an amendments or a thread document's
        // (`MURMURE-223`). `meta` travels binary.
        let first = match created {
            Created::Meta { .. } => session.export(&doc)?,
            Created::Segment { .. } | Created::Amendments { .. } | Created::Thread { .. } => {
                session.export_json(&doc)?
            }
        };
        held.doc = doc;
        held.session = Some(session);
        held.created = None;
        Ok(Some(first))
    }

    /// This channel with every live document under a fresh PeerID, as the
    /// Sync worker would issue them.
    #[cfg(test)]
    pub(crate) fn identify(mut self) -> Self {
        let ids: Vec<Id<Document>> = self.live().map(|held| held.id).collect();
        for id in ids {
            let peer = crate::session::next_peer();
            self.adopt_session(
                id,
                peer,
                Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec()),
            )
            .expect("adopt");
        }
        self
    }

    /// Whether this session may write `document_id`: its PeerID landed.
    pub fn identified(&self, document_id: Id<Document>) -> bool {
        self.live()
            .any(|held| held.id == document_id && held.session.is_some())
    }

    /// The live documents: `meta`, the active segment, the one being closed,
    /// the amendments documents and the thread documents.
    fn live(&self) -> impl Iterator<Item = &Held> {
        [Some(&self.meta), Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .chain(self.amendments.values())
            .chain(self.threads.values().map(|thread| &thread.held))
    }

    /// The live document of that identifier, to write it.
    fn held_mut(&mut self, document_id: Id<Document>) -> Option<&mut Held> {
        [
            Some(&mut self.meta),
            Some(&mut self.active),
            self.closing.as_mut(),
        ]
        .into_iter()
        .flatten()
        .chain(self.amendments.values_mut())
        .chain(self.threads.values_mut().map(|thread| &mut thread.held))
        .find(|held| held.id == document_id)
    }

    /// A channel of these documents, each already admitted.
    fn assemble(channel_id: Id<Channel>, meta: Held, active: Held, closing: Option<Held>) -> Self {
        Self {
            channel_id,
            meta,
            active,
            closing,
            loaded: HashMap::new(),
            pending: HashMap::new(),
            sequences: HashMap::new(),
            segment_size: SEGMENT_MESSAGES,
            segment_snapshot_size: SEGMENT_SNAPSHOT_SIZE,
            close_asked: CloseAsk::Idle,
            key_rotated: Rotation::Settled,
            successor: None,
            successor_session: None,
            successor_retry: None,
            finalizing: None,
            refinalizing: None,
            folding: None,
            amendments: HashMap::new(),
            amendments_pending: HashMap::new(),
            locked: HashSet::new(),
            threads: HashMap::new(),
            threads_pending: HashMap::new(),
            seek: None,
            closing_band: None,
            arrived: None,
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
        channel_id: Id<Channel>,
        meta_id: Id<Meta>,
        meta_bytes: &Payload<SnapshotBytes>,
        segments: &HashMap<Id<Segment>, Payload<SnapshotBytes>>,
    ) -> Result<Self, MurmureError> {
        let meta_id = meta_id.document_id();
        let meta = Held::served(
            meta_id,
            schema::admit(meta_bytes.into(), DocumentKind::Meta, channel_id, meta_id)?,
        );
        let live = live_segments(&meta.doc, channel_id)?;
        let admit_live = |segment: Id<Segment>| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let bytes = segments
                .get(&segment)
                .ok_or(MurmureError::SegmentMissing(id))?;
            let doc = schema::admit(bytes.into(), DocumentKind::Segment, channel_id, id)?;
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
        channel_id: Id<Channel>,
        meta: (Id<Document>, LoroDoc),
        mut segments: HashMap<Id<Document>, Vec<Arrival>>,
        sequences: HashMap<Id<Document>, Sequence>,
    ) -> Result<Self, MurmureError> {
        let (meta_id, meta_doc) = meta;
        let live = live_segments(&meta_doc, channel_id)?;
        let mut admit_live = |segment: Id<Segment>| -> Result<Held, MurmureError> {
            let id = segment.document_id();
            let arrivals = segments
                .remove(&id)
                .ok_or(MurmureError::SegmentMissing(id))?;
            let doc = schema::admit_with(DocumentKind::Segment, channel_id, id, |doc| {
                delta::import_arrivals(doc, arrivals, id, Rules::Segment)
            })?;
            Ok(Held::served(id, doc))
        };
        let active = admit_live(live.active)?;
        // The closing segment alone can stay out: the channel opens without
        // it and shows it as a band (`MURMURE-502`).
        let mut unread = None;
        let closing = live.closing.and_then(|segment| match admit_live(segment) {
            Ok(held) => Some(held),
            Err(MurmureError::SegmentMissing(_)) => {
                unread = Some((segment, Unreadable::KeyNotReceived));
                None
            }
            Err(error) => {
                tracing::warn!("[Murmure] closing segment {segment} refused: {error}");
                unread = Some((segment, Unreadable::Damaged));
                None
            }
        });
        let meta = Held::served(meta_id, meta_doc);
        let mut state = Self::assemble(channel_id, meta, active, closing);
        state.closing_band =
            unread.and_then(|(segment, cause)| state.band(segment.document_id(), cause));
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
        schema::string(&crate::schema::channel(&self.meta.doc), ChannelKey::Type)
            .and_then(|word| ChannelType::parse(&word))
    }

    /// The name of the channel, absent when it has none (`MURMURE-018`) or
    /// one past the cap.
    pub fn name(&self) -> Option<ambre_core::Validated<ambre_core::Name>> {
        schema::name(&crate::schema::channel(&self.meta.doc), ChannelKey::Name)
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
    pub fn name_direct(
        &mut self,
        name: Option<&ambre_core::Validated<ambre_core::Name>>,
    ) -> Result<Named, MurmureError> {
        if self.channel_type() != Some(ChannelType::Direct) {
            return Err(MurmureError::NotDirect(self.channel_id));
        }
        let meta = self.meta_id();
        let name = name
            .map(ambre_core::trimmed_name)
            .filter(|n| !ambre_core::name_is_empty(n));
        let (doc, session) = self.meta.session()?;
        let channel = crate::schema::channel(doc);
        match &name {
            Some(name) => channel.insert(ChannelKey::Name, name.as_str())?,
            None if channel.get(ChannelKey::Name).is_some() => channel.delete(ChannelKey::Name)?,
            None => {}
        }
        Ok(Named {
            meta,
            delta: session.export(doc)?,
            event: MurmureEvent::Named { name },
        })
    }

    /// Every operation of one held document, as one Loro update.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for an identifier the channel does not hold,
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn export_all(
        &self,
        document_id: Id<Document>,
    ) -> Result<Payload<DeltaBytes>, MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        Ok(Payload::<DeltaBytes>::new(
            doc.export(ExportMode::all_updates())?,
        ))
    }

    /// The channel this state holds.
    pub fn channel_id(&self) -> Id<Channel> {
        self.channel_id
    }

    /// The identifier of the `meta` document.
    pub fn meta_id(&self) -> Id<Meta> {
        Id::<Meta>::from_document(self.meta.id)
    }

    /// The identifier of the segment new messages go to.
    pub fn active_id(&self) -> Id<Segment> {
        Id::<Segment>::from_document(self.active.id)
    }

    /// The identifier of the segment being closed, if any.
    pub fn closing_id(&self) -> Option<Id<Segment>> {
        self.closing
            .as_ref()
            .map(|held| Id::<Segment>::from_document(held.id))
    }

    /// The band of the closing segment the channel opened without, once
    /// (`MURMURE-502`). `None` when the channel holds its closing segment.
    pub fn take_closing_band(&mut self) -> Option<MurmureEvent> {
        self.closing_band.take()
    }

    /// Whether the channel holds the document of that identifier in memory,
    /// or is loading it on the view's demand.
    pub fn holds(&self, document_id: Id<Document>) -> bool {
        self.doc(document_id).is_some() || self.is_pending(document_id)
    }

    /// Every document the channel holds or is loading, so a close can end
    /// the session of each.
    pub fn document_ids(&self) -> Vec<Id<Document>> {
        self.live()
            .map(|held| held.id)
            .chain(self.loaded.keys().map(Id::<Segment>::document_id))
            .chain(self.pending.keys().map(Id::<Segment>::document_id))
            .chain(self.amendments_pending.keys().copied())
            .chain(self.threads_pending.keys().copied())
            .collect()
    }

    /// The document of that identifier, among every one the channel holds.
    pub(crate) fn doc(&self, document_id: Id<Document>) -> Option<&LoroDoc> {
        self.live()
            .find(|held| held.id == document_id)
            .map(|held| &held.doc)
            .or_else(|| self.loaded.get(&Id::<Segment>::from_document(document_id)))
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
    pub fn snapshot(
        &self,
        document_id: Id<Document>,
    ) -> Result<Payload<SnapshotBytes>, MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let loro = Payload::<SnapshotBytes>::new(doc.export(ExportMode::Snapshot)?);
        Ok(match self.sequences.get(&document_id) {
            Some(sequence) => sequence.frame(loro),
            None => loro,
        })
    }

    /// Where `message` of `segment` stands in the relay's order, `None` for
    /// a segment the channel does not hold.
    pub fn position(&self, segment: Id<Segment>, message: Id<Message>) -> Option<Position> {
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
    pub fn delivered(&self, document_id: Id<Document>) -> Seq<Relay> {
        self.sequences
            .get(&document_id)
            .map_or(Seq::<Relay>::ZERO, Sequence::delivered)
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
        segment: Id<Segment>,
        read: Seq<Relay>,
        me: Id<User>,
    ) -> Result<ambre_core::Count<Unread>, MurmureError> {
        self.count_in(segment.document_id(), read, me, |_| true)
            .map(ambre_core::Count::<Unread>::recount)
    }

    /// The messages [`ChannelState::unread`] counts that mention `me` or the
    /// channel (`MURMURE-233`). The marks decide, never the summaries, so a
    /// false claim reads as nothing here (`MURMURE-104`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a segment the channel does not hold.
    pub fn mentions(
        &self,
        segment: Id<Segment>,
        read: Seq<Relay>,
        me: Id<User>,
    ) -> Result<ambre_core::Count<Mentions>, MurmureError> {
        self.mentions_in(segment.document_id(), read, me)
    }

    /// The replies of the held thread `thread` past the cursor `read` that
    /// mention `me` or the channel (the Murmure threads spec). They count on
    /// the channel as a message of a segment does.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a thread the channel does not hold.
    pub fn thread_mentions(
        &self,
        thread: Id<Document>,
        read: Seq<Relay>,
        me: Id<User>,
    ) -> Result<ambre_core::Count<Mentions>, MurmureError> {
        if !self.is_thread(thread) {
            return Err(MurmureError::NotHeld(thread));
        }
        self.mentions_in(thread, read, me)
    }

    /// The messages of `document_id` past `read` that mention `me` or the
    /// channel, read off their marks.
    fn mentions_in(
        &self,
        document_id: Id<Document>,
        read: Seq<Relay>,
        me: Id<User>,
    ) -> Result<ambre_core::Count<Mentions>, MurmureError> {
        let channel = self.channel_id;
        self.count_in(document_id, read, me, |message| {
            content_of(message)
                .is_some_and(|content| mention::names(&mention::read(&content, channel), me))
        })
        .map(ambre_core::Count::<Mentions>::recount)
    }

    /// The messages of a held segment or thread past `read`, written by
    /// someone other than `me`, that `counts` keeps.
    fn count_in(
        &self,
        document_id: Id<Document>,
        read: Seq<Relay>,
        me: Id<User>,
        counts: impl Fn(&Fields<MessageKey>) -> bool,
    ) -> Result<Count<Messages>, MurmureError> {
        let mut count = Count::<Messages>::ZERO;
        self.each_unread(document_id, read, me, |_, message| {
            if counts(message) {
                count = count.incremented();
            }
        })?;
        Ok(count)
    }

    /// The first message of the held segments, in list order, past the
    /// cursor `read_of` gives its segment and written by someone other than
    /// `me` (`MURMURE-499`). `None` when every held message is read.
    ///
    /// A message not numbered yet reads as unread, as it counts.
    pub fn first_unread(
        &self,
        read_of: impl Fn(Id<Segment>) -> Seq<Relay>,
        me: Id<User>,
    ) -> Option<Id<Message>> {
        let held = self.held_segments();
        segment_entries(&self.meta.doc)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| held.contains(id))
            .find_map(|segment| {
                let mut first = None;
                let read = read_of(segment);
                let found = self.each_unread(segment.document_id(), read, me, |id, _| {
                    first.get_or_insert(id);
                });
                found.ok().and(first)
            })
    }

    /// Hand `each` every message of a held segment or thread past `read`,
    /// in list order, written by someone other than `me`.
    fn each_unread(
        &self,
        document_id: Id<Document>,
        read: Seq<Relay>,
        me: Id<User>,
        mut each: impl FnMut(Id<Message>, &Fields<MessageKey>),
    ) -> Result<(), MurmureError> {
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let sequence = self.sequences.get(&document_id);
        crate::schema::root_list(doc, Root::Messages).for_each(|item| {
            let Some(message) = crate::schema::fields_of(item) else {
                return;
            };
            let Some((id, author)) = identity_of(&message) else {
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
                each(id, &message);
            }
        });
        Ok(())
    }

    /// Every segment the channel holds in memory: the live ones, then the
    /// older ones loaded on demand.
    pub fn held_segments(&self) -> Vec<Id<Segment>> {
        [Some(&self.active), self.closing.as_ref()]
            .into_iter()
            .flatten()
            .map(|held| Id::<Segment>::from_document(held.id))
            .chain(self.loaded.keys().copied())
            .collect()
    }

    /// The segments `meta` lists after `segment`, in list order: the ones
    /// whose messages all come later.
    pub fn segments_after(&self, segment: Id<Segment>) -> Vec<Id<Segment>> {
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
    pub fn messages(&self, segment: Id<Segment>) -> Result<Vec<MessageView>, MurmureError> {
        let document_id = segment.document_id();
        self.read_segment(segment)
            .ok_or(MurmureError::NotHeld(document_id))
    }

    /// The messages of one held segment as its members read them. Each
    /// amendment of a locked segment lies over the message it names, and
    /// each root carries the head of its thread.
    fn read_segment(&self, segment: Id<Segment>) -> Option<Vec<MessageView>> {
        let mut messages = read_messages(self.doc(segment.document_id())?, self.channel_id);
        if let Some(amendments) = self.amendments.get(&segment) {
            amending::overlay(&amendments.doc, &mut messages, self.channel_id);
        }
        self.overlay_threads(&mut messages);
        Some(messages)
    }

    /// The messages of every held segment that match `query`, and which
    /// segments were read (the Murmure search spec).
    ///
    /// It reads only what the channel holds: a final segment not loaded is
    /// listed, never fetched, and nothing leaves the engine.
    pub fn search(&self, query: &Query) -> ChannelMatches {
        let listed = segment_entries(&self.meta.doc);
        let held = self.held_segments();
        let covered: Vec<Id<Segment>> = listed
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| held.contains(id))
            .collect();
        let matches = covered
            .iter()
            .filter_map(|segment| Some((*segment, self.read_segment(*segment)?)))
            .flat_map(|(segment, messages)| {
                messages
                    .into_iter()
                    .filter(|message| query.matches(&message.text))
                    .map(move |message| SearchMatch { segment, message })
            })
            .collect();
        ChannelMatches {
            channel_id: self.channel_id,
            covered,
            listed: ambre_core::Count::<crate::message::Segments>::of(listed.len()),
            matches,
        }
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
        author: Id<User>,
        now: Instant<Authored>,
    ) -> Result<Sent, MurmureError> {
        self.send_attached(draft, &[], author, now)
    }

    /// [`ChannelState::send`], the message carrying `attachments` by their
    /// addresses alone (`MURMURE-017`). Each was uploaded before
    /// (`MURMURE-132`), so no byte of a file enters the segment
    /// (`MURMURE-134`).
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::send`], and
    /// [`MurmureError::BadAttachment`] for an attachment with a blank name.
    pub fn send_attached(
        &mut self,
        draft: &MessageDraft,
        attachments: &[Attachment],
        author: Id<User>,
        now: Instant<Authored>,
    ) -> Result<Sent, MurmureError> {
        // Checked before the first write, so a refused draft leaves no
        // message half written in the segment.
        if !mention::fits(&draft.mentions, &draft.text) {
            return Err(MurmureError::BadMention);
        }
        if !style::fits(&draft.styles, &draft.text) {
            return Err(MurmureError::BadStyle);
        }
        if !attachment::fits(attachments) {
            return Err(MurmureError::BadAttachment);
        }
        self.send_from(draft, attachments, author, now, None)
    }

    /// [`ChannelState::send_attached`] with the draft checked. A copy of a
    /// reply names the thread it came from (`MURMURE-109`), and takes the
    /// identifier its reply derives (`MURMURE-538`).
    fn send_from(
        &mut self,
        draft: &MessageDraft,
        attachments: &[Attachment],
        author: Id<User>,
        now: Instant<Authored>,
        copy: Option<(Id<Document>, Id<Message>)>,
    ) -> Result<Sent, MurmureError> {
        let channel = self.channel_id;
        let segment = self.active_id();
        let (doc, session) = self.active.session()?;
        let (id, mentions, styles) =
            write_message(doc, draft, attachments, author, now, channel, copy)?;
        let delta = session.export_json(doc)?;
        Ok(Sent {
            segment,
            document: segment.document_id(),
            delta,
            summary: summary_of(&mentions),
            event: MurmureEvent::Messages {
                segment,
                messages: vec![MessageView {
                    id,
                    author,
                    ts: now,
                    text: draft.text.clone(),
                    mentions,
                    styles,
                    attachments: attachments.to_vec(),
                    reactions: Vec::new(),
                    notice: None,
                    edited: crate::message::Revision::Original,
                    deleted: crate::message::Deletion::Live,
                    thread: None,
                    also_in_thread: copy.map(|(thread, _)| thread),
                }],
            },
        })
    }

    /// Announce in the active segment that `member` joins the channel
    /// (`MURMURE-057`).
    ///
    /// The notice is a message with an empty body, written by `author`, the
    /// member who invites. It names the member who joins.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses a write or the export.
    /// [`MurmureError::NoSession`] before the PeerID of the active segment
    /// landed.
    pub fn announce(
        &mut self,
        member: Id<User>,
        author: Id<User>,
        now: Instant<Authored>,
    ) -> Result<Sent, MurmureError> {
        let segment = self.active_id();
        let (doc, session) = self.active.session()?;
        let id = Id::<Message>::mint();
        let message = crate::schema::Fields::new(
            crate::schema::root_list(doc, Root::Messages).push_container(LoroMap::new())?,
        );
        message.insert(MessageKey::Id, id.to_string())?;
        message.insert(MessageKey::Author, author.to_string())?;
        message.insert(MessageKey::Ts, schema::ts_value(now))?;
        message.insert_container(MessageKey::Content, LoroText::new())?;
        message.insert_container(MessageKey::Reactions, LoroMap::new())?;
        message.insert(MessageKey::Notice, NoticeKind::MemberAdded.as_str())?;
        message.insert(MessageKey::Member, member.to_string())?;
        let delta = session.export_json(doc)?;
        Ok(Sent {
            segment,
            document: segment.document_id(),
            delta,
            summary: summary_of(&[]),
            event: MurmureEvent::Messages {
                segment,
                messages: vec![MessageView {
                    id,
                    author,
                    ts: now,
                    text: String::new(),
                    mentions: Vec::new(),
                    styles: Vec::new(),
                    attachments: Vec::new(),
                    reactions: Vec::new(),
                    notice: Some(Notice::MemberAdded { member }),
                    edited: crate::message::Revision::Original,
                    deleted: crate::message::Deletion::Live,
                    thread: None,
                    also_in_thread: None,
                }],
            },
        })
    }

    /// Add the reaction of `user` with `emoji` to `message` (`MURMURE-092`).
    ///
    /// One entry, under the member's own key, and nothing else: the message
    /// stays as it was (`MURMURE-095`). `None` when that reaction is already
    /// there, so nothing travels.
    ///
    /// # Errors
    /// [`MurmureError::NotEmoji`] for a value that is not one emoji, and
    /// every refusal of [`ChannelState::reactions_of`].
    pub fn react(
        &mut self,
        at: impl Into<MessageAt>,
        message: Id<Message>,
        emoji: &Emoji,
        user: Id<User>,
        now: Instant<Authored>,
    ) -> Result<Option<Sent>, MurmureError> {
        let at = at.into();
        if !reaction::is_emoji(emoji) {
            return Err(MurmureError::NotEmoji);
        }
        let key = reaction::ReactionKey::of(user, emoji);
        let (doc, session, map) = self.reactions_of(at, message)?;
        if map.get(key.as_str()).is_some() {
            return Ok(None);
        }
        map.insert(key.as_str(), schema::ts_value(now))?;
        let delta = session.export_json(doc)?;
        self.reacted_at(at, delta).map(Some)
    }

    /// Remove the reaction of `user` with `emoji` from `message`: their own
    /// entry alone (`MURMURE-096`, `MURMURE-097`). `None` when there is none.
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::reactions_of`].
    pub fn unreact(
        &mut self,
        at: impl Into<MessageAt>,
        message: Id<Message>,
        emoji: &Emoji,
        user: Id<User>,
    ) -> Result<Option<Sent>, MurmureError> {
        let at = at.into();
        let key = reaction::ReactionKey::of(user, emoji);
        let (doc, session, map) = self.reactions_of(at, message)?;
        if map.get(key.as_str()).is_none() {
            return Ok(None);
        }
        map.delete(key.as_str())?;
        let delta = session.export_json(doc)?;
        self.reacted_at(at, delta).map(Some)
    }

    /// The `reactions` map of `message` at `at`, with the session it is
    /// written under.
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::live_message`].
    fn reactions_of(
        &mut self,
        at: MessageAt,
        message: Id<Message>,
    ) -> Result<(&LoroDoc, &mut Session, LoroMap), MurmureError> {
        let (doc, session, map) = self.live_message(at, message)?;
        let reactions = match map.get(MessageKey::Reactions) {
            Some(ValueOrContainer::Container(Container::Map(reactions))) => reactions,
            _ => return Err(MurmureError::NoMessage(message)),
        };
        Ok((doc, session, reactions))
    }

    /// The map of `message` in the live segment or the held thread `at`,
    /// with the session it is written under.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a segment the channel does not hold, and
    /// [`MurmureError::Final`] for a final one. [`MurmureError::NoThread`]
    /// for a thread it does not hold. [`MurmureError::NoSession`] before its
    /// PeerID landed, and [`MurmureError::NoMessage`] for a message it does
    /// not hold.
    fn live_message(
        &mut self,
        at: MessageAt,
        message: Id<Message>,
    ) -> Result<(&LoroDoc, &mut Session, Fields<MessageKey>), MurmureError> {
        let held = match at {
            MessageAt::Segment(segment) => {
                let document_id = segment.document_id();
                if !self.holds(document_id) {
                    return Err(MurmureError::NotHeld(document_id));
                }
                self.held_mut(document_id)
                    .ok_or(MurmureError::Final(document_id))?
            }
            MessageAt::Thread(root) => {
                &mut self
                    .threads
                    .get_mut(&root)
                    .ok_or(MurmureError::NoThread(root))?
                    .held
            }
        };
        let (doc, session) = held.session()?;
        let map = message_map(doc, message).ok_or(MurmureError::NoMessage(message))?;
        Ok((doc, session, map))
    }

    /// The map of `message`, as [`ChannelState::live_message`], when `user`
    /// wrote it and has not deleted it.
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::live_message`], then
    /// [`MurmureError::NotAuthor`] for a message of another user and
    /// [`MurmureError::Deleted`] for a deleted one.
    fn own_message(
        &mut self,
        at: MessageAt,
        message: Id<Message>,
        user: Id<User>,
    ) -> Result<(&LoroDoc, &mut Session, Fields<MessageKey>), MurmureError> {
        let (doc, session, map) = self.live_message(at, message)?;
        // The check refuses a foreign change at every receiver, so a local
        // one is refused before it is written (`MURMURE-085`).
        if identity_of(&map).map(|(_, author)| author) != Some(user) {
            return Err(MurmureError::NotAuthor(message));
        }
        if schema::flag(&map, MessageKey::Deleted) {
            return Err(MurmureError::Deleted(message));
        }
        Ok((doc, session, map))
    }

    /// Replace the body of `message` with `draft`, and mark it edited
    /// (`MURMURE-084`).
    ///
    /// The body takes the difference alone, so a concurrent edit from
    /// another session of the author merges char by char. The identity, the
    /// author and the reactions stay as they were (`MURMURE-086`). `None`
    /// when the draft is the body already, so nothing travels.
    ///
    /// A message of a locked segment takes its edit as an amendment
    /// (`MURMURE-073`). A reply takes it in its thread (`MURMURE-485`).
    ///
    /// # Errors
    /// [`MurmureError::BadMention`] for a mention past the draft, and every
    /// refusal of [`ChannelState::own_message`]. A locked segment refuses as
    /// [`ChannelState::amend`] does.
    pub fn edit(
        &mut self,
        at: impl Into<MessageAt>,
        message: Id<Message>,
        draft: &MessageDraft,
        user: Id<User>,
    ) -> Result<Option<Sent>, MurmureError> {
        let at = at.into();
        if !mention::fits(&draft.mentions, &draft.text) {
            return Err(MurmureError::BadMention);
        }
        if !style::fits(&draft.styles, &draft.text) {
            return Err(MurmureError::BadStyle);
        }
        if let MessageAt::Segment(segment) = at
            && self.amendable(segment)
        {
            return self.amend(segment, message, user, amending::Change::Edit(draft));
        }
        let channel = self.channel_id;
        let (doc, session, map) = self.own_message(at, message, user)?;
        let content = content_of(&map).ok_or(MurmureError::NoMessage(message))?;
        if content.to_string() == draft.text
            && mention::read(&content, channel) == draft.mentions
            && style::read(&content) == style::normalized(&draft.styles)
        {
            return Ok(None);
        }
        // The marks are written again from the draft. A mention or a style
        // the edit took out leaves no mark on the text that stays.
        let len = content.len_unicode();
        if len > 0 {
            content.unmark(0..len, Mark::Mention.as_str())?;
        }
        style::clear(&content)?;
        content
            .update(&draft.text, UpdateOptions::default())
            .map_err(|error| MurmureError::Crdt(error.to_string()))?;
        mention::write(doc, &content, &draft.mentions, channel)?;
        style::write(doc, &content, &draft.styles)?;
        map.insert(MessageKey::Edited, true)?;
        let delta = session.export_json(doc)?;
        self.changed_at(at, delta).map(Some)
    }

    /// Delete `message`: mark it deleted and empty its body (`MURMURE-087`).
    ///
    /// The text goes, so no member reads it again. The `thread` field and
    /// the reactions stay, and the replies with them (`MURMURE-090`).
    ///
    /// A message of a locked segment takes its delete as an amendment, with
    /// `by: author` (`MURMURE-088`). A reply takes it in its thread
    /// (`MURMURE-485`).
    ///
    /// # Errors
    /// Every refusal of [`ChannelState::own_message`]. A deleted message is
    /// [`MurmureError::Deleted`], so a second delete writes nothing. A locked
    /// segment refuses as [`ChannelState::amend`] does.
    pub fn delete(
        &mut self,
        at: impl Into<MessageAt>,
        message: Id<Message>,
        user: Id<User>,
    ) -> Result<Sent, MurmureError> {
        let at = at.into();
        if let MessageAt::Segment(segment) = at
            && self.amendable(segment)
        {
            return self
                .amend(segment, message, user, amending::Change::Delete)?
                .ok_or(MurmureError::Deleted(message));
        }
        let (doc, session, map) = self.own_message(at, message, user)?;
        let content = content_of(&map).ok_or(MurmureError::NoMessage(message))?;
        let len = content.len_unicode();
        if len > 0 {
            content.delete(0, len)?;
        }
        // The files go with the text: the caller erases them from storage
        // (`MURMURE-240`), and no reference outlives them.
        if map.get(MessageKey::Attachments).is_some() {
            map.delete(MessageKey::Attachments)?;
        }
        map.insert(MessageKey::Deleted, true)?;
        let delta = session.export_json(doc)?;
        self.changed_at(at, delta)
    }

    /// The files a delete of `message` should erase (`MURMURE-240`). Each it
    /// carries is named once, less any another live message of `author` in a
    /// held segment still carries. One file attached twice then stays
    /// until its last message goes.
    ///
    /// Read before the delete, which empties the list. A file another member
    /// uploaded too keeps their claim on the relay (`MURMURE-242`).
    pub fn erasable(
        &self,
        segment: Id<Segment>,
        message: Id<Message>,
        author: Id<User>,
    ) -> Vec<Validated<ContentHash>> {
        let Some(target) = self.messages(segment).ok().and_then(|messages| {
            messages.into_iter().find(|view| {
                view.id == message && view.author == author && view.deleted == Deletion::Live
            })
        }) else {
            return Vec::new();
        };
        let others: Vec<MessageView> = self
            .held_segments()
            .into_iter()
            .filter_map(|id| self.messages(id).ok())
            .flatten()
            .filter(|view| {
                view.author == author && view.deleted == Deletion::Live && view.id != message
            })
            .collect();
        let mut erasable: Vec<Validated<ContentHash>> = Vec::new();
        for attachment in target.attachments {
            let elsewhere = others.iter().any(|view| {
                view.attachments
                    .iter()
                    .any(|other| other.media_ref == attachment.media_ref)
            });
            if !elsewhere && !erasable.contains(&attachment.media_ref) {
                erasable.push(attachment.media_ref);
            }
        }
        erasable
    }

    /// What an edit or a delete at `at` produced, as
    /// [`ChannelState::changed`] or its thread twin.
    ///
    /// # Errors
    /// [`MurmureError::NoThread`] for a thread the channel does not hold.
    fn changed_at(&self, at: MessageAt, delta: Payload<DeltaBytes>) -> Result<Sent, MurmureError> {
        let mut sent = self.reacted_at(at, delta)?;
        sent.summary.kind = ActivityKind::Edit;
        Ok(sent)
    }

    /// What a reaction at `at` produced, as [`ChannelState::reacted`] for a
    /// segment. A thread answers with its replies for the view.
    ///
    /// # Errors
    /// [`MurmureError::NoThread`] for a thread the channel does not hold.
    fn reacted_at(&self, at: MessageAt, delta: Payload<DeltaBytes>) -> Result<Sent, MurmureError> {
        match at {
            MessageAt::Segment(segment) => Ok(self.reacted(segment, delta)),
            MessageAt::Thread(root) => self.thread_reacted(root, delta),
        }
    }

    /// What an edit or a delete produced: the delta, its summary of kind
    /// edit, and the segment's messages for the view. An edit names no one,
    /// so its summary lists no mention.
    fn changed(&self, segment: Id<Segment>, delta: Payload<DeltaBytes>) -> Sent {
        let mut sent = self.reacted(segment, delta);
        sent.summary.kind = ActivityKind::Edit;
        sent
    }

    /// What a reaction delta produced: the delta, its summary of kind
    /// reaction (`MURMURE-093`), and the segment's messages for the view.
    fn reacted(&self, segment: Id<Segment>, delta: Payload<DeltaBytes>) -> Sent {
        let messages = self.read_segment(segment).unwrap_or_default();
        Sent {
            segment,
            document: segment.document_id(),
            delta,
            summary: reaction_summary(),
            event: MurmureEvent::Messages { segment, messages },
        }
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
        document_id: Id<Document>,
        payload: LoroPayloadRef<'_>,
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
        document_id: Id<Document>,
        order: Option<Order>,
        payload: LoroPayloadRef<'_>,
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        // An amendments document still arriving keeps its bytes too. It has
        // no message to number, but a fold names how far it reaches.
        if let Some(pending) = self.amendments_pending.get_mut(&document_id) {
            pending.arrivals.push(Arrival::Bytes(payload.to_payload()));
            if let Some(order) = order {
                self.sequences
                    .entry(document_id)
                    .or_default()
                    .note_bare(order, payload);
            }
            return Ok(Vec::new());
        }
        let amendments = self.amended_by(document_id).is_some();
        // An older segment or a thread still arriving keeps its bytes until
        // it is ready. Its numbers are noted now and matched once it is
        // admitted.
        let arriving = match self.threads_pending.get_mut(&document_id) {
            Some(pending) => Some(&mut pending.arrivals),
            None => self
                .pending
                .get_mut(&Id::<Segment>::from_document(document_id)),
        };
        if let Some(arrivals) = arriving {
            arrivals.push(Arrival::Bytes(payload.to_payload()));
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
        let status = sequence::import(doc, payload)?;
        if amendments {
            if let Some(order) = order {
                self.sequences
                    .entry(document_id)
                    .or_default()
                    .note_bare(order, payload);
            }
            return self.answer(document_id, Matching::Skip, None::<fn(&mut Sequence)>);
        }
        if status.pending.is_some()
            && matches!(order, Some(Order::Snapshot(_)))
            && let LoroPayloadRef::Snapshot(snapshot) = payload
        {
            self.replace_from_shallow(document_id, snapshot)?;
        }
        // Only an update needs the messages matched; a snapshot's numbers
        // ride in its table.
        let matching = if matches!(order, Some(Order::Update(_))) {
            Matching::Resolve
        } else {
            Matching::Skip
        };
        let note = order.map(|order| move |sequence: &mut Sequence| sequence.note(order, payload));
        self.answer(document_id, matching, note)
    }

    /// Hold `snapshot` in place of `document_id`, a copy it could not import
    /// into. A final snapshot is shallow, and Loro keeps it pending on a
    /// copy that lacks some of the deltas it folds in. Those deltas are
    /// purged, so the copy loads the snapshot and waits for none of them
    /// (`MURMURE-436`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the snapshot or the PeerID.
    fn replace_from_shallow(
        &mut self,
        document_id: Id<Document>,
        snapshot: &Payload<SnapshotBytes>,
    ) -> Result<(), MurmureError> {
        let fresh = LoroDoc::new();
        if sequence::import(&fresh, LoroPayloadRef::Snapshot(snapshot))?
            .pending
            .is_some()
        {
            return Ok(());
        }
        if let Some(held) = self.held_mut(document_id) {
            if let Some(session) = held.session {
                fresh.set_peer_id(session.peer().get())?;
            }
            held.doc = fresh;
        } else if let Some(doc) = self
            .loaded
            .get_mut(&Id::<Segment>::from_document(document_id))
        {
            *doc = fresh;
        }
        Ok(())
    }

    /// Import a delta a member signed, once the authorship check passed on
    /// it (`MURMURE-029`). A refused delta changes nothing (`MURMURE-030`).
    ///
    /// A segment delta is a JSON update, checked with no document copied
    /// (`MURMURE-226`). `meta` travels binary, and only its PeerID is checked
    /// (`AUTHOR-012`). A refused delta still counts as delivered.
    ///
    /// # Errors
    /// [`MurmureError::Refused`] for a delta the check refuses, and every
    /// refusal of [`ChannelState::apply_remote`].
    pub fn apply_signed(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: &Payload<DeltaBytes>,
        signer: Signer,
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        if document_id == self.meta.id {
            if !delta::binary_by(payload, signer.peer) {
                return Err(Refusal::NotSigners.into());
            }
            return self.apply_remote_at(document_id, Some(Order::Update(seq)), payload.into());
        }
        if !self.holds(document_id) {
            return Err(MurmureError::NotHeld(document_id));
        }
        if self.is_amendments(document_id) {
            return self.apply_amendment(document_id, seq, payload, signer);
        }
        let sequence = self.sequences.entry(document_id).or_default();
        sequence.passed(seq);
        let update = delta::read(payload)?;
        let range = delta::range(&update);
        let arriving = match self.threads_pending.get_mut(&document_id) {
            Some(pending) => Some(&mut pending.arrivals),
            None => self
                .pending
                .get_mut(&Id::<Segment>::from_document(document_id)),
        };
        if let Some(arrivals) = arriving {
            sequence.note_range(seq, range);
            arrivals.push(Arrival::Signed { update, signer });
            return Ok(Vec::new());
        }
        let rules = if self.is_thread(document_id) {
            Rules::Thread
        } else {
            Rules::Segment
        };
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        delta::verdict(doc, rules, &update, signer)?;
        let created = delta::created_messages(&update).pop();
        doc.import_json_updates(update)?;
        if let (Rules::Segment, Some(message)) = (rules, created) {
            self.arrived = Some((Id::<Segment>::from_document(document_id), message));
        }
        let note = move |sequence: &mut Sequence| sequence.note_range(seq, range);
        self.answer(document_id, Matching::Resolve, Some(note))
    }

    /// The last message a remote delta brought to a segment since the last
    /// call, so a notification opens at it (`MURMURE-507`).
    pub fn take_arrived(&mut self) -> Option<(Id<Segment>, Id<Message>)> {
        self.arrived.take()
    }

    /// What a document answers once a payload is in: `meta` its name, a
    /// segment its whole message list. `note` records where the payload
    /// stands, and [`Matching::Resolve`] matches the messages against it. A segment
    /// that came with no number keeps no entry.
    fn answer(
        &mut self,
        document_id: Id<Document>,
        matching: Matching,
        note: Option<impl FnOnce(&mut Sequence)>,
    ) -> Result<Vec<MurmureEvent>, MurmureError> {
        if document_id == self.meta.id {
            // The name is the one field of `meta` a view shows. It is read
            // whole, so a delta that touched something else repeats it.
            return Ok(vec![MurmureEvent::Named { name: self.name() }]);
        }
        // An amendments document answers with the segment it amends.
        if let Some(segment) = self.amended_by(document_id) {
            let messages = self
                .read_segment(segment)
                .ok_or(MurmureError::NotHeld(segment.document_id()))?;
            return Ok(vec![MurmureEvent::Messages { segment, messages }]);
        }
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let created = (matching == Matching::Resolve).then(|| sequence::created(doc));
        if let Some(note) = note {
            let sequence = self.sequences.entry(document_id).or_default();
            note(sequence);
            if let Some(created) = created {
                sequence.resolve_created(&created);
            }
        }
        // A thread answers with its replies and the head of its root.
        if let Some(events) = self.thread_answer(document_id) {
            return Ok(events);
        }
        // Every other document is a segment.
        let segment = Id::<Segment>::from_document(document_id);
        let messages = self
            .read_segment(segment)
            .ok_or(MurmureError::NotHeld(document_id))?;
        Ok(vec![MurmureEvent::Messages { segment, messages }])
    }

    /// The PeerID this session writes `document_id` under, `None` before the
    /// Sync worker issued it.
    pub fn peer(&self, document_id: Id<Document>) -> Option<ambre_core::PeerId> {
        self.live()
            .find(|held| held.id == document_id)
            .and_then(|held| held.session)
            .map(Session::peer)
    }

    /// The final segment listed just before `segment`, the one a scroll past
    /// the oldest loaded message asks for.
    pub fn older_than(&self, segment: Id<Segment>) -> Option<Id<Segment>> {
        let entries = segment_entries(&self.meta.doc);
        let position = entries.iter().position(|(id, _)| *id == segment)?;
        entries
            .iter()
            .take(position)
            .rev()
            .find(|(_, state)| *state == SegmentState::Final)
            .map(|(id, _)| *id)
    }

    /// The band that stands for the final segment `document_id`, which could
    /// not open for `cause` (`MURMURE-218`, `MURMURE-220`). A closing segment
    /// the channel opened without has one too (`MURMURE-502`).
    ///
    /// `None` for a document that is no final or closing segment `meta` lists,
    /// for one held or loaded, and for the successor. Its span runs from its
    /// `from_ts` to the `from_ts` of the next listed segment.
    pub fn band(&self, document_id: Id<Document>, cause: Unreadable) -> Option<MurmureEvent> {
        let segment = Id::<Segment>::from_document(document_id);
        if self.loaded.contains_key(&segment)
            || self.is_successor(segment)
            || self.closing_id() == Some(segment)
        {
            return None;
        }
        let (from, to) = segment_span(&self.meta.doc, segment)?;
        Some(MurmureEvent::Unreadable {
            segment,
            cause,
            from,
            to,
        })
    }

    /// Every final segment `meta` lists, in list order, loaded or not.
    ///
    /// Reaching the latest message reads them all (`MURMURE-115`). Their
    /// content no longer moves, so the relay may advance their cursors to
    /// their last number with no message skipped.
    pub fn final_segments(&self) -> Vec<Id<Segment>> {
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
    pub fn request_older(&mut self, segment: Id<Segment>) -> Option<Id<Segment>> {
        let older = self.older_than(segment)?;
        if self.loaded.contains_key(&older) || self.pending.contains_key(&older) {
            return None;
        }
        self.pending.insert(older, Vec::new());
        Some(older)
    }

    /// Ask again for the final segment `segment`, which the list shows as a
    /// band (`MURMURE-514`).
    ///
    /// Returns the document to open, or `None` when `meta` lists no such
    /// final segment, or when it is already loaded or on its way. Its bytes
    /// then take the path of [`ChannelState::request_older`].
    pub fn retry(&mut self, segment: Id<Segment>) -> Option<Id<Segment>> {
        if self.listed_as(segment) != Some(SegmentState::Final)
            || self.loaded.contains_key(&segment)
            || self.pending.contains_key(&segment)
        {
            return None;
        }
        self.pending.insert(segment, Vec::new());
        Some(segment)
    }

    /// Whether `segment` is the closing segment `meta` lists and the channel
    /// opened without (`MURMURE-502`). Only a new opening fetches it again
    /// (`MURMURE-514`).
    pub fn lacks_closing(&self, segment: Id<Segment>) -> bool {
        self.listed_as(segment) == Some(SegmentState::Closing) && self.closing_id() != Some(segment)
    }

    /// The state `meta` lists `segment` in, or `None` when it lists no such
    /// segment.
    fn listed_as(&self, segment: Id<Segment>) -> Option<SegmentState> {
        segment_entries(&self.meta.doc)
            .into_iter()
            .find(|(id, _)| *id == segment)
            .map(|(_, state)| state)
    }

    /// Whether `document_id` is an older segment still arriving.
    ///
    /// Takes the routing address, which names no kind: a document this
    /// channel did not ask for is simply not pending.
    pub fn is_pending(&self, document_id: Id<Document>) -> bool {
        self.pending
            .contains_key(&Id::<Segment>::from_document(document_id))
            || self.amendments_pending.contains_key(&document_id)
            || self.threads_pending.contains_key(&document_id)
    }

    /// Give up an older segment, a successor, an amendments or a thread
    /// document that could not open, so it can be asked again. Returns
    /// `false` when none of that identifier was asked for.
    pub fn cancel_older(&mut self, document_id: Id<Document>) -> bool {
        if self.amendments_pending.remove(&document_id).is_some() {
            return true;
        }
        if self.threads_pending.remove(&document_id).is_some() {
            self.sequences.remove(&document_id);
            self.end_seek_of(document_id);
            return true;
        }
        let segment = Id::<Segment>::from_document(document_id);
        self.sequences.remove(&document_id);
        self.forget_successor(segment);
        self.pending.remove(&segment).is_some()
    }

    /// The older segment, the successor or the amendments document
    /// `document_id` has all its bytes: check it and hold it (`MURMURE-023`,
    /// `MURMURE-065`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when no document of that identifier was
    /// asked for, and every refusal of [`ChannelState::load_older`]. A
    /// refused document is dropped, so it can be asked again.
    pub fn older_ready(&mut self, document_id: Id<Document>) -> Result<MurmureEvent, MurmureError> {
        if let Some(pending) = self.amendments_pending.remove(&document_id) {
            return self.amendments_ready(document_id, pending);
        }
        if let Some(pending) = self.threads_pending.remove(&document_id) {
            let ready = self.thread_ready(document_id, pending);
            if ready.is_err() {
                self.sequences.remove(&document_id);
            }
            return ready;
        }
        let segment = Id::<Segment>::from_document(document_id);
        let arrivals = self
            .pending
            .remove(&segment)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let loaded = if self.is_successor(segment) {
            let ready = self.successor_ready(segment, arrivals);
            if ready.is_err() {
                self.successor_failed(segment);
            }
            ready
        } else {
            self.load_older_from(segment, arrivals)
        };
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
        segment: Id<Segment>,
        payload: &Payload<SnapshotBytes>,
    ) -> Result<MurmureEvent, MurmureError> {
        let arrival = Arrival::Bytes(LoroPayload::Snapshot(payload.clone()));
        self.load_older_from(segment, vec![arrival])
    }

    fn load_older_from(
        &mut self,
        segment: Id<Segment>,
        arrivals: Vec<Arrival>,
    ) -> Result<MurmureEvent, MurmureError> {
        let document_id = segment.document_id();
        let listed = segment_entries(&self.meta.doc)
            .into_iter()
            .any(|(id, state)| id == segment && state == SegmentState::Final);
        if !listed {
            return Err(MurmureError::NotOlder(document_id));
        }
        let doc = schema::admit_with(DocumentKind::Segment, self.channel_id, document_id, |doc| {
            delta::import_arrivals(doc, arrivals, document_id, Rules::Segment)
        })?;
        if let Some(sequence) = self.sequences.get_mut(&document_id) {
            sequence.resolve(&doc);
        }
        self.loaded.insert(segment, doc);
        let messages = self.read_segment(segment).unwrap_or_default();
        Ok(MurmureEvent::Messages { segment, messages })
    }
}

/// The live segments `meta` lists: the active one, and the one being closed.
pub(crate) struct LiveSegments {
    pub(crate) active: Id<Segment>,
    pub(crate) closing: Option<Id<Segment>>,
}

/// The live segments a `meta` document lists (`MURMURE-022`). A final
/// segment is never among them (`MURMURE-028`).
///
/// # Errors
/// [`MurmureError::NoActiveSegment`] when `meta` lists no active segment.
pub(crate) fn live_segments(
    meta: &LoroDoc,
    channel_id: Id<Channel>,
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
    id: Id<Segment>,
    from: Instant<Authored>,
    state: SegmentState,
) -> Result<(), MurmureError> {
    let entry = crate::schema::Fields::new(
        crate::schema::root_list(meta, Root::Segments).push_container(LoroMap::new())?,
    );
    entry.insert(SegmentKey::Document, id.to_string())?;
    entry.insert(SegmentKey::From, schema::ts_value(from))?;
    entry.insert(SegmentKey::State, state.as_str())?;
    Ok(())
}

/// The segments `meta` lists, in list order. A malformed entry is skipped
/// with a warn: one bad entry must not close the channel. A segment listed
/// twice reads as its first entry: two clients writing one close may both
/// list the successor (`MURMURE-010`).
fn segment_entries(meta: &LoroDoc) -> Vec<(Id<Segment>, SegmentState)> {
    let list: LoroList = crate::schema::root_list(meta, Root::Segments);
    let mut entries = Vec::with_capacity(list.len());
    list.for_each(|item| {
        let Some(entry) = crate::schema::fields_of(item) else {
            tracing::warn!("[Murmure] a segments entry is not a map; skipped");
            return;
        };
        let id = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| Id::<Segment>::parse_str(&s).ok());
        let state = schema::string(&entry, SegmentKey::State).and_then(|s| SegmentState::parse(&s));
        match (id, state) {
            (Some(id), Some(_)) if entries.iter().any(|(listed, _)| *listed == id) => {}
            (Some(id), Some(state)) => entries.push((id, state)),
            _ => tracing::warn!("[Murmure] a segments entry is malformed; skipped"),
        }
    });
    entries
}

/// The span of the final segment `segment`: its `from_ts`, and the `from_ts`
/// of the segment listed after it. `None` when `meta` lists no final segment
/// of that identifier, or its `from_ts` is unreadable.
fn segment_span(
    meta: &LoroDoc,
    segment: Id<Segment>,
) -> Option<(Instant<Authored>, Option<Instant<Authored>>)> {
    let list: LoroList = crate::schema::root_list(meta, Root::Segments);
    let mut spans: Vec<(Id<Segment>, SegmentState, Option<u64>)> = Vec::with_capacity(list.len());
    list.for_each(|item| {
        let Some(entry) = crate::schema::fields_of(item) else {
            return;
        };
        let id = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| Id::<Segment>::parse_str(&s).ok());
        let state = schema::string(&entry, SegmentKey::State).and_then(|s| SegmentState::parse(&s));
        if let (Some(id), Some(state)) = (id, state)
            && !spans.iter().any(|(listed, _, _)| *listed == id)
        {
            spans.push((id, state, schema::unsigned(&entry, SegmentKey::From)));
        }
    });
    let position = spans.iter().position(|(id, _, _)| *id == segment)?;
    let (_, state, from) = spans.get(position)?;
    if !matches!(state, SegmentState::Final | SegmentState::Closing) {
        return None;
    }
    let to = spans
        .get(position + 1)
        .and_then(|(_, _, next)| *next)
        .map(Instant::<Authored>::new);
    Some((Instant::<Authored>::new((*from)?), to))
}

/// The messages of a segment, in list order. A malformed message is skipped
/// with a warn rather than failing the whole segment.
pub(crate) fn read_messages(doc: &LoroDoc, channel: Id<Channel>) -> Vec<MessageView> {
    let list = crate::schema::root_list(doc, Root::Messages);
    let mut messages = Vec::with_capacity(list.len());
    list.for_each(|item| match read_message(item, channel) {
        Some(view) => messages.push(view),
        None => tracing::warn!("[Murmure] a message is malformed; skipped"),
    });
    messages
}

/// The identifier and the author of one message, with no body read: what a
/// count needs. `None` when either is absent or unreadable.
fn identity_of(map: &Fields<MessageKey>) -> Option<(Id<Message>, Id<User>)> {
    let id = Id::<Message>::parse_str(&schema::string(map, MessageKey::Id)?).ok()?;
    let author = Id::<User>::parse_str(&schema::string(map, MessageKey::Author)?).ok()?;
    Some((id, author))
}

/// The body of one message, `None` when it is absent or not a text.
fn content_of(map: &Fields<MessageKey>) -> Option<LoroText> {
    match map.get(MessageKey::Content) {
        Some(ValueOrContainer::Container(Container::Text(content))) => Some(content),
        _ => None,
    }
}

/// One message, or `None` when a field is absent or unreadable.
fn read_message(item: ValueOrContainer, channel: Id<Channel>) -> Option<MessageView> {
    let map = crate::schema::fields_of(item)?;
    let (id, author) = identity_of(&map)?;
    let ts = Instant::<Authored>::new(schema::unsigned(&map, MessageKey::Ts)?);
    let content = content_of(&map)?;
    let deleted = schema::flag(&map, MessageKey::Deleted);
    Some(MessageView {
        id,
        author,
        ts,
        text: content.to_string(),
        mentions: mention::read(&content, channel),
        styles: style::read(&content),
        // A deleted message's files are erased with it (`MURMURE-240`).
        attachments: if deleted {
            Vec::new()
        } else {
            attachment::read(&map)
        },
        reactions: reaction::read(&map),
        notice: notice_of(&map)?,
        edited: Revision::from_bool(schema::flag(&map, MessageKey::Edited)),
        deleted: Deletion::from_bool(deleted),
        // The count is read from the thread document, once it is held
        // (`MURMURE-106`).
        thread: document_of(&map, MessageKey::Thread).map(|document| ThreadHead {
            document,
            replies: ambre_core::Count::<crate::message::Replies>::ZERO,
            last_replier: None,
        }),
        also_in_thread: document_of(&map, MessageKey::AlsoInThread),
    })
}

/// The document identifier under `key`, `None` when it is absent or
/// unreadable.
fn document_of(map: &Fields<MessageKey>, key: MessageKey) -> Option<Id<Document>> {
    Id::<Document>::parse_str(&schema::string(map, key)?).ok()
}

/// Append one message of `author` to the `messages` list of `doc`: the
/// active segment, or a thread (`MURMURE-080`).
///
/// The identifier is minted here, by the sending peer (`MURMURE-002`). A
/// copy of a reply in `thread` takes the one its reply derives instead
/// (`MURMURE-538`). The send writes one new message map and nothing else
/// (`MURMURE-083`). Returns the identifier and the marks as written.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
#[expect(
    clippy::type_complexity,
    reason = "each `Id<_>` names its kind, and no alias may hide it"
)]
fn write_message(
    doc: &LoroDoc,
    draft: &MessageDraft,
    attachments: &[Attachment],
    author: Id<User>,
    now: Instant<Authored>,
    channel: Id<Channel>,
    copy: Option<(Id<Document>, Id<Message>)>,
) -> Result<(Id<Message>, Vec<Mention>, Vec<StyleMark>), MurmureError> {
    let id = copy.map_or_else(Id::<Message>::mint, |(_, reply)| threading::copy_id(reply));
    let message = crate::schema::Fields::new(
        crate::schema::root_list(doc, Root::Messages).push_container(LoroMap::new())?,
    );
    message.insert(MessageKey::Id, id.to_string())?;
    message.insert(MessageKey::Author, author.to_string())?;
    message.insert(MessageKey::Ts, schema::ts_value(now))?;
    if let Some((thread, _)) = copy {
        message.insert(MessageKey::AlsoInThread, thread.to_string())?;
    }
    // The body is a char CRDT, so two peers typing in it keep both texts
    // (`MURMURE-004`, `MURMURE-005`).
    let content = message.insert_container(MessageKey::Content, LoroText::new())?;
    content.insert(0, &draft.text)?;
    mention::write(doc, &content, &draft.mentions, channel)?;
    style::write(doc, &content, &draft.styles)?;
    attachment::write(&message, attachments)?;
    // Created empty here, so no reaction ever creates a container, and
    // two first reactors never race for one (`MURMURE-256`).
    message.insert_container(MessageKey::Reactions, LoroMap::new())?;
    // The summary and the view read the marks as written.
    Ok((id, mention::read(&content, channel), style::read(&content)))
}

/// The notice a message carries: `Some(None)` for a message a member wrote,
/// and `None` for a notice this build cannot read, which is skipped.
fn notice_of(map: &Fields<MessageKey>) -> Option<Option<Notice>> {
    let Some(word) = schema::string(map, MessageKey::Notice) else {
        return Some(None);
    };
    match NoticeKind::parse(&word)? {
        NoticeKind::MemberAdded => {
            let member = Id::<User>::parse_str(&schema::string(map, MessageKey::Member)?).ok()?;
            Some(Some(Notice::MemberAdded { member }))
        }
    }
}

/// The map of the message `id` in `doc`, `None` when the segment holds none.
fn message_map(doc: &LoroDoc, id: Id<Message>) -> Option<Fields<MessageKey>> {
    let wanted = id.to_string();
    let list = crate::schema::root_list(doc, Root::Messages);
    (0..list.len()).find_map(|index| {
        let map = list.get(index).and_then(schema::fields_of::<MessageKey>)?;
        (schema::string(&map, MessageKey::Id).as_deref() == Some(wanted.as_str())).then_some(map)
    })
}

/// The activity summary of a reaction (`MURMURE-093`): it names no one.
fn reaction_summary() -> ActivitySummary {
    ActivitySummary {
        kind: ActivityKind::Reaction,
        mentions: Vec::new(),
        mentions_all: ambre_bus::MentionScope::Named,
        anchor: None,
    }
}

/// The activity summary of a new message (`MURMURE-081`). It has kind content,
/// lists each user its marks name once in text order, and says whether the
/// channel itself is named. An `@channel` lists no member (`MURMURE-230`).
fn summary_of(marked: &[Mention]) -> ActivitySummary {
    let mut mentions: Vec<Id<User>> = Vec::new();
    let mut mentions_all = ambre_bus::MentionScope::Named;
    for mention in marked {
        match &mention.target {
            MentionTarget::User(user) if !mentions.contains(user) => mentions.push(*user),
            MentionTarget::User(_) => {}
            MentionTarget::Channel => mentions_all = ambre_bus::MentionScope::All,
        }
    }
    ActivitySummary {
        kind: ActivityKind::Content,
        mentions,
        mentions_all,
        anchor: None,
    }
}
