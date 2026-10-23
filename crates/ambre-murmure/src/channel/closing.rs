//! The close of the active segment into its successor
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! The relay arbitrates a close, so a client asks it first and writes `meta`
//! with its answer (`MURMURE-011`). Every client that writes the answer
//! writes the same entries, so two writers converge on one successor.

use ambre_bus::{MurmureEvent, Timestamp};
use ambre_core::{Millis, SegmentId};
use loro::{ExportMode, LoroDoc};

use super::{ChannelState, Created, Held, live_segments, push_segment_entry, read_messages};
use crate::delta::{self, Arrival};
use crate::error::MurmureError;
use crate::schema::{self, DocumentKind, Root, SegmentKey, SegmentState};
use crate::session::Session;

/// The messages past which an active segment asks to close: the segment size
/// row of `ambre-spec-budgets.md section 6`.
pub const SEGMENT_MESSAGES: usize = 5000;

/// The snapshot bytes past which an active segment asks to close, whatever
/// its message count: the 2 MB bound of the same row (`MURMURE-481`).
pub const SEGMENT_BYTES: usize = 2 * 1024 * 1024;

/// The delay before a successor that failed to open is fetched again,
/// doubled at each failure: the row of `ambre-spec-budgets.md section 8`.
pub const SUCCESSOR_RETRY_MS: Millis = Millis(1_000);

/// The failures past which a successor is no longer fetched again.
const SUCCESSOR_FAILURES: u32 = 5;

/// A successor that failed to open, and when to fetch it again
/// (`MURMURE-482`).
#[derive(Debug)]
pub(super) struct Retry {
    segment: SegmentId,
    failures: u32,
    /// Stamped by the first tick past the failure: the engine reads no clock.
    at: Option<Timestamp>,
}

/// Who registered the segment a close moves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Successor {
    /// Another member won the race: the successor is fetched as they wrote
    /// it.
    Fetched,
    /// This client registered it: the channel holds it at once.
    Created,
}

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

    /// This channel, with segments that ask to close past `bytes` of
    /// snapshot.
    #[must_use]
    pub fn with_segment_bytes(mut self, bytes: usize) -> Self {
        self.segment_bytes = bytes;
        self
    }

    /// The active segment to close, once it holds the segment size in
    /// messages (`MURMURE-064`) or in bytes (`MURMURE-481`). It is answered
    /// once per active segment, until the relay answers or refuses. After a
    /// rotation it is answered whatever the segment holds (`MURMURE-060`).
    ///
    /// `None` while a segment is still closing: the relay refuses a second
    /// close then (`SRV-196`), and the finalization frees the slot. A closing
    /// segment `meta` lists but the channel opened without counts too
    /// (`MURMURE-503`).
    pub fn wants_close(&mut self) -> Option<SegmentId> {
        if self.close_asked || self.closing.is_some() || self.successor.is_some() {
            return None;
        }
        let messages = crate::schema::root_list(&self.active.doc, Root::Messages).len();
        if !self.key_rotated && messages < self.segment_size && !self.passes_bytes(messages) {
            return None;
        }
        let listed = live_segments(&self.meta.doc, self.channel_id).ok();
        if listed.and_then(|live| live.closing).is_some() {
            return None;
        }
        self.close_asked = true;
        Some(self.active_id())
    }

    /// Whether the snapshot of the active segment passes the byte bound.
    ///
    /// An export costs milliseconds at 5000 messages, so it is not paid at
    /// each send. Each probe spaces the next by half the messages that would
    /// fill the bound at the mean size seen so far.
    fn passes_bytes(&mut self, messages: usize) -> bool {
        if messages < self.active.probe_at {
            return false;
        }
        let Ok(snapshot) = self.active.doc.export(ExportMode::Snapshot) else {
            return false;
        };
        let bytes = snapshot.len();
        if bytes >= self.segment_bytes {
            return true;
        }
        let per_message = (bytes / messages.max(1)).max(1);
        let step = ((self.segment_bytes - bytes) / per_message / 2).max(1);
        self.active.probe_at = messages + step;
        false
    }

    /// The relay refused the close, or never answered: the next send asks
    /// again.
    pub fn close_refused(&mut self) {
        self.close_asked = false;
    }

    /// A rotation retired the channel key: the active segment closes at the
    /// next ask, so its successor opens under the new key (`MURMURE-060`).
    pub fn key_rotated(&mut self) {
        self.key_rotated = true;
    }

    /// Whether a rotation still owes the close of the active segment.
    #[must_use]
    pub fn owes_rotation_close(&self) -> bool {
        self.key_rotated
    }

    /// Write the relay's answer to a close of `closing` into `meta`: the
    /// segment is `closing`, and `active` follows it (`MURMURE-065`).
    ///
    /// A `Created` successor says this client registered `active`. The
    /// channel then holds it at once, and writes its header under its PeerID
    /// once the session lands. A `Fetched` one lost the race, and is fetched
    /// as another member wrote it. No message moves: each stays in
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
        successor: Successor,
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
        self.key_rotated = false;
        if successor == Successor::Fetched {
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
        peer: ambre_core::PeerId,
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

    /// Forget the successor that could not open. A later tick fetches it
    /// again (`MURMURE-482`).
    pub(super) fn forget_successor(&mut self, segment: SegmentId) {
        if self.successor == Some(segment) {
            self.successor = None;
            self.successor_session = None;
            self.successor_failed(segment);
        }
    }

    /// Count one more failure of `segment`, timed by the next tick.
    pub(super) fn successor_failed(&mut self, segment: SegmentId) {
        match &mut self.successor_retry {
            Some(retry) if retry.segment == segment => {
                retry.failures += 1;
                retry.at = None;
            }
            _ => {
                self.successor_retry = Some(Retry {
                    segment,
                    failures: 1,
                    at: None,
                });
            }
        }
    }

    /// The successor that failed to open, to fetch again at `now`
    /// (`MURMURE-482`). Each failure doubles the delay from
    /// [`SUCCESSOR_RETRY_MS`], and the fifth gives up. `None` while a
    /// successor is on its way, or before the delay ends.
    ///
    /// Messages still go to the active segment meanwhile, never to the
    /// successor that did not open.
    pub fn successor_to_retry(&mut self, now: Timestamp) -> Option<SegmentId> {
        if self.successor.is_some() || self.closing.is_some() {
            return None;
        }
        let retry = self.successor_retry.as_mut()?;
        if retry.failures >= SUCCESSOR_FAILURES {
            return None;
        }
        let Some(at) = retry.at else {
            let delay = SUCCESSOR_RETRY_MS.doubled(retry.failures - 1);
            retry.at = Some(now.plus(delay));
            return None;
        };
        if now < at {
            return None;
        }
        let segment = retry.segment;
        self.await_successor(segment)
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
            delta::import_arrivals(doc, arrivals, document_id, delta::Rules::Segment)
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
        self.key_rotated = false;
        self.successor_retry = None;
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
    let list = crate::schema::root_list(meta, Root::Segments);
    let mut listed = false;
    for index in 0..list.len() {
        let Some(entry) = list.get(index).and_then(crate::schema::fields_of) else {
            continue;
        };
        let id = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| SegmentId::parse_str(&s).ok());
        if id == Some(closing) {
            entry.insert(SegmentKey::State, SegmentState::Closing.as_str())?;
        } else if id == Some(active) {
            listed = true;
        }
    }
    if !listed {
        push_segment_entry(meta, active, now, SegmentState::Active)?;
    }
    Ok(())
}
