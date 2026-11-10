//! The signalling of a call (`ambre-spec-call-media.md` section 3).
//!
//! Two sockets carry it, and neither is the socket of the document plane (`SRV-049`):
//!
//! * the call socket of the relay, which checks the channel, keeps the roster, forwards the
//!   opaque wraps of the sender keys and issues an admission;
//! * the signalling socket of the SFU, which reads the admission and the session descriptions.
//!
//! An admission names the call and the participant by two pseudonyms the relay draws. So the
//! SFU learns no channel, no user and no key (`ZK-023`). The relay signs it; the SFU
//! verifies it with the public key of the relay alone.

use ambre_core::id::{Channel, Id};
use ambre_core::{
    AccessToken, ByteArray, CallAdmission, Count, CountKind, Credential, FixedBytes, FixedRole,
    Instant, Octets, ParticipantPseudonym, Payload, PayloadKind, PublicKey, RoomPseudonym, Sec,
    Signature, TimeSpan, UnixSec,
};
use serde::{Deserialize, Serialize};

use crate::transcript::SignedTranscript;

/// How many participants one call holds.
pub enum CallParticipants {}

impl CountKind for CallParticipants {
    type Repr = u32;
    const NAME: &'static str = "CallParticipants";
}

/// The participant cap of a call, the row of `ambre-spec-budgets.md` section 1. The relay
/// refuses a join beyond it with a notice (`CALL-021`), and the SFU refuses one too.
pub const PARTICIPANT_CAP: Count<CallParticipants> = Count::new(50);

/// How long an admission admits, from the instant the relay issues it. The client opens the
/// socket of the SFU at once, so a stolen admission is worth little for long.
pub const ADMISSION_LIFETIME: TimeSpan<Sec> = TimeSpan::new(60);

/// The largest wrap the relay forwards. A wrap holds one sender key sealed to one ephemeral
/// key, with its binding, so it stays far under this size.
pub const WRAP_MAX: Count<Octets> = Count::new(1024);

/// The domain an admission signature covers first, so no signature of the relay over other
/// bytes, such as an access token, verifies as an admission.
const ADMISSION_DOMAIN: &[u8] = b"ambre-call-admission-v1";

/// The decoded bytes of an admission token: the encoded admission, then the relay's signature.
/// The SFU reads them from the text of the token, and nothing else builds them.
pub enum AdmissionTokens {}

impl PayloadKind for AdmissionTokens {
    const NAME: &'static str = "AdmissionToken";
}

/// The bytes of one wrap of a sender key, sealed to one participant. Only that participant
/// opens it: neither the relay nor the SFU reads it (`CALL-018`).
pub enum SenderKeyWraps {}

impl PayloadKind for SenderKeyWraps {
    const NAME: &'static str = "SenderKeyWrap";
}

/// What the SFU learns of one participant at its join (`ZK-023`). It names the call and the
/// participant, each by a pseudonym, and the instant the admission expires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    /// The call, as the SFU knows it.
    pub room: FixedBytes<RoomPseudonym>,
    /// The participant session, as the SFU knows it.
    pub participant: FixedBytes<ParticipantPseudonym>,
    /// The instant the SFU stops accepting the admission.
    pub expires: Instant<UnixSec>,
}

/// Why the bytes of a token spell no admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionDecodeError {
    /// The bytes hold no signature after the admission.
    #[error("the token is too short to hold a signature")]
    Truncated,
    /// The bytes before the signature are no admission.
    #[error("the token holds no admission")]
    Malformed,
}

impl Admission {
    /// The bytes the relay signs and the SFU verifies: the domain, then the admission.
    ///
    /// # Errors
    ///
    /// The error of the encoder, which a struct of fixed fields never meets.
    pub fn signed_bytes(&self) -> Result<SignedTranscript, postcard::Error> {
        let mut bytes = ADMISSION_DOMAIN.to_vec();
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(SignedTranscript::new(bytes))
    }

    /// The token that carries the admission and the relay's `signature` of its
    /// [`Admission::signed_bytes`]: the encoded admission then the signature, in hex.
    ///
    /// # Errors
    ///
    /// The error of the encoder, which a struct of fixed fields never meets.
    pub fn token(
        &self,
        signature: &FixedBytes<Signature>,
    ) -> Result<Credential<CallAdmission>, postcard::Error> {
        let mut bytes = postcard::to_allocvec(self)?;
        bytes.extend_from_slice(signature.as_slice());
        Ok(Credential::new(hex::encode(bytes)))
    }

    /// The admission and the signature that the decoded bytes of a token hold. It checks no
    /// signature: the SFU verifies the result against the relay's key.
    ///
    /// # Errors
    ///
    /// [`AdmissionDecodeError`] when the bytes spell no token.
    pub fn read(
        token: &Payload<AdmissionTokens>,
    ) -> Result<(Self, FixedBytes<Signature>), AdmissionDecodeError> {
        let token = token.as_bytes();
        let split = token
            .len()
            .checked_sub(<<Signature as FixedRole>::Array as ByteArray>::LEN)
            .ok_or(AdmissionDecodeError::Truncated)?;
        let (admission, signature) = token.split_at(split);
        let signature =
            FixedBytes::from_slice(signature).map_err(|_| AdmissionDecodeError::Truncated)?;
        let admission: Self =
            postcard::from_bytes(admission).map_err(|_| AdmissionDecodeError::Malformed)?;
        Ok((admission, signature))
    }
}

/// A frame a client sends on the call socket of the relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallClientMsg {
    /// The first frame: the session's access token, and the channel the call belongs to. The
    /// call is the one the socket address names.
    Join {
        token: Credential<AccessToken>,
        channel: Id<Channel>,
    },
    /// One wrap of the sender's key, for every participant session under the key `to`.
    Wrap {
        to: FixedBytes<PublicKey>,
        wrap: Payload<SenderKeyWraps>,
    },
}

/// Why the relay refuses a join. The socket closes after the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallRefusal {
    /// The first frame is no join, or its token does not verify.
    Unauthorized,
    /// The caller is no member of the channel, or the call belongs to another channel.
    NotMember,
    /// The call holds [`PARTICIPANT_CAP`] participants already (`CALL-021`).
    Full,
    /// The call has ended: its last participant left.
    Ended,
}

/// A frame the relay sends on the call socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallServerMsg {
    /// The join is admitted: the admission to hand to the SFU, and the key of each
    /// participant session already in the call.
    Admitted {
        admission: Credential<CallAdmission>,
        roster: Vec<FixedBytes<PublicKey>>,
    },
    /// The join is refused, and the socket closes.
    Refused(CallRefusal),
    /// A participant session under this key joined the call.
    Joined(FixedBytes<PublicKey>),
    /// A participant session under this key left the call.
    Left(FixedBytes<PublicKey>),
    /// A wrap a participant sent to this socket's key.
    Wrap {
        from: FixedBytes<PublicKey>,
        to: FixedBytes<PublicKey>,
        wrap: Payload<SenderKeyWraps>,
    },
}
