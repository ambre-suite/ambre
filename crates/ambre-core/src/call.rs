//! The pseudonyms the SFU knows a call by (`ambre-spec-call-media.md` section 3, `ZK-023`).
//!
//! The relay draws both from the generator of the OS. The SFU learns a call and its
//! participants under them alone: never the channel, the call identifier, a user or a key.

use crate::fixed_bytes::FixedRole;

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
