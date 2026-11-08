//! The congestion control against a modelled link, in simulated time. The link holds a queue at its capacity,
//! a one-way delay, and a seeded loss before the queue. Nothing here reads the wall clock.

use super::*;

use crate::sim::Seeded;

/// The size of each modelled packet.
const PACKET: usize = 1200;
/// The step of the sender.
const STEP: Duration = Duration::from_millis(5);
/// The period of the transport-wide feedback of the receiver.
const FEEDBACK: Duration = Duration::from_millis(100);

/// A sender that sends at the target of its controller, up to `ideal`, over one link.
struct Link {
    congestion: Congestion,
    now: Instant,
    capacity: Bitrate,
    one_way: Duration,
    loss: f64,
    rng: Seeded,
    busy_until: Instant,
    seq: u64,
    debt: f64,
    ideal: Bitrate,
    /// Each packet sent and not yet reported: its ack as the receiver will report it.
    unreported: Vec<TwccAck>,
    next_feedback: Instant,
    /// Feedback reports on their way back, with the instant each reaches the sender.
    returning: Vec<(Instant, Vec<TwccAck>)>,
    /// The target after each step, with its instant.
    targets: Vec<(Instant, Bitrate)>,
    /// The next layer step the sender offers the controller to probe, if any.
    step_up: Option<Bitrate>,
    /// After each step: whether the delay bound read overuse, and whether a probe padded.
    probes: Vec<(Overuse, Padding)>,
}

/// Whether the delay bound read overuse after one step of the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overuse {
    No,
    Yes,
}

/// Whether a probe padded the link after one step of the sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Padding {
    No,
    Yes,
}

impl Link {
    fn new(capacity: Bitrate, rtt: Duration, loss: f64, ideal: Bitrate) -> Self {
        let now = Instant::now();
        let congestion = Congestion::new(Bitrate::kbps(1000));
        Self {
            congestion,
            now,
            capacity,
            one_way: rtt / 2,
            loss,
            rng: Seeded::new(7),
            busy_until: now,
            seq: 0,
            debt: 0.0,
            ideal,
            unreported: Vec::new(),
            next_feedback: now + FEEDBACK,
            returning: Vec::new(),
            targets: Vec::new(),
            step_up: None,
            probes: Vec::new(),
        }
    }

    fn run(&mut self, span: Duration) {
        let end = self.now + span;
        while self.now < end {
            self.step();
        }
    }

    fn step(&mut self) {
        // A probe pads above the media, up to its pace.
        let rate = match self.step_up {
            Some(step) => {
                self.congestion.probe(Some(step), self.now);
                let media = self.congestion.target().min(self.ideal);
                self.congestion
                    .padding(self.now)
                    .map_or(media, |pace| pace.max(media))
            }
            None => self.congestion.target().min(self.ideal),
        };
        self.debt += rate.as_f64() * STEP.as_secs_f64() / 8.0;
        while self.debt >= PACKET as f64 {
            self.debt -= PACKET as f64;
            self.seq += 1;
            let lost = self.rng.unit() < self.loss;
            let remote_recv = (!lost).then(|| {
                let wire = Duration::from_secs_f64(PACKET as f64 * 8.0 / self.capacity.as_f64());
                let start = self.busy_until.max(self.now);
                self.busy_until = start + wire;
                self.busy_until + self.one_way
            });
            self.unreported.push(TwccAck {
                seq: self.seq,
                sent: self.now,
                size: PACKET,
                remote_recv,
                feedback_recv: self.now,
            });
        }
        if self.now >= self.next_feedback {
            self.next_feedback += FEEDBACK;
            // The receiver reports what has arrived by now; the report travels one way back.
            let at = self.now;
            let back = at + self.one_way;
            let (arrived, waiting): (Vec<TwccAck>, Vec<TwccAck>) = self
                .unreported
                .drain(..)
                .partition(|a| a.remote_recv.is_none_or(|r| r <= at));
            self.unreported = waiting;
            let report = arrived
                .into_iter()
                .map(|a| TwccAck {
                    feedback_recv: back,
                    ..a
                })
                .collect();
            self.returning.push((back, report));
        }
        let now = self.now;
        let (due, later): (Vec<_>, Vec<_>) =
            self.returning.drain(..).partition(|(at, _)| *at <= now);
        self.returning = later;
        for (_, report) in due {
            self.congestion.on_feedback(&report);
        }
        self.targets.push((now, self.congestion.target()));
        let overuse = if self.congestion.congested() {
            Overuse::Yes
        } else {
            Overuse::No
        };
        let padding = match self.congestion.padding(now) {
            Some(_) => Padding::Yes,
            None => Padding::No,
        };
        self.probes.push((overuse, padding));
        self.now += STEP;
    }

    fn lowest_since(&self, from: Instant) -> Bitrate {
        self.targets
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(_, t)| *t)
            .fold(MAX_TARGET, |a, b| a.min(b))
    }
}

/// `CALL-044`: the target climbs to the 3 Mbit/s the sender has, and holds it. The link carries
/// 5 Mbit/s at 150 ms of round trip, with a random loss of 2 %, 5 % or 10 %.
#[test]
fn the_target_holds_under_random_loss_up_to_ten_percent() {
    for loss in [0.02, 0.05, 0.10] {
        let mut link = Link::new(
            Bitrate::mbps(5),
            Duration::from_millis(150),
            loss,
            Bitrate::mbps(3),
        );
        link.run(Duration::from_secs(30));
        let from = link.now - Duration::from_secs(15);
        let lowest = link.lowest_since(from);
        assert!(
            lowest.as_f64() >= Bitrate::mbps(3).as_f64(),
            "at a loss of {loss}, the target fell to {lowest} over the last 15 s"
        );
    }
}

/// The controller half of `CALL-038`. A link carries 3 Mbit/s, then a cap of 800 kbit/s falls
/// on it. The target falls under the cap within 1 s, so the layer choice can drop a layer.
#[test]
fn the_target_falls_under_a_new_cap_within_a_second() {
    let mut link = Link::new(
        Bitrate::mbps(5),
        Duration::from_millis(150),
        0.02,
        Bitrate::mbps(3),
    );
    link.run(Duration::from_secs(20));
    let capped = link.now;
    link.capacity = Bitrate::kbps(800);
    link.run(Duration::from_secs(3));
    let under = link
        .targets
        .iter()
        .find(|(at, t)| *at >= capped && t.as_f64() < Bitrate::kbps(800).as_f64())
        .map(|(at, _)| *at - capped);
    assert!(
        under.is_some_and(|d| d <= Duration::from_secs(1)),
        "the target fell under the cap after {under:?}"
    );
}

/// Above 10 % of random loss, the loss bound lowers the target, as the GCC draft states. No rule
/// binds the SFU there; the test keeps the loss bound from a silent removal.
#[test]
fn the_target_falls_under_random_loss_above_ten_percent() {
    let mut link = Link::new(
        Bitrate::mbps(5),
        Duration::from_millis(150),
        0.20,
        Bitrate::mbps(3),
    );
    link.run(Duration::from_secs(30));
    let from = link.now - Duration::from_secs(5);
    let lowest = link.lowest_since(from);
    assert!(
        lowest.as_f64() < Bitrate::mbps(2).as_f64(),
        "at a loss of 20 %, the target stayed at {lowest} or more"
    );
}

/// `CALL-052`: a probe that raises overuse stops at once. A link carries 1 Mbit/s at 20 ms of
/// round trip and 2 % loss, under 600 kbit/s of media. So the delay bound reads overuse within
/// the send span of a probe. The sender then offers a step of 2.5 Mbit/s, which the link cannot
/// carry. Each step that reads overuse pads nothing, no probe pads longer than its send span,
/// and the target never takes the step.
#[test]
fn a_probe_that_raises_overuse_stops_at_once() {
    let mut link = Link::new(
        Bitrate::mbps(1),
        Duration::from_millis(20),
        0.02,
        Bitrate::kbps(600),
    );
    link.run(Duration::from_secs(20));
    let from = link.probes.len();
    link.step_up = Some(Bitrate::kbps(2500));
    link.run(Duration::from_secs(20));
    let after = link.probes.get(from..).unwrap_or_default();
    let padded = after.iter().filter(|(_, p)| *p == Padding::Yes).count();
    let both = after
        .iter()
        .filter(|(o, p)| *o == Overuse::Yes && *p == Padding::Yes)
        .count();
    // The longest run of steps that padded, and whether the probe that ended it saw overuse.
    let mut longest = 0;
    let mut run = 0;
    let mut stopped_by_overuse = 0;
    for pair in after.windows(2) {
        if let [(_, Padding::Yes), (overuse, next)] = pair {
            run += 1;
            longest = longest.max(run);
            if *next == Padding::No && *overuse == Overuse::Yes {
                stopped_by_overuse += 1;
            }
        } else {
            run = 0;
        }
    }
    eprintln!("{padded} steps padded, {stopped_by_overuse} probes stopped by overuse");
    assert!(padded > 0, "no probe padded the link");
    assert!(
        stopped_by_overuse > 0,
        "no probe raised overuse in its send span"
    );
    assert_eq!(both, 0, "{both} steps padded under overuse");
    assert!(
        Duration::from_millis(5) * longest <= PROBE_SEND,
        "a probe padded for {longest} steps"
    );
    let highest = link
        .targets
        .iter()
        .skip(from)
        .map(|(_, t)| t.as_f64())
        .fold(0.0, f64::max);
    assert!(
        highest < 2_500_000.0,
        "the target took the step: {highest:.0} bit/s"
    );
}
