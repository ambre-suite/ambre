//! The socket's lifecycle as a state machine over an injected clock. The
//! browser callbacks feed it events, and the timer shell only schedules the
//! deadline it returns. A host test steps it by advancing the clock
//! (`ambre-spec-testing.md` section 5.C).

use std::rc::Rc;

use ambre_core::{Bounded, Instant, Ms, TimeSpan, UnixMs};

use ambre_protocol::ws::{CloseCode, RelayClose};

/// The first reconnect delay.
const BASE_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(250);

/// The longest reconnect delay before jitter.
const MAX_DELAY: TimeSpan<Ms> = TimeSpan::<Ms>::new(10_000);

/// The worker's wall clock. A host test swaps in one it advances by hand.
pub(crate) type Clock = Rc<dyn Fn() -> Instant<UnixMs>>;

/// The clock of the running worker: the browser's wall clock.
#[cfg(target_arch = "wasm32")]
pub(crate) fn system_clock() -> Clock {
    // The wall clock is a positive count of milliseconds well below 2^53, so
    // the conversion is exact.
    Rc::new(|| Instant::<UnixMs>::new(js_sys::Date::now() as u64))
}

/// The clock of a native build: the system's wall clock.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn system_clock() -> Clock {
    Rc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(Instant::EPOCH, |d| {
                Instant::<UnixMs>::after_epoch(TimeSpan::from_duration(d))
            })
    })
}

/// Whether any document still needs the socket when it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Demand {
    /// No document is open: the socket stays closed.
    Unneeded,
    /// A document is open: the socket reconnects.
    Needed,
}

/// What the shell does after a socket event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    /// Nothing: no socket is wanted, or no deadline is armed.
    Idle,
    /// Open a socket now.
    Open,
    /// Renew the session's token, then report the outcome.
    Renew,
    /// Arm a timer for `delay`, then report it fired.
    Wait {
        /// How long the timer waits.
        delay: TimeSpan<Ms>,
    },
    /// The account owes its key setup: stay closed until a session with keys.
    SetupOwed,
}

/// The reconnect state of one socket: how many attempts failed in a row, and
/// when the next one is due.
#[derive(Debug, Default)]
pub(crate) struct Lifecycle {
    /// Attempts since the last frame, which set the next delay.
    attempts: u32,
    /// When the armed reconnect is due, on the worker's clock.
    deadline: Option<Instant<UnixMs>>,
}

impl Lifecycle {
    /// The delay before jitter after `attempts` failures in a row. It starts at
    /// 250 ms and doubles, capped at 10 s.
    pub(crate) fn delay(attempts: u32) -> TimeSpan<Ms> {
        TimeSpan::<Ms>::new(
            BASE_DELAY
                .get()
                .saturating_mul(1 << attempts.min(6))
                .min(MAX_DELAY.get()),
        )
    }

    /// The socket closed with `code` at `now`, or failed with no close,
    /// which carries no code (`SYNC-070`). `demand` is whether any document
    /// still needs it. `jitter` is a fraction in `[0, 1)` added to the delay.
    pub(crate) fn closed(
        &mut self,
        code: Option<Bounded<CloseCode>>,
        demand: Demand,
        now: Instant<UnixMs>,
        jitter: f64,
    ) -> Next {
        if code == Some(Bounded::<CloseCode>::SETUP_REQUIRED) {
            // A renewed session would have no keys either (`SYNC-133`).
            self.deadline = None;
            return Next::SetupOwed;
        }
        if demand == Demand::Unneeded {
            return Next::Idle;
        }
        if code == Some(Bounded::<CloseCode>::UNAUTHORIZED) {
            // A dead token is cured by a renewal, which no backoff can be
            // (`SYNC-063`).
            return Next::Renew;
        }
        self.back_off(now, jitter)
    }

    /// Arms the next reconnect. The delay grows with each attempt until a
    /// frame arrives (`SYNC-064`).
    pub(crate) fn back_off(&mut self, now: Instant<UnixMs>, jitter: f64) -> Next {
        let capped = Self::delay(self.attempts).get();
        self.attempts = self.attempts.saturating_add(1);
        // `capped` is at most 10 s, so the product fits and is not negative.
        let spread = (capped as f64 * jitter.clamp(0.0, 1.0)) as u64;
        let delay = TimeSpan::<Ms>::new(capped.saturating_add(spread));
        self.deadline = Some(now.plus(delay));
        Next::Wait { delay }
    }

    /// A frame arrived, so the connection is live and the backoff starts over.
    /// A bare open is not one: a relay that refuses the session still opens
    /// the socket first (`SYNC-098`).
    pub(crate) fn frame(&mut self) {
        self.attempts = 0;
    }

    /// A renewal landed: reopen at once, with the backoff reset.
    pub(crate) fn renewed(&mut self) -> Next {
        self.reset();
        Next::Open
    }

    /// The timer fired at `now`. Opens once the deadline has passed, and
    /// arms again for what remains when the timer fired early.
    pub(crate) fn poll(&mut self, now: Instant<UnixMs>) -> Next {
        match self.deadline {
            None => Next::Idle,
            Some(due) if now >= due => {
                self.deadline = None;
                Next::Open
            }
            Some(due) => Next::Wait {
                // The arm above took `now >= due`, so the difference is
                // positive; saturating states the floor at zero.
                delay: due.since(now),
            },
        }
    }

    /// Starts over, with no attempt counted and no deadline armed.
    pub(crate) fn reset(&mut self) {
        self.attempts = 0;
        self.deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SYNC-064`: each close with no frame between lengthens the wait, up to
    /// the cap. The reconnect opens only once the clock reaches it.
    #[test]
    fn the_wait_grows_while_no_frame_arrives() {
        let mut lifecycle = Lifecycle::default();
        let mut now = Instant::<UnixMs>::new(0);
        let mut waits = Vec::new();
        for _ in 0..8 {
            let Next::Wait { delay } =
                lifecycle.closed(Bounded::<CloseCode>::new(1006), Demand::Needed, now, 0.0)
            else {
                panic!("a close backs off");
            };
            waits.push(delay.get());
            assert_eq!(
                lifecycle.poll(Instant::<UnixMs>::new(now.get() + delay.get() - 1)),
                Next::Wait {
                    delay: TimeSpan::<Ms>::new(1)
                }
            );
            now = now.plus(delay);
            assert_eq!(lifecycle.poll(now), Next::Open);
            assert_eq!(lifecycle.poll(now), Next::Idle, "one open per deadline");
        }
        assert_eq!(waits, [250, 500, 1000, 2000, 4000, 8000, 10_000, 10_000]);
    }

    /// `SYNC-098`: a frame resets the wait, and an open with no frame does
    /// not. A relay refusing every session still sees the wait grow.
    #[test]
    fn the_wait_resets_on_a_frame_alone() {
        let mut lifecycle = Lifecycle::default();
        lifecycle.closed(
            Bounded::<CloseCode>::new(1006),
            Demand::Needed,
            Instant::<UnixMs>::new(0),
            0.0,
        );
        lifecycle.poll(Instant::<UnixMs>::new(250));
        // Opened and closed again with no frame between.
        assert_eq!(
            lifecycle.closed(
                Bounded::<CloseCode>::new(1006),
                Demand::Needed,
                Instant::<UnixMs>::new(250),
                0.0
            ),
            Next::Wait {
                delay: TimeSpan::<Ms>::new(500)
            }
        );
        lifecycle.poll(Instant::<UnixMs>::new(750));
        lifecycle.frame();
        assert_eq!(
            lifecycle.closed(
                Bounded::<CloseCode>::new(1006),
                Demand::Needed,
                Instant::<UnixMs>::new(800),
                0.0
            ),
            Next::Wait {
                delay: TimeSpan::<Ms>::new(250)
            }
        );
    }

    /// `SYNC-063`: a socket refused on authentication is renewed, not retried.
    /// The renewal reopens at once, with the wait reset.
    #[test]
    fn an_authentication_refusal_renews_and_reopens() {
        let mut lifecycle = Lifecycle::default();
        lifecycle.closed(
            Bounded::<CloseCode>::new(1006),
            Demand::Needed,
            Instant::<UnixMs>::new(0),
            0.0,
        );
        assert_eq!(
            lifecycle.closed(
                Some(Bounded::<CloseCode>::UNAUTHORIZED),
                Demand::Needed,
                Instant::<UnixMs>::new(10),
                0.0
            ),
            Next::Renew
        );
        assert_eq!(lifecycle.renewed(), Next::Open);
        assert_eq!(
            lifecycle.poll(Instant::<UnixMs>::new(10_000)),
            Next::Idle,
            "the old deadline is gone"
        );
        assert_eq!(
            lifecycle.closed(
                Bounded::<CloseCode>::new(1006),
                Demand::Needed,
                Instant::<UnixMs>::new(20),
                0.0
            ),
            Next::Wait {
                delay: TimeSpan::<Ms>::new(250)
            }
        );
    }

    /// `SYNC-070`: a socket error reads as a close with no code, and takes the
    /// same path. A setup refusal and a socket nobody wants arm nothing.
    #[test]
    fn an_error_backs_off_as_a_close_does() {
        let mut errored = Lifecycle::default();
        let mut closed = Lifecycle::default();
        assert_eq!(
            errored.closed(None, Demand::Needed, Instant::<UnixMs>::new(0), 0.5),
            closed.closed(
                Bounded::<CloseCode>::new(1006),
                Demand::Needed,
                Instant::<UnixMs>::new(0),
                0.5
            )
        );
        assert_eq!(
            errored.closed(None, Demand::Needed, Instant::<UnixMs>::new(0), 0.0),
            Next::Wait {
                delay: TimeSpan::<Ms>::new(500)
            }
        );

        let mut lifecycle = Lifecycle::default();
        lifecycle.closed(
            Bounded::<CloseCode>::new(1006),
            Demand::Needed,
            Instant::<UnixMs>::new(0),
            0.0,
        );
        assert_eq!(
            lifecycle.closed(
                Some(Bounded::<CloseCode>::SETUP_REQUIRED),
                Demand::Needed,
                Instant::<UnixMs>::new(0),
                0.0
            ),
            Next::SetupOwed
        );
        assert_eq!(
            lifecycle.poll(Instant::<UnixMs>::new(u64::MAX)),
            Next::Idle,
            "no timer reopens it"
        );
        assert_eq!(
            lifecycle.closed(
                Bounded::<CloseCode>::new(1006),
                Demand::Unneeded,
                Instant::<UnixMs>::new(0),
                0.0
            ),
            Next::Idle
        );
    }
}
