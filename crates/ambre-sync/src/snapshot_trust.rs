//! Which snapshot a cold open trusts whole (`AUTHOR-030`, audit `N31`).
//!
//! The relay's word that a member confirmed a snapshot proves nothing. The
//! open trusts a snapshot whole only under a confirmation another member
//! signed over the very blob it downloads, or when this user built it. Any
//! other snapshot is held back and checked once the replay is done
//! (`AUTHOR-022`).
//!
//! The confirmation signs the digest of the blob, so the open decides in two
//! steps. [`open_start`] picks the start from what the relay serves beside
//! the blob. [`Trusted::holds`] then reads the blob downloaded, and a blob
//! the confirmation does not name is no start (audit `W101`).

use ambre_core::id::{Document, Id, Snapshot};
use ambre_core::{FixedBytes, Payload, PublicKey, Relay, Seq};
use ambre_protocol::snapshot_confirm::{Confirmed, SignedConfirmation, confirm_transcript};

use crate::http::SnapshotMetadataResponse;
use crate::membership_log::Confirmers;
use crate::pipeline::{FetchedBlob, blob_digest};

/// One snapshot the relay serves, as the open names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Named {
    pub snapshot_id: Id<Snapshot>,
    pub covered_seq: Seq<Relay>,
    /// The key the blob must verify under: who built it.
    pub author: FixedBytes<PublicKey>,
}

/// Why an open may trust a snapshot whole, before its blob is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trust {
    /// This user built it.
    Own,
    /// A member the log admits signed this confirmation, which names the
    /// blob its signer checked.
    Confirmed(SignedConfirmation),
}

/// A snapshot the open starts from, once its blob passes [`Trusted::holds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trusted {
    pub named: Named,
    pub trust: Trust,
}

impl Trusted {
    /// Whether `blob`, the blob downloaded for this snapshot, is one the
    /// trust covers. The user's own snapshot is. A confirmed one is when
    /// the confirmation verifies over the digest of `blob` (`AUTHOR-031`,
    /// audit `W101`). A writer who sealed another blob under the same
    /// identifier and horizon carries no confirmation over to it.
    pub(crate) fn holds(&self, document_id: Id<Document>, blob: &Payload<FetchedBlob>) -> bool {
        match &self.trust {
            Trust::Own => true,
            Trust::Confirmed(confirmation) => {
                let digest = blob_digest(blob);
                let transcript = confirm_transcript(Confirmed {
                    document_id,
                    snapshot_id: self.named.snapshot_id,
                    covered_seq: self.named.covered_seq,
                    author: &self.named.author,
                    blob: &digest,
                });
                ambre_crypto::signature::verify_message(
                    &confirmation.confirmer,
                    &transcript,
                    &confirmation.signature,
                )
                .is_ok()
            }
        }
    }
}

/// Where an open starts, and which snapshot it holds back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenStart {
    /// The snapshot the open adopts whole, once its blob passes
    /// [`Trusted::holds`]. `None` replays from the first delta the relay
    /// kept.
    pub start: Option<Trusted>,
    /// The latest snapshot, checked once the replay is done (`AUTHOR-022`).
    pub held: Option<Named>,
}

/// Where the open of `document_id` starts, given the snapshot the relay
/// serves as its latest, and the reference beside it.
///
/// A snapshot is trusted whole when `own_key` built it, or under a
/// confirmation that [`confirmation_admitted`]. The latest one trusted is the
/// start. Otherwise the open starts from a trusted reference, or from the
/// first delta, and holds the latest one back. The blob of the start still
/// owes [`Trusted::holds`].
pub(crate) fn open_start(
    meta: &SnapshotMetadataResponse,
    own_key: Option<&FixedBytes<PublicKey>>,
    confirmers: &Confirmers,
) -> OpenStart {
    let trusted = |named: Named, confirmation: Option<&SignedConfirmation>| {
        if Some(&named.author) == own_key {
            return Some(Trusted {
                named,
                trust: Trust::Own,
            });
        }
        confirmation
            .filter(|c| confirmation_admitted(&named, c, confirmers))
            .map(|c| Trusted {
                named,
                trust: Trust::Confirmed(*c),
            })
    };
    let latest = Named {
        snapshot_id: meta.snapshot_id,
        covered_seq: meta.covered_seq,
        author: meta.author_pubkey,
    };
    if let Some(start) = trusted(latest, meta.confirmation.as_ref()) {
        return OpenStart {
            start: Some(start),
            held: None,
        };
    }
    let reference = meta.reference.as_ref().and_then(|r| {
        let named = Named {
            snapshot_id: r.snapshot_id,
            covered_seq: r.covered_seq,
            author: r.author_pubkey,
        };
        trusted(named, r.confirmation.as_ref())
    });
    OpenStart {
        start: reference,
        held: Some(latest),
    }
}

/// Whether `confirmation` may stand for a member other than the author of
/// `named`. The confirmer is a key the log bound to a member, not the
/// author, and not a key whose binding the author signed (`AUTHOR-030`,
/// `AUTHOR-034`). Its signature is read against the blob alone, by
/// [`Trusted::holds`].
pub(crate) fn confirmation_admitted(
    named: &Named,
    confirmation: &SignedConfirmation,
    confirmers: &Confirmers,
) -> bool {
    confirmation.confirmer != named.author
        && confirmers.admits(&confirmation.confirmer, &named.author)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ReferenceSnapshot;
    use ed25519_dalek::SigningKey;
    use std::collections::{HashMap, HashSet};

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

    /// A blob of the given bytes, as object storage returns it.
    fn blob(bytes: &[u8]) -> Payload<FetchedBlob> {
        Payload::new(bytes.to_vec())
    }

    /// The confirmation `confirmer` signs over `named` of `document_id`,
    /// having checked `checked`.
    fn signed(
        document_id: Id<Document>,
        named: &Named,
        checked: &Payload<FetchedBlob>,
        confirmer: &SigningKey,
    ) -> SignedConfirmation {
        let digest = blob_digest(checked);
        let transcript = confirm_transcript(Confirmed {
            document_id,
            snapshot_id: named.snapshot_id,
            covered_seq: named.covered_seq,
            author: &named.author,
            blob: &digest,
        });
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

    /// Members whose bindings a key no test authors signed.
    fn members(keys: &[&SigningKey]) -> Confirmers {
        let binder = public(&key(99));
        Confirmers::Granted(
            keys.iter()
                .map(|k| (public(k), HashSet::from([binder])))
                .collect::<HashMap<_, _>>(),
        )
    }

    /// Whether the open of `served` starts whole from `downloaded`, the
    /// blob of its start.
    fn adopts(
        doc: Id<Document>,
        served: &SnapshotMetadataResponse,
        own: Option<&FixedBytes<PublicKey>>,
        confirmers: &Confirmers,
        downloaded: &Payload<FetchedBlob>,
    ) -> Option<Named> {
        open_start(served, own, confirmers)
            .start
            .filter(|start| start.holds(doc, downloaded))
            .map(|start| start.named)
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
        let bytes = blob(b"forged");
        let plan = |confirmation| {
            adopts(
                doc,
                &meta(&forged, confirmation, None),
                Some(&public(&alice)),
                &confirmers,
                &bytes,
            )
        };
        // No signature at all, the author's own, or one over another horizon.
        assert_eq!(plan(None), None);
        assert_eq!(plan(Some(signed(doc, &forged, &bytes, &mallory))), None);
        let shifted = Named {
            covered_seq: Seq::<Relay>::new(8),
            ..forged
        };
        assert_eq!(plan(Some(signed(doc, &shifted, &bytes, &bob))), None);
        // A key the log never bound, which the relay may hold.
        assert_eq!(plan(Some(signed(doc, &forged, &bytes, &key(9)))), None);
        // Nothing trusted: the latest is held back.
        let held = open_start(&meta(&forged, None, None), None, &confirmers);
        assert_eq!(held.start, None);
        assert_eq!(held.held, Some(forged));
        // Another member's signature over this very blob: trusted whole.
        assert_eq!(plan(Some(signed(doc, &forged, &bytes, &bob))), Some(forged));
    }

    /// `AUTHOR-031`, audit `W101`: Bob confirms the blob he checked. The
    /// writer then seals another blob under the same identifier and horizon,
    /// and the relay serves it with Bob's genuine confirmation. The open
    /// does not adopt it. Before the fix, the transcript named no blob, and
    /// every cold open adopted the second one.
    #[test]
    fn a_confirmation_holds_for_the_blob_it_names_alone() {
        let doc = Id::<Document>::mint();
        let (mallory, bob, carol) = (key(1), key(2), key(3));
        let confirmers = members(&[&mallory, &bob, &carol]);
        let snapshot = named(&mallory, 9);
        let checked = blob(b"the honest blob Bob checked");
        let swapped = blob(b"another blob under the same id");
        let served = meta(
            &snapshot,
            Some(signed(doc, &snapshot, &checked, &bob)),
            None,
        );
        let own = Some(&public(&carol));
        assert_eq!(
            adopts(doc, &served, own, &confirmers, &checked),
            Some(snapshot)
        );
        assert_eq!(adopts(doc, &served, own, &confirmers, &swapped), None);
        // A blob of the user's own adopts on its own word.
        let mine = meta(&snapshot, None, None);
        let author = Some(&snapshot.author);
        assert_eq!(
            adopts(doc, &mine, author, &confirmers, &swapped),
            Some(snapshot)
        );
    }

    /// `AUTHOR-034`, audit `W203`: Mallory's second account, whose binding
    /// she signed, confirms her snapshot. The open holds it back. The same
    /// account still confirms a snapshot of another author.
    #[test]
    fn a_confirmer_the_author_bound_does_not_make_a_snapshot_trusted() {
        let doc = Id::<Document>::mint();
        let (mallory, guest, alice) = (key(1), key(2), key(3));
        let confirmers = Confirmers::Granted(HashMap::from([
            (public(&mallory), HashSet::from([public(&alice)])),
            (public(&guest), HashSet::from([public(&mallory)])),
            (public(&alice), HashSet::from([public(&alice)])),
        ]));
        let bytes = blob(b"snapshot");
        let by_mallory = named(&mallory, 9);
        let served = meta(
            &by_mallory,
            Some(signed(doc, &by_mallory, &bytes, &guest)),
            None,
        );
        let plan = open_start(&served, None, &confirmers);
        assert_eq!(plan.start, None);
        assert_eq!(plan.held, Some(by_mallory));
        let by_alice = named(&alice, 9);
        let served = meta(
            &by_alice,
            Some(signed(doc, &by_alice, &bytes, &guest)),
            None,
        );
        assert_eq!(
            adopts(doc, &served, None, &confirmers, &bytes),
            Some(by_alice)
        );
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
        let bytes = blob(b"reference");
        let good = signed(doc, &reference, &bytes, &bob);
        let plan = |confirmation, own: &SigningKey| {
            let served = meta(&latest, None, Some((&reference, confirmation)));
            open_start(&served, Some(&public(own)), &confirmers)
        };
        let with_good = plan(Some(good), &bob);
        assert_eq!(with_good.start.map(|s| s.named), Some(reference));
        assert_eq!(with_good.held, Some(latest));
        assert!(with_good.start.is_some_and(|s| s.holds(doc, &bytes)));
        assert_eq!(
            plan(None, &bob),
            OpenStart {
                start: None,
                held: Some(latest),
            }
        );
        // Its author opens its own snapshot whole.
        assert_eq!(plan(None, &mallory).start.map(|s| s.named), Some(latest));
        // An unanchored log admits any confirmer but the author.
        let unchecked = adopts(
            doc,
            &meta(&latest, Some(signed(doc, &latest, &bytes, &key(9))), None),
            None,
            &Confirmers::Unchecked,
            &bytes,
        );
        assert_eq!(unchecked, Some(latest));
    }
}
