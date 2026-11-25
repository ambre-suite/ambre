//! What a confirmation signs (`AUTHOR-031`, `SRV-284`).
//!
//! A member whose deltas match a snapshot confirms it. The confirmer signs
//! the document, the snapshot, the horizon, the snapshot's author and the
//! digest of the sealed blob it checked. A cold open then trusts a snapshot
//! whole on that signature, and never on the relay's word that another
//! member confirmed it (audit `N31`). The digest binds the signature to one
//! blob. A writer who seals another blob under the same identifier and
//! horizon carries no confirmation over to it (audit `W101`).
//!
//! A confirmer whose walk readmits a writer also signs the head of that walk.
//! An open then admits a snapshot folding that writer past the lifted cap,
//! as a delta signed under that head (`AUTHOR-038`, `SYNC-317`).

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{FixedBytes, PublicKey, Relay, Seq, Signature, SnapshotDigest};
use serde::{Deserialize, Serialize};

use crate::membership::LogHead;
use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
/// Version 2 adds the digest of the blob (audit `W101`).
pub const CONFIRM_DOMAIN: &[u8] = b"ambre-snapshot-confirm-v2";

/// The domain of a confirmation that names the confirmer's log head
/// (`AUTHOR-038`). A confirmation with no head keeps the bytes of version 2,
/// so no stored signature changes.
pub const CONFIRM_HEAD_DOMAIN: &[u8] = b"ambre-snapshot-confirm-v3";

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
    /// The head of the log the confirmer walked as it checked the blob, when
    /// that walk readmits a writer (`AUTHOR-038`).
    pub head: Option<LogHead>,
}

/// The bytes a confirmer signs, each field at a fixed length. The domain
/// comes first, then the document, the snapshot, the horizon, the author's
/// signing key and the digest of the blob. A head follows last, under the
/// domain of version 3. It takes its place, a presence byte, and the digest
/// of its last entry, zeroed when absent.
#[must_use]
pub fn confirm_transcript(confirmed: Confirmed<'_>) -> SignedTranscript {
    // Both domains, two UUIDs, one horizon, one key, one digest and one head.
    // The compiler sums the constants, so the sum cannot overflow at run time.
    let mut transcript = Vec::with_capacity(
        const { CONFIRM_HEAD_DOMAIN.len() + 16 + 16 + 8 + 32 + 32 + 8 + 1 + 32 },
    );
    transcript.extend_from_slice(match confirmed.head {
        None => CONFIRM_DOMAIN,
        Some(_) => CONFIRM_HEAD_DOMAIN,
    });
    transcript.extend_from_slice(confirmed.document_id.as_bytes());
    transcript.extend_from_slice(confirmed.snapshot_id.as_bytes());
    transcript.extend_from_slice(&confirmed.covered_seq.get().to_be_bytes());
    transcript.extend_from_slice(confirmed.author.as_array());
    transcript.extend_from_slice(confirmed.blob.as_array());
    if let Some(head) = confirmed.head {
        transcript.extend_from_slice(&head.position.get().to_be_bytes());
        match head.previous {
            Some(digest) => {
                transcript.push(1);
                transcript.extend_from_slice(digest.as_array());
            }
            None => {
                transcript.push(0);
                transcript.extend_from_slice(&[0; 32]);
            }
        }
    }
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
    /// The log head the transcript names, absent on a confirmation of
    /// version 2 (`AUTHOR-038`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<LogHead>,
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
            head: None,
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

    /// A confirmation that names a head signs it under its own domain, and
    /// another place or another last entry changes the bytes. One with no
    /// head keeps the bytes of version 2 (`AUTHOR-038`).
    #[test]
    fn a_confirmation_signs_the_head_it_names() {
        let author = FixedBytes::from_array([1; 32]);
        let blob = FixedBytes::from_array([4; 32]);
        let head = LogHead {
            position: Seq::new(3),
            previous: Some(FixedBytes::from_array([6; 32])),
        };
        let bare = Confirmed {
            document_id: Id::<Document>::mint(),
            snapshot_id: Id::<Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(7),
            author: &author,
            blob: &blob,
            head: None,
        };
        let headed = Confirmed {
            head: Some(head),
            ..bare
        };
        assert!(
            confirm_transcript(bare)
                .as_bytes()
                .starts_with(CONFIRM_DOMAIN)
        );
        let signed = confirm_transcript(headed);
        assert!(signed.as_bytes().starts_with(CONFIRM_HEAD_DOMAIN));
        let others = [
            None,
            Some(LogHead {
                position: Seq::new(4),
                ..head
            }),
            Some(LogHead {
                previous: Some(FixedBytes::from_array([7; 32])),
                ..head
            }),
            Some(LogHead {
                previous: None,
                ..head
            }),
        ];
        assert!(others.iter().all(|other| {
            confirm_transcript(Confirmed {
                head: *other,
                ..headed
            }) != signed
        }));
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
            head: None,
        });
        assert_ne!(report, confirm);
    }
}
