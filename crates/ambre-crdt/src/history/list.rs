//! The list half of a step: each element a commit put in, took out or
//! moved, named by its value and placed after its old neighbour
//! (`ambre-spec-undo.md section 3`).

use std::collections::HashSet;

use ambre_core::HistoryStep;
use ambre_core::index::{Len, Pos};
use loro::event::ListDiffItem;
use loro::{
    ContainerID, ContainerTrait, ContainerType, LoroDoc, LoroList, LoroMovableList, LoroValue,
    ValueOrContainer,
};

use super::{Motion, Slot};

/// A list of either kind, the one container a step moves elements in.
pub(super) enum SeqList {
    Plain(LoroList),
    Movable(LoroMovableList),
}

/// The live list `id` names, `None` for any other container or for a list a
/// peer deleted.
pub(super) fn seq_list(doc: &LoroDoc, id: &ContainerID) -> Option<SeqList> {
    let list = match id.container_type() {
        ContainerType::List => SeqList::Plain(doc.get_list(id.clone())),
        ContainerType::MovableList => SeqList::Movable(doc.get_movable_list(id.clone())),
        _ => return None,
    };
    (!list.is_deleted()).then_some(list)
}

impl SeqList {
    /// Whether a peer deleted the list, or the container that held it.
    fn is_deleted(&self) -> bool {
        match self {
            Self::Plain(list) => list.is_deleted(),
            Self::Movable(list) => list.is_deleted(),
        }
    }

    /// The values of the list, in order.
    pub(super) fn values(&self) -> Vec<LoroValue> {
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

/// One element a commit put into a list or took out of it. It keeps its place
/// and the element just before it, on that side of the commit.
#[derive(Debug, Clone)]
pub(super) struct Placed {
    pub(super) value: LoroValue,
    at: Pos<Slot>,
    pred: Option<LoroValue>,
}

/// One element a commit moved inside a list, with its place and its
/// neighbour on each side of the commit.
#[derive(Debug, Clone)]
pub(super) struct Moved {
    value: LoroValue,
    from: Placed,
    to: Placed,
}

/// What one commit did to one list.
#[derive(Debug, Clone)]
pub(super) struct ListWrite {
    pub(super) list: ContainerID,
    /// The elements the commit took out, in their order before it.
    pub(super) removed: Vec<Placed>,
    /// The elements the commit put in, in their order after it.
    pub(super) added: Vec<Placed>,
    /// The elements the commit moved.
    moved: Vec<Moved>,
}

/// Apply the list diff `items` to the mirror `list`, and read what it did.
/// An element taken out keeps its place and neighbour before. An element put
/// in keeps its place and neighbour after, and a move keeps both.
pub(super) fn take_list(
    list: &mut Vec<LoroValue>,
    items: &[ListDiffItem],
    id: ContainerID,
) -> ListWrite {
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
pub(super) fn plain(value: &ValueOrContainer) -> LoroValue {
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
    pub(super) fn write(&self, list: &SeqList, cur: &mut Vec<LoroValue>, step: HistoryStep) {
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
