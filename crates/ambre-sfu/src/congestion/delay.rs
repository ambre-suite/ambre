// Ported from the delay-based estimator of str0m 0.25.0, `src/bwe/delay/`. str0m ported it from
// the goog_cc module of libwebrtc.
//
// Copyright 2022 Martin Algesten
//
// Licensed under the MIT license. The full notice is `LICENSE-str0m-MIT.txt` at the root of this
// crate, and it applies to this file.

//! The delay half of the congestion control: the packet groups, the trendline of the queue delay,
//! and the rate control that follows it.
//!
//! A growing queue delay is the one sign of congestion this half reads. A lost packet carries no
//! arrival time, so it adds no delay sample.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ambre_core::{Count, CountKind};
use str0m::bwe::Bitrate;

/// Packets sent within this span of the first packet of a group belong to that group.
const GROUP_SPAN: Duration = Duration::from_millis(5);
/// A packet that arrives within this span of its group, and earlier than its send gap, joins a
/// burst of that group.
const BURST_GAP: Duration = Duration::from_millis(5);
/// The longest burst one group may hold.
const BURST_SPAN: Duration = Duration::from_millis(100);

/// How many delay samples the trendline has read, or has seen above its threshold.
enum Samples {}

impl CountKind for Samples {
    type Repr = u32;
    const NAME: &'static str = "DelaySampleCount";
}

/// The count of delay samples the trendline fits a line through.
const WINDOW: usize = 20;
/// The smoothing of the accumulated delay.
const SMOOTHING: f64 = 0.9;
/// The gain of the trend against the threshold.
const TREND_GAIN: f64 = 4.0;
/// The count of samples above which the trend gains no more weight.
const TREND_SAMPLES: Count<Samples> = Count::new(60);
/// The count of samples the trendline stops counting at.
const SAMPLES_CAP: Count<Samples> = Count::new(1000);
/// The first threshold of the trend, in milliseconds.
const THRESHOLD_START: f64 = 12.5;
/// The bounds of the threshold, in milliseconds.
const THRESHOLD_MIN: f64 = 6.0;
const THRESHOLD_MAX: f64 = 600.0;
/// A trend this far above the threshold is a spike, and the threshold does not adapt to it.
const SPIKE_OFFSET: f64 = 15.0;
/// The adaptation rates of the threshold, per millisecond, up and down.
const K_UP: f64 = 0.0087;
const K_DOWN: f64 = 0.039;
/// The longest step one adaptation of the threshold takes.
const THRESHOLD_STEP: Duration = Duration::from_millis(100);
/// How long the trend stays above the threshold before the detector calls overuse.
const OVERUSE_TIME: Duration = Duration::from_millis(10);

/// The share of the acknowledged rate a decrease keeps.
const BETA: f64 = 0.85;
/// The growth of the target per second, far from the known capacity of the link.
const GROWTH: f64 = 1.08;
/// The growth of the target per second before the first decrease. libwebrtc probes at three and
/// six times the start rate; the SFU sends no probe, and pads up to the next layer step instead.
const START_GROWTH: f64 = 2.0;
/// The highest target, as a multiple of the acknowledged rate.
const ACKED_RATIO: f64 = 1.5;
/// The headroom above that multiple, so a quiet link can start to grow.
const ACKED_HEADROOM: Bitrate = Bitrate::kbps(10);
/// Added to the round trip, the time the link takes to answer a change of rate.
const RESPONSE_EXTRA: Duration = Duration::from_millis(100);
/// The least growth of one additive step.
const ADDITIVE_FLOOR: f64 = 1000.0;
/// The packet size the additive step assumes.
const PACKET_BITS: f64 = 1200.0 * 8.0;
/// The picture rate the additive step assumes.
const FRAME_RATE: f64 = 30.0;
/// The weight of a new sample of the acknowledged rate at a decrease.
const CAPACITY_WEIGHT: f64 = 0.05;
/// The width of the band around the known capacity, in standard deviations.
const CAPACITY_BAND: f64 = 3.0;

/// One packet the receiver acknowledged.
#[derive(Debug, Clone, Copy)]
pub struct Arrival {
    pub sent: Instant,
    pub received: Instant,
}

/// The state of the link the delay trend reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Usage {
    Normal,
    Overuse,
    Underuse,
}

/// The packets of one group: the first and the last send time, and the last arrival.
#[derive(Debug, Clone, Copy)]
struct Group {
    first_sent: Instant,
    first_received: Instant,
    last_sent: Instant,
    last_received: Instant,
}

impl Group {
    fn start(packet: Arrival) -> Self {
        Self {
            first_sent: packet.sent,
            first_received: packet.received,
            last_sent: packet.sent,
            last_received: packet.received,
        }
    }

    /// Whether `packet` belongs to this group, or starts the next one.
    fn holds(&self, packet: Arrival) -> bool {
        if packet.sent <= self.last_sent {
            return true;
        }
        let send_gap = packet.sent - self.last_sent;
        // A burst arrives faster than it left: the queue drained, and the burst is one group.
        let burst = packet
            .received
            .checked_duration_since(self.last_received)
            .is_some_and(|gap| gap <= BURST_GAP && gap < send_gap)
            && packet
                .received
                .saturating_duration_since(self.first_received)
                < BURST_SPAN;
        burst || packet.sent.saturating_duration_since(self.first_sent) <= GROUP_SPAN
    }

    fn add(&mut self, packet: Arrival) {
        self.last_sent = self.last_sent.max(packet.sent);
        self.last_received = self.last_received.max(packet.received);
    }
}

/// The change of the queue delay between two groups, and the arrival of the later one.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// The arrival gap minus the send gap, in milliseconds.
    delta_ms: f64,
    send_gap: Duration,
    received: Instant,
}

/// The groups of the packets, and the delay change between each two.
#[derive(Debug, Default)]
struct Groups {
    previous: Option<Group>,
    current: Option<Group>,
    /// The first send time, so the millisecond grid of the send times stays fixed.
    origin: Option<Instant>,
}

impl Groups {
    /// Adds `packet`, and answers a sample when it closes a group.
    fn add(&mut self, packet: Arrival) -> Option<Sample> {
        // libwebrtc reads send times to the millisecond, so packets sent together share a group.
        let origin = *self.origin.get_or_insert(packet.sent);
        let elapsed = packet.sent.checked_duration_since(origin)?;
        let rounded = Duration::from_millis(
            u64::try_from(elapsed.as_micros().saturating_add(500) / 1000).unwrap_or(u64::MAX),
        );
        let packet = Arrival {
            sent: origin + rounded,
            ..packet
        };
        let Some(current) = self.current.as_mut() else {
            self.current = Some(Group::start(packet));
            return None;
        };
        if packet.sent < current.first_sent {
            // Reordered across groups: the packet adds nothing.
            return None;
        }
        if current.holds(packet) {
            current.add(packet);
            return None;
        }
        let closed = *current;
        self.current = Some(Group::start(packet));
        let sample = self.previous.and_then(|previous| {
            let send_gap = closed
                .last_sent
                .checked_duration_since(previous.last_sent)?;
            let arrival_ms = signed_ms(closed.last_received, previous.last_received);
            Some(Sample {
                delta_ms: arrival_ms - send_gap.as_secs_f64() * 1000.0,
                send_gap,
                received: closed.last_received,
            })
        });
        self.previous = Some(closed);
        sample
    }
}

/// `later - earlier` in milliseconds, negative when `later` comes first.
fn signed_ms(later: Instant, earlier: Instant) -> f64 {
    match later.checked_duration_since(earlier) {
        Some(d) => d.as_secs_f64() * 1000.0,
        None => -(earlier.saturating_duration_since(later).as_secs_f64() * 1000.0),
    }
}

/// One point of the trendline: the arrival, and the smoothed accumulated delay.
#[derive(Debug, Clone, Copy)]
struct Point {
    at_ms: f64,
    delay_ms: f64,
}

/// The trendline of the queue delay, and the detector that compares it to an adaptive threshold.
#[derive(Debug)]
struct Trendline {
    history: VecDeque<Point>,
    first: Option<Instant>,
    samples: Count<Samples>,
    accumulated_ms: f64,
    smoothed_ms: f64,
    threshold_ms: f64,
    previous_trend: f64,
    /// How long the trend has stayed above the threshold, and how many samples saw it.
    over: Option<(Duration, Count<Samples>)>,
    last_adapt: Option<Instant>,
    usage: Usage,
}

impl Trendline {
    fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(WINDOW + 1),
            first: None,
            samples: Count::ZERO,
            accumulated_ms: 0.0,
            smoothed_ms: 0.0,
            threshold_ms: THRESHOLD_START,
            previous_trend: 0.0,
            over: None,
            last_adapt: None,
            usage: Usage::Normal,
        }
    }

    fn add(&mut self, sample: Sample) {
        let first = *self.first.get_or_insert(sample.received);
        self.samples = self.samples.incremented().min(SAMPLES_CAP);
        self.accumulated_ms += sample.delta_ms;
        self.smoothed_ms = SMOOTHING * self.smoothed_ms + (1.0 - SMOOTHING) * self.accumulated_ms;
        let point = Point {
            at_ms: signed_ms(sample.received, first),
            delay_ms: self.smoothed_ms,
        };
        // Arrivals may come out of order across reports: keep the history sorted.
        let at = self
            .history
            .iter()
            .rposition(|p| p.at_ms <= point.at_ms)
            .map_or(0, |i| i + 1);
        self.history.insert(at, point);
        while self.history.len() > WINDOW {
            self.history.pop_front();
        }
        if self.history.len() == WINDOW {
            let trend = self.slope().unwrap_or(self.previous_trend);
            self.detect(trend, sample);
        }
    }

    /// The slope of the least-squares line through the history.
    fn slope(&self) -> Option<f64> {
        let count = self.history.len() as f64;
        let (sum_x, sum_y) = self
            .history
            .iter()
            .fold((0.0, 0.0), |(x, y), p| (x + p.at_ms, y + p.delay_ms));
        let (mean_x, mean_y) = (sum_x / count, sum_y / count);
        let (num, den) = self.history.iter().fold((0.0, 0.0), |(n, d), p| {
            let dx = p.at_ms - mean_x;
            (n + dx * (p.delay_ms - mean_y), d + dx * dx)
        });
        (den != 0.0).then(|| num / den)
    }

    fn detect(&mut self, trend: f64, sample: Sample) {
        if self.samples.get() < 2 {
            self.usage = Usage::Normal;
            return;
        }
        let modified = f64::from(self.samples.min(TREND_SAMPLES).get()) * trend * TREND_GAIN;
        if modified > self.threshold_ms {
            // Assume the overuse began half a send gap before the first sample above it.
            let (time, count) = self.over.get_or_insert((sample.send_gap / 2, Count::ZERO));
            if !count.is_zero() {
                *time += sample.send_gap;
            }
            *count = count.incremented();
            if *time > OVERUSE_TIME && count.get() > 1 && trend > self.previous_trend {
                self.over = None;
                self.usage = Usage::Overuse;
            }
        } else if modified < -self.threshold_ms {
            self.over = None;
            self.usage = Usage::Underuse;
        } else {
            self.over = None;
            self.usage = Usage::Normal;
        }
        self.previous_trend = trend;
        self.adapt(modified, sample.received);
    }

    /// Moves the threshold toward the trend, so the detector follows a link it shares with
    /// flows that read no delay.
    fn adapt(&mut self, modified: f64, now: Instant) {
        let last = *self.last_adapt.get_or_insert(now);
        self.last_adapt = Some(now);
        let size = modified.abs();
        if size > self.threshold_ms + SPIKE_OFFSET {
            return;
        }
        let k = if size < self.threshold_ms {
            K_DOWN
        } else {
            K_UP
        };
        let step_ms = now
            .saturating_duration_since(last)
            .min(THRESHOLD_STEP)
            .as_secs_f64()
            * 1000.0;
        self.threshold_ms += k * (size - self.threshold_ms) * step_ms;
        self.threshold_ms = self.threshold_ms.clamp(THRESHOLD_MIN, THRESHOLD_MAX);
    }
}

/// What the rate control does with the next signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Hold,
    Increase,
    Decrease,
}

/// Whether the target ever decreased: before the first decrease, it grows at the start rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Steady,
}

/// The acknowledged rate at the decreases: the capacity of the link the target nears.
#[derive(Debug, Clone, Copy)]
struct Capacity {
    mean_bps: f64,
    variance: f64,
}

impl Capacity {
    fn update(previous: Option<Self>, sample: Bitrate) -> Self {
        let sample_bps = sample.as_f64();
        let Some(previous) = previous else {
            return Self {
                mean_bps: sample_bps,
                variance: 0.0,
            };
        };
        let delta = sample_bps - previous.mean_bps;
        Self {
            mean_bps: previous.mean_bps + CAPACITY_WEIGHT * delta,
            variance: (1.0 - CAPACITY_WEIGHT)
                * (previous.variance + CAPACITY_WEIGHT * delta * delta),
        }
    }

    fn holds(&self, rate: Bitrate) -> bool {
        let bps = rate.as_f64();
        (bps - self.mean_bps).abs() <= CAPACITY_BAND * self.variance.sqrt()
    }

    fn below(&self, rate: Bitrate) -> bool {
        let bps = rate.as_f64();
        bps < self.mean_bps - CAPACITY_BAND * self.variance.sqrt()
    }

    fn above(&self, rate: Bitrate) -> bool {
        let bps = rate.as_f64();
        bps > self.mean_bps + CAPACITY_BAND * self.variance.sqrt()
    }
}

/// The additive-increase, multiplicative-decrease control of the target, driven by the trend.
#[derive(Debug)]
struct RateControl {
    state: State,
    target: Bitrate,
    floor: Bitrate,
    ceiling: Bitrate,
    capacity: Option<Capacity>,
    last_change: Option<Instant>,
    phase: Phase,
}

impl RateControl {
    fn update(&mut self, usage: Usage, acked: Bitrate, rtt: Duration, now: Instant) {
        self.state = match (self.state, usage) {
            (_, Usage::Overuse) => State::Decrease,
            (_, Usage::Underuse) => State::Hold,
            (State::Decrease, Usage::Normal) => State::Hold,
            (State::Hold | State::Increase, Usage::Normal) => State::Increase,
        };
        match self.state {
            State::Hold => {}
            State::Increase => self.increase(acked, rtt, now),
            State::Decrease => {
                let since = self.last_change.map(|at| now.saturating_duration_since(at));
                let interval = rtt.clamp(Duration::from_millis(10), Duration::from_millis(200));
                let starved = acked.as_f64() < self.target.as_f64() * 0.5;
                if since.is_none_or(|s| s >= interval) || starved {
                    self.decrease(acked, now);
                }
            }
        }
    }

    fn increase(&mut self, acked: Bitrate, rtt: Duration, now: Instant) {
        let limit = acked * ACKED_RATIO + ACKED_HEADROOM;
        let last = *self.last_change.get_or_insert(now);
        self.last_change = Some(now);
        if self.target.as_f64() >= limit.as_f64() {
            return;
        }
        if self.capacity.is_some_and(|c| c.above(acked)) {
            // The link carries more than it did at the last decrease: its capacity is unknown.
            self.capacity = None;
        }
        let since = now
            .saturating_duration_since(last)
            .min(Duration::from_secs(1));
        let target = self.target.as_f64();
        let next = if self.capacity.is_some_and(|c| c.holds(acked)) {
            // Near the capacity: about half a packet per response time.
            let response = rtt + RESPONSE_EXTRA;
            let alpha = 0.5 * (since.as_secs_f64() / response.as_secs_f64()).min(1.0);
            let per_frame = target / FRAME_RATE;
            let packets = (per_frame / PACKET_BITS).ceil().max(1.0);
            target + (alpha * per_frame / packets).max(ADDITIVE_FLOOR)
        } else {
            let growth = match self.phase {
                Phase::Start => START_GROWTH,
                Phase::Steady => GROWTH,
            };
            target + ((growth.powf(since.as_secs_f64()) - 1.0) * target).max(ADDITIVE_FLOOR)
        };
        self.set(Bitrate::from(next.min(limit.as_f64())), now);
    }

    fn decrease(&mut self, acked: Bitrate, now: Instant) {
        if self.capacity.is_some_and(|c| c.below(acked)) {
            self.capacity = None;
        }
        self.capacity = Some(Capacity::update(self.capacity, acked));
        self.phase = Phase::Steady;
        self.state = State::Hold;
        self.set((acked * BETA).min(self.target), now);
    }

    fn set(&mut self, target: Bitrate, now: Instant) {
        self.target = target.clamp(self.floor, self.ceiling);
        self.last_change = Some(now);
    }
}

/// The delay-based estimate of one downlink.
#[derive(Debug)]
pub struct DelayControl {
    groups: Groups,
    trendline: Trendline,
    rate: RateControl,
}

impl DelayControl {
    /// A control that starts at `initial`, and stays between `floor` and `ceiling`.
    pub fn new(initial: Bitrate, floor: Bitrate, ceiling: Bitrate) -> Self {
        Self {
            groups: Groups::default(),
            trendline: Trendline::new(),
            rate: RateControl {
                state: State::Increase,
                target: initial.clamp(floor, ceiling),
                floor,
                ceiling,
                capacity: None,
                last_change: None,
                phase: Phase::Start,
            },
        }
    }

    /// Reads the arrivals of one feedback report, in send order, and moves the target.
    pub fn update(
        &mut self,
        arrivals: impl Iterator<Item = Arrival>,
        acked: Bitrate,
        rtt: Duration,
        now: Instant,
    ) {
        for arrival in arrivals {
            if let Some(sample) = self.groups.add(arrival) {
                self.trendline.add(sample);
            }
        }
        self.rate.update(self.trendline.usage, acked, rtt, now);
    }

    /// Whether the trend of the queue delay reads overuse now.
    pub fn overusing(&self) -> bool {
        self.trendline.usage == Usage::Overuse
    }

    /// The delay-based target.
    pub fn target(&self) -> Bitrate {
        self.rate.target
    }

    /// Sets the target to `rate`, which a probe saw the link carry with no overuse. The link
    /// carries more than at the last decrease, so its known capacity no longer holds.
    pub fn raise(&mut self, rate: Bitrate, now: Instant) {
        if rate.as_f64() > self.rate.target.as_f64() {
            self.rate.capacity = None;
            self.rate.set(rate, now);
        }
    }

    /// Lifts the target back to `floor`, after a decrease that a failed probe caused.
    pub fn restore(&mut self, floor: Bitrate) {
        if floor.as_f64() > self.rate.target.as_f64() {
            self.rate.target = floor.clamp(self.rate.floor, self.rate.ceiling);
        }
    }
}
