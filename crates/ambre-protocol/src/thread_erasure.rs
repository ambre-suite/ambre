//! What an erasure of a thread name signs (`SYNC-322`, `SRV-347`).
//!
//! Any member sets the absent `thread` field of a root. A field that names no document of the channel fails each removal, as the
//! census refuses a listing short of a thread (`SYNC-321`). An owner of the
//! channel signs an erasure of that name. The relay records it, and every
//! census then passes that thread. The owner signs the channel and the thread,
//! so the relay cannot replay the signature against another thread.

use ambre_core::id::{Channel, Document, Id};

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
}
