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

use ambre_core::index::Pos;
use ambre_core::{
    AudioSlots, ClearChunk, KeyIdentifier, Payload, SealedChunk, SenderIndex, Seq, SlotSender,
    Track,
};
use ambre_crypto::call::{ChunkError, ChunkSealer, KeyHandoff, ReceiveKeys};

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

/// One chunk the worker opened: the sender its key identifier names, and the clear chunk for
/// the decoder.
#[derive(Debug)]
pub struct Opened {
    /// The sender of the chunk, read from its key identifier.
    pub sender: Seq<SenderIndex>,
    /// The chunk in the clear.
    pub clear: Payload<ClearChunk>,
}

/// The sender one receive transform carried last. The SFU moves a slot to another sender at
/// the next packet (`CALL-047`), so the page learns of each move from here (`CALL-084`).
#[derive(Debug)]
pub struct SlotWatch {
    slot: Pos<AudioSlots>,
    last: Option<Seq<SenderIndex>>,
}

impl SlotWatch {
    /// The watch of the slot `slot`, which has opened no chunk yet.
    #[must_use]
    pub const fn new(slot: Pos<AudioSlots>) -> Self {
        Self { slot, last: None }
    }

    /// Reads the sender of a chunk the slot opened. Answers the note for the page at the first
    /// chunk of a new sender, and `None` at every other chunk.
    pub fn opened(&mut self, sender: Seq<SenderIndex>) -> Option<SlotSender> {
        if self.last == Some(sender) {
            return None;
        }
        self.last = Some(sender);
        Some(SlotSender {
            slot: self.slot,
            sender,
        })
    }
}

/// Why the worker sent no sealed chunk for a clear one. The chunk is dropped either way, never
/// sent in the clear (`CALL-001`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsealed {
    /// The Sync worker handed no key yet, or none that fits the header.
    NoKey,
    /// The clear chunk is past the fixed size of a sealed chunk (`CALL-067`).
    TooLarge,
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

    /// Seals one clear chunk of the encoder.
    ///
    /// # Errors
    ///
    /// [`Unsealed`] when the worker holds no sealer yet, or the chunk is past the fixed size.
    /// The chunk is dropped, never sent in the clear (`CALL-001`).
    pub fn seal(&mut self, clear: &Payload<ClearChunk>) -> Result<Payload<SealedChunk>, Unsealed> {
        let sealer = self.sealer.as_mut().ok_or(Unsealed::NoKey)?;
        sealer.seal(clear).map_err(|e| match e {
            ChunkError::ChunkTooLarge => Unsealed::TooLarge,
            _ => Unsealed::NoKey,
        })
    }

    /// Opens one chunk the network handed a receiver, or `None` when no key the worker holds
    /// opens it. A `None` chunk is dropped, and never reaches the decoder (`CALL-022`).
    pub fn open(&mut self, sealed: Payload<SealedChunk>) -> Option<Opened> {
        self.receive
            .open_audio(sealed)
            .ok()
            .map(|(sender, clear)| Opened { sender, clear })
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
