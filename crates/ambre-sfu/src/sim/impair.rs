//! The impairment of one direction of a link: a seeded loss, a fixed delay, and an optional
//! cap on the rate.
//!
//! The same seed drops the same datagrams, in the order they arrive, so a run under loss can
//! be replayed. A cap queues each datagram behind the ones before it, as a router buffer does.
//! It drops a datagram that the queue would hold longer than the queue limit.

use std::time::{Duration, Instant};

/// A generator of SplitMix64: small, seeded, and the same on every machine.
#[derive(Clone, Debug)]
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A draw in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The settings of one direction.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    /// The one-way delay.
    pub delay: Duration,
    /// The share of datagrams lost, from 0 to 1.
    pub loss: f64,
    /// The cap on the rate, in bits per second.
    pub cap: Option<u64>,
    /// The longest a datagram may wait behind the cap.
    pub queue: Duration,
}

impl Profile {
    /// No delay, no loss, no cap.
    pub const CLEAN: Self = Self {
        delay: Duration::ZERO,
        loss: 0.0,
        cap: None,
        queue: Duration::from_millis(300),
    };

    /// Half of `rtt` of delay, and `loss`.
    pub fn half(rtt: Duration, loss: f64) -> Self {
        Self {
            delay: rtt / 2,
            loss,
            ..Self::CLEAN
        }
    }
}

/// One direction of a link.
#[derive(Debug)]
pub struct Impairment {
    pub profile: Profile,
    rng: Seeded,
    /// When the cap has sent the last queued datagram.
    busy_until: Option<Instant>,
    /// Counts, for a report.
    pub sent: u64,
    pub lost: u64,
    pub overflowed: u64,
}

impl Impairment {
    pub fn new(profile: Profile, seed: u64) -> Self {
        Self {
            profile,
            rng: Seeded::new(seed),
            busy_until: None,
            sent: 0,
            lost: 0,
            overflowed: 0,
        }
    }

    /// When a datagram of `len` bytes that enters at `now` leaves the link, or none when the
    /// link drops it.
    pub fn admit(&mut self, now: Instant, len: usize) -> Option<Instant> {
        // The draw happens for every datagram, so a cap does not shift the loss pattern.
        let lost = self.rng.unit() < self.profile.loss;
        if lost {
            self.lost += 1;
            return None;
        }
        let departs = match self.profile.cap {
            None => now,
            Some(bps) => {
                let start = self.busy_until.map_or(now, |b| b.max(now));
                let wire = Duration::from_secs_f64((len as f64 * 8.0) / bps as f64);
                let departs = start + wire;
                if departs.saturating_duration_since(now) > self.profile.queue {
                    self.overflowed += 1;
                    return None;
                }
                self.busy_until = Some(departs);
                departs
            }
        };
        self.sent += 1;
        Some(departs + self.profile.delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_seed_drops_the_same_datagrams_and_another_seed_does_not() {
        let now = Instant::now();
        let profile = Profile::half(Duration::from_millis(150), 0.02);
        let pattern = |seed| {
            let mut link = Impairment::new(profile, seed);
            (0..10_000)
                .map(|_| link.admit(now, 1200).is_some())
                .collect::<Vec<_>>()
        };
        assert_eq!(pattern(7), pattern(7));
        assert_ne!(pattern(7), pattern(8));
        let lost = pattern(7).iter().filter(|kept| !**kept).count();
        assert!((150..250).contains(&lost), "lost {lost} of 10 000 at 2 %");
    }

    #[test]
    fn the_delay_is_half_the_round_trip() {
        let now = Instant::now();
        let mut link = Impairment::new(Profile::half(Duration::from_millis(150), 0.0), 1);
        assert_eq!(link.admit(now, 100), Some(now + Duration::from_millis(75)));
    }

    #[test]
    fn a_cap_queues_then_drops_past_the_queue_limit() {
        let now = Instant::now();
        let profile = Profile {
            cap: Some(80_000),
            ..Profile::CLEAN
        };
        let mut link = Impairment::new(profile, 1);
        // 1000 bytes at 80 kbit/s take 100 ms each: the fourth waits 400 ms, past 300 ms.
        let leaves: Vec<_> = (0..4).map(|_| link.admit(now, 1000)).collect();
        assert_eq!(
            leaves.first(),
            Some(&Some(now + Duration::from_millis(100)))
        );
        assert_eq!(leaves.get(2), Some(&Some(now + Duration::from_millis(300))));
        assert_eq!(leaves.get(3), Some(&None));
    }
}
