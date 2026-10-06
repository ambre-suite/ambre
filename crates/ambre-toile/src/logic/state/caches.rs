//! Derived-cache maintenance for [`ToileState`](super::ToileState): the
//! `bounds` map, the `rstar` spatial index and the materialized `layers` order.
//! Split out of `state/mod.rs`. The incremental helpers keep the caches in step
//! with local edits; [`rebuild_caches`](ToileState::rebuild_caches) reseeds them
//! wholesale after a remote import.

use std::collections::HashSet;

use loro::{LoroMap, LoroValue};

use crate::logic::index::{ElementBounds, SpatialIndex};
use crate::logic::model::{ElementId, NodeId};

use super::codec::{expanded_aabb, f64_of, map_f64};
use super::{CHILDREN, ELEMENTS, LAYERS, ToileState};

impl ToileState {
    /// Recompute one element's AABB from its map and mirror it into `bounds` and
    /// the R-tree.
    pub(super) fn update_bounds(&mut self, id: &ElementId, em: &LoroMap) {
        let rect = expanded_aabb(
            f64_of(em, "x"),
            f64_of(em, "y"),
            f64_of(em, "w"),
            f64_of(em, "h"),
            f64_of(em, "rot"),
        );
        self.bounds.insert(id.clone(), rect);
        self.index
            .update(ElementBounds::from_rect(id.clone(), rect));
    }

    /// Full rebuild of every derived cache from `doc`. Used after a remote
    /// import; the incremental path (local commands) keeps them in step directly.
    pub(super) fn rebuild_caches(&mut self) {
        self.bounds.clear();
        self.parents.clear();
        let mut leaves = Vec::new();
        if let LoroValue::Map(m) = self.walk_elements() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(fields) = v {
                    // Groups and frames name their members; the same pass
                    // indexes them, so the parent lookup costs no walk of its own.
                    if let Some(LoroValue::List(kids)) = fields.get(CHILDREN) {
                        let container = ElementId(key.clone());
                        for kid in kids.iter() {
                            if let LoroValue::String(s) = kid {
                                self.link_parent(&ElementId(s.to_string()), &container);
                            }
                        }
                    }
                    // Groups carry no stored geometry; their bounds are derived
                    // from members by `recompute_groups` after this leaf pass.
                    if matches!(fields.get("type"), Some(LoroValue::String(t)) if t.as_str() == "group")
                    {
                        continue;
                    }
                    let x = map_f64(fields, "x");
                    let y = map_f64(fields, "y");
                    let w = map_f64(fields, "w");
                    let h = map_f64(fields, "h");
                    let rot = map_f64(fields, "rot");
                    let id = ElementId(key.clone());
                    let rect = expanded_aabb(x, y, w, h, rot);
                    self.bounds.insert(id.clone(), rect);
                    leaves.push(ElementBounds::from_rect(id, rect));
                }
            }
        }
        self.index = SpatialIndex::build(leaves.into_iter());
        self.rematerialize_layers();
        self.recompute_groups();
    }

    /// The whole element map, resolved. Every deep read of the board goes
    /// through here, so `board_walks` counts what a command costs (`TOILE-056`).
    pub(super) fn walk_elements(&self) -> LoroValue {
        self.board_walks
            .set(self.board_walks.get().saturating_add(1));
        self.doc.get_map(ELEMENTS).get_deep_value()
    }

    /// Record that `parent`'s `children` list names `child` once more.
    pub(super) fn link_parent(&mut self, child: &ElementId, parent: &ElementId) {
        self.parents
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
    }

    /// Record that `parent`'s `children` list names `child` once less.
    pub(super) fn unlink_parent(&mut self, child: &ElementId, parent: &ElementId) {
        if let Some(list) = self.parents.get_mut(child) {
            if let Some(idx) = list.iter().position(|p| p == parent) {
                list.remove(idx);
            }
            if list.is_empty() {
                self.parents.remove(child);
            }
        }
    }

    /// Drop every entry naming `child`, once the board no longer holds it.
    pub(super) fn forget_parents(&mut self, child: &ElementId) {
        self.parents.remove(child);
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
        if let LoroValue::List(list) = self.doc.get_movable_list(LAYERS).get_deep_value() {
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

    /// Rewrite `layers` keeping only nodes for which `keep` holds. Used by
    /// deletion cleanup. Rebuilds the list in place. An entry whose tag this
    /// build cannot decode is never named by a deletion. It is preserved
    /// verbatim (`TOILE-129`), not destroyed by an unrelated delete.
    pub(super) fn strip_layers(&self, keep: impl Fn(&NodeId) -> bool) {
        let list = self.doc.get_movable_list(LAYERS);
        // Collect survivors from the current list value, then rewrite.
        let mut survivors: Vec<String> = Vec::new();
        if let LoroValue::List(cur) = list.get_deep_value() {
            for v in cur.iter() {
                if let LoroValue::String(s) = v {
                    match NodeId::decode(s) {
                        Some(node) if keep(&node) => survivors.push(s.as_str().to_owned()),
                        Some(_) => {} // a node the deletion named
                        None => survivors.push(s.as_str().to_owned()), // unreadable: not ours
                    }
                }
            }
        }
        let len = list.len();
        let _ = list.delete(0, len);
        for s in survivors {
            let _ = list.push(s.as_str());
        }
    }

    /// The `LoroMap` handle for an element, or `None` if absent.
    pub(super) fn element_map(&self, id: &ElementId) -> Option<LoroMap> {
        self.doc
            .get_map(ELEMENTS)
            .get(id.as_str())?
            .into_container()
            .ok()?
            .into_map()
            .ok()
    }
}
