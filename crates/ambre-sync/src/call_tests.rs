//! The key model of a call (`ambre-spec-call-media.md section 3`). A reference model runs
//! over sequences of joins, leaves, removals and delays. Each member opens exactly the
//! chunks it should.
//!
//! Each actor runs the real keys of a session. Its Sync worker holds the `CallKeys`, and a
//! call worker would hold the sealer and the receive keys. A test relay carries every join
//! and every wrap at once. An actor who leaves or is removed keeps what it holds. It also
//! keeps receiving every wrap addressed to its old index, as a curious member would. The model
//! knows none of the keys: it states who may open a chunk from who was in the call when.

use std::collections::{BTreeMap, BTreeSet};

use ambre_core::Role;
use ambre_core::id::{Channel, Id, Meeting};
use ambre_core::{
    Authored, ClearChunk, Instant, KeyIdentifier, Payload, SealedChunk, SenderIndex, Seq, Track,
    UnixMs,
};
use ambre_crypto::call::{ChunkHeader, ChunkSealer, ReceiveKeys, SenderKey, SignedJoin};
use ambre_protocol::MembershipAct;
use proptest::prelude::*;

use super::{Arrival, CallKeys, Handoff, Joining, Moves, Refusal, SWITCH_DELAY};
use crate::membership_log::tests::{DOC, Person, append};
use crate::membership_log::{Log, digest, no_self_join};

const CALL: Id<Meeting> = Id::<Meeting>::from_bytes([11; 16]);
const CHANNEL: Id<Channel> = Id::<Channel>::from_bytes([12; 16]);
/// The actors who are members of the channel; the last actor is not.
const MEMBERS: usize = 5;
const ACTORS: usize = MEMBERS + 1;

/// The real state of one session of an actor.
struct Session {
    keys: CallKeys,
    sealer: Option<ChunkSealer>,
    receive: ReceiveKeys,
    /// Every key ever handed to the session, which a curious member keeps.
    held: Vec<SenderKey>,
}

/// Whether any key of `held`, or a ratchet of one, opens `chunk` under its own identifier.
/// A member who kept every key could open that much.
fn curious_open(held: &[SenderKey], chunk: &Payload<SealedChunk>) -> bool {
    let Ok(header) = ChunkHeader::read_audio(chunk) else {
        return false;
    };
    held.iter().any(|key| {
        let mut key = key.clone();
        (0..16).any(|_| {
            let mut keys = ReceiveKeys::default();
            keys.insert(header.kid.sender, header.kid.generation, key.clone());
            let opened = keys
                .open_audio(Payload::new(chunk.as_bytes().to_vec()))
                .is_ok();
            key = key.ratcheted();
            opened
        })
    })
}

impl Session {
    fn hand(&mut self, handoff: Handoff) {
        match handoff {
            Handoff::Seal { generation, key } => {
                self.held.push(key.clone());
                let kid = KeyIdentifier {
                    track: Track::Audio,
                    sender: self.keys.own(),
                    generation,
                };
                self.sealer = ChunkSealer::new(&key, kid).ok();
            }
            Handoff::Open {
                sender,
                generation,
                key,
            } => {
                self.held.push(key.clone());
                self.receive.insert(sender, generation, key);
            }
            Handoff::Forget { sender } => self.receive.forget(sender),
        }
    }
}

/// Where a session of an actor stands.
enum Standing {
    /// In the call, since the time it joined.
    In,
    /// Left or removed at a time, and still holding what it held.
    Gone { at: u64 },
    /// Refused by the participants: it holds nothing of theirs.
    Refused,
}

/// The test world: the channel log, the sessions, the clock, and the reference model.
struct World {
    owner: Person,
    people: Vec<Person>,
    entries: Vec<ambre_protocol::MembershipEntry>,
    log: Log,
    sessions: BTreeMap<usize, (Session, Standing)>,
    next_index: u64,
    now: u64,
    removed: BTreeSet<usize>,
    /// The reference model: for each sender in the call, the time of the first leave
    /// since its last switch, and the times of its switches.
    first_leave: BTreeMap<usize, u64>,
    switches: BTreeMap<usize, Vec<u64>>,
    /// When each actor in the call joined, and which actors were in the call with it.
    met: BTreeMap<usize, BTreeSet<usize>>,
    /// The chunks sealed so far, with their sender and their time, newest last.
    archive: Vec<(usize, u64, Payload<SealedChunk>)>,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Join(usize),
    Leave(usize),
    Remove(usize),
    Advance(u64),
    Speak(usize),
    Replay(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..ACTORS).prop_map(Op::Join),
        1 => (0..ACTORS).prop_map(Op::Leave),
        1 => (0..MEMBERS).prop_map(Op::Remove),
        2 => (0u64..4_000).prop_map(Op::Advance),
        4 => (0..ACTORS).prop_map(Op::Speak),
        1 => (0..ACTORS).prop_map(Op::Replay),
    ]
}

fn walk(entries: &[ambre_protocol::MembershipEntry]) -> Log {
    Log::walk(DOC, entries.first().and_then(digest), entries, no_self_join)
}

impl World {
    fn new() -> Self {
        let owner = Person::new(100);
        let people: Vec<Person> = (0..ACTORS)
            .map(|i| Person::new(u8::try_from(i + 1).unwrap_or(1)))
            .collect();
        let mut entries = Vec::new();
        append(
            &mut entries,
            &owner,
            &owner,
            MembershipAct::Create,
            Role::Owner,
        );
        for person in people.iter().take(MEMBERS) {
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
            sessions: BTreeMap::new(),
            next_index: 1,
            now: 1_000_000,
            removed: BTreeSet::new(),
            first_leave: BTreeMap::new(),
            switches: BTreeMap::new(),
            met: BTreeMap::new(),
            archive: Vec::new(),
        }
    }

    fn instant(&self) -> Instant<UnixMs> {
        Instant::new(self.now)
    }

    fn in_call(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .filter(|(_, (_, standing))| matches!(standing, Standing::In))
            .map(|(&actor, _)| actor)
            .collect()
    }

    fn index_of(&self, actor: usize) -> Option<Seq<SenderIndex>> {
        self.sessions.get(&actor).map(|(s, _)| s.keys.own())
    }

    /// Carries every wrap to the session that holds its index, a gone one included.
    fn deliver(&mut self, from: usize, moves: Moves) {
        let Some(from_index) = self.index_of(from) else {
            return;
        };
        if let Some((session, _)) = self.sessions.get_mut(&from) {
            for handoff in moves.handoffs {
                session.hand(handoff);
            }
        }
        for wrap in moves.wraps {
            let to = self
                .sessions
                .iter()
                .find(|(_, (s, _))| s.keys.own() == wrap.to)
                .map(|(&actor, _)| actor);
            let to = to.unwrap_or_else(|| panic!("a wrap to no session"));
            // A wrap goes to a participant in the call alone (`CALL-030`, `CALL-019`).
            assert!(
                matches!(self.sessions.get(&to), Some((_, Standing::In))),
                "a wrap reached actor {to}, who is not in the call"
            );
            if let Some((session, _)) = self.sessions.get_mut(&to) {
                let handoff = session.keys.receive(from_index, &wrap.wrapped);
                let handoff = handoff.unwrap_or_else(|e| panic!("a wrap does not open: {e:?}"));
                session.hand(handoff);
            }
        }
    }

    fn join(&mut self, actor: usize) {
        if matches!(self.sessions.get(&actor), Some((_, Standing::In))) {
            return;
        }
        // A rejoin is a new session: the old one and what it held are dropped.
        self.sessions.remove(&actor);
        let Some(person) = self.people.get(actor) else {
            return;
        };
        let joining = Joining {
            call: CALL,
            channel: CHANNEL,
            member: person.id,
            sender: Seq::new(self.next_index),
            at: Instant::<Authored>::new(self.now),
        };
        self.next_index += 1;
        let (keys, handoff) =
            CallKeys::start(&person.key, joining).unwrap_or_else(|e| panic!("{e:?}"));
        let join: SignedJoin = keys.join().clone();
        let mut session = Session {
            keys,
            sealer: None,
            receive: ReceiveKeys::default(),
            held: Vec::new(),
        };
        session.hand(handoff);
        let present = self.in_call();
        let member = actor < MEMBERS && !self.removed.contains(&actor);
        // The joiner admits each participant it finds in the roster.
        let mut own_moves = Vec::new();
        for &other in &present {
            let other_join = self
                .sessions
                .get(&other)
                .map(|(s, _)| s.keys.join().clone());
            let Some(other_join) = other_join else {
                continue;
            };
            let moves = session.keys.admit(&other_join, &self.log, Arrival::Present);
            own_moves.push(moves.unwrap_or_else(|e| panic!("a member refused: {e:?}")));
        }
        let standing = if member {
            Standing::In
        } else {
            Standing::Refused
        };
        self.sessions.insert(actor, (session, standing));
        // Each participant admits the joiner, or refuses a joiner the log does not hold.
        for &other in &present {
            let result = self
                .sessions
                .get_mut(&other)
                .map(|(s, _)| s.keys.admit(&join, &self.log, Arrival::New));
            match (member, result) {
                (true, Some(Ok(moves))) => self.deliver(other, moves),
                (false, Some(Err(Refusal::NotAMember))) => {}
                (_, result) => panic!("admit of actor {actor} gave {result:?}"),
            }
        }
        if member {
            for moves in own_moves {
                self.deliver(actor, moves);
            }
            let mut with: BTreeSet<usize> = present.iter().copied().collect();
            with.insert(actor);
            for &other in &present {
                self.met.entry(other).or_default().insert(actor);
            }
            self.met.insert(actor, with);
            self.switches.insert(actor, Vec::new());
        }
    }

    /// Every participant still in the call learns that `actor` is gone.
    fn gone(&mut self, actor: usize) {
        let Some(index) = self.index_of(actor) else {
            return;
        };
        if let Some((_, standing)) = self.sessions.get_mut(&actor) {
            *standing = Standing::Gone { at: self.now };
        }
        self.first_leave.remove(&actor);
        self.switches.remove(&actor);
        for other in self.in_call() {
            self.first_leave.entry(other).or_insert(self.now);
            let now = self.instant();
            let moves = self
                .sessions
                .get_mut(&other)
                .map(|(s, _)| s.keys.depart(index, now));
            if let Some(moves) = moves {
                self.deliver(other, moves.unwrap_or_else(|e| panic!("{e:?}")));
            }
        }
    }

    fn leave(&mut self, actor: usize) {
        if matches!(self.sessions.get(&actor), Some((_, Standing::In))) {
            self.gone(actor);
        }
    }

    fn remove(&mut self, actor: usize) {
        if self.removed.contains(&actor) {
            return;
        }
        let Some(person) = self.people.get(actor) else {
            return;
        };
        append(
            &mut self.entries,
            &self.owner,
            person,
            MembershipAct::Revoke,
            Role::Editor,
        );
        self.log = walk(&self.entries);
        self.removed.insert(actor);
        if !matches!(self.sessions.get(&actor), Some((_, Standing::In))) {
            return;
        }
        if let Some((_, standing)) = self.sessions.get_mut(&actor) {
            *standing = Standing::Gone { at: self.now };
        }
        self.first_leave.remove(&actor);
        self.switches.remove(&actor);
        // Each participant reads the removal from the log alone (`CALL-028`).
        for other in self.in_call() {
            self.first_leave.entry(other).or_insert(self.now);
            let now = self.instant();
            let moves = self
                .sessions
                .get_mut(&other)
                .map(|(s, _)| s.keys.heed(&self.log, now));
            if let Some(moves) = moves {
                self.deliver(other, moves.unwrap_or_else(|e| panic!("{e:?}")));
            }
        }
    }

    fn advance(&mut self, by: u64) {
        self.now += by;
        let now = self.instant();
        for actor in self.in_call() {
            let handoff = self
                .sessions
                .get_mut(&actor)
                .and_then(|(s, _)| s.keys.tick(now));
            let model_due = self
                .first_leave
                .get(&actor)
                .is_some_and(|&first| first + SWITCH_DELAY.get() <= self.now);
            assert_eq!(handoff.is_some(), model_due, "the switch of actor {actor}");
            if model_due {
                self.first_leave.remove(&actor);
                self.switches.entry(actor).or_default().push(self.now);
            }
            if let (Some(handoff), Some((session, _))) = (handoff, self.sessions.get_mut(&actor)) {
                session.hand(handoff);
            }
        }
    }

    /// Whether the model lets `reader` open a chunk `sender` sealed at `at`.
    fn may_open(&self, reader: usize, sender: usize, at: u64) -> bool {
        if reader == sender {
            return false;
        }
        let met = self.met.get(&reader).is_some_and(|m| m.contains(&sender))
            && self.met.get(&sender).is_some_and(|m| m.contains(&reader));
        match self.sessions.get(&reader).map(|(_, standing)| standing) {
            Some(Standing::In) => met,
            Some(Standing::Gone { at: gone }) => {
                let switched = self
                    .switches
                    .get(&sender)
                    .is_some_and(|times| times.iter().any(|&t| t > *gone && t <= at));
                met && !switched
            }
            Some(Standing::Refused) | None => false,
        }
    }

    fn speak(&mut self, actor: usize) {
        if !matches!(self.sessions.get(&actor), Some((_, Standing::In))) {
            return;
        }
        let clear = Payload::<ClearChunk>::new(vec![u8::try_from(actor).unwrap_or(0); 40]);
        let chunk = self
            .sessions
            .get_mut(&actor)
            .and_then(|(s, _)| s.sealer.as_mut())
            .map(|sealer| sealer.seal(&clear));
        let chunk = chunk
            .unwrap_or_else(|| panic!("no sealer"))
            .unwrap_or_else(|e| panic!("{e}"));
        let readers: Vec<usize> = self.sessions.keys().copied().collect();
        for reader in readers {
            let expected = self.may_open(reader, actor, self.now);
            // A participant in the call opens through its receive keys alone. Any other
            // reader tries every key it ever held.
            let opened = match self.sessions.get_mut(&reader) {
                Some((session, Standing::In)) => session
                    .receive
                    .open_audio(Payload::new(chunk.as_bytes().to_vec()))
                    .is_ok(),
                Some((session, _)) => curious_open(&session.held, &chunk),
                None => false,
            };
            assert_eq!(
                opened, expected,
                "actor {reader} opening a chunk of actor {actor} at {}",
                self.now
            );
        }
        self.archive.push((actor, self.now, chunk));
        if self.archive.len() > 12 {
            self.archive.remove(0);
        }
    }

    /// A joiner opens no chunk sealed before its join.
    fn probe_history(&mut self, actor: usize) {
        let Some((session, Standing::In)) = self.sessions.get(&actor) else {
            return;
        };
        for (sender, _, chunk) in &self.archive {
            if *sender != actor {
                assert!(
                    !curious_open(&session.held, chunk),
                    "a joiner opened a chunk sealed before its join"
                );
            }
        }
    }

    /// Delivers again the join of a session in the call, which every participant saw.
    fn replay(&mut self, pick: usize) {
        let present = self.in_call();
        let Some(&from) = present.get(pick % present.len().max(1)) else {
            return;
        };
        let Some(join) = self.sessions.get(&from).map(|(s, _)| s.keys.join().clone()) else {
            return;
        };
        for actor in self.in_call() {
            let result = self
                .sessions
                .get_mut(&actor)
                .map(|(s, _)| s.keys.admit(&join, &self.log, Arrival::New));
            assert!(
                matches!(result, Some(Err(_))),
                "a replayed join was admitted: {result:?}"
            );
        }
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Join(actor) => {
                if !matches!(self.sessions.get(&actor), Some((_, Standing::In))) {
                    self.join(actor);
                    self.probe_history(actor);
                }
            }
            Op::Leave(actor) => self.leave(actor),
            Op::Remove(actor) => self.remove(actor),
            Op::Advance(by) => self.advance(by),
            Op::Speak(actor) => self.speak(actor),
            Op::Replay(pick) => self.replay(pick),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Over any sequence of joins, leaves, removals and delays, each actor opens exactly
    /// what the model allows. A participant opens every chunk of the others. A joiner opens
    /// nothing sealed before its join. A member who left or was removed opens nothing after
    /// the switch of each sender. An outsider opens nothing.
    #[test]
    fn each_member_opens_exactly_what_it_should(ops in prop::collection::vec(op(), 1..60)) {
        let mut world = World::new();
        for op in ops {
            world.apply(op);
        }
    }
}

fn person_join(world: &mut World, actor: usize) -> Seq<SenderIndex> {
    world.join(actor);
    world.index_of(actor).unwrap_or(Seq::ZERO)
}

/// A leave switches each remaining sender within the delay, and not before it.
#[test]
fn a_leave_switches_each_remaining_sender_within_the_delay() {
    let mut world = World::new();
    for actor in 0..3 {
        person_join(&mut world, actor);
    }
    world.leave(2);
    world.advance(SWITCH_DELAY.get() - 1);
    world.speak(0);
    assert!(world.switches.get(&0).is_some_and(Vec::is_empty));
    world.advance(1);
    world.speak(0);
    assert_eq!(world.switches.get(&0).map(Vec::len), Some(1));
}

/// A join a client has seen is refused, and so is a new key under an index seen before.
#[test]
fn a_join_seen_already_is_refused() {
    let mut world = World::new();
    person_join(&mut world, 0);
    person_join(&mut world, 1);
    let join = world.sessions.get(&1).map(|(s, _)| s.keys.join().clone());
    let Some(join) = join else { panic!("no join") };
    let result = world
        .sessions
        .get_mut(&0)
        .map(|(s, _)| s.keys.admit(&join, &world.log, Arrival::New).map(|_| ()));
    assert_eq!(result, Some(Err(Refusal::Replayed)));
}

/// A joiner the log of the channel does not hold gets no wrap.
#[test]
fn a_joiner_outside_the_channel_gets_no_wrap() {
    let mut world = World::new();
    person_join(&mut world, 0);
    let outsider = &world.people[MEMBERS];
    let (keys, _) = CallKeys::start(
        &outsider.key,
        Joining {
            call: CALL,
            channel: CHANNEL,
            member: outsider.id,
            sender: Seq::new(99),
            at: Instant::<Authored>::new(0),
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let join = keys.join().clone();
    let result = world.sessions.get_mut(&0).map(|(s, _)| {
        s.keys
            .admit(&join, &world.log, Arrival::New)
            .map(|m| m.wraps.len())
    });
    assert_eq!(result, Some(Err(Refusal::NotAMember)));
}
