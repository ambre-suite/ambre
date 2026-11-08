// ambre-trame/src/logic/line_history.rs
// The history of a line insert or delete, kept beside the undo manager's
// (`TRAME-286`, `TRAME-289`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// Loro's undo manager answers an undo by a checkout. A checkout rebuilds the
// state of every map the item wrote. An undo of a line delete therefore walks
// the whole cell map and the whole generation record. A first undo also builds
// a cache of the whole history.
//
// So a line insert or delete records its own inverse as it writes. It notes
// each key it touched, with the value on each side. It notes each list
// element it put in or took out, with the element before it. Its commit
// carries an origin the manager excludes, and one stack orders these steps
// with the items of the manager.

use std::collections::HashSet;

use ambre_core::HistoryStep;
use ambre_core::crdt::CommitOrigin;
use loro::{
    ContainerID, ContainerTrait, ContainerType, InternalString, LoroDoc, LoroMap, ValueOrContainer,
};

use super::remote::LocalNote;
use super::*;
use crate::model::Spot;

/// One key a line edit wrote, with its value on each side of the edit.
#[derive(Debug, Clone)]
struct MapWrite {
    map: ContainerID,
    key: InternalString,
    /// The value before the edit, `None` for an absent key.
    before: Option<LoroValue>,
    /// The value after the edit, `None` for an absent key.
    after: Option<LoroValue>,
}

/// What a structural edit did to one element of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListEdit {
    /// An insert put the element in.
    Inserted,
    /// A delete took the element out.
    Removed,
}

/// One element a line insert put into a list, or a line delete took out.
#[derive(Debug, Clone)]
struct ListWrite {
    list: ContainerID,
    edit: ListEdit,
    spot: Spot,
}

/// What a line insert or delete writes, noted as it writes it.
#[derive(Debug, Default)]
pub(super) struct Journal {
    writes: Vec<MapWrite>,
    seen: HashSet<(ContainerID, InternalString)>,
    lists: Vec<ListWrite>,
    /// Set when a write lands where the journal cannot replay it. The edit
    /// then goes to the undo manager as before.
    partial: Partial,
}

ambre_core::define_flag! {
    /// Whether the journal holds every write of the edit.
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

    /// Note the elements an edit put into `list` or took out of it, in the
    /// order it wrote them.
    pub(super) fn note_list(&mut self, list: ContainerID, edit: ListEdit, spots: &[Spot]) {
        self.lists.extend(spots.iter().map(|spot| ListWrite {
            list: list.clone(),
            edit,
            spot: spot.clone(),
        }));
    }

    /// The step the journal holds, each value after the edit read from
    /// `doc`. `None` when a write escaped the journal.
    fn finish(mut self, doc: &LoroDoc) -> Option<LineStep> {
        if self.partial == Partial::Lost {
            return None;
        }
        for write in &mut self.writes {
            write.after = plain(map_of(doc, &write.map)?.get(&write.key))?;
        }
        Some(LineStep {
            writes: self.writes,
            lists: self.lists,
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

/// The inverse of one line insert or delete, and the edit again.
#[derive(Debug, Clone)]
pub(super) struct LineStep {
    writes: Vec<MapWrite>,
    lists: Vec<ListWrite>,
}

impl LineStep {
    /// Write the step into `doc`: `Undo` takes the edit back, `Redo` does it
    /// again. A key a later write changed keeps that write, and an element
    /// already where the step would put it is left alone. The step reverts
    /// this peer's work and never a peer's (`UNDO-007`).
    fn write(&self, doc: &LoroDoc, step: HistoryStep) {
        // An undo walks the list writes back in reverse, so the element before
        // each one stands again when it lands. A redo walks them forward.
        match step {
            HistoryStep::Undo => {
                for write in self.lists.iter().rev() {
                    match write.edit {
                        ListEdit::Removed => restore(doc, write),
                        ListEdit::Inserted => remove(doc, write),
                    }
                }
            }
            HistoryStep::Redo => {
                for write in &self.lists {
                    match write.edit {
                        ListEdit::Removed => remove(doc, write),
                        ListEdit::Inserted => restore(doc, write),
                    }
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

/// Put the element of `write` back after the element that stood before it.
fn restore(doc: &LoroDoc, write: &ListWrite) {
    crate::model::restore_spot(doc, &write.list, &write.spot);
}

/// Take the element of `write` out of its list, wherever it stands.
fn remove(doc: &LoroDoc, write: &ListWrite) {
    crate::model::remove_spot(doc, &write.list, &write.spot);
}

/// One step of the history: an item of the undo manager, or a line insert or
/// delete this module holds.
#[derive(Debug)]
pub(super) enum Step {
    /// An item the undo manager holds. The manager replays it.
    Managed,
    /// A line insert or delete, replayed from its journal.
    Line(Box<LineStep>),
}

/// The history of the document: the steps an undo walks back, and the steps
/// a redo walks forward. The managed steps match the items of the undo
/// manager's two stacks, one for one and in order.
#[derive(Debug, Default)]
pub(super) struct History {
    past: Vec<Step>,
    future: Vec<Step>,
}

impl History {
    /// How many managed steps `steps` holds.
    fn managed(steps: &[Step]) -> usize {
        steps.iter().filter(|s| matches!(s, Step::Managed)).count()
    }

    /// Drop the topmost managed steps of `steps` until `count` remain.
    fn trim_top(steps: &mut Vec<Step>, count: usize) {
        let mut extra = Self::managed(steps).saturating_sub(count);
        let mut at = steps.len();
        while extra > 0 && at > 0 {
            at -= 1;
            if matches!(steps.get(at), Some(Step::Managed)) {
                steps.remove(at);
                extra -= 1;
            }
        }
    }

    /// Drop the oldest managed steps of `steps` until `count` remain.
    fn trim_bottom(steps: &mut Vec<Step>, count: usize) {
        let mut extra = Self::managed(steps).saturating_sub(count);
        steps.retain(|s| {
            if extra > 0 && matches!(s, Step::Managed) {
                extra -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// The path a line insert or delete takes, for the test that holds the
/// recorded step against the undo manager (`TRAME-286`, `TRAME-289`).
#[cfg(all(test, not(target_arch = "wasm32")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryPath {
    /// The edit records its own inverse.
    Recorded,
    /// The reference model: the undo manager holds every insert and delete.
    Managed,
}

impl TrameState {
    /// Whether the last step an undo would take back is a recorded line
    /// insert or delete.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn last_step_is_recorded(&self) -> bool {
        matches!(self.history.past.last(), Some(Step::Line(_)))
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
    /// redo side, as the manager clears its own.
    pub(super) fn follow_manager(&mut self, depth: usize) {
        let now = self.undo_manager.undo_count();
        if now > depth {
            self.history
                .past
                .extend(std::iter::repeat_with(|| Step::Managed).take(now - depth));
            self.history.future.clear();
        }
        // A checkout or a new peer id empties the manager's stacks.
        History::trim_bottom(&mut self.history.past, now);
        History::trim_bottom(&mut self.history.future, self.undo_manager.redo_count());
    }

    /// Open a journal for the line insert or delete that starts.
    pub(super) fn open_journal(&self) {
        *self.journal.borrow_mut() = Some(Journal::default());
    }

    /// Note the key `key` of `map` before a write, when a journal is open.
    pub(super) fn note_write(&self, map: &LoroMap, key: &str) {
        if let Some(journal) = self.journal.borrow_mut().as_mut() {
            journal.note(map, key);
        }
    }

    /// Note the elements an edit put into `list` or took out of it, when a
    /// journal is open.
    pub(super) fn note_list(&self, list: ContainerID, edit: ListEdit, spots: &[Spot]) {
        if let Some(journal) = self.journal.borrow_mut().as_mut() {
            journal.note_list(list, edit, spots);
        }
    }

    /// Drop the open journal, for an edit that wrote nothing.
    pub(super) fn drop_journal(&self) {
        self.journal.borrow_mut().take();
    }

    /// Commit the line insert or delete the open journal noted. A whole
    /// journal becomes a step of this history, and its commit carries the
    /// origin the undo manager excludes. Any other edit commits as before,
    /// for the manager.
    pub(super) fn commit_recorded(&mut self) {
        let journal = self.journal.borrow_mut().take();
        #[cfg(all(test, not(target_arch = "wasm32")))]
        let journal = journal.filter(|_| self.history_path == HistoryPath::Recorded);
        let Some(step) = journal.and_then(|j| j.finish(&self.doc)) else {
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
    /// the undo manager. A line insert or delete replays its journal. The tail carries
    /// every cache either way (`after_history`).
    pub(super) fn step_history(&mut self, step: HistoryStep) -> TrameOutcome {
        let from = match step {
            HistoryStep::Undo => &mut self.history.past,
            HistoryStep::Redo => &mut self.history.future,
        };
        match from.pop() {
            None => TrameOutcome::display_only(Vec::new()),
            Some(Step::Managed) => {
                from.push(Step::Managed);
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
        to.extend(std::iter::repeat_with(|| Step::Managed).take(gained));
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
