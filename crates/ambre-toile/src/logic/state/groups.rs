//! Grouping, frame membership and the group/frame tree helpers for
//! [`ToileState`](super::ToileState). Split out of
//! `state/mod.rs`. A node belongs to at most one container; group geometry is
//! derived (never in the CRDT) and lives only in the `bounds` / R-tree caches.

use std::collections::HashSet;

use kurbo::Rect;
use loro::{LoroMap, LoroMovableList, LoroValue};

use crate::logic::command::{ToileError, ToileEvent, ToileOutcome};
use crate::logic::index::ElementBounds;
use crate::logic::model::{ElementId, ElementKind, NodeId};
use ambre_bus::app::toile::FRAME_NAME_MAX;

use super::ToileState;
use super::codec::{em_seed_group, str_of};
use super::codec::{has_type, value_has_type};
use super::names::{ElementKey, FieldValues, Fields, Root};

impl ToileState {
    // -- groups -----------------------------------------------------

    /// Bind `ids` into a fresh group element. Members already in another group
    /// are first removed from it (a node belongs to at most one group). The group
    /// `NodeId` is inserted into `layers` just below its lowest member so the
    /// members keep painting above it in their current relative order.
    pub(super) fn group_elements(&mut self, ids: &[ElementId]) -> ToileOutcome {
        // A refusal is loud, never silent (`TOILE-055`): fewer than two
        // distinct members, or no fresh id.
        let id = match self.group_inner(ids) {
            Ok(id) => id,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        self.doc.commit();
        self.rematerialize_layers();
        // Seed the group's derived bounds from its members, then feed the R-tree.
        self.recompute_groups();
        self.emit_with_update(vec![
            ToileEvent::ElementCreated {
                id,
                correlate: None,
            },
            self.viewport_event(),
        ])
    }

    /// Bind `ids` into a fresh group element **without** committing, returning the
    /// group id. The refusal is [`ToileError::GroupNeedsTwo`] when fewer than two
    /// distinct members survive filtering, and [`ToileError::IdsExhausted`] when
    /// the session has no fresh id.
    /// The caller owns the `doc.commit()` so an import rebuilds each `<g>` inside
    /// the one import undo group (`toile-import-export`). Intermediate cache reads
    /// (`prune_empty_groups`, `rematerialize_layers`) are pure and stay here so the
    /// `insert_at` computation sees the detached state.
    pub(super) fn group_inner(&mut self, ids: &[ElementId]) -> Result<ElementId, ToileError> {
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
            return Err(ToileError::GroupNeedsTwo);
        }
        // Mint before the first write: a spent counter refuses the group with
        // the board untouched, so no detach rides along with the next commit.
        let id = self.mint_element_id()?;
        // A member leaves every container it sat in, a frame included, since
        // it belongs to one at most (`TOILE-149`). When every member sat in one
        // frame, the new group takes their place in it.
        let frame_of = |m: &ElementId| {
            self.parents_of(m)
                .iter()
                .find(|p| self.is_frame(p))
                .cloned()
        };
        let shared_frame = members
            .first()
            .and_then(frame_of)
            .filter(|f| members.iter().all(|m| frame_of(m).as_ref() == Some(f)));
        // Detach each member, then prune any group emptied by the detach, so the
        // incremental caches stay equal to a full rebuild. Done before computing
        // `insert_at` since pruning shifts `layers`.
        for m in &members {
            for p in self.parents_of(m).to_vec() {
                self.remove_child(&p, m);
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

        let refused = || ToileError::WriteFailed("the board refused the new group".to_owned());
        let Ok(em) = self
            .doc
            .get_map(Root::Elements.as_str())
            .insert_container(id.as_str(), LoroMap::new())
            .map(Fields::<ElementKey>::of)
        else {
            return Err(refused());
        };
        em_seed_group(&em);
        let Ok(children) = em.insert_container(ElementKey::Children, LoroMovableList::new()) else {
            return Err(refused());
        };
        for m in &members {
            if children.push(m.as_str()).is_ok() {
                self.link_parent(m, &id);
            }
        }
        // Insert the group NodeId just below its lowest member.
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .insert(insert_at, NodeId::Element(id.clone()).encode().as_str());
        if let Some(f) = shared_frame {
            if let Some(list) = self.children_list(&f)
                && list.push(id.as_str()).is_ok()
            {
                self.link_parent(&id, &f);
            }
            self.enforce_frame_zorder(&f);
        }
        Ok(id)
    }

    /// Drop each group's binding: remove the group node and re-parent its
    /// children to the group's own parent (an enclosing group, or the board).
    /// The children survive with their `layers` order intact.
    ///
    /// The list is one gesture, so every unbind lands in one commit and one
    /// undo restores them all (`TOILE-094`). Each unbind reads the parent cache
    /// the previous one updated, so a group listed with its own enclosing group
    /// unbinds in either order.
    ///
    /// An id that is not a group changes nothing (`TOILE-159`). Beside a group
    /// it is answered with nothing, since the surface sends the whole selection
    /// and the user asked to ungroup what can be (`TOILE-264`). A list with no
    /// group answers one error, naming its first id. An id the board does not
    /// hold always answers its own error (`TOILE-160`).
    pub(super) fn ungroup_elements(&mut self, groups: &[ElementId]) -> ToileOutcome {
        let mut display = Vec::new();
        let mut unbound = Vec::new();
        let mut not_a_group = None;
        let mut seen = HashSet::new();
        for group in groups {
            if !seen.insert(group) {
                continue;
            }
            match self.element_map(group) {
                None => display.push(ToileEvent::Error(ToileError::NoSuchElement(group.clone()))),
                Some(gm) if !has_type(&gm, ElementKind::Group) => {
                    not_a_group = not_a_group.or(Some(group));
                }
                Some(_) => {
                    self.unbind_group(group);
                    unbound.push(group);
                }
            }
        }
        if unbound.is_empty() {
            if let Some(id) = not_a_group {
                display.push(ToileEvent::Error(ToileError::NotAGroup(id.clone())));
            }
            return ToileOutcome::display_only(display);
        }
        // One rewrite of the paint order for the whole list, not one per group.
        let gone: HashSet<&ElementId> = unbound.iter().copied().collect();
        self.strip_layers(|node| !matches!(node, NodeId::Element(e) if gone.contains(e)));
        self.doc.commit();

        for group in unbound {
            self.bounds.remove(group);
            self.index.remove(group);
        }
        self.rematerialize_layers();
        self.recompute_groups();
        display.push(self.viewport_event());
        self.emit_with_update(display)
    }

    /// Write one group's unbind into the open transaction. The caller has
    /// checked that `group` is a group, and owns the paint order, the commit
    /// and the caches.
    fn unbind_group(&mut self, group: &ElementId) {
        let children = self.children_of(group);
        let parent = self.enclosing_group(group);

        // Remove the group element; the caller strips its layers node.
        let _ = self
            .doc
            .get_map(Root::Elements.as_str())
            .delete(group.as_str());

        // Re-parent the children to the enclosing group (board parent is implicit).
        // Drop the ungrouped group from the parent's `children`, else it lingers as
        // a dangling ref, desyncing recompute_groups from a full rebuild. Then
        // append the surviving children.
        if let Some(p) = &parent {
            self.remove_child(p, group);
            if let Some(list) = self.children_list(p) {
                for c in &children {
                    if list.push(c.as_str()).is_ok() {
                        self.link_parent(c, p);
                    }
                }
            }
        }
        for c in &children {
            self.unlink_parent(c, group);
        }
        self.forget_parents(group);
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
        // Refused aloud, as a rename of a non-frame is, so a surface can tell
        // the refusal from a gesture that changed nothing (`TOILE-050`).
        if !has_type(&fm, ElementKind::Frame) {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
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
                if list.delete(idx, 1).is_ok() {
                    self.unlink_parent(id, frame);
                }
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
            if list.push(id.as_str()).is_ok() {
                self.link_parent(id, frame);
            }
            changed = true;
        }
        if !changed {
            return ToileOutcome::display_only(Vec::new());
        }
        // Prune any group emptied by moving its last child into the frame.
        self.prune_empty_groups();
        // The clamp joins the same commit: membership and order undo together
        // (`TOILE-048`).
        self.enforce_frame_zorder(frame);
        self.doc.commit();
        self.rematerialize_layers();
        // A member taken from its group leaves that group's box (`TOILE-009`).
        self.recompute_groups();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Take each of `ids` out of every frame whose membership list holds it
    /// (`TOILE-UI-149`). Membership is read here, never inferred from a box: an
    /// element dragged out of its frame's box is still that frame's member. A
    /// concurrent add can leave one id in two lists, so every list is swept.
    /// Nothing held writes nothing. One commit, so one undo item.
    pub(super) fn remove_from_frames(&mut self, ids: &[ElementId]) -> ToileOutcome {
        let gone: HashSet<&str> = ids.iter().map(ElementId::as_str).collect();
        let mut changed = false;
        for frame in &self.frame_ids() {
            let Some(list) = self.children_list(frame) else {
                continue;
            };
            let LoroValue::List(items) = list.get_deep_value() else {
                continue;
            };
            // Back to front, so a delete never shifts an index still to visit.
            for (idx, v) in items.iter().enumerate().rev() {
                if let LoroValue::String(s) = v
                    && gone.contains(s.as_str())
                {
                    if list.delete(idx, 1).is_ok() {
                        self.unlink_parent(&ElementId(s.to_string()), frame);
                    }
                    changed = true;
                }
            }
        }
        if !changed {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Every frame in paint order.
    fn frame_ids(&self) -> Vec<ElementId> {
        self.layers
            .iter()
            .filter_map(|node| match node {
                NodeId::Element(id) if self.is_frame(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// The ids of `ids` that some frame's children list holds, in the order
    /// given. Membership is the list, never the box (`TOILE-UI-384`). Read from
    /// the parent cache, so the cost follows `ids`, not the board (`TOILE-056`).
    pub(super) fn framed_among(&self, ids: &[ElementId]) -> Vec<ElementId> {
        ids.iter()
            .filter(|id| self.parents_of(id).iter().any(|p| self.is_frame(p)))
            .cloned()
            .collect()
    }

    /// The title a frame carries, `None` until it is named or for no frame.
    /// Test / integration accessor; every surface reads it off the display
    /// payload instead.
    pub fn frame_name(&self, frame: &ElementId) -> Option<String> {
        self.element_map(frame)
            .and_then(|em| str_of(&em, ElementKey::Name))
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
        if !has_type(&fm, ElementKind::Frame) {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        let name: String = name
            .trim()
            .chars()
            .take(FRAME_NAME_MAX.to_usize())
            .collect();
        if str_of(&fm, ElementKey::Name).unwrap_or_default() == name {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = fm.insert(ElementKey::Name, name.as_str());
        self.doc.commit();
        // The deck panel reads a frame by its title, and holds no copy of its
        // own (`TOILE-UI-343`).
        self.emit_with_update(vec![self.viewport_event(), ToileEvent::DeckChanged])
    }

    /// Clamp every child of `frame` to paint after (above) the frame in
    /// `layers`. A child sitting below the frame is moved to just above it, and
    /// this repeats until the invariant holds. The caller commits, so the
    /// reordering joins its batch, and rematerializes `layers`. The raw list
    /// already holds the caller's uncommitted writes.
    ///
    /// Indices are read from the raw list, never from the materialized order.
    /// Concurrent rewrites leave duplicate and unreadable entries the
    /// materialized order drops, so its indices name other raw slots. Moving
    /// by them once moved the wrong node forever. Each move here takes one child
    /// entry from below the frame's first entry, so the loop ends within the
    /// list's length.
    fn enforce_frame_zorder(&self, frame: &ElementId) {
        let children: HashSet<ElementId> = self.children_of(frame).into_iter().collect();
        if children.is_empty() {
            return;
        }
        let list = self.doc.get_movable_list(Root::Layers.as_str());
        for _ in 0..=list.len() {
            let LoroValue::List(raw) = list.get_deep_value() else {
                break;
            };
            // The frame paints at its first entry; any child entry before it
            // paints below it.
            let mut below = None;
            let mut fidx = None;
            for (i, v) in raw.iter().enumerate() {
                let LoroValue::String(s) = v else {
                    continue;
                };
                match NodeId::decode(s) {
                    Some(NodeId::Element(e)) if &e == frame => {
                        fidx = Some(i);
                        break;
                    }
                    Some(NodeId::Element(e)) if below.is_none() && children.contains(&e) => {
                        below = Some(i);
                    }
                    _ => {}
                }
            }
            let (Some(fidx), Some(ci)) = (fidx, below) else {
                break;
            };
            // Removing ci (< fidx) shifts the frame down to fidx-1, so moving the
            // child to fidx lands it just above the frame.
            if list.mov(ci, fidx).is_err() {
                break;
            }
        }
    }

    // -- tree helpers ----------------------------------------------------------

    /// The `children` movable-list handle of a group / frame element, if present.
    fn children_list(&self, id: &ElementId) -> Option<LoroMovableList> {
        self.element_map(id)?
            .get(ElementKey::Children)?
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

    /// The group whose `children` contains `id`, if any. A node belongs to at
    /// most one group; concurrent binds can break that, and the smallest id
    /// wins. The cache holds parents in arrival order, which differs per peer,
    /// so the first entry would not converge (`TOILE-266`). Read from the
    /// parent cache, never from a walk of the board (`TOILE-056`).
    pub(super) fn enclosing_group(&self, id: &ElementId) -> Option<ElementId> {
        self.parents_of(id)
            .iter()
            .filter(|p| self.is_group(p))
            .min_by(|a, b| a.as_str().cmp(b.as_str()))
            .cloned()
    }

    /// Remove `child` from `group`'s `children` list (first match). The cache
    /// forgets the link even when `group` is gone, as a cascading delete leaves it.
    pub(super) fn remove_child(&mut self, group: &ElementId, child: &ElementId) {
        self.unlink_parent(child, group);
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
            .map(|em| has_type(&em, ElementKind::Group))
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
                if let LoroValue::Map(m) = self.walk_elements() {
                    for (key, val) in m.iter() {
                        if matches!(val, LoroValue::Map(f)
                            if value_has_type(FieldValues::of(f), ElementKind::Group))
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
            let elems = self.doc.get_map(Root::Elements.as_str());
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
            if let LoroValue::Map(m) = self.walk_elements() {
                for (key, val) in m.iter() {
                    if let LoroValue::Map(fields) = val
                        && value_has_type(FieldValues::of(fields), ElementKind::Group)
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

#[cfg(test)]
mod tests {
    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent, ToileOutcome};
    use crate::logic::model::{ElementId, ElementKind, NodeId, ShapeKind};

    use super::super::{Root, ToileState};

    fn create(st: &mut ToileState, kind: ElementKind) -> ElementId {
        let out: ToileOutcome = st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("a create names its element")
    }

    /// `TOILE-011`: the clamp ends on a paint order holding a duplicate entry,
    /// as two concurrent rewrites leave one. It once moved by the materialized
    /// order's indices, which the duplicate shifts, and looped forever.
    #[test]
    fn the_frame_clamp_ends_on_a_duplicate_entry() {
        let mut st = ToileState::new();
        let r = create(&mut st, ElementKind::Shape(ShapeKind::Rect));
        let f = create(&mut st, ElementKind::Frame);
        let tag = NodeId::Element(r.clone()).encode();
        let _ = st
            .doc
            .get_movable_list(Root::Layers.as_str())
            .insert(0, tag.as_str());
        st.doc.commit();
        st.rematerialize_layers();

        st.handle_command(ToileCommand::SetFrameChildren {
            frame: f.clone(),
            add: vec![r.clone()],
            remove: vec![],
        });
        let at = |id: &ElementId| {
            st.layer_order()
                .iter()
                .position(|n| n == &NodeId::Element(id.clone()))
        };
        assert!(at(&f) < at(&r), "the child paints below its frame");
    }
}
