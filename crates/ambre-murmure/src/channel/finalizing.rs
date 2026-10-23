//! The finalization of the closing segment
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! The relay arbitrates it: it waits out the grace period, locks the segment
//! and grants one lease at a time. So a client asks, and waits as long as
//! the relay says. The lease holder seals a shallow snapshot of the segment
//! once it holds every delta up to the horizon. The engine reads no clock:
//! each delay is stamped by the next tick.

use ambre_bus::Timestamp;
use ambre_core::{DeltaBytes, LeaseId, Millis, SegmentId, Seq, SnapshotBytes};
use loro::{ExportMode, LoroDoc};

use super::ChannelState;
use crate::error::MurmureError;
use crate::schema::{self, Root, SegmentKey, SegmentState};

/// The delay before a failed lease request or commit is asked again,
/// doubled at each failure: the row of `ambre-spec-budgets.md section 8`.
pub const FINALIZE_RETRY_MS: Millis = Millis(2_000);

/// The doublings past which the delay stops growing: about half an hour.
pub(super) const RETRY_DOUBLINGS: u32 = 10;

/// The finalization of one closing segment, as this client drives it.
#[derive(Debug)]
pub(super) struct Finalizing {
    segment: SegmentId,
    step: Step,
    /// Failed lease requests and commits since the last answer that granted
    /// or deferred the lease.
    failures: u32,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Ask for the lease `after` milliseconds past the tick that stamps `at`.
    Due {
        after: Millis,
        at: Option<Timestamp>,
    },
    /// A lease request or a commit is on its way.
    Asked,
    /// This client holds the lease until `until`, stamped by the next tick.
    /// The snapshot leaves once the deltas up to `horizon` are in.
    Held {
        lease: LeaseId,
        horizon: Seq,
        expires_in: ambre_core::Millis,
        until: Option<Timestamp>,
    },
    /// This copy holds an edit the relay refused for the lock, so it is not
    /// the relay's. Another member finalizes the segment.
    Withdrawn,
}

/// A final snapshot to upload under a lease (`MURMURE-068`).
#[derive(Debug)]
pub struct FinalSnapshot {
    /// The segment it finalizes.
    pub segment: SegmentId,
    /// The lease the commit names.
    pub lease: LeaseId,
    /// The last delta it covers, the one the relay returned.
    pub horizon: Seq,
    /// The shallow snapshot, framed as every channel snapshot is.
    pub payload: SnapshotBytes,
}

impl ChannelState {
    /// The segment this client finalizes now: a final one whose snapshot a
    /// report dropped, or else the closing one.
    pub fn finalizing_id(&self) -> Option<SegmentId> {
        self.refinalizing.or_else(|| self.closing_id())
    }

    /// A report dropped the final snapshot of `segment`: finalize it again,
    /// at the horizon the relay already holds (`MURMURE-238`). `false` for a
    /// segment this copy does not hold as a final one: another member does it.
    pub fn refinalize(&mut self, segment: SegmentId) -> bool {
        let held = self.loaded.contains_key(&segment) && self.final_segments().contains(&segment);
        if held && self.refinalizing.is_none() {
            self.refinalizing = Some(segment);
        }
        self.refinalizing == Some(segment)
    }

    /// The step of the segment `segment`, started at its first read. `None`
    /// for a segment this client does not finalize now.
    fn finalize_step(&mut self, segment: SegmentId) -> Option<&mut Finalizing> {
        if self.finalizing_id() != Some(segment) {
            return None;
        }
        if self.finalizing.as_ref().map(|f| f.segment) != Some(segment) {
            self.finalizing = Some(Finalizing {
                segment,
                step: Step::Due {
                    after: Millis(0),
                    at: None,
                },
                failures: 0,
            });
        }
        self.finalizing.as_mut()
    }

    /// The closing segment whose finalize lease to ask for at `now`
    /// (`MURMURE-067`). The first tick past the close asks at once: the
    /// relay answers how long the grace period still runs (`SRV-199`).
    pub fn lease_to_ask(&mut self, now: Timestamp) -> Option<SegmentId> {
        let segment = self.finalizing_id()?;
        let finalizing = self.finalize_step(segment)?;
        match &mut finalizing.step {
            Step::Due { after, at } => {
                let at = *at.get_or_insert(now.plus(*after));
                if now < at {
                    return None;
                }
            }
            // A lease that ran out before its snapshot left is asked again.
            Step::Held { until, .. } if until.is_some_and(|until| now >= until) => {}
            Step::Held { .. } | Step::Asked | Step::Withdrawn => return None,
        }
        finalizing.step = Step::Asked;
        Some(segment)
    }

    /// The relay deferred the lease of `segment` by `retry_in` milliseconds:
    /// the grace period runs, or another member holds the lease.
    pub fn lease_deferred(&mut self, segment: SegmentId, retry_in: ambre_core::Millis) {
        if let Some(finalizing) = self.finalize_step(segment) {
            finalizing.failures = 0;
            finalizing.step = Step::Due {
                after: retry_in,
                at: None,
            };
        }
    }

    /// The relay granted this client the lease of `segment`, for
    /// `expires_in` milliseconds. Its snapshot covers the deltas up to
    /// `horizon`.
    pub fn lease_granted(
        &mut self,
        segment: SegmentId,
        lease: LeaseId,
        horizon: Seq,
        expires_in: ambre_core::Millis,
    ) {
        if let Some(finalizing) = self.finalize_step(segment) {
            finalizing.failures = 0;
            finalizing.step = Step::Held {
                lease,
                horizon,
                expires_in,
                until: None,
            };
        }
    }

    /// A lease request or a commit of `segment` failed. It is asked again
    /// past [`FINALIZE_RETRY_MS`], doubled at each failure.
    pub fn finalize_failed(&mut self, segment: SegmentId) {
        if let Some(finalizing) = self.finalize_step(segment) {
            if matches!(finalizing.step, Step::Withdrawn) {
                return;
            }
            let doublings = finalizing.failures.min(RETRY_DOUBLINGS);
            finalizing.failures = finalizing.failures.saturating_add(1);
            finalizing.step = Step::Due {
                after: FINALIZE_RETRY_MS.doubled(doublings),
                at: None,
            };
        }
    }

    /// This copy of `segment` holds an edit the relay refused: it never
    /// finalizes the segment, and leaves it to another member. A segment
    /// finalized again is withdrawn once another member finalized it first.
    pub fn finalize_withdrawn(&mut self, segment: SegmentId) {
        if self.refinalizing == Some(segment) {
            self.refinalizing = None;
            self.finalizing = None;
            return;
        }
        if let Some(finalizing) = self.finalize_step(segment) {
            finalizing.step = Step::Withdrawn;
        }
    }

    /// The final snapshot of [`ChannelState::finalizing_id`], once this client
    /// holds its lease and every delta up to the horizon. `covered` is the
    /// last delta of that segment it holds with no gap below it.
    ///
    /// The snapshot is shallow: it holds the state, and no text deleted
    /// before it (`MURMURE-063`). Its messages read as in the segment
    /// (`MURMURE-070`). The relay refused every delta past the horizon, so
    /// the state at `covered` is the state at the horizon.
    ///
    /// Stamps the end of the lease at the first call past the grant.
    ///
    /// # Errors
    /// [`MurmureError::Crdt`] when Loro refuses the export.
    pub fn final_snapshot(
        &mut self,
        now: Timestamp,
        covered: Seq,
    ) -> Result<Option<FinalSnapshot>, MurmureError> {
        let Some(segment) = self.finalizing_id() else {
            return Ok(None);
        };
        let Some(finalizing) = self.finalize_step(segment) else {
            return Ok(None);
        };
        let Step::Held {
            lease,
            horizon,
            expires_in,
            until,
        } = &mut finalizing.step
        else {
            return Ok(None);
        };
        let (lease, horizon) = (*lease, *horizon);
        let until = *until.get_or_insert(Timestamp(now.0.saturating_add(expires_in.0)));
        if covered < horizon || now >= until {
            return Ok(None);
        }
        finalizing.step = Step::Asked;
        let document_id = segment.document_id();
        let doc = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        let shallow =
            SnapshotBytes::new(doc.export(ExportMode::shallow_snapshot(&doc.oplog_frontiers()))?);
        let payload = match self.sequences.get(&document_id) {
            Some(sequence) => sequence.frame(shallow),
            None => shallow,
        };
        Ok(Some(FinalSnapshot {
            segment,
            lease,
            horizon,
            payload,
        }))
    }

    /// The relay accepted the final snapshot of `segment`: mark it `final` in
    /// `meta` (`MURMURE-069`). Returns the `meta` delta to send.
    ///
    /// The segment is held as a final one from here, with every message it
    /// read before (`MURMURE-070`). The closing slot is free for the next
    /// close. A segment finalized again is final in `meta` already, so
    /// nothing is sent (`MURMURE-238`).
    ///
    /// # Errors
    /// - [`MurmureError::NotHeld`]: this client does not finalize `segment`.
    /// - [`MurmureError::NoSession`]: the PeerID of `meta` has not landed.
    /// - [`MurmureError::Crdt`]: Loro refused a write.
    pub fn mark_final(&mut self, segment: SegmentId) -> Result<Option<DeltaBytes>, MurmureError> {
        if self.refinalizing == Some(segment) {
            self.refinalizing = None;
            self.finalizing = None;
            return Ok(None);
        }
        if self.closing_id() != Some(segment) {
            return Err(MurmureError::NotHeld(segment.document_id()));
        }
        let (doc, session) = self.meta.session()?;
        record_final(doc, segment)?;
        let meta = session.export(doc)?;
        self.hold_final();
        Ok(Some(meta))
    }

    /// The closing segment another member finalized, once `meta` names it
    /// `final`. It is held as a final one from here, and the closing slot is
    /// free. `None` while `meta` still names it closing.
    pub fn settle_final(&mut self) -> Option<SegmentId> {
        let segment = self.closing_id()?;
        let state = super::segment_entries(&self.meta.doc)
            .into_iter()
            .find(|(id, _)| *id == segment)
            .map(|(_, state)| state);
        if state != Some(SegmentState::Final) {
            return None;
        }
        self.hold_final();
        Some(segment)
    }

    /// Move the closing segment among the final ones. A segment finalized
    /// again keeps its step.
    fn hold_final(&mut self) {
        let closing = self.closing_id();
        if self.finalizing.as_ref().map(|f| f.segment) == closing {
            self.finalizing = None;
        }
        if let Some(held) = self.closing.take() {
            self.loaded
                .insert(SegmentId::from_document(held.id), held.doc);
        }
    }
}

/// Mark every entry of `segment` final. Two clients writing it write the
/// same state.
///
/// # Errors
/// [`MurmureError::Crdt`] when Loro refuses a write.
fn record_final(meta: &LoroDoc, segment: SegmentId) -> Result<(), MurmureError> {
    let list = crate::schema::root_list(meta, Root::Segments);
    for index in 0..list.len() {
        let Some(entry) = list.get(index).and_then(crate::schema::fields_of) else {
            continue;
        };
        let id = schema::string(&entry, SegmentKey::Document)
            .and_then(|s| SegmentId::parse_str(&s).ok());
        if id == Some(segment) {
            entry.insert(SegmentKey::State, SegmentState::Final.as_str())?;
        }
    }
    Ok(())
}
