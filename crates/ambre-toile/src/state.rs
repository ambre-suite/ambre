//! `ToileState` -- the per-document Toile engine (Logic thread side, §7/§8).
//!
//! Owns the single-board `LoroDoc`, the derived `SpatialIndex` and bounds /
//! layers caches, and turns typed [`ToileCommand`]s into a [`ToileOutcome`]
//! (display events + CRDT / ephemeral bytes). The `LoroDoc` never leaves this
//! thread; it exits only as exported bytes (§1).
//!
//! **Phase 0 scope.** Element / connector / stroke lifecycle, z-order, groups
//! (§2.F bis), local undo/redo (§8 bis), selection stats, viewport-culled
//! `DisplayViewport` derivation, and the remote-apply / snapshot paths. Frames
//! content, clipboard, find and nested-doc editing are subsequent slices; every
//! command handled here is complete.

use std::collections::{HashMap, HashSet};

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use loro::{ExportMode, LoroDoc, LoroMap, LoroMovableList, LoroValue, UndoManager, VersionVector};

use crate::command::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, End, InitContent,
    SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome, ZTarget,
};
use crate::connector::Connector;
use crate::index::{ElementBounds, SpatialIndex, Viewport};
use crate::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType,
    ShapeKind, StrokePoint,
};
use crate::stroke::smooth_stroke;

// Root container names (§2.B).
const ELEMENTS: &str = "elements";
const CONNECTORS: &str = "connectors";
const LAYERS: &str = "layers";
const BOARD: &str = "board";
/// Per-group / per-frame membership list key inside an element map (§2.F bis).
const CHILDREN: &str = "children";

/// Per-document Toile engine.
pub struct ToileState {
    doc: LoroDoc,
    index: SpatialIndex,
    /// Rotation-expanded world AABB per element; feeds culling + connector
    /// routing. Derived, never authoritative.
    bounds: HashMap<ElementId, Rect>,
    /// Materialized paint order (§2.G), mirrored from the `layers` list.
    layers: Vec<NodeId>,
    /// Per-session monotonic id-minting counter (§2.A).
    counter: u32,
    peer: u64,
    /// Last viewport the UI reported; drives culling.
    viewport: Viewport,
    /// Version vector already exported to Sync; deltas are `export(Updates{from})`.
    exported_vv: VersionVector,
    /// Local-only undo/redo (§8 bis): undoes only this peer's operations, one
    /// item per atomic user action (one `doc.commit()` per mutating command).
    undo: UndoManager,
}

impl Default for ToileState {
    fn default() -> Self {
        Self::new()
    }
}

impl ToileState {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        // Board metadata is scaffolding; commit it before any user edit.
        let board = doc.get_map(BOARD);
        let _ = board.insert("bg_color", 0xFF1E_1E1Eu32 as i64);
        let _ = board.insert("grid_size", 32.0);
        let _ = board.insert("grid_snap", false);
        doc.commit();
        let peer = doc.peer_id();
        // Create the undo manager AFTER the initial commit: the board scaffolding
        // is document setup, not a user edit, so it must not sit on the stack.
        let undo = UndoManager::new(&doc);
        Self {
            doc,
            index: SpatialIndex::default(),
            bounds: HashMap::new(),
            layers: Vec::new(),
            counter: 0,
            peer,
            viewport: Viewport::default(),
            exported_vv: VersionVector::default(),
            undo,
        }
    }

    // -- id minting (§2.A) -----------------------------------------------------

    fn mint_element_id(&mut self) -> ElementId {
        self.counter += 1;
        ElementId::mint(self.peer, self.counter)
    }

    fn mint_connector_id(&mut self) -> ConnectorId {
        self.counter += 1;
        ConnectorId::mint(self.peer, self.counter)
    }

    // -- command dispatch ------------------------------------------------------

    pub fn handle_command(&mut self, cmd: ToileCommand) -> ToileOutcome {
        match cmd {
            ToileCommand::CreateElement {
                kind,
                at,
                size,
                content,
            } => self.create_element(kind, at, size, content),
            ToileCommand::MoveElements { ids, delta } => self.move_elements(&ids, delta),
            ToileCommand::ResizeElement { id, new } => self.resize_element(&id, new),
            ToileCommand::RotateElement { id, radians } => self.rotate_element(&id, radians),
            ToileCommand::RestyleElements { ids, style } => self.restyle_elements(&ids, style),
            ToileCommand::DeleteElements { ids } => self.delete_elements(&ids),
            ToileCommand::LockElements { ids, locked } => self.lock_elements(&ids, locked),
            ToileCommand::Reorder { node, to } => self.reorder(node, to),
            ToileCommand::GroupElements { ids } => self.group_elements(&ids),
            ToileCommand::UngroupElements { group } => self.ungroup_elements(&group),
            ToileCommand::Undo => self.apply_history(true),
            ToileCommand::Redo => self.apply_history(false),
            ToileCommand::CreateConnector {
                from,
                from_anch,
                to,
                to_anch,
                path_type,
                arrows,
            } => self.create_connector(from, from_anch, to, to_anch, path_type, arrows),
            ToileCommand::RetargetConnector {
                id,
                endpoint,
                to,
                anch,
            } => self.retarget_connector(id, endpoint, to, anch),
            ToileCommand::DeleteConnector { id } => self.delete_connector(id),
            ToileCommand::CommitStroke { points, style } => self.commit_stroke(points, style),
            ToileCommand::RequestSelectionStats { ids } => self.selection_stats(&ids),
            ToileCommand::RequestSnapshot => ToileOutcome::display_only(Vec::new()),
            ToileCommand::SetViewport(vp) => {
                self.viewport = vp;
                ToileOutcome::display_only(vec![self.viewport_event()])
            }
        }
    }

    // -- remote / persistence (§8) --------------------------------------------

    /// Apply a remote delta batch or full snapshot (`doc.import` accepts both),
    /// rebuild the derived caches, and return the display events to repaint. A
    /// remote import is never re-broadcast.
    pub fn apply_remote(&mut self, update: &[u8]) -> Vec<ToileEvent> {
        if let Err(e) = self.doc.import(update) {
            tracing::warn!("Toile: remote import failed: {e}");
            return vec![ToileEvent::Error(ToileError::NoSuchElement(String::new()))];
        }
        self.rebuild_caches();
        vec![self.viewport_event()]
    }

    /// Full CRDT snapshot for local persistence / compaction / SnapshotNeeded.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
    }

    // -- element lifecycle -----------------------------------------------------

    fn create_element(
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
            _ => {}
        }
    }

    fn move_elements(&mut self, ids: &[ElementId], delta: Vec2) -> ToileOutcome {
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

    fn resize_element(&mut self, id: &ElementId, new: Rect) -> ToileOutcome {
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

    fn rotate_element(&mut self, id: &ElementId, radians: f64) -> ToileOutcome {
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

    fn restyle_elements(&mut self, ids: &[ElementId], style: ElementStyle) -> ToileOutcome {
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

    fn lock_elements(&mut self, ids: &[ElementId], locked: bool) -> ToileOutcome {
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

    fn delete_elements(&mut self, ids: &[ElementId]) -> ToileOutcome {
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
        self.doc.commit();

        for id in &expanded {
            self.bounds.remove(id);
            self.index.remove(id);
        }
        self.rematerialize_layers();
        self.recompute_groups();
        self.emit_with_update(vec![self.viewport_event()])
    }

    // -- z-order (§2.G) --------------------------------------------------------

    fn reorder(&mut self, node: NodeId, to: ZTarget) -> ToileOutcome {
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

    // -- groups (§2.F bis) -----------------------------------------------------

    /// Bind `ids` into a fresh group element. Members already in another group
    /// are first removed from it (a node belongs to at most one group). The group
    /// `NodeId` is inserted into `layers` just below its lowest member so the
    /// members keep painting above it in their current relative order.
    fn group_elements(&mut self, ids: &[ElementId]) -> ToileOutcome {
        // Keep only existing, distinct members, in the request order.
        let mut members: Vec<ElementId> = Vec::new();
        for id in ids {
            if self.bounds.contains_key(id) && !members.contains(id) {
                members.push(id.clone());
            }
        }
        if members.len() < 2 {
            return ToileOutcome::display_only(Vec::new());
        }
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

        // Detach each member from its current group, if any.
        for m in &members {
            if let Some(g) = self.enclosing_group(m) {
                self.remove_child(&g, m);
            }
        }

        let id = self.mint_element_id();
        let elems = self.doc.get_map(ELEMENTS);
        let Ok(em) = elems.insert_container(id.as_str(), LoroMap::new()) else {
            return self.err_no_element(&id);
        };
        em_seed_group(&em);
        let Ok(children) = em.insert_container(CHILDREN, LoroMovableList::new()) else {
            return self.err_no_element(&id);
        };
        for m in &members {
            let _ = children.push(m.as_str());
        }
        // Insert the group NodeId just below its lowest member.
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .insert(insert_at, NodeId::Element(id.clone()).encode().as_str());
        self.doc.commit();

        self.rematerialize_layers();
        // Seed the group's derived bounds from its members, then feed the R-tree.
        self.recompute_groups();
        self.emit_with_update(vec![ToileEvent::ElementCreated(id), self.viewport_event()])
    }

    /// Drop a group's binding: remove the group node and re-parent its children
    /// to the group's own parent (an enclosing group, or the board). The children
    /// survive with their `layers` order intact (§2.F bis).
    fn ungroup_elements(&mut self, group: &ElementId) -> ToileOutcome {
        let Some(gm) = self.element_map(group) else {
            return self.err_no_element(group);
        };
        if str_of(&gm, "type") != "group" {
            return ToileOutcome::display_only(Vec::new());
        }
        let children = self.children_of(group);
        let parent = self.enclosing_group(group);

        // Remove the group element and its layers node.
        let _ = self.doc.get_map(ELEMENTS).delete(group.as_str());
        self.strip_layers(|node| !matches!(node, NodeId::Element(e) if e == group));

        // Re-parent the children to the enclosing group (board parent is implicit).
        if let Some(p) = &parent {
            let plist = self.children_list(p);
            if let Some(list) = plist {
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

    // -- undo / redo (§8 bis) --------------------------------------------------

    /// Apply one undo or redo step. The inverse ops are committed by
    /// `undo()`/`redo()`; reseed the derived caches and broadcast the delta like
    /// any structural edit. A no-op (empty stack) only reports availability.
    fn apply_history(&mut self, undo: bool) -> ToileOutcome {
        let changed = if undo {
            self.undo.undo()
        } else {
            self.undo.redo()
        }
        .unwrap_or(false);
        if !changed {
            return ToileOutcome::display_only(vec![self.undo_state_event()]);
        }
        self.rebuild_caches();
        self.emit_with_update(vec![self.viewport_event(), self.undo_state_event()])
    }

    fn undo_state_event(&self) -> ToileEvent {
        ToileEvent::UndoState {
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
        }
    }

    // -- connectors (§5) -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn create_connector(
        &mut self,
        from: ElementId,
        from_anch: Anchor,
        to: ElementId,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    ) -> ToileOutcome {
        let id = self.mint_connector_id();
        let conns = self.doc.get_map(CONNECTORS);
        let Ok(cm) = conns.insert_container(id.as_str(), LoroMap::new()) else {
            return self.err_no_connector(&id);
        };
        write_connector(&cm, &from, from_anch, &to, to_anch, path_type, arrows);
        let _ = self
            .doc
            .get_movable_list(LAYERS)
            .push(NodeId::Connector(id.clone()).encode().as_str());
        self.doc.commit();
        self.layers.push(NodeId::Connector(id));
        self.emit_with_update(vec![self.viewport_event()])
    }

    fn retarget_connector(
        &mut self,
        id: ConnectorId,
        endpoint: End,
        to: ElementId,
        anch: Anchor,
    ) -> ToileOutcome {
        let Some(cm) = self.connector_map(&id) else {
            return self.err_no_connector(&id);
        };
        match endpoint {
            End::From => {
                let _ = cm.insert("from", to.as_str());
                let _ = cm.insert("from_anch", encode_anchor(anch));
            }
            End::To => {
                let _ = cm.insert("to", to.as_str());
                let _ = cm.insert("to_anch", encode_anchor(anch));
            }
        }
        self.doc.commit();
        // A retarget to a missing element surfaces as dangling at derivation.
        let mut events = vec![self.viewport_event()];
        if !self.bounds.contains_key(&to) {
            events.push(ToileEvent::DanglingConnector(id));
        }
        self.emit_with_update(events)
    }

    fn delete_connector(&mut self, id: ConnectorId) -> ToileOutcome {
        let conns = self.doc.get_map(CONNECTORS);
        if conns.get(id.as_str()).is_none() {
            return self.err_no_connector(&id);
        }
        let _ = conns.delete(id.as_str());
        self.strip_layers(|node| !matches!(node, NodeId::Connector(c) if c == &id));
        self.doc.commit();
        self.rematerialize_layers();
        self.emit_with_update(vec![self.viewport_event()])
    }

    // -- freehand (§4) ---------------------------------------------------------

    fn commit_stroke(&mut self, points: Vec<StrokePoint>, style: ElementStyle) -> ToileOutcome {
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

    // -- selection aggregates (§8 quinquies) -----------------------------------

    fn selection_stats(&self, ids: &[ElementId]) -> ToileOutcome {
        let mut union: Option<Rect> = None;
        let mut count = 0u32;
        for id in ids {
            if let Some(r) = self.bounds.get(id) {
                union = Some(match union {
                    Some(u) => u.union(*r),
                    None => *r,
                });
                count += 1;
            }
        }
        ToileOutcome::display_only(vec![ToileEvent::SelectionStats(SelectionStats {
            count,
            bounds: union,
        })])
    }

    // -- derivation (§9) -------------------------------------------------------

    fn viewport_event(&self) -> ToileEvent {
        ToileEvent::ViewportData(self.derive_viewport())
    }

    /// Build the culled, z-ordered `DisplayViewport` for the current viewport.
    fn derive_viewport(&self) -> DisplayViewport {
        let visible: HashSet<ElementId> = self.index.query_visible(&self.viewport).collect();
        let mut nodes = Vec::new();
        for node in &self.layers {
            match node {
                NodeId::Element(id) if visible.contains(id) => {
                    if let Some(de) = self.display_element(id) {
                        nodes.push(DisplayNode::Element(de));
                    }
                }
                NodeId::Connector(id) => {
                    if let Some(dc) = self.display_connector(id) {
                        nodes.push(DisplayNode::Connector(dc));
                    }
                }
                NodeId::Element(_) => {} // culled
            }
        }
        DisplayViewport {
            viewport: self.viewport,
            nodes,
        }
    }

    fn display_element(&self, id: &ElementId) -> Option<DisplayElement> {
        let em = self.element_map(id)?;
        // A group has no stored geometry: its bounds are the derived member union
        // (§2.F bis). Read them from the derived cache.
        let is_group = str_of(&em, "type") == "group";
        let (x, y, w, h) = if is_group {
            let r = self.bounds.get(id).copied().unwrap_or_default();
            (r.x0, r.y0, r.width(), r.height())
        } else {
            (
                f64_of(&em, "x"),
                f64_of(&em, "y"),
                f64_of(&em, "w"),
                f64_of(&em, "h"),
            )
        };
        let rot = f64_of(&em, "rot");
        let locked = bool_of(&em, "locked");
        let style = ElementStyle::from_bytes(&bin_of(&em, "style"));
        let type_tag = str_of(&em, "type");
        let shape_kind = ShapeKind::from_tag(&str_of(&em, "shape"));
        let kind = kind_from_tags(&type_tag, shape_kind);
        let bounds = Rect::new(x, y, x + w, y + h);
        // World transform: rotation about the AABB center.
        let center = bounds.center();
        let transform = Affine::translate(center.to_vec2())
            * Affine::rotate(rot)
            * Affine::translate(-center.to_vec2());
        let geom = self.geom_for(&em, kind, bounds);
        Some(DisplayElement {
            id: id.clone(),
            kind,
            transform,
            bounds,
            locked,
            style,
            geom,
        })
    }

    fn geom_for(&self, em: &LoroMap, kind: ElementKind, bounds: Rect) -> DisplayGeom {
        match kind {
            ElementKind::Shape(ShapeKind::Rect) => DisplayGeom::Rect,
            ElementKind::Shape(ShapeKind::Ellipse) => DisplayGeom::Ellipse,
            ElementKind::Shape(ShapeKind::Triangle) => DisplayGeom::Triangle,
            ElementKind::Shape(ShapeKind::Line) => {
                let mut p = BezPath::new();
                p.move_to((bounds.x0, bounds.y0));
                p.line_to((bounds.x1, bounds.y1));
                DisplayGeom::Path(p)
            }
            ElementKind::Shape(ShapeKind::Path) => {
                DisplayGeom::Path(BezPath::from_svg(&str_of(em, "path")).unwrap_or_default())
            }
            ElementKind::Image => DisplayGeom::Image(str_of(em, "hash")),
            ElementKind::Embed => DisplayGeom::Embed(str_of(em, "url")),
            ElementKind::DocEmbed => {
                // Parse back the reference; on any parse failure fall to Frame so
                // the node still paints a placeholder rather than vanishing.
                match (
                    ambre_core::Uuid::parse_str(&str_of(em, "doc_id")),
                    doc_type_from_tag(&str_of(em, "doc_type")),
                ) {
                    (Ok(document_id), Some(doc_type)) => DisplayGeom::DocEmbed {
                        document_id,
                        doc_type,
                        focused: false,
                    },
                    _ => DisplayGeom::Frame,
                }
            }
            ElementKind::Frame => DisplayGeom::Frame,
            ElementKind::Group => DisplayGeom::Group,
            ElementKind::Sticky | ElementKind::Text => DisplayGeom::Text(str_of(em, "text")),
            ElementKind::Drawing => {
                let pts: Vec<StrokePoint> =
                    postcard::from_bytes(&bin_of(em, "points")).unwrap_or_default();
                let mut out = Vec::new();
                smooth_stroke(&pts, &mut out);
                DisplayGeom::Stroke(out)
            }
        }
    }

    fn display_connector(&self, id: &ConnectorId) -> Option<DisplayConnector> {
        let cm = self.connector_map(id)?;
        let conn = read_connector(id.clone(), &cm)?;
        let path = conn.route(&self.bounds)?; // dropped if an endpoint is gone (§5.A)
        Some(DisplayConnector {
            id: id.clone(),
            path,
            style: ElementStyle::default(),
            arrows: conn.arrows,
        })
    }

    // -- cache maintenance -----------------------------------------------------

    /// Recompute one element's AABB from its map and mirror it into `bounds` and
    /// the R-tree.
    fn update_bounds(&mut self, id: &ElementId, em: &LoroMap) {
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

    /// Full rebuild of every derived cache from `doc` (§1). Used after a remote
    /// import; the incremental path (local commands) keeps them in step directly.
    fn rebuild_caches(&mut self) {
        self.bounds.clear();
        let elems = self.doc.get_map(ELEMENTS);
        let mut leaves = Vec::new();
        if let LoroValue::Map(m) = elems.get_deep_value() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(fields) = v {
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

    /// Re-read the `layers` list into `self.layers`, skipping malformed tags and
    /// de-duplicating (keep first) per §2.H.
    fn rematerialize_layers(&mut self) {
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
    /// deletion cleanup (§2.H). Rebuilds the list in place.
    fn strip_layers(&self, keep: impl Fn(&NodeId) -> bool) {
        let list = self.doc.get_movable_list(LAYERS);
        // Collect survivors from the current list value, then rewrite.
        let mut survivors = Vec::new();
        if let LoroValue::List(cur) = list.get_deep_value() {
            for v in cur.iter() {
                if let LoroValue::String(s) = v
                    && let Some(node) = NodeId::decode(s)
                    && keep(&node)
                {
                    survivors.push(node.encode());
                }
            }
        }
        let len = list.len();
        let _ = list.delete(0, len);
        for s in survivors {
            let _ = list.push(s.as_str());
        }
    }

    // -- small helpers ---------------------------------------------------------

    /// The `LoroMap` handle for an element, or `None` if absent.
    fn element_map(&self, id: &ElementId) -> Option<LoroMap> {
        self.doc
            .get_map(ELEMENTS)
            .get(id.as_str())?
            .into_container()
            .ok()?
            .into_map()
            .ok()
    }

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
    fn children_of(&self, id: &ElementId) -> Vec<ElementId> {
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
    /// most one group, §2.F bis).
    fn enclosing_group(&self, id: &ElementId) -> Option<ElementId> {
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
    fn remove_child(&self, group: &ElementId, child: &ElementId) {
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
    fn collect_leaves(&self, id: &ElementId, out: &mut HashSet<ElementId>) {
        if self.is_group(id) {
            for c in self.children_of(id) {
                self.collect_leaves(&c, out);
            }
        } else {
            out.insert(id.clone());
        }
    }

    /// Append `id` and every descendant (groups included) to `out`.
    fn collect_subtree(&self, id: &ElementId, out: &mut HashSet<ElementId>) {
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

    /// Recompute every group's derived bounds as the union of its members', to a
    /// fixpoint (nested groups resolve bottom-up). Group geometry is derived and
    /// lives only in the `bounds` / R-tree caches, never in the CRDT (§2.F bis).
    fn recompute_groups(&mut self) {
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

    fn connector_map(&self, id: &ConnectorId) -> Option<LoroMap> {
        self.doc
            .get_map(CONNECTORS)
            .get(id.as_str())?
            .into_container()
            .ok()?
            .into_map()
            .ok()
    }

    /// Ids of every connector referencing one of `del` at either endpoint.
    fn connectors_touching(&self, del: &HashSet<&ElementId>) -> HashSet<ConnectorId> {
        let mut out = HashSet::new();
        if let LoroValue::Map(m) = self.doc.get_map(CONNECTORS).get_deep_value() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(f) = v {
                    let from = map_str(f, "from");
                    let to = map_str(f, "to");
                    if del.contains(&ElementId(from)) || del.contains(&ElementId(to)) {
                        out.insert(ConnectorId(key.clone()));
                    }
                }
            }
        }
        out
    }

    /// Wrap the current export delta (since `exported_vv`) into the outcome and
    /// advance the version vector. Every mutating command routes through here.
    fn emit_with_update(&mut self, display: Vec<ToileEvent>) -> ToileOutcome {
        let update = self
            .doc
            .export(ExportMode::updates(&self.exported_vv))
            .ok()
            .filter(|b| !b.is_empty());
        self.exported_vv = self.doc.oplog_vv();
        ToileOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    fn err_no_element(&self, id: &ElementId) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchElement(
            id.0.clone(),
        ))])
    }

    fn err_no_connector(&self, id: &ConnectorId) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchConnector(
            id.0.clone(),
        ))])
    }

    // -- test / integration accessors -----------------------------------------

    /// Number of elements currently in the CRDT. Test helper.
    pub fn element_count(&self) -> usize {
        self.bounds.len()
    }

    /// Materialized paint order. Test helper.
    pub fn layer_order(&self) -> &[NodeId] {
        &self.layers
    }

    /// Derived world AABB of an element. Test helper.
    pub fn bounds_of(&self, id: &ElementId) -> Option<Rect> {
        self.bounds.get(id).copied()
    }
}

// -----------------------------------------------------------------------------
// Free helpers
// -----------------------------------------------------------------------------

/// Rotation-expanded axis-aligned bounds of a rect rotated by `rot` about its
/// center (§3). For `rot == 0` this is the rect itself.
fn expanded_aabb(x: f64, y: f64, w: f64, h: f64, rot: f64) -> Rect {
    let r = Rect::new(x, y, x + w, y + h);
    if rot == 0.0 {
        return r;
    }
    let c = r.center();
    let a = Affine::rotate_about(rot, c);
    let corners = [
        Point::new(r.x0, r.y0),
        Point::new(r.x1, r.y0),
        Point::new(r.x1, r.y1),
        Point::new(r.x0, r.y1),
    ];
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for corner in corners {
        let p = a * corner;
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
    }
    Rect::new(min.x, min.y, max.x, max.y)
}

/// World AABB spanning a stroke's raw samples (with a small margin so a single
/// point still has non-zero area for culling).
fn stroke_aabb(points: &[StrokePoint]) -> Rect {
    if points.is_empty() {
        return Rect::new(0.0, 0.0, 1.0, 1.0);
    }
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        min.x = min.x.min(p.x as f64);
        min.y = min.y.min(p.y as f64);
        max.x = max.x.max(p.x as f64);
        max.y = max.y.max(p.y as f64);
    }
    Rect::new(
        min.x,
        min.y,
        (max.x).max(min.x + 1.0),
        (max.y).max(min.y + 1.0),
    )
}

/// Seed a group element's scalar fields (§2.F bis). A group paints nothing and
/// its geometry is a derived union, so it carries no x/y/w/h: only the type flag,
/// lock state, rotation and default style live in the CRDT; the `children` list
/// is attached by the caller.
fn em_seed_group(em: &LoroMap) {
    let _ = em.insert("type", "group");
    let _ = em.insert("rot", 0.0);
    let _ = em.insert("locked", false);
    let _ = em.insert(
        "style",
        LoroValue::Binary(ElementStyle::default().to_bytes().into()),
    );
}

fn content_matches(kind: ElementKind, content: &InitContent) -> bool {
    matches!(
        (kind, content),
        (ElementKind::Shape(ShapeKind::Path), InitContent::Path(_))
            | (ElementKind::Shape(_), InitContent::None)
            | (ElementKind::Image, InitContent::Image(_))
            | (ElementKind::Embed, InitContent::Embed(_))
            | (ElementKind::DocEmbed, InitContent::DocEmbed { .. })
            | (
                ElementKind::Sticky | ElementKind::Text,
                InitContent::None | InitContent::Text(_)
            )
            | (ElementKind::Frame, InitContent::None)
    )
}

fn kind_from_tags(type_tag: &str, shape: Option<ShapeKind>) -> ElementKind {
    match type_tag {
        "shape" => ElementKind::Shape(shape.unwrap_or(ShapeKind::Rect)),
        "sticky" => ElementKind::Sticky,
        "text" => ElementKind::Text,
        "image" => ElementKind::Image,
        "frame" => ElementKind::Frame,
        "group" => ElementKind::Group,
        "drawing" => ElementKind::Drawing,
        "embed" => ElementKind::Embed,
        "doc_embed" => ElementKind::DocEmbed,
        _ => ElementKind::Shape(ShapeKind::Rect),
    }
}

fn doc_type_from_tag(s: &str) -> Option<ambre_core::DocType> {
    use ambre_core::DocType;
    match s {
        "Trame" => Some(DocType::Trame),
        "Prose" => Some(DocType::Prose),
        "Toile" => Some(DocType::Toile),
        "Murmure" => Some(DocType::Murmure),
        _ => None,
    }
}

// -- connector map (de)serialization ------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_connector(
    cm: &LoroMap,
    from: &ElementId,
    from_anch: Anchor,
    to: &ElementId,
    to_anch: Anchor,
    path_type: PathType,
    arrows: EndCaps,
) {
    let _ = cm.insert("from", from.as_str());
    let _ = cm.insert("from_anch", encode_anchor(from_anch));
    let _ = cm.insert("to", to.as_str());
    let _ = cm.insert("to_anch", encode_anchor(to_anch));
    let _ = cm.insert("path_type", encode_path_type(path_type));
    let _ = cm.insert("arrow_start", arrows.start);
    let _ = cm.insert("arrow_end", arrows.end);
}

fn read_connector(id: ConnectorId, cm: &LoroMap) -> Option<Connector> {
    Some(Connector {
        id,
        from: ElementId(str_of(cm, "from")),
        from_anch: decode_anchor(&str_of(cm, "from_anch")),
        to: ElementId(str_of(cm, "to")),
        to_anch: decode_anchor(&str_of(cm, "to_anch")),
        path_type: decode_path_type(&str_of(cm, "path_type")),
        arrows: EndCaps {
            start: bool_of(cm, "arrow_start"),
            end: bool_of(cm, "arrow_end"),
        },
    })
}

/// Anchors are encoded as compact strings; `Free` carries its `(u, v)`.
fn encode_anchor(a: Anchor) -> String {
    match a {
        Anchor::Top => "t".into(),
        Anchor::Bottom => "b".into(),
        Anchor::Left => "l".into(),
        Anchor::Right => "r".into(),
        Anchor::Center => "c".into(),
        Anchor::Free { u, v } => format!("f:{u}:{v}"),
    }
}

fn decode_anchor(s: &str) -> Anchor {
    match s {
        "t" => Anchor::Top,
        "b" => Anchor::Bottom,
        "l" => Anchor::Left,
        "r" => Anchor::Right,
        "c" => Anchor::Center,
        _ => {
            let mut it = s.split(':');
            if it.next() == Some("f") {
                let u = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.5);
                let v = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.5);
                Anchor::Free { u, v }
            } else {
                Anchor::Center
            }
        }
    }
}

fn encode_path_type(p: PathType) -> &'static str {
    match p {
        PathType::Straight => "s",
        PathType::Elbow => "e",
        PathType::Curved => "c",
    }
}

fn decode_path_type(s: &str) -> PathType {
    match s {
        "e" => PathType::Elbow,
        "c" => PathType::Curved,
        _ => PathType::Straight,
    }
}

// -- LoroMap handle field readers ---------------------------------------------

fn f64_of(m: &LoroMap, k: &str) -> f64 {
    match m.get(k).map(|v| v.into_value()) {
        Some(Ok(LoroValue::Double(d))) => d,
        _ => 0.0,
    }
}

fn bool_of(m: &LoroMap, k: &str) -> bool {
    matches!(
        m.get(k).map(|v| v.into_value()),
        Some(Ok(LoroValue::Bool(true)))
    )
}

fn str_of(m: &LoroMap, k: &str) -> String {
    match m.get(k).map(|v| v.into_value()) {
        Some(Ok(LoroValue::String(s))) => s.to_string(),
        _ => String::new(),
    }
}

fn bin_of(m: &LoroMap, k: &str) -> Vec<u8> {
    match m.get(k).map(|v| v.into_value()) {
        Some(Ok(LoroValue::Binary(b))) => b.to_vec(),
        _ => Vec::new(),
    }
}

// -- deep-value (LoroMapValue) field readers ----------------------------------
//
// Used on the from-scratch rebuild path (`rebuild_caches`), which reads whole
// container deep-values rather than per-field handles.

fn map_f64(m: &loro::LoroMapValue, k: &str) -> f64 {
    match m.get(k) {
        Some(LoroValue::Double(d)) => *d,
        _ => 0.0,
    }
}

fn map_str(m: &loro::LoroMapValue, k: &str) -> String {
    match m.get(k) {
        Some(LoroValue::String(s)) => s.to_string(),
        _ => String::new(),
    }
}
