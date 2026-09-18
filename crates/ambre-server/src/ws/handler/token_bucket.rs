// ambre-server/src/ws/handler/token_bucket.rs Per-connection inbound-message rate
// limiter for the WebSocket loop.
//
// The REST/WS *upgrade* is rate-limited (see `middleware::rate_limit`), but once a
// socket is open there is no bound on how many messages a client may send. Each
// `Update` drives a Postgres INSERT + Redis RPUSH + S3 pending write + PubSub
// fan-out. An authorized `editor` flooding updates is therefore a real
// storage/cost amplification, and `Ephemeral` broadcasts have no cap either. A
// small in-memory token bucket per connection bounds the sustained rate and the
// burst without a per-message Redis round-trip -- it lives entirely on
// `WsConnectionState`.

#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

/// A monotonic-clock token bucket.
///
/// `capacity` tokens accumulate at `refill_per_sec`; each accepted message costs
/// one token. When empty, `try_consume` returns `false` until enough time has
/// elapsed to refill at least one token. Fractional tokens are kept so a
/// sustained rate below the refill rate is never rejected by rounding.
pub(crate) struct TokenBucket {
    /// Maximum number of tokens the bucket can hold (the burst allowance).
    capacity: f64,
    /// Tokens replenished per second (the sustained rate).
    refill_per_sec: f64,
    /// Current token count, in `[0, capacity]`.
    tokens: f64,
    /// Last instant the bucket was refilled.
    last_refill: Instant,
}

impl TokenBucket {
    /// Builds a bucket that starts full (`capacity` tokens), refilling at
    /// `refill_per_sec`. A full start lets a freshly connected client burst up to
    /// `capacity` messages (for example an initial batch of subscribes) immediately.
    pub(crate) fn new(capacity: u32, refill_per_sec: u32) -> Self {
        let capacity = capacity.max(1) as f64;
        Self {
            capacity,
            refill_per_sec: refill_per_sec.max(1) as f64,
            tokens: capacity,
            last_refill: Instant::now(),
        }
    }

    /// Refills the bucket for the time elapsed since the last call, then tries to
    /// spend one token. Returns `true` when the message is allowed.
    ///
    /// `now` is injected so the behaviour is deterministic under test; callers on
    /// the hot path pass `Instant::now()`.
    pub(crate) fn try_consume(&mut self, now: Instant) -> bool {
        // Refill first: add the tokens accrued over the elapsed window, capped at
        // capacity. `saturating_duration_since` guards against a non-monotonic
        // `now` (should not happen with `Instant`, but keeps the math safe).
        let elapsed = now.saturating_duration_since(self.last_refill);
        self.last_refill = now;
        self.tokens =
            (self.tokens + elapsed.as_secs_f64() * self.refill_per_sec).min(self.capacity);

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Convenience: `Duration` a caller can advance a test clock by to gain one token.
#[cfg(test)]
pub(crate) fn one_token_interval(refill_per_sec: u32) -> Duration {
    Duration::from_secs_f64(1.0 / refill_per_sec.max(1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_full_and_allows_a_full_burst() {
        let start = Instant::now();
        let mut b = TokenBucket::new(5, 1);
        // Five immediate messages are allowed (the burst), the sixth is not.
        for i in 0..5 {
            assert!(b.try_consume(start), "message {i} within burst must pass");
        }
        assert!(
            !b.try_consume(start),
            "the message past the burst must be rejected"
        );
    }

    #[test]
    fn refills_at_the_configured_rate() {
        let start = Instant::now();
        let mut b = TokenBucket::new(2, 10); // 10 tokens/sec
        assert!(b.try_consume(start));
        assert!(b.try_consume(start));
        assert!(!b.try_consume(start), "burst of 2 is exhausted");

        // After one refill interval (100 ms) exactly one token is back.
        let t1 = start + one_token_interval(10);
        assert!(b.try_consume(t1), "one token must have refilled");
        assert!(!b.try_consume(t1), "only one token refilled, not two");
    }

    #[test]
    fn refill_is_capped_at_capacity() {
        let start = Instant::now();
        let mut b = TokenBucket::new(3, 100);
        // Drain, then wait a long time: the bucket cannot exceed its capacity.
        assert!(b.try_consume(start));
        let much_later = start + Duration::from_secs(3600);
        for i in 0..3 {
            assert!(b.try_consume(much_later), "capacity token {i} available");
        }
        assert!(
            !b.try_consume(much_later),
            "refill must not accumulate beyond capacity"
        );
    }

    #[test]
    fn sustained_rate_below_refill_is_never_rejected() {
        // A client sending exactly at the refill rate must never be throttled,
        // even far past the initial burst (fractional tokens are preserved).
        let mut now = Instant::now();
        let mut b = TokenBucket::new(4, 20); // 20/sec => 50 ms per token
        let step = one_token_interval(20);
        for i in 0..1000 {
            assert!(
                b.try_consume(now),
                "message {i} at the sustained rate must pass"
            );
            now += step;
        }
    }
}
