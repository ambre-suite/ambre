//! Element / stroke lifecycle for [`ToileState`](super::ToileState):
//! create, move, resize, rotate, restyle, lock, delete, z-order reorder and
//! freehand stroke commit. Split out of `state/mod.rs`; every method here is a
//! `handle_command` target and commits exactly one undo item.

use ambre_core::Rgba;
use std::collections::HashSet;

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use crate::logic::align::{AlignOp, align_deltas};
use crate::logic::command::{
    ElementCreation, InitContent, MediaRef, ToileError, ToileEvent, ToileOutcome, ZTarget,
};
use crate::logic::index::ElementBounds;
use crate::logic::model::{ElementKind, ElementStyle, LinkTarget, NodeId, ShapeKind, StrokePoint};
use crate::logic::text_merge::merge_edits;
use ambre_bus::Token;
use ambre_core::id::{Element, Id};

use super::codec::has_type;
use super::codec::{
    bin_of, body_text_or_seed, content_matches, expanded_aabb, locked, store_corner, store_size,
    stored_rect, stroke_aabb,
};
use super::names::{ElementKey, Fields, Root};
use super::{ToileState, write_edits};

impl ToileState {
    pub(super) fn create_element(
        &mut self,
        kind: ElementKind,
        at: Point,
        size: kurbo::Size,
        content: InitContent,
        fill: Option<Rgba>,
        correlate: Option<Token<ElementCreation>>,
    ) -> ToileOutcome {
        let id = match self.create_element_inner(kind, at, size, content, fill) {
            Ok(id) => id,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        self.doc.commit();
        self.emit_with_update(vec![
            ToileEvent::ElementCreated { id, correlate },
            self.viewport_event(),
        ])
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
        fill: Option<Rgba>,
    ) -> Result<Id<Element>, ToileError> {
        // Group and Drawing are not creatable here: a group is born via
        // GroupElements, a stroke via CommitStroke.
        if matches!(kind, ElementKind::Group | ElementKind::Drawing) {
            return Err(ToileError::NotCreatable { kind });
        }
        if !content_matches(kind, &content) {
            return Err(ToileError::ContentKindMismatch);
        }
        // A birth box must be finite and not negative, with one side at least.
        // A flat pen stroke has no height and is still drawn (`TOILE-188`).
        let usable = |v: f64| v.is_finite() && v >= 0.0;
        if !usable(size.width) || !usable(size.height) || size.width + size.height == 0.0 {
            return Err(ToileError::UnusableSize);
        }
        if !at.x.is_finite() || !at.y.is_finite() {
            return Err(ToileError::UnusableSize);
        }

        let id = self.mint_element_id()?;
        let elems = self.doc.get_map(Root::Elements.as_str());
        let Ok(em) = elems
            .insert_container(id.as_str(), LoroMap::new())
            .map(Fields::<ElementKey>::new)
        else {
            return Err(ToileError::NoSuchElement(id));
        };
        let _ = em.insert(ElementKey::Type, kind.type_tag());
        store_corner(&em, at);
        store_size(&em, size);
        let _ = em.insert(ElementKey::Rot, 0.0);
        let _ = em.insert(ElementKey::Locked, false);
        // Seed a per-kind default style (sticky = yellow card, others = white),
        // then apply an optional creation-time fill override.
        let mut style = ElementStyle::for_kind(kind);
        if let Some(f) = fill {
            style.fill = Some(f);
        }
        let _ = em.insert(
            ElementKey::Style,
            LoroValue::Binary(style.to_bytes().into()),
        );
        self.seed_payload(&em, kind, &content);
        // Push onto the top of the z-order.
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .push(NodeId::Element(id.clone()).encode().as_str());

        let rect = expanded_aabb(
            Rect::new(at.x, at.y, at.x + size.width, at.y + size.height),
            ambre_core::Radians::ZERO,
        );
        self.bounds.insert(id.clone(), rect);
        self.index
            .insert(ElementBounds::from_rect(id.clone(), rect));
        self.layers.push(NodeId::Element(id.clone()));
        Ok(id)
    }

    /// Seed the per-type payload fields. Kept minimal for Phase 0.
    fn seed_payload(&self, em: &Fields<ElementKey>, kind: ElementKind, content: &InitContent) {
        match (kind, content) {
            (ElementKind::Shape(sk), _) => {
                let _ = em.insert(ElementKey::Shape, sk.tag());
                if let (ShapeKind::Path, InitContent::Path(p)) = (sk, content) {
                    let _ = em.insert(ElementKey::Path, p.to_svg());
                }
            }
            (ElementKind::Image, InitContent::Image(MediaRef::Stored(hash))) => {
                let _ = em.insert(ElementKey::Hash, hash.as_str());
            }
            (ElementKind::Embed, InitContent::Embed(url)) => {
                let _ = em.insert(ElementKey::Url, url.as_str());
            }
            (
                ElementKind::DocEmbed,
                InitContent::DocEmbed {
                    document_id,
                    doc_type,
                },
            ) => {
                let _ = em.insert(ElementKey::DocId, document_id.to_string());
                let _ = em.insert(ElementKey::DocType, format!("{doc_type:?}"));
            }
            // The body is a text seeded at birth, as a frame's notes are, so
            // every peer types into the one container (`TOILE-030`).
            (ElementKind::Sticky | ElementKind::Text, InitContent::Text(t)) => {
                if let Ok(text) = em.insert_container(ElementKey::Body, LoroText::new())
                    && !t.is_empty()
                {
                    let _ = text.insert(0, t);
                }
            }
            (ElementKind::Sticky | ElementKind::Text, InitContent::None) => {
                let _ = em.insert_container(ElementKey::Body, LoroText::new());
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
                let _ = em.insert_container(ElementKey::Children, LoroMovableList::new());
                let _ = em.insert_container(ElementKey::Notes, LoroText::new());
            }
            _ => {}
        }
    }

    /// Write the body of a sticky / text element. The editor hands the body it
    /// opened on and the whole body it closes with. The write merges that
    /// change into the stored body, then writes the merge's edits into its
    /// `LoroText`.
    /// So two authors editing one note each keep the characters they typed
    /// (`TOILE-030`), even when one edit lands while the other's editor is
    /// open (`TOILE-UI-381`). Only sticky / text elements carry that field; a
    /// request against any other kind (or a missing element) is a rejected
    /// no-op.
    pub(super) fn set_text(&mut self, id: &Id<Element>, base: &str, text: &str) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if locked(&em) {
            return ToileOutcome::display_only(Vec::new());
        }
        // The `"text"` field exists only on sticky / text elements (seeded at
        // creation). Reject a SetText aimed at any other kind.
        if em.get(ElementKey::Body).is_none() {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }
        let Some(body) = body_text_or_seed(&em) else {
            return ToileOutcome::display_only(Vec::new());
        };
        let merge = merge_edits(base, &body.to_string(), text);
        if merge.edits.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = write_edits(&body, &merge.edits);
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    /// Set or clear one element's link register (`toile-links`). The register is
    /// its own field, never part of the style. A style is written whole. A link
    /// folded into it would be erased by any restyle of a selection the element
    /// happens to be in (`TOILE-062`). A locked element refuses the write like
    /// it refuses every other edit.
    pub(super) fn set_link(&mut self, id: &Id<Element>, link: Option<LinkTarget>) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if locked(&em) {
            return ToileOutcome::display_only(Vec::new());
        }
        match link {
            Some(l) => {
                let _ = em.insert(ElementKey::Link, LoroValue::Binary(l.to_bytes().into()));
            }
            // Absence is the value (`TOILE-230`): a cleared link is a missing
            // key, not a key holding an empty blob.
            None => {
                let _ = em.delete(ElementKey::Link);
            }
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn move_elements(&mut self, ids: &[Id<Element>], delta: Vec2) -> ToileOutcome {
        // An id the board does not hold answers an error, not a silent skip
        // (`TOILE-194`).
        for id in ids {
            if self.element_map(id).is_none() {
                return self.err_no_element(id);
            }
        }
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
            if locked(&em) {
                continue; // locked blocks edit locally (advisory)
            }
            store_corner(&em, stored_rect(&em).origin() + delta);
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
    pub(super) fn align_elements(&mut self, ids: &[Id<Element>], op: AlignOp) -> ToileOutcome {
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
            if self.element_map(id).is_some_and(|em| locked(&em)) {
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
                if locked(&em) {
                    continue; // a locked leaf inside a moving group stays put
                }
                store_corner(&em, stored_rect(&em).origin() + delta);
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

    pub(super) fn resize_element(&mut self, id: &Id<Element>, new: Rect) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        // A group's box is derived from its members, so a resize has nothing
        // to write (`TOILE-150`, `TOILE-155`).
        if locked(&em) || has_type(&em, ElementKind::Group) {
            return ToileOutcome::display_only(Vec::new());
        }
        let r = new.abs();
        store_corner(&em, r.origin());
        store_size(&em, r.size());
        self.update_bounds(id, &em);
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn rotate_element(
        &mut self,
        id: &Id<Element>,
        radians: ambre_core::Radians,
    ) -> ToileOutcome {
        let Some(em) = self.element_map(id) else {
            return self.err_no_element(id);
        };
        if locked(&em) {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = em.insert(ElementKey::Rot, radians.get());
        // A group holds no coordinates: its box stays the union of its members'
        // (`TOILE-009`), never a leaf box read from absent fields.
        if !has_type(&em, ElementKind::Group) {
            self.update_bounds(id, &em);
        }
        self.recompute_groups();
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn restyle_elements(
        &mut self,
        ids: &[Id<Element>],
        style: ElementStyle,
    ) -> ToileOutcome {
        // An id the board does not hold answers an error, not a silent skip
        // (`TOILE-194`).
        for id in ids {
            if self.element_map(id).is_none() {
                return self.err_no_element(id);
            }
        }
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
    pub(super) fn restyle_merge_inner(
        &mut self,
        ids: &[Id<Element>],
        style: &ElementStyle,
    ) -> bool {
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if locked(&em) {
                continue;
            }
            let existing = self.style_read(bin_of(&em, ElementKey::Style).as_deref());
            let merged = existing.merged_over(style);
            let _ = em.insert(
                ElementKey::Style,
                LoroValue::Binary(merged.to_bytes().into()),
            );
            touched = true;
        }
        touched
    }

    /// Overwrite the style register of every unlocked element in `ids`, **without**
    /// committing. Returns whether anything changed. The caller owns the
    /// `doc.commit()`, so an import can restyle a fresh element inside the same
    /// undo group as its `CreateElement`. Style is an independent LWW register that
    /// cannot ride the create.
    pub(super) fn restyle_inner(&mut self, ids: &[Id<Element>], style: &ElementStyle) -> bool {
        let bytes = style.to_bytes();
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            if locked(&em) {
                continue;
            }
            let _ = em.insert(ElementKey::Style, LoroValue::Binary(bytes.clone().into()));
            touched = true;
        }
        touched
    }

    pub(super) fn lock_elements(
        &mut self,
        ids: &[Id<Element>],
        lock: crate::message::LockState,
    ) -> ToileOutcome {
        let mut touched = false;
        for id in ids {
            let Some(em) = self.element_map(id) else {
                continue;
            };
            let _ = em.insert(ElementKey::Locked, lock.is_locked());
            touched = true;
        }
        if !touched {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
    }

    pub(super) fn delete_elements(&mut self, ids: &[Id<Element>]) -> ToileOutcome {
        // An id the board does not hold answers an error, not a silent skip
        // (`TOILE-201`).
        for id in ids {
            if self.element_map(id).is_none() {
                return self.err_no_element(id);
            }
        }
        // A locked element refuses the delete as it refuses every other write
        // (`TOILE-033`). A group with a locked member is kept whole, since its
        // cascade would remove that member.
        let ids: Vec<Id<Element>> = ids
            .iter()
            .filter(|id| !self.holds_a_lock(id))
            .cloned()
            .collect();
        let deck_before = self.slide_order();
        if ids.is_empty() || !self.delete_from_doc(&ids) {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();

        self.rematerialize_layers();
        self.recompute_groups();
        let mut events = vec![self.viewport_event()];
        events.extend(self.deck_change(&deck_before));
        self.emit_with_update(events)
    }

    /// Whether `id` or any element of its subtree is locked.
    fn holds_a_lock(&self, id: &Id<Element>) -> bool {
        let mut subtree = HashSet::new();
        self.collect_subtree(id, &mut subtree);
        subtree
            .iter()
            .any(|e| self.element_map(e).is_some_and(|em| locked(&em)))
    }

    /// Remove `ids` (and, for groups, their whole subtree) from the CRDT and the
    /// incremental caches, without committing. Drops group membership, orphaned
    /// connectors and layer entries, and prunes emptied groups (`toile-elements`).
    /// Returns whether anything was removed. The caller owns the `doc.commit()`
    /// so a deletion shares one commit and one undo item with a sibling
    /// mutation: a boolean op consuming its sources.
    pub(super) fn delete_from_doc(&mut self, ids: &[Id<Element>]) -> bool {
        // Deleting a group cascades to its members: expand the request
        // to the full subtree of every listed group before removing anything.
        let mut expanded: HashSet<Id<Element>> = HashSet::new();
        for id in ids {
            self.collect_subtree(id, &mut expanded);
        }
        let del: HashSet<&Id<Element>> = expanded.iter().collect();
        let mut removed_any = false;
        let elems = self.doc.get_map(Root::Elements.as_str());
        for id in &expanded {
            if elems.get(id.as_str()).is_some() {
                let _ = elems.delete(id.as_str());
                removed_any = true;
            }
        }
        if !removed_any {
            return false;
        }
        // Drop the id from every list that names it, a frame's as a group's
        // (`TOILE-271`). The removal is a document write, so a peer that
        // refreshes the list unlinks the id too. The parent cache names
        // each list once per entry, so a list holding the id twice loses both.
        // A deleted frame keeps its members on the board, and the cache drops
        // the links its list held.
        for id in &expanded {
            for parent in self.parents_of(id).to_vec() {
                self.remove_child(&parent, id);
            }
            self.forget_links(id);
        }
        // Drop locally-owned connectors whose endpoint was deleted and
        // strip both elements and orphaned connectors from `layers`.
        let orphan_conns = self.connectors_touching(&del);
        let conns = self.doc.get_map(Root::Connectors.as_str());
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
        // A locked element keeps its place in the order (`TOILE-033`).
        if let NodeId::Element(id) = &node
            && self.holds_a_lock(id)
        {
            return ToileOutcome::display_only(Vec::new());
        }
        let last = self.layers.len().saturating_sub(1);
        let to_idx = match to {
            ZTarget::Front => last,
            ZTarget::Back => 0,
            ZTarget::Forward => (from_idx + 1).min(last),
            ZTarget::Backward => from_idx.saturating_sub(1),
            ZTarget::Index(i) => i.to_usize().min(last),
        };
        if to_idx == from_idx {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .mov(from_idx, to_idx);
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
        let id = match self.mint_element_id() {
            Ok(id) => id,
            Err(e) => return ToileOutcome::display_only(vec![ToileEvent::Error(e)]),
        };
        // World AABB of the raw samples.
        let rect = stroke_aabb(&points);
        let elems = self.doc.get_map(Root::Elements.as_str());
        let Ok(em) = elems
            .insert_container(id.as_str(), LoroMap::new())
            .map(Fields::<ElementKey>::new)
        else {
            return self.err_no_element(&id);
        };
        let _ = em.insert(ElementKey::Type, ElementKind::Drawing.type_tag());
        store_corner(&em, rect.origin());
        store_size(&em, rect.size());
        let _ = em.insert(ElementKey::Rot, 0.0);
        let _ = em.insert(ElementKey::Locked, false);
        let _ = em.insert(
            ElementKey::Style,
            LoroValue::Binary(style.to_bytes().into()),
        );
        let _ = em.insert(
            ElementKey::Points,
            LoroValue::Binary(postcard::to_allocvec(&points).unwrap_or_default().into()),
        );
        let _ = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .push(NodeId::Element(id.clone()).encode().as_str());
        self.doc.commit();

        self.bounds.insert(id.clone(), rect);
        self.index
            .insert(ElementBounds::from_rect(id.clone(), rect));
        self.layers.push(NodeId::Element(id.clone()));
        self.emit_with_update(vec![
            ToileEvent::ElementCreated {
                id,
                correlate: None,
            },
            self.viewport_event(),
        ])
    }
}

#[cfg(test)]
mod inner_tests {
    //! The commit-free `*_inner` helpers (`toile-import-export`). Several creates,
    //! a restyle and a group must share ONE `doc.commit()` and collapse to ONE
    //! undo item, the invariant `apply_import` relies on.

    use ambre_core::index::Pos;
    use ambre_core::{Px, Rgba, StoredBoard};
    use kurbo::{Point, Size};

    use crate::logic::align::AlignOp;
    use crate::logic::command::{
        DisplayGeom, DisplayNode, ElementCreation, InitContent, ToileCommand, ToileEvent,
    };
    use crate::logic::model::{ElementKind, ElementStyle, NodeId, ShapeKind};
    use crate::logic::state::ToileState;
    use ambre_bus::Token;
    use ambre_core::id::{Element, Id, User};
    use ambre_core::{Authored, Instant};

    fn rect_at(st: &mut ToileState, x: f64) -> Id<Element> {
        st.create_element_inner(
            ElementKind::Shape(ShapeKind::Rect),
            Point::new(x, 0.0),
            Size::new(10.0, 10.0),
            InitContent::None,
            None,
        )
        .expect("create")
    }

    /// `TOILE-IO-059`: an image create naming an import slot Sync has not
    /// answered is refused, so the slot's token never lands in the document.
    #[test]
    fn an_image_naming_a_pending_slot_is_refused() {
        let mut st = ToileState::new();
        let refused = st.create_element_inner(
            ElementKind::Image,
            Point::ZERO,
            Size::new(10.0, 10.0),
            InitContent::Image(crate::logic::command::MediaRef::Pending(
                Pos::<crate::logic::command::ImportSlot>::ZERO,
            )),
            None,
        );
        assert_eq!(
            refused,
            Err(crate::logic::command::ToileError::ContentKindMismatch)
        );
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::ZERO
        );
    }

    #[test]
    fn many_inner_creates_share_one_undo_item() {
        let mut st = ToileState::new();
        for i in 0..5 {
            rect_at(&mut st, i as f64 * 20.0);
        }
        st.doc.commit();
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(5)
        );
        // A single Undo unwinds all five: they landed in one commit.
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::ZERO
        );
    }

    #[test]
    fn create_restyle_and_group_are_one_undo_item() {
        let mut st = ToileState::new();
        let a = rect_at(&mut st, 0.0);
        let b = rect_at(&mut st, 20.0);
        let style = ElementStyle {
            fill: Some(Rgba::from_bits(0x3366_CCFF)),
            ..ElementStyle::default()
        };
        assert!(st.restyle_inner(std::slice::from_ref(&a), &style));
        assert!(st.group_inner(&[a, b]).is_ok());
        st.doc.commit();
        st.rematerialize_layers();
        st.recompute_groups();
        // Two rects plus the group element.
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(3)
        );
        // One Undo removes the whole batch (creates + restyle + group).
        st.handle_command(ToileCommand::Undo);
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::ZERO
        );
    }

    /// A session whose counter reached `u32::MAX` mints no further id. Each
    /// create refuses, and the element that holds the last id keeps its box:
    /// a repeated id would overwrite it.
    #[test]
    fn a_spent_counter_refuses_each_create_and_overwrites_nothing() {
        use crate::logic::command::ToileError;
        let mut st = ToileState::new();
        let a = rect_at(&mut st, 0.0);
        st.counter = crate::message::IdCounter::new(u32::MAX - 1);
        let last = rect_at(&mut st, 20.0);
        assert!(last.as_str().ends_with("_ffffffff"));
        st.doc.commit();
        let refused = st.create_element_inner(
            ElementKind::Shape(ShapeKind::Rect),
            Point::new(90.0, 90.0),
            Size::new(5.0, 5.0),
            InitContent::None,
            None,
        );
        assert_eq!(refused, Err(ToileError::IdsExhausted));
        assert_eq!(
            st.group_inner(&[a.clone(), last.clone()]),
            Err(ToileError::IdsExhausted)
        );
        let pasted = st.handle_command(ToileCommand::DuplicateElements {
            ids: vec![a],
            delta: kurbo::Vec2::new(5.0, 5.0),
        });
        assert!(
            pasted
                .display
                .iter()
                .any(|e| matches!(e, ToileEvent::Error(ToileError::IdsExhausted)))
        );
        st.doc.commit();
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(2)
        );
        assert_eq!(
            st.bounds.get(&last).map(|r| (r.x0, r.width())),
            Some((20.0, 10.0))
        );
    }

    /// A stroke, an arrow, a boolean result and a comment thread each need a
    /// fresh id. With the counter spent, each answers the same refusal and
    /// writes nothing (`TOILE-270`).
    #[test]
    fn a_spent_counter_refuses_every_other_create_and_writes_nothing() {
        use crate::logic::command::ToileError;
        use crate::message::{
            Anchor, BoolOp, Cap, CommentAnchor, EndCaps, PathType, Sources, StrokePoint,
        };
        let mut st = ToileState::new();
        st.set_user_id(Id::<User>::mint());
        let a = rect_at(&mut st, 0.0);
        let b = rect_at(&mut st, 5.0);
        st.doc.commit();
        st.counter = crate::message::IdCounter::new(u32::MAX);
        let vv = st.doc.oplog_vv();
        let doc = st.doc.get_deep_value();
        let point = |x: f32| {
            StrokePoint::full_pressure(Px::<StoredBoard>::new(x), Px::<StoredBoard>::new(x))
        };
        let commands = [
            (
                "stroke",
                ToileCommand::CommitStroke {
                    points: vec![point(0.0), point(4.0)],
                    style: ElementStyle::default(),
                },
            ),
            (
                "arrow",
                ToileCommand::CreateConnector {
                    from: a.clone(),
                    from_anch: Anchor::Right,
                    to: b.clone(),
                    to_anch: Anchor::Left,
                    path_type: PathType::Straight,
                    arrows: EndCaps {
                        start: Cap::Plain,
                        end: Cap::Arrow,
                    },
                },
            ),
            (
                "boolean",
                ToileCommand::BooleanOp {
                    op: BoolOp::Union,
                    ids: vec![a, b],
                    sources: Sources::Consume,
                },
            ),
            (
                "comment",
                ToileCommand::CreateComment {
                    anchor: CommentAnchor::Point(Point::new(1.0, 1.0)),
                    body: "late".to_owned(),
                    ts: Instant::<Authored>::new(1),
                },
            ),
        ];
        for (name, command) in commands {
            let out = st.handle_command(command);
            assert!(
                out.display
                    .iter()
                    .any(|e| matches!(e, ToileEvent::Error(ToileError::IdsExhausted))),
                "{name}"
            );
            assert_eq!(st.doc.get_deep_value(), doc, "{name}");
        }
        st.doc.commit();
        assert_eq!(st.doc.oplog_vv(), vv, "nothing left pending");
    }

    /// A group refused for want of an id writes nothing first. Its members
    /// stay in the group they sat in, the caches hold, and no detach is left
    /// pending to ride along with the next commit.
    #[test]
    #[cfg(not(target_arch = "wasm32"))] // `debug_caches` is a host helper
    fn a_group_refused_at_the_top_leaves_its_members_where_they_sat() {
        use crate::logic::command::ToileError;
        let mut st = ToileState::new();
        let a = rect_at(&mut st, 0.0);
        let b = rect_at(&mut st, 20.0);
        let first = st.group_inner(&[a.clone(), b.clone()]).expect("group");
        st.doc.commit();
        st.counter = crate::message::IdCounter::new(u32::MAX);
        let doc_before = st.doc.get_deep_value();
        let caches_before = st.debug_caches();
        let vv_before = st.doc.oplog_vv();
        assert_eq!(
            st.group_inner(&[a.clone(), b.clone()]),
            Err(ToileError::IdsExhausted)
        );
        assert_eq!(st.doc.get_deep_value(), doc_before);
        assert_eq!(st.debug_caches(), caches_before);
        st.doc.commit();
        assert_eq!(st.doc.oplog_vv(), vv_before);
        assert_eq!(st.parents_of(&a).to_vec(), vec![first.clone()]);
        assert_eq!(st.parents_of(&b).to_vec(), vec![first]);
    }

    /// A rect of `w` x 10 at `(x, y)`, committed on its own. Each fixture is
    /// then its own undo item, and the alignment under test is the last one.
    fn rect(st: &mut ToileState, x: f64, y: f64, w: f64) -> Id<Element> {
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
            lock: crate::message::LockState::Locked,
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

    /// `TOILE-053`: a group aligns by its own box and carries its subtree, the
    /// way a drag does.
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
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(1)
        );
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

    /// `TOILE-061`: deleting one element of a board of 200 changes the order
    /// by one entry. The change a peer receives holds a handful of operations,
    /// not one per surviving node, and the survivors keep their order.
    #[test]
    fn a_delete_rewrites_no_other_layer_entry() {
        let mut st = ToileState::new();
        let ids: Vec<_> = (0..200)
            .map(|i| rect(&mut st, i as f64 * 20.0, 0.0, 10.0))
            .collect();
        let before = st.debug_raw_layers();
        let ops = st.doc.len_ops();

        st.handle_command(ToileCommand::DeleteElements {
            ids: vec![ids[100].clone()],
        });

        let gone = NodeId::Element(ids[100].clone()).encode();
        let expected: Vec<_> = before.into_iter().filter(|s| s != &gone).collect();
        assert_eq!(
            st.debug_raw_layers(),
            expected,
            "the survivors keep their order"
        );
        let spent = st.doc.len_ops() - ops;
        assert!(spent < 10, "one delete costs {spent} operations");
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
        let shadow = crate::message::Shadow {
            color: Rgba::from_bits(0x0000_0000),
            dx: Px::<StoredBoard>::new(1.0),
            dy: Px::<StoredBoard>::new(1.0),
            blur: Px::<StoredBoard>::new(2.0),
        };
        let style_a = ElementStyle {
            fill: Some(Rgba::from_bits(0xff00_00ff)),
            corner: Px::<StoredBoard>::new(12.0),
            shadow: Some(shadow),
            font: crate::message::FontSpec {
                weight: ambre_core::Bounded::<ambre_core::FontWeight>::new(600).unwrap(),
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        let style_b = ElementStyle {
            fill: Some(Rgba::from_bits(0x00ff_00ff)),
            corner: Px::<StoredBoard>::ZERO,
            shadow: None,
            font: crate::message::FontSpec {
                weight: ambre_core::Bounded::<ambre_core::FontWeight>::REGULAR,
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
                fill: Some(Rgba::from_bits(0x0000_ffff)),
                ..ElementStyle::default()
            },
        });

        // The hidden fields survive; the shown fill changes on both.
        assert_eq!(st.style_of(&a).corner, Px::<StoredBoard>::new(12.0));
        assert!(st.style_of(&a).shadow.is_some());
        assert_eq!(st.style_of(&a).font.weight.get(), 600);
        assert_eq!(st.style_of(&b).corner, Px::<StoredBoard>::ZERO);
        assert!(st.style_of(&b).shadow.is_none());
        assert_eq!(
            st.style_of(&b).font.weight,
            ambre_core::Bounded::<ambre_core::FontWeight>::REGULAR
        );
        assert_eq!(st.style_of(&a).fill, Some(Rgba::from_bits(0x0000_ffff)));
        assert_eq!(st.style_of(&b).fill, Some(Rgba::from_bits(0x0000_ffff)));
    }

    /// `TOILE-UI-194`: a create echoes its correlate token, so a deferred open
    /// can match this reply and no other create's.
    #[test]
    fn a_create_echoes_its_correlate_token() {
        let token = Token::<ElementCreation>::FIRST.next();
        let mut st = ToileState::new();
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(0.0, 0.0),
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: Some(token),
        });
        assert!(out.display.iter().any(|e| matches!(
            e,
            ToileEvent::ElementCreated { correlate: Some(echo), .. } if *echo == token
        )));
    }

    /// `TOILE-UI-209`: a locked element refuses the text write, so the screen
    /// must not echo a body the write will not take.
    #[test]
    fn a_locked_element_refuses_a_text_write() {
        let mut st = ToileState::new();
        let id = st
            .create_element_inner(
                ElementKind::Sticky,
                Point::new(0.0, 0.0),
                Size::new(120.0, 80.0),
                InitContent::Text("hello".to_owned()),
                None,
            )
            .expect("create");
        st.doc.commit();
        st.handle_command(ToileCommand::LockElements {
            ids: vec![id.clone()],
            lock: crate::message::LockState::Locked,
        });
        st.handle_command(ToileCommand::SetText {
            id: id.clone(),
            base: "hello".to_owned(),
            text: "changed".to_owned(),
        });
        let body = st
            .debug_viewport()
            .nodes
            .into_iter()
            .find_map(|n| match n {
                DisplayNode::Element(e) if e.id == id => match e.geom {
                    DisplayGeom::Sticky(t) => Some(t),
                    _ => None,
                },
                _ => None,
            })
            .expect("the sticky is on the board");
        assert_eq!(body, "hello", "the refused write leaves the body unchanged");
    }
}
