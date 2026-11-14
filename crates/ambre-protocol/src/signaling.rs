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

use std::fmt;

use ambre_core::id::{Channel, Id};
use ambre_core::{
    AccessToken, ByteArray, CallAdmission, Count, Credential, FixedBytes, FixedRole, Instant,
    Octets, ParticipantPseudonym, Payload, PayloadKind, RoomPseudonym, Rule, Sec, SenderIndex,
    SenderKeyWraps, Seq, Signature, SignedJoins, TimeSpan, TurnPassword, UnixSec, Validated,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::transcript::SignedTranscript;
use ambre_core::NarrowCount;

/// How many participants one call holds.
pub enum CallParticipants {}

impl NarrowCount for CallParticipants {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CallParticipants");
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
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("AdmissionToken");
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

/// The largest signed join the relay forwards. A signed join has one size, far under it.
pub const SIGNED_JOIN_MAX: Count<Octets> = Count::new(512);

/// A frame a client sends on the call socket of the relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallClientMsg {
    /// The first frame: the session's access token, and the channel the call belongs to. The
    /// call is the one the socket address names.
    Join {
        token: Credential<AccessToken>,
        channel: Id<Channel>,
    },
    /// The signed join of this session, under the sender index of its admission. The relay
    /// keeps it while the seat is open, and forwards it unchanged (`CALL-057`). Only the first
    /// one of a seat counts.
    Announce(Payload<SignedJoins>),
    /// One wrap of the sender's key, for the participant session of the sender index `to`
    /// alone (`CALL-058`).
    Wrap {
        to: Seq<SenderIndex>,
        wrap: Payload<SenderKeyWraps>,
    },
    /// The first frame of a session that starts the call: as [`CallClientMsg::Join`], and the
    /// relay creates the state of the call when it holds none. A join finds that state, or is
    /// refused as [`CallRefusal::Ended`] (`CALL-068`).
    Start {
        token: Credential<AccessToken>,
        channel: Id<Channel>,
    },
    /// The session leaves the call. The relay answers [`CallServerMsg::Departed`], then closes
    /// the socket (`CALL-069`).
    Leave,
}

/// Whether a leave emptied its call, as the relay answers it (`CALL-069`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Departure {
    /// Another seat holds the call.
    Others,
    /// The seat was the last of the call, and the relay dropped its state.
    Last,
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
    /// The call gave out every sender index a key identifier carries, and reuses none
    /// (`CALL-056`).
    Exhausted,
    /// The join starts no call, and the relay holds no state of it: the call ended
    /// (`CALL-068`).
    Ended,
}

/// One participant session of a call that announced its join, as a joiner learns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seated {
    /// The sender index the relay gave the session.
    pub sender: Seq<SenderIndex>,
    /// Its signed join, as the session announced it.
    pub join: Payload<SignedJoins>,
}

/// A frame the relay sends on the call socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallServerMsg {
    /// The join is admitted. The frame holds the admission to hand to the SFU, and the address
    /// of the SFU when the deployment runs one. It holds the sender index of this session, and
    /// each session in the call that announced its join (`CALL-056`, `CALL-057`).
    Admitted {
        admission: Credential<CallAdmission>,
        sfu: Option<Validated<SfuSocket>>,
        sender: Seq<SenderIndex>,
        roster: Vec<Seated>,
    },
    /// The join is refused, and the socket closes.
    Refused(CallRefusal),
    /// A participant session announced its join.
    Joined(Seated),
    /// The participant session of this sender index left the call.
    Left(Seq<SenderIndex>),
    /// A wrap a participant sent to this session's sender index.
    Wrap {
        from: Seq<SenderIndex>,
        to: Seq<SenderIndex>,
        wrap: Payload<SenderKeyWraps>,
    },
    /// The relay lost the state of the call, and the socket closes after this notice. A
    /// rejoin is refused as ended (`SRV-249`, `CALL-068`).
    Lost,
    /// The answer to [`CallClientMsg::Leave`]: whether the seat was the last of the call. The
    /// socket closes after it (`CALL-069`).
    Departed(Departure),
}

/// What the relay answers an ask of who is in a call (`CALL-070`), as the JSON of
/// `GET /api/calls/{call}/roster`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallRoster {
    /// The relay holds no state of the call in that channel, or the asker is no member of it
    /// (`CALL-071`).
    Gone,
    /// The seat of each session that announced its join, by sender index.
    Live(Vec<Seated>),
}

/// The address of the signalling socket of the SFU: `ws:` or `wss:`, then a host and a path.
/// The relay names it in an admission, from its configuration.
pub enum SfuSocket {}

impl Rule for SfuSocket {
    type Error = &'static str;
    const LOGGED_AS: Option<&'static str> = Some("SfuSocket");

    fn admit(text: String) -> Result<String, Self::Error> {
        let scheme = text.starts_with("ws://") || text.starts_with("wss://");
        let printable = text.bytes().all(|b| b.is_ascii_graphic());
        if scheme && printable && text.len() <= 256 {
            Ok(text)
        } else {
            Err("an SFU socket starts with ws:// or wss://, and holds no space")
        }
    }
}

/// A TURN server address a browser reads: `turn:` or `turns:`, then a host and a query.
pub enum TurnUrl {}

impl Rule for TurnUrl {
    type Error = &'static str;
    const LOGGED_AS: Option<&'static str> = Some("TurnUrl");

    fn admit(text: String) -> Result<String, Self::Error> {
        let scheme = text.starts_with("turn:") || text.starts_with("turns:");
        let printable = text.bytes().all(|b| b.is_ascii_graphic());
        if scheme && printable && text.len() <= 256 {
            Ok(text)
        } else {
            Err("a TURN URL starts with turn: or turns:, and holds no space")
        }
    }
}

/// The username of a TURN credential: the instant it expires, then the participant's
/// pseudonym, as coturn reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnUsername {
    expires: Instant<UnixSec>,
    participant: FixedBytes<ParticipantPseudonym>,
}

impl TurnUsername {
    /// The username of `participant`, valid until `expires`.
    #[must_use]
    pub const fn new(
        expires: Instant<UnixSec>,
        participant: FixedBytes<ParticipantPseudonym>,
    ) -> Self {
        Self {
            expires,
            participant,
        }
    }
}

impl fmt::Display for TurnUsername {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.expires.get(), self.participant.to_hex())
    }
}

impl Serialize for TurnUsername {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TurnUsername {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let text = String::deserialize(input)?;
        let (expires, participant) = text
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("a TURN username holds a colon"))?;
        Ok(Self {
            expires: Instant::new(expires.parse().map_err(serde::de::Error::custom)?),
            participant: FixedBytes::from_hex(participant).map_err(serde::de::Error::custom)?,
        })
    }
}

/// One entry of the ICE servers a browser hands its peer connection, as `RTCIceServer`
/// spells it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IceServer {
    /// The addresses of the TURN server.
    pub urls: Vec<Validated<TurnUrl>>,
    /// The username of the credential.
    pub username: TurnUsername,
    /// The password of the credential.
    pub credential: Credential<TurnPassword>,
}

/// A session description of the peer connection of a call, as SDP text. The SFU parses it;
/// the Sync worker carries it between the socket of the SFU and the UI thread.
pub enum SessionDescription {}

impl Rule for SessionDescription {
    type Error = &'static str;
    const LOGGED_AS: Option<&'static str> = Some("SessionDescription");

    fn admit(text: String) -> Result<String, Self::Error> {
        if text.len() <= SESSION_DESCRIPTION_MAX.saturating_len().to_usize() {
            Ok(text)
        } else {
            Err("a session description is longer than the SFU reads")
        }
    }
}

/// The longest session description the socket of the SFU carries. Twelve slots of media
/// stay far under it.
pub const SESSION_DESCRIPTION_MAX: Count<Octets> = Count::new(64 * 1024);

/// A message on the signalling socket of the SFU, as JSON of one key. The client sends
/// `{"admission": "<token>"}` first, and the SFU answers `{"ice": [...]}`. Then the session
/// descriptions go in either direction: `{"offer": "<sdp>"}` or `{"answer": "<sdp>"}`. The
/// client sends `"share"` when its screen sender takes a track, which starts a share
/// (`CALL-106`). The SFU sends `"unshared"` to a sharer once a later share took the screen
/// (`CALL-107`). These are all the SFU reads of a participant (`ZK-023`, `ZK-033`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SfuMsg {
    Admission(Credential<CallAdmission>),
    Ice(Vec<IceServer>),
    Offer(Validated<SessionDescription>),
    Answer(Validated<SessionDescription>),
    Share,
    Unshared,
}
