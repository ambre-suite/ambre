// ambre-trame/src/logic/line_history.rs
// The history of a line delete, kept beside the undo manager's (`TRAME-286`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// Loro's undo manager answers an undo by a checkout. A checkout rebuilds the
// state of every map the item wrote. An undo of a line delete therefore walks
// the whole cell map and the whole generation record.
//
// So a line delete records its own inverse as it writes. It notes each key it touched, with the value
// on each side. It notes each list element it removed, with the element
// before it. Its commit carries an origin the manager excludes, and one stack
// orders these steps with the items of the manager.
//
// The manager reads each excluded commit as a remote edit, and transforms its
// items by position through it. An undo of a recorded delete puts back a new
// element. The manager sees one element deleted and another inserted. An item
// whose undo deletes or moves the old element then misses it. So a delete
// commits for the manager when it removes an element in reach of an item. An
// element is in reach when an operation the item holds, or a later one,
// placed it in a list the item places in.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ambre_core::HistoryStep;
use ambre_core::crdt::CommitOrigin;
use loro::{
    ContainerID, ContainerTrait, ContainerType, Counter, InternalString, LoroDoc, LoroMap, PeerID,
    UndoItemMeta, UndoManager, ValueOrContainer,
};

use super::remote::LocalNote;
use super::*;
use crate::model::Spot;

/// One key a line delete wrote, with its value on each side of the delete.
#[derive(Debug, Clone)]
struct MapWrite {
    map: ContainerID,
    key: InternalString,
    /// The value before the delete, `None` for an absent key.
    before: Option<LoroValue>,
    /// The value after the delete, `None` for an absent key.
    after: Option<LoroValue>,
}

/// One element a line delete took out of a list.
#[derive(Debug, Clone)]
struct ListDelete {
    list: ContainerID,
    spot: Spot,
}

/// What a line delete writes, noted as it writes it.
#[derive(Debug, Default)]
pub(super) struct Journal {
    writes: Vec<MapWrite>,
    seen: HashSet<(ContainerID, InternalString)>,
    deletes: Vec<ListDelete>,
    /// Set when a write lands where the journal cannot replay it. The delete
    /// then goes to the undo manager as before.
    partial: Partial,
}

ambre_core::define_flag! {
    /// Whether the journal holds every write of the delete.
    enum Partial {
        /// Every write is held.
        Whole,
        /// A write landed in a nested container or held a container value.
        Lost,
    }
}

impl Journal {
    /// Note the key `key` of `map` just before a write, the first time only.
    pub(super) fn note(&mut self, map: &LoroMap, key: &str) {
        let id = map.id();
        if !matches!(id, ContainerID::Root { .. }) {
            self.partial = Partial::Lost;
            return;
        }
        let key = InternalString::from(key);
        if !self.seen.insert((id.clone(), key.clone())) {
            return;
        }
        let Some(before) = plain(map.get(&key)) else {
            self.partial = Partial::Lost;
            return;
        };
        self.writes.push(MapWrite {
            map: id,
            key,
            before,
            after: None,
        });
    }

    /// Note the elements a delete took out of `list`, in the order it took
    /// them.
    pub(super) fn note_list(&mut self, list: ContainerID, spots: &[Spot]) {
        self.deletes.extend(spots.iter().map(|spot| ListDelete {
            list: list.clone(),
            spot: spot.clone(),
        }));
    }

    /// The step the journal holds, each value after the delete read from
    /// `doc`. `None` when a write escaped the journal. `None` too when an
    /// element it removed is in reach of an item of the manager. `reach`
    /// holds what each item places, and `peer` is this peer.
    fn finish(mut self, doc: &LoroDoc, reach: &[&Places], peer: PeerID) -> Option<LineStep> {
        if self.partial == Partial::Lost {
            return None;
        }
        let reached = |d: &ListDelete| {
            reach
                .iter()
                .flat_map(|places| places.iter())
                .any(|(list, from)| {
                    list.as_ref().is_none_or(|list| *list == d.list)
                        && d.spot
                            .placed
                            .is_none_or(|id| id.peer == peer && id.counter >= *from)
                })
        };
        if self.deletes.iter().any(reached) {
            return None;
        }
        for write in &mut self.writes {
            write.after = plain(map_of(doc, &write.map)?.get(&write.key))?;
        }
        Some(LineStep {
            writes: self.writes,
            deletes: self.deletes,
        })
    }
}

/// A value read from a map, `Some(None)` for an absent key and `None` for a
/// container the journal cannot replay.
fn plain(read: Option<ValueOrContainer>) -> Option<Option<LoroValue>> {
    match read {
        None => Some(None),
        Some(ValueOrContainer::Value(value)) => Some(Some(value)),
        Some(ValueOrContainer::Container(_)) => None,
    }
}

/// The root map `id` names.
fn map_of(doc: &LoroDoc, id: &ContainerID) -> Option<LoroMap> {
    match id {
        ContainerID::Root {
            name,
            container_type: ContainerType::Map,
        } => Some(doc.get_map(name.as_str())),
        _ => None,
    }
}

/// The inverse of one line delete, and the delete again.
#[derive(Debug, Clone)]
pub(super) struct LineStep {
    writes: Vec<MapWrite>,
    deletes: Vec<ListDelete>,
}

impl LineStep {
    /// Write the step into `doc`: `Undo` takes the delete back, `Redo` does it
    /// again. A key a later write changed keeps that write, and an element
    /// already where the step would put it is left alone. The step reverts
    /// this peer's work and never a peer's (`UNDO-007`).
    fn write(&self, doc: &LoroDoc, step: HistoryStep) {
        // The elements come back in the reverse order of their deletes, so
        // the element before each one stands again when it lands.
        match step {
            HistoryStep::Undo => {
                for delete in self.deletes.iter().rev() {
                    restore(doc, delete);
                }
            }
            HistoryStep::Redo => {
                for delete in &self.deletes {
                    remove(doc, delete);
                }
            }
        }
        for write in &self.writes {
            let (from, to) = match step {
                HistoryStep::Undo => (&write.after, &write.before),
                HistoryStep::Redo => (&write.before, &write.after),
            };
            let Some(map) = map_of(doc, &write.map) else {
                continue;
            };
            if plain(map.get(&write.key)).as_ref() != Some(from) {
                continue;
            }
            let _ = match to {
                Some(value) => map.insert(&write.key, value.clone()),
                None => map.delete(&write.key),
            };
        }
    }
}

/// Put the element of `delete` back after the element that stood before it.
fn restore(doc: &LoroDoc, delete: &ListDelete) {
    crate::model::restore_spot(doc, &delete.list, &delete.spot);
}

/// Take the element of `delete` out of its list again, wherever it stands.
fn remove(doc: &LoroDoc, delete: &ListDelete) {
    crate::model::remove_spot(doc, &delete.list, &delete.spot);
}

/// Where an item of the manager places list elements: each list, `None` for
/// every list, with the first operation of this peer that places there. An
/// undo of the item deletes or moves what it placed.
pub(super) type Places = Vec<(Option<ContainerID>, Counter)>;

/// Fold `more` into `places`, each list keeping its first operation.
fn widen(places: &mut Places, more: Places) {
    for (list, from) in more {
        match places.iter_mut().find(|(at, _)| *at == list) {
            Some((_, first)) => *first = (*first).min(from),
            None => places.push((list, from)),
        }
    }
}

/// One step of the history: an item of the undo manager, or a line delete
/// this module holds.
#[derive(Debug)]
pub(super) enum Step {
    /// An item the undo manager holds, with the lists it places elements in.
    /// The manager replays it.
    Managed(Places),
    /// A line delete, replayed from its journal.
    Line(Box<LineStep>),
}

/// The history of the document: the steps an undo walks back, and the steps
/// a redo walks forward. The managed steps match the items of the undo
/// manager's two stacks, one for one and in order.
#[derive(Debug)]
pub(super) struct History {
    past: Vec<Step>,
    future: Vec<Step>,
    /// Where the items the manager pushed since the last read place list
    /// elements.
    placing: Arc<Mutex<Places>>,
}

impl History {
    /// An empty history that watches each item `manager` pushes. An item
    /// pushed by an undo or a redo carries no event, and counts as placing.
    pub(super) fn watching(manager: &mut UndoManager) -> Self {
        let placing = Arc::new(Mutex::new(Places::new()));
        let seen = Arc::clone(&placing);
        manager.set_on_push(Some(Box::new(move |_, span, event| {
            let places: Places = match event {
                None => vec![(None, span.start)],
                Some(event) => event
                    .events
                    .iter()
                    .filter(|diff| diff.target.container_type() == ContainerType::MovableList)
                    .map(|diff| (Some(diff.target.clone()), span.start))
                    .collect(),
            };
            if let Ok(mut seen) = seen.lock() {
                widen(&mut seen, places);
            }
            UndoItemMeta::new()
        })));
        Self {
            past: Vec::new(),
            future: Vec::new(),
            placing,
        }
    }

    /// Where the items pushed since the last call place list elements.
    fn take_placing(&self) -> Places {
        self.placing
            .lock()
            .map(|mut seen| std::mem::take(&mut *seen))
            .unwrap_or_default()
    }

    /// How many managed steps `steps` holds.
    fn managed(steps: &[Step]) -> usize {
        steps
            .iter()
            .filter(|s| matches!(s, Step::Managed(_)))
            .count()
    }

    /// Drop the topmost managed steps of `steps` until `count` remain.
    fn trim_top(steps: &mut Vec<Step>, count: usize) {
        let mut extra = Self::managed(steps).saturating_sub(count);
        let mut at = steps.len();
        while extra > 0 && at > 0 {
            at -= 1;
            if matches!(steps.get(at), Some(Step::Managed(_))) {
                steps.remove(at);
                extra -= 1;
            }
        }
    }

    /// Drop the oldest managed steps of `steps` until `count` remain.
    fn trim_bottom(steps: &mut Vec<Step>, count: usize) {
        let mut extra = Self::managed(steps).saturating_sub(count);
        steps.retain(|s| {
            if extra > 0 && matches!(s, Step::Managed(_)) {
                extra -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// The path a line delete takes, for the test that holds the recorded step
/// against the undo manager (`TRAME-286`).
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryPath {
    /// The delete records its own inverse.
    Recorded,
    /// The reference model: the undo manager holds every delete.
    Managed,
}

impl TrameState {
    /// Whether the last step an undo would take back is a recorded line
    /// delete.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn last_step_is_recorded(&self) -> bool {
        matches!(self.history.past.last(), Some(Step::Line(_)))
    }

    /// Whether the next step a redo would take is a recorded line delete.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn next_redo_is_recorded(&self) -> bool {
        matches!(self.history.future.last(), Some(Step::Line(_)))
    }

    /// Whether an undo has a step to take back.
    pub(super) fn can_undo(&self) -> ambre_core::Availability {
        ambre_core::Availability::from_bool(!self.history.past.is_empty())
    }

    /// Whether a redo has a step to do again.
    pub(super) fn can_redo(&self) -> ambre_core::Availability {
        ambre_core::Availability::from_bool(!self.history.future.is_empty())
    }

    /// Follow the undo manager across a command that is no undo and no redo.
    /// `depth` is its undo stack before the command. A new item clears the
    /// redo side, as the manager clears its own. A write the manager merged
    /// into its top item widens what that item places.
    pub(super) fn follow_manager(&mut self, depth: usize) {
        let now = self.undo_manager.undo_count();
        let placed = self.history.take_placing();
        if now > depth {
            self.history
                .past
                .extend(std::iter::repeat_with(|| Step::Managed(placed.clone())).take(now - depth));
            self.history.future.clear();
        } else if !placed.is_empty()
            && let Some(Step::Managed(top)) = self
                .history
                .past
                .iter_mut()
                .rev()
                .find(|s| matches!(s, Step::Managed(_)))
        {
            widen(top, placed);
        }
        // A checkout or a new peer id empties the manager's stacks.
        History::trim_bottom(&mut self.history.past, now);
        History::trim_bottom(&mut self.history.future, self.undo_manager.redo_count());
    }

    /// Open a journal for the line delete that starts.
    pub(super) fn open_journal(&self) {
        *self.journal.borrow_mut() = Some(Journal::default());
    }

    /// Note the key `key` of `map` before a write, when a journal is open.
    pub(super) fn note_write(&self, map: &LoroMap, key: &str) {
        if let Some(journal) = self.journal.borrow_mut().as_mut() {
            journal.note(map, key);
        }
    }

    /// Note the elements a delete took out of `list`, when a journal is open.
    pub(super) fn note_list(&self, list: ContainerID, spots: &[Spot]) {
        if let Some(journal) = self.journal.borrow_mut().as_mut() {
            journal.note_list(list, spots);
        }
    }

    /// Drop the open journal, for a delete that removed nothing.
    pub(super) fn drop_journal(&self) {
        self.journal.borrow_mut().take();
    }

    /// Commit the line delete the open journal noted. A whole journal becomes
    /// a step of this history, and its commit carries the origin the undo
    /// manager excludes. Any other delete commits as before, for the manager.
    pub(super) fn commit_line_delete(&mut self) {
        let journal = self.journal.borrow_mut().take();
        #[cfg(all(test, not(target_arch = "wasm32")))]
        let journal = journal.filter(|_| self.history_path == HistoryPath::Recorded);
        // What the items of the manager place. An undo of a recorded delete
        // puts back an element they no longer name. So a delete of an element
        // in their reach goes to the manager.
        let reach: Vec<&Places> = self
            .history
            .past
            .iter()
            .filter_map(|s| match s {
                Step::Managed(places) => Some(places),
                Step::Line(_) => None,
            })
            .collect();
        let peer = self.doc.peer_id();
        let Some(step) = journal.and_then(|j| j.finish(&self.doc, &reach, peer)) else {
            self.doc.commit();
            return;
        };
        // The default sheet a first edit stores goes with this commit, not
        // as an item of the manager after it (`TRAME-077`).
        self.hold_default_sheet();
        self.doc
            .set_next_commit_origin(CommitOrigin::Recorded.as_str());
        self.doc.commit();
        // The manager keeps its redo items, and no managed step names them
        // any more: a redo only ever replays the items above them.
        self.history.past.push(Step::Line(Box::new(step)));
        self.history.future.clear();
    }

    /// Take one step of the history back or forward. A managed step goes to
    /// the undo manager. A line delete replays its journal. The tail carries
    /// every cache either way (`after_history`).
    pub(super) fn step_history(&mut self, step: HistoryStep) -> TrameOutcome {
        let from = match step {
            HistoryStep::Undo => &mut self.history.past,
            HistoryStep::Redo => &mut self.history.future,
        };
        match from.pop() {
            None => TrameOutcome::display_only(Vec::new()),
            Some(Step::Managed(places)) => {
                from.push(Step::Managed(places));
                self.managed_step(step)
            }
            Some(Step::Line(line)) => {
                let outcome = self.line_step(step, &line);
                let to = match step {
                    HistoryStep::Undo => &mut self.history.future,
                    HistoryStep::Redo => &mut self.history.past,
                };
                to.push(Step::Line(line));
                self.refresh_undo_state(outcome)
            }
        }
    }

    /// Replay the top item of the undo manager, then move the managed steps
    /// by what its two stacks did.
    fn managed_step(&mut self, step: HistoryStep) -> TrameOutcome {
        let undo = self.undo_manager.undo_count();
        let redo = self.undo_manager.redo_count();
        let outcome = self.apply_history(step);
        let placed = self.history.take_placing();
        let undo_now = self.undo_manager.undo_count();
        let redo_now = self.undo_manager.redo_count();
        let (from, to, gained) = match step {
            HistoryStep::Undo => (
                &mut self.history.past,
                &mut self.history.future,
                redo_now.saturating_sub(redo),
            ),
            HistoryStep::Redo => (
                &mut self.history.future,
                &mut self.history.past,
                undo_now.saturating_sub(undo),
            ),
        };
        let left = match step {
            HistoryStep::Undo => undo_now,
            HistoryStep::Redo => redo_now,
        };
        History::trim_top(from, left);
        to.extend(std::iter::repeat_with(|| Step::Managed(placed.clone())).take(gained));
        self.refresh_undo_state(outcome)
    }

    /// Write one line step, then run the tail of a history step on what it
    /// wrote.
    fn line_step(&mut self, step: HistoryStep, line: &LineStep) -> TrameOutcome {
        let order = self.sheet_order_ids();
        drop(self.take_imported());
        self.engine.changed.clear();
        self.note_local(LocalNote::Noted);
        line.write(&self.doc, step);
        self.doc
            .set_next_commit_origin(CommitOrigin::Recorded.as_str());
        self.doc.commit();
        self.note_local(LocalNote::Ignored);
        let written = self.take_imported();
        match step {
            HistoryStep::Undo => self.after_undo(&written, &order),
            HistoryStep::Redo => self.after_redo(&written, &order),
        }
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
