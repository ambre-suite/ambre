//! The call worker: the third instance of the client during a call
//! (`ambre-spec-call-media.md section 3`, `ambre-spec-architecture.md section 2`).
//!
//! The browser hands each encoded audio chunk to a transform that runs here. The worker
//! seals each chunk a sender hands it, and opens each chunk a receiver hands it, before the
//! decoder reads it (`CALL-001`, `CALL-003`, `CALL-022`). It holds the sender keys of its
//! current call alone, which the Sync worker hands it on a port of their own (`CALL-004`,
//! `CALL-064`). The end of the call drops every one of them (`CALL-029`).
//!
//! [`CallWorker`] decides, and reads no browser object. So the host tier runs the seal and
//! the open the browser runs. The wasm entry in `worker.rs` carries the frames and the port.

use ambre_core::{ClearChunk, KeyIdentifier, Payload, SealedChunk, Track};
use ambre_crypto::call::{ChunkSealer, KeyHandoff, ReceiveKeys};

#[cfg(target_arch = "wasm32")]
mod worker;

/// What the call worker holds: the sealer of this session's audio, and the keys of the other
/// participants of the current call. No other field holds a key (`CALL-004`).
#[derive(Default)]
pub struct CallWorker {
    /// The sealer of this session's audio, once the Sync worker handed its key.
    sealer: Option<ChunkSealer>,
    /// The keys of the other participants of the call.
    receive: ReceiveKeys,
}

/// What a handoff did to the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// The worker took the key, or dropped the keys of one sender.
    Kept,
    /// The call ended, and the worker holds no key of it any more (`CALL-029`).
    Erased,
}

impl CallWorker {
    /// Takes one handoff of the Sync worker. A `Seal` replaces the sealer, so the next chunk
    /// seals under the new key; an `End` drops every key of the call.
    pub fn hand(&mut self, handoff: KeyHandoff) -> Handled {
        match handoff {
            KeyHandoff::Seal {
                sender,
                generation,
                key,
            } => {
                let kid = KeyIdentifier {
                    track: Track::Audio,
                    sender,
                    generation,
                };
                // A key identifier past the width of the header seals nothing: the chunks
                // drop until the Sync worker hands a key that fits.
                self.sealer = ChunkSealer::new(&key, kid).ok();
                Handled::Kept
            }
            KeyHandoff::Open {
                sender,
                generation,
                key,
            } => {
                self.receive.insert(sender, generation, key);
                Handled::Kept
            }
            KeyHandoff::Forget { sender } => {
                self.receive.forget(sender);
                Handled::Kept
            }
            KeyHandoff::End => {
                // Each key sits in a `Secret` that wipes it when its last owner drops it.
                *self = Self::default();
                Handled::Erased
            }
        }
    }

    /// Seals one clear chunk of the encoder, or `None` when the worker holds no sealer yet or
    /// the chunk is past the fixed size. A `None` chunk is dropped, never sent in the clear
    /// (`CALL-001`).
    pub fn seal(&mut self, clear: &Payload<ClearChunk>) -> Option<Payload<SealedChunk>> {
        self.sealer.as_mut()?.seal(clear).ok()
    }

    /// Opens one chunk the network handed a receiver, or `None` when no key the worker holds
    /// opens it. A `None` chunk is dropped, and never reaches the decoder (`CALL-022`).
    pub fn open(&mut self, sealed: Payload<SealedChunk>) -> Option<Payload<ClearChunk>> {
        self.receive.open_audio(sealed).ok().map(|(_, clear)| clear)
    }

    /// Whether the worker holds no key: no sealer, and no key of another participant.
    #[must_use]
    pub fn holds_no_key(&self) -> bool {
        let Self { sealer, receive } = self;
        sealer.is_none() && receive.is_empty()
    }
}

#[cfg(test)]
mod tests;
