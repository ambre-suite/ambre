//! What a mismatch report signs (`AUTHOR-023`, `SRV-133`).
//!
//! A member whose deltas disagree with a pending snapshot reports it, and the
//! relay drops the snapshot. The reporter signs the document, the snapshot and
//! the snapshot's author. So the relay can check it, and nobody can replay it
//! against another snapshot.

use ambre_core::{DocumentId, PublicKey, SnapshotId};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
pub const REPORT_DOMAIN: &[u8] = b"ambre-snapshot-report-v1";

/// The bytes a reporter signs: the domain, then the document, the snapshot and
/// the author's signing key, each at a fixed length.
#[must_use]
pub fn report_transcript(
    document_id: DocumentId,
    snapshot_id: SnapshotId,
    author: &PublicKey,
) -> SignedTranscript {
    let mut transcript = Vec::with_capacity(REPORT_DOMAIN.len() + 16 + 16 + 32);
    transcript.extend_from_slice(REPORT_DOMAIN);
    transcript.extend_from_slice(document_id.as_uuid().as_bytes());
    transcript.extend_from_slice(snapshot_id.as_uuid().as_bytes());
    transcript.extend_from_slice(author.as_array());
    SignedTranscript::new(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report names one snapshot of one document by one author: changing
    /// any of the three changes what is signed.
    #[test]
    fn the_transcript_names_the_document_the_snapshot_and_the_author() {
        let doc = DocumentId::new();
        let snap = SnapshotId::new();
        let base = report_transcript(doc, snap, &PublicKey::from_array([1; 32]));
        assert!(base.as_bytes().starts_with(REPORT_DOMAIN));
        assert_ne!(
            base,
            report_transcript(DocumentId::new(), snap, &PublicKey::from_array([1; 32]))
        );
        assert_ne!(
            base,
            report_transcript(doc, SnapshotId::new(), &PublicKey::from_array([1; 32]))
        );
        assert_ne!(
            base,
            report_transcript(doc, snap, &PublicKey::from_array([2; 32]))
        );
    }
}
