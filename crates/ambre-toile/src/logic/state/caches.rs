//! Derived-cache maintenance for [`ToileState`](super::ToileState): the
//! `bounds` map, the `rstar` spatial index and the materialized `layers` order.
//! Split out of `state/mod.rs`. The incremental helpers keep the caches in step
//! with local edits; [`rebuild_caches`](ToileState::rebuild_caches) reseeds them
//! wholesale after a remote import.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use kurbo::Rect;
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, Index, LoroDoc, LoroValue, Subscription};

use crate::logic::index::{ElementBounds, SpatialIndex};
use crate::logic::model::{ConnectorId, ElementId, NodeId};

use super::ToileState;
use super::codec::{expanded_aabb, map_placement, rotation_of, stored_rect};
use super::codec::{has_type, value_has_type};
use super::names::{ElementKey, FieldValues, Fields, Root, entry, root_map};
use crate::logic::model::ElementKind;

impl ToileState {
    /// Recompute one element's AABB from its map and mirror it into `bounds` and
    /// the R-tree.
    pub(super) fn update_bounds(&mut self, id: &ElementId, em: &Fields<ElementKey>) {
        let rect = expanded_aabb(stored_rect(em), rotation_of(em));
        self.bounds.insert(id.clone(), rect);
        self.index
            .update(ElementBounds::from_rect(id.clone(), rect));
    }

    /// Full rebuild of every derived cache from `doc`. Used after a remote
    /// import; the incremental path (local commands) keeps them in step directly.
    pub(super) fn rebuild_caches(&mut self) {
        self.bounds.clear();
        self.parents.clear();
        self.members.clear();
        let mut leaves = Vec::new();
        if let LoroValue::Map(m) = self.walk_elements() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(fields) = v {
                    let fields = FieldValues::<ElementKey>::of(fields);
                    // Groups and frames name their members; the same pass
                    // indexes them, so the parent lookup costs no walk of its own.
                    if let Some(LoroValue::List(kids)) = fields.get(ElementKey::Children) {
                        let container = ElementId(key.clone());
                        for kid in kids.iter() {
                            if let LoroValue::String(s) = kid {
                                self.link_parent(&ElementId(s.to_string()), &container);
                            }
                        }
                    }
                    // Groups carry no stored geometry; their bounds are derived
                    // from members by `recompute_groups` after this leaf pass.
                    if value_has_type(fields, ElementKind::Group) {
                        continue;
                    }
                    let (stored, rot) = map_placement(fields);
                    let id = ElementId(key.clone());
                    let rect = expanded_aabb(stored, rot);
                    self.bounds.insert(id.clone(), rect);
                    leaves.push(ElementBounds::from_rect(id, rect));
                }
            }
        }
        self.index = SpatialIndex::build(leaves.into_iter());
        self.rematerialize_layers();
        self.recompute_groups();
    }

    /// Drop what the subscription noted of the board so far: the next refresh
    /// reads only what lands after this call. The arrows it noted stay noted,
    /// for [`wires`](Self::wires) to read.
    pub(super) fn forget_touched(&self) {
        self.touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .board = BoardTouched::default();
    }

    /// Bring every derived cache in step with what a remote change wrote, and
    /// with nothing else (`TOILE-092`). The document's own events name the
    /// elements it touched and whether it reordered. A change the events cannot
    /// place falls back to the full rebuild.
    pub(super) fn refresh_caches(&mut self) {
        let touched = std::mem::take(
            &mut self
                .touched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .board,
        );
        if touched.unplaced {
            self.rebuild_caches();
            return;
        }
        for id in &touched.elements {
            self.refresh_element(id);
        }
        if touched.layers {
            self.rematerialize_layers();
        }
        self.refresh_groups(&touched.elements);
    }

    /// Re-read one element: its bounds, its leaf and what its `children` list
    /// names. A group's bounds are left to [`refresh_groups`](Self::refresh_groups).
    fn refresh_element(&mut self, id: &ElementId) {
        let Some(em) = self.element_map(id) else {
            self.bounds.remove(id);
            self.index.remove(id);
            self.relink_members(id, Vec::new());
            return;
        };
        let kids = self.children_of(id);
        self.relink_members(id, kids);
        if !has_type(&em, ElementKind::Group) {
            self.update_bounds(id, &em);
        }
    }

    /// Recompute the bounds of every group a touched element sits in, at any
    /// depth, and of every touched group, to a fixpoint. A group missing a
    /// member's bounds holds none, as after a full rebuild.
    fn refresh_groups(&mut self, touched: &HashSet<ElementId>) {
        let mut groups: Vec<ElementId> = Vec::new();
        let mut seen: HashSet<ElementId> = HashSet::new();
        let mut stack: Vec<ElementId> = touched.iter().cloned().collect();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if self
                .element_map(&id)
                .is_some_and(|em| has_type(&em, ElementKind::Group))
            {
                groups.push(id.clone());
            }
            stack.extend(self.parents_of(&id).iter().cloned());
        }
        let mut changed = true;
        while changed {
            changed = false;
            for g in &groups {
                let kids = self.members.get(g).map_or(&[][..], Vec::as_slice);
                let mut union: Option<Rect> = None;
                let mut all = !kids.is_empty();
                for k in kids {
                    match self.bounds.get(k) {
                        Some(r) => union = Some(union.map_or(*r, |u| u.union(*r))),
                        None => all = false,
                    }
                }
                let next = if all { union } else { None };
                if self.bounds.get(g).copied() == next {
                    continue;
                }
                match next {
                    Some(rect) => {
                        self.bounds.insert(g.clone(), rect);
                        self.index.update(ElementBounds::from_rect(g.clone(), rect));
                    }
                    None => {
                        self.bounds.remove(g);
                        self.index.remove(g);
                    }
                }
                changed = true;
            }
        }
    }

    /// The whole element map, resolved. Every deep read of the board goes
    /// through here, so `board_walks` counts what a command costs (`TOILE-056`).
    pub(super) fn walk_elements(&self) -> LoroValue {
        self.board_walks.set(self.board_walks.get().incremented());
        self.doc.get_map(Root::Elements.as_str()).get_deep_value()
    }

    /// Record that `parent`'s `children` list names `child` once more.
    pub(super) fn link_parent(&mut self, child: &ElementId, parent: &ElementId) {
        self.parents
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
        self.members
            .entry(parent.clone())
            .or_default()
            .push(child.clone());
    }

    /// Record that `parent`'s `children` list names `child` once less.
    pub(super) fn unlink_parent(&mut self, child: &ElementId, parent: &ElementId) {
        drop_one(&mut self.parents, child, parent);
        drop_one(&mut self.members, parent, child);
    }

    /// Drop every entry naming `child`, once the board no longer holds it.
    pub(super) fn forget_parents(&mut self, child: &ElementId) {
        for parent in self.parents.remove(child).unwrap_or_default() {
            drop_one(&mut self.members, &parent, child);
        }
    }

    /// Drop every entry naming `id`, as a member and as a container, once the
    /// board no longer holds it and no list names it. A peer refreshing the
    /// removed map drops the links its list held, so the two caches agree.
    pub(super) fn forget_links(&mut self, id: &ElementId) {
        self.forget_parents(id);
        self.relink_members(id, Vec::new());
    }

    /// Replace what `parent`'s `children` list names with `kids`, read back
    /// from the document after a change touched it.
    fn relink_members(&mut self, parent: &ElementId, kids: Vec<ElementId>) {
        for old in self.members.get(parent).cloned().unwrap_or_default() {
            self.unlink_parent(&old, parent);
        }
        for kid in &kids {
            self.link_parent(kid, parent);
        }
    }

    /// The containers whose `children` list names `id`, stale ones included.
    /// A reader filters by kind, which also skips a container since deleted.
    pub(super) fn parents_of(&self, id: &ElementId) -> &[ElementId] {
        self.parents.get(id).map_or(&[], Vec::as_slice)
    }

    /// Re-read the `layers` list into `self.layers`, skipping malformed tags and
    /// de-duplicating (keep first) per `toile-z-order`.
    pub(super) fn rematerialize_layers(&mut self) {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        if let LoroValue::List(list) = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .get_deep_value()
        {
            for v in list.iter() {
                if let LoroValue::String(s) = v
                    && let Some(node) = NodeId::decode(s)
                    && seen.insert(node.clone())
                {
                    out.push(node);
                }
            }
        }
        self.layers = out;
    }

    /// Delete from `layers` every entry naming a node for which `keep` fails.
    /// Used by deletion cleanup. Each dropped entry is one delete at its own
    /// index, so the rest of the order is never rewritten (`TOILE-061`). An
    /// entry whose tag this build cannot decode is never named by a deletion.
    /// It is preserved verbatim (`TOILE-129`), not destroyed by an unrelated
    /// delete.
    pub(super) fn strip_layers(&self, keep: impl Fn(&NodeId) -> bool) {
        let list = self.doc.get_movable_list(Root::Layers.as_str());
        let mut dropped: Vec<usize> = Vec::new();
        if let LoroValue::List(cur) = list.get_deep_value() {
            for (i, v) in cur.iter().enumerate() {
                if let LoroValue::String(s) = v
                    && let Some(node) = NodeId::decode(s)
                    && !keep(&node)
                {
                    dropped.push(i);
                }
            }
        }
        // From the back, so each index still names the entry it was read at.
        for i in dropped.into_iter().rev() {
            let _ = list.delete(i, 1);
        }
    }

    /// The map of an element, or `None` if absent.
    pub(super) fn element_map(&self, id: &ElementId) -> Option<Fields<ElementKey>> {
        entry(&root_map(&self.doc, Root::Elements), id.as_str())
    }
}

/// Remove one `value` from the list `map` keeps under `key`, and the key once
/// the list is empty.
fn drop_one(map: &mut HashMap<ElementId, Vec<ElementId>>, key: &ElementId, value: &ElementId) {
    if let Some(list) = map.get_mut(key) {
        if let Some(idx) = list.iter().position(|v| v == value) {
            list.remove(idx);
        }
        if list.is_empty() {
            map.remove(key);
        }
    }
}

/// What the document was written since the caches last read it, as its own
/// events report it. The board half feeds the remote refresh (`TOILE-092`).
/// The arrow half feeds the arrow cache the cull reads (`TOILE-045`).
#[derive(Debug, Default)]
pub(super) struct Touched {
    board: BoardTouched,
    pub(super) wires: WiresTouched,
}

/// The elements and the paint order a change wrote.
#[derive(Debug, Default)]
struct BoardTouched {
    /// Every element a change wrote to: its entry, or a container under it.
    elements: HashSet<ElementId>,
    /// Whether the paint order changed.
    layers: bool,
    /// Set by a change under the element map the events do not place under
    /// one element. The caches then rebuild whole.
    unplaced: bool,
}

/// The arrows a change wrote.
#[derive(Debug, Default)]
pub(super) struct WiresTouched {
    pub(super) ids: HashSet<ConnectorId>,
    /// Set by a change under the arrow map the events do not place under one
    /// arrow. The arrow cache then reads the whole map again.
    pub(super) unplaced: bool,
}

/// The shared sink a document subscription writes into.
pub(super) type TouchedSink = Arc<Mutex<Touched>>;

/// Subscribe to `doc`, noting into the returned sink what every commit and
/// every import writes. The subscription lives as long as the handle.
pub(super) fn watch(doc: &LoroDoc) -> (TouchedSink, Subscription) {
    let sink: TouchedSink = Arc::default();
    let into = Arc::clone(&sink);
    let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
        let mut touched = into.lock().unwrap_or_else(PoisonError::into_inner);
        for change in &event.events {
            note(change, &mut touched);
        }
    }));
    (sink, subscription)
}

/// Place one container change: the paint order, or the entries of the element
/// map or the arrow map it lands under.
fn note(change: &ContainerDiff<'_>, touched: &mut Touched) {
    if matches!(change.target, ContainerID::Root { name, .. } if name.as_str() == Root::Layers.as_str())
    {
        touched.board.layers = true;
        return;
    }
    let board = &mut touched.board;
    if let Some(keys) = keys_under(change, Root::Elements) {
        match keys {
            Some(keys) => board.elements.extend(keys.into_iter().map(ElementId)),
            None => board.unplaced = true,
        }
        return;
    }
    let wires = &mut touched.wires;
    if let Some(keys) = keys_under(change, Root::Connectors) {
        match keys {
            Some(keys) => wires.ids.extend(keys.into_iter().map(ConnectorId)),
            None => wires.unplaced = true,
        }
    }
}

/// The entries of the root map `root` that one change wrote. They are the keys
/// it updated on the map itself, or the one entry its container sits under.
/// `None` when the change is not under `root`, and `Some(None)` when
/// it is but names no entry.
fn keys_under(change: &ContainerDiff<'_>, root: Root) -> Option<Option<Vec<String>>> {
    let is_root = |c: &ContainerID| matches!(c, ContainerID::Root { name, .. } if name.as_str() == root.as_str());
    if is_root(change.target) {
        return Some(match &change.diff {
            Diff::Map(map) => Some(map.updated.keys().map(ToString::to_string).collect()),
            _ => None,
        });
    }
    let at = change.path.iter().position(|(c, _)| is_root(c))?;
    Some(match change.path.get(at + 1) {
        Some((_, Index::Key(key))) => Some(vec![key.to_string()]),
        _ => None,
    })
}
