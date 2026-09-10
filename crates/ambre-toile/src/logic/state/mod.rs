//! `ToileState` -- the per-document Toile engine (Logic thread side,).
//!
//! Owns the single-board `LoroDoc`, the derived `SpatialIndex` and bounds /
//! layers caches, and turns typed [`ToileCommand`]s into a [`ToileOutcome`]
//! (display events + CRDT / ephemeral bytes). The `LoroDoc` never leaves this
//! thread; it exits only as exported bytes.
//!
//! **Phase 0 scope.** Element / connector / stroke lifecycle, z-order, groups
//! frame membership, local undo/redo, clipboard
//! selection stats, viewport-culled `DisplayViewport` derivation, and
//! the remote-apply / snapshot paths. Rich text (`EditText`, blocked on Prose's
//! shared `TextDelta`), find and nested-doc editing are subsequent slices; every
//! command handled here is complete.
//!
//! This module holds the state, the command dispatcher and the remote / undo /
//! persistence paths; the `impl` is split by responsibility across the sibling
//! modules below (each reaches the struct's fields, consts and shared helpers via
//! `super::`):
//! - [`elements`] -- element / stroke lifecycle and z-order
//! - [`groups`] -- grouping, frame membership, the group/frame tree
//! - [`presentation`] -- slide sequence CRDT + local playback (`toile-presentation`)
//! - [`connectors`] -- connector lifecycle and queries
//! - [`clipboard`] -- copy / paste / duplicate
//! - [`import`] -- atomic foreign-file import + media map seeding (`toile-import-export`)
//! - [`docfile`] -- native `.ambre` export / import (`ambre-spec-docfile.md`)
//! - [`comments`] -- anchored comment threads (`toile-comments`)
//! - [`caches`] -- derived `bounds` / index / `layers` maintenance
//! - [`derive`] -- read-only `DisplayViewport` derivation
//! - [`codec`] -- pure geometry / (de)serialization helpers

mod autolayout;
mod boolean;
mod caches;
mod clipboard;
mod codec;
mod comments;
mod connectors;
mod derive;
mod docfile;
mod elements;
mod export;
mod find;
/// The per-app half of `shape_of` / `canonical_dump` (`fixtures`). Fixture
/// code (I4): behind the non-default `fixtures` feature, never in a client build.
#[cfg(feature = "fixtures")]
mod fixtures;
mod groups;
mod import;
mod presence;
pub use presence::PRESENCE_TTL_MS;
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
use crate::logic::model::{ConnectorId, ElementId, FrameId, NodeId, PlaybackState};

// Root container names.
const ELEMENTS: &str = "elements";
const CONNECTORS: &str = "connectors";
const LAYERS: &str = "layers";
const BOARD: &str = "board";
/// Per-group / per-frame membership list key inside an element map.
const CHILDREN: &str = "children";
/// Collaborative speaker-notes LoroText key inside a frame element map.
const NOTES: &str = "notes";
/// Auto-layout spec LWW register key inside a frame element map.
const AUTOLAYOUT: &str = "autolayout";
/// Presentation root map key (`toile-presentation`).
const PRESENTATION: &str = "presentation";
/// Slide sequence movable-list key inside the `presentation` map.
const ORDER: &str = "order";
/// Presentation config map key inside the `presentation` map.
const CONFIG: &str = "config";
/// Reactions root map key: `reactions[target_id][user_id]`.
const REACTIONS: &str = "reactions";
/// Comment threads root map key: `comments[thread_id]` (`toile-comments`).
const COMMENTS: &str = "comments";
/// The commit origin every comment write carries. The `UndoManager` excludes
/// the prefix, so `Ctrl+Z` on the board never retracts a message a teammate has
/// already read (`TOILE-FEAT-017`).
const COMMENT_ORIGIN: &str = "cmt";
/// Media root map key: `media[blake2b(plaintext)]` -> `{w, h, mime, thumb}`
/// (`ambre-spec-toile-schema.md`). Created and populated by `toile-media`; an
/// `image` element's `hash` field is a key here. Dedup: N elements may share one
/// entry, so `seed_media` never overwrites an existing key.
const MEDIA: &str = "media";

/// Per-document Toile engine.
pub struct ToileState {
    doc: LoroDoc,
    index: SpatialIndex,
    /// Rotation-expanded world AABB per element; feeds culling + connector
    /// routing. Derived, never authoritative.
    bounds: HashMap<ElementId, Rect>,
    /// Materialized paint order, mirrored from the `layers` list.
    layers: Vec<NodeId>,
    /// Per-session monotonic id-minting counter.
    counter: u32,
    peer: u64,
    /// Last viewport the UI reported; drives culling.
    viewport: Viewport,
    /// Version vector already exported to Sync; deltas are `export(Updates{from})`.
    exported_vv: VersionVector,
    /// Local-only undo/redo: undoes only this peer's operations, one
    /// item per atomic user action (one `doc.commit()` per mutating command).
    undo: UndoManager,
    /// Canonical per-user identifier, seeded from `LogicEvent::SessionReady`
    /// by the dispatcher. `None` until the session is authenticated. A
    /// stable `UserId`, NOT `peer` (which is per-session/per-device and only
    /// mints element ids); used to key this peer's own persisted reactions
    /// (`toile-reactions`) and to tag outbound presence with a stable
    /// identity. Same field as `trame-engine`.
    my_user_id: Option<UserId>,
    /// This session's display name, seeded from `SessionReady` beside the
    /// `UserId` and stamped on every outbound presence sample
    /// (`TOILE-UI-076`). Cosmetic: it decides neither identity nor colour
    /// (`PRESENCE-004`).
    my_name: Option<String>,
    /// Remote peers' latest presence (cursors, live strokes, lasers, selections)
    /// and the local clock each was last seen at, keyed by canonical `UserId`.
    /// Ephemeral: never in Loro, never persisted, rebuilt from inbound frames.
    /// The local peer's own presence is not stored here (the UI draws its own
    /// cursor). A peer whose last sample is older than the TTL is dropped, which
    /// is how a departure travels with no departure signal (`TOILE-185`).
    presence: HashMap<UserId, (PresencePayload, i64)>,
    /// The latest wall clock a local `Presence` command carried, and the only
    /// clock this thread has (`toile-presence`). Monotonic: it never moves
    /// backwards, whatever the machine's own clock does.
    now_ms: i64,
    /// Presentation (focus) mode toggle (`toile-presentation`). Local/per-viewer,
    /// never in Loro and never synced -- only the slide sequence is shared.
    present: bool,
    /// Local playback cursor + focus flag. Momentary-gesture state, like
    /// [`present`](Self::present); reset on `EnterPresent`, clamped by
    /// `SetSlideOrder`.
    playback: PlaybackState,
    /// "Follow presenter" broadcast toggle: when set, the playback commands
    /// additionally emit an ephemeral frame carrying the active slide index so
    /// followers glide along. Local role, never in Loro and never synced.
    broadcasting: bool,
    /// The presenter this viewer is following, or `None`. When `Some(u)`,
    /// an inbound presence frame from `u` that carries a `present_slide` moves this
    /// viewer's active slide; a manual navigation clears it ("take control").
    /// Local role, never in Loro and never synced.
    following: Option<UserId>,
    /// Imports awaiting media-upload confirmations (`toile-media`). An
    /// import that carries raster media is NOT applied until every blob's real
    /// blake2b returns from Sync, so the whole import stays one atomic undo group
    /// Rare and short-lived; dies with the `DocState`. See `state::import`.
    pending_imports: Vec<import::PendingImport>,
}

impl Default for ToileState {
    fn default() -> Self {
        Self::new()
    }
}

impl ToileState {
    pub fn new() -> Self {
        Self::from_doc(LoroDoc::new())
    }

    /// A board whose CRDT identity and commit clock are **pinned**, so the same
    /// command sequence produces the same bytes twice (`FIXTURE-001`).
    ///
    /// Two of the three entropy sources of a Toile document are closed here; the
    /// third, `document_id`, belongs to the caller and is supplied to
    /// [`export_ambre_plaintext`](Self::export_ambre_plaintext).
    ///
    /// * **`PeerID`** -- `LoroDoc::new()` draws one at random, and
    ///   [`peer`](Self::peer) is cached at construction and minted into every
    ///   [`ElementId`] / [`ConnectorId`], so an unpinned peer changes the
    ///   board's *content*, not merely its encoding.
    /// * **Commit timestamps** -- Loro records one per commit unless told not to.
    ///
    /// Set before the board-scaffolding commit of [`new`](Self::new), so that op
    /// is attributed to `peer` like every op after it.
    ///
    /// **Never for a live document.** Two concurrent writers sharing a `PeerID`
    /// mint conflicting `OpID`s and corrupt the history; this exists for
    /// single-owner, offline, host-side reproduction (a fixture generator, a
    /// deterministic test). The client path is [`new`](Self::new).
    pub fn with_peer(peer: u64) -> Self {
        let doc = LoroDoc::new();
        // `set_peer_id` before any op: it is the peer the board scaffolding is
        // attributed to, and every minted element id captures it.
        doc.set_peer_id(peer)
            .expect("a fresh LoroDoc accepts a peer id");
        // No wall clock in the ops: a recorded timestamp would differ between two
        // runs of the same generator and defeat the byte diff of `fixtures`.
        doc.set_record_timestamp(false);
        Self::from_doc(doc)
    }

    /// Shared body of [`new`](Self::new) and [`with_peer`](Self::with_peer): the
    /// board scaffolding and every derived cache, over a `LoroDoc` the caller has
    /// already configured.
    fn from_doc(doc: LoroDoc) -> Self {
        // Board metadata is scaffolding; commit it before any user edit.
        let board = doc.get_map(BOARD);
        let _ = board.insert("bg_color", 0xFF1E_1E1Eu32 as i64);
        let _ = board.insert("grid_size", 32.0);
        let _ = board.insert("grid_snap", false);
        doc.commit();
        let peer = doc.peer_id();
        // Create the undo manager AFTER the initial commit: the board scaffolding
        // is document setup, not a user edit, so it must not sit on the stack.
        let mut undo = UndoManager::new(&doc);
        // Keep every comment write off the undo stack (`TOILE-FEAT-017`): undo
        // reverts what the user drew, never a message a teammate has read.
        undo.add_exclude_origin_prefix(COMMENT_ORIGIN);
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
            my_name: None,
            presence: HashMap::new(),
            now_ms: 0,
            present: false,
            playback: PlaybackState::default(),
            broadcasting: false,
            following: None,
            pending_imports: Vec::new(),
        }
    }

    // -- session identity -------------------------------------------------

    /// Seed the canonical `UserId` from `LogicEvent::SessionReady`. Called
    /// by the dispatcher once the session is authenticated; idempotent (a
    /// re-login with the same account is a no-op). Keys this peer's reactions
    /// (`toile-reactions`) and tags outbound presence.
    pub fn set_user_id(&mut self, user_id: UserId) {
        self.my_user_id = Some(user_id);
    }

    /// The canonical `UserId` for this session, if the session is authenticated.
    pub fn user_id(&self) -> Option<UserId> {
        self.my_user_id
    }

    /// Seed this session's display name, from the same `SessionReady` that
    /// carries the `UserId`. It rides every outbound presence sample so a
    /// collaborator reads a name rather than a hex prefix (`TOILE-UI-076`), and
    /// it decides nothing (`PRESENCE-004`).
    pub fn set_display_name(&mut self, name: String) {
        self.my_name = Some(name);
    }

    /// Whether this viewer is broadcasting its playback position. Test /
    /// integration accessor.
    pub fn is_broadcasting(&self) -> bool {
        self.broadcasting
    }

    /// The presenter this viewer is currently following, if any. Test /
    /// integration accessor.
    pub fn following(&self) -> Option<UserId> {
        self.following
    }

    // -- id minting -----------------------------------------------------

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
                fill,
            } => self.create_element(kind, at, size, content, fill),
            ToileCommand::MoveElements { ids, delta } => self.move_elements(&ids, delta),
            ToileCommand::AlignElements { ids, op } => self.align_elements(&ids, op),
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
            ToileCommand::CreateComment { anchor, body, ts } => {
                self.create_comment(&anchor, &body, ts)
            }
            ToileCommand::ReplyComment { thread, body, ts } => {
                self.reply_comment(&thread, &body, ts)
            }
            ToileCommand::EditComment {
                thread,
                message,
                delta,
            } => self.edit_comment(&thread, message, &delta),
            ToileCommand::ResolveThread { thread, resolved } => {
                self.resolve_thread(&thread, resolved)
            }
            ToileCommand::ReadComments { thread } => self.read_comments(thread.as_ref()),
            ToileCommand::BooleanOp {
                op,
                ids,
                keep_sources,
            } => self.boolean_op(op, &ids, keep_sources),
            ToileCommand::ReadDeck => self.read_deck(),
            // One answer per address the document resolves; an address it does
            // not carry is answered with nothing, so the element keeps the
            // placeholder it is already painting (`toile-media`).
            ToileCommand::RequestMedia { hashes } => ToileOutcome::display_only(
                hashes
                    .iter()
                    .filter_map(|h| {
                        self.decoded_thumbnail(h).map(|(width, height, pixels)| {
                            ToileEvent::MediaDecoded {
                                hash: h.clone(),
                                width,
                                height,
                                pixels,
                            }
                        })
                    })
                    .collect(),
            ),
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
            ToileCommand::SetLink { id, link } => self.set_link(&id, link),
            ToileCommand::RestyleConnector { id, style } => self.restyle_connector(id, style),
            ToileCommand::SetConnectorLabel { id, text } => self.set_connector_label(id, &text),
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
            ToileCommand::ImportSvg { bytes, at } => self.import_svg_bytes(&bytes, at),
            ToileCommand::ImportImage { bytes, mime: _, at } => self.import_image_bytes(&bytes, at),
            ToileCommand::ExportSvg => self.export_svg_document(),
            // Served by the Logic thread, which holds the base font faces the
            // render walk needs (`toile-pdf-export`, `TOILE-IO-029`). Its
            // dispatcher intercepts the command before the document lookup, so
            // this arm is reached only by a caller that drives `ToileState`
            // directly -- and it answers rather than falling silent.
            ToileCommand::ExportPdf => ToileOutcome::display_only(vec![ToileEvent::ExportFailed(
                "the deck is exported by the Logic thread, not by the board".to_owned(),
            )]),
            ToileCommand::ImportFigma { .. } | ToileCommand::ImportMiro { .. } => {
                // Figma / Miro converters land in a later interop slice; the board
                // is untouched until then (atomicity).
                ToileOutcome::display_only(vec![ToileEvent::ImportFailed(
                    "Figma / Miro import is not yet available".to_owned(),
                )])
            }
            ToileCommand::RequestSnapshot => ToileOutcome::display_only(Vec::new()),
            ToileCommand::SetViewport(vp) => {
                self.viewport = vp;
                ToileOutcome::display_only(vec![self.viewport_event()])
            }
            ToileCommand::Presence { payload, now_ms } => self.broadcast_presence(&payload, now_ms),
        }
    }

    // -- remote / persistence --------------------------------------------

    /// Apply a remote delta batch or full snapshot (`doc.import` accepts both),
    /// rebuild the derived caches, and return the display events to repaint. A
    /// remote import is never re-broadcast.
    pub fn apply_remote(&mut self, update: &[u8]) -> Vec<ToileEvent> {
        if let Err(e) = self.doc.import(update) {
            tracing::warn!("Toile: remote import failed: {e}");
            return vec![ToileEvent::Error(ToileError::NoSuchElement(String::new()))];
        }
        let deck_before = self.slide_order();
        self.rebuild_caches();
        let mut events = vec![self.viewport_event()];
        events.extend(self.deck_change(&deck_before));
        events
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
        let deck_before = self.slide_order();
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
        let mut events = vec![self.viewport_event()];
        events.extend(self.deck_change(&deck_before));
        events
    }

    /// Full CRDT snapshot for local persistence / compaction / SnapshotNeeded.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
    }

    /// Import a full CRDT snapshot (local restore, or a fixture loaded through
    /// the production importer) and return the display events the UI needs.
    ///
    /// The counterpart of [`export_snapshot`](Self::export_snapshot), and the
    /// Toile half of `DocState::import_snapshot` -- **production code, ungated**
    /// (`FIXTURE-007`): the enum method dispatches here for the Toile variant,
    /// so putting this behind the `fixtures` feature would make an ordinary
    /// client build fail to compile and take Toile's local restore with it.
    ///
    /// Loro's `import` ingests snapshots and incremental updates alike, so this
    /// is [`apply_remote`](Self::apply_remote)'s body under the name a loader can
    /// address uniformly across apps -- which is the whole reason it exists:
    /// `TrameState::import_snapshot` was already spelled that way, `ToileState`
    /// spelled it `apply_remote`, and a loader cannot dispatch on two names.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Vec<ToileEvent> {
        self.apply_remote(payload)
    }

    // -- undo / redo --------------------------------------------------

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
        let deck_before = self.slide_order();
        self.rebuild_caches();
        let mut events = vec![self.viewport_event(), self.undo_state_event()];
        events.extend(self.deck_change(&deck_before));
        self.emit_with_update(events)
    }

    /// `DeckChanged` when the deck is no longer `before` (`toile-presentation`).
    /// Every write that can move the deck without naming it -- a history step, a
    /// remote delta, a deleted frame -- calls this, because the panel holds no
    /// authoritative state and would otherwise show a deck nobody has.
    pub(super) fn deck_change(&self, before: &[FrameId]) -> Option<ToileEvent> {
        (self.slide_order() != before).then_some(ToileEvent::DeckChanged)
    }

    pub(super) fn undo_state_event(&self) -> ToileEvent {
        ToileEvent::UndoState {
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
        }
    }

    // -- selection aggregates -----------------------------------

    fn selection_stats(&self, ids: &[ElementId]) -> ToileOutcome {
        let mut union: Option<Rect> = None;
        let mut count = 0u32;
        // Empty ids = whole-board extent: the union of every element's bounds.
        // Used by the View -> Fit action (`toile-viewport`), which needs
        // the full board rect the UI does not hold (it sees only the culled
        // viewport). A non-empty selection keeps the per-selection aggregate the
        // status bar consumes.
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
            requests: Vec::new(),
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
