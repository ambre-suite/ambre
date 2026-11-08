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
        }
    }

    fn run(&mut self, span: Duration) {
        let end = self.now + span;
        while self.now < end {
            self.step();
        }
    }

    fn step(&mut self) {
        let rate = self.congestion.target().min(self.ideal);
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
