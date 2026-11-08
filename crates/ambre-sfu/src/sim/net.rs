//! The SFU core and synthetic participants on one simulated network, in simulated time.
//!
//! Each participant reaches the SFU over a link of two impairments, one per direction. A
//! datagram leaves at the time its impairment answers, so a run is the same on every machine.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use ambre_core::{Seq, Validated};
use str0m::net::Transmit;

use super::client::Synth;
use super::impair::{Impairment, Profile};
use crate::forward::{Participant, Room, Sfu, Signal};

/// The step of the simulated clock.
const STEP: Duration = Duration::from_millis(2);

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

/// One participant and its link.
pub struct Member {
    pub synth: Synth,
    pub id: Seq<Participant>,
    pub up: Impairment,
    pub down: Impairment,
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
}

impl Net {
    pub fn new() -> Self {
        let now = Instant::now();
        let sfu_addr: SocketAddr = (Ipv4Addr::new(10, 0, 0, 254), 4000).into();
        Self {
            now,
            epoch: now,
            sfu: Sfu::new(sfu_addr),
            sfu_addr,
            members: Vec::new(),
            flights: BinaryHeap::new(),
            order: 0,
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
                To::Sfu => self.sfu.receive(self.now, flight.source, &flight.contents),
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
        while let Some((id, Signal::Offer(offer))) = self.sfu.poll_signal() {
            if let Some(m) = self.members.iter_mut().find(|m| m.id == id) {
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
            if let Some(at) = m.down.admit(self.now, t.contents.len()) {
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
        for m in &mut self.members {
            for t in m.synth.outbox.drain(..) {
                if let Some(at) = m.up.admit(self.now, t.contents.len()) {
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
