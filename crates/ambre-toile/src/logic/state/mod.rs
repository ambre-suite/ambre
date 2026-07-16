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
//! - [`presentation`] -- slide sequence CRDT + local playback (`toile-features` §1)
//! - [`connectors`] -- connector lifecycle and queries (§5)
//! - [`clipboard`] -- copy / paste / duplicate (§8 ter)
//! - [`caches`] -- derived `bounds` / index / `layers` maintenance (§1)
//! - [`derive`] -- read-only `DisplayViewport` derivation (§9)
//! - [`codec`] -- pure geometry / (de)serialization helpers

mod autolayout;
mod boolean;
mod caches;
mod clipboard;
mod codec;
mod connectors;
mod derive;
mod elements;
mod find;
mod groups;
mod presence;
mod presentation;
mod reactions;

use std::collections::HashMap;

use kurbo::Rect;
use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use ambre_bus::UserId;

use crate::logic::command::{
    PresencePayload, SelectionStats, ToileCommand, ToileError, ToileEvent, ToileOutcome,
};
use crate::logic::index::{SpatialIndex, Viewport};
use crate::logic::model::{ConnectorId, ElementId, NodeId, PlaybackState};

// Root container names (§2.B).
const ELEMENTS: &str = "elements";
const CONNECTORS: &str = "connectors";
const LAYERS: &str = "layers";
const BOARD: &str = "board";
/// Per-group / per-frame membership list key inside an element map (§2.F bis).
const CHILDREN: &str = "children";
/// Collaborative speaker-notes LoroText key inside a frame element map (§1.D).
const NOTES: &str = "notes";
/// Auto-layout spec LWW register key inside a frame element map (§2).
const AUTOLAYOUT: &str = "autolayout";
/// Presentation root map key (`toile-features` §1.A).
const PRESENTATION: &str = "presentation";
/// Slide sequence movable-list key inside the `presentation` map (§1.A).
const ORDER: &str = "order";
/// Presentation config map key inside the `presentation` map (§1.A).
const CONFIG: &str = "config";
/// Reactions root map key: `reactions[target_id][user_id]` (§4.A).
const REACTIONS: &str = "reactions";

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
    /// Canonical per-user identifier, seeded from `LogicEvent::SessionReady`
    /// (§7) by the dispatcher. `None` until the session is authenticated. A
    /// stable `UserId`, NOT `peer` (which is per-session/per-device and only
    /// mints element ids); used to key this peer's own persisted reactions
    /// (`toile-features §4.A`) and to tag outbound presence with a stable
    /// identity. Same field as `trame-logic §7`.
    my_user_id: Option<UserId>,
    /// Remote peers' latest presence (cursors, live strokes, lasers, selections),
    /// keyed by canonical `UserId` (§7). Ephemeral: never in Loro, never
    /// persisted, rebuilt from inbound frames. The local peer's own presence is
    /// not stored here (the UI draws its own cursor).
    presence: HashMap<UserId, PresencePayload>,
    /// Presentation (focus) mode toggle (`toile-features` §1.B). Local/per-viewer,
    /// never in Loro and never synced -- only the slide sequence is shared.
    present: bool,
    /// Local playback cursor + focus flag (§1.B). Momentary-gesture state, like
    /// [`present`](Self::present); reset on `EnterPresent`, clamped by
    /// `SetSlideOrder`.
    playback: PlaybackState,
    /// "Follow presenter" broadcast toggle (§1.B): when set, the playback commands
    /// additionally emit an ephemeral frame carrying the active slide index so
    /// followers glide along. Local role, never in Loro and never synced.
    broadcasting: bool,
    /// The presenter this viewer is following (§1.B), or `None`. When `Some(u)`,
    /// an inbound presence frame from `u` that carries a `present_slide` moves this
    /// viewer's active slide; a manual navigation clears it ("take control").
    /// Local role, never in Loro and never synced.
    following: Option<UserId>,
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
            my_user_id: None,
            presence: HashMap::new(),
            present: false,
            playback: PlaybackState::default(),
            broadcasting: false,
            following: None,
        }
    }

    // -- session identity (§7) -------------------------------------------------

    /// Seed the canonical `UserId` from `LogicEvent::SessionReady` (§7). Called
    /// by the dispatcher once the session is authenticated; idempotent (a
    /// re-login with the same account is a no-op). Keys this peer's reactions
    /// (`toile-features §4.A`) and tags outbound presence.
    pub fn set_user_id(&mut self, user_id: UserId) {
        self.my_user_id = Some(user_id);
    }

    /// The canonical `UserId` for this session, if the session is authenticated.
    pub fn user_id(&self) -> Option<UserId> {
        self.my_user_id
    }

    /// Whether this viewer is broadcasting its playback position (§1.B). Test /
    /// integration accessor.
    pub fn is_broadcasting(&self) -> bool {
        self.broadcasting
    }

    /// The presenter this viewer is currently following, if any (§1.B). Test /
    /// integration accessor.
    pub fn following(&self) -> Option<UserId> {
        self.following
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
            ToileCommand::SetText { id, text } => self.set_text(&id, &text),
            ToileCommand::LockElements { ids, locked } => self.lock_elements(&ids, locked),
            ToileCommand::Reorder { node, to } => self.reorder(node, to),
            ToileCommand::GroupElements { ids } => self.group_elements(&ids),
            ToileCommand::UngroupElements { group } => self.ungroup_elements(&group),
            ToileCommand::SetFrameChildren { frame, add, remove } => {
                self.set_frame_children(&frame, &add, &remove)
            }
            ToileCommand::SetSlideOrder { order } => self.set_slide_order(&order),
            ToileCommand::NextSlide => self.next_slide(),
            ToileCommand::PrevSlide => self.prev_slide(),
            ToileCommand::GotoSlide(idx) => self.goto_slide(idx),
            ToileCommand::EnterPresent { focus } => self.enter_present(focus),
            ToileCommand::ExitPresent => self.exit_present(),
            ToileCommand::SetPresentBroadcast(on) => self.set_present_broadcast(on),
            ToileCommand::FollowPresenter(who) => self.follow_presenter(who),
            ToileCommand::EditNotes { frame, delta } => self.edit_notes(&frame, &delta),
            ToileCommand::SetAutoLayout { frame, layout } => self.set_auto_layout(&frame, layout),
            ToileCommand::React { target, reaction } => self.react(&target, reaction),
            ToileCommand::BooleanOp {
                op,
                ids,
                keep_sources,
            } => self.boolean_op(op, &ids, keep_sources),
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
            ToileCommand::Find {
                query,
                options,
                from,
                backward,
            } => self.find(&query, options, from.as_ref(), backward),
            ToileCommand::RequestSelectionStats { ids } => self.selection_stats(&ids),
            ToileCommand::RequestSnapshot => ToileOutcome::display_only(Vec::new()),
            ToileCommand::SetViewport(vp) => {
                self.viewport = vp;
                ToileOutcome::display_only(vec![self.viewport_event()])
            }
            ToileCommand::Presence(payload) => self.broadcast_presence(&payload),
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
        // Empty ids = whole-board extent: the union of every element's bounds.
        // Used by the View -> Fit action (`toile-ui §5quinquies.E`), which needs
        // the full board rect the UI does not hold (it sees only the culled
        // viewport). A non-empty selection keeps the per-selection aggregate the
        // status bar consumes (§8bis).
        if ids.is_empty() {
            for r in self.bounds.values() {
                union = Some(match union {
                    Some(u) => u.union(*r),
                    None => *r,
                });
                count += 1;
            }
            return ToileOutcome::display_only(vec![ToileEvent::SelectionStats(SelectionStats {
                count,
                bounds: union,
            })]);
        }
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

    /// Overwrite one raw string field of an element's map, behind the engine's
    /// back. Test helper: it exists to forge the state a newer or broken peer
    /// could merge into us, which no command of ours can produce.
    pub fn debug_set_element_field(&mut self, id: &ElementId, key: &str, value: &str) {
        if let Some(em) = self.element_map(id) {
            let _ = em.insert(key, value);
        }
        self.doc.commit();
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
