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
    Container, ContainerID, ContainerTrait, ContainerType, EventTriggerKind, InternalString,
    LoroDoc, LoroValue, Subscription,
};

use list::{ListWrite, plain, seq_list, take_list};
use rich::Rich;
use shot::{Held, Parent, Shot, attach};
use text::{Direction, Shifts, TextWrite};
use tree::{Forest, Kept, TreeWrite};

/// The index space of a list: one slot per element.
#[derive(Debug, PartialEq, Eq)]
enum Slot {}

impl Axis for Slot {
    const NAME: &'static str = "Slot";
}

/// The index space of a text: one place per Unicode scalar.
#[derive(Debug, PartialEq, Eq)]
enum Chars {}

impl Axis for Chars {
    const NAME: &'static str = "Chars";
}

/// The index space of the history: one place per step.
#[derive(Debug, PartialEq, Eq)]
pub enum Steps {}

impl Axis for Steps {
    const NAME: &'static str = "Steps";
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
}

impl std::fmt::Debug for Hide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hide")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// The new identity of each container a replay created again by value.
#[derive(Debug, Default, Clone)]
struct Renames {
    to: HashMap<ContainerID, ContainerID>,
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
        /// container in a list or created a node with no marker to hide it. Or
        /// it changed a container of a kind the history does not replay.
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
    /// The containers the commit created. A write into one rides its
    /// creation by value, so a replay skips it.
    born: HashSet<ContainerID>,
}

impl Chunk {
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

/// The history of the document: the steps an undo walks back, and the steps
/// a redo walks forward.
#[derive(Debug, Default)]
struct History {
    past: Vec<Step>,
    future: Vec<Step>,
    group: Group,
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

    /// Forget every step, for a commit no step can replay or a new identity.
    fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.group = Group::Closed;
    }

    /// End the step of the last command and open one for the next.
    fn seal(&mut self) {
        self.group = Group::Open {
            started: Started::Not,
        };
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
            self.group = Group::Closed;
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
/// container put into a list, or a counter.
pub struct UndoHistory {
    recorder: Arc<Mutex<Recorder>>,
    /// Keeps the recorder subscribed for the life of the history.
    _watch: Subscription,
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
        let recorder = Arc::new(Mutex::new(Recorder {
            mirror: Mirror::read(doc),
            history: History::default(),
            renames: Renames::default(),
            reach,
            reader: Some(doc.clone()),
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
        }
    }

    /// The recorder, locked.
    fn held(&self) -> std::sync::MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether an undo has a step to take back.
    #[must_use]
    pub fn can_undo(&self) -> Availability {
        Availability::from_bool(!self.held().history.past.is_empty())
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
        Len::of(self.held().history.past.len())
    }

    /// End the step of the last command and open one for the command that
    /// starts: every commit it makes joins one step.
    pub fn seal(&self) {
        self.held().history.seal();
    }

    /// End the step under way: each later commit is a step of its own until
    /// the next [`seal`](Self::seal).
    pub fn close(&self) {
        self.held().history.group = Group::Closed;
    }

    /// Forget every step, for a new identity: a step names this peer's work.
    pub fn clear(&self) {
        self.held().history.clear();
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
            if self.mirror.take(change, journal, doc, self.reach.hide()) == Replayable::Lost {
                replayable = Replayable::Lost;
            }
        }
        match event.triggered_by {
            EventTriggerKind::Local if record => match replayable {
                Replayable::Whole => self.history.record(chunk, touched),
                Replayable::Lost => self.history.clear(),
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
        doc: &LoroDoc,
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
                    // Key order, so a replay writes the same operations on
                    // every run whatever order the delta hands the keys out in.
                    writes.sort_unstable_by(|a, b| a.key.cmp(&b.key));
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
                if let Some(chunk) = journal {
                    chunk.texts.push(TextWrite::record(doc, target, edits));
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
                chunk.trees.push(write);
                Replayable::Whole
            }
            _ => Replayable::Lost,
        }
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
    fn write(&mut self, doc: &LoroDoc, undo: Direction, renames: &mut Renames) {
        let id = renames.resolve(&self.map);
        let map = match doc.get_container(id) {
            Some(Container::Map(map)) if !map.is_deleted() => map,
            _ => return,
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
            Some(held) => attach(&Parent::Map(&map, key.as_str()), held, renames),
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
        for chunk in chunks {
            let Chunk {
                maps,
                lists,
                texts,
                trees,
                born,
            } = chunk;
            let write_lists = |read: &mut HashMap<ContainerID, Vec<LoroValue>>,
                               renames: &Renames| {
                let writes: Vec<&ListWrite> = match undo {
                    Direction::Back => lists.iter().rev().collect(),
                    Direction::Forward => lists.iter().collect(),
                };
                for write in writes.into_iter().filter(|w| !born.contains(&w.list)) {
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
            let texts = texts.iter_mut().filter(|t| !born.contains(&t.text));
            match undo {
                Direction::Back => {
                    for text in texts.rev() {
                        text.write(doc, undo, renames, &mut shifts);
                    }
                    write_lists(&mut read, renames);
                    for write in maps.iter_mut().filter(|w| !born.contains(&w.map)) {
                        write.write(doc, undo, renames);
                    }
                    for tree in trees.iter_mut().rev() {
                        tree.write(doc, undo, hide);
                    }
                }
                Direction::Forward => {
                    for tree in trees.iter_mut() {
                        tree.write(doc, undo, hide);
                    }
                    for write in maps.iter_mut().filter(|w| !born.contains(&w.map)) {
                        write.write(doc, undo, renames);
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
