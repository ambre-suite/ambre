//! What a confirmation signs (`AUTHOR-031`, `SRV-284`).
//!
//! A member whose deltas match a snapshot confirms it. The confirmer signs
//! the document, the snapshot, the horizon, the snapshot's author and the
//! digest of the sealed blob it checked. A cold open then trusts a snapshot
//! whole on that signature, and never on the relay's word that another
//! member confirmed it (audit `N31`). The digest binds the signature to one
//! blob. A writer who seals another blob under the same identifier and
//! horizon carries no confirmation over to it (audit `W101`).

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{FixedBytes, PublicKey, Relay, Seq, Signature, SnapshotDigest};
use serde::{Deserialize, Serialize};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
/// Version 2 adds the digest of the blob (audit `W101`).
pub const CONFIRM_DOMAIN: &[u8] = b"ambre-snapshot-confirm-v2";

/// What one confirmation names: one blob of one snapshot of one document,
/// at one horizon, by one author.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirmed<'a> {
    /// The document the snapshot folds.
    pub document_id: Id<Document>,
    /// The snapshot the relay files the blob under.
    pub snapshot_id: Id<Snapshot>,
    /// The horizon of the snapshot (`SYNC-259`).
    pub covered_seq: Seq<Relay>,
    /// The signing key of the snapshot's author.
    pub author: &'a FixedBytes<PublicKey>,
    /// The digest of the sealed blob the confirmer checked.
    pub blob: &'a FixedBytes<SnapshotDigest>,
}

/// The bytes a confirmer signs, each field at a fixed length. The domain
/// comes first, then the document, the snapshot, the horizon, the author's
/// signing key and the digest of the blob.
#[must_use]
pub fn confirm_transcript(confirmed: Confirmed<'_>) -> SignedTranscript {
    // The domain, two UUIDs, one horizon, one key and one digest. The
    // compiler sums the constants, so the sum cannot overflow at run time.
    let mut transcript = Vec::with_capacity(const { CONFIRM_DOMAIN.len() + 16 + 16 + 8 + 32 + 32 });
    transcript.extend_from_slice(CONFIRM_DOMAIN);
    transcript.extend_from_slice(confirmed.document_id.as_bytes());
    transcript.extend_from_slice(confirmed.snapshot_id.as_bytes());
    transcript.extend_from_slice(&confirmed.covered_seq.get().to_be_bytes());
    transcript.extend_from_slice(confirmed.author.as_array());
    transcript.extend_from_slice(confirmed.blob.as_array());
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

    /// A confirmation names one blob at one horizon of one snapshot of one
    /// document by one author. A change of any of the five changes what is
    /// signed (`AUTHOR-031`, audit `W101`).
    #[test]
    fn the_transcript_names_the_document_the_snapshot_the_horizon_the_author_and_the_blob() {
        let author = FixedBytes::from_array([1; 32]);
        let blob = FixedBytes::from_array([4; 32]);
        let base = Confirmed {
            document_id: Id::<Document>::mint(),
            snapshot_id: Id::<Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(7),
            author: &author,
            blob: &blob,
        };
        let signed = confirm_transcript(base);
        assert!(signed.as_bytes().starts_with(CONFIRM_DOMAIN));
        let (other_author, other_blob) = (
            FixedBytes::from_array([2; 32]),
            FixedBytes::from_array([5; 32]),
        );
        let others = [
            Confirmed {
                document_id: Id::<Document>::mint(),
                ..base
            },
            Confirmed {
                snapshot_id: Id::<Snapshot>::mint(),
                ..base
            },
            Confirmed {
                covered_seq: Seq::<Relay>::new(8),
                ..base
            },
            Confirmed {
                author: &other_author,
                ..base
            },
            Confirmed {
                blob: &other_blob,
                ..base
            },
        ];
        assert!(
            others
                .iter()
                .all(|other| confirm_transcript(*other) != signed)
        );
    }

    /// A report and a confirmation of one snapshot never share their bytes,
    /// so neither signature stands for the other.
    #[test]
    fn a_confirmation_is_never_a_report() {
        let (doc, snap) = (Id::<Document>::mint(), Id::<Snapshot>::mint());
        let author = FixedBytes::from_array([1; 32]);
        let blob = FixedBytes::from_array([0; 32]);
        let report = crate::snapshot_report::report_transcript(doc, snap, &author);
        let confirm = confirm_transcript(Confirmed {
            document_id: doc,
            snapshot_id: snap,
            covered_seq: Seq::<Relay>::ZERO,
            author: &author,
            blob: &blob,
        });
        assert_ne!(report, confirm);
    }
}
