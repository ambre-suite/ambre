//! The convergence of a channel over generated interleavings (`MURMURE-582`).
//!
//! `proptest` generates operations of three members on one channel. They
//! send, edit, delete, react and unreact. They reply, which may create a
//! thread, and they close the active segment.
//!
//! A model of the relay stores each document's deltas in one log, in the
//! order it accepts them. Each member reads each log in that order, as the
//! relay serialises one document (`MURMURE-435`). The generator interleaves
//! the logs of different documents freely. A member subscribes to a thread
//! or a successor once a document it holds names it, and fetches its log
//! whole then. Once every member read every log, the views must be
//! identical.
//!
//! The finalization of a closed segment, amendments and folds are out of
//! scope: they need the lease of the relay, and their rows hold them.

use std::collections::HashMap;

use crate::message::{Deletion, MessageAt, MessageDraft, MessageView};
use crate::message::{MurmureEvent, ReplyEcho};
use ambre_bus::{Authored, Emoji, Instant};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes, Relay, Seq, SnapshotBytes};
use proptest::prelude::*;
use proptest::sample::Index;

use crate::logic::session::next_peer;
use crate::{ChannelState, ChannelType, LinkedOpening, Signer, Successor, ThreadTarget};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);
const PEERS: usize = 3;
const EMOJIS: [&str; 2] = ["\u{1F44D}", "\u{1F600}"];

/// One generated step. A `pick` selects among what the member sees at that
/// step, so every step names a target the member can act on.
#[derive(Debug, Clone)]
enum Op {
    Send {
        peer: usize,
        text: String,
    },
    Reply {
        peer: usize,
        pick: Index,
        text: String,
        echo: ReplyEcho,
    },
    Edit {
        peer: usize,
        pick: Index,
        text: String,
    },
    Delete {
        peer: usize,
        pick: Index,
    },
    React {
        peer: usize,
        pick: Index,
        emoji: usize,
    },
    Unreact {
        peer: usize,
        pick: Index,
        emoji: usize,
    },
    Close {
        peer: usize,
    },
    Deliver {
        peer: usize,
        pick: Index,
    },
}

/// One delta the relay accepted, with who signed it.
struct Entry {
    delta: Payload<DeltaBytes>,
    signer: Signer,
    by: usize,
}

/// The relay: one ordered log per document, the thread it linked to each
/// root, and the successor it registered for the one close.
#[derive(Default)]
struct RelayModel {
    logs: HashMap<Id<Document>, Vec<Entry>>,
    threads: HashMap<Id<Message>, Id<Document>>,
    successor: Option<(Id<Segment>, Id<Segment>)>,
}

/// A member: its channel, its user, and how far it read each log.
struct Member {
    state: ChannelState,
    user: Id<User>,
    read: HashMap<Id<Document>, usize>,
}

/// A message a member sees, with where it lies.
#[derive(Debug, Clone)]
struct Seen {
    at: MessageAt,
    view: MessageView,
}

struct World {
    relay: RelayModel,
    members: Vec<Member>,
}

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

fn fresh_entry() -> (ambre_core::PeerId, Payload<PeerEntryBytes>) {
    let peer = next_peer();
    let entry = Payload::<PeerEntryBytes>::new(peer.get().to_le_bytes().to_vec());
    (peer, entry)
}

/// A member's copy of `state`, opened from its live documents.
fn peer_of(state: &ChannelState) -> ChannelState {
    let segments: HashMap<Id<Segment>, Payload<SnapshotBytes>> = state
        .held_segments()
        .into_iter()
        .map(|segment| {
            let bytes = state.snapshot(segment.document_id()).expect("segment");
            (segment, bytes)
        })
        .collect();
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments)
        .expect("peer")
        .identify()
}

impl World {
    /// Three members of one channel whose active segment holds two
    /// messages of the first member.
    fn new() -> Self {
        let first = Id::<User>::mint();
        let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
            .expect("create")
            .identify();
        for text in ["a", "b"] {
            state.send(&draft(text), first, NOW).expect("send");
        }
        let mut members: Vec<Member> = (1..PEERS)
            .map(|_| Member {
                state: peer_of(&state),
                user: Id::<User>::mint(),
                read: HashMap::new(),
            })
            .collect();
        members.insert(
            0,
            Member {
                state,
                user: first,
                read: HashMap::new(),
            },
        );
        Self {
            relay: RelayModel::default(),
            members,
        }
    }

    /// The relay accepts `delta` of member `by` into the log of `document`.
    fn publish(&mut self, by: usize, document: Id<Document>, delta: Payload<DeltaBytes>) {
        let member = &self.members[by];
        let signer = Signer {
            nonce: crate::logic::delta::TEST_NONCE,
            peer: member.state.peer(document).expect("a session"),
            user: Some(member.user),
        };
        self.relay
            .logs
            .entry(document)
            .or_default()
            .push(Entry { delta, signer, by });
    }

    /// The documents whose log holds a delta member `peer` has not read and
    /// did not write, in a stable order.
    fn unread(&self, peer: usize) -> Vec<Id<Document>> {
        let member = &self.members[peer];
        let mut documents: Vec<Id<Document>> = self
            .relay
            .logs
            .iter()
            .filter(|(document, log)| {
                member.state.holds(**document)
                    && log
                        .iter()
                        .skip(member.read.get(*document).copied().unwrap_or(0))
                        .any(|entry| entry.by != peer)
            })
            .map(|(document, _)| *document)
            .collect();
        documents.sort_by_key(ToString::to_string);
        documents
    }

    /// Member `peer` reads the next foreign delta of `document`. Its own
    /// deltas are skipped: it wrote them already.
    fn deliver(&mut self, peer: usize, document: Id<Document>) {
        let log = self
            .relay
            .logs
            .get(&document)
            .map_or(&[][..], Vec::as_slice);
        let member = &mut self.members[peer];
        let mut at = member.read.get(&document).copied().unwrap_or(0);
        while let Some(entry) = log.get(at) {
            at += 1;
            if entry.by == peer {
                continue;
            }
            member
                .state
                .apply_signed(
                    document,
                    Seq::<Relay>::new(at as u64),
                    &entry.delta,
                    entry.signer,
                )
                .expect("a relayed delta applies");
            break;
        }
        member.read.insert(document, at);
        self.follow(peer);
    }

    /// Member `peer` fetches `document` whole, admits it and adopts its
    /// session, as the Sync worker does for a thread or a successor.
    fn fetch(&mut self, peer: usize, document: Id<Document>) {
        let log = self
            .relay
            .logs
            .get(&document)
            .map_or(&[][..], Vec::as_slice);
        let member = &mut self.members[peer];
        for (at, entry) in (1..).zip(log) {
            assert_ne!(entry.by, peer, "a member never fetches its own document");
            member
                .state
                .apply_signed(document, Seq::<Relay>::new(at), &entry.delta, entry.signer)
                .expect("a fetched delta waits");
        }
        member.read.insert(document, log.len());
        member.state.older_ready(document).expect("admitted");
        let (id, entry) = fresh_entry();
        member
            .state
            .adopt_session(document, id, entry)
            .expect("adopt");
    }

    /// Fetch every successor and thread that member `peer` learned of.
    fn follow(&mut self, peer: usize) {
        loop {
            let state = &mut self.members[peer].state;
            let mut asked: Vec<Id<Document>> = state.threads_to_open();
            asked.extend(
                state
                    .successor_to_open()
                    .map(|segment| segment.document_id()),
            );
            if asked.is_empty() {
                return;
            }
            for document in asked {
                self.fetch(peer, document);
            }
        }
    }

    /// Every message member `peer` sees: the held segments, then the held
    /// threads.
    fn seen(&self, peer: usize) -> Vec<Seen> {
        let state = &self.members[peer].state;
        let mut seen: Vec<Seen> = state
            .held_segments()
            .into_iter()
            .flat_map(|segment| {
                let messages = state.messages(segment).expect("held");
                messages.into_iter().map(move |view| Seen {
                    at: MessageAt::Segment(segment),
                    view,
                })
            })
            .collect();
        for thread in sorted(state.held_threads()) {
            if let Some(MurmureEvent::ThreadMessages { root, messages, .. }) =
                state.thread_view(thread)
            {
                seen.extend(messages.into_iter().map(|view| Seen {
                    at: MessageAt::Thread(root),
                    view,
                }));
            }
        }
        seen
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Send { peer, text } => {
                let member = &mut self.members[*peer];
                let sent = member
                    .state
                    .send(&draft(text), member.user, NOW)
                    .expect("send");
                self.publish(*peer, sent.document, sent.delta);
            }
            Op::Reply {
                peer,
                pick,
                text,
                echo,
            } => self.reply(*peer, pick, text, *echo),
            Op::Edit { peer, pick, text } => {
                let Some(target) = self.own_live(*peer, pick) else {
                    return;
                };
                let member = &mut self.members[*peer];
                let edited = member
                    .state
                    .edit(target.at, target.view.id, &draft(text), member.user)
                    .expect("edit");
                if let Some(sent) = edited {
                    self.publish(*peer, sent.document, sent.delta);
                }
            }
            Op::Delete { peer, pick } => {
                let Some(target) = self.own_live(*peer, pick) else {
                    return;
                };
                let member = &mut self.members[*peer];
                let sent = member
                    .state
                    .delete(target.at, target.view.id, member.user)
                    .expect("delete");
                self.publish(*peer, sent.document, sent.delta);
            }
            Op::React { peer, pick, emoji } | Op::Unreact { peer, pick, emoji } => {
                let live: Vec<Seen> = self
                    .seen(*peer)
                    .into_iter()
                    .filter(|seen| seen.view.deleted == Deletion::Live)
                    .collect();
                if live.is_empty() {
                    return;
                }
                let target = pick.get(&live).clone();
                let emoji = Emoji::from(EMOJIS[*emoji]);
                let member = &mut self.members[*peer];
                let sent = if matches!(op, Op::React { .. }) {
                    member
                        .state
                        .react(target.at, target.view.id, &emoji, member.user, NOW)
                } else {
                    member
                        .state
                        .unreact(target.at, target.view.id, &emoji, member.user)
                }
                .expect("react");
                if let Some(sent) = sent {
                    self.publish(*peer, sent.document, sent.delta);
                }
            }
            Op::Close { peer } => self.close(*peer),
            Op::Deliver { peer, pick } => {
                let unread = self.unread(*peer);
                if !unread.is_empty() {
                    self.deliver(*peer, *pick.get(&unread));
                }
            }
        }
    }

    /// A live message of member `peer`'s own, picked by `pick`.
    fn own_live(&self, peer: usize, pick: &Index) -> Option<Seen> {
        let user = self.members[peer].user;
        let own: Vec<Seen> = self
            .seen(peer)
            .into_iter()
            .filter(|seen| seen.view.author == user && seen.view.deleted == Deletion::Live)
            .collect();
        (!own.is_empty()).then(|| pick.get(&own).clone())
    }

    /// Member `peer` replies to a live root of a held segment. A root with
    /// no thread at the relay gets this member's; a root the relay linked
    /// already takes the winner, as `SRV-214` answers.
    fn reply(&mut self, peer: usize, pick: &Index, text: &str, echo: ReplyEcho) {
        let roots: Vec<Seen> = self
            .seen(peer)
            .into_iter()
            .filter(|seen| {
                matches!(seen.at, MessageAt::Segment(_))
                    && seen.view.deleted == Deletion::Live
                    && seen.view.also_in_thread.is_none()
            })
            .collect();
        if roots.is_empty() {
            return;
        }
        let root = pick.get(&roots).view.id;
        let target = self.members[peer]
            .state
            .reply_target(root, NOW)
            .expect("target");
        match target {
            ThreadTarget::Write(_) => {}
            ThreadTarget::Open(thread) => self.fetch(peer, thread),
            ThreadTarget::Create(own) => self.link(peer, root, own),
        }
        let member = &mut self.members[peer];
        let replied = member
            .state
            .reply(root, &draft(text), member.user, NOW, echo)
            .expect("reply");
        self.publish(peer, replied.reply.document, replied.reply.delta);
        if let Some(copy) = replied.copy {
            self.publish(peer, copy.document, copy.delta);
        }
    }

    /// The relay links a thread to `root`: `own` when it is the first, the
    /// thread it linked before when not.
    fn link(&mut self, peer: usize, root: Id<Message>, own: Id<Document>) {
        let segment = self.members[peer]
            .state
            .root_segment(root)
            .expect("a held root");
        let winner = *self.relay.threads.entry(root).or_insert(own);
        if winner == own {
            let (id, entry) = fresh_entry();
            let first = self.members[peer]
                .state
                .adopt_session(own, id, entry)
                .expect("adopt")
                .expect("the header travels");
            self.publish(peer, own, first);
        }
        let linked = self.members[peer]
            .state
            .thread_linked(segment, root, winner, None)
            .expect("link");
        if let Some(named) = linked.root {
            self.publish(peer, named.document, named.delta);
        }
        if winner != own && linked.open == LinkedOpening::Open {
            self.fetch(peer, winner);
        }
    }

    /// Member `peer` closes the active segment. The first close registers
    /// its successor; a later close of the same segment learns the winner,
    /// as `MURMURE-011` answers. One close per run: the next one waits on a
    /// finalization the model leaves out.
    fn close(&mut self, peer: usize) {
        let old = self.members[peer].state.active_id();
        let successor = match self.relay.successor {
            None => {
                let successor = Id::<Segment>::mint();
                self.relay.successor = Some((old, successor));
                Successor::Created
            }
            Some((closed, _)) if closed == old => Successor::Fetched,
            Some(_) => return,
        };
        let new = self.relay.successor.map(|(_, new)| new).expect("set");
        let closed = self.members[peer]
            .state
            .close_into(old, new, successor, NOW)
            .expect("close");
        if successor == Successor::Created {
            let (id, entry) = fresh_entry();
            let first = self.members[peer]
                .state
                .adopt_session(new.document_id(), id, entry)
                .expect("adopt")
                .expect("the successor's first update");
            self.publish(peer, new.document_id(), first);
        }
        let meta = self.members[peer].state.meta_id().document_id();
        self.publish(peer, meta, closed.meta);
        if let Some(fetch) = closed.fetch {
            self.fetch(peer, fetch.document_id());
        }
    }

    /// Every member reads every log to its end.
    fn drain(&mut self) {
        loop {
            let mut moved = false;
            for peer in 0..PEERS {
                for document in self.unread(peer) {
                    self.deliver(peer, document);
                    moved = true;
                }
            }
            if !moved {
                return;
            }
        }
    }
}

/// `items` in the order of their text: an identifier has no `Ord`, and a
/// member's held order is no part of what converges.
fn sorted<T: ToString>(mut items: Vec<T>) -> Vec<T> {
    items.sort_by_key(ToString::to_string);
    items
}

/// What a member reads of the channel: its live segments, each segment's
/// messages with their thread heads, and each thread's replies.
#[derive(Debug, PartialEq)]
struct View {
    active: Id<Segment>,
    closing: Option<Id<Segment>>,
    segments: Vec<(Id<Segment>, Vec<MessageView>)>,
    threads: Vec<Option<MurmureEvent>>,
}

fn view_of(state: &ChannelState) -> View {
    View {
        active: state.active_id(),
        closing: state.closing_id(),
        segments: sorted(state.held_segments())
            .into_iter()
            .map(|segment| (segment, state.messages(segment).expect("held")))
            .collect(),
        threads: sorted(state.held_threads())
            .into_iter()
            .map(|thread| state.thread_view(thread))
            .collect(),
    }
}

/// The thread head each root shows is an overlay of the held thread: it
/// must count what a full read of that thread counts.
fn assert_heads_match_threads(state: &ChannelState) {
    for segment in state.held_segments() {
        for root in state.messages(segment).expect("held") {
            let Some(head) = root.thread else {
                continue;
            };
            let Some(MurmureEvent::ThreadMessages { messages, .. }) =
                state.thread_view(head.document)
            else {
                panic!("a named thread is held after a full exchange");
            };
            let live: Vec<&MessageView> = messages
                .iter()
                .filter(|reply| reply.deleted == Deletion::Live)
                .collect();
            assert_eq!(
                head.replies.get() as usize,
                live.len(),
                "the head counts its replies"
            );
            assert_eq!(head.last_replier, live.last().map(|reply| reply.author));
        }
    }
}

fn text() -> impl Strategy<Value = String> {
    "[a-c]{1,3}"
}

fn op() -> impl Strategy<Value = Op> {
    let peer = 0..PEERS;
    prop_oneof![
        3 => (peer.clone(), text()).prop_map(|(peer, text)| Op::Send { peer, text }),
        2 => (peer.clone(), any::<Index>(), text(), prop_oneof![
            Just(ReplyEcho::ThreadOnly),
            Just(ReplyEcho::AlsoInChannel),
        ])
            .prop_map(|(peer, pick, text, echo)| Op::Reply { peer, pick, text, echo }),
        2 => (peer.clone(), any::<Index>(), text())
            .prop_map(|(peer, pick, text)| Op::Edit { peer, pick, text }),
        1 => (peer.clone(), any::<Index>()).prop_map(|(peer, pick)| Op::Delete { peer, pick }),
        2 => (peer.clone(), any::<Index>(), 0..EMOJIS.len())
            .prop_map(|(peer, pick, emoji)| Op::React { peer, pick, emoji }),
        1 => (peer.clone(), any::<Index>(), 0..EMOJIS.len())
            .prop_map(|(peer, pick, emoji)| Op::Unreact { peer, pick, emoji }),
        1 => peer.clone().prop_map(|peer| Op::Close { peer }),
        6 => (peer, any::<Index>()).prop_map(|(peer, pick)| Op::Deliver { peer, pick }),
    ]
}

/// Run `ops`, exchange every delta, and check each member reads one view.
fn converge(ops: &[Op]) {
    let mut world = World::new();
    for op in ops {
        world.apply(op);
    }
    world.drain();
    let first = view_of(&world.members[0].state);
    for member in &world.members {
        assert_eq!(view_of(&member.state), first, "every member reads one view");
        assert_heads_match_threads(&member.state);
    }
}

proptest! {
    // Each case replays up to 30 steps on three members; 128 cases keep
    // the host tier within a few seconds.
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// Three members acting on one channel, their deltas read in any order
    /// the relay allows, read one channel once every delta is exchanged.
    #[test]
    fn members_converge_over_any_relay_interleaving(
        ops in proptest::collection::vec(op(), 0..30)
    ) {
        converge(&ops);
    }
}
