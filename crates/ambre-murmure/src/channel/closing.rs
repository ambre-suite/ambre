//! The close of the active segment into its successor
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! The relay arbitrates a close, so a client asks it first and writes `meta`
//! with its answer (`MURMURE-011`). Every client that writes the answer
//! writes the same entries, so two writers converge on one successor.

use ambre_bus::{MurmureEvent, Timestamp};
use ambre_core::SegmentId;
use loro::{Container, LoroDoc, ValueOrContainer};

use super::{ChannelState, Created, Held, live_segments, push_segment_entry, read_messages};
use crate::delta::{self, Arrival};
use crate::error::MurmureError;
use crate::schema::{
    self, DocumentKind, MESSAGES, SEGMENT_DOCUMENT, SEGMENT_STATE, SEGMENTS, SegmentState,
};
use crate::session::Session;

/// The messages past which an active segment asks to close: the segment size
/// row of `ambre-spec-budgets.md section 6`. The 2 MB bound of the row is not
/// checked here: 5000 chat-length messages stay well under it.
pub const SEGMENT_MESSAGES: usize = 5000;

/// What writing a close produced: the `meta` delta to send, and the successor
/// to fetch when another client created it.
#[derive(Debug)]
pub struct Closed {
    /// Every operation the close wrote in `meta`, as a Loro update.
    pub meta: Vec<u8>,
    /// The successor to open, when this client did not create it. Its bytes
    /// then go to [`ChannelState::apply_remote`], and
    /// [`ChannelState::segment_ready`] admits it once they are all in.
    pub fetch: Option<SegmentId>,
}

impl ChannelState {
    /// This channel, with segments that ask to close past `messages`.
    #[must_use]
    pub fn with_segment_size(mut self, messages: usize) -> Self {
        self.segment_size = messages;
        self
    }

    /// The active segment to close, once it holds the segment size
    /// (`MURMURE-064`). It is answered once per active segment, until the
    /// relay answers or refuses.
    ///
    /// `None` while a segment is still closing: the relay refuses a second
    /// close then (`SRV-196`), and the finalization frees the slot.
    pub fn wants_close(&mut self) -> Option<SegmentId> {
        if self.close_asked || self.closing.is_some() || self.successor.is_some() {
            return None;
        }
        if self.active.doc.get_list(MESSAGES).len() < self.segment_size {
            return None;
        }
        self.close_asked = true;
        Some(self.active_id())
    }

    /// The relay refused the close, or never answered: the next send asks
    /// again.
    pub fn close_refused(&mut self) {
        self.close_asked = false;
    }

    /// Write the relay's answer to a close of `closing` into `meta`: the
    /// segment is `closing`, and `active` follows it (`MURMURE-065`).
    ///
    /// `created` says this client registered `active`: the channel then
    /// holds it at once, and writes its header under its PeerID once the
    /// session lands. Otherwise the close lost its race, and the successor is
    /// fetched as another member wrote it. No message moves: each stays in
    /// the segment it was sent to (`MURMURE-066`).
    ///
    /// # Errors
    /// - [`MurmureError::NotActive`]: `closing` is no longer the active
    ///   segment.
    /// - [`MurmureError::StillClosing`]: another segment is closing.
    /// - [`MurmureError::NoSession`]: the PeerID of `meta` has not landed.
    /// - [`MurmureError::Crdt`]: Loro refused a write.
    pub fn close_into(
        &mut self,
        closing: SegmentId,
        active: SegmentId,
        created: bool,
        now: Timestamp,
    ) -> Result<Closed, MurmureError> {
        if closing != self.active_id() {
            return Err(MurmureError::NotActive(closing.document_id()));
        }
        if self.closing.is_some() {
            return Err(MurmureError::StillClosing(self.channel_id));
        }
        let (doc, session) = self.meta.session()?;
        record_close(doc, closing, active, now)?;
        let meta = session.export(doc)?;
        self.close_asked = false;
        if !created {
            return Ok(Closed {
                meta,
                fetch: self.await_successor(active),
            });
        }
        let doc = LoroDoc::new();
        let created = Created::Segment { now };
        created.write(&doc, self.channel_id)?;
        let successor = Held {
            created: Some(created),
            ..Held::served(active.document_id(), doc)
        };
        self.closing = Some(std::mem::replace(&mut self.active, successor));
        Ok(Closed { meta, fetch: None })
    }

    /// The successor `meta` names once another client closed the active
    /// segment, to fetch (`MURMURE-065`). `None` when `meta` names no new
    /// active segment, or when it is already on its way.
    pub fn successor_to_open(&mut self) -> Option<SegmentId> {
        let live = live_segments(&self.meta.doc, self.channel_id).ok()?;
        if live.active == self.active_id() || live.closing != Some(self.active_id()) {
            return None;
        }
        self.await_successor(live.active)
    }

    /// Wait for the bytes of `successor`. Messages still go to the active
    /// segment meanwhile, which takes them while it closes.
    fn await_successor(&mut self, successor: SegmentId) -> Option<SegmentId> {
        if self.successor.is_some() || self.closing.is_some() {
            return None;
        }
        self.successor = Some(successor);
        self.pending.insert(successor, Vec::new());
        Some(successor)
    }

    /// Note the identity issued for the successor still arriving, adopted
    /// once it is admitted. Returns whether the document is that successor.
    pub(super) fn note_successor_session(
        &mut self,
        segment: SegmentId,
        peer: u64,
        entry: Vec<u8>,
    ) -> bool {
        if self.successor != Some(segment) {
            return false;
        }
        self.successor_session = Some((peer, entry));
        true
    }

    /// Whether `segment` is the successor still arriving.
    pub(super) fn is_successor(&self, segment: SegmentId) -> bool {
        self.successor == Some(segment)
    }

    /// Forget the successor that could not open. The next `meta` delta asks
    /// for it again.
    pub(super) fn forget_successor(&mut self, segment: SegmentId) {
        if self.successor == Some(segment) {
            self.successor = None;
            self.successor_session = None;
        }
    }

    /// The successor has all its bytes: check it, and hold it as the active
    /// segment. The old active segment is the closing one now.
    pub(super) fn successor_ready(
        &mut self,
        segment: SegmentId,
        arrivals: Vec<Arrival>,
    ) -> Result<MurmureEvent, MurmureError> {
        self.successor = None;
        let identity = self.successor_session.take();
        let document_id = segment.document_id();
        let doc = schema::admit_with(DocumentKind::Segment, self.channel_id, document_id, |doc| {
            delta::import_arrivals(doc, arrivals, document_id)
        })?;
        if let Some(sequence) = self.sequences.get_mut(&document_id) {
            sequence.resolve(&doc);
        }
        let messages = read_messages(&doc, self.channel_id);
        let mut held = Held::served(document_id, doc);
        if let Some((peer, entry)) = identity {
            held.session = Some(Session::adopt(&held.doc, peer, entry)?);
        }
        self.closing = Some(std::mem::replace(&mut self.active, held));
        self.close_asked = false;
        Ok(MurmureEvent::Messages { segment, messages })
    }
}

/// Mark every entry of `closing` closing, and list `active` after it unless
/// `meta` lists it already. Two clients writing one answer write the same
/// states, and a duplicate entry of `active` reads as one segment.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
fn record_close(
    meta: &LoroDoc,
    closing: SegmentId,
    active: SegmentId,
    now: Timestamp,
) -> Result<(), MurmureError> {
    let list = meta.get_list(SEGMENTS);
    let mut listed = false;
    for index in 0..list.len() {
        let Some(ValueOrContainer::Container(Container::Map(entry))) = list.get(index) else {
            continue;
        };
        let id =
            schema::string(&entry, SEGMENT_DOCUMENT).and_then(|s| SegmentId::parse_str(&s).ok());
        if id == Some(closing) {
            entry.insert(SEGMENT_STATE, SegmentState::Closing.as_str())?;
        } else if id == Some(active) {
            listed = true;
        }
    }
    if !listed {
        push_segment_entry(meta, active, now, SegmentState::Active)?;
    }
    Ok(())
}
