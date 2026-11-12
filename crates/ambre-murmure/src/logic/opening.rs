//! A channel on its way to open (`features/ambre-spec-murmure-engine.md
//! section 3`).
//!
//! The relay serves each document of a channel as an ordinary document: a
//! snapshot, a replay, then a signal that the replay is done. So a channel
//! opens in two steps. First its `meta` document, which names the live
//! segments. Then those segments (`MURMURE-025`). Only then is a
//! [`ChannelState`] built, and nothing of the channel is readable before.

use std::collections::{HashMap, HashSet};

use ambre_core::id::{Channel, Document, Id, Meta, Segment};
use ambre_core::{DeltaBytes, LoroPayloadRef, Payload, Relay, Seq};
use loro::LoroDoc;

use crate::logic::channel::{ChannelState, live_segments};
use crate::logic::delta::{self, Arrival, Refusal, Signer};
use crate::logic::error::MurmureError;
use crate::logic::schema::{self, DocumentKind};
use crate::logic::sequence::{Order, Sequence};

/// Where an opening channel stands after a document of it is ready.
#[derive(Debug)]
pub enum Opening {
    /// Still waiting for a segment already asked for.
    Waiting,
    /// `meta` is in and names these live segments: open each.
    OpenSegments(Vec<Id<Segment>>),
    /// Every document is in, and the channel is open.
    Open(Box<ChannelState>),
}

/// What an opening did with a payload it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Receipt {
    /// The payload belongs to a document of this opening, which keeps it.
    Kept,
    /// No document of this opening is the one named, so nothing was kept.
    NotExpected,
}

/// The documents of a channel that is opening, and their bytes so far.
#[derive(Debug)]
pub struct OpeningChannel {
    channel_id: Id<Channel>,
    meta_id: Id<Document>,
    /// Payloads of each expected document, in arrival order, until it is
    /// ready. `meta` only ever holds bytes.
    payloads: HashMap<Id<Document>, Vec<Arrival>>,
    /// `meta` once admitted. `None` while its bytes are still arriving.
    meta: Option<LoroDoc>,
    /// The live segments `meta` named that are not ready yet.
    awaiting: HashSet<Id<Document>>,
    /// The closing segment `meta` named, the one live segment the channel
    /// can open without.
    closing: Option<Id<Document>>,
    /// What the relay numbered of each segment so far, handed to the channel
    /// once it opens.
    sequences: HashMap<Id<Document>, Sequence>,
}

impl OpeningChannel {
    /// Begin opening `channel_id`, whose `meta` document the relay named.
    pub fn new(channel_id: Id<Channel>, meta_id: Id<Meta>) -> Self {
        let meta_id = meta_id.document_id();
        Self {
            channel_id,
            meta_id,
            payloads: HashMap::from([(meta_id, Vec::new())]),
            meta: None,
            awaiting: HashSet::new(),
            closing: None,
            sequences: HashMap::new(),
        }
    }

    /// The channel being opened.
    pub fn channel_id(&self) -> Id<Channel> {
        self.channel_id
    }

    /// The `meta` document of the channel.
    pub fn meta_id(&self) -> Id<Meta> {
        Id::<Meta>::from_document(self.meta_id)
    }

    /// Whether the bytes of `document_id` belong to this opening.
    pub fn expects(&self, document_id: Id<Document>) -> bool {
        self.payloads.contains_key(&document_id)
    }

    /// Every document this opening asked the relay for, so a close or a
    /// failure can end the session of each.
    pub fn document_ids(&self) -> Vec<Id<Document>> {
        self.payloads.keys().copied().collect()
    }

    /// Keep the bytes of an expected document until it is ready. A document
    /// this opening does not expect is [`Receipt::NotExpected`].
    pub fn receive(&mut self, document_id: Id<Document>, payload: LoroPayloadRef<'_>) -> Receipt {
        self.receive_at(document_id, None, payload)
    }

    /// [`OpeningChannel::receive`], with where the relay placed the payload.
    /// A segment's numbers are noted now and matched once the channel opens
    /// (`features/ambre-spec-murmure-unread.md section 3`).
    pub fn receive_at(
        &mut self,
        document_id: Id<Document>,
        order: Option<Order>,
        payload: LoroPayloadRef<'_>,
    ) -> Receipt {
        // `meta` already admitted takes a live delta at once, as an open
        // channel would.
        if document_id == self.meta_id
            && let Some(meta) = &self.meta
        {
            if let Err(e) = meta.import(payload.as_bytes()) {
                tracing::warn!("[Murmure] delta for meta {document_id} refused: {e:?}");
            }
            return Receipt::Kept;
        }
        let Some(held) = self.payloads.get_mut(&document_id) else {
            return Receipt::NotExpected;
        };
        held.push(Arrival::Bytes(payload.to_payload()));
        // `meta` holds no message: only a segment is numbered.
        if let Some(order) = order
            && document_id != self.meta_id
        {
            self.sequences
                .entry(document_id)
                .or_default()
                .note(order, payload);
        }
        Receipt::Kept
    }

    /// Keep a delta a member signed until its document is ready
    /// (`MURMURE-029`). A segment delta is read now and checked once the
    /// segment opens, against its registry then. `meta` is checked now.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a document this opening does not expect,
    /// and [`MurmureError::Refused`] for a delta that reads as no update of
    /// its signer. A refused delta still counts as delivered.
    pub fn receive_signed(
        &mut self,
        document_id: Id<Document>,
        seq: Seq<Relay>,
        payload: &Payload<DeltaBytes>,
        signer: Signer,
    ) -> Result<(), MurmureError> {
        if !self.expects(document_id) {
            return Err(MurmureError::NotHeld(document_id));
        }
        if document_id == self.meta_id {
            if !delta::binary_by(payload, signer.peer) {
                return Err(Refusal::NotSigners.into());
            }
            let _kept = self.receive(document_id, payload.into());
            return Ok(());
        }
        let sequence = self.sequences.entry(document_id).or_default();
        sequence.passed(seq);
        let update = delta::read(payload)?;
        sequence.note_range(seq, delta::range(&update));
        if let Some(held) = self.payloads.get_mut(&document_id) {
            held.push(Arrival::Signed { update, signer });
        }
        Ok(())
    }

    /// `document_id` has all its bytes. `meta` names the live segments to
    /// open next. The last live segment opens the channel.
    ///
    /// # Errors
    /// Every refusal of the header check on `meta` or on a segment, and
    /// [`MurmureError::NoActiveSegment`] for a `meta` that lists none. The
    /// opening is then over: the caller drops it and closes its documents.
    /// [`MurmureError::NotHeld`] for a document this opening does not expect.
    pub fn ready(&mut self, document_id: Id<Document>) -> Result<Opening, MurmureError> {
        if document_id == self.meta_id && self.meta.is_none() {
            return self.meta_ready();
        }
        if !self.awaiting.remove(&document_id) {
            return if self.expects(document_id) {
                // A second ready for a document already in changes nothing.
                Ok(Opening::Waiting)
            } else {
                Err(MurmureError::NotHeld(document_id))
            };
        }
        self.finish()
    }

    /// The closing segment could not open for want of its key: the channel
    /// opens without it, and shows it as a band.
    ///
    /// Returns `None` for any other document, whose failure ends the opening.
    /// Otherwise as [`OpeningChannel::ready`].
    pub fn give_up(&mut self, document_id: Id<Document>) -> Option<Result<Opening, MurmureError>> {
        if self.closing != Some(document_id) || !self.awaiting.remove(&document_id) {
            return None;
        }
        self.payloads.remove(&document_id);
        self.sequences.remove(&document_id);
        Some(self.finish())
    }

    /// Open the channel once no live segment is awaited.
    fn finish(&mut self) -> Result<Opening, MurmureError> {
        if !self.awaiting.is_empty() {
            return Ok(Opening::Waiting);
        }
        let meta = self
            .meta
            .take()
            .ok_or(MurmureError::NotHeld(self.meta_id))?;
        self.payloads.remove(&self.meta_id);
        let state = ChannelState::open_from_payloads(
            self.channel_id,
            (self.meta_id, meta),
            std::mem::take(&mut self.payloads),
            std::mem::take(&mut self.sequences),
        )?;
        Ok(Opening::Open(Box::new(state)))
    }

    fn meta_ready(&mut self) -> Result<Opening, MurmureError> {
        let payloads: Vec<LoroPayloadRef<'_>> = self
            .payloads
            .get(&self.meta_id)
            .ok_or(MurmureError::NotHeld(self.meta_id))?
            .iter()
            .filter_map(|arrival| match arrival {
                Arrival::Bytes(bytes) => Some(bytes.borrowed()),
                Arrival::Signed { .. } => None,
            })
            .collect();
        let meta = schema::admit_all(&payloads, DocumentKind::Meta, self.channel_id, self.meta_id)?;
        let live = live_segments(&meta, self.channel_id)?;
        let segments: Vec<Id<Segment>> = [Some(live.active), live.closing]
            .into_iter()
            .flatten()
            .collect();
        for segment in &segments {
            self.payloads.insert(segment.document_id(), Vec::new());
            self.awaiting.insert(segment.document_id());
        }
        self.closing = live.closing.map(|segment| segment.document_id());
        self.meta = Some(meta);
        // The entry of `meta` stays, emptied, so `expects` still names it.
        if let Some(held) = self.payloads.get_mut(&self.meta_id) {
            held.clear();
        }
        Ok(Opening::OpenSegments(segments))
    }
}
