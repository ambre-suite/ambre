//! A channel on its way to open (`features/ambre-spec-murmure-engine.md
//! section 3`).
//!
//! The relay serves each document of a channel as an ordinary document: a
//! snapshot, a replay, then a signal that the replay is done. So a channel
//! opens in two steps. First its `meta` document, which names the live
//! segments. Then those segments (`MURMURE-025`). Only then is a
//! [`ChannelState`] built, and nothing of the channel is readable before.

use std::collections::{HashMap, HashSet};

use ambre_core::{ChannelId, DocumentId, MetaId, SegmentId};
use loro::LoroDoc;

use crate::channel::{ChannelState, live_segments};
use crate::error::MurmureError;
use crate::schema::{self, DocumentKind};
use crate::sequence::{Order, Sequence};

/// Where an opening channel stands after a document of it is ready.
#[derive(Debug)]
pub enum Opening {
    /// Still waiting for a segment already asked for.
    Waiting,
    /// `meta` is in and names these live segments: open each.
    OpenSegments(Vec<SegmentId>),
    /// Every document is in, and the channel is open.
    Open(Box<ChannelState>),
}

/// The documents of a channel that is opening, and their bytes so far.
#[derive(Debug)]
pub struct OpeningChannel {
    channel_id: ChannelId,
    meta_id: DocumentId,
    /// Bytes of each expected document, in arrival order, until it is ready.
    payloads: HashMap<DocumentId, Vec<Vec<u8>>>,
    /// `meta` once admitted. `None` while its bytes are still arriving.
    meta: Option<LoroDoc>,
    /// The live segments `meta` named that are not ready yet.
    awaiting: HashSet<DocumentId>,
    /// What the relay numbered of each segment so far, handed to the channel
    /// once it opens.
    sequences: HashMap<DocumentId, Sequence>,
}

impl OpeningChannel {
    /// Begin opening `channel_id`, whose `meta` document the relay named.
    pub fn new(channel_id: ChannelId, meta_id: MetaId) -> Self {
        let meta_id = meta_id.document_id();
        Self {
            channel_id,
            meta_id,
            payloads: HashMap::from([(meta_id, Vec::new())]),
            meta: None,
            awaiting: HashSet::new(),
            sequences: HashMap::new(),
        }
    }

    /// The channel being opened.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// The `meta` document of the channel.
    pub fn meta_id(&self) -> MetaId {
        MetaId::from_document(self.meta_id)
    }

    /// Whether the bytes of `document_id` belong to this opening.
    pub fn expects(&self, document_id: DocumentId) -> bool {
        self.payloads.contains_key(&document_id)
    }

    /// Every document this opening asked the relay for, so a close or a
    /// failure can end the session of each.
    pub fn document_ids(&self) -> Vec<DocumentId> {
        self.payloads.keys().copied().collect()
    }

    /// Keep the bytes of an expected document until it is ready. Returns
    /// `false` for a document this opening does not expect.
    pub fn receive(&mut self, document_id: DocumentId, payload: &[u8]) -> bool {
        self.receive_at(document_id, None, payload)
    }

    /// [`OpeningChannel::receive`], with where the relay placed the payload.
    /// A segment's numbers are noted now and matched once the channel opens
    /// (`features/ambre-spec-murmure-unread.md section 3`).
    pub fn receive_at(
        &mut self,
        document_id: DocumentId,
        order: Option<Order>,
        payload: &[u8],
    ) -> bool {
        // `meta` already admitted takes a live delta at once, as an open
        // channel would.
        if document_id == self.meta_id
            && let Some(meta) = &self.meta
        {
            if let Err(e) = meta.import(payload) {
                tracing::warn!("[Murmure] delta for meta {document_id} refused: {e:?}");
            }
            return true;
        }
        let Some(held) = self.payloads.get_mut(&document_id) else {
            return false;
        };
        held.push(payload.to_vec());
        // `meta` holds no message: only a segment is numbered.
        if let Some(order) = order
            && document_id != self.meta_id
        {
            self.sequences
                .entry(document_id)
                .or_default()
                .note(order, payload);
        }
        true
    }

    /// `document_id` has all its bytes. `meta` names the live segments to
    /// open next. The last live segment opens the channel.
    ///
    /// # Errors
    /// Every refusal of the header check on `meta` or on a segment, and
    /// [`MurmureError::NoActiveSegment`] for a `meta` that lists none. The
    /// opening is then over: the caller drops it and closes its documents.
    /// [`MurmureError::NotHeld`] for a document this opening does not expect.
    pub fn ready(&mut self, document_id: DocumentId) -> Result<Opening, MurmureError> {
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
            &self.payloads,
            std::mem::take(&mut self.sequences),
        )?;
        Ok(Opening::Open(Box::new(state)))
    }

    fn meta_ready(&mut self) -> Result<Opening, MurmureError> {
        let payloads: Vec<&[u8]> = self.payloads[&self.meta_id]
            .iter()
            .map(Vec::as_slice)
            .collect();
        let meta = schema::admit_all(&payloads, DocumentKind::Meta, self.channel_id, self.meta_id)?;
        let live = live_segments(&meta, self.channel_id)?;
        let segments: Vec<SegmentId> = [Some(live.active), live.closing]
            .into_iter()
            .flatten()
            .collect();
        for segment in &segments {
            self.payloads.insert(segment.document_id(), Vec::new());
            self.awaiting.insert(segment.document_id());
        }
        self.meta = Some(meta);
        // The entry of `meta` stays, emptied, so `expects` still names it.
        if let Some(held) = self.payloads.get_mut(&self.meta_id) {
            held.clear();
        }
        Ok(Opening::OpenSegments(segments))
    }
}
