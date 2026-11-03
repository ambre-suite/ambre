//! The fold of the amendments of a final segment
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! Once the amendments of a final segment waited out the fold delay, a member
//! folds them into a new shallow snapshot of the segment (`MURMURE-075`). The
//! relay arbitrates it with a lease. Another member checks the fold by folding
//! its own copy, and its confirmation erases the amendments once the fold
//! holds them all (`MURMURE-076`). `meta` then marks them folded, and every
//! member folds its own copy the same way. A change the relay never
//! sequenced comes back as a replay, so nothing is dropped (`MURMURE-077`).
//! The engine reads no clock: each delay is stamped by the next tick.

use ambre_bus::{Authored, FoldAnswer, Instant, MessageView, MurmureEvent, SnapshotVerdict};
use ambre_core::id::{Channel, Document, Id, Lease, Segment};
use ambre_core::{DeltaBytes, Failures, Ms, Payload, Relay, Seq, SnapshotBytes, Tally, TimeSpan};
use loro::{ExportMode, LoroDoc, UpdateOptions, VersionVector};

use super::amending::{entry_content, folded_amendments, segment_entry};
use super::finalizing::{FINALIZE_RETRY, RETRY_DOUBLINGS};
use super::relocating::replay_of;
use super::{ChannelState, Replay, content_of};
use crate::error::MurmureError;
use crate::schema::{self, AmendmentKey, HeaderKey, Mark, MessageKey, Root, SegmentKey};
use crate::sequence::Sequence;
use crate::{mention, style};

/// The delay before a fold lease is asked again once a fold left: the
/// `FOLD_DELAY` row of `ambre-spec-budgets.md section 8`, which the relay
/// holds. The relay answers sooner when a fold is owed earlier.
pub const FOLD_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(7 * 24 * 60 * 60 * 1000);

/// The fold of one final segment, as this client drives it.
#[derive(Debug)]
pub(super) struct Folding {
    segment: Id<Segment>,
    step: Step,
    /// Failed lease requests and commits since the last answer that granted
    /// or deferred the lease.
    failures: Tally<Failures>,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Ask for the lease `after` milliseconds past the tick that stamps `at`.
    Due {
        after: TimeSpan<Ms>,
        at: Option<Instant<Authored>>,
    },
    /// A lease request or a commit is on its way.
    Asked,
    /// This client holds the lease until `until`, stamped by the next tick.
    Held {
        grant: Grant,
        until: Option<Instant<Authored>>,
    },
}

/// What the relay granted with a fold lease.
#[derive(Debug, Clone, Copy)]
struct Grant {
    lease: Id<Lease>,
    horizon: Seq<Relay>,
    amendments: Id<Document>,
    amendments_seq: Seq<Relay>,
    expires_in: TimeSpan<Ms>,
}

/// A fold snapshot to upload under a lease (`MURMURE-075`).
#[derive(Debug)]
pub struct FoldSnapshot {
    /// The segment it folds into.
    pub segment: Id<Segment>,
    /// The lease the commit names.
    pub lease: Id<Lease>,
    /// The last delta of the segment it covers, the one the relay returned.
    pub horizon: Seq<Relay>,
    /// The amendments document it folds in.
    pub amendments: Id<Document>,
    /// The last delta number of those amendments it holds.
    pub amendments_seq: Seq<Relay>,
    /// The shallow snapshot, framed as every channel snapshot is.
    pub payload: Payload<SnapshotBytes>,
}

/// What a fold `meta` marks did to this copy.
#[derive(Debug)]
pub struct Settled {
    /// The segment the amendments folded into.
    pub segment: Id<Segment>,
    /// The amendments document it no longer holds.
    pub amendments: Id<Document>,
    /// One per change of this client the relay never sequenced, to write
    /// again as an amendment (`MURMURE-077`).
    pub replay: Vec<Replay>,
    /// The segment's messages, read as the fold left them.
    pub event: MurmureEvent,
}

impl ChannelState {
    /// The final segment whose amendments this client folds: the first one,
    /// in list order, that it holds with the amendments the relay linked.
    fn fold_candidate(&self) -> Option<Id<Segment>> {
        self.final_segments().into_iter().find(|segment| {
            self.loaded.contains_key(segment)
                && self
                    .amendments
                    .get(segment)
                    .is_some_and(|held| held.created.is_none())
        })
    }

    /// The step of the segment this client folds now, started at its first
    /// read. `None` when no held segment has amendments.
    fn fold_step(&mut self) -> Option<&mut Folding> {
        let segment = self.fold_candidate()?;
        if self.folding.as_ref().map(|f| f.segment) != Some(segment) {
            self.folding = Some(Folding {
                segment,
                step: Step::Due {
                    after: TimeSpan::<Ms>::new(0),
                    at: None,
                },
                failures: Tally::<Failures>::ZERO,
            });
        }
        self.folding.as_mut()
    }

    /// The step of `segment`, `None` unless this client folds it now.
    fn fold_step_of(&mut self, segment: Id<Segment>) -> Option<&mut Folding> {
        self.fold_step()
            .filter(|folding| folding.segment == segment)
    }

    /// The final segment whose fold lease to ask for at `now`. The first
    /// tick asks at once: the relay answers how long the fold delay still
    /// runs (`SRV-206`).
    pub fn fold_lease_to_ask(&mut self, now: Instant<Authored>) -> Option<Id<Segment>> {
        let folding = self.fold_step()?;
        match &mut folding.step {
            Step::Due { after, at } => {
                let at = *at.get_or_insert(now.plus(*after));
                if now < at {
                    return None;
                }
            }
            // A lease that ran out before its snapshot left is asked again.
            Step::Held { until, .. } if until.is_some_and(|until| now >= until) => {}
            Step::Held { .. } | Step::Asked => return None,
        }
        folding.step = Step::Asked;
        Some(folding.segment)
    }

    /// The relay answered the fold lease request for `segment`.
    pub fn fold_answered(&mut self, segment: Id<Segment>, answer: FoldAnswer) {
        let Some(folding) = self.fold_step_of(segment) else {
            return;
        };
        folding.failures = Tally::<Failures>::ZERO;
        folding.step = match answer {
            FoldAnswer::Granted {
                lease,
                horizon,
                amendments,
                amendments_seq,
                expires_in,
            } => Step::Held {
                grant: Grant {
                    lease,
                    horizon,
                    amendments,
                    amendments_seq,
                    expires_in,
                },
                until: None,
            },
            FoldAnswer::Deferred { retry_in } => Step::Due {
                after: retry_in,
                at: None,
            },
        };
    }

    /// The relay took the fold of `segment`. The next one is asked past the
    /// fold delay; another member's check decides this one.
    pub fn fold_committed(&mut self, segment: Id<Segment>) {
        if let Some(folding) = self.fold_step_of(segment) {
            folding.failures = Tally::<Failures>::ZERO;
            folding.step = Step::Due {
                after: FOLD_DELAY,
                at: None,
            };
        }
    }

    /// A lease request or a commit of `segment` failed, or the relay names
    /// other amendments than this copy holds. It is asked again past
    /// [`FINALIZE_RETRY`], doubled at each failure.
    pub fn fold_failed(&mut self, segment: Id<Segment>) {
        if let Some(folding) = self.fold_step_of(segment) {
            let doublings = folding.failures.min(RETRY_DOUBLINGS);
            folding.failures = folding.failures.incremented();
            folding.step = Step::Due {
                after: FINALIZE_RETRY.doubled(doublings),
                at: None,
            };
        }
    }

    /// The fold snapshot of the segment this client folds, once it holds the
    /// lease and the amendments up to the number the relay returned.
    ///
    /// It folds the amendments as the relay holds them, so a change of this
    /// client still on its way stays out. It is shallow, and its header names
    /// the amendments and how far it reaches.
    ///
    /// Stamps the end of the lease at the first call past the grant.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the fold or the export.
    pub fn fold_snapshot(
        &mut self,
        now: Instant<Authored>,
    ) -> Result<Option<FoldSnapshot>, MurmureError> {
        let Some(folding) = self.fold_step() else {
            return Ok(None);
        };
        let segment = folding.segment;
        let Step::Held { grant, until } = &mut folding.step else {
            return Ok(None);
        };
        let grant = *grant;
        let until = *until.get_or_insert(Instant::<Authored>::new(
            now.get().saturating_add(grant.expires_in.get()),
        ));
        let held = self.amendments.get(&segment).map(|held| held.id);
        if held != Some(grant.amendments) {
            self.fold_failed(segment);
            return Ok(None);
        }
        let reached = self.delivered(grant.amendments);
        if reached < grant.amendments_seq || now >= until {
            return Ok(None);
        }
        if let Some(folding) = self.folding.as_mut() {
            folding.step = Step::Asked;
        }
        let doc = self.folded(segment, grant.amendments, reached)?;
        let shallow = Payload::<SnapshotBytes>::new(
            doc.export(ExportMode::shallow_snapshot(&doc.oplog_frontiers()))?,
        );
        let payload = match self.sequences.get(&segment.document_id()) {
            Some(sequence) => sequence.frame(shallow),
            None => shallow,
        };
        Ok(Some(FoldSnapshot {
            segment,
            lease: grant.lease,
            horizon: grant.horizon,
            amendments: grant.amendments,
            amendments_seq: reached,
            payload,
        }))
    }

    /// A copy of `segment` with `amendments` folded in, as the relay holds
    /// them up to `reached`. This copy stays as it is.
    fn folded(
        &self,
        segment: Id<Segment>,
        amendments: Id<Document>,
        reached: Seq<Relay>,
    ) -> Result<LoroDoc, MurmureError> {
        let document_id = segment.document_id();
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let fork = doc.fork();
        let relayed = self.relayed_amendments(segment, amendments)?;
        fold_into(&fork, &relayed, self.channel_id, amendments, reached)?;
        Ok(fork)
    }

    /// The held `amendments` of `segment` as the relay holds them: every
    /// operation it delivered, and none of this client's still on its way.
    fn relayed_amendments(
        &self,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) -> Result<LoroDoc, MurmureError> {
        let held = self
            .amendments
            .get(&segment)
            .filter(|held| held.id == amendments)
            .ok_or(MurmureError::NoAmendments(segment))?;
        let empty = VersionVector::new();
        let relayed = self
            .sequences
            .get(&amendments)
            .map_or(&empty, Sequence::relayed);
        let own = held.doc.oplog_vv();
        let mut kept = VersionVector::new();
        for (&peer, &end) in own.iter() {
            let delivered = relayed.get(&peer).copied().unwrap_or(0).min(end);
            if delivered > 0 {
                kept.insert(peer, delivered);
            }
        }
        Ok(held.doc.fork_at(&held.doc.vv_to_frontiers(&kept))?)
    }

    /// Check `theirs`, a snapshot of `document_id` whose header names the
    /// `amendments` it folds up to `seq` (`MURMURE-075`).
    ///
    /// It matches when it holds the state this copy reaches by the same
    /// fold. Unchecked when this copy holds other amendments, or another
    /// number of them.
    pub(super) fn check_fold(
        &self,
        document_id: Id<Document>,
        theirs: &LoroDoc,
        amendments: Id<Document>,
        seq: Seq<Relay>,
    ) -> SnapshotVerdict {
        let segment = Id::<Segment>::from_document(document_id);
        let held = self.amendments.get(&segment).map(|held| held.id);
        if held != Some(amendments) || self.delivered(amendments) != seq {
            return SnapshotVerdict::Unchecked;
        }
        match self.folded(segment, amendments, seq) {
            Ok(ours) if ours.get_deep_value() == theirs.get_deep_value() => {
                SnapshotVerdict::Match { extras: Vec::new() }
            }
            Ok(_) => SnapshotVerdict::Mismatch,
            Err(_) => SnapshotVerdict::Unchecked,
        }
    }

    /// The relay erased `amendments` once a fold holding them all was
    /// confirmed (`MURMURE-076`): `meta` marks them folded in the entry of
    /// `segment`. Returns the `meta` delta to send, `None` when it marks
    /// them already.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when `meta` lists no such segment,
    /// [`MurmureError::NoSession`] before the PeerID of `meta` landed, and
    /// [`MurmureError::Crdt`] when Loro refuses the write or the export.
    pub fn mark_folded(
        &mut self,
        segment: Id<Segment>,
        amendments: Id<Document>,
    ) -> Result<Option<Payload<DeltaBytes>>, MurmureError> {
        if folded_amendments(&self.meta.doc, segment) == Some(amendments) {
            return Ok(None);
        }
        let entry = segment_entry(&self.meta.doc, segment)
            .ok_or(MurmureError::NotHeld(segment.document_id()))?;
        let (doc, session) = self.meta.session()?;
        entry.insert(SegmentKey::Folded, amendments.to_string())?;
        Ok(Some(session.export(doc)?))
    }

    /// The relay no longer holds `amendments`, which this copy holds linked
    /// to a segment: a delta to them was refused as not found. Short of a
    /// channel deletion, only a confirmed fold erases linked amendments. So
    /// `meta` marks them folded as [`ChannelState::mark_folded`] does
    /// (`MURMURE-483`).
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] when this copy holds no such linked
    /// amendments, and every refusal of [`ChannelState::mark_folded`].
    pub fn amendments_gone(
        &mut self,
        amendments: Id<Document>,
    ) -> Result<Option<Payload<DeltaBytes>>, MurmureError> {
        let segment = self
            .amendments
            .iter()
            .find(|(_, held)| held.id == amendments && held.created.is_none())
            .map(|(segment, _)| *segment)
            .ok_or(MurmureError::NotHeld(amendments))?;
        self.mark_folded(segment, amendments)
    }

    /// Fold every held amendments document `meta` marks folded into its
    /// segment, as the relay held it, and drop it.
    ///
    /// The segment then reads as the fold snapshot does. A change of this
    /// client the relay never sequenced comes back as a [`Replay`], so it is
    /// written again in a new amendments document (`MURMURE-077`).
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the fold.
    pub fn settle_folds(&mut self) -> Result<Vec<Settled>, MurmureError> {
        let marked: Vec<(Id<Segment>, Id<Document>)> = self
            .amendments
            .iter()
            .map(|(segment, held)| (*segment, held.id))
            .filter(|(segment, id)| folded_amendments(&self.meta.doc, *segment) == Some(*id))
            .collect();
        self.amendments_pending
            .retain(|id, pending| folded_amendments(&self.meta.doc, pending.segment) != Some(*id));
        let mut settled = Vec::with_capacity(marked.len());
        for (segment, amendments) in marked {
            let Some(read) = self.read_segment(segment) else {
                continue;
            };
            let reached = self.delivered(amendments);
            let relayed = self.relayed_amendments(segment, amendments)?;
            if let Some(doc) = self.doc(segment.document_id()) {
                fold_into(doc, &relayed, self.channel_id, amendments, reached)?;
            }
            self.amendments.remove(&segment);
            self.sequences.remove(&amendments);
            if self.folding.as_ref().is_some_and(|f| f.segment == segment) {
                self.folding = None;
            }
            let messages = self.read_segment(segment).unwrap_or_default();
            let replay = unsequenced(segment, &messages, read);
            settled.push(Settled {
                segment,
                amendments,
                replay,
                event: MurmureEvent::Messages { segment, messages },
            });
        }
        Ok(settled)
    }
}

#[cfg(test)]
impl ChannelState {
    /// Fold the held amendments of `segment` into this copy, as
    /// [`ChannelState::settle_folds`] does, and keep them held. The schema
    /// belts walk the segment and its amendments side by side.
    pub(crate) fn fold_in_place(&self, segment: Id<Segment>) -> Result<(), MurmureError> {
        let amendments = self
            .amendments
            .get(&segment)
            .ok_or(MurmureError::NoAmendments(segment))?;
        let doc = self
            .doc(segment.document_id())
            .ok_or(MurmureError::NotHeld(segment.document_id()))?;
        let reached = self.delivered(amendments.id);
        fold_into(
            doc,
            &amendments.doc,
            self.channel_id,
            amendments.id,
            reached,
        )
    }
}

/// The changes `read` shows that `folded` lacks: one per message, in segment
/// order.
fn unsequenced(
    segment: Id<Segment>,
    folded: &[MessageView],
    read: Vec<MessageView>,
) -> Vec<Replay> {
    read.into_iter()
        .filter_map(|now| {
            let before = folded.iter().find(|view| view.id == now.id)?;
            replay_of(segment, before, now)
        })
        .collect()
}

/// The amendments document and the number a snapshot header names, `None`
/// for a snapshot that folds nothing.
pub(crate) fn header_fold(doc: &LoroDoc) -> Option<(Id<Document>, Seq<Relay>)> {
    let header = crate::schema::header(doc);
    let amendments =
        Id::<Document>::parse_str(&schema::string(&header, HeaderKey::Folded)?).ok()?;
    Some((
        amendments,
        Seq::<Relay>::new(schema::unsigned(&header, HeaderKey::FoldedSeq)?),
    ))
}

/// Fold `amendments` into the messages of `segment`, as a reader reads them
/// over it. A delete empties the body and marks the message deleted, and an
/// edit replaces the body and marks it edited. A message deleted in its
/// segment stays as it is. A thread the amendments record lands in the
/// root's empty `thread` field. The header names the amendments and `reached`.
///
/// Two clients folding the same amendments into the same state reach the
/// same state.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
fn fold_into(
    segment: &LoroDoc,
    amendments: &LoroDoc,
    channel: Id<Channel>,
    amendments_id: Id<Document>,
    reached: Seq<Relay>,
) -> Result<(), MurmureError> {
    let root = crate::schema::root_map(amendments, Root::Amendments);
    let threads = crate::schema::root_map(amendments, Root::Threads);
    let list = crate::schema::root_list(segment, Root::Messages);
    for index in 0..list.len() {
        let Some(message) = list.get(index).and_then(crate::schema::fields_of) else {
            continue;
        };
        let Some(id) = schema::string(&message, MessageKey::Id) else {
            continue;
        };
        // A deleted root keeps its thread readable (`MURMURE-484`).
        if schema::string(&message, MessageKey::Thread).is_none()
            && let Some(thread) = schema::string_value(threads.get(&id))
        {
            message.insert(MessageKey::Thread, thread)?;
        }
        if schema::flag(&message, MessageKey::Deleted) {
            continue;
        }
        let Some(entry) = root.get(&id).and_then(crate::schema::fields_of) else {
            continue;
        };
        let Some(content) = content_of(&message) else {
            continue;
        };
        let len = content.len_unicode();
        if schema::flag(&entry, AmendmentKey::Deleted) {
            if len > 0 {
                content.delete(0, len)?;
            }
            message.insert(MessageKey::Deleted, true)?;
        } else if let Some(edited) = entry_content(&entry) {
            let mentions = mention::read(&edited, channel);
            let styles = style::read(&edited);
            if len > 0 {
                content.unmark(0..len, Mark::Mention.as_str())?;
            }
            style::clear(&content)?;
            content
                .update(&edited.to_string(), UpdateOptions::default())
                .map_err(|error| MurmureError::Crdt(error.to_string()))?;
            mention::write(segment, &content, &mentions, channel)?;
            style::write(segment, &content, &styles)?;
            message.insert(MessageKey::Edited, true)?;
        }
    }
    let header = crate::schema::header(segment);
    header.insert(HeaderKey::Folded, amendments_id.to_string())?;
    header.insert(
        HeaderKey::FoldedSeq,
        i64::try_from(reached.get()).unwrap_or(i64::MAX),
    )?;
    Ok(())
}
