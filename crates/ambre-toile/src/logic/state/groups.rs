//! Grouping, frame membership and the group/frame tree helpers for
//! [`ToileState`](super::ToileState). Split out of
//! `state/mod.rs`. A node belongs to at most one container; group geometry is
//! derived (never in the CRDT) and lives only in the `bounds` / R-tree caches.

use std::collections::HashSet;

use kurbo::Rect;
use loro::{LoroMap, LoroMovableList, LoroValue};

use crate::logic::command::{ToileError, ToileEvent, ToileOutcome};
use crate::logic::index::ElementBounds;
use crate::logic::model::{ElementId, NodeId};
use ambre_bus::app::toile::FRAME_NAME_MAX;

use super::codec::{em_seed_group, str_of};
use super::{CHILDREN, ELEMENTS, LAYERS, ToileState};

impl ToileState {
    // -- groups -----------------------------------------------------

    /// Bind `ids` into a fresh group element. Members already in another group
    /// are first removed from it (a node belongs to at most one group). The group
    /// `NodeId` is inserted into `layers` just below its lowest member so the
    /// members keep painting above it in their current relative order.
    pub(super) fn group_elements(&mut self, ids: &[ElementId]) -> ToileOutcome {
        let Some(id) = self.group_inner(ids) else {
            // Fewer than two distinct members: refuse loudly, not in silence
            // (`TOILE-055`).
            return ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::GroupNeedsTwo)]);
        };
        self.doc.commit();
        self.rematerialize_layers();
        // Seed the group's derived bounds from its members, then feed the R-tree.
        self.recompute_groups();
        self.emit_with_update(vec![
            ToileEvent::ElementCreated { id, correlate: 0 },
            self.viewport_event(),
        ])
    }

    /// Bind `ids` into a fresh group element **without** committing, returning the
    /// group id (or `None` when fewer than two distinct members survive filtering).
    /// The caller owns the `doc.commit()` so an import rebuilds each `<g>` inside
    /// the one import undo group (`toile-import-export`). Intermediate cache reads
    /// (`prune_empty_groups`, `rematerialize_layers`) are pure and stay here so the
    /// `insert_at` computation sees the detached state.
    pub(super) fn group_inner(&mut self, ids: &[ElementId]) -> Option<ElementId> {
        // Keep only existing, distinct members, in the request order.
        let mut members: Vec<ElementId> = Vec::new();
        for id in ids {
            if self.bounds.contains_key(id) && !members.contains(id) {
                members.push(id.clone());
            }
        }
        // Drop any member that is an ancestor of another member: selecting a group
        // together with one of its own descendants is contradictory. Keep the
        // descendants (the ancestor would otherwise be emptied by the detach below
        // and pruned mid-op, leaving a dangling child ref in the new group,).
        members = members
            .iter()
            .filter(|m| {
                let mut sub = HashSet::new();
                self.collect_subtree(m, &mut sub);
                sub.remove(*m);
                !members.iter().any(|o| o != *m && sub.contains(o))
            })
            .cloned()
            .collect();
        if members.len() < 2 {
            return None;
        }
        // Detach each member from its current group, then prune any group emptied
        // by the detach, so the incremental caches stay equal to a full rebuild.
        // Done before computing `insert_at` since pruning shifts `layers`.
        for m in &members {
            if let Some(g) = self.enclosing_group(m) {
                self.remove_child(&g, m);
            }
        }
        self.prune_empty_groups();
        self.rematerialize_layers();

        // Lowest layers index among the members (where the group node will sit).
        let insert_at = members
            .iter()
            .filter_map(|m| {
                self.layers
                    .iter()
                    .position(|n| n == &NodeId::Element(m.clone()))
            })
            .min()
            .unwrap_or(0);

        let id = self.mint_element_id();
        let elems = self.doc.get_map(ELEMENTS);
        let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
            return None;
        };
        em_seed_group(&em);
        let Ok(children) = em.insert_container(CHILDREN, LoroMovableList::new()) else {
            return None;
        };
        for m in &members {
            let _ = children.push(m.as_str());
        }
        // Insert the group NodeId just below its lowest member.
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .insert(insert_at, NodeId::Element(id.clone()).encode().as_str());
        Some(id)
    }

    /// Drop a group's binding: remove the group node and re-parent its children
    /// to the group's own parent (an enclosing group, or the board). The children
    /// survive with their `layers` order intact.
    pub(super) fn ungroup_elements(&mut self, group: &ElementId) -> ToileOutcome {
        let Some(gm) = self.element_map(group) else {
            return self.err_no_element(group);
        };
        if str_of(&gm, "type") != "group" {
            // An id that is not a group refuses loudly (`TOILE-159`).
            return ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NotAGroup(
                group.0.clone(),
            ))]);
        }
        let children = self.children_of(group);
        let parent = self.enclosing_group(group);

        // Remove the group element and its layers node.
        let _ = self.doc.get_map(ELEMENTS).delete(group.as_str());
        self.strip_layers(|node| !matches!(node, NodeId::Element(e) if e == group));

        // Re-parent the children to the enclosing group (board parent is implicit).
        // Drop the ungrouped group from the parent's `children`, else it lingers as
        // a dangling ref, desyncing recompute_groups from a full rebuild. Then
        // append the surviving children.
        if let Some(p) = &parent {
            self.remove_child(p, group);
            if let Some(list) = self.children_list(p) {
                for c in &children {
                    let _ = list.push(c.as_str());
                }
            }
        }
        self.doc.commit();

        self.bounds.remove(group);
        self.index.remove(group);
        self.rematerialize_layers();
        self.recompute_groups();
        self.emit_with_update(vec![self.viewport_event()])
    }

    // -- frames ---------------------------------------------------------

    /// Set a frame's membership: remove then add ids on the frame's `children`
    /// list, and enforce the frame/child z-order invariant. A node belongs
    /// to at most one container, so an added id is first detached from any group.
    /// One undo item.
    pub(super) fn set_frame_children(
        &mut self,
        frame: &ElementId,
        add: &[ElementId],
        remove: &[ElementId],
    ) -> ToileOutcome {
        let Some(fm) = self.element_map(frame) else {
            return self.err_no_element(frame);
        };
        if str_of(&fm, "type") != "frame" {
            return ToileOutcome::display_only(Vec::new());
        }
        let Some(list) = self.children_list(frame) else {
            return self.err_no_element(frame);
        };

        let mut changed = false;
        // Remove first, so a same-id remove+add re-appends at the end.
        for id in remove {
            if let LoroValue::List(items) = list.get_deep_value()
                && let Some(idx) = items
                    .iter()
                    .position(|v| matches!(v, LoroValue::String(s) if s.as_str() == id.as_str()))
            {
                let _ = list.delete(idx, 1);
                changed = true;
            }
        }
        // Add: skip the frame itself, unknown ids, and current members.
        let existing = self.children_of(frame);
        for id in add {
            if id == frame || !self.bounds.contains_key(id) || existing.contains(id) {
                continue;
            }
            if let Some(g) = self.enclosing_group(id) {
                self.remove_child(&g, id);
            }
            let _ = list.push(id.as_str());
            changed = true;
        }
        if !changed {
            return ToileOutcome::display_only(Vec::new());
        }
        // Prune any group emptied by moving its last child into the frame.
        self.prune_empty_groups();
        self.doc.commit();
        self.enforce_frame_zorder(frame);
        self.rematerialize_layers();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// The title a frame carries, empty until it is named. Test / integration
    /// accessor; every surface reads it off the display payload instead.
    pub fn frame_name(&self, frame: &ElementId) -> String {
        self.element_map(frame)
            .map(|em| str_of(&em, "name"))
            .unwrap_or_default()
    }

    /// Name a frame (`TOILE-247`). The title is one LWW register, so two users
    /// naming one frame converge on one of the two whole names rather than
    /// interleaving them (`TOILE-252`).
    ///
    /// The name is trimmed and cut to [`FRAME_NAME_MAX`] characters, so no
    /// title outgrows what every peer replicates (`TOILE-249`). A name that
    /// did not change writes nothing (`TOILE-250`), and an element that is not
    /// a frame is refused (`TOILE-251`). One commit, so one undo item
    /// (`TOILE-248`).
    pub(super) fn rename_frame(&mut self, frame: &ElementId, name: &str) -> ToileOutcome {
        let Some(fm) = self.element_map(frame) else {
            return self.err_no_element(frame);
        };
        if str_of(&fm, "type") != "frame" {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        let name: String = name.trim().chars().take(FRAME_NAME_MAX).collect();
        if str_of(&fm, "name") == name {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = fm.insert("name", name.as_str());
        self.doc.commit();
        // The deck panel reads a frame by its title, and holds no copy of its
        // own (`TOILE-UI-343`).
        self.emit_with_update(vec![self.viewport_event(), ToileEvent::DeckChanged])
    }

    /// Clamp every child of `frame` to paint after (above) the frame in
    /// `layers`. A child sitting below the frame is moved to just above it, and
    /// this repeats until the invariant holds. Commits the reordering as part of
    /// the batch.
    fn enforce_frame_zorder(&mut self, frame: &ElementId) {
        let children = self.children_of(frame);
        if children.is_empty() {
            return;
        }
        let list = self.doc.get_movable_list(LAYERS);
        let mut moved = false;
        loop {
            self.rematerialize_layers();
            let order = &self.layers;
            let Some(fidx) = order
                .iter()
                .position(|n| matches!(n, NodeId::Element(e) if e == frame))
            else {
                break;
            };
            let below = children.iter().find_map(|c| {
                order
                    .iter()
                    .position(|n| matches!(n, NodeId::Element(e) if e == c))
                    .filter(|ci| *ci < fidx)
            });
            let Some(ci) = below else { break };
            // Removing ci (< fidx) shifts the frame down to fidx-1, so moving the
            // child to fidx lands it just above the frame.
            let _ = list.mov(ci, fidx);
            moved = true;
        }
        if moved {
            self.doc.commit();
        }
    }

    // -- tree helpers ----------------------------------------------------------

    /// The `children` movable-list handle of a group / frame element, if present.
    fn children_list(&self, id: &ElementId) -> Option<LoroMovableList> {
        self.element_map(id)?
            .get(CHILDREN)?
            .into_container()
            .ok()?
            .into_movable_list()
            .ok()
    }

    /// Member element ids of a group / frame, in `children` order.
    pub(super) fn children_of(&self, id: &ElementId) -> Vec<ElementId> {
        let mut out = Vec::new();
        if let Some(list) = self.children_list(id)
            && let LoroValue::List(items) = list.get_deep_value()
        {
            for v in items.iter() {
                if let LoroValue::String(s) = v {
                    out.push(ElementId(s.to_string()));
                }
            }
        }
        out
    }

    /// The group whose `children` contains `id`, if any (a node belongs to at
    /// most one group,).
    pub(super) fn enclosing_group(&self, id: &ElementId) -> Option<ElementId> {
        let elems = self.doc.get_map(ELEMENTS);
        let LoroValue::Map(m) = elems.get_deep_value() else {
            return None;
        };
        for (key, v) in m.iter() {
            if let LoroValue::Map(fields) = v
                && matches!(fields.get("type"), Some(LoroValue::String(t)) if t.as_str() == "group")
                && let Some(LoroValue::List(kids)) = fields.get(CHILDREN)
                && kids
                    .iter()
                    .any(|k| matches!(k, LoroValue::String(s) if s.as_str() == id.as_str()))
            {
                return Some(ElementId(key.clone()));
            }
        }
        None
    }

    /// Remove `child` from `group`'s `children` list (first match).
    pub(super) fn remove_child(&self, group: &ElementId, child: &ElementId) {
        let Some(list) = self.children_list(group) else {
            return;
        };
        if let LoroValue::List(items) = list.get_deep_value()
            && let Some(idx) = items
                .iter()
                .position(|v| matches!(v, LoroValue::String(s) if s.as_str() == child.as_str()))
        {
            let _ = list.delete(idx, 1);
        }
    }

    /// Append `id`'s subtree leaves (non-group descendants) to `out`. A plain
    /// element contributes itself; a group recurses into its children.
    pub(super) fn collect_leaves(&self, id: &ElementId, out: &mut HashSet<ElementId>) {
        if self.is_group(id) {
            for c in self.children_of(id) {
                self.collect_leaves(&c, out);
            }
        } else {
            out.insert(id.clone());
        }
    }

    /// Append `id` and every descendant (groups included) to `out`.
    pub(super) fn collect_subtree(&self, id: &ElementId, out: &mut HashSet<ElementId>) {
        if !out.insert(id.clone()) {
            return;
        }
        if self.is_group(id) {
            for c in self.children_of(id) {
                self.collect_subtree(&c, out);
            }
        }
    }

    fn is_group(&self, id: &ElementId) -> bool {
        self.element_map(id)
            .map(|em| str_of(&em, "type") == "group")
            .unwrap_or(false)
    }

    /// Delete every childless group to a fixpoint: the element, its layers node
    /// and its derived bounds. Pruning a group that was the last member of its
    /// parent cascades upward. An empty group is meaningless (created with >=2
    /// members, only shrinks via re-parent or member deletion). Left behind, it
    /// desyncs the incremental caches from a full rebuild, which never
    /// materializes one: a rebuild's `recompute_groups` skips a childless group.
    /// Runs inside the caller's single `doc.commit()` so it stays one undo item.
    pub(super) fn prune_empty_groups(&mut self) {
        loop {
            let empties: Vec<ElementId> = {
                let mut v = Vec::new();
                if let LoroValue::Map(m) = self.doc.get_map(ELEMENTS).get_deep_value() {
                    for (key, val) in m.iter() {
                        if matches!(val, LoroValue::Map(f)
                            if matches!(f.get("type"), Some(LoroValue::String(t)) if t.as_str() == "group"))
                        {
                            let id = ElementId(key.clone());
                            if self.children_of(&id).is_empty() {
                                v.push(id);
                            }
                        }
                    }
                }
                v
            };
            if empties.is_empty() {
                break;
            }
            let removed: HashSet<ElementId> = empties.iter().cloned().collect();
            let elems = self.doc.get_map(ELEMENTS);
            for id in &empties {
                let _ = elems.delete(id.as_str());
                self.bounds.remove(id);
                self.index.remove(id);
                if let Some(g) = self.enclosing_group(id) {
                    self.remove_child(&g, id);
                }
            }
            self.strip_layers(|node| !matches!(node, NodeId::Element(e) if removed.contains(e)));
        }
    }

    /// Recompute every group's derived bounds as the union of its members', to a
    /// fixpoint (nested groups resolve bottom-up). Group geometry is derived and
    /// lives only in the `bounds` / R-tree caches, never in the CRDT.
    pub(super) fn recompute_groups(&mut self) {
        let groups: Vec<(ElementId, Vec<ElementId>)> = {
            let mut v = Vec::new();
            if let LoroValue::Map(m) = self.doc.get_map(ELEMENTS).get_deep_value() {
                for (key, val) in m.iter() {
                    if let LoroValue::Map(fields) = val
                        && matches!(fields.get("type"), Some(LoroValue::String(t)) if t.as_str() == "group")
                    {
                        let id = ElementId(key.clone());
                        let kids = self.children_of(&id);
                        v.push((id, kids));
                    }
                }
            }
            v
        };
        let mut changed = true;
        while changed {
            changed = false;
            for (g, kids) in &groups {
                let mut union: Option<Rect> = None;
                let mut all = !kids.is_empty();
                for k in kids {
                    match self.bounds.get(k) {
                        Some(r) => {
                            union = Some(match union {
                                Some(u) => u.union(*r),
                                None => *r,
                            })
                        }
                        None => all = false,
                    }
                }
                if let (true, Some(rect)) = (all, union)
                    && self.bounds.get(g) != Some(&rect)
                {
                    self.bounds.insert(g.clone(), rect);
                    self.index.update(ElementBounds::from_rect(g.clone(), rect));
                    changed = true;
                }
            }
        }
    }
}
