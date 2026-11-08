// ambre-trame/src/logic/history.rs
// Trame's one undo history: each edit records its own inverse (`TRAME-288`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// Loro's undo manager answers an undo by a checkout. A checkout rebuilds the
// state of every map the item wrote. On a sheet of 10k formulas the undo of
// one cell write so costs about 250 million instructions.
//
// So every edit records its own inverse, and no undo manager runs. A
// subscription keeps a mirror of every root container. Each local commit
// reads the value each key held and the place each list element held from
// that mirror, before the mirror takes the commit. An undo writes those values
// back and puts each element back after its old neighbour. An element is
// named by its value, so a step never depends on the identity of an element
// in the list.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::HistoryStep;
use ambre_core::crdt::{CommitOrigin, FoundationRoot};
use ambre_core::index::Pos;
use loro::event::{ContainerDiff, Diff, DiffEvent, ListDiffItem};
use loro::{
    ContainerID, ContainerTrait, ContainerType, EventTriggerKind, InternalString, LoroDoc, LoroMap,
    Subscription, ValueOrContainer,
};

use super::remote::LocalNote;
use super::*;
use crate::model::{RootList, Slot, root_list};

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
pub(super) struct Step {
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
pub(super) struct History {
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
pub(super) struct Recorder {
    mirror: Mirror,
    history: History,
}

/// The recorder the subscription and the state share.
pub(super) type RecorderSink = Arc<Mutex<Recorder>>;

/// The origins of a local commit that is no edit of the user. They are the
/// materialized lines, the registry entry, and the replay of a step.
const UNRECORDED: [CommitOrigin; 3] = [
    CommitOrigin::Materialize,
    CommitOrigin::Peers,
    CommitOrigin::Recorded,
];

/// Subscribe the recorder to `doc`, its mirror read from what `doc` holds
/// now. The subscription lives as long as the state holds it.
pub(super) fn watch(doc: &LoroDoc) -> (RecorderSink, Subscription) {
    let sink: RecorderSink = Arc::new(Mutex::new(Recorder {
        mirror: Mirror::read(doc),
        history: History::default(),
    }));
    let into = Arc::clone(&sink);
    let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
        let mut held = into.lock().unwrap_or_else(PoisonError::into_inner);
        let held = &mut *held;
        let record = event.triggered_by == EventTriggerKind::Local
            && !UNRECORDED
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
            if held.mirror.take(change, journal) == Replayable::Lost {
                replayable = Replayable::Lost;
            }
        }
        match event.triggered_by {
            EventTriggerKind::Local if record => match replayable {
                Replayable::Whole => held.history.record(chunk, touched),
                Replayable::Lost => held.history.clear(),
            },
            EventTriggerKind::Import => held.history.meet_import(&touched),
            _ => {}
        }
    }));
    (sink, subscription)
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
        // The last first, so each place the write noted still holds.
        for placed in out.iter().rev() {
            if let Some(at) = find(cur, &placed.value, placed.at) {
                list.take(at);
                cur.remove(at.to_usize());
            }
        }
        // In their order on the side the step leaves, so the neighbour of each
        // one stands already when it lands.
        let mut back = back;
        back.sort_by_key(|(_, placed)| placed.at);
        let moves: HashSet<&LoroValue> = self.moved.iter().map(|m| &m.value).collect();
        for (value, placed) in back {
            if moves.contains(value) {
                let Some(from) = find(cur, value, placed.at) else {
                    continue;
                };
                cur.remove(from.to_usize());
                let to = place_after(cur, placed.pred.as_ref(), placed.at);
                list.shift(from, to, value);
                cur.insert(to.to_usize(), value.clone());
            } else {
                let to = place_after(cur, placed.pred.as_ref(), placed.at);
                if cur.get(to.to_usize()) == Some(value) {
                    continue;
                }
                list.put(to, value);
                cur.insert(to.to_usize(), value.clone());
            }
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

/// The path an undo takes, for the test that holds the history against the
/// undo manager (`TRAME-286`).
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryPath {
    /// Each edit records its own inverse.
    Recorded,
    /// The reference model: the undo manager holds every edit.
    Managed,
}

impl TrameState {
    /// The recorder, locked.
    fn recorder(&self) -> std::sync::MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the test runs the reference model.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn managed(&self) -> bool {
        self.history_path == HistoryPath::Managed
    }

    /// How many steps an undo can take back, for the tests that count them.
    #[cfg(test)]
    pub(super) fn undo_depth(&self) -> usize {
        self.recorder().history.past.len()
    }

    /// Whether an undo has a step to take back.
    pub(super) fn can_undo(&self) -> ambre_core::Availability {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return ambre_core::Availability::from_bool(self.oracle.can_undo());
        }
        ambre_core::Availability::from_bool(!self.recorder().history.past.is_empty())
    }

    /// Whether a redo has a step to do again.
    pub(super) fn can_redo(&self) -> ambre_core::Availability {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return ambre_core::Availability::from_bool(self.oracle.can_redo());
        }
        ambre_core::Availability::from_bool(!self.recorder().history.future.is_empty())
    }

    /// End the step of the last command and open one for the command that
    /// starts: every commit it makes joins one step (`TRAME-017`).
    pub(super) fn seal_history(&mut self) {
        self.recorder().history.seal();
        #[cfg(all(test, not(target_arch = "wasm32")))]
        {
            self.oracle.group_end();
            let _ = self.oracle.group_start();
        }
    }

    /// Forget every step, for a new identity: a step names this peer's work.
    pub(super) fn clear_history(&mut self) {
        self.recorder().history.clear();
    }

    /// Take one step of the history back or forward, then run the tail on
    /// what it wrote (`after_history`). An empty side reports nothing: the
    /// availability the chrome holds is still true (`UNDO-009`).
    pub(super) fn step_history(&mut self, step: HistoryStep) -> TrameOutcome {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        if self.managed() {
            return self.apply_history(step);
        }
        let taken = {
            let mut held = self.recorder();
            match step {
                HistoryStep::Undo => held.history.past.pop(),
                HistoryStep::Redo => held.history.future.pop(),
            }
        };
        let Some(taken) = taken else {
            return TrameOutcome::display_only(Vec::new());
        };
        let order = self.sheet_order_ids();
        drop(self.take_imported());
        self.engine.changed.clear();
        self.note_local(LocalNote::Noted);
        {
            let recorder = Arc::clone(&self.recorder);
            let lists = move |name: &InternalString| {
                let held = recorder.lock().unwrap_or_else(PoisonError::into_inner);
                match held.mirror.roots.get(name) {
                    Some(Mirrored::List(items)) => items.clone(),
                    _ => Vec::new(),
                }
            };
            taken.write(&self.doc, step, &lists);
        }
        self.doc
            .set_next_commit_origin(CommitOrigin::Recorded.as_str());
        self.doc.commit();
        self.note_local(LocalNote::Ignored);
        let written = self.take_imported();
        {
            let mut held = self.recorder();
            match step {
                HistoryStep::Undo => held.history.future.push(taken),
                HistoryStep::Redo => held.history.past.push(taken),
            }
        }
        let outcome = match step {
            HistoryStep::Undo => self.after_undo(&written, &order),
            HistoryStep::Redo => self.after_redo(&written, &order),
        };
        self.refresh_undo_state(outcome)
    }

    /// Report the availability the history holds once the step moved, in
    /// place of the one the tail read before.
    fn refresh_undo_state(&self, mut outcome: TrameOutcome) -> TrameOutcome {
        if let Some(last) = outcome.display.last_mut()
            && matches!(last, TrameEvent::UndoState { .. })
        {
            *last = self.undo_state_event();
        }
        outcome
    }
}
