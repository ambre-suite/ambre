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
//! take back (`SYNC-330`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};

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
    /// The link receipts the sealed state of the channel holds (`SYNC-330`).
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

/// The 16 bytes that open the link key slot of the Sync worker. The patch of
/// an install finds them once in the wasm, and writes its key in the 32 bytes
/// after them (`ZK-090`, `scripts/link_key.py`). Random bytes, so no other
/// data of the build spells them.
const LINK_KEY_SLOT_MARKER: [u8; 16] = [
    0x46, 0x11, 0x76, 0xfb, 0x8d, 0x6a, 0xaf, 0xeb, 0x98, 0x92, 0x88, 0x04, 0x0c, 0x45, 0x91, 0xf9,
];

/// The link key slot: the marker, then the 32 bytes of the key, in this order
/// and with no padding. A release publishes the key bytes as zeros, which
/// name no key (`ZK-057`). The key bytes are atomics, so the compiler holds
/// the global as mutable. No optimisation then folds the zeros of the build
/// into the code that reads them, and a patch of the bytes takes effect.
#[repr(C)]
struct LinkKeySlot {
    marker: [u8; 16],
    key: [AtomicU8; 32],
}

impl LinkKeySlot {
    /// The slot as a build writes it: the marker, and no key.
    const fn empty() -> Self {
        Self {
            marker: LINK_KEY_SLOT_MARKER,
            key: [const { AtomicU8::new(0) }; 32],
        }
    }

    /// The key the slot holds, or `None` when its 32 bytes are zero.
    fn key(&self) -> Option<FixedBytes<PublicKey>> {
        let bytes = self.key.each_ref().map(|b| b.load(Ordering::Relaxed));
        (bytes != [0; 32]).then(|| FixedBytes::from_array(bytes))
    }
}

/// The one link key slot of the build. `#[used]` keeps the static whole in
/// the data of the wasm, so the marker stays beside the key bytes it opens.
#[used]
static LINK_KEY_SLOT: LinkKeySlot = LinkKeySlot::empty();

/// The link key this client counts receipts under (`SYNC-331`): the key the
/// install patched into its slot (`ZK-090`). A development build or a test
/// whose slot is empty takes the key of the public dev seed. Any other build
/// with an empty slot takes none, so no receipt proves a link, and the client
/// names no thread (`SYNC-332`).
fn link_key() -> Option<FixedBytes<PublicKey>> {
    let empty = if cfg!(any(test, feature = "dev-login")) {
        EmptySlot::TakesDevKey
    } else {
        EmptySlot::NamesNoKey
    };
    key_of(&LINK_KEY_SLOT, empty)
}

/// What an empty slot gives.
#[derive(Clone, Copy)]
enum EmptySlot {
    /// No key: a release the install did not patch.
    NamesNoKey,
    /// The key of the public dev seed: a development build or a test.
    TakesDevKey,
}

/// The key `slot` gives, or what `empty` gives when it holds none.
fn key_of(slot: &LinkKeySlot, empty: EmptySlot) -> Option<FixedBytes<PublicKey>> {
    match (slot.key(), empty) {
        (Some(key), _) => Some(key),
        (None, EmptySlot::TakesDevKey) => Some(ambre_protocol::thread_erasure::DEV_LINK_KEY),
        (None, EmptySlot::NamesNoKey) => {
            crate::log::warning!(
                "[Sync] this client holds no thread link key: the install patched none into its slot (ZK-090)"
            );
            None
        }
    }
}

/// Whether a receipt of `receipts` proves that the relay linked `thread` in
/// `channel_id` (`SYNC-330`). The pinned link key signed such a receipt over
/// that channel and that thread (`SYNC-331`).
pub(crate) fn linked(
    channel_id: Id<Channel>,
    thread: Id<Document>,
    receipts: &[LinkReceipt],
) -> bool {
    linked_under(link_key().as_ref(), channel_id, thread, receipts)
}

/// Whether `key` signed a receipt of `receipts` over `thread` in
/// `channel_id`. No key proves no link (`ZK-090`).
fn linked_under(
    key: Option<&FixedBytes<PublicKey>>,
    channel_id: Id<Channel>,
    thread: Id<Document>,
    receipts: &[LinkReceipt],
) -> bool {
    let Some(key) = key else {
        return false;
    };
    let transcript = ambre_protocol::thread_erasure::link_transcript(channel_id, thread);
    receipts
        .iter()
        .any(|r| r.thread == thread && holds(key, &transcript, &r.signature))
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
/// does a thread under a valid link receipt (`SYNC-330`).
///
/// # Errors
/// [`ambre_core::LocalFailure::ChannelDocumentHidden`] in four cases:
/// * a thread is left and this user holds no such key;
/// * the log does not stand on the footing;
/// * a hidden thread is one that a valid revoke of that log caps
///   (`SYNC-329`);
/// * a hidden thread is under a valid link receipt (`SYNC-330`).
///
/// The error of each request otherwise.
pub(crate) async fn erase_hidden<H: ShareHttp>(
    http: &H,
    p: ErasureParams,
) -> Result<(), http::HttpError> {
    // The relay signed the link of such a thread, so its listing hides it,
    // and no erasure covers it (`SYNC-330`).
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

    /// `ZK-090`: the slot lays out the marker, then the 32 key bytes, with no
    /// padding. The patch of an install searches for that layout.
    #[test]
    fn the_link_key_slot_is_the_marker_then_32_key_bytes() {
        assert_eq!(core::mem::size_of::<LinkKeySlot>(), 48);
        assert_eq!(core::mem::offset_of!(LinkKeySlot, marker), 0);
        assert_eq!(core::mem::offset_of!(LinkKeySlot, key), 16);
        assert_eq!(LINK_KEY_SLOT.marker, LINK_KEY_SLOT_MARKER);
    }

    /// `ZK-090`: an empty slot names no key outside a development build, and
    /// the key a patch writes is the key the client counts receipts under.
    #[test]
    fn a_patched_slot_gives_its_key_and_an_empty_one_none() {
        let slot = LinkKeySlot::empty();
        assert_eq!(key_of(&slot, EmptySlot::NamesNoKey), None);
        assert_eq!(
            key_of(&slot, EmptySlot::TakesDevKey),
            Some(ambre_protocol::thread_erasure::DEV_LINK_KEY)
        );
        let named = [0x5a; 32];
        for (byte, value) in slot.key.iter().zip(named) {
            byte.store(value, Ordering::Relaxed);
        }
        let key = Some(FixedBytes::<PublicKey>::from_array(named));
        assert_eq!(key_of(&slot, EmptySlot::NamesNoKey), key);
        assert_eq!(key_of(&slot, EmptySlot::TakesDevKey), key);
    }

    /// `ZK-090`: a client whose slot is empty counts no receipt, even one the
    /// key of the public dev seed signed, which a dev build counts.
    #[test]
    fn an_empty_slot_counts_no_receipt() {
        let (channel_id, thread) = (Id::<Channel>::mint(), Id::<Document>::mint());
        let relay = ed25519_dalek::SigningKey::from_bytes(&[0x4c; 32]);
        let transcript = ambre_protocol::thread_erasure::link_transcript(channel_id, thread);
        let receipt = LinkReceipt {
            thread,
            signature: FixedBytes::from_array(
                ed25519_dalek::Signer::sign(&relay, transcript.as_bytes()).to_bytes(),
            ),
        };
        let empty = LinkKeySlot::empty();
        let dev = key_of(&empty, EmptySlot::TakesDevKey);
        assert!(linked_under(dev.as_ref(), channel_id, thread, &[receipt]));
        let none = key_of(&empty, EmptySlot::NamesNoKey);
        assert!(!linked_under(none.as_ref(), channel_id, thread, &[receipt]));
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
