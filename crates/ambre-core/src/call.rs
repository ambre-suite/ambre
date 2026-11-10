//! The names of a call's media keys and chunks (`ambre-spec-call-media.md section 3`).
//!
//! `ambre-crypto` seals and opens a chunk, the Sync worker keeps the sender keys, and the
//! call worker holds them for one call. All three read these types, so the types live
//! here. No item of this module encrypts or derives anything.
//!
//! It also names the two pseudonyms the SFU knows a call by (`ZK-023`). The relay draws both
//! from the generator of the OS, so the SFU never learns the channel, the call, a user or a
//! key.

use crate::count::{Count, CountKind};
use crate::fixed_bytes::{FixedBytes, FixedRole};
use crate::payload::PayloadKind;
use crate::seq::{Seq, SeqKind, Wire};

/// The track a chunk belongs to: the first field of its key identifier.
///
/// The value is the one the key identifier carries in its two low bits. Each track of a
/// sender seals under a key of its own, since the key identifier enters the key schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Track {
    /// The microphone.
    Audio,
    /// The camera.
    Camera,
    /// A shared screen or window.
    Screen,
}

impl Track {
    /// The two bits the key identifier spells the track with.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Audio => 0,
            Self::Camera => 1,
            Self::Screen => 2,
        }
    }

    /// The track two bits spell, or `None` for the one value no track holds.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Audio),
            1 => Some(Self::Camera),
            2 => Some(Self::Screen),
            _ => None,
        }
    }
}

/// The index of one participant session in one call: the second field of a key
/// identifier. A reload is a new join with a new index, and no index is resumed.
pub enum SenderIndex {}

impl SeqKind for SenderIndex {
    type IssueRight = ();
    const NAME: &'static str = "SenderIndex";
}

/// A key identifier carries the index on the wire.
impl Wire for SenderIndex {}

/// The generation of one sender key: the third field of a key identifier.
///
/// A generation is an epoch and a step. A fresh random key opens the next epoch at step
/// zero. A ratchet moves the key one step up inside its epoch. So a ratchet and a fresh
/// key never land on one generation, and every generation of a sender strictly
/// increases.
pub enum KeyGeneration {}

impl SeqKind for KeyGeneration {
    type IssueRight = ();
    const NAME: &'static str = "KeyGeneration";
}

/// A key identifier carries the generation on the wire.
impl Wire for KeyGeneration {}

impl Seq<KeyGeneration> {
    /// The generation one ratchet reaches, or `None` past the last step of the epoch.
    #[must_use]
    pub const fn ratcheted(self) -> Option<Self> {
        // The low 32 bits are the step, the 16 bits above them the epoch.
        let step = self.get() & 0xFFFF_FFFF;
        if step == 0xFFFF_FFFF {
            return None;
        }
        Some(Self::new(self.get() + 1))
    }

    /// The first generation of the epoch after this one, for a fresh key, or `None` past
    /// the last epoch a key identifier carries.
    #[must_use]
    pub const fn next_epoch(self) -> Option<Self> {
        let next = ((self.get() >> 32) + 1) << 32;
        if next >= 1 << 48 {
            return None;
        }
        Some(Self::new(next))
    }

    /// The number of ratchets from `earlier` up to this generation, or `None` when the two
    /// lie in different epochs or `earlier` comes after.
    #[must_use]
    pub const fn ratchets_since(self, earlier: Self) -> Option<Count<Ratchets>> {
        if self.get() >> 32 != earlier.get() >> 32 || self.get() < earlier.get() {
            return None;
        }
        Some(Count::new(self.get() - earlier.get()))
    }

    /// Whether a key identifier can carry this generation.
    #[must_use]
    pub const fn fits_a_key_identifier(self) -> bool {
        self.get() < 1 << 48
    }
}

/// A number of ratchets of one sender key, inside one epoch.
pub enum Ratchets {}

impl CountKind for Ratchets {
    type Repr = u64;
    const NAME: &'static str = "Ratchets";
}

/// The role of one chunk of a call in the clear: `Payload<ClearChunk>`. The browser
/// encoder hands it to the transform, and the transform hands it to the decoder. Its
/// `Debug` form prints its length alone.
pub enum ClearChunk {}

impl PayloadKind for ClearChunk {
    const NAME: &'static str = "ClearChunk";
}

/// The role of one sealed chunk of a call, as it leaves the transform for the peer
/// connection or reaches it from there: `Payload<SealedChunk>`. The SFU forwards it and
/// opens none of it.
pub enum SealedChunk {}

impl PayloadKind for SealedChunk {
    const NAME: &'static str = "SealedChunk";
}

/// The bytes of one signed join of a call: the claim, then the Ed25519 signature of the
/// member's identity. The relay carries them from the joiner to each other participant,
/// and opens nothing of them (`CALL-057`).
pub enum SignedJoins {}

impl PayloadKind for SignedJoins {
    const NAME: &'static str = "SignedJoin";
}

/// The bytes of one wrap of a sender key, sealed to one participant. Only that participant
/// opens it: neither the relay nor the SFU reads it (`CALL-018`).
pub enum SenderKeyWraps {}

impl PayloadKind for SenderKeyWraps {
    const NAME: &'static str = "SenderKeyWrap";
}

/// The counter of one chunk under one key: the field of the chunk header after the key
/// identifier. One key seals each counter once (`CALL-005`).
pub enum ChunkCounter {}

impl SeqKind for ChunkCounter {
    type IssueRight = ();
    const NAME: &'static str = "ChunkCounter";
}

/// A chunk header carries the counter on the wire.
impl Wire for ChunkCounter {}

/// The key identifier of a chunk: which key of which sender sealed it.
///
/// The three fields name one key each. The chunk header spells them as one 64-bit
/// number. The generation takes the high 48 bits, the sender index the next 14 bits, and
/// the track the low 2 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyIdentifier {
    /// The track the key seals.
    pub track: Track,
    /// The participant session that holds the key.
    pub sender: Seq<SenderIndex>,
    /// The generation of the sender key.
    pub generation: Seq<KeyGeneration>,
}

impl KeyIdentifier {
    /// The 8 bytes the chunk header spells the identifier with, or `None` for a sender
    /// index or a generation past the width of its field.
    #[must_use]
    pub fn to_bytes(self) -> Option<FixedBytes<KeyIdentifierBytes>> {
        let fits = self.sender < SENDER_INDEX_LIMIT && self.generation.fits_a_key_identifier();
        fits.then(|| {
            let packed = (self.generation.get() << 16)
                | (self.sender.get() << 2)
                | u64::from(self.track.bits());
            FixedBytes::from_array(packed.to_be_bytes())
        })
    }

    /// The identifier 8 header bytes spell, or `None` for the track value no track holds.
    #[must_use]
    pub fn from_bytes(bytes: &FixedBytes<KeyIdentifierBytes>) -> Option<Self> {
        let packed = u64::from_be_bytes(*bytes.as_array());
        let track = Track::from_bits(u8::try_from(packed & 0b11).ok()?)?;
        Some(Self {
            track,
            sender: Seq::new((packed >> 2) & 0x3FFF),
            generation: Seq::new(packed >> 16),
        })
    }
}

/// The role of the 8 big-endian bytes a chunk header spells a key identifier with:
/// `FixedBytes<KeyIdentifierBytes>`. The generation takes the high 48 bits, the sender
/// index the next 14 bits, and the track the low 2 bits.
pub enum KeyIdentifierBytes {}

impl FixedRole for KeyIdentifierBytes {
    const NAME: &'static str = "KeyIdentifierBytes";
    type Array = [u8; 8];
}

/// The first sender index past the 14 bits a key identifier carries.
pub const SENDER_INDEX_LIMIT: Seq<SenderIndex> = Seq::new(1 << 14);

/// Bytes of a call pseudonym: 128 random bits, as many as a UUID holds.
pub const PSEUDONYM_LEN: usize = 16;

/// The role of the pseudonym of one call on the SFU: `FixedBytes<RoomPseudonym>`. The relay
/// draws it when the call starts, and every admission to that call names it.
pub enum RoomPseudonym {}

impl FixedRole for RoomPseudonym {
    const NAME: &'static str = "RoomPseudonym";
    type Array = [u8; PSEUDONYM_LEN];
}

/// The role of the pseudonym of one participant session on the SFU:
/// `FixedBytes<ParticipantPseudonym>`. The relay draws a fresh one for each join, so two
/// joins of one member share nothing the SFU reads.
pub enum ParticipantPseudonym {}

impl FixedRole for ParticipantPseudonym {
    const NAME: &'static str = "ParticipantPseudonym";
    type Array = [u8; PSEUDONYM_LEN];
}

#[cfg(test)]
mod tests {
    use super::{KeyGeneration, Seq, Track};
    use crate::count::Count;

    /// A ratchet stays in its epoch, and a fresh key never meets a ratchet of the epoch
    /// before.
    #[test]
    fn a_ratchet_and_a_fresh_epoch_never_share_a_generation() {
        let first = Seq::<KeyGeneration>::ZERO;
        let ratcheted = first.ratcheted();
        let fresh = first.next_epoch();
        assert_eq!(ratcheted.map(Seq::get), Some(1));
        assert_eq!(fresh.map(Seq::get), Some(1 << 32));
        assert_eq!(fresh.and_then(|f| f.ratchets_since(first)), None);
        assert_eq!(
            ratcheted
                .and_then(|r| r.ratchets_since(first))
                .map(Count::get),
            Some(1)
        );
        let last_step = Seq::<KeyGeneration>::new((1 << 32) - 1);
        assert_eq!(last_step.ratcheted(), None);
        let last_epoch = Seq::<KeyGeneration>::new(((1 << 16) - 1) << 32);
        assert_eq!(last_epoch.next_epoch(), None);
    }

    /// A key identifier reads back from its 8 bytes, and a field past its width has none.
    #[test]
    fn a_key_identifier_reads_back_from_its_bytes() {
        let kid = super::KeyIdentifier {
            track: Track::Screen,
            sender: Seq::new(0x3FFF),
            generation: Seq::new((1 << 48) - 1),
        };
        let bytes = kid.to_bytes();
        assert_eq!(
            bytes.as_ref().and_then(super::KeyIdentifier::from_bytes),
            Some(kid)
        );
        let wide = super::KeyIdentifier {
            sender: super::SENDER_INDEX_LIMIT,
            ..kid
        };
        assert_eq!(wide.to_bytes(), None);
    }

    /// Each track reads back from its two bits, and the fourth value names none.
    #[test]
    fn each_track_reads_back_from_its_bits() {
        for track in [Track::Audio, Track::Camera, Track::Screen] {
            assert_eq!(Track::from_bits(track.bits()), Some(track));
        }
        assert_eq!(Track::from_bits(3), None);
    }
}
