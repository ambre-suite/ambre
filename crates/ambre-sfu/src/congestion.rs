//! The congestion control of one receiver, of Ambre's own.
//!
//! str0m hands the SFU the transport-wide feedback of each receiver. The SFU hands str0m the
//! target back, so the pacer and the padding follow it (`vendor/str0m/PATCH.md`). The target is
//! the lower of two bounds:
//!
//! * the delay bound, the GCC design of libwebrtc as str0m ports it (`delay.rs`): a growing queue
//!   delay lowers it, and nothing else does;
//! * the loss bound, which a loss above 10 % lowers, as the GCC draft states. Up to 10 % of loss
//!   holds it, so a random loss of a few percent leaves the target where the delay puts it
//!   (`CALL-044`).
//!
//! A decrease sets the target under the acknowledged rate, and the target then grows back by
//! about one packet per round trip. A drop on a delay step with no lasting congestion would
//! hold for many seconds. So the controller probes while the delay bound reads no overuse and a
//! higher layer exists. It paces above the next layer step for a short span. The target takes
//! that step when the link carries it with no overuse (`CALL-050`).

mod delay;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ambre_core::{Count, CountKind, Octets};
use str0m::bwe::{Bitrate, TwccAck};

use self::delay::{Arrival, DelayControl};

/// The lowest target: the lowest layer of one video and the audio.
const MIN_TARGET: Bitrate = Bitrate::kbps(100);

/// The highest target.
const MAX_TARGET: Bitrate = Bitrate::mbps(30);

/// The span of arrivals the acknowledged rate counts.
const ACKED_SPAN: Duration = Duration::from_millis(150);

/// The span of reports the loss counts.
const LOSS_SPAN: Duration = Duration::from_secs(1);

/// How many packets a feedback report or a loss reading holds.
pub enum Packets {}

impl CountKind for Packets {
    type Repr = u32;
    const NAME: &'static str = "PacketCount";
}

/// The least count of packets one loss reading needs.
const LOSS_PACKETS: Count<Packets> = Count::new(50);

/// The loss above which the loss bound falls, and the loss under which it rises again.
const LOSS_HIGH: f64 = 0.10;
const LOSS_LOW: f64 = 0.02;

/// The margin of the loss reading, in standard errors. A random loss of exactly 10 % reads above
/// 10 % in half of its windows. So the bound falls only when the loss is above 10 % beyond doubt.
const LOSS_CONFIDENCE: f64 = 2.0;

/// The least time between two falls of the loss bound, beyond one round trip.
const LOSS_INTERVAL: Duration = Duration::from_millis(300);

/// The growth of the loss bound per second, under 2 % of loss.
const LOSS_GROWTH: f64 = 1.08;

/// The weight of a new round-trip sample.
const RTT_WEIGHT: f64 = 0.125;

/// The round trip before the first sample.
const RTT_START: Duration = Duration::from_millis(100);

/// How long a probe paces at its rate. The span holds one window of the acknowledged rate, and
/// the gap between two feedback reports.
const PROBE_SEND: Duration = Duration::from_millis(300);

/// How long a probe waits for its verdict after its send span, beyond one round trip.
const PROBE_SLACK: Duration = Duration::from_millis(100);

/// The pace of a probe, as a multiple of the step it probes. The padding fills the gaps between
/// the media alone, so the pacer sends less than its pace.
const PROBE_PACE: f64 = 1.6;

/// The share of the step the acknowledged rate reaches when the link carries the probe. The
/// rest absorbs the random loss of the link.
const PROBE_DELIVERED: f64 = 0.95;

/// The wait before the next probe, after the first failed probe. Each further failure doubles
/// it, up to the longest wait, so a capped link sees a probe rarely.
const PROBE_BACKOFF: Duration = Duration::from_secs(1);
const PROBE_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// One probe: the controller paces at `rate` from `started`.
#[derive(Debug, Clone, Copy)]
struct Probe {
    rate: Bitrate,
    started: Instant,
    /// The target when the probe started. A decrease the probe causes stops there.
    floor: Bitrate,
}

/// What one feedback report says of the loss.
#[derive(Debug, Clone, Copy)]
struct Report {
    at: Instant,
    packets: Count<Packets>,
    lost: Count<Packets>,
}

/// The bound that loss above 10 % sets.
#[derive(Debug)]
struct LossBound {
    /// Each report of the last second.
    reports: VecDeque<Report>,
    /// None while no loss above 10 % has bound the target since the last recovery.
    bound: Option<Bitrate>,
    last_fall: Option<Instant>,
    last_update: Option<Instant>,
}

impl LossBound {
    fn update(&mut self, report: Report, delay: Bitrate, rtt: Duration) {
        let now = report.at;
        self.reports.push_back(report);
        while self
            .reports
            .front()
            .is_some_and(|r| now.saturating_duration_since(r.at) > LOSS_SPAN)
        {
            self.reports.pop_front();
        }
        let since = self
            .last_update
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.last_update = Some(now);
        let packets: Count<Packets> = self.reports.iter().map(|r| r.packets).sum();
        let lost: Count<Packets> = self.reports.iter().map(|r| r.lost).sum();
        if packets < LOSS_PACKETS {
            return;
        }
        let total = f64::from(packets.get());
        let loss = f64::from(lost.get()) / total;
        let error = (loss * (1.0 - loss) / total).sqrt();
        if loss - LOSS_CONFIDENCE * error > LOSS_HIGH {
            let due = self
                .last_fall
                .is_none_or(|at| now.saturating_duration_since(at) >= rtt + LOSS_INTERVAL);
            if due {
                let from = self.bound.unwrap_or(delay).min(delay);
                self.bound = Some(from * (1.0 - 0.5 * loss));
                self.last_fall = Some(now);
            }
        } else if loss < LOSS_LOW {
            self.bound = self.bound.and_then(|bound| {
                let grown = bound * LOSS_GROWTH.powf(since.as_secs_f64().min(1.0));
                (grown.as_f64() < delay.as_f64()).then_some(grown)
            });
        }
    }
}

/// The congestion control of the downlink of one receiver.
#[derive(Debug)]
pub struct Congestion {
    delay: DelayControl,
    loss: LossBound,
    /// The arrival and the size of each packet within the span of the acknowledged rate.
    acked: VecDeque<(Instant, Count<Octets>)>,
    rtt: Option<Duration>,
    target: Bitrate,
    probe: Option<Probe>,
    /// The target before a probe that raised overuse, and the end of the guard. Until the
    /// overuse ends, no decrease goes under it: the queue is the probe's, and it drains once the
    /// probe stops. The guard ends anyway at its end, so a cap that falls during a probe still
    /// lowers the target.
    guard: Option<(Bitrate, Instant)>,
    /// The earliest instant of the next probe, after a failed one.
    next_probe: Option<Instant>,
    backoff: Duration,
}

impl Congestion {
    /// A controller that starts at `initial`.
    pub fn new(initial: Bitrate) -> Self {
        Self {
            delay: DelayControl::new(initial, MIN_TARGET, MAX_TARGET),
            loss: LossBound {
                reports: VecDeque::new(),
                bound: None,
                last_fall: None,
                last_update: None,
            },
            acked: VecDeque::new(),
            rtt: None,
            target: initial.clamp(MIN_TARGET, MAX_TARGET),
            probe: None,
            guard: None,
            next_probe: None,
            backoff: PROBE_BACKOFF,
        }
    }

    /// Reads the rate of the next layer step of the receiver, or none at the top, and starts or
    /// stops a probe.
    ///
    /// A probe starts while the delay bound reads no overuse, no loss above 10 % bounds the
    /// target, and `step` is above the target. It stops when no step above the target is left.
    pub fn probe(&mut self, step: Option<Bitrate>, now: Instant) {
        let above = step.filter(|rate| rate.as_f64() > self.target.as_f64());
        if self.probe.is_some() {
            if above.is_none() {
                self.probe = None;
            }
            return;
        }
        let quiet = !self.delay.overusing() && self.loss.bound.is_none() && self.guard.is_none();
        let due = self.next_probe.is_none_or(|at| now >= at);
        if let (Some(rate), true, true) = (above, quiet, due) {
            self.probe = Some(Probe {
                rate,
                started: now,
                floor: self.target,
            });
        }
    }

    /// What the pacer pads up to during the send span of a probe, none outside it.
    pub fn padding(&self, now: Instant) -> Option<Bitrate> {
        self.probe
            .filter(|p| now < p.started + PROBE_SEND)
            .map(|p| p.rate * PROBE_PACE)
    }

    /// The rate the pacer sends at most: the pace of a probe during its send span, else the
    /// target.
    pub fn pacing(&self, now: Instant) -> Bitrate {
        self.padding(now)
            .map_or(self.target, |pace| pace.max(self.target))
    }

    /// Whether a probe runs, or waits for its verdict.
    pub fn probing(&self) -> bool {
        self.probe.is_some()
    }

    /// Ends the probe as failed, and doubles the wait before the next one.
    fn fail(&mut self, now: Instant) {
        self.probe = None;
        self.next_probe = Some(now + self.backoff);
        self.backoff = (self.backoff * 2).min(PROBE_BACKOFF_MAX);
    }

    /// Judges the running probe on one feedback report.
    fn judge(&mut self, acked: Bitrate, rtt: Duration, now: Instant) {
        let Some(probe) = self.probe else {
            return;
        };
        if self.delay.overusing() {
            // The probe stops at once, and its queue lowers nothing under the target it found.
            self.guard = Some((probe.floor, now + rtt + PROBE_SEND * 2));
            self.fail(now);
        } else if now >= probe.started + rtt + ACKED_SPAN
            && acked.as_f64() >= probe.rate.as_f64() * PROBE_DELIVERED
        {
            self.delay.raise(probe.rate, now);
            self.probe = None;
            self.next_probe = None;
            self.backoff = PROBE_BACKOFF;
        } else if now >= probe.started + rtt + PROBE_SEND + PROBE_SLACK {
            self.fail(now);
        }
    }

    /// Holds the target of a failed probe while the overuse it caused lasts.
    fn hold_guard(&mut self, now: Instant) {
        let Some((guard, until)) = self.guard else {
            return;
        };
        if self.delay.overusing() && now < until {
            self.delay.restore(guard);
            self.loss.bound = self.loss.bound.map(|bound| bound.max(guard));
        } else {
            self.guard = None;
        }
    }

    /// The rate the SFU sends the receiver at most.
    pub fn target(&self) -> Bitrate {
        self.target
    }

    /// Whether the queue delay of the downlink grows now: the delay bound reads overuse.
    pub fn congested(&self) -> bool {
        self.delay.overusing()
    }

    /// Reads one transport-wide feedback report, and answers the new target when it moved.
    pub fn on_feedback(&mut self, acks: &[TwccAck]) -> Option<Bitrate> {
        let now = acks.iter().map(|a| a.feedback_recv).max()?;
        let mut arrivals: Vec<Arrival> = acks
            .iter()
            .filter_map(|a| {
                a.remote_recv.map(|received| Arrival {
                    sent: a.sent,
                    received,
                })
            })
            .collect();
        arrivals.sort_by_key(|a| a.sent);
        for ack in acks {
            if let Some(received) = ack.remote_recv {
                self.acked.push_back((received, Count::of(ack.size)));
            }
            let sample = ack.feedback_recv.saturating_duration_since(ack.sent);
            self.rtt = Some(self.rtt.map_or(sample, |rtt| {
                rtt.mul_f64(1.0 - RTT_WEIGHT) + sample.mul_f64(RTT_WEIGHT)
            }));
        }
        let latest = self.acked.iter().map(|(at, _)| *at).max();
        if let Some(latest) = latest {
            self.acked
                .retain(|(at, _)| latest.saturating_duration_since(*at) <= ACKED_SPAN);
        }
        let bytes: Count<Octets> = self.acked.iter().map(|(_, size)| *size).sum();
        let acked = Bitrate::from(bytes.get() as f64 * 8.0 / ACKED_SPAN.as_secs_f64());
        let rtt = self.rtt.unwrap_or(RTT_START);
        self.delay.update(arrivals.into_iter(), acked, rtt, now);
        self.judge(acked, rtt, now);
        self.hold_guard(now);
        let report = Report {
            at: now,
            packets: Count::of(acks.len()),
            lost: Count::of(acks.iter().filter(|a| a.remote_recv.is_none()).count()),
        };
        self.loss.update(report, self.delay.target(), rtt);
        self.hold_guard(now);
        let target = self
            .loss
            .bound
            .map_or(self.delay.target(), |bound| bound.min(self.delay.target()))
            .clamp(MIN_TARGET, MAX_TARGET);
        let moved = target != self.target;
        self.target = target;
        moved.then_some(target)
    }
}

#[cfg(test)]
mod tests;
