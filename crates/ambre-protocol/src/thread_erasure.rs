//! What an erasure of a thread name signs (`SYNC-322`, `SRV-347`).
//!
//! Any member sets the absent `thread` field of a root. A field that names no document of the channel fails each removal, as the
//! census refuses a listing short of a thread (`SYNC-321`). An owner of the
//! channel signs an erasure of that name. The relay records it, and every
//! census then passes that thread. The owner signs the channel and the thread,
//! so the relay cannot replay the signature against another thread.

use ambre_core::id::{Channel, Document, Id};
use ambre_core::{FixedBytes, PublicKey};

use crate::transcript::SignedTranscript;

/// The domain the transcript opens with, so the signature serves no other use.
pub const ERASURE_DOMAIN: &[u8] = b"ambre-thread-erasure-v1";

/// The bytes an owner signs: the domain, then the channel and the thread,
/// each at a fixed length.
#[must_use]
pub fn erasure_transcript(channel_id: Id<Channel>, thread: Id<Document>) -> SignedTranscript {
    // The domain and two UUIDs. The compiler sums the constants, so the sum
    // cannot overflow at run time.
    let mut transcript = Vec::with_capacity(const { ERASURE_DOMAIN.len() + 16 + 16 });
    transcript.extend_from_slice(ERASURE_DOMAIN);
    transcript.extend_from_slice(channel_id.as_bytes());
    transcript.extend_from_slice(thread.as_bytes());
    SignedTranscript::new(transcript)
}

/// The domain of a link receipt, apart from the erasure domain, so neither
/// signature verifies as the other.
pub const LINK_DOMAIN: &[u8] = b"ambre-thread-link-v1";

/// The public half of the link key a development relay signs under, from a
/// seed of `0x4c` bytes that anyone knows. Only a development build takes it:
/// a member could forge a receipt under it (`SYNC-331`).
pub const DEV_LINK_KEY: FixedBytes<PublicKey> = FixedBytes::from_array([
    0x92, 0x73, 0x08, 0xf5, 0x3d, 0x67, 0x65, 0x50, 0x3d, 0x2b, 0x64, 0xc1, 0x03, 0xab, 0x5a, 0xa4,
    0x72, 0x06, 0xab, 0x39, 0x50, 0x2c, 0xfe, 0x8f, 0x98, 0x44, 0x2f, 0xdb, 0x54, 0x31, 0x00, 0xb7,
]);

/// The bytes the relay signs in a link receipt: the link domain, then the
/// channel and the thread it linked (`SRV-363`). The relay cannot take back
/// a receipt it signed, so a census refuses to erase that thread
/// (`SYNC-330`).
#[must_use]
pub fn link_transcript(channel_id: Id<Channel>, thread: Id<Document>) -> SignedTranscript {
    let mut transcript = Vec::with_capacity(const { LINK_DOMAIN.len() + 16 + 16 });
    transcript.extend_from_slice(LINK_DOMAIN);
    transcript.extend_from_slice(channel_id.as_bytes());
    transcript.extend_from_slice(thread.as_bytes());
    SignedTranscript::new(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An erasure names one thread of one channel: changing either one
    /// changes what is signed.
    #[test]
    fn the_transcript_names_the_channel_and_the_thread() {
        let channel = Id::<Channel>::mint();
        let thread = Id::<Document>::mint();
        let base = erasure_transcript(channel, thread);
        assert!(base.as_bytes().starts_with(ERASURE_DOMAIN));
        assert_ne!(base, erasure_transcript(Id::<Channel>::mint(), thread));
        assert_ne!(base, erasure_transcript(channel, Id::<Document>::mint()));
    }

    /// A link receipt names one thread of one channel, under its own domain:
    /// it never reads as an erasure of the same thread.
    #[test]
    fn a_link_transcript_is_no_erasure_transcript() {
        let channel = Id::<Channel>::mint();
        let thread = Id::<Document>::mint();
        let link = link_transcript(channel, thread);
        assert!(link.as_bytes().starts_with(LINK_DOMAIN));
        assert_ne!(link, erasure_transcript(channel, thread));
        assert_ne!(link, link_transcript(Id::<Channel>::mint(), thread));
        assert_ne!(link, link_transcript(channel, Id::<Document>::mint()));
    }
}
