//! The check of a segment snapshot another member built
//! (`features/ambre-spec-murmure-segments.md section 3`).
//!
//! A final snapshot is shallow: it holds the state and none of the history,
//! so the operation check of the document plane cannot read it. A member
//! holding every delta it covers compares the two states instead. The check
//! reads this copy and never writes it (`MURMURE-153`).

use ambre_bus::SnapshotVerdict;
use ambre_core::id::{Document, Id};
use ambre_core::{LoroPayloadRef, Seq, SnapshotBytes};
use loro::{LoroDoc, VersionVector};

use super::{ChannelState, folding};
use crate::error::MurmureError;
use crate::sequence::{self, Sequence};

impl ChannelState {
    /// Check `snapshot`, a snapshot of `document_id` covering every delta up
    /// to `covered`, against this copy (`MURMURE-071`).
    ///
    /// It matches when it holds the state this copy had at its version. One
    /// holding an operation no delta carried is a mismatch (`MURMURE-012`).
    /// So is one dropping a delta the relay sequenced (`MURMURE-072`).
    /// An edit of this client the relay never sequenced is left out of the
    /// comparison. Unchecked when this copy holds a delta past `covered`.
    ///
    /// # Errors
    /// [`MurmureError::NotHeld`] for a document the channel does not hold.
    pub fn check_snapshot(
        &self,
        document_id: Id<Document>,
        covered: Seq,
        snapshot: &SnapshotBytes,
    ) -> Result<SnapshotVerdict, MurmureError> {
        let own = self
            .doc(document_id)
            .ok_or(MurmureError::NotHeld(document_id))?;
        if self.delivered(document_id) != covered {
            return Ok(SnapshotVerdict::Unchecked);
        }
        let theirs = LoroDoc::new();
        if sequence::import(&theirs, LoroPayloadRef::Snapshot(snapshot)).is_err() {
            return Ok(SnapshotVerdict::Mismatch);
        }
        // A fold holds operations no delta carried: it is checked by folding
        // this copy the same way (`MURMURE-075`).
        if let Some((amendments, seq)) = folding::header_fold(&theirs) {
            return Ok(self.check_fold(document_id, &theirs, amendments, seq));
        }
        let (own_vv, their_vv) = (own.oplog_vv(), theirs.oplog_vv());
        let held = |vv: &VersionVector, peer| vv.get(peer).copied().unwrap_or(0);
        if their_vv
            .iter()
            .any(|(peer, &end)| held(&own_vv, peer) < end)
        {
            return Ok(SnapshotVerdict::Mismatch);
        }
        let empty = VersionVector::new();
        let relayed = self
            .sequences
            .get(&document_id)
            .map_or(&empty, Sequence::relayed);
        if relayed
            .iter()
            .any(|(peer, &end)| held(&their_vv, peer) < end)
        {
            return Ok(SnapshotVerdict::Mismatch);
        }
        let ours = if own_vv == their_vv {
            own.get_deep_value()
        } else {
            match own.fork_at(&own.vv_to_frontiers(&their_vv)) {
                Ok(fork) => fork.get_deep_value(),
                Err(_) => return Ok(SnapshotVerdict::Unchecked),
            }
        };
        Ok(if ours == theirs.get_deep_value() {
            SnapshotVerdict::Match { extras: Vec::new() }
        } else {
            SnapshotVerdict::Mismatch
        })
    }
}
