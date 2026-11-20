//! The TURN credentials the SFU mints for each participant (`CALL-014`).
//!
//! A browser whose network blocks UDP reaches the SFU through a TURN relay: coturn, in its
//! `use-auth-secret` mode. A username is an expiry and a name. Its password is an HMAC of the
//! username under a secret that coturn and the SFU share. The SFU mints one for each admitted participant, under its pseudonym, so neither
//! coturn nor the relay of Ambre learns who the participant is. The relay holds no part of it.
//!
//! coturn checks the expiry when an allocation first authenticates, then keeps the key of the
//! allocation for each refresh (`ns_turn_server.c`, `check_stun_auth`). So the lifetime bounds
//! when a credential opens a new allocation, never how long one lasts (audit `P60`). The SFU
//! mints a fresh credential on a renewal of the seat once the last one nears its expiry, and
//! the page restarts ICE with it (`CALL-153`, `CALL-154`).

use ambre_core::{FixedBytes, Instant, ParticipantPseudonym, Sec, TimeSpan, UnixSec, Validated};

use crate::secrets::TurnSecret;

/// How long a TURN credential admits a new allocation, the row of `ambre-spec-budgets.md`
/// section 1. A member removed from the call opens no allocation past it (audit `P60`).
pub const TURN_LIFETIME: TimeSpan<Sec> = TimeSpan::new(300);

/// How long before the expiry of the last credential a renewal of the seat mints a fresh one,
/// the same row. Three heartbeats of the relay fall in it, so one late beat still brings the
/// fresh credential before the expiry.
pub const TURN_MARGIN: TimeSpan<Sec> = TimeSpan::new(60);

pub use ambre_protocol::signaling::{IceServer, TurnUrl, TurnUsername};

/// The TURN servers the SFU hands each participant, and the secret it mints with.
#[derive(Debug)]
pub struct TurnServers {
    urls: Vec<Validated<TurnUrl>>,
    secret: TurnSecret,
    lifetime: TimeSpan<Sec>,
    margin: TimeSpan<Sec>,
}

impl TurnServers {
    /// The servers at `urls`, minted for under `secret`, for [`TURN_LIFETIME`] each.
    pub fn new(urls: Vec<Validated<TurnUrl>>, secret: TurnSecret) -> Self {
        Self {
            urls,
            secret,
            lifetime: TURN_LIFETIME,
            margin: TURN_MARGIN,
        }
    }

    /// The same servers, each credential valid for `lifetime`, and renewed `margin` before
    /// its expiry. A test shortens both to cross an expiry within its run.
    #[must_use]
    pub fn lasting(self, lifetime: TimeSpan<Sec>, margin: TimeSpan<Sec>) -> Self {
        Self {
            lifetime,
            margin,
            ..self
        }
    }

    /// A credential for `participant`, minted at `now`. It expires at [`Self::expiry`] of
    /// `now`.
    pub fn credential(
        &self,
        participant: FixedBytes<ParticipantPseudonym>,
        now: Instant<UnixSec>,
    ) -> IceServer {
        let username = TurnUsername::new(self.expiry(now), participant);
        IceServer {
            urls: self.urls.clone(),
            credential: self.secret.mint(&username),
            username,
        }
    }

    /// The instant a credential minted at `now` expires.
    pub const fn expiry(&self, now: Instant<UnixSec>) -> Instant<UnixSec> {
        now.plus(self.lifetime)
    }

    /// Whether a renewal at `now` mints a fresh credential, for a participant whose last one
    /// expires at `held`: the last one expires within the margin, or expired already.
    pub fn is_due(&self, held: Instant<UnixSec>, now: Instant<UnixSec>) -> bool {
        held.since(now) <= self.margin
    }
}

#[cfg(test)]
mod tests {
    use ambre_core::{FixedBytes, Instant, TimeSpan, UnixSec, Validated};

    use super::{TURN_LIFETIME, TURN_MARGIN, TurnServers, TurnUrl};
    use crate::secrets::TurnSecret;

    /// One TURN server under a fixed secret, at the production lifetime.
    fn servers() -> TurnServers {
        TurnServers::new(
            vec![Validated::<TurnUrl>::parse("turn:127.0.0.1:3478?transport=tcp").expect("a URL")],
            TurnSecret::new("north-wind".to_owned()),
        )
    }

    /// `CALL-153`, audit `P60`: a credential lives the lifetime of the budget row, never
    /// hours. Its username names that expiry, which coturn reads.
    #[test]
    fn a_credential_expires_one_turn_lifetime_after_it_is_minted() {
        let now = Instant::<UnixSec>::new(1_700_000_000);
        let ice = servers().credential(FixedBytes::from_array([0xab; 16]), now);
        assert_eq!(
            ice.username.to_string(),
            format!("{}:{}", 1_700_000_000 + 300, "ab".repeat(16))
        );
        assert_eq!(TURN_LIFETIME, TimeSpan::new(300));
        assert!(
            TURN_MARGIN < TURN_LIFETIME,
            "a margin past the lifetime renews on every beat"
        );
    }

    /// `CALL-153`: a renewal mints a fresh credential once the last one expires within the
    /// margin, or expired already, and never before. So a call restarts ICE once per
    /// lifetime, and not on every heartbeat.
    #[test]
    fn a_renewal_mints_a_fresh_credential_within_the_margin_alone() {
        let turn = servers();
        let minted = Instant::<UnixSec>::new(1_700_000_000);
        let held = turn.expiry(minted);
        let at = |after: u64| minted.plus(TimeSpan::new(after));
        assert!(
            !turn.is_due(held, at(0)),
            "a fresh credential is renewed at once"
        );
        assert!(!turn.is_due(held, at(239)), "renewed before the margin");
        assert!(
            turn.is_due(held, at(240)),
            "not renewed at the start of the margin"
        );
        assert!(turn.is_due(held, at(299)));
        assert!(turn.is_due(held, at(300)), "not renewed at the expiry");
        assert!(turn.is_due(held, at(900)), "not renewed once expired");

        let short = servers().lasting(TimeSpan::new(30), TimeSpan::new(6));
        let held = short.expiry(minted);
        assert_eq!(held, at(30));
        assert!(!short.is_due(held, at(23)));
        assert!(short.is_due(held, at(24)));
    }
}
