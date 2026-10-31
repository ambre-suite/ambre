//! The close of the active segment into its successor
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! The relay arbitrates a close, so a client asks it first and writes `meta`
//! with its answer (`MURMURE-011`). Every client that writes the answer
//! writes the same entries, so two writers converge on one successor.

use ambre_bus::{Messages, MurmureEvent, Timestamp};
use ambre_core::id::{Id, Segment};
use ambre_core::{ByteSize, DeltaBytes, Failures, Millis, Payload, PeerEntryBytes, Tally};
use loro::{ExportMode, LoroDoc};

use super::{
    ChannelState, CloseAsk, Created, Held, Rotation, live_segments, push_segment_entry,
    read_messages,
};
use crate::delta::{self, Arrival};
use crate::error::MurmureError;
use crate::schema::{self, DocumentKind, Root, SegmentKey, SegmentState};
use crate::session::Session;

/// The messages past which an active segment asks to close: the segment size
/// row of `ambre-spec-budgets.md section 6`.
pub const SEGMENT_MESSAGES: Tally<Messages> = Tally::<Messages>::new(5000);

/// The snapshot bytes past which an active segment asks to close, whatever
/// its message count: the 2 MB bound of the same row (`MURMURE-481`).
pub const SEGMENT_SNAPSHOT_SIZE: ByteSize = ByteSize::new(2 * 1024 * 1024);

/// The delay before a successor that failed to open is fetched again,
/// doubled at each failure: the row of `ambre-spec-budgets.md section 8`.
pub const SUCCESSOR_RETRY: Millis = Millis::new(1_000);

/// The failures past which a successor is no longer fetched again.
const SUCCESSOR_FAILURES: Tally<Failures> = Tally::<Failures>::new(5);

/// A successor that failed to open, and when to fetch it again
/// (`MURMURE-482`).
#[derive(Debug)]
pub(super) struct Retry {
    segment: Id<Segment>,
    failures: Tally<Failures>,
    /// Stamped by the first tick past the failure: the engine reads no clock.
    at: Option<Timestamp>,
}

pub use ambre_bus::app::murmure::Successor;

/// What writing a close produced: the `meta` delta to send, and the successor
/// to fetch when another client created it.
#[derive(Debug)]
pub struct Closed {
    /// Every operation the close wrote in `meta`, as a Loro update.
    pub meta: Payload<DeltaBytes>,
    /// The successor to open, when this client did not create it. Its bytes
    /// then go to [`ChannelState::apply_remote`], and
    /// [`ChannelState::segment_ready`] admits it once they are all in.
    pub fetch: Option<Id<Segment>>,
}

impl ChannelState {
    /// This channel, with segments that ask to close past `messages`.
    #[must_use]
    pub fn with_segment_size(mut self, messages: Tally<Messages>) -> Self {
        self.segment_size = messages;
        self
    }

    /// This channel, with segments that ask to close past `bytes` of
    /// snapshot.
    #[must_use]
    pub fn with_segment_snapshot_size(mut self, bytes: ByteSize) -> Self {
        self.segment_snapshot_size = bytes;
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
    pub fn wants_close(&mut self) -> Option<Id<Segment>> {
        if self.close_asked == CloseAsk::Asked || self.closing.is_some() || self.successor.is_some()
        {
            return None;
        }
        let messages = Tally::<Messages>::of_len(
            crate::schema::root_list(&self.active.doc, Root::Messages).len(),
        );
        if self.key_rotated == Rotation::Settled
            && messages < self.segment_size
            && !self.passes_bytes(messages)
        {
            return None;
        }
        let listed = live_segments(&self.meta.doc, self.channel_id).ok();
        if listed.and_then(|live| live.closing).is_some() {
            return None;
        }
        self.close_asked = CloseAsk::Asked;
        Some(self.active_id())
    }

    /// Whether the snapshot of the active segment passes the byte bound.
    ///
    /// An export costs milliseconds at 5000 messages, so it is not paid at
    /// each send. Each probe spaces the next by half the messages that would
    /// fill the bound at the mean size seen so far.
    fn passes_bytes(&mut self, messages: Tally<Messages>) -> bool {
        if messages < self.active.probe_at {
            return false;
        }
        let Ok(snapshot) = self.active.doc.export(ExportMode::Snapshot) else {
            return false;
        };
        let bytes = ByteSize::of_len(snapshot.len());
        if bytes >= self.segment_snapshot_size {
            return true;
        }
        let held = usize::try_from(messages.get()).unwrap_or(usize::MAX);
        let per_message = (snapshot.len() / held.max(1)).max(1);
        let room = self.segment_snapshot_size.saturating_sub(bytes).as_len();
        let step = (room / per_message / 2).max(1);
        self.active.probe_at = messages.plus(Tally::<Messages>::of_len(step));
        false
    }

    /// The relay refused the close, or never answered: the next send asks
    /// again.
    pub fn close_refused(&mut self) {
        self.close_asked = CloseAsk::Idle;
    }

    /// A rotation retired the channel key: the active segment closes at the
    /// next ask, so its successor opens under the new key (`MURMURE-060`).
    pub fn key_rotated(&mut self) {
        self.key_rotated = Rotation::OwesClose;
    }

    /// Whether a rotation still owes the close of the active segment.
    #[must_use]
    pub fn owes_rotation_close(&self) -> bool {
        self.key_rotated == Rotation::OwesClose
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
        closing: Id<Segment>,
        active: Id<Segment>,
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
        self.close_asked = CloseAsk::Idle;
        self.key_rotated = Rotation::Settled;
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
    pub fn successor_to_open(&mut self) -> Option<Id<Segment>> {
        let live = live_segments(&self.meta.doc, self.channel_id).ok()?;
        if live.active == self.active_id() || live.closing != Some(self.active_id()) {
            return None;
        }
        self.await_successor(live.active)
    }

    /// Wait for the bytes of `successor`. Messages still go to the active
    /// segment meanwhile, which takes them while it closes.
    fn await_successor(&mut self, successor: Id<Segment>) -> Option<Id<Segment>> {
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
        segment: Id<Segment>,
        peer: ambre_core::PeerId,
        entry: PeerEntryBytes,
    ) -> bool {
        if self.successor != Some(segment) {
            return false;
        }
        self.successor_session = Some((peer, entry));
        true
    }

    /// Whether `segment` is the successor still arriving.
    pub(super) fn is_successor(&self, segment: Id<Segment>) -> bool {
        self.successor == Some(segment)
    }

    /// Forget the successor that could not open. A later tick fetches it
    /// again (`MURMURE-482`).
    pub(super) fn forget_successor(&mut self, segment: Id<Segment>) {
        if self.successor == Some(segment) {
            self.successor = None;
            self.successor_session = None;
            self.successor_failed(segment);
        }
    }

    /// Count one more failure of `segment`, timed by the next tick.
    pub(super) fn successor_failed(&mut self, segment: Id<Segment>) {
        match &mut self.successor_retry {
            Some(retry) if retry.segment == segment => {
                retry.failures = retry.failures.incremented();
                retry.at = None;
            }
            _ => {
                self.successor_retry = Some(Retry {
                    segment,
                    failures: Tally::<Failures>::new(1),
                    at: None,
                });
            }
        }
    }

    /// The successor that failed to open, to fetch again at `now`
    /// (`MURMURE-482`). Each failure doubles the delay from
    /// [`SUCCESSOR_RETRY`], and the fifth gives up. `None` while a
    /// successor is on its way, or before the delay ends.
    ///
    /// Messages still go to the active segment meanwhile, never to the
    /// successor that did not open.
    pub fn successor_to_retry(&mut self, now: Timestamp) -> Option<Id<Segment>> {
        if self.successor.is_some() || self.closing.is_some() {
            return None;
        }
        let retry = self.successor_retry.as_mut()?;
        if retry.failures >= SUCCESSOR_FAILURES {
            return None;
        }
        let Some(at) = retry.at else {
            let delay = SUCCESSOR_RETRY.doubled(retry.failures.minus(Tally::<Failures>::new(1)));
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
        segment: Id<Segment>,
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
        self.close_asked = CloseAsk::Idle;
        self.key_rotated = Rotation::Settled;
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
    closing: Id<Segment>,
    active: Id<Segment>,
    now: Timestamp,
) -> Result<(), MurmureError> {
    let list = crate::schema::root_list(meta, Root::Segments);
    let mut listed = false;
    for index in 0..list.len() {
        let Some(entry) = list.get(index).and_then(crate::schema::fields_of) else {
            continue;
        };
        let id = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| Id::<Segment>::parse_str(&s).ok());
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
