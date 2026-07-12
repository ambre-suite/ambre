//! `ToileState` -- the per-document Toile engine (Logic thread side, §7/§8).
//!
//! Owns the single-board `LoroDoc`, the derived `SpatialIndex` and bounds /
//! layers caches, and turns typed [`ToileCommand`]s into a [`ToileOutcome`]
//! (display events + CRDT / ephemeral bytes). The `LoroDoc` never leaves this
//! thread; it exits only as exported bytes (§1).
//!
//! **Phase 0 scope.** Element / connector / stroke lifecycle, z-order, groups
//! (§2.F bis), frame membership (§2.F/§2.G), local undo/redo (§8 bis), clipboard
//! (§8 ter), selection stats, viewport-culled `DisplayViewport` derivation, and
//! the remote-apply / snapshot paths. Rich text (`EditText`, blocked on Prose's
//! shared `TextDelta`), find and nested-doc editing are subsequent slices; every
//! command handled here is complete.
//!
//! This module holds the state, the command dispatcher and the remote / undo /
//! persistence paths; the `impl` is split by responsibility across the sibling
//! modules below (each reaches the struct's fields, consts and shared helpers via
//! `super::`):
//! - [`elements`] -- element / stroke lifecycle and z-order (§8, §4, §2.G)
//! - [`groups`] -- grouping, frame membership, the group/frame tree (§2.F)
//! - [`connectors`] -- connector lifecycle and queries (§5)
//! - [`clipboard`] -- copy / paste / duplicate (§8 ter)
//! - [`caches`] -- derived `bounds` / index / `layers` maintenance (§1)
//! - [`derive`] -- read-only `DisplayViewport` derivation (§9)
//! - [`codec`] -- pure geometry / (de)serialization helpers

mod caches;
mod clipboard;
mod codec;
mod connectors;
mod derive;
mod elements;
mod groups;

use std::collections::HashMap;

use kurbo::Rect;
use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use crate::logic::command::{SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome};
use crate::logic::index::{SpatialIndex, Viewport};
use crate::logic::model::{ConnectorId, ElementId, NodeId};

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

    pub(super) fn mint_element_id(&mut self) -> ElementId {
        self.counter += 1;
        ElementId::mint(self.peer, self.counter)
    }

    pub(super) fn mint_connector_id(&mut self) -> ConnectorId {
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
            ToileCommand::SetFrameChildren { frame, add, remove } => {
                self.set_frame_children(&frame, &add, &remove)
            }
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
            ToileCommand::CopyElements { ids } => self.copy_elements(&ids),
            ToileCommand::PasteElements { payload, at } => self.paste_elements(&payload, at),
            ToileCommand::DuplicateElements { ids, delta } => self.duplicate_elements(&ids, delta),
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

    /// Apply a batch of remote deltas (a server history replay on open) as one
    /// `import` sweep + a single cache rebuild, so the UI repaints once instead
    /// of once per delta. Mirrors [`apply_remote`](Self::apply_remote) but
    /// amortizes the derived-cache rebuild over the whole batch. A failed import
    /// on one delta is logged and skipped; the surviving deltas still merge.
    pub fn apply_remote_batch(&mut self, updates: &[Vec<u8>]) -> Vec<ToileEvent> {
        if updates.is_empty() {
            return Vec::new();
        }
        let mut any_ok = false;
        for update in updates {
            match self.doc.import(update) {
                Ok(_) => any_ok = true,
                Err(e) => tracing::warn!("Toile: remote batch import failed: {e}"),
            }
        }
        if !any_ok {
            return vec![ToileEvent::Error(ToileError::NoSuchElement(String::new()))];
        }
        self.rebuild_caches();
        vec![self.viewport_event()]
    }

    /// Apply a remote ephemeral (presence) payload and return the presence events
    /// to surface. Presence overlay is a Phase 1 UI slice (`toile-ui §13`,
    /// `UserId`-keyed cursors / selections); until the `ToileEvent` presence
    /// variant lands there is nothing to decode, so this is an accepted no-op that
    /// keeps the dispatcher wiring complete (§8).
    pub fn apply_ephemeral(&mut self, _payload: &[u8]) -> Vec<ToileEvent> {
        Vec::new()
    }

    /// Full CRDT snapshot for local persistence / compaction / SnapshotNeeded.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
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

    pub(super) fn undo_state_event(&self) -> ToileEvent {
        ToileEvent::UndoState {
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
        }
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

    // -- outcome plumbing ------------------------------------------------------

    /// Wrap the current export delta (since `exported_vv`) into the outcome and
    /// advance the version vector. Every mutating command routes through here.
    pub(super) fn emit_with_update(&mut self, display: Vec<ToileEvent>) -> ToileOutcome {
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

    pub(super) fn err_no_element(&self, id: &ElementId) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchElement(
            id.0.clone(),
        ))])
    }

    pub(super) fn err_no_connector(&self, id: &ConnectorId) -> ToileOutcome {
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

    /// Membership of a frame / group, in `children` order. Test helper.
    pub fn membership_of(&self, id: &ElementId) -> Vec<ElementId> {
        self.children_of(id)
    }

    /// The culled, z-ordered `DisplayViewport` for the current viewport. Test
    /// helper (the production path returns it inside a `ToileEvent`).
    pub fn debug_viewport(&self) -> crate::logic::command::DisplayViewport {
        self.derive_viewport()
    }

    /// Connector ids currently in the CRDT, sorted. Test helper.
    pub fn connector_ids(&self) -> Vec<ConnectorId> {
        let mut ids = Vec::new();
        if let LoroValue::Map(m) = self.doc.get_map(CONNECTORS).get_deep_value() {
            ids.extend(m.keys().map(|k| ConnectorId(k.clone())));
        }
        ids.sort();
        ids
    }
}
