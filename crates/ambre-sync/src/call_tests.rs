//! The key model of a call (`ambre-spec-call-media.md section 3`). A reference model runs
//! over sequences of joins, leaves, removals and delays. Each member opens exactly the
//! chunks it should.
//!
//! Each actor runs the real keys of a session. Its Sync worker holds the `CallKeys`, and a
//! call worker would hold the sealer and the receive keys. A test relay carries every join
//! and every wrap at once. An actor who leaves or is removed keeps what it holds. It also
//! keeps receiving every wrap addressed to its old index, as a curious member would. The model
//! knows none of the keys: it states who may open a chunk from who was in the call when.
//!
//! The same world runs a long call to check that no key and nonce pair seals two chunks
//! (`CALL-147`). It reads the pair from the cipher itself. ChaCha20 writes one keystream under
//! one key and one nonce, so two chunks that share a pair share a keystream.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use ambre_core::Role;
use ambre_core::id::{Call, Channel, Id};
use ambre_core::{
    Authored, ClearChunk, FixedBytes, Instant, KeyGeneration, KeyIdentifier, PageLoad, Payload,
    SealedChunk, SenderIndex, Seq, Track, UnixMs,
};
use ambre_crypto::call::{ChunkHeader, ChunkSealer, ReceiveKeys, SenderKey, SignedJoin};
use ambre_protocol::MembershipAct;
use proptest::prelude::*;

use super::{Arrival, CallKeys, JOIN_CLOCK_SKEW, Joining, Moves, Refusal, SWITCH_DELAY};
use crate::membership_log::tests::{DOC, Person, append};
use crate::membership_log::{Log, digest, no_self_join};
use ambre_crypto::call::KeyHandoff;

const CALL: Id<Call> = Id::<Call>::from_bytes([11; 16]);
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
    /// The generation of the last `Seal` handed to the session.
    top: Option<Seq<KeyGeneration>>,
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

impl Session {
    fn hand(&mut self, handoff: KeyHandoff) {
        match handoff {
            KeyHandoff::Seal {
                generation, key, ..
            } => {
                // The Sync worker moves the generation of its own key up at each `Seal`, so
                // the call worker never takes one twice (`CALL-147`).
                assert!(
                    self.top.is_none_or(|top| generation > top),
                    "a Seal at generation {generation:?} after {:?}",
                    self.top
                );
                self.top = Some(generation);
                self.held.push(key.clone());
                let kid = KeyIdentifier {
                    track: Track::Audio,
                    sender: self.keys.own(),
                    generation,
                };
                self.sealer = ChunkSealer::new(&key, kid).ok();
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

/// What a chunk a participant seals is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// Every reader opens exactly what the model allows, and the keystream is new.
    EveryReader,
    /// The keystream alone is new: a long call, where reading every chunk costs too much.
    KeystreamAlone,
}

/// The test world: the channel log, the sessions, the clock, and the reference model.
struct World {
    check: Check,
    /// The keystream of every chunk sealed so far: the sealed body XOR its clear body.
    keystreams: HashSet<Vec<u8>>,
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

/// The operations of a long call: members join and leave again and again, and two of them
/// may be removed. Most operations speak, so each key seals many chunks.
fn long_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..ACTORS).prop_map(Op::Join),
        3 => (0..ACTORS).prop_map(Op::Leave),
        1 => (0..2usize).prop_map(Op::Remove),
        3 => (0u64..4_000).prop_map(Op::Advance),
        12 => (0..ACTORS).prop_map(Op::Speak),
    ]
}

/// The keystream that sealed the body of an audio chunk: its sealed body XOR its clear body.
/// The seal pads `clear` to a big-endian length, the chunk, then zeros (`CALL-007`).
fn keystream_of(chunk: &Payload<SealedChunk>, clear: &Payload<ClearChunk>) -> Vec<u8> {
    const HEADER: usize = 17;
    const BODY: usize = 162;
    let clear = clear.as_bytes();
    let length = u16::try_from(clear.len()).unwrap_or_else(|e| panic!("{e}"));
    let mut body = length.to_be_bytes().to_vec();
    body.extend_from_slice(clear);
    body.resize(BODY, 0);
    let sealed = chunk
        .as_bytes()
        .get(HEADER..HEADER + BODY)
        .unwrap_or_else(|| panic!("a sealed chunk shorter than its body"));
    sealed.iter().zip(&body).map(|(s, c)| s ^ c).collect()
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
            check: Check::EveryReader,
            keystreams: HashSet::new(),
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
                let now = Instant::<UnixMs>::new(self.now);
                let handoffs = session.keys.receive(from_index, &wrap.wrapped, now);
                let handoffs = handoffs.unwrap_or_else(|e| panic!("a wrap does not open: {e:?}"));
                for handoff in handoffs {
                    session.hand(handoff);
                }
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
            CallKeys::start(&person.key, joining, &self.log).unwrap_or_else(|e| panic!("{e:?}"));
        let join: SignedJoin = keys.join().clone();
        let mut session = Session {
            keys,
            sealer: None,
            receive: ReceiveKeys::default(),
            held: Vec::new(),
            top: None,
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
        let keystream = keystream_of(&chunk, &clear);
        assert!(
            self.keystreams.insert(keystream),
            "actor {actor} sealed a chunk under a key and a nonce already used, at {}",
            self.now
        );
        if self.check == Check::KeystreamAlone {
            return;
        }
        let readers: Vec<usize> = self.sessions.keys().copied().collect();
        for reader in readers {
            let expected = self.may_open(reader, actor, self.now);
            // A participant in the call opens through its receive keys alone. Any other
            // reader tries every key it ever held.
            let opened = match self.sessions.get_mut(&reader) {
                Some((session, Standing::In)) => session
                    .receive
                    .open_audio(
                        Payload::new(chunk.as_bytes().to_vec()),
                        Instant::<PageLoad>::default(),
                    )
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
        if self.check == Check::KeystreamAlone {
            return;
        }
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// `CALL-147`: over a long call, where members leave and join again and the others switch
    /// their keys, no two chunks share a keystream. So no key and nonce pair sealed twice, and
    /// each `Seal` the Sync worker hands moves the generation of its session up.
    #[test]
    fn no_key_and_nonce_pair_repeats_over_a_long_call(
        ops in prop::collection::vec(long_op(), 400..800)
    ) {
        let mut world = World::new();
        world.check = Check::KeystreamAlone;
        for op in ops {
            world.apply(op);
        }
        prop_assert!(world.keystreams.len() > 50, "a call too short to tell");
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

/// A relay that replays to a later joiner the join of an earlier session gets no wrap
/// (`CALL-124`, audit P5). A join whose clock lags by the skew alone still gets one.
#[test]
fn a_join_older_than_its_own_is_refused() {
    let mut world = World::new();
    let old = join_of(&world, 1, CALL, CHANNEL, 60);
    world.advance(JOIN_CLOCK_SKEW.get() + 1);
    person_join(&mut world, 0);
    assert_eq!(admit_of(&mut world, 0, &old), Some(Err(Refusal::Stale)));
    let person = &world.people[1];
    let (keys, _) = CallKeys::start(
        &person.key,
        Joining {
            call: CALL,
            channel: CHANNEL,
            member: person.id,
            sender: Seq::new(61),
            at: Instant::<Authored>::new(world.now - JOIN_CLOCK_SKEW.get()),
        },
        &world.log,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let lagging = keys.join().clone();
    let wraps = world.sessions.get_mut(&0).map(|(s, _)| {
        s.keys
            .admit(&lagging, &world.log, Arrival::New)
            .map(|m| m.wraps.len())
    });
    assert_eq!(wraps, Some(Ok(1)));
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
        &world.log,
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

/// The join of member `actor` under `sender`, for `call` in `channel`.
fn join_of(
    world: &World,
    actor: usize,
    call: Id<Call>,
    channel: Id<Channel>,
    sender: u64,
) -> SignedJoin {
    let person = &world.people[actor];
    let (keys, _) = CallKeys::start(
        &person.key,
        Joining {
            call,
            channel,
            member: person.id,
            sender: Seq::new(sender),
            at: Instant::<Authored>::new(world.now),
        },
        &world.log,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    keys.join().clone()
}

fn admit_of(world: &mut World, actor: usize, join: &SignedJoin) -> Option<Result<(), Refusal>> {
    let log = &world.log;
    world
        .sessions
        .get_mut(&actor)
        .map(|(s, _)| s.keys.admit(join, log, Arrival::New).map(|_| ()))
}

/// A join for another call, or for the same call in another channel, gets no wrap.
#[test]
fn a_join_for_another_call_or_channel_is_refused() {
    let mut world = World::new();
    person_join(&mut world, 0);
    let other_call = Id::<Call>::from_bytes([21; 16]);
    let other_channel = Id::<Channel>::from_bytes([22; 16]);
    let to_call = join_of(&world, 1, other_call, CHANNEL, 50);
    assert_eq!(
        admit_of(&mut world, 0, &to_call),
        Some(Err(Refusal::OtherCall))
    );
    let to_channel = join_of(&world, 1, CALL, other_channel, 51);
    assert_eq!(
        admit_of(&mut world, 0, &to_channel),
        Some(Err(Refusal::OtherCall))
    );
}

/// A fresh exchange key under a sender index seen before is refused.
#[test]
fn a_fresh_key_under_an_index_seen_before_is_refused() {
    let mut world = World::new();
    person_join(&mut world, 0);
    let index = person_join(&mut world, 1);
    let again = join_of(&world, 2, CALL, CHANNEL, index.get());
    assert_eq!(
        admit_of(&mut world, 0, &again),
        Some(Err(Refusal::Replayed))
    );
}

/// The join `signer` signs for `member` under `sender`, carrying the exchange key `exchange`
/// that another join drew.
fn claiming(
    world: &World,
    signer: &Person,
    member: Id<ambre_core::id::User>,
    sender: u64,
    exchange: FixedBytes<ambre_core::ExchangePublicKey>,
) -> SignedJoin {
    let claim = ambre_crypto::call::JoinClaim {
        call: CALL,
        channel: CHANNEL,
        member,
        identity: FixedBytes::from_array(signer.key.verifying_key().to_bytes()),
        at: Instant::<Authored>::new(world.now),
        sender: Seq::new(sender),
        exchange,
    };
    SignedJoin::sign(&signer.key, claim).unwrap_or_else(|e| panic!("{e:?}"))
}

/// A join that claims the exchange key of a later joiner evicts nobody (`CALL-133`, audit
/// `N28`). A stranger's claim is refused and marks nothing. A member's claim under its own
/// identity is admitted, and the genuine joiner is still admitted after both.
#[test]
fn a_join_claiming_another_exchange_key_evicts_nobody() {
    let mut world = World::new();
    person_join(&mut world, 0);
    let genuine = join_of(&world, 1, CALL, CHANNEL, 70);
    let exchange = genuine
        .verified()
        .map(|claim| claim.exchange)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let stranger = Person::new(200);
    let burn = claiming(&world, &stranger, stranger.id, 68, exchange);
    assert_eq!(
        admit_of(&mut world, 0, &burn),
        Some(Err(Refusal::NotAMember))
    );
    let mallory = &world.people[2];
    let squat = claiming(&world, mallory, mallory.id, 69, exchange);
    assert_eq!(admit_of(&mut world, 0, &squat), Some(Ok(())));
    assert_eq!(admit_of(&mut world, 0, &genuine), Some(Ok(())));
    assert_eq!(
        admit_of(&mut world, 0, &genuine),
        Some(Err(Refusal::Replayed))
    );
}

/// The keys of the session `sender` of member `actor`, joined at the time of `world`.
fn session_of(world: &World, actor: usize, sender: u64) -> CallKeys {
    session_under(world, actor, sender, &world.log)
}

/// The keys of the session `sender` of member `actor`, joined under `log` at the time of
/// `world`.
fn session_under(world: &World, actor: usize, sender: u64, log: &Log) -> CallKeys {
    let person = &world.people[actor];
    let (keys, _) = CallKeys::start(
        &person.key,
        Joining {
            call: CALL,
            channel: CHANNEL,
            member: person.id,
            sender: Seq::new(sender),
            at: Instant::<Authored>::new(world.now),
        },
        log,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    keys
}

/// The moves of `keys` as it admits each of `others`, already in the call.
fn admit_each(keys: &mut CallKeys, others: &[&CallKeys], log: &Log) -> Vec<Moves> {
    others
        .iter()
        .map(|other| {
            keys.admit(other.join(), log, Arrival::Present)
                .unwrap_or_else(|e| panic!("{e:?}"))
        })
        .collect()
}

/// The log of `world` once its owner revoked `actor`.
fn revoked(world: &World, actor: usize) -> Log {
    let mut entries = world.entries.clone();
    append(
        &mut entries,
        &world.owner,
        &world.people[actor],
        MembershipAct::Revoke,
        Role::Editor,
    );
    walk(&entries)
}

/// `CALL-158`: Eve sits in the call from two sessions. The log removes her, and Alice heeds it.
/// Both sessions of Eve depart before Alice wraps a fresh key. The one fresh key goes to
/// Roger alone, never to the second session of Eve.
#[test]
fn a_removal_departs_every_session_of_the_member_before_a_fresh_key() {
    let world = World::new();
    let mut alice = session_of(&world, 0, 1);
    let roger = session_of(&world, 1, 2);
    let eve_laptop = session_of(&world, 2, 3);
    let eve_phone = session_of(&world, 2, 4);
    admit_each(&mut alice, &[&roger, &eve_laptop, &eve_phone], &world.log);

    let now = Instant::<UnixMs>::new(world.now);
    let moves = alice
        .heed(&revoked(&world, 2), now)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let to: Vec<_> = moves.wraps.iter().map(|wrap| wrap.to).collect();
    assert_eq!(to, [roger.own()], "a fresh key reached a session of Eve");
    let forgotten: BTreeSet<_> = moves
        .handoffs
        .iter()
        .filter_map(|handoff| match handoff {
            KeyHandoff::Forget { sender } => Some(*sender),
            _ => None,
        })
        .collect();
    assert_eq!(
        forgotten,
        BTreeSet::from([eve_laptop.own(), eve_phone.own()])
    );
}

/// `CALL-157`: Eve is removed, and a malicious relay withholds from Roger the wrap of the fresh
/// key of Alice. Roger hands the call worker the close of the older key of Alice anyway. Past
/// the switch delay, no chunk Eve seals under that key opens, and a replayed wrap of it is
/// refused. The fresh key of Alice still opens when its wrap arrives late.
#[test]
fn a_removal_closes_the_older_key_of_a_sender_whose_fresh_wrap_is_withheld() {
    let world = World::new();
    let mut alice = session_of(&world, 0, 1);
    let mut roger = session_of(&world, 1, 2);
    let mut eve = session_of(&world, 2, 3);
    let to_others = admit_each(&mut alice, &[&roger, &eve], &world.log);
    admit_each(&mut roger, &[&alice, &eve], &world.log);
    admit_each(&mut eve, &[&alice, &roger], &world.log);
    let start = Instant::<UnixMs>::new(world.now);
    let wrap_to = |moves: &[Moves], to: Seq<SenderIndex>| {
        moves
            .iter()
            .flat_map(|m| &m.wraps)
            .find(|wrap| wrap.to == to)
            .map(|wrap| wrap.wrapped.clone())
            .unwrap_or_else(|| panic!("no wrap to {to:?}"))
    };
    let old_to_roger = wrap_to(&to_others, roger.own());
    let old_to_eve = wrap_to(&to_others, eve.own());

    // The call worker of Roger, on its own monotonic clock.
    let mut worker = ReceiveKeys::default();
    let t0 = Instant::<PageLoad>::new(0.0);
    for handoff in roger
        .receive(alice.own(), &old_to_roger, start)
        .unwrap_or_else(|e| panic!("{e:?}"))
    {
        if let KeyHandoff::Open {
            sender,
            generation,
            key,
        } = handoff
        {
            worker.insert(sender, generation, key, t0);
        }
    }
    let eve_key = eve
        .receive(alice.own(), &old_to_eve, start)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .into_iter()
        .find_map(|handoff| match handoff {
            KeyHandoff::Open { key, .. } => Some(key),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Eve opened no key of Alice"));

    let log = revoked(&world, 2);
    let fresh = alice.heed(&log, start).unwrap_or_else(|e| panic!("{e:?}"));
    let closes: Vec<_> = roger
        .heed(&log, start)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .handoffs
        .into_iter()
        .filter(|handoff| matches!(handoff, KeyHandoff::Close { .. }))
        .collect();
    let [
        KeyHandoff::Close {
            sender,
            generation,
            within,
        },
    ] = closes.as_slice()
    else {
        panic!("the removal closed {closes:?}");
    };
    assert_eq!((*sender, *generation), (alice.own(), Seq::ZERO));
    worker.close(*sender, *generation, *within, t0);

    // The wrap of the fresh key never reaches Roger. Eve seals under the older key of Alice.
    let kid = KeyIdentifier {
        track: Track::Audio,
        sender: alice.own(),
        generation: Seq::ZERO,
    };
    let mut forger = ChunkSealer::new(&eve_key, kid).unwrap_or_else(|e| panic!("{e:?}"));
    let mut forged = |text: &[u8]| {
        forger
            .seal(&Payload::new(text.to_vec()))
            .unwrap_or_else(|e| panic!("{e:?}"))
    };
    let delay = SWITCH_DELAY.to_fractional();
    let just_before = t0.after(delay).before(ambre_core::TimeSpan::new(1.0));
    assert!(worker.open_audio(forged(b"in time"), just_before).is_ok());
    let due = t0.after(delay);
    assert!(worker.open_audio(forged(b"forged"), due).is_err());

    // A replay of the older wrap past the delay hands no key.
    let past = start.plus(SWITCH_DELAY);
    assert_eq!(
        roger
            .receive(alice.own(), &old_to_roger, past)
            .map(|h| h.len()),
        Err(Refusal::Retired)
    );
    // The wrap of the fresh key, delivered late, opens with no close.
    let fresh_to_roger = wrap_to(&[fresh], roger.own());
    let handoffs = roger
        .receive(alice.own(), &fresh_to_roger, past)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(handoffs.as_slice(), [KeyHandoff::Open { .. }]));
}

/// The wrap among `moves` to the session `to`.
fn wrap_among(moves: &[Moves], to: Seq<SenderIndex>) -> ambre_crypto::call::WrappedSenderKey {
    moves
        .iter()
        .flat_map(|m| &m.wraps)
        .find(|wrap| wrap.to == to)
        .map(|wrap| wrap.wrapped.clone())
        .unwrap_or_else(|| panic!("no wrap to {to:?}"))
}

/// `CALL-159`: Eve is removed, and Roger heeds it. Carol joins after the removal, and the relay
/// serves her the log before it. Her own log holds Eve, so Carol wraps her key to Eve. Roger
/// admits Carol under the current log. Her key names the place of the log before the removal.
/// Roger hands its close with it inside the switch delay, and refuses it past the delay.
///
/// Dave joins under the current log, and his key opens at Roger with no close.
#[test]
fn a_joiner_under_a_rolled_back_log_has_its_key_closed_at_the_removal() {
    let world = World::new();
    let mut alice = session_of(&world, 0, 1);
    let mut roger = session_of(&world, 1, 2);
    let eve = session_of(&world, 2, 3);
    admit_each(&mut alice, &[&roger, &eve], &world.log);
    admit_each(&mut roger, &[&alice, &eve], &world.log);
    let start = Instant::<UnixMs>::new(world.now);
    let log = revoked(&world, 2);
    alice.heed(&log, start).unwrap_or_else(|e| panic!("{e:?}"));
    roger.heed(&log, start).unwrap_or_else(|e| panic!("{e:?}"));

    // The relay serves Carol the log before the removal, and the roster with Eve in it.
    let mut carol = session_under(&world, 3, 4, &world.log);
    let carol_moves = admit_each(&mut carol, &[&alice, &roger, &eve], &world.log);
    wrap_among(&carol_moves, eve.own());
    roger
        .admit(carol.join(), &log, Arrival::New)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let carol_to_roger = wrap_among(&carol_moves, roger.own());
    let handoffs = roger
        .receive(carol.own(), &carol_to_roger, start)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        matches!(
            handoffs.as_slice(),
            [KeyHandoff::Open { .. }, KeyHandoff::Close { sender, within, .. }]
                if *sender == carol.own() && *within <= SWITCH_DELAY
        ),
        "the key of Carol opened with no close: {handoffs:?}"
    );
    let past = start.plus(SWITCH_DELAY);
    assert_eq!(
        roger
            .receive(carol.own(), &carol_to_roger, past)
            .map(|h| h.len()),
        Err(Refusal::Retired)
    );

    // Dave reads the current log: his key names a place that holds the removal.
    let mut dave = session_under(&world, 4, 5, &log);
    let dave_moves = admit_each(&mut dave, &[&alice, &roger], &log);
    roger
        .admit(dave.join(), &log, Arrival::New)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let handoffs = roger
        .receive(dave.own(), &wrap_among(&dave_moves, roger.own()), past)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(handoffs.as_slice(), [KeyHandoff::Open { .. }]));
}

/// `CALL-159`: Carol joined under the log the relay rolled back, and wrapped her key to Eve.
/// Once Carol reads the current log, she departs Eve and draws a fresh key that names its
/// place. Roger opens that fresh key past the switch delay of the removal, with no close, and
/// the fresh key reaches no session of Eve.
#[test]
fn a_rolled_back_joiner_opens_again_once_it_heeds_the_removal() {
    let world = World::new();
    let mut roger = session_of(&world, 1, 2);
    let eve = session_of(&world, 2, 3);
    admit_each(&mut roger, &[&eve], &world.log);
    let start = Instant::<UnixMs>::new(world.now);
    let log = revoked(&world, 2);
    roger.heed(&log, start).unwrap_or_else(|e| panic!("{e:?}"));

    let mut carol = session_under(&world, 3, 4, &world.log);
    admit_each(&mut carol, &[&roger, &eve], &world.log);
    roger
        .admit(carol.join(), &log, Arrival::New)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let fresh = carol.heed(&log, start).unwrap_or_else(|e| panic!("{e:?}"));
    let to: Vec<_> = fresh.wraps.iter().map(|wrap| wrap.to).collect();
    assert_eq!(to, [roger.own()], "the fresh key of Carol reached Eve");
    let past = start.plus(SWITCH_DELAY);
    let handoffs = roger
        .receive(carol.own(), &wrap_among(&[fresh], roger.own()), past)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(handoffs.as_slice(), [KeyHandoff::Open { .. }]));
}
