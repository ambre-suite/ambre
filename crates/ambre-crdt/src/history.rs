//! One undo history for every application: each local commit records its own
//! inverse (`ambre-spec-undo.md section 3`).
//!
//! Loro's undo manager answers an undo by a checkout. A checkout rebuilds the
//! state of every map the item wrote. The undo of one cell write on a sheet of
//! 10k formulas so costs about 250 million instructions. The manager also
//! restores a value over a peer's later write (`UNDO-007`).
//!
//! So every edit records its own inverse, and no undo manager runs. A
//! subscription keeps a mirror of every root container. Each local commit
//! reads the value each key held and the place each list element held from
//! that mirror, before the mirror takes the commit. An undo writes those values
//! back and puts each element back after its old neighbour. An element is
//! named by its value, so a step never depends on the identity of an element
//! in the list.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::crdt::{CommitOrigin, FoundationRoot};
use ambre_core::index::{Axis, Len, Pos};
use ambre_core::{Availability, HistoryStep};
use loro::event::{ContainerDiff, Diff, DiffEvent, ListDiffItem};
use loro::{
    ContainerID, ContainerTrait, ContainerType, EventTriggerKind, InternalString, LoroDoc,
    LoroList, LoroMap, LoroMovableList, LoroValue, Subscription, ValueOrContainer,
};

/// The index space of a root list: one slot per element.
#[derive(Debug, PartialEq, Eq)]
enum Slot {}

impl Axis for Slot {
    const NAME: &'static str = "Slot";
}

/// The index space of the history: one place per step.
#[derive(Debug, PartialEq, Eq)]
pub enum Steps {}

impl Axis for Steps {
    const NAME: &'static str = "Steps";
}

/// A root list of either kind, the one container a step moves elements in.
enum RootList {
    Plain(LoroList),
    Movable(LoroMovableList),
}

/// The root list `id` names, `None` for any other container.
fn root_list(doc: &LoroDoc, id: &ContainerID) -> Option<RootList> {
    match id {
        ContainerID::Root {
            name,
            container_type: ContainerType::List,
        } => Some(RootList::Plain(doc.get_list(name.as_str()))),
        ContainerID::Root {
            name,
            container_type: ContainerType::MovableList,
        } => Some(RootList::Movable(doc.get_movable_list(name.as_str()))),
        _ => None,
    }
}

impl RootList {
    /// The values of the list, in order.
    fn values(&self) -> Vec<LoroValue> {
        let value = match self {
            Self::Plain(list) => list.get_value(),
            Self::Movable(list) => list.get_value(),
        };
        match value {
            LoroValue::List(items) => items.to_vec(),
            _ => Vec::new(),
        }
    }

    /// Insert `value` at `at`.
    fn put(&self, at: Pos<Slot>, value: &LoroValue) {
        let _ = match self {
            Self::Plain(list) => list.insert(at.to_usize(), value.clone()),
            Self::Movable(list) => list.insert(at.to_usize(), value.clone()),
        };
    }

    /// Delete the `len` elements from `at` on.
    fn take(&self, at: Pos<Slot>, len: Len<Slot>) {
        let _ = match self {
            Self::Plain(list) => list.delete(at.to_usize(), len.to_usize()),
            Self::Movable(list) => list.delete(at.to_usize(), len.to_usize()),
        };
    }

    /// Move the element at `from` so it stands at `to`. A plain list has no
    /// move, so it takes the element out and puts its value back.
    fn shift(&self, from: Pos<Slot>, to: Pos<Slot>, value: &LoroValue) {
        match self {
            Self::Plain(list) => {
                let _ = list.delete(from.to_usize(), 1);
                let _ = list.insert(to.to_usize(), value.clone());
            }
            Self::Movable(list) => {
                let _ = list.mov(from.to_usize(), to.to_usize());
            }
        }
    }
}

/// The plain content of one root container, as the last event left it.
#[derive(Debug)]
enum Mirrored {
    Map(HashMap<InternalString, LoroValue>),
    List(Vec<LoroValue>),
}

/// The plain content of every root container of the document.
#[derive(Debug, Default)]
struct Mirror {
    roots: HashMap<InternalString, Mirrored>,
}

ambre_core::define_flag! {
    /// Whether a commit wrote only what a step can replay.
    enum Replayable {
        /// Every write is a plain value in a root map or list.
        Whole,
        /// A write landed in a nested container, held a container value, or
        /// changed a container of a kind the history does not replay.
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
    map: InternalString,
    key: InternalString,
    /// The value before the commit, `None` for an absent key.
    before: Option<LoroValue>,
    /// The value after the commit, `None` for an absent key.
    after: Option<LoroValue>,
}

/// One element a commit put into a list or took out of it. It keeps its place
/// and the element just before it, on that side of the commit.
#[derive(Debug, Clone)]
struct Placed {
    value: LoroValue,
    at: Pos<Slot>,
    pred: Option<LoroValue>,
}

/// One element a commit moved inside a list, with its place and its
/// neighbour on each side of the commit.
#[derive(Debug, Clone)]
struct Moved {
    value: LoroValue,
    from: Placed,
    to: Placed,
}

/// What one commit did to one list.
#[derive(Debug, Clone)]
struct ListWrite {
    list: ContainerID,
    /// The elements the commit took out, in their order before it.
    removed: Vec<Placed>,
    /// The elements the commit put in, in their order after it.
    added: Vec<Placed>,
    /// The elements the commit moved.
    moved: Vec<Moved>,
}

/// What one commit wrote, read against the mirror.
#[derive(Debug, Default, Clone)]
struct Chunk {
    maps: Vec<MapWrite>,
    lists: Vec<ListWrite>,
}

impl Chunk {
    /// Whether the commit wrote nothing a step replays.
    fn is_empty(&self) -> bool {
        self.maps.is_empty() && self.lists.is_empty()
    }
}

/// One step of the history: the commits of one command, oldest first.
#[derive(Debug, Default)]
struct Step {
    chunks: Vec<Chunk>,
    touched: HashSet<InternalString>,
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
    fn record(&mut self, chunk: Chunk, touched: HashSet<InternalString>) {
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
    fn meet_import(&mut self, touched: &HashSet<InternalString>) {
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
}

/// The origins of a local commit no application records: the registry entry
/// (`AUTHOR-009`) and the replay of a step.
const UNRECORDED: [CommitOrigin; 2] = [CommitOrigin::Peers, CommitOrigin::Recorded];

/// The one undo history of a document: each local commit records its own
/// inverse, read against a mirror of every root container (`ambre-spec-undo.md section 3`).
///
/// It records root maps and root lists. A commit that writes a nested
/// container, or a container value, clears it: no step can replay it.
pub struct UndoHistory {
    recorder: Arc<Mutex<Recorder>>,
    /// Keeps the recorder subscribed for the life of the history.
    _watch: Subscription,
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
    #[must_use]
    pub fn watch(doc: &LoroDoc, skipped: &[CommitOrigin]) -> Self {
        let recorder = Arc::new(Mutex::new(Recorder {
            mirror: Mirror::read(doc),
            history: History::default(),
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

    /// Forget every step, for a new identity: a step names this peer's work.
    pub fn clear(&self) {
        self.held().history.clear();
    }

    /// Write one step of the history into `doc`, back or forward, and move
    /// it to the other side. Nothing is committed: the caller commits under
    /// [`CommitOrigin::Recorded`], so the replay records no step.
    /// [`Replayed::Nothing`] when that side is empty, and `doc` is untouched.
    pub fn write(&self, doc: &LoroDoc, step: HistoryStep) -> Replayed {
        let mut held = self.held();
        let held = &mut *held;
        let taken = match step {
            HistoryStep::Undo => held.history.past.pop(),
            HistoryStep::Redo => held.history.future.pop(),
        };
        let Some(taken) = taken else {
            return Replayed::Nothing;
        };
        let mirror = &held.mirror;
        let lists = |name: &InternalString| match mirror.roots.get(name) {
            Some(Mirrored::List(items)) => items.clone(),
            _ => Vec::new(),
        };
        taken.write(doc, step, &lists);
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
        let record = event.triggered_by == EventTriggerKind::Local
            && !skipped
                .iter()
                .any(|origin| event.origin.starts_with(origin.as_str()));
        let mut chunk = Chunk::default();
        let mut replayable = Replayable::Whole;
        let mut touched = HashSet::new();
        for change in &event.events {
            if let ContainerID::Root { name, .. } = change.target {
                touched.insert(name.clone());
            }
            let journal = if record { Some(&mut chunk) } else { None };
            if self.mirror.take(change, journal) == Replayable::Lost {
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
    /// The mirror of what `doc` holds now: every root map and list.
    fn read(doc: &LoroDoc) -> Self {
        let LoroValue::Map(roots) = doc.get_value() else {
            return Self::default();
        };
        let roots = roots
            .iter()
            .filter_map(|(name, value)| {
                let mirrored = match value {
                    LoroValue::Container(ContainerID::Root {
                        container_type: ContainerType::Map,
                        name,
                    }) => match doc.get_map(name.as_str()).get_value() {
                        LoroValue::Map(map) => Mirrored::Map(
                            map.iter()
                                .map(|(k, v)| (InternalString::from(k.as_str()), v.clone()))
                                .collect(),
                        ),
                        _ => return None,
                    },
                    LoroValue::Container(id @ ContainerID::Root { .. }) => {
                        match root_list(doc, id).map(|list| list.values()) {
                            Some(items) => Mirrored::List(items),
                            None => return None,
                        }
                    }
                    _ => return None,
                };
                Some((InternalString::from(name.as_str()), mirrored))
            })
            .collect();
        Self { roots }
    }

    /// Take one container change, and note it in `journal` when the commit is
    /// recorded. Returns whether the change is one a step replays.
    fn take(&mut self, change: &ContainerDiff<'_>, journal: Option<&mut Chunk>) -> Replayable {
        let ContainerID::Root {
            name,
            container_type,
        } = change.target
        else {
            return Replayable::Lost;
        };
        // The registry entry is bookkeeping: no undo retracts it (`AUTHOR-009`).
        let journal = journal.filter(|_| name.as_str() != FoundationRoot::Peers.as_str());
        match (&change.diff, container_type) {
            (Diff::Map(delta), ContainerType::Map) => {
                let Mirrored::Map(map) = self
                    .roots
                    .entry(name.clone())
                    .or_insert_with(|| Mirrored::Map(HashMap::new()))
                else {
                    return Replayable::Lost;
                };
                let mut replayable = Replayable::Whole;
                let mut writes = Vec::new();
                for (key, value) in &delta.updated {
                    let key = InternalString::from(key.as_ref());
                    // A container stands in the mirror as its id, so a later
                    // write over it is known for one.
                    let after = value.as_ref().map(plain);
                    let before = match &after {
                        Some(value) => map.insert(key.clone(), value.clone()),
                        None => map.remove(&key),
                    };
                    if [&before, &after]
                        .iter()
                        .any(|v| matches!(v, Some(LoroValue::Container(_))))
                    {
                        replayable = Replayable::Lost;
                    }
                    writes.push(MapWrite {
                        map: name.clone(),
                        key,
                        before,
                        after,
                    });
                }
                if let Some(chunk) = journal {
                    // Key order, so a replay writes the same operations on
                    // every run whatever order the delta hands the keys out in.
                    writes.sort_unstable_by(|a, b| a.key.cmp(&b.key));
                    chunk.maps.extend(writes);
                }
                replayable
            }
            (Diff::List(items), ContainerType::List | ContainerType::MovableList) => {
                let Mirrored::List(list) = self
                    .roots
                    .entry(name.clone())
                    .or_insert_with(|| Mirrored::List(Vec::new()))
                else {
                    return Replayable::Lost;
                };
                let write = take_list(list, items, change.target.clone());
                if write
                    .added
                    .iter()
                    .any(|p| matches!(p.value, LoroValue::Container(_)))
                {
                    return Replayable::Lost;
                }
                if let Some(chunk) = journal {
                    chunk.lists.push(write);
                }
                Replayable::Whole
            }
            _ => Replayable::Lost,
        }
    }
}

/// Apply the list diff `items` to the mirror `list`, and read what it did.
/// An element taken out keeps its place and neighbour before. An element put
/// in keeps its place and neighbour after, and a move keeps both.
fn take_list(list: &mut Vec<LoroValue>, items: &[ListDiffItem], id: ContainerID) -> ListWrite {
    // First pass: the places, before and after, of each element the diff
    // removes or inserts. The neighbour before is read from the mirror while
    // it still holds the list before the diff.
    let mut removed = Vec::new();
    let mut inserted: Vec<(Pos<Slot>, LoroValue, Motion)> = Vec::new();
    let mut before = Pos::<Slot>::ZERO;
    let mut after = Pos::<Slot>::ZERO;
    for item in items {
        match item {
            ListDiffItem::Retain { retain } => {
                before = Pos::from_usize(before.to_usize() + retain);
                after = Pos::from_usize(after.to_usize() + retain);
            }
            ListDiffItem::Delete { delete } => {
                for at in before.to_usize()..before.to_usize() + delete {
                    if let Some(value) = list.get(at) {
                        removed.push(Placed {
                            value: value.clone(),
                            at: Pos::from_usize(at),
                            pred: at.checked_sub(1).and_then(|b| list.get(b)).cloned(),
                        });
                    }
                }
                before = Pos::from_usize(before.to_usize() + delete);
            }
            ListDiffItem::Insert { insert, is_move } => {
                for value in insert {
                    let value = plain(value);
                    let motion = if *is_move {
                        Motion::Move
                    } else {
                        Motion::Fresh
                    };
                    inserted.push((after, value, motion));
                    after = Pos::from_usize(after.to_usize() + 1);
                }
            }
        }
    }
    // Second pass: the mirror takes the diff.
    let mut at = 0usize;
    for item in items {
        match item {
            ListDiffItem::Retain { retain } => at += retain,
            ListDiffItem::Delete { delete } => {
                let end = (at + delete).min(list.len());
                list.drain(at.min(end)..end);
            }
            ListDiffItem::Insert { insert, .. } => {
                for value in insert {
                    let value = plain(value);
                    list.insert(at.min(list.len()), value);
                    at += 1;
                }
            }
        }
    }
    // The neighbour after each inserted element, read from the list after
    // the diff. A move is an insert of a value the diff also removes.
    let mut added = Vec::new();
    let mut moved = Vec::new();
    for (at, value, motion) in inserted {
        let pred = at
            .to_usize()
            .checked_sub(1)
            .and_then(|b| list.get(b))
            .cloned();
        let to = Placed {
            value: value.clone(),
            at,
            pred,
        };
        let from = match motion {
            Motion::Move => removed
                .iter()
                .position(|r: &Placed| r.value == value)
                .map(|i| removed.remove(i)),
            Motion::Fresh => None,
        };
        match from {
            Some(from) => moved.push(Moved { value, from, to }),
            None => added.push(to),
        }
    }
    ListWrite {
        list: id,
        removed,
        added,
        moved,
    }
}

/// The value an event hands out, a container standing as its id.
fn plain(value: &ValueOrContainer) -> LoroValue {
    match value {
        ValueOrContainer::Value(value) => value.clone(),
        ValueOrContainer::Container(container) => LoroValue::Container(container.id()),
    }
}

/// The place of `value` in `cur`: at `hint` when it stands there, else the
/// first place that holds it.
fn find(cur: &[LoroValue], value: &LoroValue, hint: Pos<Slot>) -> Option<Pos<Slot>> {
    if cur.get(hint.to_usize()) == Some(value) {
        return Some(hint);
    }
    cur.iter().position(|v| v == value).map(Pos::from_usize)
}

/// The place an element goes so it stands right after `pred`. That is `hint`
/// when `pred` stands just before it, else the place after the first one that
/// holds `pred`. When `pred` is gone, the element goes back to `hint`.
fn place_after(cur: &[LoroValue], pred: Option<&LoroValue>, hint: Pos<Slot>) -> Pos<Slot> {
    let Some(pred) = pred else {
        return Pos::ZERO;
    };
    let fallback = || Pos::from_usize(hint.to_usize().min(cur.len()));
    match hint.to_usize().checked_sub(1) {
        Some(before) if cur.get(before) == Some(pred) => hint,
        _ => cur
            .iter()
            .position(|v| v == pred)
            .map_or_else(fallback, |at| Pos::from_usize(at + 1)),
    }
}

impl ListWrite {
    /// Replay the list write into `list`, whose values are `cur`: `Undo`
    /// takes it back, `Redo` makes it again. An element named by its value is
    /// found wherever it stands. One a peer took out meanwhile is left out.
    fn write(&self, list: &RootList, cur: &mut Vec<LoroValue>, step: HistoryStep) {
        // The side the step leaves: the elements to take out, and the
        // elements to put back with their place and neighbour there.
        let (out, back): (&[Placed], Vec<(&LoroValue, &Placed)>) = match step {
            HistoryStep::Undo => (
                &self.added,
                self.removed
                    .iter()
                    .map(|p| (&p.value, p))
                    .chain(self.moved.iter().map(|m| (&m.value, &m.from)))
                    .collect(),
            ),
            HistoryStep::Redo => (
                &self.removed,
                self.added
                    .iter()
                    .map(|p| (&p.value, p))
                    .chain(self.moved.iter().map(|m| (&m.value, &m.to)))
                    .collect(),
            ),
        };
        // Every element to take out is found before any goes, so each place
        // the write noted still holds. Each run of adjacent places then goes
        // in one delete, the last run first.
        let mut gone: Vec<Pos<Slot>> = out
            .iter()
            .filter_map(|placed| find(cur, &placed.value, placed.at))
            .collect();
        gone.sort_unstable();
        gone.dedup();
        for run in gone.chunk_by(|a, b| b.prev() == Some(*a)).rev() {
            if let Some(first) = run.first() {
                list.take(*first, Len::of(run.len()));
            }
        }
        if !gone.is_empty() {
            let mut next = gone.iter().peekable();
            let mut at = Pos::<Slot>::ZERO;
            cur.retain(|_| {
                let kept = next.peek().is_none_or(|g| **g != at);
                if !kept {
                    next.next();
                }
                at += Len::ONE;
                kept
            });
        }
        // In their order on the side the step leaves, so the neighbour of each
        // one stands already when it lands. A run of elements, each after the
        // one before it, lands in one place.
        let mut back = back;
        back.sort_by_key(|(_, placed)| placed.at);
        let moves: HashSet<&LoroValue> = self.moved.iter().map(|m| &m.value).collect();
        let mut rest = back.as_slice();
        while let Some(((value, placed), tail)) = rest.split_first() {
            if moves.contains(value) {
                rest = tail;
                let Some(from) = find(cur, value, placed.at) else {
                    continue;
                };
                cur.remove(from.to_usize());
                let to = place_after(cur, placed.pred.as_ref(), placed.at);
                list.shift(from, to, value);
                cur.insert(to.to_usize(), (*value).clone());
                continue;
            }
            let length = 1 + tail
                .iter()
                .zip(rest.iter())
                .take_while(|((next, at), (prev, _))| {
                    !moves.contains(next) && at.pred.as_ref() == Some(*prev)
                })
                .count();
            let (run, after) = rest.split_at(length.min(rest.len()));
            rest = after;
            let to = place_after(cur, placed.pred.as_ref(), placed.at);
            if cur.get(to.to_usize()) == Some(*value) {
                continue;
            }
            let values: Vec<LoroValue> = run.iter().map(|(v, _)| (*v).clone()).collect();
            for (offset, value) in values.iter().enumerate() {
                list.put(Pos::from_usize(to.to_usize() + offset), value);
            }
            let at = to.to_usize().min(cur.len());
            cur.splice(at..at, values);
        }
    }
}

impl MapWrite {
    /// Replay the write into `doc`. A key a later write changed keeps that
    /// write: the step reverts this peer's work and never a peer's
    /// (`UNDO-007`). The undo manager restores the value over the peer's.
    fn write(&self, doc: &LoroDoc, step: HistoryStep) {
        let (from, to) = match step {
            HistoryStep::Undo => (&self.after, &self.before),
            HistoryStep::Redo => (&self.before, &self.after),
        };
        let map: LoroMap = doc.get_map(self.map.as_str());
        let now = match map.get(&self.key) {
            None => None,
            Some(ValueOrContainer::Value(value)) => Some(value),
            Some(ValueOrContainer::Container(_)) => return,
        };
        if now.as_ref() != from.as_ref() {
            return;
        }
        let _ = match to {
            Some(value) => map.insert(&self.key, value.clone()),
            None => map.delete(&self.key),
        };
    }
}

impl Step {
    /// Write the step into `doc`: `Undo` takes each commit back, the last
    /// first, and `Redo` makes each again, the first first. `lists` reads the
    /// values a list holds now from the mirror.
    fn write(
        &self,
        doc: &LoroDoc,
        step: HistoryStep,
        lists: &dyn Fn(&InternalString) -> Vec<LoroValue>,
    ) {
        let chunks: Vec<&Chunk> = match step {
            HistoryStep::Undo => self.chunks.iter().rev().collect(),
            HistoryStep::Redo => self.chunks.iter().collect(),
        };
        // Each list is read once for the whole step, then followed as the
        // step writes it.
        let mut read: HashMap<InternalString, Vec<LoroValue>> = HashMap::new();
        for chunk in chunks {
            let writes: Vec<&ListWrite> = match step {
                HistoryStep::Undo => chunk.lists.iter().rev().collect(),
                HistoryStep::Redo => chunk.lists.iter().collect(),
            };
            for write in writes {
                let (Some(list), ContainerID::Root { name, .. }) =
                    (root_list(doc, &write.list), &write.list)
                else {
                    continue;
                };
                let cur = read.entry(name.clone()).or_insert_with(|| lists(name));
                write.write(&list, cur, step);
            }
            for write in &chunk.maps {
                write.write(doc, step);
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

    #[test]
    fn a_write_into_a_nested_container_ends_the_history() {
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
