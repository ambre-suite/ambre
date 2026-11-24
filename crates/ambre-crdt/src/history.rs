//! One undo history for every application: each local commit records its own
//! inverse (`ambre-spec-undo.md section 3`).
//!
//! Loro's undo manager answers an undo by a checkout. A checkout rebuilds the
//! state of every map the item wrote. The undo of one cell write on a sheet of
//! 10k formulas so costs about 250 million instructions. The manager also
//! restores a value over a peer's later write (`UNDO-007`).
//!
//! So every edit records its own inverse, and no undo manager runs. A
//! subscription keeps a mirror of every container. Each local commit reads
//! what it overwrites from that mirror, before the mirror takes the commit:
//!
//! * a map key keeps its value on each side; a nested container a commit
//!   deletes is kept by value, and created again by value;
//! * a list element keeps its place and its neighbour, named by its value;
//! * a text keeps the characters each edit put in or took out, placed by Loro
//!   cursors;
//! * a tree node keeps its parent and its sibling; a node a commit created is
//!   hidden on an undo, never deleted.

mod ident;
mod list;
mod rich;
mod shot;
mod text;
mod tree;

use ambre_core::hash::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::crdt::{CommitOrigin, FoundationRoot};
use ambre_core::index::{Axis, Len};
use ambre_core::{Availability, HistoryStep};
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{
    Container, ContainerID, ContainerTrait, ContainerType, EventTriggerKind, IdSpan,
    InternalString, LoroDoc, LoroMap, LoroValue, Subscription, TreeID,
};

use ambre_core::Flag as _;
use ambre_core::crdt::CrdtName as _;
use ident::{Commit, Idents};
use list::{ListWrite, plain, seq_list, take_list};
use rich::Rich;
use shot::{Held, Parent, Shot, attach};
use text::{Direction, Shifts, TextWrite};
use tree::{Forest, Kept, TreeWrite};

/// The index space of a list: one slot per element.
#[derive(Debug, PartialEq, Eq)]
enum Slot {}

impl Axis for Slot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Slot");
}

/// The index space of a text: one place per Unicode scalar.
#[derive(Debug, PartialEq, Eq)]
enum Chars {}

impl Axis for Chars {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Chars");
}

/// The index space of the history: one place per step.
#[derive(Debug, PartialEq, Eq)]
pub enum Steps {}

impl Axis for Steps {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Steps");
}

/// The marker an undo of a tree node's creation writes into the node's map,
/// so the node hides and keeps its identity. A redo takes the marker off.
/// The application names the key and computes the value when the undo runs.
#[derive(Clone)]
pub struct Hide {
    /// The key of the node map the marker sits under.
    pub key: InternalString,
    /// The value the marker takes, read from the document when the undo runs.
    pub value: fn(&LoroDoc) -> LoroValue,
    /// The container that holds the node's content outside its map, when the
    /// application keeps one. The commit that creates the node creates that
    /// content too. An undo hides it with the node, and the step keeps no
    /// write of that commit into it.
    pub body: fn(TreeID) -> Option<ContainerID>,
}

impl std::fmt::Debug for Hide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hide")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// The new identity of each container a replay created again by value, and
/// of each character a replay put back (`UNDO-024`).
#[derive(Debug, Default, Clone)]
struct Renames {
    to: HashMap<ContainerID, ContainerID>,
    chars: Idents,
}

impl Renames {
    /// The identity `id` stands under now. A container created again twice
    /// follows the chain to its last identity.
    fn resolve(&self, id: &ContainerID) -> ContainerID {
        let mut now = id;
        // Each replay adds one link, so the chain is as long as the
        // history and holds no loop. The bound guards the walk all the same.
        for _ in 0..=self.to.len() {
            match self.to.get(now) {
                Some(next) => now = next,
                None => break,
            }
        }
        now.clone()
    }

    /// Note that the container `old` stands under `new` from now on. The link
    /// starts where the chain of `old` ends, so a step that names any
    /// identity the container ever had still reaches `new`.
    fn note(&mut self, old: &ContainerID, new: ContainerID) {
        let now = self.resolve(old);
        if now != new {
            self.to.insert(now, new);
        }
    }
}

/// The plain content of one container, as the last event left it.
#[derive(Debug)]
enum Mirrored {
    Map(HashMap<InternalString, LoroValue>),
    List(Vec<LoroValue>),
    Text(Rich),
    Tree(Forest),
}

/// The plain content of every container of the document. A nested container
/// stands in its parent as its id.
#[derive(Debug, Default)]
struct Mirror {
    containers: HashMap<ContainerID, Mirrored>,
}

ambre_core::define_flag! {
    /// Whether a commit wrote only what a step can replay.
    enum Replayable {
        /// Every write is one a step replays.
        Whole,
        /// A write the history cannot replay. It deleted a tree node, put a
        /// container in a list an earlier commit created, or created a node
        /// with no marker to hide it. Or it changed a container of a kind the
        /// history does not replay.
        Lost,
    }
}

ambre_core::define_flag! {
    /// Whether a list insert puts a new element in or moves one.
    enum Motion {
        /// A new element.
        Fresh,
        /// An element the same diff takes out elsewhere.
        Move,
    }
}

/// One key a commit wrote, with its value on each side of the commit.
#[derive(Debug, Clone)]
struct MapWrite {
    map: ContainerID,
    key: InternalString,
    /// The value before the commit, `None` for an absent key.
    before: Option<Held>,
    /// The value after the commit, `None` for an absent key.
    after: Option<Held>,
}

/// What one commit wrote, read against the mirror.
#[derive(Debug, Default, Clone)]
struct Chunk {
    maps: Vec<MapWrite>,
    lists: Vec<ListWrite>,
    texts: Vec<TextWrite>,
    trees: Vec<TreeWrite>,
    /// The containers the commit created, while the commit is read. A write
    /// into one rides its creation by value, so no step keeps it.
    born: HashSet<ContainerID>,
}

impl Chunk {
    /// Drop each write into a container the commit created, then the set of
    /// those containers. The creation carries what they hold: an undo takes
    /// it out whole, or hides it. A Prose import so keeps one tree write per commit. It
    /// kept a map write and a text write per paragraph before
    /// (`Prose document, held after an import` row of `ambre-spec-budgets.md`).
    /// The pass runs once the whole commit is read: the order of the
    /// container changes in an event is no part of the rule.
    fn settle(&mut self) {
        let born = std::mem::take(&mut self.born);
        self.maps.retain(|write| !born.contains(&write.map));
        self.lists.retain(|write| !born.contains(&write.list));
        self.texts.retain(|write| !born.contains(&write.text));
        self.maps.shrink_to_fit();
        self.lists.shrink_to_fit();
        self.texts.shrink_to_fit();
        for tree in &mut self.trees {
            tree.shrink_to_fit();
        }
        self.trees.shrink_to_fit();
    }

    /// Whether the commit wrote nothing a step replays.
    fn is_empty(&self) -> bool {
        self.maps.is_empty()
            && self.lists.is_empty()
            && self.texts.is_empty()
            && self.trees.is_empty()
    }
}

/// One step of the history: the commits of one command, oldest first.
#[derive(Debug, Default)]
struct Step {
    chunks: Vec<Chunk>,
    touched: HashSet<ContainerID>,
}

/// Whether the commits that come take part in the step of the command under
/// way.
#[derive(Debug, Default)]
enum Group {
    /// Each commit is a step of its own: no command runs, or a peer's change
    /// met what the command wrote.
    #[default]
    Closed,
    /// The commits of the command under way join one step.
    Open { started: Started },
}

ambre_core::define_flag! {
    /// Whether the command under way has a step yet.
    enum Started {
        /// No commit of the command has landed.
        Not,
        /// The top step of the past is the command's.
        Yes,
    }
}

ambre_core::define_flag! {
    /// Whether the steps a hold keeps still come before the steps under way.
    enum Below {
        /// No commit since the hold ended the history.
        Kept,
        /// A commit the history cannot replay landed since the hold: a
        /// command that completes drops the kept steps with the rest.
        Lost,
    }
}

/// The steps a command that may fail midway sets aside while it runs. Its
/// failure gives them back once its writes are taken back (`UNDO-021`).
#[derive(Debug, Default)]
struct Hold {
    /// The past as it stood when the command started, oldest first.
    past: Vec<Step>,
    /// The future as it stood when the command started.
    future: Vec<Step>,
    below: Below,
}

/// The history of the document: the steps an undo walks back, and the steps
/// a redo walks forward.
#[derive(Debug, Default)]
struct History {
    past: Vec<Step>,
    future: Vec<Step>,
    group: Group,
    /// The steps before the command under way, when that command may fail
    /// midway.
    hold: Option<Hold>,
}

impl History {
    /// Take one recorded commit. A new step clears the redo side.
    fn record(&mut self, chunk: Chunk, touched: HashSet<ContainerID>) {
        if chunk.is_empty() {
            return;
        }
        let joins = matches!(
            self.group,
            Group::Open {
                started: Started::Yes
            }
        );
        match self.past.last_mut().filter(|_| joins) {
            Some(step) => {
                step.chunks.push(chunk);
                step.touched.extend(touched);
            }
            None => self.past.push(Step {
                chunks: vec![chunk],
                touched,
            }),
        }
        if let Group::Open { started } = &mut self.group {
            *started = Started::Yes;
        }
        self.future.clear();
    }

    /// Forget every step, for a new identity: the steps a hold keeps go too.
    fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.group = Group::Closed;
        if let Some(hold) = &mut self.hold {
            *hold = Hold::default();
        }
    }

    /// Forget every step, for a commit no step can replay. A hold keeps its
    /// steps: a failure of its command takes that commit back too.
    fn lose(&mut self) {
        self.past.clear();
        self.future.clear();
        self.group = Group::Closed;
        if let Some(hold) = &mut self.hold {
            hold.below = Below::Lost;
        }
    }

    /// Whether an undo has a step to take back, a held step included.
    fn can_undo(&self) -> Availability {
        Availability::from_bool(
            !self.past.is_empty()
                || self
                    .hold
                    .as_ref()
                    .is_some_and(|hold| hold.below == Below::Kept && !hold.past.is_empty()),
        )
    }

    /// Open the step of a command that may fail midway, and set the steps
    /// before it aside.
    fn hold(&mut self) {
        self.release();
        self.hold = Some(Hold {
            past: std::mem::take(&mut self.past),
            future: std::mem::take(&mut self.future),
            below: Below::Kept,
        });
        self.seal();
    }

    /// The held command completed: its steps go on top of the held ones.
    /// The held future survives only a command that recorded no step.
    fn release(&mut self) {
        let Some(hold) = self.hold.take() else {
            return;
        };
        if hold.below == Below::Kept {
            let mut past = hold.past;
            if self.past.is_empty() {
                self.future = hold.future;
            }
            past.append(&mut self.past);
            self.past = past;
        }
        self.close();
    }

    /// The held command failed, and its writes were taken back: the history
    /// stands as it did before the command.
    fn restore(&mut self) {
        let hold = self.hold.take().unwrap_or_default();
        self.past = hold.past;
        self.future = hold.future;
        self.group = Group::Closed;
    }

    /// End the step of the last command and open one for the next.
    fn seal(&mut self) {
        self.close();
        self.group = Group::Open {
            started: Started::Not,
        };
    }

    /// End the group under way. The step it built keeps no set of touched
    /// containers: only an open step meets a peer's change.
    fn close(&mut self) {
        if matches!(
            self.group,
            Group::Open {
                started: Started::Yes
            }
        ) && let Some(step) = self.past.last_mut()
        {
            step.touched = HashSet::default();
        }
        self.group = Group::Closed;
    }

    /// A peer's change that meets what the open step wrote ends it. Each later
    /// commit is then a step of its own, as the undo manager ends its group.
    fn meet_import(&mut self, touched: &HashSet<ContainerID>) {
        let met = matches!(
            self.group,
            Group::Open {
                started: Started::Yes
            }
        ) && self
            .past
            .last()
            .is_some_and(|step| !step.touched.is_disjoint(touched));
        if met {
            self.close();
        }
    }
}

/// What the subscription holds: the mirror and the history it records into.
#[derive(Debug, Default)]
struct Recorder {
    mirror: Mirror,
    history: History,
    renames: Renames,
    reach: Reach,
    /// A handle on the document, to read the cursors of a text edit. The
    /// history lets go of it before it unsubscribes (`Drop for UndoHistory`).
    reader: Option<LoroDoc>,
    /// The span of operations of the last local commit, which the hook
    /// before each commit writes. A deletion reads in it the identities of
    /// the characters it took out (`UNDO-024`).
    commit: Arc<Mutex<Option<IdSpan>>>,
}

/// Which containers a history records.
#[derive(Debug, Default)]
enum Reach {
    /// Root maps and root lists alone.
    #[default]
    Roots,
    /// Every container, a tree node's creation hidden under the marker.
    Nested { hide: Option<Hide> },
}

impl Reach {
    /// The marker a tree node's creation hides under, if any.
    fn hide(&self) -> Option<&Hide> {
        match self {
            Self::Roots => None,
            Self::Nested { hide } => hide.as_ref(),
        }
    }
}

/// The origins of a local commit no application records: the registry entry
/// (`AUTHOR-009`) and the replay of a step.
const UNRECORDED: [CommitOrigin; 2] = [CommitOrigin::Peers, CommitOrigin::Recorded];

/// The one undo history of a document: each local commit records its own
/// inverse, read against a mirror of every container (`ambre-spec-undo.md section 3`).
///
/// It records maps, lists and texts at any depth, and the creations and moves
/// of a tree. A commit it cannot replay clears it: a tree node deleted, a
/// container put into a list an earlier commit created, or a counter.
pub struct UndoHistory {
    recorder: Arc<Mutex<Recorder>>,
    /// Keeps the recorder subscribed for the life of the history.
    _watch: Subscription,
    /// Keeps the span of each local commit noted for the life of the history.
    _commits: Subscription,
}

impl Drop for UndoHistory {
    /// Let go of the recorder's handle on the document while the subscription
    /// still stands, and outside the lock. The last handle commits what is
    /// pending as it drops. Dropped inside the unsubscription, that commit
    /// would wait on the lock Loro holds to unsubscribe.
    fn drop(&mut self) {
        let reader = self.held().reader.take();
        drop(reader);
    }
}

impl std::fmt::Debug for UndoHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UndoHistory").finish_non_exhaustive()
    }
}

impl UndoHistory {
    /// Record every local commit of `doc` from now on, its mirror read from
    /// what `doc` holds now. A commit whose origin starts with one of
    /// `skipped`, or with an origin no application records, is no step.
    ///
    /// The history reaches root maps and root lists alone: a commit that
    /// writes a nested container, a container value or a text clears it.
    #[must_use]
    pub fn watch(doc: &LoroDoc, skipped: &[CommitOrigin]) -> Self {
        Self::start(doc, skipped, Reach::Roots)
    }

    /// [`watch`](Self::watch), reaching every container: nested maps and
    /// lists, texts and trees. An undo of a tree node's creation hides the node
    /// under `hide`; with no marker, a commit that creates a node clears the
    /// history.
    #[must_use]
    pub fn watch_nested(doc: &LoroDoc, skipped: &[CommitOrigin], hide: Option<Hide>) -> Self {
        Self::start(doc, skipped, Reach::Nested { hide })
    }

    fn start(doc: &LoroDoc, skipped: &[CommitOrigin], reach: Reach) -> Self {
        let commit: Arc<Mutex<Option<IdSpan>>> = Arc::default();
        // A lock of its own: a replay commits while the recorder is locked.
        let noted = Arc::clone(&commit);
        let commits = doc.subscribe_pre_commit(Box::new(move |payload| {
            let meta = &payload.change_meta;
            let span = i32::try_from(meta.len)
                .ok()
                .and_then(|len| meta.id.counter.checked_add(len))
                .map(|end| IdSpan::new(meta.id.peer, meta.id.counter, end));
            *noted.lock().unwrap_or_else(PoisonError::into_inner) = span;
            true
        }));
        let recorder = Arc::new(Mutex::new(Recorder {
            mirror: Mirror::read(doc),
            history: History::default(),
            renames: Renames::default(),
            reach,
            reader: Some(doc.clone()),
            commit,
        }));
        let into = Arc::clone(&recorder);
        let skipped: Vec<CommitOrigin> = UNRECORDED.iter().chain(skipped).copied().collect();
        let watch = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
            let mut held = into.lock().unwrap_or_else(PoisonError::into_inner);
            held.take(&event, &skipped);
        }));
        Self {
            recorder,
            _watch: watch,
            _commits: commits,
        }
    }

    /// The recorder, locked.
    fn held(&self) -> std::sync::MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether an undo has a step to take back.
    #[must_use]
    pub fn can_undo(&self) -> Availability {
        self.held().history.can_undo()
    }

    /// Whether a redo has a step to do again.
    #[must_use]
    pub fn can_redo(&self) -> Availability {
        Availability::from_bool(!self.held().history.future.is_empty())
    }

    /// Whether `step` has a step to take, back or forward.
    #[must_use]
    pub fn can(&self, step: HistoryStep) -> Availability {
        match step {
            HistoryStep::Undo => self.can_undo(),
            HistoryStep::Redo => self.can_redo(),
        }
    }

    /// How many steps an undo can take back, for the tests that count them.
    #[must_use]
    pub fn depth(&self) -> Len<Steps> {
        let held = self.held();
        let history = &held.history;
        let kept = history
            .hold
            .as_ref()
            .filter(|hold| hold.below == Below::Kept)
            .map_or(0, |hold| hold.past.len());
        Len::of(history.past.len() + kept)
    }

    /// End the step of the last command and open one for the command that
    /// starts: every commit it makes joins one step.
    pub fn seal(&self) {
        self.held().history.seal();
    }

    /// End the step under way: each later commit is a step of its own until
    /// the next [`seal`](Self::seal).
    pub fn close(&self) {
        self.held().history.close();
    }

    /// Forget every step, for a new identity: a step names this peer's work.
    /// The steps a [`hold`](Self::hold) keeps go too.
    pub fn clear(&self) {
        self.held().history.clear();
    }

    /// Open the step of a command that may fail midway, as
    /// [`seal`](Self::seal) does, and set the steps before it aside
    /// (`UNDO-021`). A commit the history cannot replay then forgets the
    /// steps of the command alone. The command ends with
    /// [`release`](Self::release) or [`restore`](Self::restore).
    pub fn hold(&self) {
        self.held().history.hold();
    }

    /// End the held command that completed, and close its step. Its steps
    /// go on top of the held ones. A commit the history could not replay
    /// since the hold forgets the held steps: no undo walks past it.
    pub fn release(&self) {
        self.held().history.release();
    }

    /// End the held command that failed, once the caller took back every
    /// write it made. The history then stands as it did at the hold, both
    /// sides, and keeps no step of the command. With no hold, it forgets
    /// every step.
    pub fn restore(&self) {
        self.held().history.restore();
    }

    /// Take back the step of the command under way, and forget it: neither
    /// side keeps it, so no redo writes it again. The step under way then
    /// ends. Nothing is committed: the caller commits under
    /// [`CommitOrigin::Recorded`], as for [`write`](Self::write).
    ///
    /// A command that failed midway calls it, so its partial writes leave the
    /// document as the command found it. [`Replayed::Nothing`] when the
    /// command holds no step: it recorded none yet, or a commit the history
    /// cannot replay cleared it. `doc` is then untouched, and no earlier step
    /// is taken back.
    pub fn withdraw(&self, doc: &LoroDoc) -> Replayed {
        let (taken, renames) = {
            let mut held = self.held();
            let started = matches!(
                held.history.group,
                Group::Open {
                    started: Started::Yes
                }
            );
            held.history.group = Group::Closed;
            let taken = if started {
                held.history.past.pop()
            } else {
                None
            };
            (taken, held.renames.clone())
        };
        let Some(mut taken) = taken else {
            return Replayed::Nothing;
        };
        // The cursors are read unlocked, as `write` reads them.
        taken.read(doc, &renames);
        let mut held = self.held();
        let held = &mut *held;
        taken.write(
            doc,
            HistoryStep::Undo,
            &held.mirror,
            &mut held.renames,
            held.reach.hide(),
        );
        Replayed::Step
    }

    /// Write one step of the history into `doc`, back or forward, and move
    /// it to the other side. Nothing is committed: the caller commits under
    /// [`CommitOrigin::Recorded`], so the replay records no step.
    /// [`Replayed::Nothing`] when that side is empty, and `doc` is untouched.
    pub fn write(&self, doc: &LoroDoc, step: HistoryStep) -> Replayed {
        let (taken, renames) = {
            let mut held = self.held();
            let taken = match step {
                HistoryStep::Undo => held.history.past.pop(),
                HistoryStep::Redo => held.history.future.pop(),
            };
            (taken, held.renames.clone())
        };
        let Some(mut taken) = taken else {
            return Replayed::Nothing;
        };
        // Every cursor is read before anything is written. The recorder stays
        // unlocked meanwhile. A cursor on a gone character makes Loro commit
        // what is pending, and that commit reaches the subscription.
        taken.read(doc, &renames);
        let mut held = self.held();
        let held = &mut *held;
        taken.write(
            doc,
            step,
            &held.mirror,
            &mut held.renames,
            held.reach.hide(),
        );
        match step {
            HistoryStep::Undo => held.history.future.push(taken),
            HistoryStep::Redo => held.history.past.push(taken),
        }
        Replayed::Step
    }
}

ambre_core::define_flag! {
    /// Whether [`UndoHistory::write`] had a step to write.
    pub enum Replayed {
        /// That side of the history was empty.
        Nothing,
        /// A step was written and moved to the other side.
        Step,
    }
}

impl Recorder {
    /// Take one event: the mirror follows it, and a local commit of the user
    /// becomes a step. A peer's change ends the open step where it meets it.
    fn take(&mut self, event: &DiffEvent<'_>, skipped: &[CommitOrigin]) {
        // A history that is dropping has no handle left, and records nothing.
        let Some(doc) = self.reader.take() else {
            return;
        };
        self.take_with(event, skipped, &doc);
        self.reader = Some(doc);
    }

    /// [`take`](Self::take), with the handle on the document.
    fn take_with(&mut self, event: &DiffEvent<'_>, skipped: &[CommitOrigin], doc: &LoroDoc) {
        let record = event.triggered_by == EventTriggerKind::Local
            && !skipped
                .iter()
                .any(|origin| event.origin.starts_with(origin.as_str()));
        // A span outlives no commit: an event that finds none reads no
        // identity, rather than the identities of an earlier commit.
        let span = self
            .commit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .filter(|_| record);
        let commit = Commit::new(doc, span);
        let mut chunk = Chunk::default();
        let mut replayable = Replayable::Whole;
        let mut touched = HashSet::default();
        for change in &event.events {
            let root = match change.target {
                ContainerID::Root { .. } => Some(change.target),
                ContainerID::Normal { .. } => change.path.first().map(|(id, _)| id),
            };
            touched.extend(root.cloned());
            if matches!(self.reach, Reach::Roots) && self.mirror.nests(change) {
                replayable = Replayable::Lost;
            }
            let journal = if record { Some(&mut chunk) } else { None };
            if self
                .mirror
                .take(change, journal, &commit, self.reach.hide())
                == Replayable::Lost
            {
                replayable = Replayable::Lost;
            }
        }
        match event.triggered_by {
            EventTriggerKind::Local if record => match replayable {
                Replayable::Whole => {
                    chunk.settle();
                    self.history.record(chunk, touched);
                }
                Replayable::Lost => self.history.lose(),
            },
            EventTriggerKind::Import => self.history.meet_import(&touched),
            _ => {}
        }
    }
}

impl Mirror {
    /// The mirror of what `doc` holds now: every container, nested ones
    /// included.
    fn read(doc: &LoroDoc) -> Self {
        let mut mirror = Self::default();
        let LoroValue::Map(roots) = doc.get_value() else {
            return mirror;
        };
        let mut queue: Vec<ContainerID> = roots
            .values()
            .filter_map(|value| match value {
                LoroValue::Container(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        while let Some(id) = queue.pop() {
            let Some(container) = doc.get_container(id.clone()) else {
                continue;
            };
            let children = |value: &LoroValue| match value {
                LoroValue::Container(child) => Some(child.clone()),
                _ => None,
            };
            let mirrored = match container {
                Container::Map(map) => match map.get_value() {
                    LoroValue::Map(entries) => {
                        queue.extend(entries.values().filter_map(children));
                        Mirrored::Map(
                            entries
                                .iter()
                                .map(|(k, v)| (InternalString::from(k.as_str()), v.clone()))
                                .collect(),
                        )
                    }
                    _ => continue,
                },
                Container::List(list) => {
                    let items = seq_values(list.get_value());
                    queue.extend(items.iter().filter_map(children));
                    Mirrored::List(items)
                }
                Container::MovableList(list) => {
                    let items = seq_values(list.get_value());
                    queue.extend(items.iter().filter_map(children));
                    Mirrored::List(items)
                }
                Container::Text(text) => Mirrored::Text(Rich::from_delta(&text.to_delta())),
                Container::Tree(tree) => {
                    let forest = Forest::read(&tree);
                    queue.extend(forest.nodes().map(|node| node.associated_meta_container()));
                    Mirrored::Tree(forest)
                }
                _ => continue,
            };
            mirror.containers.insert(id, mirrored);
        }
        mirror
    }

    /// Whether `change` reaches past the root maps and lists. Such a change
    /// lands in a nested container or a text, puts a container in, or writes
    /// over one. Read before the mirror takes the change.
    fn nests(&self, change: &ContainerDiff<'_>) -> bool {
        let is_container = |value: &Option<loro::ValueOrContainer>| {
            matches!(value, Some(loro::ValueOrContainer::Container(_)))
        };
        if !matches!(change.target, ContainerID::Root { .. }) {
            return true;
        }
        match &change.diff {
            Diff::Map(delta) => {
                let held = match self.containers.get(change.target) {
                    Some(Mirrored::Map(entries)) => Some(entries),
                    _ => None,
                };
                delta.updated.iter().any(|(key, value)| {
                    is_container(value)
                        || held.is_some_and(|entries| {
                            matches!(
                                entries.get(&InternalString::from(key.as_ref())),
                                Some(LoroValue::Container(_))
                            )
                        })
                })
            }
            Diff::List(items) => items.iter().any(|item| match item {
                loro::event::ListDiffItem::Insert { insert, .. } => insert
                    .iter()
                    .any(|v| matches!(v, loro::ValueOrContainer::Container(_))),
                _ => false,
            }),
            _ => true,
        }
    }

    /// Forget the container `id` and every container nested in it: its parent
    /// deleted it.
    fn forget(&mut self, id: &ContainerID) {
        let mut stack = vec![id.clone()];
        while let Some(id) = stack.pop() {
            let nested = |value: &LoroValue| match value {
                LoroValue::Container(child) => Some(child.clone()),
                _ => None,
            };
            match self.containers.remove(&id) {
                Some(Mirrored::Map(entries)) => stack.extend(entries.values().filter_map(nested)),
                Some(Mirrored::List(items)) => stack.extend(items.iter().filter_map(nested)),
                Some(Mirrored::Tree(forest)) => {
                    stack.extend(forest.nodes().map(|node| node.associated_meta_container()));
                }
                Some(Mirrored::Text(_)) | None => {}
            }
        }
    }

    /// Take one container change, and note it in `journal` when the commit is
    /// recorded. Returns whether the change is one a step replays.
    fn take(
        &mut self,
        change: &ContainerDiff<'_>,
        journal: Option<&mut Chunk>,
        commit: &Commit<'_>,
        hide: Option<&Hide>,
    ) -> Replayable {
        let target = change.target;
        // The registry entry is bookkeeping: no undo retracts it (`AUTHOR-009`).
        let registry = matches!(
            target,
            ContainerID::Root { name, .. } if name.as_str() == FoundationRoot::Peers.as_str()
        );
        let journal = journal.filter(|_| !registry);
        match (&change.diff, target.container_type()) {
            (Diff::Map(delta), ContainerType::Map) => {
                let Mirrored::Map(entries) = self
                    .containers
                    .entry(target.clone())
                    .or_insert_with(|| Mirrored::Map(HashMap::default()))
                else {
                    return Replayable::Lost;
                };
                let mut written = Vec::new();
                for (key, value) in &delta.updated {
                    let key = InternalString::from(key.as_ref());
                    // A container stands in the mirror as its id, so a later
                    // write over it is known for one.
                    let after = value.as_ref().map(plain);
                    let before = match &after {
                        Some(value) => entries.insert(key.clone(), value.clone()),
                        None => entries.remove(&key),
                    };
                    written.push((key, before, after));
                }
                if journal.is_some() {
                    // Key order, so a replay writes the same operations on
                    // every run whatever order the delta hands the keys out
                    // in. The tuples sort, smaller than the writes built
                    // from them.
                    written.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }
                let mut writes = Vec::new();
                let mut gone = Vec::new();
                for (key, before, after) in written {
                    // A container that leaves is kept by value, read from the
                    // mirror before the mirror forgets it.
                    let before = before.map(|value| match value {
                        LoroValue::Container(id) => {
                            let shot = Shot::from_mirror(self, &id);
                            gone.push(id);
                            Held::Container(shot)
                        }
                        value => Held::Value(value),
                    });
                    writes.push(MapWrite {
                        map: target.clone(),
                        key,
                        before,
                        after: after.as_ref().map(Held::of),
                    });
                }
                for id in &gone {
                    self.forget(id);
                }
                if let Some(chunk) = journal {
                    chunk
                        .born
                        .extend(writes.iter().filter_map(|w| match &w.after {
                            Some(Held::Container(shot)) => Some(shot.id.clone()),
                            _ => None,
                        }));
                    chunk.maps.extend(writes);
                }
                Replayable::Whole
            }
            (Diff::List(items), ContainerType::List | ContainerType::MovableList) => {
                let Mirrored::List(list) = self
                    .containers
                    .entry(target.clone())
                    .or_insert_with(|| Mirrored::List(Vec::new()))
                else {
                    return Replayable::Lost;
                };
                let write = take_list(list, items, target.clone());
                let gone: Vec<ContainerID> = write
                    .removed
                    .iter()
                    .filter_map(|p| match &p.value {
                        LoroValue::Container(id) => Some(id.clone()),
                        _ => None,
                    })
                    .collect();
                for id in &gone {
                    self.forget(id);
                }
                let nested = write
                    .added
                    .iter()
                    .any(|p| matches!(p.value, LoroValue::Container(_)));
                match journal {
                    // A list the same commit created leaves and comes back
                    // whole, by value, with its parent (`UNDO-017`). Each
                    // container it takes is born with it.
                    Some(chunk) if chunk.born.contains(target) => {
                        chunk
                            .born
                            .extend(write.added.iter().filter_map(|p| match &p.value {
                                LoroValue::Container(id) => Some(id.clone()),
                                _ => None,
                            }));
                        chunk.lists.push(write);
                        Replayable::Whole
                    }
                    Some(_) if nested || !gone.is_empty() => Replayable::Lost,
                    Some(chunk) => {
                        chunk.lists.push(write);
                        Replayable::Whole
                    }
                    None => Replayable::Whole,
                }
            }
            (Diff::Text(delta), ContainerType::Text) => {
                let Mirrored::Text(rich) = self
                    .containers
                    .entry(target.clone())
                    .or_insert_with(|| Mirrored::Text(Rich::default()))
                else {
                    return Replayable::Lost;
                };
                let edits = rich.take(delta);
                // A text the commit created rides its creation: its cursors
                // are not worth reading.
                if let Some(chunk) = journal.filter(|chunk| !chunk.born.contains(target)) {
                    let took = edits
                        .iter()
                        .any(|edit| matches!(edit, rich::Edit::Took { .. }));
                    let deleted = if took { commit.deleted(target) } else { None };
                    chunk
                        .texts
                        .push(TextWrite::record(commit.doc(), target, edits, deleted));
                }
                Replayable::Whole
            }
            (Diff::Tree(diff), ContainerType::Tree) => {
                let Mirrored::Tree(forest) = self
                    .containers
                    .entry(target.clone())
                    .or_insert_with(|| Mirrored::Tree(Forest::default()))
                else {
                    return Replayable::Lost;
                };
                let mut gone = Vec::new();
                let (mut write, kept) = forest.take(diff, &mut gone);
                for node in gone {
                    self.forget(&node.associated_meta_container());
                }
                let Some(chunk) = journal else {
                    return Replayable::Whole;
                };
                if kept == Kept::Lost || (!write.born.is_empty() && hide.is_none()) {
                    return Replayable::Lost;
                }
                write.tree = Some(target.clone());
                chunk.born.extend(write.born_metas());
                if let Some(hide) = hide {
                    chunk
                        .born
                        .extend(write.born.iter().filter_map(|(node, _)| (hide.body)(*node)));
                }
                chunk.trees.push(write);
                Replayable::Whole
            }
            _ => Replayable::Lost,
        }
    }
}

/// The live map `id` names, `None` for any other container or for a map a
/// peer deleted.
fn live_map(doc: &LoroDoc, id: &ContainerID) -> Option<LoroMap> {
    match doc.get_container(id.clone()) {
        Some(Container::Map(map)) if !map.is_deleted() => Some(map),
        _ => None,
    }
}

/// The values of a list read whole.
fn seq_values(value: LoroValue) -> Vec<LoroValue> {
    match value {
        LoroValue::List(items) => items.to_vec(),
        _ => Vec::new(),
    }
}

impl MapWrite {
    /// Replay the write into `doc`. A key a later write changed keeps that
    /// write: the step reverts this peer's work and never a peer's
    /// (`UNDO-007`). A map a peer deleted, or replaced, takes nothing: an undo
    /// never raises a container a peer removed.
    ///
    /// `roots` holds the handle of each root map the step already reached.
    /// A root map is never deleted or renamed. So one lookup serves each of
    /// its keys, and a block of lines writes a thousand keys into one map.
    fn write(
        &mut self,
        doc: &LoroDoc,
        undo: Direction,
        renames: &mut Renames,
        roots: &mut HashMap<ContainerID, Option<LoroMap>>,
    ) {
        let id = renames.resolve(&self.map);
        let nested;
        let map = if matches!(id, ContainerID::Root { .. }) {
            match roots.entry(id).or_insert_with_key(|id| live_map(doc, id)) {
                Some(map) => &*map,
                None => return,
            }
        } else {
            match live_map(doc, &id) {
                Some(map) => {
                    nested = map;
                    &nested
                }
                None => return,
            }
        };
        let Self {
            key, before, after, ..
        } = self;
        let (from, to) = match undo {
            Direction::Back => (after, before),
            Direction::Forward => (before, after),
        };
        let now = map.get(key).map(|value| match value {
            loro::ValueOrContainer::Value(value) => value,
            loro::ValueOrContainer::Container(container) => LoroValue::Container(container.id()),
        });
        let same = match from {
            None => now.is_none(),
            Some(held) => held.is(now.as_ref(), renames),
        };
        if !same {
            return;
        }
        // A container that leaves is captured as it stands, so the replay
        // that brings it back brings what it held.
        if let Some(Held::Container(shot)) = from {
            shot.body = Shot::from_doc(doc, &renames.resolve(&shot.id)).body;
        }
        match to {
            None => {
                let _ = map.delete(key);
            }
            Some(held) => attach(&Parent::Map(map, key.as_str()), held, renames),
        }
    }
}

impl Step {
    /// Read every cursor of the step's text edits, before anything is written.
    fn read(&mut self, doc: &LoroDoc, renames: &Renames) {
        for chunk in &mut self.chunks {
            for text in &mut chunk.texts {
                text.read(doc, renames);
            }
        }
    }

    /// Write the step into `doc`: `Undo` takes each commit back, the last
    /// first, and `Redo` makes each again, the first first. `mirror` holds
    /// the lists as they stand before the replay.
    fn write(
        &mut self,
        doc: &LoroDoc,
        step: HistoryStep,
        mirror: &Mirror,
        renames: &mut Renames,
        hide: Option<&Hide>,
    ) {
        let undo = match step {
            HistoryStep::Undo => Direction::Back,
            HistoryStep::Redo => Direction::Forward,
        };
        let chunks: Vec<&mut Chunk> = match undo {
            Direction::Back => self.chunks.iter_mut().rev().collect(),
            Direction::Forward => self.chunks.iter_mut().collect(),
        };
        // Each list is read once for the whole step, then followed as the
        // step writes it. Each text follows the places the step moved.
        let mut read: HashMap<ContainerID, Vec<LoroValue>> = HashMap::default();
        let mut shifts: HashMap<ContainerID, Shifts> = HashMap::default();
        let mut roots: HashMap<ContainerID, Option<LoroMap>> = HashMap::default();
        for chunk in chunks {
            let Chunk {
                maps,
                lists,
                texts,
                trees,
                born: _,
            } = chunk;
            let write_lists = |read: &mut HashMap<ContainerID, Vec<LoroValue>>,
                               renames: &Renames| {
                let writes: Vec<&ListWrite> = match undo {
                    Direction::Back => lists.iter().rev().collect(),
                    Direction::Forward => lists.iter().collect(),
                };
                for write in writes {
                    let id = renames.resolve(&write.list);
                    let Some(list) = seq_list(doc, &id) else {
                        continue;
                    };
                    let cur = read.entry(id.clone()).or_insert_with(|| {
                        match mirror.containers.get(&id) {
                            Some(Mirrored::List(items)) => items.clone(),
                            _ => list.values(),
                        }
                    });
                    write.write(&list, cur, step);
                }
            };
            let texts = texts.iter_mut();
            match undo {
                Direction::Back => {
                    for text in texts.rev() {
                        text.write(doc, undo, renames, &mut shifts);
                    }
                    write_lists(&mut read, renames);
                    for write in maps.iter_mut() {
                        write.write(doc, undo, renames, &mut roots);
                    }
                    for tree in trees.iter_mut().rev() {
                        tree.write(doc, undo, hide);
                    }
                }
                Direction::Forward => {
                    for tree in trees.iter_mut() {
                        tree.write(doc, undo, hide);
                    }
                    for write in maps.iter_mut() {
                        write.write(doc, undo, renames, &mut roots);
                    }
                    write_lists(&mut read, renames);
                    for text in texts {
                        text.write(doc, undo, renames, &mut shifts);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod format_tests;

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod ident_tests;

#[cfg(test)]
mod tests {
    //! The history on a bare document, with no application around it.

    use ambre_core::crdt::CommitOrigin;
    use ambre_core::{Availability, HistoryStep};
    use loro::{LoroDoc, LoroValue};

    use super::{Replayed, UndoHistory};

    /// Write `key` = `value` in the root map `m` and commit, as a user edit.
    fn write(doc: &LoroDoc, history: &UndoHistory, key: &str, value: i64) {
        history.seal();
        doc.get_map("m")
            .insert(key, value)
            .expect("a root map takes a value");
        doc.commit();
    }

    /// Replay one step and commit it under the replay origin.
    fn replay(doc: &LoroDoc, history: &UndoHistory, step: HistoryStep) -> Replayed {
        let replayed = history.write(doc, step);
        doc.set_next_commit_origin(CommitOrigin::Recorded.as_str());
        doc.commit();
        replayed
    }

    fn read(doc: &LoroDoc, key: &str) -> Option<LoroValue> {
        doc.get_map("m").get(key).and_then(|v| v.into_value().ok())
    }

    #[test]
    fn an_undo_writes_the_value_back_and_a_redo_writes_it_again() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        write(&doc, &history, "a", 2);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(1)));
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "a"), None);
        assert_eq!(history.can_undo(), Availability::Disabled);
        assert_eq!(replay(&doc, &history, HistoryStep::Redo), Replayed::Step);
        assert_eq!(replay(&doc, &history, HistoryStep::Redo), Replayed::Step);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(2)));
        assert_eq!(history.can_redo(), Availability::Disabled);
    }

    /// A command that fails midway takes its own step back over several
    /// commits, and no redo writes it again. The step before it stays.
    #[test]
    fn a_withdrawn_step_is_taken_back_and_forgotten() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        history.seal();
        for value in [2, 3] {
            doc.get_map("m").insert("a", value).expect("a write");
            doc.get_map("m").insert("b", value).expect("a write");
            doc.commit();
        }
        assert_eq!(history.withdraw(&doc), Replayed::Step);
        doc.set_next_commit_origin(CommitOrigin::Recorded.as_str());
        doc.commit();
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(1)));
        assert_eq!(read(&doc, "b"), None);
        assert_eq!(history.can_redo(), Availability::Disabled);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "a"), None);
    }

    /// A command that recorded no step withdraws nothing: the step of the
    /// command before it is never taken back.
    #[test]
    fn a_command_with_no_step_withdraws_nothing() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        history.seal();
        let before = doc.oplog_vv();
        assert_eq!(history.withdraw(&doc), Replayed::Nothing);
        doc.commit();
        assert_eq!(doc.oplog_vv(), before);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(1)));
        assert_eq!(history.can_undo(), Availability::Enabled);
    }

    /// A commit the history cannot replay: a counter.
    fn lose(doc: &LoroDoc) {
        doc.get_counter("lost")
            .increment(1.0)
            .expect("a counter counts");
        doc.commit();
    }

    /// Take back the write of `key` the held command made, under the replay
    /// origin, as the caller of a failed command does.
    fn take_back(doc: &LoroDoc, key: &str) {
        doc.get_map("m").delete(key).expect("a key leaves");
        doc.set_next_commit_origin(CommitOrigin::Recorded.as_str());
        doc.commit();
    }

    /// `UNDO-021`: a held command that fails gives the history back as
    /// it stood, both sides, even after a commit no step replays.
    #[test]
    fn a_failed_hold_gives_back_both_sides_after_a_lost_commit() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        write(&doc, &history, "a", 2);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        history.hold();
        doc.get_map("m").insert("b", 1).expect("a write");
        doc.commit();
        lose(&doc);
        assert_eq!(history.withdraw(&doc), Replayed::Nothing);
        take_back(&doc, "b");
        history.restore();
        assert_eq!(history.depth(), ambre_core::index::Len::of(1));
        assert_eq!(replay(&doc, &history, HistoryStep::Redo), Replayed::Step);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(2)));
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "a"), None);
        assert_eq!(read(&doc, "b"), None);
    }

    /// `UNDO-021`: a held command whose step is withdrawn leaves the
    /// steps before it, and the redo side, as they stood.
    #[test]
    fn a_withdrawn_hold_keeps_the_redo_side() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        write(&doc, &history, "a", 2);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        history.hold();
        doc.get_map("m").insert("b", 1).expect("a write");
        doc.commit();
        assert_eq!(history.withdraw(&doc), Replayed::Step);
        doc.set_next_commit_origin(CommitOrigin::Recorded.as_str());
        doc.commit();
        history.restore();
        assert_eq!(read(&doc, "b"), None);
        assert_eq!(replay(&doc, &history, HistoryStep::Redo), Replayed::Step);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(2)));
    }

    /// A held command that completes puts its one step on top of the held
    /// steps, and clears the redo side as any step does.
    #[test]
    fn a_released_hold_puts_its_step_on_top() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        write(&doc, &history, "a", 2);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        history.hold();
        assert_eq!(history.can_undo(), Availability::Enabled);
        for value in [1, 2] {
            doc.get_map("m").insert("b", value).expect("a write");
            doc.commit();
        }
        history.release();
        assert_eq!(history.can_redo(), Availability::Disabled);
        assert_eq!(history.depth(), ambre_core::index::Len::of(2));
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "b"), None);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(read(&doc, "a"), None);
    }

    /// A held command that recorded no step gives the redo side back.
    #[test]
    fn a_released_hold_with_no_step_keeps_the_redo_side() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        history.hold();
        history.release();
        assert_eq!(history.can_redo(), Availability::Enabled);
    }

    /// A held command that completes after a commit no step replays
    /// forgets the held steps: no undo walks past that commit.
    #[test]
    fn a_released_hold_after_a_lost_commit_forgets_the_held_steps() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        history.hold();
        lose(&doc);
        doc.get_map("m").insert("b", 1).expect("a write");
        doc.commit();
        history.release();
        assert_eq!(history.depth(), ambre_core::index::Len::of(1));
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(history.can_undo(), Availability::Disabled);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(1)));
    }

    /// A new identity forgets the held steps too: they name the old one.
    #[test]
    fn a_new_identity_forgets_the_held_steps() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        history.hold();
        history.clear();
        history.restore();
        assert_eq!(history.can_undo(), Availability::Disabled);
    }

    #[test]
    fn an_empty_side_writes_nothing() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        let before = doc.oplog_vv();
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Nothing);
        assert_eq!(doc.oplog_vv(), before);
    }

    /// `UNDO-007`: a key a peer wrote after the edit keeps the peer's value.
    #[test]
    fn an_undo_keeps_a_later_write_of_a_peer() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        let peer = LoroDoc::new();
        peer.import(
            &doc.export(loro::ExportMode::all_updates())
                .expect("an export"),
        )
        .expect("an import");
        peer.get_map("m").insert("a", 9).expect("a write");
        peer.commit();
        doc.import(
            &peer
                .export(loro::ExportMode::all_updates())
                .expect("an export"),
        )
        .expect("an import");
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(read(&doc, "a"), Some(LoroValue::I64(9)));
    }

    #[test]
    fn a_skipped_origin_records_no_step() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[CommitOrigin::Comment]);
        doc.get_map("m").insert("a", 1).expect("a write");
        doc.set_next_commit_origin(CommitOrigin::Comment.as_str());
        doc.commit();
        assert_eq!(history.can_undo(), Availability::Disabled);
    }

    /// Sync `from` into `to`: every update `to` lacks.
    fn sync(from: &LoroDoc, to: &LoroDoc) {
        to.import(
            &from
                .export(loro::ExportMode::all_updates())
                .expect("an export"),
        )
        .expect("an import");
    }

    /// A nested map deleted, then undone, comes back by value under its key,
    /// its text with it; a later step on it still reaches it.
    #[test]
    fn a_deleted_nested_map_comes_back_by_value() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        history.seal();
        let element = doc
            .get_map("elements")
            .insert_container("e", loro::LoroMap::new())
            .expect("a nested map");
        element.insert("x", 1).expect("a write");
        let body = element
            .insert_container("text", loro::LoroText::new())
            .expect("a nested text");
        body.insert(0, "hi").expect("a text write");
        doc.commit();
        history.seal();
        element.insert("x", 2).expect("a move");
        doc.commit();
        history.seal();
        doc.get_map("elements").delete("e").expect("a delete");
        doc.commit();
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(element_of(&doc), Some((2, "hi".to_owned())));
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(element_of(&doc), Some((1, "hi".to_owned())));
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(element_of(&doc), None);
        for _ in 0..3 {
            let _ = replay(&doc, &history, HistoryStep::Redo);
        }
        assert_eq!(element_of(&doc), None);
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(element_of(&doc), Some((2, "hi".to_owned())));
    }

    /// A map created again twice keeps one chain of identities: a step
    /// recorded against its second identity still reaches its third.
    #[test]
    fn a_map_created_again_twice_is_still_reached() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let elements = doc.get_map("elements");
        history.seal();
        let element = elements
            .insert_container("e", loro::LoroMap::new())
            .expect("a nested map");
        element.insert("x", 1).expect("a write");
        element
            .insert_container("text", loro::LoroText::new())
            .expect("a nested text")
            .insert(0, "hi")
            .expect("a text write");
        doc.commit();
        history.seal();
        elements.delete("e").expect("a delete");
        doc.commit();
        let _ = replay(&doc, &history, HistoryStep::Undo);
        let Some(loro::ValueOrContainer::Container(loro::Container::Map(again))) =
            elements.get("e")
        else {
            panic!("the undo puts the map back");
        };
        history.seal();
        again.insert("x", 2).expect("a move");
        doc.commit();
        for step in [HistoryStep::Undo, HistoryStep::Undo] {
            let _ = replay(&doc, &history, step);
        }
        assert_eq!(element_of(&doc), None);
        for step in [HistoryStep::Redo, HistoryStep::Redo] {
            let _ = replay(&doc, &history, step);
        }
        assert_eq!(element_of(&doc), Some((2, "hi".to_owned())));
    }

    /// The `x` and the text of the element `e`, `None` when it is absent.
    fn element_of(doc: &LoroDoc) -> Option<(i64, String)> {
        let loro::ValueOrContainer::Container(loro::Container::Map(element)) =
            doc.get_map("elements").get("e")?
        else {
            return None;
        };
        let x = element.get("x")?.into_value().ok()?.into_i64().ok()?;
        let loro::ValueOrContainer::Container(loro::Container::Text(text)) = element.get("text")?
        else {
            return None;
        };
        Some((x, text.to_string()))
    }

    /// `UNDO-017`: a map pushed into a list that the same commit created
    /// rides the creation of the list. The commit is one step. Its undo takes
    /// the whole entry out, and its redo writes it back by value.
    #[test]
    fn a_map_pushed_into_a_list_its_commit_created_is_one_step() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        write(&doc, &history, "before", 1);
        history.seal();
        let entry = doc
            .get_map("elements")
            .insert_container("e", loro::LoroMap::new())
            .expect("a nested map");
        entry.insert("x", 1).expect("a write");
        let list = entry
            .insert_container("replies", loro::LoroList::new())
            .expect("a nested list");
        let reply = list
            .push_container(loro::LoroMap::new())
            .expect("a map in the list");
        reply.insert("id", 7).expect("a key of the reply");
        reply
            .insert_container("text", loro::LoroText::new())
            .expect("a nested text")
            .insert(0, "hi")
            .expect("a text write");
        doc.commit();
        assert_eq!(history.depth().get(), 2, "the commit cleared the history");
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert!(doc.get_map("elements").get("e").is_none());
        assert_eq!(read(&doc, "before"), Some(LoroValue::I64(1)));
        assert_eq!(replay(&doc, &history, HistoryStep::Redo), Replayed::Step);
        assert_eq!(reply_text_of(&doc).as_deref(), Some("hi"));
        assert_eq!(reply_id_of(&doc), Some(7));
    }

    /// `UNDO-017`, the frame: a map pushed into a list an earlier commit
    /// created still ends the history. No step can replay it.
    #[test]
    fn a_map_pushed_into_an_older_list_clears_the_history() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let list = doc
            .get_map("elements")
            .insert_container("replies", loro::LoroList::new())
            .expect("a nested list");
        doc.commit();
        write(&doc, &history, "before", 1);
        history.seal();
        let _ = list
            .push_container(loro::LoroMap::new())
            .expect("a map in the list");
        doc.commit();
        assert_eq!(history.can_undo(), Availability::Disabled);
    }

    /// The first reply of the element `e`, `None` when absent.
    fn first_reply_of(doc: &LoroDoc) -> Option<loro::LoroMap> {
        let loro::ValueOrContainer::Container(loro::Container::Map(entry)) =
            doc.get_map("elements").get("e")?
        else {
            return None;
        };
        let loro::ValueOrContainer::Container(loro::Container::List(list)) =
            entry.get("replies")?
        else {
            return None;
        };
        match list.get(0)? {
            loro::ValueOrContainer::Container(loro::Container::Map(reply)) => Some(reply),
            _ => None,
        }
    }

    /// The `id` key of the first reply of the element `e`.
    fn reply_id_of(doc: &LoroDoc) -> Option<i64> {
        first_reply_of(doc)?
            .get("id")?
            .into_value()
            .ok()?
            .into_i64()
            .ok()
    }

    /// The text of the first reply of the element `e`, `None` when absent.
    fn reply_text_of(doc: &LoroDoc) -> Option<String> {
        let reply = first_reply_of(doc)?;
        let loro::ValueOrContainer::Container(loro::Container::Text(text)) = reply.get("text")?
        else {
            return None;
        };
        Some(text.to_string())
    }

    /// An undo never raises a nested map a peer deleted: the undo of a move on
    /// it writes nothing.
    #[test]
    fn an_undo_leaves_a_map_a_peer_deleted_deleted() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let element = doc
            .get_map("elements")
            .insert_container("e", loro::LoroMap::new())
            .expect("a nested map");
        element.insert("x", 1).expect("a write");
        doc.commit();
        history.seal();
        element.insert("x", 2).expect("a move");
        doc.commit();
        let peer = LoroDoc::new();
        sync(&doc, &peer);
        peer.get_map("elements").delete("e").expect("a peer delete");
        peer.commit();
        sync(&peer, &doc);
        let before = doc.oplog_vv();
        assert_eq!(replay(&doc, &history, HistoryStep::Undo), Replayed::Step);
        assert_eq!(doc.oplog_vv(), before, "the undo wrote into a deleted map");
        assert!(doc.get_map("elements").get("e").is_none());
    }

    /// An undo of typing takes out this peer's characters alone, wherever a
    /// peer's typing moved them; a redo puts them back.
    #[test]
    fn an_undo_of_typing_keeps_a_peer_s_characters() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let text = doc.get_text("t");
        text.insert(0, "ac").expect("a seed");
        doc.commit();
        history.seal();
        text.insert(1, "XY").expect("typing");
        doc.commit();
        let peer = LoroDoc::new();
        sync(&doc, &peer);
        peer.get_text("t")
            .insert(0, "P")
            .expect("a peer types before");
        peer.get_text("t")
            .insert(5, "Q")
            .expect("a peer types after");
        peer.commit();
        sync(&peer, &doc);
        assert_eq!(text.to_string(), "PaXYcQ");
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(text.to_string(), "PacQ");
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(text.to_string(), "PaXYcQ");
    }

    /// `UNDO-020`: a redo of typing puts the characters back with the marks
    /// they held when the undo took them out, a peer's mark included. The
    /// step keeps no copy of them, so the undo reads them from the text.
    #[test]
    fn a_redo_of_typing_keeps_a_mark_a_peer_set() {
        let style = Some(loro::StyleConfig {
            expand: loro::ExpandType::None,
        });
        let doc = LoroDoc::new();
        doc.config_default_text_style(style);
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let text = doc.get_text("t");
        text.insert(0, "ac").expect("a seed");
        doc.commit();
        history.seal();
        text.insert(1, "XY").expect("typing");
        doc.commit();
        let peer = LoroDoc::new();
        peer.config_default_text_style(style);
        sync(&doc, &peer);
        peer.get_text("t")
            .mark(1..3, "b", true)
            .expect("a peer marks");
        peer.commit();
        sync(&peer, &doc);
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(text.to_string(), "ac");
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(text.to_string(), "aXYc");
        let marked: Vec<String> = text
            .to_delta()
            .into_iter()
            .filter_map(|item| match item {
                loro::TextDelta::Insert { insert, attributes }
                    if attributes
                        .as_ref()
                        .is_some_and(|marks| marks.get("b") == Some(&LoroValue::Bool(true))) =>
                {
                    Some(insert)
                }
                _ => None,
            })
            .collect();
        assert_eq!(marked, vec!["XY".to_owned()]);
    }

    /// `UNDO-018`: an undo of typing leaves the characters alone when a peer
    /// replaced one of them since. The two ends still stand, and the count
    /// still holds.
    #[test]
    fn an_undo_of_typing_leaves_what_a_peer_replaced_inside() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let text = doc.get_text("t");
        text.insert(0, "ac").expect("a seed");
        doc.commit();
        history.seal();
        text.insert(1, "XYZ").expect("typing");
        doc.commit();
        let peer = LoroDoc::new();
        sync(&doc, &peer);
        let theirs = peer.get_text("t");
        theirs.delete(2, 1).expect("a peer takes Y out");
        theirs.insert(2, "W").expect("a peer types W");
        peer.commit();
        sync(&peer, &doc);
        assert_eq!(text.to_string(), "aXWZc");
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(text.to_string(), "aXWZc");
    }

    /// Two keys typed at one place, undone twice and redone twice, come back
    /// in their order. The second key's neighbour went with the first undo
    /// and came back under a new identity, so its cursor names a dead
    /// character.
    #[test]
    fn two_keys_at_one_place_redo_in_their_order() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let text = doc.get_text("t");
        for key in ["h", "a"] {
            history.seal();
            text.insert(0, key).expect("a key");
            doc.commit();
        }
        for step in [HistoryStep::Undo, HistoryStep::Undo] {
            let _ = replay(&doc, &history, step);
        }
        assert_eq!(text.to_string(), "");
        for step in [HistoryStep::Redo, HistoryStep::Redo] {
            let _ = replay(&doc, &history, step);
        }
        assert_eq!(text.to_string(), "ah");
    }

    /// An undo of a deletion puts the characters back with their marks.
    #[test]
    fn an_undone_deletion_puts_the_marks_back() {
        let doc = LoroDoc::new();
        doc.config_default_text_style(Some(loro::StyleConfig {
            expand: loro::ExpandType::After,
        }));
        let history = UndoHistory::watch_nested(&doc, &[], None);
        let text = doc.get_text("t");
        text.insert(0, "abcd").expect("a seed");
        text.mark(1..3, "b", true).expect("a mark");
        doc.commit();
        history.seal();
        text.delete(1, 2).expect("a delete");
        doc.commit();
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(text.to_string(), "abcd");
        assert_eq!(
            text.slice_delta(0, 4, loro::cursor::PosType::Unicode)
                .expect("a delta")
                .len(),
            3,
            "the restored run is marked apart from its neighbours"
        );
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(text.to_string(), "ad");
    }

    /// An undo of a tree node's creation hides the node; its redo shows the
    /// same node again.
    #[test]
    fn an_undone_node_creation_hides_the_same_node() {
        let doc = LoroDoc::new();
        let tree = doc.get_tree("blocks");
        tree.enable_fractional_index(0);
        let history = UndoHistory::watch_nested(
            &doc,
            &[],
            Some(super::Hide {
                key: loro::InternalString::from("gone"),
                value: |_| loro::LoroValue::Bool(true),
                body: |_| None,
            }),
        );
        history.seal();
        let node = tree.create(None).expect("a node");
        tree.get_meta(node)
            .expect("a meta")
            .insert("t", 1)
            .expect("a field");
        doc.commit();
        let _ = replay(&doc, &history, HistoryStep::Undo);
        let meta = tree.get_meta(node).expect("a meta");
        assert!(meta.get("gone").is_some());
        assert!(meta.get("t").is_some(), "a hidden node keeps its fields");
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert!(tree.get_meta(node).expect("a meta").get("gone").is_none());
        assert_eq!(tree.roots(), vec![node]);
    }

    /// The writes of the step on top of the past, by kind: maps, lists,
    /// texts and trees.
    fn kept(history: &UndoHistory) -> [usize; 4] {
        let held = history.held();
        let chunks = held
            .history
            .past
            .last()
            .map_or(&[][..], |step| &step.chunks);
        chunks.iter().fold([0; 4], |[m, l, t, r], chunk| {
            [
                m + chunk.maps.len(),
                l + chunk.lists.len(),
                t + chunk.texts.len(),
                r + chunk.trees.len(),
            ]
        })
    }

    /// `UNDO-022`: a map and a text created in one commit with their content
    /// leave one write in the step, the key that holds the map. The undo
    /// takes both out, and the redo writes them back with their content.
    #[test]
    fn a_step_keeps_no_write_into_a_container_its_commit_created() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        history.seal();
        let element = doc
            .get_map("elements")
            .insert_container("e", loro::LoroMap::new())
            .expect("a nested map");
        element.insert("x", 1).expect("a write");
        let body = element
            .insert_container("text", loro::LoroText::new())
            .expect("a nested text");
        body.insert(0, "hi").expect("a text write");
        doc.commit();
        assert_eq!(kept(&history), [1, 0, 0, 0]);
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(element_of(&doc), None);
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(element_of(&doc), Some((1, "hi".to_owned())));
    }

    /// The body text of node `node`, a root container outside its map.
    fn body_of(node: loro::TreeID) -> Option<loro::ContainerID> {
        Some(loro::ContainerID::new_root(
            &format!("body:{node}"),
            loro::ContainerType::Text,
        ))
    }

    /// `UNDO-023`: a node created with its body text in one commit leaves the
    /// tree write alone in the step. The undo hides the node with its body,
    /// and the redo shows both as they stood. A later edit of the body is a
    /// step of its own, which an undo takes out.
    #[test]
    fn an_undone_node_creation_hides_its_body_with_it() {
        let doc = LoroDoc::new();
        let tree = doc.get_tree("blocks");
        tree.enable_fractional_index(0);
        let history = UndoHistory::watch_nested(
            &doc,
            &[],
            Some(super::Hide {
                key: loro::InternalString::from("gone"),
                value: |_| loro::LoroValue::Bool(true),
                body: body_of,
            }),
        );
        history.seal();
        let node = tree.create(None).expect("a node");
        let body = doc.get_text(format!("body:{node}").as_str());
        body.insert(0, "hello").expect("a text write");
        doc.commit();
        assert_eq!(kept(&history), [0, 0, 0, 1]);
        history.seal();
        body.insert(5, "!").expect("a later edit");
        doc.commit();
        assert_eq!(kept(&history), [0, 0, 1, 0]);
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(body.to_string(), "hello");
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert!(tree.get_meta(node).expect("a meta").get("gone").is_some());
        assert_eq!(body.to_string(), "hello", "a hidden node keeps its body");
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert!(tree.get_meta(node).expect("a meta").get("gone").is_none());
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(body.to_string(), "hello!");
    }

    /// A closed step keeps no set of the containers it touched: only the
    /// open step meets a peer's change.
    #[test]
    fn a_closed_step_forgets_what_it_touched() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        let touched = |history: &UndoHistory| {
            history
                .held()
                .history
                .past
                .last()
                .map_or(0, |step| step.touched.len())
        };
        assert_eq!(touched(&history), 1);
        history.close();
        assert_eq!(touched(&history), 0);
    }

    /// A history of the roots alone ends at a write into a nested container.
    #[test]
    fn a_write_into_a_nested_container_ends_a_history_of_the_roots() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        write(&doc, &history, "a", 1);
        history.seal();
        let _ = doc
            .get_map("m")
            .insert_container("n", loro::LoroMap::new())
            .expect("a nested map");
        doc.commit();
        assert_eq!(history.can_undo(), Availability::Disabled);
    }

    /// With no marker named, a node's creation ends the history.
    #[test]
    fn a_node_creation_with_no_marker_ends_the_history() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch_nested(&doc, &[], None);
        write(&doc, &history, "a", 1);
        history.seal();
        let _ = doc.get_tree("blocks").create(None).expect("a node");
        doc.commit();
        assert_eq!(history.can_undo(), Availability::Disabled);
    }

    #[test]
    fn a_list_insert_undoes_and_redoes_in_place() {
        let doc = LoroDoc::new();
        let history = UndoHistory::watch(&doc, &[]);
        let list = doc.get_movable_list("l");
        for v in ["x", "z"] {
            list.push(v).expect("a push");
        }
        doc.commit();
        history.seal();
        list.insert(1, "y").expect("an insert");
        doc.commit();
        let _ = replay(&doc, &history, HistoryStep::Undo);
        assert_eq!(strings(&list), ["x", "z"]);
        let _ = replay(&doc, &history, HistoryStep::Redo);
        assert_eq!(strings(&list), ["x", "y", "z"]);
    }

    fn strings(list: &loro::LoroMovableList) -> Vec<String> {
        match list.get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| v.as_string().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }
}
