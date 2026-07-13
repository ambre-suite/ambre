//! Element / stroke lifecycle for [`ToileState`](super::ToileState) (§8):
//! create, move, resize, rotate, restyle, lock, delete, z-order reorder and
//! freehand stroke commit. Split out of `state/mod.rs`; every method here is a
//! `handle_command` target and commits exactly one undo item.

use std::collections::HashSet;

use kurbo::{Point, Rect, Vec2};
use loro::{LoroMap, LoroMovableList, LoroValue};

use crate::logic::command::{InitContent, ToileError, ToileEvent, ToileOutcome, ZTarget};
use crate::logic::index::ElementBounds;
use crate::logic::model::{ElementId, ElementKind, ElementStyle, NodeId, ShapeKind, StrokePoint};

use super::codec::{bool_of, content_matches, expanded_aabb, f64_of, stroke_aabb};
use super::{CHILDREN, CONNECTORS, ELEMENTS, LAYERS, ToileState};

impl ToileState {
    pub(super) fn create_element(
        &mut self,
        kind: ElementKind,
        at: Point,
        size: kurbo::Size,
        content: InitContent,
    ) -> ToileOutcome {
        // Group and Drawing are not creatable here (§8): a group is born via
        // GroupElements, a stroke via CommitStroke.
        if matches!(kind, ElementKind::Group | ElementKind::Drawing) {
            return ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NotCreatable {
                kind: kind.type_tag(),
            })]);
        }
        if !content_matches(kind, &content) {
            return ToileOutcome::display_only(vec![ToileEvent::Error(
                ToileError::ContentKindMismatch,
            )]);
        }

        let id = self.mint_element_id();
        let elems = self.doc.get_map(ELEMENTS);
        let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
            return ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchElement(
                id.0,
            ))]);
        };
        let _ = em.insert("type", kind.type_tag());
        let _ = em.insert("x", at.x);
        let _ = em.insert("y", at.y);
        let _ = em.insert("w", size.width);
        let _ = em.insert("h", size.height);
        let _ = em.insert("rot", 0.0);
        let _ = em.insert("locked", false);
        let _ = em.insert(
            "style",
            LoroValue::Binary(ElementStyle::default().to_bytes().into()),
        );
        self.seed_payload(&em, kind, &content);
        // Push onto the top of the z-order.
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .push(NodeId::Element(id.clone()).encode().as_str());
        self.doc.commit();

        let rect = expanded_aabb(at.x, at.y, size.width, size.height, 0.0);
        self.bounds.insert(id.clone(), rect);
        self.index
            .insert(ElementBounds::from_rect(id.clone(), rect));
        self.layers.push(NodeId::Element(id.clone()));

        self.emit_with_update(vec![ToileEvent::ElementCreated(id), self.viewport_event()])
    }

    /// Seed the per-type payload fields (§2.C). Kept minimal for Phase 0.
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
                // Frame content (§2.C): an empty membership list at birth. Notes /
                // autolayout are seeded by their toile-features commands.
                let _ = em.insert_container(CHILDREN, LoroMovableList::new());
            }
            _ => {}
        }
    }

    /// Overwrite the plaintext body of a sticky / text element (§6). Phase 0
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
        // descendants so translating a group moves its whole subtree (§2.F bis).
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
                continue; // locked blocks edit locally (§6, advisory)
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
        if !touched {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc.commit();
        self.emit_with_update(vec![self.viewport_event()])
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
        // Deleting a group cascades to its members (§2.F bis): expand the request
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
            return ToileOutcome::display_only(Vec::new());
        }
        // Drop any group membership referencing a deleted node (§2.F bis / §2.H).
        for id in &expanded {
            if let Some(g) = self.enclosing_group(id) {
                self.remove_child(&g, id);
            }
        }
        // Drop locally-owned connectors whose endpoint was deleted (§5.A) and
        // strip both elements and orphaned connectors from `layers` (§2.H).
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
        // prune it so the incremental caches match a full rebuild (§1).
        self.prune_empty_groups();
        self.doc.commit();

        self.rematerialize_layers();
        self.recompute_groups();
        self.emit_with_update(vec![self.viewport_event()])
    }

    // -- z-order (§2.G) --------------------------------------------------------

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

    // -- freehand (§4) ---------------------------------------------------------

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
