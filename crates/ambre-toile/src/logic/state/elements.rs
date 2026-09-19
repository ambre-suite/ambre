//! Element / stroke lifecycle for [`ToileState`](super::ToileState):
//! create, move, resize, rotate, restyle, lock, delete, z-order reorder and
//! freehand stroke commit. Split out of `state/mod.rs`; every method here is a
//! `handle_command` target and commits exactly one undo item.

use std::collections::HashSet;

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use crate::logic::align::{AlignOp, align_deltas};
use crate::logic::command::{InitContent, ToileError, ToileEvent, ToileOutcome, ZTarget};
use crate::logic::index::ElementBounds;
use crate::logic::model::{
    ElementId, ElementKind, ElementStyle, LinkTarget, NodeId, ShapeKind, StrokePoint,
};

use super::codec::{bin_of, bool_of, content_matches, expanded_aabb, f64_of, stroke_aabb};
use super::{CHILDREN, CONNECTORS, ELEMENTS, LAYERS, NOTES, ToileState};

impl ToileState {
    pub(super) fn create_element(
        &mut self,
        kind: ElementKind,
        at: Point,
        size: kurbo::Size,
        content: InitContent,
        fill: Option<u32>,
    ) -> ToileOutcome {
        let id = match self.create_element_inner(kind, at, size, content, fill) {
            Ok(id) => id,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::ElementCreated(id), self.viewport_event()])
    }

    /// Create one element and update the incremental caches, **without**
    /// committing. The caller owns the `doc.commit()`, so an import can
    /// land N creates in a single undo group (`toile-import-export`). Returns the
    /// minted id, or the rejection reason as a plain [`ToileError`] (validation
    /// is identical to [`create_element`](Self::create_element)).
    pub(super) fn create_element_inner(
        &mut self,
        kind: ElementKind,
        at: Point,
        size: kurbo::Size,
        content: InitContent,
        fill: Option<u32>,
    ) -> Result<ElementId, ToileError> {
        // Group and Drawing are not creatable here: a group is born via
        // GroupElements, a stroke via CommitStroke.
        if matches!(kind, ElementKind::Group | ElementKind::Drawing) {
            return Err(ToileError::NotCreatable {
                kind: kind.type_tag(),
            });
        }
        if !content_matches(kind, &content) {
            return Err(ToileError::ContentKindMismatch);
        }

        let id = self.mint_element_id();
        let elems = self.doc.get_map(ELEMENTS);
        let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
            return Err(ToileError::NoSuchElement(id.0));
        };
        let _ = em.insert("type", kind.type_tag());
        let _ = em.insert("x", at.x);
        let _ = em.insert("y", at.y);
        let _ = em.insert("w", size.width);
        let _ = em.insert("h", size.height);
        let _ = em.insert("rot", 0.0);
        let _ = em.insert("locked", false);
        // Seed a per-kind default style (sticky = yellow card, others = white),
        // then apply an optional creation-time fill override.
        let mut style = ElementStyle::for_kind(kind);
        if let Some(f) = fill {
            style.fill = Some(f);
        }
        let _ = em.insert("style", LoroValue::Binary(style.to_bytes().into()));
        self.seed_payload(&em, kind, &content);
        // Push onto the top of the z-order.
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .push(NodeId::Element(id.clone()).encode().as_str());

        let rect = expanded_aabb(at.x, at.y, size.width, size.height, 0.0);
        self.bounds.insert(id.clone(), rect);
        self.index
            .insert(ElementBounds::from_rect(id.clone(), rect));
        self.layers.push(NodeId::Element(id.clone()));
        Ok(id)
    }

    /// Seed the per-type payload fields. Kept minimal for Phase 0.
    fn seed_payload(&self, em: &LoroMap, kind: ElementKind, content: &InitContent) {
        match (kind, content) {
            (ElementKind::Shape(sk), _) => {
                let _ = em.insert("shape", sk.tag());
                if let (ShapeKind::Path, InitContent::Path(p)) = (sk, content) {
                    let _ = em.insert("path", p.to_svg());
                }
            }
            (ElementKind::Image, InitContent::Image(hash)) => {
                let _ = em.insert("hash", hash.as_str());
            }
            (ElementKind::Embed, InitContent::Embed(url)) => {
                let _ = em.insert("url", url.as_str());
            }
            (
                ElementKind::DocEmbed,
                InitContent::DocEmbed {
                    document_id,
                    doc_type,
                },
            ) => {
                let _ = em.insert("doc_id", document_id.to_string());
                let _ = em.insert("doc_type", format!("{doc_type:?}"));
            }
            (ElementKind::Sticky | ElementKind::Text, InitContent::Text(t)) => {
                let _ = em.insert("text", t.as_str());
            }
            (ElementKind::Sticky | ElementKind::Text, InitContent::None) => {
                let _ = em.insert("text", "");
            }
            (ElementKind::Frame, _) => {
                // Frame content: an empty membership list plus an empty
                // speaker-notes text at birth. Notes MUST be seeded here, not
                // lazily on first edit. Seeding at creation gives every peer the
                // same shared LoroText container (broadcast in the creation delta)
                // before any edit, so concurrent notes edits converge. A lazy
                // first edit on each peer would mint two rival containers under
                // the same map key -> LWW collision, one side's notes lost.
                // Autolayout stays lazy (absent = free positioning).
                let _ = em.insert_container(CHILDREN, LoroMovableList::new());
                let _ = em.insert_container(NOTES, LoroText::new());
            }
            _ => {}
        }
    }

    /// Overwrite the plaintext body of a sticky / text element. Phase 0
    /// stores the body as a single LWW string field, so an edit is a full-body
    /// `insert` on the `"text"` key. Only sticky / text elements carry that field;
    /// a request against any other kind (or a missing element) is a rejected no-op.
    pub(super) fn set_text(&mut self, id: &ElementId, text: &str) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if bool_of(&em, "locked") {
            return ToileOutcome::display_only(Vec::new());
        }
        // The `"text"` field exists only on sticky / text elements (seeded at
        // creation). Reject a SetText aimed at any other kind.
        if em.get("text").is_none() {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        let _ = em.insert("text", text);
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Set or clear one element's link register (`toile-links`). The register is
    /// its own field, never part of the style. A style is written whole. A link
    /// folded into it would be erased by any restyle of a selection the element
    /// happens to be in (`TOILE-062`). A locked element refuses the write like
    /// it refuses every other edit.
    pub(super) fn set_link(&mut self, id: &ElementId, link: Option<LinkTarget>) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if bool_of(&em, "locked") {
            return ToileOutcome::display_only(Vec::new());
        }
        match link {
            Some(l) => {
                let _ = em.insert("link", LoroValue::Binary(l.to_bytes().into()));
            }
            // Absence is the value (`TOILE-230`): a cleared link is a missing
            // key, not a key holding an empty blob.
            None => {
                let _ = em.delete("link");
            }
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn move_elements(&mut self, ids: &[ElementId], delta: Vec2) -> ToileOutcome {
        // A group transforms as one: expand every selected group to its leaf
        // descendants so translating a group moves its whole subtree.
        let mut leaves = HashSet::new();
        for id in ids {
            self.collect_leaves(id, &mut leaves);
        }
        let mut touched = false;
        for id in &leaves {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if bool_of(&em, "locked") {
                continue; // locked blocks edit locally (advisory)
            }
            let x = f64_of(&em, "x") + delta.x;
            let y = f64_of(&em, "y") + delta.y;
            let _ = em.insert("x", x);
            let _ = em.insert("y", y);
            self.update_bounds(id, &em);
            touched = true;
        }
        if !touched {
            return ToileOutcome::display_only(Vec::new());
        }
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Align or distribute the named nodes, as one batch and one undo item
    /// (`TOILE-037`).
    ///
    /// The boxes are the derived, rotation-expanded ones the picker and the
    /// selection box already use. A rotated element then aligns by what the user
    /// sees rather than by an edge hidden under its own tilt. A group is one box
    /// and its whole subtree takes the group's translation, which is what
    /// `move_elements` does for a drag. A locked element is dropped before the
    /// union is measured. It cannot move, and letting it set the reference edge
    /// would let a locked background swallow the whole operation (`TOILE-234`).
    pub(super) fn align_elements(&mut self, ids: &[ElementId], op: AlignOp) -> ToileOutcome {
        // Selection order, deduplicated: an id repeated in the selection would
        // otherwise weigh twice in a distribution.
        let mut seen = HashSet::new();
        let mut movable = Vec::new();
        for id in ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(bounds) = self.bounds.get(id).copied() else {
                continue; // an id nobody minted, or a stale selection
            };
            if self
                .element_map(id)
                .is_some_and(|em| bool_of(&em, "locked"))
            {
                continue;
            }
            movable.push((id.clone(), bounds));
        }
        let boxes: Vec<Rect> = movable.iter().map(|(_, b)| *b).collect();
        let deltas = align_deltas(&boxes, op);

        let mut touched = false;
        for ((id, _), delta) in movable.iter().zip(deltas) {
            if delta == Vec2::ZERO {
                continue;
            }
            let mut leaves = HashSet::new();
            self.collect_leaves(id, &mut leaves);
            for leaf in &leaves {
                let Some(em) = self.element_map(leaf) else {
                    continue;
                };
                if bool_of(&em, "locked") {
                    continue; // a locked leaf inside a moving group stays put
                }
                let _ = em.insert("x", f64_of(&em, "x") + delta.x);
                let _ = em.insert("y", f64_of(&em, "y") + delta.y);
                self.update_bounds(leaf, &em);
                touched = true;
            }
        }
        if !touched {
            return ToileOutcome::display_only(Vec::new());
        }
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn resize_element(&mut self, id: &ElementId, new: Rect) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if bool_of(&em, "locked") {
            return ToileOutcome::display_only(Vec::new());
        }
        let r = new.abs();
        let _ = em.insert("x", r.x0);
        let _ = em.insert("y", r.y0);
        let _ = em.insert("w", r.width());
        let _ = em.insert("h", r.height());
        self.update_bounds(id, &em);
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn rotate_element(&mut self, id: &ElementId, radians: f64) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if bool_of(&em, "locked") {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = em.insert("rot", radians);
        self.update_bounds(id, &em);
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn restyle_elements(
        &mut self,
        ids: &[ElementId],
        style: ElementStyle,
    ) -> ToileOutcome {
        if !self.restyle_merge_inner(ids, &style) {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Overwrite the shown style fields of every unlocked element in `ids`,
    /// **without** committing. Returns whether anything changed. The caller owns
    /// the `doc.commit()`. The surface's whole-selection restyle uses this; the
    /// import path uses [`restyle_inner`](Self::restyle_inner), which overwrites
    /// whole. Each element keeps its own corner radius, shadow and non-bold
    /// weight (`TOILE-UI-174`).
    pub(super) fn restyle_merge_inner(&mut self, ids: &[ElementId], style: &ElementStyle) -> bool {
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if bool_of(&em, "locked") {
                continue;
            }
            let existing = self.style_read(&bin_of(&em, "style"));
            let merged = existing.merged_over(style);
            let _ = em.insert("style", LoroValue::Binary(merged.to_bytes().into()));
            touched = true;
        }
        touched
    }

    /// Overwrite the style register of every unlocked element in `ids`, **without**
    /// committing. Returns whether anything changed. The caller owns the
    /// `doc.commit()`, so an import can restyle a fresh element inside the same
    /// undo group as its `CreateElement`. Style is an independent LWW register that
    /// cannot ride the create.
    pub(super) fn restyle_inner(&mut self, ids: &[ElementId], style: &ElementStyle) -> bool {
        let bytes = style.to_bytes();
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if bool_of(&em, "locked") {
                continue;
            }
            let _ = em.insert("style", LoroValue::Binary(bytes.clone().into()));
            touched = true;
        }
        touched
    }

    pub(super) fn lock_elements(&mut self, ids: &[ElementId], locked: bool) -> ToileOutcome {
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            let _ = em.insert("locked", locked);
            touched = true;
        }
        if !touched {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn delete_elements(&mut self, ids: &[ElementId]) -> ToileOutcome {
        let deck_before = self.slide_order();
        if !self.delete_from_doc(ids) {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();

        self.rematerialize_layers();
        self.recompute_groups();
        let mut events = vec![self.viewport_event()];
        events.extend(self.deck_change(&deck_before));
        self.emit_with_update(events)
    }

    /// Remove `ids` (and, for groups, their whole subtree) from the CRDT and the
    /// incremental caches, without committing. Drops group membership, orphaned
    /// connectors and layer entries, and prunes emptied groups (`toile-elements`).
    /// Returns whether anything was removed. The caller owns the `doc.commit()`
    /// so a deletion shares one commit and one undo item with a sibling
    /// mutation: a boolean op consuming its sources.
    pub(super) fn delete_from_doc(&mut self, ids: &[ElementId]) -> bool {
        // Deleting a group cascades to its members: expand the request
        // to the full subtree of every listed group before removing anything.
        let mut expanded: HashSet<ElementId> = HashSet::new();
        for id in ids {
            self.collect_subtree(id, &mut expanded);
        }
        let del: HashSet<&ElementId> = expanded.iter().collect();
        let mut removed_any = false;
        let elems = self.doc.get_map(ELEMENTS);
        for id in &expanded {
            if elems.get(id.as_str()).is_some() {
                let _ = elems.delete(id.as_str());
                removed_any = true;
            }
        }
        if !removed_any {
            return false;
        }
        // Drop any group membership referencing a deleted node.
        for id in &expanded {
            if let Some(g) = self.enclosing_group(id) {
                self.remove_child(&g, id);
            }
        }
        // Drop locally-owned connectors whose endpoint was deleted and
        // strip both elements and orphaned connectors from `layers`.
        let orphan_conns = self.connectors_touching(&del);
        let conns = self.doc.get_map(CONNECTORS);
        for cid in &orphan_conns {
            let _ = conns.delete(cid.as_str());
        }
        self.strip_layers(|node| match node {
            NodeId::Element(e) => !del.contains(e),
            NodeId::Connector(c) => !orphan_conns.contains(c),
        });
        // A deleted frame leaves the deck in the same write, so the two never
        // disagree (`TOILE-FEAT-027`). A dead entry would be counted in the deck
        // length and stop the camera when a viewer reached it. The PDF export
        // already skips it.
        self.drop_slides(&del);
        for id in &expanded {
            self.bounds.remove(id);
            self.index.remove(id);
        }
        // A group whose members were all deleted is now empty and meaningless;
        // prune it so the incremental caches match a full rebuild.
        self.prune_empty_groups();
        true
    }

    // -- z-order --------------------------------------------------------

    pub(super) fn reorder(&mut self, node: NodeId, to: ZTarget) -> ToileOutcome {
        let Some(from_idx) = self.layers.iter().position(|n| n == &node) else {
            return ToileOutcome::display_only(Vec::new());
        };
        let last = self.layers.len().saturating_sub(1);
        let to_idx = match to {
            ZTarget::Front => last,
            ZTarget::Back => 0,
            ZTarget::Forward => (from_idx + 1).min(last),
            ZTarget::Backward => from_idx.saturating_sub(1),
            ZTarget::Index(i) => i.min(last),
        };
        if to_idx == from_idx {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = self.doc.get_movable_list(LAYERS).mov(from_idx, to_idx);
        self.doc.commit();
        self.rematerialize_layers();
        self.emit_with_update(vec![self.viewport_event()])
    }

    // -- freehand ---------------------------------------------------------

    pub(super) fn commit_stroke(
        &mut self,
        points: Vec<StrokePoint>,
        style: ElementStyle,
    ) -> ToileOutcome {
        let id = self.mint_element_id();
        // World AABB of the raw samples.
        let rect = stroke_aabb(&points);
        let elems = self.doc.get_map(ELEMENTS);
        let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
            return self.err_no_element(&id);
        };
        let _ = em.insert("type", ElementKind::Drawing.type_tag());
        let _ = em.insert("x", rect.x0);
        let _ = em.insert("y", rect.y0);
        let _ = em.insert("w", rect.width());
        let _ = em.insert("h", rect.height());
        let _ = em.insert("rot", 0.0);
        let _ = em.insert("locked", false);
        let _ = em.insert("style", LoroValue::Binary(style.to_bytes().into()));
        let _ = em.insert(
            "points",
            LoroValue::Binary(postcard::to_allocvec(&points).unwrap_or_default().into()),
        );
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .push(NodeId::Element(id.clone()).encode().as_str());
        self.doc.commit();

        self.bounds.insert(id.clone(), rect);
        self.index
            .insert(ElementBounds::from_rect(id.clone(), rect));
        self.layers.push(NodeId::Element(id.clone()));
        self.emit_with_update(vec![ToileEvent::ElementCreated(id), self.viewport_event()])
    }
}

#[cfg(test)]
mod inner_tests {
    //! The commit-free `*_inner` helpers (`toile-import-export`). Several creates,
    //! a restyle and a group must share ONE `doc.commit()` and collapse to ONE
    //! undo item, the invariant `apply_import` relies on.

    use kurbo::{Point, Size};

    use crate::logic::align::AlignOp;
    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ElementStyle, NodeId, ShapeKind};
    use crate::logic::state::ToileState;

    fn rect_at(st: &mut ToileState, x: f64) -> crate::logic::model::ElementId {
        st.create_element_inner(
            ElementKind::Shape(ShapeKind::Rect),
            Point::new(x, 0.0),
            Size::new(10.0, 10.0),
            InitContent::None,
            None,
        )
        .expect("create")
    }

    #[test]
    fn many_inner_creates_share_one_undo_item() {
        let mut st = ToileState::new();
        for i in 0..5 {
            rect_at(&mut st, i as f64 * 20.0);
        }
        st.doc.commit();
        assert_eq!(st.element_count(), 5);
        // A single Undo unwinds all five: they landed in one commit.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 0);
    }

    #[test]
    fn create_restyle_and_group_are_one_undo_item() {
        let mut st = ToileState::new();
        let a = rect_at(&mut st, 0.0);
        let b = rect_at(&mut st, 20.0);
        let style = ElementStyle {
            fill: Some(0xFF_3366CC),
            ..ElementStyle::default()
        };
        assert!(st.restyle_inner(std::slice::from_ref(&a), &style));
        assert!(st.group_inner(&[a, b]).is_some());
        st.doc.commit();
        st.rematerialize_layers();
        st.recompute_groups();
        // Two rects plus the group element.
        assert_eq!(st.element_count(), 3);
        // One Undo removes the whole batch (creates + restyle + group).
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 0);
    }

    /// A rect of `w` x 10 at `(x, y)`, committed on its own. Each fixture is
    /// then its own undo item, and the alignment under test is the last one.
    fn rect(st: &mut ToileState, x: f64, y: f64, w: f64) -> crate::logic::model::ElementId {
        let id = st
            .create_element_inner(
                ElementKind::Shape(ShapeKind::Rect),
                Point::new(x, y),
                Size::new(w, 10.0),
                InitContent::None,
                None,
            )
            .expect("create");
        st.doc.commit();
        id
    }

    /// `TOILE-037`: the whole batch is one undo item, and one Undo puts every
    /// element back where it was.
    #[test]
    fn an_alignment_is_one_undo_item() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 40.0, 30.0, 10.0);
        let c = rect(&mut st, 90.0, 60.0, 10.0);
        let before: Vec<_> = [&a, &b, &c]
            .map(|id| st.bounds_of(id).expect("bounds"))
            .into();

        st.handle_command(ToileCommand::AlignElements {
            ids: vec![a.clone(), b.clone(), c.clone()],
            op: AlignOp::Left,
        });
        assert!(
            [&a, &b, &c]
                .iter()
                .all(|id| st.bounds_of(id).expect("bounds").x0 == 0.0)
        );

        st.handle_command(ToileCommand::Undo);
        let after: Vec<_> = [&a, &b, &c]
            .map(|id| st.bounds_of(id).expect("bounds"))
            .into();
        assert_eq!(before, after, "one undo unwinds the whole batch");
    }

    /// `TOILE-204`: an element the selection does not name keeps its place, and
    /// so does the axis the operation does not work on.
    #[test]
    fn an_alignment_moves_only_what_it_names() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 40.0, 30.0, 10.0);
        let bystander = rect(&mut st, 500.0, 500.0, 10.0);

        st.handle_command(ToileCommand::AlignElements {
            ids: vec![a, b.clone()],
            op: AlignOp::Right,
        });
        assert_eq!(st.bounds_of(&bystander).expect("bounds").x0, 500.0);
        assert_eq!(st.bounds_of(&b).expect("bounds").y0, 30.0);
    }

    /// `TOILE-234`: a locked element neither moves nor sets the reference edge.
    #[test]
    fn a_locked_element_takes_no_part() {
        let mut st = ToileState::new();
        let background = rect(&mut st, -500.0, 0.0, 10.0);
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 40.0, 0.0, 10.0);
        st.handle_command(ToileCommand::LockElements {
            ids: vec![background.clone()],
            locked: true,
        });

        st.handle_command(ToileCommand::AlignElements {
            ids: vec![background.clone(), a.clone(), b.clone()],
            op: AlignOp::Left,
        });
        assert_eq!(st.bounds_of(&background).expect("bounds").x0, -500.0);
        // The union is the two unlocked boxes, so they meet at 0, not at -500.
        assert_eq!(st.bounds_of(&a).expect("bounds").x0, 0.0);
        assert_eq!(st.bounds_of(&b).expect("bounds").x0, 0.0);
    }

    /// A group aligns by its own box and carries its subtree, the way a drag
    /// does.
    #[test]
    fn a_group_aligns_as_one_box() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 100.0, 0.0, 10.0);
        let b = rect(&mut st, 130.0, 0.0, 10.0);
        let group = st.group_inner(&[a.clone(), b.clone()]).expect("group");
        st.doc.commit();
        st.rematerialize_layers();
        st.recompute_groups();
        let far = rect(&mut st, 0.0, 40.0, 10.0);

        st.handle_command(ToileCommand::AlignElements {
            ids: vec![group.clone(), far],
            op: AlignOp::Left,
        });
        // The group's left edge lands on the union edge, and both members moved
        // by the same amount.
        assert_eq!(st.bounds_of(&group).expect("bounds").x0, 0.0);
        assert_eq!(st.bounds_of(&a).expect("bounds").x0, 0.0);
        assert_eq!(st.bounds_of(&b).expect("bounds").x0, 30.0);
    }

    /// An alignment that would write nothing writes nothing: no commit, so no
    /// undo item and no delta for the peers.
    #[test]
    fn an_alignment_that_changes_nothing_mints_no_undo_item() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 0.0, 40.0, 10.0);
        st.handle_command(ToileCommand::AlignElements {
            ids: vec![a.clone(), b],
            op: AlignOp::Left,
        });
        // The last undo item is still the second create, which it unwinds.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(st.element_count(), 1);
        assert!(st.bounds_of(&a).is_some());
    }

    /// `TOILE-129`: a delete that names one node preserves an entry this build
    /// cannot decode, instead of destroying it on an unrelated write.
    #[test]
    fn a_delete_preserves_an_undecodable_layer_entry() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 40.0, 0.0, 10.0);
        st.debug_push_layer_raw("X:future_tag");

        st.handle_command(ToileCommand::DeleteElements {
            ids: vec![a.clone()],
        });

        let raw = st.debug_raw_layers();
        let a_entry = NodeId::Element(a).encode();
        let b_entry = NodeId::Element(b).encode();
        assert!(
            raw.iter().any(|s| s == "X:future_tag"),
            "the undecodable entry survives the unrelated delete"
        );
        assert!(
            raw.iter().any(|s| s == &b_entry),
            "the untouched entry survives"
        );
        assert!(
            !raw.iter().any(|s| s == &a_entry),
            "the deleted entry is gone"
        );
    }

    /// `TOILE-UI-174`: a whole-selection restyle overwrites only the fields a
    /// surface shows; each member keeps its own corner radius, shadow and
    /// non-bold weight.
    #[test]
    fn a_restyle_preserves_the_fields_no_surface_shows() {
        let mut st = ToileState::new();
        let a = rect(&mut st, 0.0, 0.0, 10.0);
        let b = rect(&mut st, 40.0, 0.0, 10.0);
        // Seed two members with differing hidden fields, whole (the import path).
        let shadow = ambre_bus::app::toile::Shadow {
            color: 0,
            dx: 1.0,
            dy: 1.0,
            blur: 2.0,
        };
        let style_a = ElementStyle {
            fill: Some(0xffff_0000),
            corner: 12.0,
            shadow: Some(shadow),
            font: ambre_bus::app::toile::FontSpec {
                weight: 600,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        let style_b = ElementStyle {
            fill: Some(0xff00_ff00),
            corner: 0.0,
            shadow: None,
            font: ambre_bus::app::toile::FontSpec {
                weight: 400,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        st.restyle_inner(std::slice::from_ref(&a), &style_a);
        st.restyle_inner(std::slice::from_ref(&b), &style_b);
        st.doc.commit();

        // Restyle both with a shared fill change.
        st.handle_command(ToileCommand::RestyleElements {
            ids: vec![a.clone(), b.clone()],
            style: ElementStyle {
                fill: Some(0xff00_00ff),
                ..ElementStyle::default()
            },
        });

        // The hidden fields survive; the shown fill changes on both.
        assert_eq!(st.style_of(&a).corner, 12.0);
        assert!(st.style_of(&a).shadow.is_some());
        assert_eq!(st.style_of(&a).font.weight, 600);
        assert_eq!(st.style_of(&b).corner, 0.0);
        assert!(st.style_of(&b).shadow.is_none());
        assert_eq!(st.style_of(&b).font.weight, 400);
        assert_eq!(st.style_of(&a).fill, Some(0xff00_00ff));
        assert_eq!(st.style_of(&b).fill, Some(0xff00_00ff));
    }
}
