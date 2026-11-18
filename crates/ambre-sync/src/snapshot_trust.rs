//! Which snapshot a cold open trusts whole (`AUTHOR-030`, audit `N31`).
//!
//! The relay's word that a member confirmed a snapshot proves nothing. The
//! open trusts a snapshot whole only under a confirmation another member
//! signed, or when this user built it. Any other snapshot is held back and
//! checked once the replay is done (`AUTHOR-022`).

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{FixedBytes, PublicKey, Relay, Seq};
use ambre_protocol::snapshot_confirm::{SignedConfirmation, confirm_transcript};

use crate::http::SnapshotMetadataResponse;
use crate::membership_log::Confirmers;

/// One snapshot the relay serves, as the open names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Named {
    pub snapshot_id: Id<Snapshot>,
    pub covered_seq: Seq<Relay>,
    /// The key the blob must verify under: who built it.
    pub author: FixedBytes<PublicKey>,
}

/// Where an open starts, and which snapshot it holds back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenStart {
    /// The snapshot the open adopts whole. `None` replays from the first
    /// delta the relay kept.
    pub start: Option<Named>,
    /// The latest snapshot, checked once the replay is done (`AUTHOR-022`).
    pub held: Option<Named>,
}

/// Where the open of `document_id` starts, given the snapshot the relay
/// serves as its latest, and the reference beside it.
///
/// A snapshot is trusted whole when `own_key` built it, or under a
/// confirmation that [`confirmation_holds`]. The latest one trusted is the
/// start. Otherwise the open starts from a trusted reference, or from the
/// first delta, and holds the latest one back.
pub(crate) fn open_start(
    document_id: Id<Document>,
    meta: &SnapshotMetadataResponse,
    own_key: Option<&FixedBytes<PublicKey>>,
    confirmers: &Confirmers,
) -> OpenStart {
    let trusted = |named: &Named, confirmation: Option<&SignedConfirmation>| {
        Some(&named.author) == own_key
            || confirmation.is_some_and(|c| confirmation_holds(document_id, named, c, confirmers))
    };
    let latest = Named {
        snapshot_id: meta.snapshot_id,
        covered_seq: meta.covered_seq,
        author: meta.author_pubkey,
    };
    if trusted(&latest, meta.confirmation.as_ref()) {
        return OpenStart {
            start: Some(latest),
            held: None,
        };
    }
    let reference = meta.reference.as_ref().and_then(|r| {
        let named = Named {
            snapshot_id: r.snapshot_id,
            covered_seq: r.covered_seq,
            author: r.author_pubkey,
        };
        trusted(&named, r.confirmation.as_ref()).then_some(named)
    });
    OpenStart {
        start: reference,
        held: Some(latest),
    }
}

/// Whether `confirmation` makes `named` a snapshot of `document_id` that a
/// member other than its author checked. The confirmer is a key the log
/// bound to a member, and not the author. Its signature covers the document,
/// the snapshot, the horizon and the author.
pub(crate) fn confirmation_holds(
    document_id: Id<Document>,
    named: &Named,
    confirmation: &SignedConfirmation,
    confirmers: &Confirmers,
) -> bool {
    let transcript = confirm_transcript(
        document_id,
        named.snapshot_id,
        named.covered_seq,
        &named.author,
    );
    confirmation.confirmer != named.author
        && confirmers.admits(&confirmation.confirmer)
        && ambre_crypto::signature::verify_message(
            &confirmation.confirmer,
            &transcript,
            &confirmation.signature,
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReferenceSnapshot;
    use ed25519_dalek::SigningKey;
    use std::collections::HashSet;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(key: &SigningKey) -> FixedBytes<PublicKey> {
        FixedBytes::from_array(key.verifying_key().to_bytes())
    }

    fn named(author: &SigningKey, horizon: u64) -> Named {
        Named {
            snapshot_id: Id::<Snapshot>::mint(),
            covered_seq: Seq::<Relay>::new(horizon),
            author: public(author),
        }
    }

    /// The confirmation `confirmer` signs over `named` of `document_id`.
    fn signed(
        document_id: Id<Document>,
        named: &Named,
        confirmer: &SigningKey,
    ) -> SignedConfirmation {
        let transcript = confirm_transcript(
            document_id,
            named.snapshot_id,
            named.covered_seq,
            &named.author,
        );
        SignedConfirmation {
            confirmer: public(confirmer),
            signature: ambre_crypto::signature::sign_message(confirmer, &transcript),
        }
    }

    /// What the relay serves for `latest`, with `reference` beside it.
    fn meta(
        latest: &Named,
        confirmation: Option<SignedConfirmation>,
        reference: Option<(&Named, Option<SignedConfirmation>)>,
    ) -> SnapshotMetadataResponse {
        SnapshotMetadataResponse {
            snapshot_id: latest.snapshot_id,
            covered_seq: latest.covered_seq,
            author_pubkey: latest.author,
            reference: reference.map(|(r, confirmation)| ReferenceSnapshot {
                snapshot_id: r.snapshot_id,
                covered_seq: r.covered_seq,
                author_pubkey: r.author,
                confirmation,
            }),
            confirmation,
        }
    }

    fn members(keys: &[&SigningKey]) -> Confirmers {
        Confirmers::Granted(keys.iter().map(|k| public(k)).collect::<HashSet<_>>())
    }

    /// `AUTHOR-030`, audit `N31`: a snapshot the relay calls confirmed, with
    /// no confirmation another member signed, is never trusted whole. The
    /// open holds it back and checks it once the replay is done. Before the
    /// fix, the open adopted it on the relay's status alone.
    #[test]
    fn a_snapshot_the_relay_alone_calls_confirmed_is_held_back() {
        let doc = Id::<Document>::mint();
        let (mallory, bob, alice) = (key(1), key(2), key(3));
        let confirmers = members(&[&mallory, &bob, &alice]);
        let forged = named(&mallory, 9);
        let plan = |confirmation| {
            open_start(
                doc,
                &meta(&forged, confirmation, None),
                Some(&public(&alice)),
                &confirmers,
            )
        };
        let held = OpenStart {
            start: None,
            held: Some(forged),
        };
        // No signature at all, the author's own, or one over another horizon.
        assert_eq!(plan(None), held);
        assert_eq!(plan(Some(signed(doc, &forged, &mallory))), held);
        let shifted = Named {
            covered_seq: Seq::<Relay>::new(8),
            ..forged
        };
        assert_eq!(plan(Some(signed(doc, &shifted, &bob))), held);
        // A key the log never bound, which the relay may hold.
        assert_eq!(plan(Some(signed(doc, &forged, &key(9)))), held);
        // Another member's signature over this very snapshot: trusted whole.
        let trusted = plan(Some(signed(doc, &forged, &bob)));
        assert_eq!(trusted.start, Some(forged));
        assert_eq!(trusted.held, None);
    }

    /// `AUTHOR-030`: a snapshot this user built opens whole on its own word.
    /// One another user confirmed alone opens from the reference only under
    /// the reference's signed confirmation, and from the first delta
    /// otherwise.
    #[test]
    fn the_reference_is_the_start_only_under_its_signed_confirmation() {
        let doc = Id::<Document>::mint();
        let (mallory, bob, alice) = (key(1), key(2), key(3));
        let confirmers = members(&[&mallory, &bob, &alice]);
        let reference = named(&alice, 4);
        let latest = named(&mallory, 9);
        let good = signed(doc, &reference, &bob);
        let plan = |confirmation, own: &SigningKey| {
            let served = meta(&latest, None, Some((&reference, confirmation)));
            open_start(doc, &served, Some(&public(own)), &confirmers)
        };
        assert_eq!(
            plan(Some(good), &bob),
            OpenStart {
                start: Some(reference),
                held: Some(latest),
            }
        );
        assert_eq!(
            plan(None, &bob),
            OpenStart {
                start: None,
                held: Some(latest),
            }
        );
        // Its author opens its own snapshot whole.
        assert_eq!(plan(None, &mallory).start, Some(latest));
        // An unanchored log admits any confirmer but the author.
        let unchecked = open_start(
            doc,
            &meta(&latest, Some(signed(doc, &latest, &key(9))), None),
            None,
            &Confirmers::Unchecked,
        );
        assert_eq!(unchecked.start, Some(latest));
    }
}
