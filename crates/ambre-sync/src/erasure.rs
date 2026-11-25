//! The erasure of a thread name that no document of the channel answers
//! (`SYNC-322`, `SYNC-323`).
//!
//! Any member sets the absent `thread` field of a root, and no client can check that the relay linked that document. The census
//! refuses a listing short of a thread a removal names (`SYNC-321`). So a
//! field that names no document of the channel fails each removal.
//!
//! An owner of the channel signs an erasure of that name, and the relay keeps
//! it. A direct message has no owner, so each of its present writers may
//! sign one (`SYNC-327`). A census passes a hidden thread only under such an
//! erasure, whose signature it checks against the keys the log of the
//! `meta` holds. The
//! relay alone thus never clears a thread. A document the `meta` names takes
//! no erasure: only a thread does. Nor does a thread a valid revoke of that
//! log caps, which stood in the channel at that post (`SYNC-329`).
//!
//! Nor does a thread whose link receipt the relay signed, which it cannot
//! take back (`SYNC-370`).

use std::collections::HashSet;

use ambre_bus::LinkReceipt;
use ambre_core::id::{Channel, Document, Id};
use ambre_core::{FixedBytes, PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::http;
use crate::key_vouch::Footing;
use crate::membership_log::LogRule;
use crate::sharing::{Author, ShareHttp};

/// One erasure of a thread name, as the relay lists it (`SRV-347`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ErasureEntry {
    #[serde(with = "ambre_core::serde_id::text")]
    pub thread: Id<Document>,
    pub signer: FixedBytes<PublicKey>,
    pub signature: FixedBytes<Signature>,
}

/// `GET /api/channels/{id}/erasures`: every erasure the channel keeps.
#[derive(Deserialize, Serialize)]
pub(crate) struct ErasuresResponse {
    pub erasures: Vec<ErasureEntry>,
}

/// `POST /api/channels/{id}/erasures`: an owner's erasure of one name.
#[derive(Deserialize, Serialize)]
struct EraseRequest {
    #[serde(with = "ambre_core::serde_id::text")]
    thread: Id<Document>,
    signature: FixedBytes<Signature>,
}

/// The answer to an erasure: the name it cleared.
#[derive(Deserialize, Serialize)]
struct ErasedResponse {
    #[serde(with = "ambre_core::serde_id::text")]
    thread: Id<Document>,
}

/// What an erasure of the hidden threads of a removal needs.
pub(crate) struct ErasureParams {
    pub url_scheme: http::UrlScheme,
    pub api_base: http::Url<http::RelayHost>,
    pub channel_id: Id<Channel>,
    /// The `meta` of the channel, whose log names its owners.
    pub meta: Id<Document>,
    /// The threads the removal names and the listing leaves out.
    pub hidden: Vec<Id<Document>>,
    /// The link receipts the sealed state of the channel holds (`SYNC-370`).
    pub receipts: Vec<LinkReceipt>,
    /// The footing of the open of the `meta` (audit `P26`).
    pub footing: Footing,
    pub rule: LogRule,
    /// This user, who signs an erasure when the log holds them as an owner.
    pub author: Author,
}

/// The signature `author` puts on the erasure of `thread` in `channel_id`.
pub(crate) fn sign_erasure(
    author: &Author,
    channel_id: Id<Channel>,
    thread: Id<Document>,
) -> FixedBytes<Signature> {
    let transcript = ambre_protocol::thread_erasure::erasure_transcript(channel_id, thread);
    FixedBytes::from_array(
        ed25519_dalek::Signer::sign(&author.signing_key, transcript.as_bytes()).to_bytes(),
    )
}

/// Whether `signature` by `key` holds over `transcript`.
fn holds(
    key: &FixedBytes<PublicKey>,
    transcript: &ambre_protocol::SignedTranscript,
    signature: &FixedBytes<Signature>,
) -> bool {
    let signature = ed25519_dalek::Signature::from_bytes(signature.as_array());
    ed25519_dalek::VerifyingKey::from_bytes(key.as_array())
        .and_then(|key| key.verify_strict(transcript.as_bytes(), &signature))
        .is_ok()
}

/// Whether `erasure` carries a valid signature of its signer over the
/// erasure of its thread in `channel_id`.
fn verifies(channel_id: Id<Channel>, erasure: &ErasureEntry) -> bool {
    let transcript = ambre_protocol::thread_erasure::erasure_transcript(channel_id, erasure.thread);
    holds(&erasure.signer, &transcript, &erasure.signature)
}

/// The link key a build names in `AMBRE_THREAD_LINK_KEY`, read as the crate
/// compiles (`ZK-062`). An empty name names none, as a build argument left
/// empty does. A name that spells no key fails the build, and so does
/// the development key, under which any member forges a receipt. Each panic
/// runs in the compiler alone: a const item never evaluates at run time.
const NAMED_LINK_KEY: Option<FixedBytes<PublicKey>> = match option_env!("AMBRE_THREAD_LINK_KEY") {
    None => None,
    Some(text) if text.is_empty() => None,
    Some(text) => {
        let Some(key) = FixedBytes::<PublicKey>::from_hex_const(text) else {
            panic!("AMBRE_THREAD_LINK_KEY must name 32 bytes in 64 hex digits");
        };
        let (mut named, mut dev): (&[u8], &[u8]) = (
            key.as_array(),
            ambre_protocol::thread_erasure::DEV_LINK_KEY.as_array(),
        );
        let mut same = true;
        while let ([one, named_rest @ ..], [other, dev_rest @ ..]) = (named, dev) {
            same &= *one == *other;
            (named, dev) = (named_rest, dev_rest);
        }
        assert!(
            !same,
            "AMBRE_THREAD_LINK_KEY names the development link key"
        );
        Some(key)
    }
};

/// The link key this build pins (`SYNC-371`). A release build names it at
/// build time (`ZK-062`). A development build or a test pins the key of the
/// public dev seed. Any other build pins none, so no receipt proves a link.
fn link_key() -> Option<FixedBytes<PublicKey>> {
    match NAMED_LINK_KEY {
        Some(key) => Some(key),
        None if cfg!(any(test, feature = "dev-login")) => {
            Some(ambre_protocol::thread_erasure::DEV_LINK_KEY)
        }
        None => None,
    }
}

/// Whether a receipt of `receipts` proves that the relay linked `thread` in
/// `channel_id` (`SYNC-370`). The pinned link key signed such a receipt over
/// that channel and that thread (`SYNC-371`).
pub(crate) fn linked(
    channel_id: Id<Channel>,
    thread: Id<Document>,
    receipts: &[LinkReceipt],
) -> bool {
    let Some(key) = link_key() else {
        return false;
    };
    let transcript = ambre_protocol::thread_erasure::link_transcript(channel_id, thread);
    receipts
        .iter()
        .any(|r| r.thread == thread && holds(&key, &transcript, &r.signature))
}

/// The threads of `hidden` that no erasure covers. An erasure covers a
/// thread when a key of `owners` signed it for `channel_id` (`SYNC-322`).
pub(crate) fn unerased(
    channel_id: Id<Channel>,
    hidden: &[Id<Document>],
    erasures: &[ErasureEntry],
    owners: &HashSet<FixedBytes<PublicKey>>,
) -> Vec<Id<Document>> {
    hidden
        .iter()
        .copied()
        .filter(|thread| {
            !erasures.iter().any(|erasure| {
                erasure.thread == *thread
                    && owners.contains(&erasure.signer)
                    && verifies(channel_id, erasure)
            })
        })
        .collect()
}

/// The refusal of a listing that hides a thread no erasure covers.
fn hidden_error() -> http::HttpError {
    http::HttpError::Local(ambre_core::LocalFailure::ChannelDocumentHidden)
}

/// Clears each hidden thread of a removal, or fails it (`SYNC-322`,
/// `SYNC-323`).
///
/// The owners come from the log of the `meta`, walked from the footing of
/// its open. A log with no owner, as a direct message holds, gives the
/// erasure to each present writer instead (`SYNC-327`). A thread such a
/// key's erasure covers passes. When one is left and this user holds such a
/// key, this user signs an erasure of each one left, and the relay records
/// it.
///
/// A thread that a valid revoke of the log caps stood in the channel. So
/// the listing hides it, and no erasure covers it (`SYNC-329`). Neither
/// does a thread under a valid link receipt (`SYNC-370`).
///
/// # Errors
/// [`ambre_core::LocalFailure::ChannelDocumentHidden`] in four cases:
/// * a thread is left and this user holds no such key;
/// * the log does not stand on the footing;
/// * a hidden thread is one that a valid revoke of that log caps
///   (`SYNC-329`);
/// * a hidden thread is under a valid link receipt (`SYNC-370`).
///
/// The error of each request otherwise.
pub(crate) async fn erase_hidden<H: ShareHttp>(
    http: &H,
    p: ErasureParams,
) -> Result<(), http::HttpError> {
    // The relay signed the link of such a thread, so its listing hides it,
    // and no erasure covers it (`SYNC-370`).
    if p.hidden
        .iter()
        .any(|t| linked(p.channel_id, *t, &p.receipts))
    {
        return Err(hidden_error());
    }
    let root = http::Url::<http::ApiRoot>::of(p.url_scheme, &p.api_base);
    let members: http::DocumentMembersResponse = http
        .get_json(&root.route(format!("/documents/{}/members", p.meta)))
        .await?;
    let walked = p
        .footing
        .walk(&members.log, &p.rule)
        .map_err(|_| hidden_error())?;
    // A thread a signed revoke caps stood in the channel at that post: the
    // listing hides it, and no erasure covers it (`SYNC-329`).
    if p.hidden.iter().any(|t| walked.log().caps_document(*t)) {
        return Err(hidden_error());
    }
    let owners = walked.log().erasure_keys();
    let erasures_url = root.route(format!("/channels/{}/erasures", p.channel_id));
    let listed: ErasuresResponse = http.get_json(&erasures_url).await?;
    let left = unerased(p.channel_id, &p.hidden, &listed.erasures, &owners);
    if left.is_empty() {
        return Ok(());
    }
    if !owners.contains(&p.author.public()) {
        return Err(hidden_error());
    }
    for thread in left {
        let body = EraseRequest {
            thread,
            signature: sign_erasure(&p.author, p.channel_id, thread),
        };
        let erased: ErasedResponse = http.post_json(&erasures_url, &body).await?;
        if erased.thread != thread {
            return Err(hidden_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_crypto::MasterSeed;

    fn author(seed: u8) -> Author {
        Author {
            user_id: Id::<ambre_core::id::User>::mint(),
            signing_key: MasterSeed::from_array([seed; 32]).derive_signing_key(),
        }
    }

    fn erasure(by: &Author, channel_id: Id<Channel>, thread: Id<Document>) -> ErasureEntry {
        ErasureEntry {
            thread,
            signer: by.public(),
            signature: sign_erasure(by, channel_id, thread),
        }
    }

    /// `SYNC-322`: a hidden thread passes under an erasure an owner signed for
    /// that channel and that thread alone. Four erasures leave it hidden: by
    /// a key that owns nothing, of another channel, of another thread, and
    /// under a forged signature.
    #[test]
    fn a_hidden_thread_passes_only_under_an_owners_erasure() {
        let (owner, editor) = (author(1), author(2));
        let owners = HashSet::from([owner.public()]);
        let channel = Id::<Channel>::mint();
        let (phantom, other) = (Id::<Document>::mint(), Id::<Document>::mint());
        let hidden = [phantom, other];

        let held = [erasure(&owner, channel, phantom)];
        assert_eq!(unerased(channel, &hidden, &held, &owners), [other]);

        let mut forged = erasure(&owner, channel, other);
        forged.signer = owner.public();
        forged.signature = sign_erasure(&editor, channel, other);
        let refused = [
            erasure(&editor, channel, phantom),
            erasure(&owner, Id::<Channel>::mint(), phantom),
            ErasureEntry {
                thread: phantom,
                ..erasure(&owner, channel, other)
            },
            forged,
        ];
        for erasure in refused {
            assert_eq!(
                unerased(channel, &hidden, std::slice::from_ref(&erasure), &owners),
                hidden,
                "{erasure:?}"
            );
        }
        assert_eq!(unerased(channel, &hidden, &[], &owners), hidden);
    }
}
