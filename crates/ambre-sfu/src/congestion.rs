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
        let report = Report {
            at: now,
            packets: Count::of(acks.len()),
            lost: Count::of(acks.iter().filter(|a| a.remote_recv.is_none()).count()),
        };
        self.loss.update(report, self.delay.target(), rtt);
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
