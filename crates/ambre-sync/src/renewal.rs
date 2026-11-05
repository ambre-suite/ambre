//! The renewal of the access token ahead of its expiry, as a state machine
//! over an injected clock (`SYNC-183`). The relay announces how long each
//! access token lives. The worker renews the pair a margin before that
//! lifetime runs out, so no request and no reconnect meets a refusal. The
//! refusal stays the fallback (`SYNC-062`, `SYNC-063`). The timer shell
//! only schedules the delay this machine returns, so a host test steps it by
//! advancing the clock (`ambre-spec-testing.md` section 5.C).

use ambre_core::{Instant, Issuer, Ms, Sec, Seq, TimeSpan, UnixMs};

use crate::generation::RenewalTicket;
use crate::http::{AccessToken, Credential, TokenPair};
use crate::{SyncEvent, SyncState};

/// How long before the announced expiry the worker renews the access token
/// (`ambre-spec-budgets.md` section 8).
pub(crate) const RENEWAL_MARGIN: TimeSpan<Sec> = TimeSpan::<Sec>::new(60);

/// What the shell does after the renewal state moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Due {
    /// Nothing: no renewal is armed, or the timer is stale.
    Idle,
    /// Renew the pair now.
    Renew,
    /// Arm a timer for `delay`, then report it fired with `ticket`.
    Wait {
        /// How long the timer waits.
        delay: TimeSpan<Ms>,
        /// The ticket the timer reports, which only the latest pair holds.
        ticket: Seq<RenewalTicket>,
    },
}

/// The one renewal armed: when it is due, and the ticket of its timer.
#[derive(Debug, Clone, Copy)]
struct Armed {
    due: Instant<UnixMs>,
    ticket: Seq<RenewalTicket>,
}

/// The renewal state of one session: at most one renewal armed, for the
/// pair stored last.
#[derive(Debug, Default)]
pub(crate) struct Renewal {
    armed: Option<Armed>,
    tickets: Issuer<RenewalTicket>,
}

impl Renewal {
    /// A pair whose access token lives `lifetime` was stored at `now`. The
    /// renewal falls due the margin before that lifetime ends. A lifetime no
    /// longer than the margin arms nothing. A short lifetime then never draws
    /// a renewal loop, and the refusal renews that pair instead.
    pub(crate) fn stored(&mut self, now: Instant<UnixMs>, lifetime: TimeSpan<Sec>) -> Due {
        let ticket = self.tickets.issue();
        let ahead = lifetime
            .checked_sub(RENEWAL_MARGIN)
            .filter(|ahead| *ahead > TimeSpan::ZERO);
        let Some(ahead) = ahead else {
            self.armed = None;
            return Due::Idle;
        };
        let delay = ahead.to_floor::<Ms>();
        self.armed = Some(Armed {
            due: now.plus(delay),
            ticket,
        });
        Due::Wait { delay, ticket }
    }

    /// The timer of `ticket` fired at `now`. A timer armed for an earlier
    /// pair is stale. A timer that fired before the due instant arms again
    /// for what remains. Otherwise the renewal runs, once: the machine
    /// disarms until the next pair is stored.
    pub(crate) fn fired(&mut self, now: Instant<UnixMs>, ticket: Seq<RenewalTicket>) -> Due {
        match self.armed {
            Some(armed) if armed.ticket == ticket && now >= armed.due => {
                self.armed = None;
                Due::Renew
            }
            Some(armed) if armed.ticket == ticket => Due::Wait {
                delay: armed.due.since(now),
                ticket,
            },
            _ => Due::Idle,
        }
    }

    /// Disarms the renewal: the session ended, so a timer still pending
    /// renews nothing.
    pub(crate) fn cancel(&mut self) {
        self.armed = None;
    }
}

impl SyncState {
    /// Stores `tokens` as the session's pair, and arms its renewal ahead of
    /// the expiry the relay announced. A pair the store already holds keeps
    /// the renewal it armed on arrival. An unlock hands back the pair it
    /// found, and the lifetime of that pair did not start again.
    pub(crate) fn hold_tokens(&mut self, tokens: TokenPair) {
        let lifetime = tokens.lifetime();
        let held = self
            .tokens
            .borrow()
            .as_ref()
            .is_some_and(|held| held.access_token().matches(tokens.access_token()));
        *self.tokens.borrow_mut() = Some(tokens);
        if held {
            return;
        }
        let due = self.renewal.stored((self.clock)(), lifetime);
        self.follow_renewal(due);
    }

    /// A renewal replaced the pair in the store, whichever path ran it. Arms
    /// the renewal of the new pair, or disarms when the session ended.
    pub(crate) fn tokens_renewed(&mut self) {
        let lifetime = self.tokens.borrow().as_ref().map(TokenPair::lifetime);
        let due = match lifetime {
            Some(lifetime) => self.renewal.stored((self.clock)(), lifetime),
            None => {
                self.renewal.cancel();
                Due::Idle
            }
        };
        self.follow_renewal(due);
    }

    /// The renewal timer of `ticket` fired.
    pub(crate) fn renewal_due(&mut self, ticket: Seq<RenewalTicket>) {
        let due = self.renewal.fired((self.clock)(), ticket);
        self.follow_renewal(due);
    }

    /// Carries out what the renewal state decided.
    fn follow_renewal(&mut self, due: Due) {
        match due {
            Due::Idle => {}
            Due::Renew => {
                let stale = self
                    .tokens
                    .borrow()
                    .as_ref()
                    .map(|t| t.access_token().clone());
                if let Some(stale) = stale {
                    self.renew_ahead(stale);
                }
            }
            Due::Wait { delay, ticket } => {
                self.arm_event_timer(SyncEvent::RenewalDue { ticket }, delay);
            }
        }
    }

    /// Renews the pair that holds `stale`, off the select loop. A success
    /// reports back through [`SyncEvent::TokensRenewed`], which arms the
    /// next renewal. A failure leaves the pair as it is: the next refusal
    /// renews it (`SYNC-185`).
    #[cfg(target_arch = "wasm32")]
    fn renew_ahead(&mut self, stale: Credential<AccessToken>) {
        let auth = self.auth_context();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = auth.renew(&stale).await {
                tracing::warn!("Sync: token renewal ahead of the expiry failed: {e}");
            }
        });
    }

    /// Native: no request. A host test reads the token each renewal was
    /// asked for.
    #[cfg(not(target_arch = "wasm32"))]
    fn renew_ahead(&mut self, stale: Credential<AccessToken>) {
        self.renewals_ahead.push(stale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant<UnixMs> {
        Instant::<UnixMs>::new(ms)
    }

    /// `SYNC-183`: the renewal falls due the margin before the lifetime the
    /// relay announced, and runs once. A timer that fires early arms again
    /// for what remains.
    #[test]
    fn the_renewal_falls_due_the_margin_before_the_announced_expiry() {
        let mut renewal = Renewal::default();
        let lifetime = TimeSpan::<Sec>::new(900);
        let Due::Wait { delay, ticket } = renewal.stored(at(1_000), lifetime) else {
            panic!("a fifteen-minute token arms a renewal");
        };
        let ahead = lifetime
            .checked_sub(RENEWAL_MARGIN)
            .expect("past the margin");
        assert_eq!(delay, ahead.to_floor::<Ms>());
        let due = at(1_000).plus(delay);
        assert_eq!(
            renewal.fired(at(due.get() - 1), ticket),
            Due::Wait {
                delay: TimeSpan::<Ms>::new(1),
                ticket
            }
        );
        assert_eq!(renewal.fired(due, ticket), Due::Renew);
        assert_eq!(
            renewal.fired(due, ticket),
            Due::Idle,
            "one renewal per pair"
        );
    }

    /// `SYNC-184`: only the timer of the pair stored last renews. The timer
    /// of a pair another renewal replaced stays idle.
    #[test]
    fn only_the_timer_of_the_latest_pair_renews() {
        let mut renewal = Renewal::default();
        let lifetime = TimeSpan::<Sec>::new(900);
        let Due::Wait { ticket: first, .. } = renewal.stored(at(0), lifetime) else {
            panic!("a renewal is armed");
        };
        let Due::Wait {
            delay,
            ticket: second,
        } = renewal.stored(at(5_000), lifetime)
        else {
            panic!("a renewal is armed");
        };
        let due = at(5_000).plus(delay);
        assert_eq!(renewal.fired(due, first), Due::Idle);
        assert_eq!(renewal.fired(due, second), Due::Renew);
    }

    /// `SYNC-183`: the worker reads the lifetime the relay announced with the
    /// pair. It renews that pair once, on its own clock, the margin before
    /// the expiry. A pair a renewal stored arms the next renewal, and an
    /// unlock that hands back the held pair arms nothing again.
    #[test]
    fn a_stored_pair_is_renewed_ahead_of_its_announced_expiry() {
        let (sink, _rx) = crate::LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let now = std::rc::Rc::new(std::cell::Cell::new(10_000_u64));
        let clock = now.clone();
        state.clock = std::rc::Rc::new(move || at(clock.get()));
        let announced = |access: &str| -> TokenPair {
            serde_json::from_value(serde_json::json!({
                "access_token": access,
                "refresh_token": "refresh",
                "access_expires_in": 900,
            }))
            .expect("the relay's pair")
        };
        let fire_at = |state: &mut SyncState, ms: u64| {
            now.set(ms);
            let ticket = state.renewal.tickets.latest();
            state.handle_sync_event(SyncEvent::RenewalDue { ticket });
            state
                .renewals_ahead
                .iter()
                .map(|t| t.secret().expose().to_owned())
                .collect::<Vec<_>>()
        };
        let ahead = TimeSpan::<Sec>::new(900)
            .checked_sub(RENEWAL_MARGIN)
            .expect("past the margin")
            .to_floor::<Ms>()
            .get();

        state.hold_tokens(announced("access-1"));
        assert!(fire_at(&mut state, 10_000 + ahead - 1).is_empty(), "early");
        assert_eq!(fire_at(&mut state, 10_000 + ahead), ["access-1"]);
        assert_eq!(fire_at(&mut state, 10_000 + ahead), ["access-1"], "once");

        // The renewal stored a new pair: its own lifetime counts from now.
        let renewed_at = 10_000 + ahead + 500;
        now.set(renewed_at);
        *state.tokens.borrow_mut() = Some(announced("access-2"));
        state.handle_sync_event(SyncEvent::TokensRenewed);
        state.hold_tokens(announced("access-2"));
        assert_eq!(fire_at(&mut state, renewed_at + ahead - 1).len(), 1);
        assert_eq!(
            fire_at(&mut state, renewed_at + ahead),
            ["access-1", "access-2"]
        );
    }

    /// `SYNC-185`: a lifetime no longer than the margin arms nothing, and a
    /// cancel makes the pending timer idle. The refusal renews either pair.
    #[test]
    fn a_short_lifetime_or_a_cancel_arms_nothing() {
        let mut renewal = Renewal::default();
        assert_eq!(renewal.stored(at(0), RENEWAL_MARGIN), Due::Idle);
        assert_eq!(renewal.stored(at(0), TimeSpan::ZERO), Due::Idle);
        let Due::Wait { delay, ticket } = renewal.stored(at(0), TimeSpan::<Sec>::new(900)) else {
            panic!("a renewal is armed");
        };
        renewal.cancel();
        assert_eq!(renewal.fired(at(0).plus(delay), ticket), Due::Idle);
    }
}
