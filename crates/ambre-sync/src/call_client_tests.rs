//! The call client over a model of the relay's call socket (`ambre-spec-call-media.md
//! section 3`, signalling).
//!
//! The model relay keeps the rules of the call socket that the database tier holds on the
//! relay:
//!
//! * one sender index per join, never reused (`CALL-056`);
//! * each announced join to every other seat, and to a later seat in its roster (`CALL-057`);
//! * each wrap to the seat of its index alone (`CALL-058`).
//!
//! Each session runs the real client and the real keys. A model of the call worker takes its
//! handoffs: a sealer for its own key, and the receive keys of the others.

use std::collections::BTreeMap;

use ambre_core::id::{Call, Channel, Id};
use ambre_core::{
    ClearChunk, Instant, KeyIdentifier, Ms, PageLoad, Payload, Role, SealedChunk, SenderIndex, Seq,
    TimeSpan, Track, UnixMs, Validated,
};
use ambre_crypto::call::{ChunkSealer, ReceiveKeys, SenderKey};
use ambre_protocol::MembershipAct;
use ambre_protocol::signaling::{CallClientMsg, CallServerMsg, Seated, SfuMsg};

use super::{CallClient, CallEnd, CallTarget, HELD_WRAPS, Steps};
use crate::membership_log::tests::{DOC, Person, append};
use crate::membership_log::{Log, digest, no_self_join};
use ambre_crypto::call::KeyHandoff;

const TARGET: CallTarget = CallTarget {
    call: Id::<Call>::from_bytes([21; 16]),
    channel: Id::<Channel>::from_bytes([22; 16]),
};

/// What the call worker of one session holds, as the handoffs built it.
#[derive(Default)]
struct Worker {
    sealer: Option<ChunkSealer>,
    receive: ReceiveKeys,
    /// Every key handed to the session, which a curious member keeps.
    held: Vec<SenderKey>,
}

impl Worker {
    fn hand(&mut self, own: Option<Seq<SenderIndex>>, handoff: KeyHandoff) {
        match handoff {
            KeyHandoff::Seal {
                generation, key, ..
            } => {
                self.held.push(key.clone());
                self.sealer = own.and_then(|sender| {
                    ChunkSealer::new(
                        &key,
                        KeyIdentifier {
                            track: Track::Audio,
                            sender,
                            generation,
                        },
                    )
                    .ok()
                });
            }
            KeyHandoff::Open {
                sender,
                generation,
                key,
            } => {
                self.held.push(key.clone());
                self.receive
                    .insert(sender, generation, key, Instant::<PageLoad>::default());
            }
            KeyHandoff::Forget { sender } => self.receive.forget(sender),
            // The receiver clock of this model stays at zero, as for each `Open`.
            KeyHandoff::Close {
                sender,
                generation,
                within,
            } => self
                .receive
                .close(sender, generation, within, Instant::<PageLoad>::default()),
            // A curious member keeps every key it held past the end of the call.
            KeyHandoff::End => {}
        }
    }

    fn seal(&mut self) -> Payload<SealedChunk> {
        let sealer = self.sealer.as_mut().expect("a session seals under its key");
        sealer
            .seal(&Payload::<ClearChunk>::new(b"opus".to_vec()))
            .expect("a chunk seals")
    }

    fn opens(&mut self, chunk: &Payload<SealedChunk>) -> bool {
        self.receive
            .open_audio(
                Payload::new(chunk.as_bytes().to_vec()),
                Instant::<PageLoad>::default(),
            )
            .is_ok()
    }

    /// Whether any key the session ever held, or a ratchet of one, opens `chunk`.
    fn curious_opens(&self, chunk: &Payload<SealedChunk>) -> bool {
        let Ok(header) = ambre_crypto::call::ChunkHeader::read_audio(chunk) else {
            return false;
        };
        self.held.iter().any(|key| {
            let mut key = key.clone();
            (0..8).any(|_| {
                let mut keys = ReceiveKeys::default();
                keys.insert(
                    header.kid.sender,
                    header.kid.generation,
                    key.clone(),
                    Instant::<PageLoad>::default(),
                );
                let opened = keys
                    .open_audio(
                        Payload::new(chunk.as_bytes().to_vec()),
                        Instant::<PageLoad>::default(),
                    )
                    .is_ok();
                key = key.ratcheted();
                opened
            })
        })
    }
}

/// One session: the person, the client, the worker, and where its socket stands.
struct Session {
    person: usize,
    client: CallClient,
    worker: Worker,
    /// The seat the model relay gave it, while its socket is open.
    seat: Option<Seq<SenderIndex>>,
    sfu: Vec<SfuMsg>,
    /// The signals for the page, in order.
    page: Vec<ambre_bus::CallSignal>,
    ended: Option<CallEnd>,
}

/// The test world: the people, the channel log, the sessions and the model relay.
struct World {
    owner: Person,
    people: Vec<Person>,
    entries: Vec<ambre_protocol::MembershipEntry>,
    log: Log,
    sessions: Vec<Session>,
    /// The seats of the model relay: the session of each sender index, and its announced
    /// join. The next index only grows.
    seats: BTreeMap<Seq<SenderIndex>, (usize, Option<Payload<ambre_core::SignedJoins>>)>,
    next_index: u64,
    now: u64,
    /// While set, the model relay holds each announced join, as a relay of several
    /// processes may. The wraps of the joiner then arrive first.
    defer_joins: bool,
    deferred: Vec<(usize, CallServerMsg)>,
}

fn walk(entries: &[ambre_protocol::MembershipEntry]) -> Log {
    Log::walk(DOC, entries.first().and_then(digest), entries, no_self_join)
}

impl World {
    /// A channel whose owner invited `members` people. One more person is no member.
    fn new(members: usize) -> Self {
        let owner = Person::new(100);
        let people: Vec<Person> = (0..=members)
            .map(|i| Person::new(u8::try_from(i + 1).expect("a few people")))
            .collect();
        let mut entries = Vec::new();
        append(
            &mut entries,
            &owner,
            &owner,
            MembershipAct::Create,
            Role::Owner,
        );
        for person in people.iter().take(members) {
            append(
                &mut entries,
                &owner,
                person,
                MembershipAct::Invite,
                Role::Editor,
            );
        }
        let log = walk(&entries);
        Self {
            owner,
            people,
            entries,
            log,
            sessions: Vec::new(),
            seats: BTreeMap::new(),
            next_index: 0,
            now: 2_000_000,
            defer_joins: false,
            deferred: Vec::new(),
        }
    }

    fn instant(&self) -> Instant<UnixMs> {
        Instant::new(self.now)
    }

    /// Opens a session of `person`: its client sends the join frame, and the model relay
    /// seats it and answers its admission.
    fn join(&mut self, person: usize) -> usize {
        let member = self.people.get(person).expect("a person").id;
        let client = CallClient::new(TARGET, member);
        let token = ambre_core::Credential::new("token".to_owned());
        assert!(matches!(
            client.join_frame(token),
            CallClientMsg::Join { channel, .. } if channel == TARGET.channel
        ));
        self.sessions.push(Session {
            person,
            client,
            worker: Worker::default(),
            seat: None,
            sfu: Vec::new(),
            page: Vec::new(),
            ended: None,
        });
        let session = self.sessions.len() - 1;
        let sender = Seq::new(self.next_index);
        self.next_index += 1;
        self.seats.insert(sender, (session, None));
        let roster = self
            .seats
            .iter()
            .filter_map(|(&index, (_, join))| {
                join.clone().map(|join| Seated {
                    sender: index,
                    join,
                })
            })
            .collect();
        self.sessions.get_mut(session).expect("a session").seat = Some(sender);
        let admitted = CallServerMsg::Admitted {
            admission: ambre_core::Credential::new("admission".to_owned()),
            sfu: Some(Validated::parse("wss://sfu.example/call").expect("an address")),
            sender,
            roster,
        };
        self.deliver(session, admitted);
        session
    }

    /// Hands `msg` to the client of `session`, then carries out its steps.
    fn deliver(&mut self, session: usize, msg: CallServerMsg) {
        self.deliver_timed(session, msg);
    }

    /// As [`Self::deliver`], and answers the delay after which the client asks a tick.
    fn deliver_timed(&mut self, session: usize, msg: CallServerMsg) -> Option<TimeSpan<Ms>> {
        let now = self.instant();
        let log = walk(&self.entries);
        let s = self.sessions.get_mut(session)?;
        let key = &self.people.get(s.person).expect("a person").key;
        let steps = s.client.on_relay(msg, key, &log, now);
        let tick_in = steps.tick_in;
        self.carry(session, steps);
        tick_in
    }

    /// Delivers each join the model relay held, in order.
    fn release(&mut self) {
        self.defer_joins = false;
        for (session, msg) in std::mem::take(&mut self.deferred) {
            self.deliver(session, msg);
        }
    }

    /// Hands `count` wraps to `session` from senders it never admits. Each one waits.
    fn hold_strangers(&mut self, session: usize, count: u64) {
        let to = self
            .sessions
            .get(session)
            .and_then(|s| s.seat)
            .expect("a seat");
        // A wrap of the length the client reads: the nonce, the sender index, the key, the
        // departures and the place, then the tag (`CALL-159`).
        let stranger_wrap = Payload::new(vec![7; 24 + 8 + 32 + (1 + 8 * 16) + 8 + 16]);
        assert!(
            ambre_crypto::call::WrappedSenderKey::from_bytes(&stranger_wrap).is_ok(),
            "a stranger's wrap has the length of a wrap"
        );
        for stranger in 0..count {
            let wrap = CallServerMsg::Wrap {
                from: Seq::new(10_000 + stranger),
                to,
                wrap: stranger_wrap.clone(),
            };
            self.deliver(session, wrap);
        }
    }

    /// The model relay: each frame of a session's steps, under the rules of the call socket.
    fn carry(&mut self, session: usize, steps: Steps) {
        let Some(s) = self.sessions.get_mut(session) else {
            return;
        };
        let own = s.client.own();
        for handoff in steps.handoffs {
            s.worker.hand(own, handoff);
        }
        s.sfu.extend(steps.sfu);
        s.page.extend(steps.page);
        s.ended = s.ended.or(steps.ended);
        let Some(from) = s.seat else {
            return;
        };
        for frame in steps.relay {
            match frame {
                CallClientMsg::Join { .. } | CallClientMsg::Start { .. } | CallClientMsg::Leave => {
                }
                CallClientMsg::Announce(join) => {
                    let Some((_, kept)) = self.seats.get_mut(&from) else {
                        continue;
                    };
                    if kept.is_some() {
                        continue;
                    }
                    *kept = Some(join.clone());
                    let others: Vec<usize> = self
                        .seats
                        .iter()
                        .filter(|(index, _)| **index != from)
                        .map(|(_, (other, _))| *other)
                        .collect();
                    for other in others {
                        let joined = CallServerMsg::Joined(Seated {
                            sender: from,
                            join: join.clone(),
                        });
                        if self.defer_joins {
                            self.deferred.push((other, joined));
                        } else {
                            self.deliver(other, joined);
                        }
                    }
                }
                CallClientMsg::Wrap { to, wrap } => {
                    if let Some(&(other, _)) = self.seats.get(&to) {
                        self.deliver(other, CallServerMsg::Wrap { from, to, wrap });
                    }
                }
            }
        }
    }

    /// The socket of `session` closes: the model relay announces the leave to the others.
    fn leave(&mut self, session: usize) {
        let Some(seat) = self.sessions.get_mut(session).and_then(|s| s.seat.take()) else {
            return;
        };
        self.seats.remove(&seat);
        let others: Vec<usize> = self.seats.values().map(|(other, _)| *other).collect();
        for other in others {
            self.deliver(other, CallServerMsg::Left(seat));
        }
    }

    /// The owner revokes `person` in the log of the channel. No session reads it yet.
    fn revoke(&mut self, person: usize) {
        let removed = self.people.get(person).expect("a person");
        append(
            &mut self.entries,
            &self.owner,
            removed,
            MembershipAct::Revoke,
            Role::Editor,
        );
        self.log = walk(&self.entries);
    }

    /// `session` reads the log again, and answers the delay of the tick its client asks.
    fn heed_timed(&mut self, session: usize) -> Option<TimeSpan<Ms>> {
        let now = self.instant();
        let s = self.sessions.get_mut(session)?;
        let steps = s.client.heed(&self.log, now);
        let tick_in = steps.tick_in;
        self.carry(session, steps);
        tick_in
    }

    /// The owner removes `person` from the channel. Each session reads the log again.
    fn remove(&mut self, person: usize) {
        self.revoke(person);
        let now = self.instant();
        for session in 0..self.sessions.len() {
            let Some(s) = self.sessions.get_mut(session) else {
                continue;
            };
            let steps = s.client.heed(&self.log, now);
            self.carry(session, steps);
        }
    }

    /// The clock moves past the switch delay, and each session ticks.
    fn switch(&mut self) {
        self.tick_after(TimeSpan::new(crate::call::SWITCH_DELAY.get() + 1));
    }

    /// The clock moves by `span`, and each session ticks.
    fn tick_after(&mut self, span: TimeSpan<Ms>) {
        self.now += span.get();
        let now = self.instant();
        for session in 0..self.sessions.len() {
            let Some(s) = self.sessions.get_mut(session) else {
                continue;
            };
            let steps = s.client.tick(now);
            self.carry(session, steps);
        }
    }

    fn worker(&mut self, session: usize) -> &mut Worker {
        &mut self.sessions.get_mut(session).expect("a session").worker
    }
}

/// `CALL-059`: two clients join one call over the relay. Each one signs its join under the
/// index of its admission and announces it, and each one wraps its key to the other. Each
/// then holds the sender key of the other: a chunk one seals, the other opens.
#[test]
fn two_clients_over_the_relay_each_hold_the_other_sender_key() {
    let mut world = World::new(2);
    let alice = world.join(0);
    let bob = world.join(1);
    for session in [alice, bob] {
        assert_eq!(world.sessions.get(session).and_then(|s| s.ended), None);
    }
    let from_alice = world.worker(alice).seal();
    let from_bob = world.worker(bob).seal();
    assert!(world.worker(bob).opens(&from_alice), "bob opens alice");
    assert!(world.worker(alice).opens(&from_bob), "alice opens bob");
}

/// `CALL-059`: a third client joins late. Each of the three holds the sender key of each
/// other, and the one who joined late opens nothing sealed before its join.
#[test]
fn a_late_client_holds_every_key_from_its_join_on() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let before = world.worker(alice).seal();
    let carol = world.join(2);
    let sessions = [alice, bob, carol];
    for &from in &sessions {
        let chunk = world.worker(from).seal();
        for &to in sessions.iter().filter(|&&to| to != from) {
            assert!(
                world.worker(to).opens(&chunk),
                "session {to} opens session {from}"
            );
        }
    }
    assert!(
        !world.worker(carol).curious_opens(&before),
        "a late joiner opens a chunk sealed before its join"
    );
}

/// `CALL-060`: the owner removes Bob from the channel during the call. The relay still
/// carries his socket and every wrap to his seat, as a relay that knows no log does. Alice
/// reads the log again and switches. Once the switch delay passes, Bob opens nothing she
/// seals, under any key he ever held or a ratchet of one.
#[test]
fn a_member_removed_from_the_channel_holds_no_key_of_the_next_generation() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let carol = world.join(2);
    let before = world.worker(alice).seal();
    assert!(world.worker(bob).opens(&before));
    world.remove(1);
    world.switch();
    let after = world.worker(alice).seal();
    assert!(
        !world.worker(bob).curious_opens(&after),
        "a removed member opens the next generation"
    );
    assert!(world.worker(carol).opens(&after), "carol still opens alice");
}

/// `CALL-061`: a seat whose signed join names another sender index than the one the relay
/// gave it gets no wrap. Here the relay hands Alice a fresh join of Bob, signed under the
/// index 7, as the join of the seat 8. The same join under its own seat then gets her wrap.
#[test]
fn a_join_signed_under_another_index_than_its_seat_gets_no_wrap() {
    let mut world = World::new(2);
    let alice = world.join(0);
    let now = world.instant();
    let log = walk(&world.entries);
    let bob = world.people.get(1).expect("bob");
    let mut client = CallClient::new(TARGET, bob.id);
    let steps = client.on_relay(
        CallServerMsg::Admitted {
            admission: ambre_core::Credential::new("admission".to_owned()),
            sfu: None,
            sender: Seq::new(7),
            roster: Vec::new(),
        },
        &bob.key,
        &log,
        now,
    );
    let Some(CallClientMsg::Announce(join)) = steps.relay.into_iter().next() else {
        panic!("bob announces his join");
    };
    let s = world.sessions.get_mut(alice).expect("alice");
    let key = &world.people.first().expect("alice").key;
    let mut meet = |sender: u64| {
        s.client.on_relay(
            CallServerMsg::Joined(Seated {
                sender: Seq::new(sender),
                join: join.clone(),
            }),
            key,
            &log,
            now,
        )
    };
    let relabelled = meet(8);
    assert!(
        relabelled.relay.is_empty(),
        "a wrap left for a relabelled join"
    );
    let own = meet(7);
    assert!(
        own.relay
            .iter()
            .any(|f| matches!(f, CallClientMsg::Wrap { to, .. } if *to == Seq::new(7))),
        "the join under its own seat gets a wrap"
    );
}

/// `CALL-019`, through the client: a person who is no member of the channel joins the call.
/// The relay seats it, as a relay whose check it got past would. No member wraps a key to it,
/// so it opens nothing they seal.
#[test]
fn a_joiner_outside_the_channel_gets_no_key_through_the_client() {
    let mut world = World::new(2);
    let alice = world.join(0);
    let mallory = world.join(2);
    let chunk = world.worker(alice).seal();
    assert!(!world.worker(mallory).curious_opens(&chunk));
}

/// `CALL-062`: an admission that names the SFU opens its socket, with the admission as the
/// first frame. The refusal of a join ends the call for the client.
#[test]
fn an_admission_opens_the_socket_of_the_sfu_with_the_admission_first() {
    let mut world = World::new(1);
    let alice = world.join(0);
    let sfu = &world.sessions.get(alice).expect("alice").sfu;
    assert!(matches!(sfu.first(), Some(SfuMsg::Admission(_))), "{sfu:?}");
    let now = world.instant();
    let mut refused = CallClient::new(TARGET, world.people.first().expect("a person").id);
    let key = &world.people.first().expect("a person").key;
    let steps = refused.on_relay(
        CallServerMsg::Refused(ambre_protocol::CallRefusal::Full),
        key,
        &world.log,
        now,
    );
    assert_eq!(
        steps.ended,
        Some(CallEnd::Refused(ambre_protocol::CallRefusal::Full))
    );
    assert_eq!(refused.own(), None);
}

/// `CALL-027`, through the client: a leave switches each remaining session to a fresh key
/// once the delay passes. The one who left opens nothing sealed after the switch.
#[test]
fn a_leave_switches_the_remaining_clients_and_the_leaver_opens_nothing_after() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let carol = world.join(2);
    world.leave(bob);
    world.switch();
    let after = world.worker(alice).seal();
    assert!(!world.worker(bob).curious_opens(&after));
    assert!(world.worker(carol).opens(&after));
}

/// `CALL-087`, through the client: the page learns the member of each sender index the client
/// admits. It learns them from the roster and from a later join, and it learns each leave. A joiner outside the
/// channel names no member to the page.
#[test]
fn the_page_learns_the_member_of_each_admitted_sender_and_its_leave() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let carol = world.join(2);
    let mallory = world.join(3);
    let seat = |world: &World, session: usize| {
        world
            .sessions
            .get(session)
            .and_then(|s| s.seat)
            .expect("a seat")
    };
    let member = |world: &World, person: usize| world.people.get(person).expect("a person").id;
    let (bob_seat, carol_seat) = (seat(&world, bob), seat(&world, carol));
    world.leave(bob);
    let admitted = |world: &World,
                    session: usize|
     -> Vec<(u64, ambre_core::id::Id<ambre_core::id::User>)> {
        world
            .sessions
            .get(session)
            .expect("a session")
            .page
            .iter()
            .filter_map(|signal| match signal {
                ambre_bus::CallSignal::Admitted { sender, member } => Some((sender.get(), *member)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        admitted(&world, alice),
        vec![
            (bob_seat.get(), member(&world, 1)),
            (carol_seat.get(), member(&world, 2))
        ]
    );
    // Carol met Alice and Bob in the roster of her admission.
    assert_eq!(admitted(&world, carol).len(), 2);
    // Mallory is no member, so no one names her; she still reads the three members.
    assert_eq!(admitted(&world, mallory).len(), 3);
    let departed = world
        .sessions
        .get(alice)
        .expect("alice")
        .page
        .iter()
        .any(|signal| matches!(signal, ambre_bus::CallSignal::Departed(s) if *s == bob_seat));
    assert!(departed, "Alice never read Bob leave");
}

/// A wrap that arrives ahead of the join of its sender waits for it, then opens: the relay may
/// reorder the two across its processes.
#[test]
fn a_wrap_ahead_of_its_join_opens_once_the_join_arrives() {
    let mut world = World::new(2);
    let alice = world.join(0);
    world.hold_strangers(alice, 3);
    world.defer_joins = true;
    let bob = world.join(1);
    let from_bob = world.worker(bob).seal();
    assert!(
        !world.worker(alice).opens(&from_bob),
        "no key before the join"
    );
    world.release();
    let from_bob = world.worker(bob).seal();
    assert!(world.worker(alice).opens(&from_bob), "the held wrap opens");
}

/// The client holds `HELD_WRAPS` wraps of senders it has not admitted, and drops the next one.
#[test]
fn the_client_holds_a_bounded_number_of_wraps_ahead_of_their_joins() {
    for (strangers, opens) in [(HELD_WRAPS.get() - 1, true), (HELD_WRAPS.get(), false)] {
        let mut world = World::new(2);
        let alice = world.join(0);
        world.hold_strangers(alice, strangers);
        world.defer_joins = true;
        let bob = world.join(1);
        world.release();
        let from_bob = world.worker(bob).seal();
        assert_eq!(
            world.worker(alice).opens(&from_bob),
            opens,
            "{strangers} wraps held before the one of bob"
        );
    }
}

/// A leave drops the wraps its sender left waiting, and keeps those of every other sender.
#[test]
fn a_leave_keeps_the_wraps_other_senders_left_waiting() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let carol = world.join(2);
    world.defer_joins = true;
    let bob = world.join(1);
    world.leave(carol);
    world.release();
    let from_bob = world.worker(bob).seal();
    assert!(
        world.worker(alice).opens(&from_bob),
        "the wrap of bob survives a leave"
    );
}

/// After a leave, the client asks a tick at most 10 ms past the switch delay. The tick at
/// that delay switches its key.
#[test]
fn a_leave_asks_a_tick_that_finds_the_switch_due() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let carol = world.join(2);
    let seat = world
        .sessions
        .get_mut(bob)
        .and_then(|s| s.seat.take())
        .expect("a seat");
    world.seats.remove(&seat);
    world.deliver(carol, CallServerMsg::Left(seat));
    let tick_in = world
        .deliver_timed(alice, CallServerMsg::Left(seat))
        .expect("a tick");
    let delay = crate::call::SWITCH_DELAY.get();
    assert!(tick_in.get() > delay && tick_in.get() <= delay + 10);
    world.tick_after(tick_in);
    let after = world.worker(alice).seal();
    assert!(!world.worker(bob).curious_opens(&after));
    assert!(world.worker(carol).opens(&after));
}

/// `CALL-118`: a tick that finds the switch not yet due asks another. The wall clock stepped
/// back between the leave and the timer, and the removed key would serve until the next leave.
/// Once the switch is done, a tick asks none.
#[test]
fn a_tick_before_the_switch_asks_another() {
    let mut world = World::new(3);
    let alice = world.join(0);
    let bob = world.join(1);
    let carol = world.join(2);
    let seat = world
        .sessions
        .get_mut(bob)
        .and_then(|s| s.seat.take())
        .expect("a seat");
    world.seats.remove(&seat);
    world.deliver(carol, CallServerMsg::Left(seat));
    world
        .deliver_timed(alice, CallServerMsg::Left(seat))
        .expect("a tick");
    let delay = crate::call::SWITCH_DELAY.get();
    let early = Instant::new(world.now + delay - 100);
    let steps = world
        .sessions
        .get_mut(alice)
        .expect("a session")
        .client
        .tick(early);
    assert!(steps.handoffs.is_empty(), "the switch is not due");
    assert!(
        steps.tick_in.is_some(),
        "a tick before the switch asks another"
    );
    world.switch();
    let after = world.worker(alice).seal();
    assert!(!world.worker(bob).curious_opens(&after));
    let later = world.instant();
    let steps = world
        .sessions
        .get_mut(alice)
        .expect("a session")
        .client
        .tick(later);
    assert!(
        steps.tick_in.is_none(),
        "a tick after the switch asks no other"
    );
}

/// A read of the log that removes a participant asks a tick, even with no one left to wrap
/// to. A read that changes nothing asks none.
#[test]
fn a_read_of_the_log_asks_a_tick_when_it_switches_alone() {
    let mut world = World::new(2);
    let alice = world.join(0);
    world.join(1);
    assert_eq!(world.heed_timed(alice), None);
    world.revoke(1);
    assert!(world.heed_timed(alice).is_some());
}
