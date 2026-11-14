//! The SFU core and synthetic participants on one simulated network, in simulated time.
//!
//! Each participant reaches the SFU over a link of two impairments, one per direction. A
//! datagram leaves at the time its impairment answers, so a run is the same on every machine.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use ambre_core::{Payload, Seq, Validated};
use str0m::net::Transmit;

use super::client::Synth;
use super::impair::{Impairment, Profile};
use crate::forward::{Participant, Room, Sfu, Signal};

/// The step of the simulated clock. The SFU wakes str0m at the timeout it asks, and tokio
/// rounds that timeout to 1 ms. The pacer of str0m sends one padding packet per wake. A coarser step
/// would starve a probe of the padding the SFU sends (`CALL-050`).
const STEP: Duration = Duration::from_millis(1);

/// Where the simulated clock starts, past the origin of str0m's clock. The test exports of
/// str0m pin the wall clock of that origin, so each simulated instant maps to one NTP time
/// in every process. Started from `Instant::now()` instead, the RTT str0m reads from a
/// sender report rounds another way in each process. That RTT times the retries of a NACK,
/// and a retry one step apart moved the layer choice of a capped receiver.
const ORIGIN_OFFSET: Duration = Duration::from_secs(1);

/// Where a datagram in flight goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum To {
    Sfu,
    Client(usize),
}

/// A datagram in flight, ordered by its arrival time, then by its send order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Flight {
    at: Instant,
    order: u64,
    to: To,
    source: SocketAddr,
    contents: Vec<u8>,
}

/// The trace of the path of a run: what the guard of `sim/paths.rs` compares between
/// processes.
#[derive(Debug)]
pub struct Path {
    /// The datagrams that entered a link.
    pub datagrams: u64,
    /// An FNV-1a digest of each datagram: its time, its link, its size and its fate.
    pub digest: u64,
}

/// One direction of the link of a participant, by its index.
#[derive(Clone, Copy, Debug)]
enum Hop {
    Up(usize),
    Down(usize),
}

/// The FNV-1a offset basis and prime, 64 bits.
const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl Default for Path {
    fn default() -> Self {
        Self {
            datagrams: 0,
            digest: FNV_BASIS,
        }
    }
}

impl Path {
    /// Notes a datagram of `contents` that entered `hop` at `at` past the epoch, and
    /// leaves it at `out`, or never.
    ///
    /// The size of a DTLS record stays out. Its handshake keys come from the OS generator,
    /// so the length of its ECDSA signature varies by a byte or two between processes. Its
    /// time and its link stay in.
    fn note(&mut self, at: Duration, hop: Hop, contents: &[u8], out: Option<Duration>) {
        // RFC 7983: a first byte from 20 to 63 opens a DTLS record.
        let dtls = contents.first().is_some_and(|b| (20..=63).contains(b));
        let size = if dtls { 0 } else { contents.len() };
        let hop = match hop {
            Hop::Up(index) => index as u64 * 2,
            Hop::Down(index) => index as u64 * 2 + 1,
        };
        let fate = out.map_or(u64::MAX, |d| d.as_nanos() as u64);
        self.datagrams += 1;
        for word in [at.as_nanos() as u64, hop, size as u64, fate] {
            for byte in word.to_le_bytes() {
                self.digest = (self.digest ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
            }
        }
    }
}

/// One participant and its link.
pub struct Member {
    pub synth: Synth,
    pub id: Seq<Participant>,
    pub up: Impairment,
    pub down: Impairment,
    /// The offers the SFU sent this participant.
    pub offers: usize,
}

/// The simulated network.
pub struct Net {
    pub now: Instant,
    pub epoch: Instant,
    pub sfu: Sfu,
    sfu_addr: SocketAddr,
    pub members: Vec<Member>,
    flights: BinaryHeap<Reverse<Flight>>,
    order: u64,
    /// The seed of the run, which the trace of its path names.
    pub seed: u64,
    pub path: Path,
}

impl Net {
    /// A network with no participant. `seed` seeds the random numbers str0m draws on this
    /// thread, so a run follows one path whatever the load of the machine.
    pub fn new(seed: u64) -> Self {
        fastrand::seed(seed);
        let now = str0m::_internal_test_exports::time_origin() + ORIGIN_OFFSET;
        let sfu_addr: SocketAddr = (Ipv4Addr::new(10, 0, 0, 254), 4000).into();
        Self {
            now,
            epoch: now,
            sfu: Sfu::serial(sfu_addr),
            sfu_addr,
            members: Vec::new(),
            flights: BinaryHeap::new(),
            order: 0,
            seed,
            path: Path::default(),
        }
    }

    /// Joins a participant to `room` over a link of `profile` each way, its loss seeded by
    /// `seed`, and answers its index.
    pub fn join(&mut self, room: &str, profile: Profile, seed: u64) -> usize {
        let index = self.members.len();
        let tag = u16::try_from(index).expect("a few participants");
        let addr: SocketAddr = (Ipv4Addr::new(10, 0, 1, 1), 5000 + tag).into();
        let (mut synth, offer) = Synth::new(self.now, self.epoch, tag, addr);
        synth.playout_rtt = profile.delay * 2;
        let room = Validated::<Room>::parse(room).expect("a room name");
        let (id, answer) = self
            .sfu
            .join(self.now, room, offer)
            .expect("the SFU admits the participant");
        synth.accept_answer(answer);
        self.members.push(Member {
            synth,
            id,
            up: Impairment::new(profile, seed.wrapping_mul(2)),
            down: Impairment::new(profile, seed.wrapping_mul(2).wrapping_add(1)),
            offers: 0,
        });
        index
    }

    pub fn member(&mut self, index: usize) -> &mut Member {
        self.members.get_mut(index).expect("a participant")
    }

    /// Runs the network for `span` of simulated time.
    pub fn run(&mut self, span: Duration) {
        let end = self.now + span;
        while self.now < end {
            self.step();
        }
    }

    fn step(&mut self) {
        while self
            .flights
            .peek()
            .is_some_and(|Reverse(f)| f.at <= self.now)
        {
            let Some(Reverse(flight)) = self.flights.pop() else {
                break;
            };
            match flight.to {
                To::Sfu => self
                    .sfu
                    .receive(self.now, flight.source, Payload::new(flight.contents)),
                To::Client(index) => {
                    if let Some(m) = self.members.get_mut(index) {
                        m.synth.receive(self.now, flight.source, &flight.contents);
                    }
                }
            }
        }
        self.sfu.tick(self.now);
        for m in &mut self.members {
            m.synth.tick(self.now);
        }
        self.route();
        self.now += STEP;
    }

    /// Sends what every side has written, through its link.
    fn route(&mut self) {
        let mut from_sfu: Vec<Transmit> = Vec::new();
        self.sfu.poll(self.now, |t| from_sfu.push(t));
        while let Some((id, signal)) = self.sfu.poll_signal() {
            let Signal::Offer(offer) = signal else {
                continue;
            };
            if let Some(m) = self.members.iter_mut().find(|m| m.id == id) {
                m.offers += 1;
                let answer = m.synth.accept_offer(offer);
                self.sfu.accept_answer(id, answer);
            }
        }
        for t in from_sfu {
            let Some(index) = self
                .members
                .iter()
                .position(|m| m.synth.addr == t.destination)
            else {
                continue;
            };
            let Some(m) = self.members.get_mut(index) else {
                continue;
            };
            let admitted = m.down.admit(self.now, t.contents.len());
            let since = |at: Instant| at - self.epoch;
            self.path.note(
                since(self.now),
                Hop::Down(index),
                &t.contents,
                admitted.map(since),
            );
            if let Some(at) = admitted {
                self.order += 1;
                self.flights.push(Reverse(Flight {
                    at,
                    order: self.order,
                    to: To::Client(index),
                    source: self.sfu_addr,
                    contents: t.contents.to_vec(),
                }));
            }
        }
        for (index, m) in self.members.iter_mut().enumerate() {
            for t in m.synth.outbox.drain(..) {
                let admitted = m.up.admit(self.now, t.contents.len());
                let since = |at: Instant| at - self.epoch;
                self.path.note(
                    since(self.now),
                    Hop::Up(index),
                    &t.contents,
                    admitted.map(since),
                );
                if let Some(at) = admitted {
                    self.order += 1;
                    self.flights.push(Reverse(Flight {
                        at,
                        order: self.order,
                        to: To::Sfu,
                        source: t.source,
                        contents: t.contents.to_vec(),
                    }));
                }
            }
        }
    }
}
