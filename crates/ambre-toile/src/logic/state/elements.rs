//! Element / stroke lifecycle for [`ToileState`](super::ToileState):
//! create, move, resize, rotate, restyle, lock, delete, z-order reorder and
//! freehand stroke commit. Split out of `state/mod.rs`; every method here is a
//! `handle_command` target and commits exactly one undo item.

use std::collections::HashSet;

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use crate::logic::command::{InitContent, ToileError, ToileEvent, ToileOutcome, ZTarget};
use crate::logic::index::ElementBounds;
use crate::logic::model::{ElementId, ElementKind, ElementStyle, NodeId, ShapeKind, StrokePoint};

use super::codec::{bool_of, content_matches, expanded_aabb, f64_of, stroke_aabb};
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
                // lazily on first edit: seeding at creation gives every peer the
                // same shared LoroText container (broadcast in the creation delta)
                // before any edit, so concurrent notes edits converge.
                // A lazy first edit on each peer would mint two rival containers
                // under the same map key -> LWW collision, one side's notes lost.
                // Autolayout stays lazy (absent = free positioning,).
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
        if !self.restyle_inner(ids, &style) {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Overwrite the style register of every unlocked element in `ids`, **without**
    /// committing. Returns whether anything changed. The caller owns the
    /// `doc.commit()` so an import can restyle a fresh element inside the same
    /// undo group as its `CreateElement`: style is an independent LWW
    /// register that cannot ride the create.
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
        if !self.delete_from_doc(ids) {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();

        self.rematerialize_layers();
        self.recompute_groups();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Remove `ids` (and, for groups, their whole subtree) from the CRDT and the
    /// incremental caches, without committing. Drops group membership, orphaned
    /// connectors and layer entries, and prunes emptied groups (`toile-elements`).
    /// Returns whether anything was removed. The caller owns the `doc.commit()`
    /// so a deletion can share one commit / one undo item with a sibling mutation
    /// (e.g. a boolean op consuming its sources).
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
    //! The commit-free `*_inner` helpers (`toile-import-export`): several creates,
    //! a restyle and a group must share ONE `doc.commit()` and collapse to ONE
    //! undo item, the invariant `apply_import` relies on.

    use kurbo::{Point, Size};

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ElementStyle, ShapeKind};
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
}
