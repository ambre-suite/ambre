//! The TURN credentials the SFU mints for each participant (`CALL-014`).
//!
//! A browser whose network blocks UDP reaches the SFU through a TURN relay: coturn, in its
//! `use-auth-secret` mode. A username is an expiry and a name. Its password is an HMAC of the
//! username under a secret that coturn and the SFU share. The SFU mints one for each admitted participant, under its pseudonym, so neither
//! coturn nor the relay of Ambre learns who the participant is. The relay holds no part of it.

use ambre_core::{FixedBytes, Instant, ParticipantPseudonym, Sec, TimeSpan, UnixSec, Validated};

use crate::secrets::TurnSecret;

/// How long a TURN credential admits. coturn checks it on every refresh of an allocation, so
/// it outlasts any call.
pub const TURN_LIFETIME: TimeSpan<Sec> = TimeSpan::new(12 * 3600);

pub use ambre_protocol::signaling::{IceServer, TurnUrl, TurnUsername};

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
