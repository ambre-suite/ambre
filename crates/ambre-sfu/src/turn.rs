//! The TURN credentials the SFU mints for each participant (`CALL-014`).
//!
//! A browser whose network blocks UDP reaches the SFU through a TURN relay: coturn, in its
//! `use-auth-secret` mode. A username is an expiry and a name. Its password is an HMAC of the
//! username under a secret that coturn and the SFU share. The SFU mints one for each admitted participant, under its pseudonym, so neither
//! coturn nor the relay of Ambre learns who the participant is. The relay holds no part of it.

use std::fmt;

use ambre_core::{
    Credential, FixedBytes, Instant, ParticipantPseudonym, Rule, Sec, TimeSpan, TurnPassword,
    UnixSec, Validated,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::secrets::TurnSecret;

/// How long a TURN credential admits. coturn checks it on every refresh of an allocation, so
/// it outlasts any call.
pub const TURN_LIFETIME: TimeSpan<Sec> = TimeSpan::new(12 * 3600);

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
#[derive(Debug, Serialize, Deserialize)]
pub struct IceServer {
    /// The addresses of the TURN server.
    pub urls: Vec<Validated<TurnUrl>>,
    /// The username of the credential.
    pub username: TurnUsername,
    /// The password of the credential.
    pub credential: Credential<TurnPassword>,
}

/// The TURN servers the SFU hands each participant, and the secret it mints with.
#[derive(Debug)]
pub struct TurnServers {
    urls: Vec<Validated<TurnUrl>>,
    secret: TurnSecret,
}

impl TurnServers {
    /// The servers at `urls`, minted for under `secret`.
    pub fn new(urls: Vec<Validated<TurnUrl>>, secret: TurnSecret) -> Self {
        Self { urls, secret }
    }

    /// A credential for `participant`, minted at `now`.
    pub fn credential(
        &self,
        participant: FixedBytes<ParticipantPseudonym>,
        now: Instant<UnixSec>,
    ) -> IceServer {
        let username = TurnUsername::new(now.plus(TURN_LIFETIME), participant);
        IceServer {
            urls: self.urls.clone(),
            credential: self.secret.mint(&username),
            username,
        }
    }
}
