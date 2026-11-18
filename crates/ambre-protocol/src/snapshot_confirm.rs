//! What a confirmation signs (`AUTHOR-031`, `SRV-284`).
//!
//! A member whose deltas match a snapshot confirms it. The confirmer signs
//! the document, the snapshot, the horizon and the snapshot's author. A cold
//! open then trusts a snapshot whole on that signature, and never on the
//! relay's word that another member confirmed it (audit `N31`).

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{FixedBytes, PublicKey, Relay, Seq, Signature};
use serde::{Deserialize, Serialize};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
pub const CONFIRM_DOMAIN: &[u8] = b"ambre-snapshot-confirm-v1";

/// The bytes a confirmer signs: the domain, then the document, the snapshot,
/// the horizon and the author's signing key, each at a fixed length.
#[must_use]
pub fn confirm_transcript(
    document_id: Id<Document>,
    snapshot_id: Id<Snapshot>,
    covered_seq: Seq<Relay>,
    author: &FixedBytes<PublicKey>,
) -> SignedTranscript {
    // The domain, two UUIDs, one horizon and one key. The compiler sums the
    // constants, so the sum cannot overflow at run time.
    let mut transcript = Vec::with_capacity(const { CONFIRM_DOMAIN.len() + 16 + 16 + 8 + 32 });
    transcript.extend_from_slice(CONFIRM_DOMAIN);
    transcript.extend_from_slice(document_id.as_bytes());
    transcript.extend_from_slice(snapshot_id.as_bytes());
    transcript.extend_from_slice(&covered_seq.get().to_be_bytes());
    transcript.extend_from_slice(author.as_array());
    SignedTranscript::new(transcript)
}

/// One confirmation as the relay stores and serves it: who signed, and the
/// signature over [`confirm_transcript`], both hex in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedConfirmation {
    /// The confirmer's signing key.
    pub confirmer: FixedBytes<PublicKey>,
    /// The confirmer's signature over the transcript.
    pub signature: FixedBytes<Signature>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confirmation names one horizon of one snapshot of one document by
    /// one author: changing any of the four changes what is signed.
    #[test]
    fn the_transcript_names_the_document_the_snapshot_the_horizon_and_the_author() {
        let doc = Id::<Document>::mint();
        let snap = Id::<Snapshot>::mint();
        let horizon = Seq::<Relay>::new(7);
        let author = FixedBytes::from_array([1; 32]);
        let base = confirm_transcript(doc, snap, horizon, &author);
        assert!(base.as_bytes().starts_with(CONFIRM_DOMAIN));
        let others = [
            confirm_transcript(Id::<Document>::mint(), snap, horizon, &author),
            confirm_transcript(doc, Id::<Snapshot>::mint(), horizon, &author),
            confirm_transcript(doc, snap, Seq::<Relay>::new(8), &author),
            confirm_transcript(doc, snap, horizon, &FixedBytes::from_array([2; 32])),
        ];
        assert!(others.iter().all(|other| *other != base));
    }

    /// A report and a confirmation of one snapshot never share their bytes,
    /// so neither signature stands for the other.
    #[test]
    fn a_confirmation_is_never_a_report() {
        let (doc, snap) = (Id::<Document>::mint(), Id::<Snapshot>::mint());
        let author = FixedBytes::from_array([1; 32]);
        let report = crate::snapshot_report::report_transcript(doc, snap, &author);
        let confirm = confirm_transcript(doc, snap, Seq::<Relay>::ZERO, &author);
        assert_ne!(report, confirm);
    }
}
