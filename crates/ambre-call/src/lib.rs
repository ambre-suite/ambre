//! The call worker: the third instance of the client during a call
//! (`ambre-spec-call-media.md section 3`, `ambre-spec-architecture.md section 2`).
//!
//! The browser hands each encoded audio and video chunk to a transform that runs here. The worker
//! seals each chunk a sender hands it, and opens each chunk a receiver hands it, before the
//! decoder reads it (`CALL-001`, `CALL-003`, `CALL-022`). It holds the sender keys of its
//! current call alone, which the Sync worker hands it on a port of their own (`CALL-004`,
//! `CALL-064`). The end of the call drops every one of them (`CALL-029`).
//!
//! [`CallWorker`] decides, and reads no browser object. So the host tier runs the seal and
//! the open the browser runs. The wasm entry in `worker.rs` carries the frames and the port.

use ambre_core::{
    ClearChunk, Instant, KeyGeneration, KeyIdentifier, MediaSlot, PageLoad, Payload, PictureKind,
    SealedChunk, SenderIndex, Seq, SlotSender, Track,
};
use ambre_crypto::call::{ChunkError, ChunkSealer, KeyHandoff, ReceiveKeys};

#[cfg(target_arch = "wasm32")]
mod worker;

/// What the call worker holds: the sealers of this session's audio, camera and screen, and the
/// keys of the other participants of the current call. No other field holds a key (`CALL-004`).
#[derive(Default)]
pub struct CallWorker {
    /// The sealer of this session's audio, once the Sync worker handed its key.
    sealer: Option<ChunkSealer>,
    /// The sealer of this session's camera, under the same sender key as its audio.
    camera: Option<ChunkSealer>,
    /// The sealer of this session's screen, under the key identifier of the screen track
    /// (`CALL-102`).
    screen: Option<ChunkSealer>,
    /// The keys of the other participants of the call.
    receive: ReceiveKeys,
    /// The sender index and the generation of the last `Seal` handoff the worker took. A new
    /// sealer issues its counters from 1 again. So a `Seal` at or below this generation would
    /// seal a chunk under a key and a nonce already used (`CALL-147`).
    sealed: Option<(Seq<SenderIndex>, Seq<KeyGeneration>)>,
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
    slot: MediaSlot,
    last: Option<Seq<SenderIndex>>,
}

impl SlotWatch {
    /// The watch of the slot `slot`, which has opened no chunk yet.
    #[must_use]
    pub const fn new(slot: MediaSlot) -> Self {
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
    /// The VP8 chunk is shorter than its clear prefix, or its keyframe bit and its kind differ
    /// (`CALL-097`).
    Refused,
}

/// What a handoff did to the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// The worker took the key, or dropped the keys of one sender.
    Kept,
    /// The worker dropped a `Seal` that does not move its generation up: it keeps sealing
    /// under the sealers it holds (`CALL-147`).
    Refused,
    /// The call ended, and the worker holds no key of it any more (`CALL-029`).
    Erased,
}

impl CallWorker {
    /// Takes one handoff of the Sync worker at `now`, on the monotonic clock of the worker. A
    /// `Seal` of a higher generation replaces the sealer, so the next chunk seals under the new
    /// key. A `Seal` at or below the last one is refused, so no counter starts again under a key
    /// identifier already sealed under (`CALL-147`). An `End` drops every key of the call. An `Open` of a fresh epoch closes the older epochs of its sender
    /// once the switch delay passed after `now` (`CALL-135`). A `Close` of a removal closes the
    /// epochs it names once its delay passed after `now`, whether a fresh key arrives or not
    /// (`CALL-157`).
    pub fn hand(&mut self, handoff: KeyHandoff, now: Instant<PageLoad>) -> Handled {
        match handoff {
            KeyHandoff::Seal {
                sender,
                generation,
                key,
            } => {
                if self
                    .sealed
                    .is_some_and(|(held, top)| held == sender && generation <= top)
                {
                    return Handled::Refused;
                }
                self.sealed = Some((sender, generation));
                let sealer_of = |track| {
                    let kid = KeyIdentifier {
                        track,
                        sender,
                        generation,
                    };
                    // A key identifier past the width of the header seals nothing: the chunks
                    // drop until the Sync worker hands a key that fits.
                    ChunkSealer::new(&key, kid).ok()
                };
                self.sealer = sealer_of(Track::Audio);
                self.camera = sealer_of(Track::Camera);
                self.screen = sealer_of(Track::Screen);
                Handled::Kept
            }
            KeyHandoff::Open {
                sender,
                generation,
                key,
            } => {
                self.receive.insert(sender, generation, key, now);
                Handled::Kept
            }
            KeyHandoff::Forget { sender } => {
                self.receive.forget(sender);
                Handled::Kept
            }
            KeyHandoff::Close {
                sender,
                generation,
                within,
            } => {
                self.receive.close(sender, generation, within, now);
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

    /// Seals one clear VP8 chunk of the camera or of the screen, as `track` names, of the kind
    /// its encoder names. Each track seals under its own key identifier (`CALL-102`). Its clear
    /// prefix leaves unchanged, so the SFU reads it (`CALL-023`).
    ///
    /// # Errors
    ///
    /// [`Unsealed::NoKey`] when the worker holds no sealer of `track` yet, audio included.
    /// [`Unsealed::Refused`] for a chunk shorter than its prefix or whose keyframe bit and kind
    /// differ (`CALL-097`). The chunk is dropped, never sent in the clear (`CALL-001`).
    pub fn seal_video(
        &mut self,
        track: Track,
        clear: &Payload<ClearChunk>,
        kind: PictureKind,
    ) -> Result<Payload<SealedChunk>, Unsealed> {
        let sealer = match track {
            Track::Camera => self.camera.as_mut(),
            Track::Screen => self.screen.as_mut(),
            Track::Audio => None,
        }
        .ok_or(Unsealed::NoKey)?;
        sealer.seal_video(clear, kind).map_err(|e| match e {
            ChunkError::Malformed | ChunkError::KindMismatch => Unsealed::Refused,
            _ => Unsealed::NoKey,
        })
    }

    /// Opens one chunk the network handed a receiver of `track` at `now`, or `None` when no key
    /// the worker holds opens it. A `None` chunk is dropped, and never reaches the decoder
    /// (`CALL-022`). A retired epoch opens nothing (`CALL-125`, `CALL-135`).
    pub fn open(
        &mut self,
        track: Track,
        sealed: Payload<SealedChunk>,
        now: Instant<PageLoad>,
    ) -> Option<Opened> {
        match track {
            Track::Audio => self.receive.open_audio(sealed, now),
            Track::Camera => self.receive.open_video(sealed, now),
            Track::Screen => self.receive.open_screen(sealed, now),
        }
        .ok()
        .map(|(sender, clear)| Opened { sender, clear })
    }

    /// Whether the worker holds no key: no sealer, and no key of another participant.
    #[must_use]
    pub fn holds_no_key(&self) -> bool {
        let Self {
            sealer,
            camera,
            screen,
            receive,
            sealed: _,
        } = self;
        sealer.is_none() && camera.is_none() && screen.is_none() && receive.is_empty()
    }
}

#[cfg(test)]
mod tests;
