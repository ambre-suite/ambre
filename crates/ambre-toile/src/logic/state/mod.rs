//! `ToileState` -- the per-document Toile engine (Logic thread side,).
//!
//! Owns the single-board `LoroDoc`, the derived `SpatialIndex` and the bounds /
//! layers caches. It turns typed [`ToileCommand`]s into a [`ToileOutcome`],
//! that is display events plus CRDT / ephemeral bytes. The `LoroDoc` never
//! leaves this thread; it exits only as exported bytes.
//!
//! **Phase 0 scope.** Element / connector / stroke lifecycle, z-order, groups
//! frame membership, local undo/redo, clipboard
//! selection stats, viewport-culled `DisplayViewport` derivation, and
//! the remote-apply / snapshot paths. Rich text (`EditText`, blocked on Prose's
//! shared `TextDelta`), find and nested-doc editing are subsequent slices; every
//! command handled here is complete.
//!
//! This module holds the state, the command dispatcher and the remote / undo /
//! persistence paths. The `impl` splits by responsibility across the sibling
//! modules below, and each of them reaches the struct's fields, consts and
//! shared helpers through `super::`:
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
/// The generators, and the per-app half of `shape_of` / `canonical_dump`
/// (`fixtures`). Public, because `ambre-logic`'s registry names the generators;
/// fixture code (I4), so the whole module is behind the non-default `fixtures`
/// feature and reaches no client build.
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod groups;
mod import;
mod presence;
pub use presence::PRESENCE_TTL_MS;
pub(crate) mod presentation;
mod reactions;
/// The schema shape belt (`SCHEMA-018`): the key set, the value types and the
/// `postcard` layouts a build writes, frozen per `SCHEMA_VERSION`. Test-only,
/// so no client build carries it.
#[cfg(test)]
mod schema_shape;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use kurbo::Rect;
use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use ambre_bus::UserId;
use ambre_core::schema::SchemaError;

use crate::logic::command::{
    PresencePayload, SelectionStats, StatsRequestKind, ToileCommand, ToileError, ToileEvent,
    ToileOutcome,
};
use crate::logic::index::{SpatialIndex, Viewport};
use crate::logic::model::{ConnectorId, ElementId, ElementStyle, FrameId, NodeId, PlaybackState};
use crate::logic::schema;

// Root container names.
pub(crate) const ELEMENTS: &str = "elements";
pub(crate) use codec::BODY;
pub(crate) const CONNECTORS: &str = "connectors";
const LAYERS: &str = "layers";
const BOARD: &str = "board";
/// Per-group / per-frame membership list key inside an element map.
const CHILDREN: &str = "children";
/// Collaborative speaker-notes LoroText key inside a frame element map.
const NOTES: &str = "notes";
/// Auto-layout spec LWW register key inside a frame element map.
const AUTOLAYOUT: &str = "autolayout";
/// Slide sequence root movable list (`toile-presentation`). A root container's
/// identity is the name itself. Every peer therefore addresses the same list,
/// and none mints a rival one by writing into it first (`TOILE-029`).
pub(crate) const PRESENTATION_ORDER: &str = "presentation_order";
/// Presentation config root map, a root container for the same reason.
pub(crate) const PRESENTATION_CONFIG: &str = "presentation_config";
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
    /// The containers (groups and frames) whose `children` list names each
    /// element, one entry per occurrence, in discovery order. Derived, never
    /// authoritative: a read skips an entry whose container the board no longer
    /// holds, so finding an element's container never walks the board
    /// (`TOILE-056`).
    parents: HashMap<ElementId, Vec<ElementId>>,
    /// How many times the whole element map has been resolved. A cost probe a
    /// test reads to show a read walks no board (`TOILE-056`); a `Cell`, since
    /// the reads that would walk take `&self`.
    board_walks: Cell<u64>,
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
    /// by the dispatcher. `None` until the session carries an authenticated
    /// user. A stable `UserId`, NOT `peer`, which is per-session / per-device
    /// and only mints element ids. It keys this peer's own persisted reactions
    /// (`toile-reactions`) and tags outbound presence with a stable identity.
    /// Same field as `trame-engine`.
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
    /// cursor). The map drops a peer whose last sample is older than the TTL,
    /// which is how a departure travels with no departure signal
    /// (`TOILE-185`).
    presence: HashMap<UserId, (PresencePayload, i64)>,
    /// The latest wall clock a local `Presence` command carried, and the only
    /// clock this thread has (`toile-presence`). Monotonic: it never moves
    /// backwards, whatever the machine's own clock does.
    now_ms: i64,
    /// Presentation (focus) mode toggle (`toile-presentation`). Local/per-viewer,
    /// never in Loro and never synced -- only the slide sequence travels.
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
    /// import that carries raster media waits here until every blob's real
    /// blake2b returns from Sync. The whole import therefore stays one atomic
    /// undo group. Rare and short-lived; dies with the `DocState`. See
    /// `state::import`.
    pending_imports: Vec<import::PendingImport>,
    /// A read met a register this build cannot decode (`SCHEMA-003`). The decode
    /// helpers set it, and every import drains it. A `Cell`, because a caller
    /// reads a board through `&self` and the read is where the failure surfaces.
    ///
    /// * A flag rather than a tally. Every later read meets the same unreadable
    ///   register, so a count would measure how often the board redrew.
    /// * An import drains it, a repaint never does. A register a display-only
    ///   read met first therefore reaches the user at the next import, not at
    ///   once. Late, and never wrong: the register is still unreadable.
    undecodable: std::cell::Cell<bool>,
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

    /// A board with a **pinned** CRDT identity and commit clock, so the same
    /// command sequence produces the same bytes twice (`FIXTURE-001`).
    ///
    /// This constructor closes two of the three entropy sources of a Toile
    /// document. The third, `document_id`, belongs to the caller, who hands it
    /// to [`export_ambre_plaintext`](Self::export_ambre_plaintext).
    ///
    /// * **`PeerID`** -- `LoroDoc::new()` draws one at random. Construction
    ///   caches [`peer`](Self::peer) and mints it into every [`ElementId`] /
    ///   [`ConnectorId`], so an unpinned peer changes the board's *content*, not
    ///   merely its encoding.
    /// * **Commit timestamps** -- Loro records one per commit unless told not to.
    ///
    /// Both land before the board-scaffolding commit of [`new`](Self::new), so
    /// `peer` owns that op like every op after it.
    ///
    /// **Never for a live document.** Two concurrent writers sharing a `PeerID`
    /// mint conflicting `OpID`s and corrupt the history. This exists for
    /// single-owner, offline, host-side reproduction (a fixture generator, a
    /// deterministic test). The client path is [`new`](Self::new).
    ///
    /// # Panics
    ///
    /// When a fresh `LoroDoc` refuses `peer`, which it does only after an op is
    /// recorded. No op precedes this call.
    pub fn with_peer(peer: u64) -> Self {
        let doc = LoroDoc::new();
        // `set_peer_id` before any op: this peer owns the board scaffolding, and
        // every minted element id captures it.
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
        // The presentation defaults sit in a root map, so they are registers on
        // a container every peer already addresses (`TOILE-029`).
        presentation::seed_presentation_config(&doc);
        // The schema version is board scaffolding too, and it rides the same
        // commit so no board ever persists without it (`SCHEMA-004`).
        schema::stamp(&doc);
        doc.commit();
        let peer = doc.peer_id();
        // Create the undo manager AFTER the initial commit. The board
        // scaffolding is document setup, not a user edit, so it must not sit on
        // the stack.
        let mut undo = UndoManager::new(&doc);
        // Keep every comment write off the undo stack (`TOILE-FEAT-017`): undo
        // reverts what the user drew, never a message a teammate has read.
        undo.add_exclude_origin_prefix(COMMENT_ORIGIN);
        Self {
            doc,
            index: SpatialIndex::default(),
            bounds: HashMap::new(),
            layers: Vec::new(),
            parents: HashMap::new(),
            board_walks: Cell::new(0),
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
            undecodable: std::cell::Cell::new(false),
        }
    }

    // -- session identity -------------------------------------------------

    /// Seed the canonical `UserId` from `LogicEvent::SessionReady`. Called
    /// by the dispatcher once the session carries an authenticated user;
    /// idempotent (a re-login with the same account is a no-op). Keys this
    /// peer's reactions (`toile-reactions`) and tags outbound presence.
    pub fn set_user_id(&mut self, user_id: UserId) {
        self.my_user_id = Some(user_id);
    }

    /// The canonical `UserId` for this session, when a user has signed in.
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

    /// Handle a UI command and return a typed outcome.
    ///
    /// Any command that produced a CRDT delta also moved the `UndoManager`
    /// stacks, so the chrome's Undo/Redo affordances must be refreshed
    /// (`UNDO-006`). Undo and Redo already append their own `UndoState`; skip
    /// the duplicate.
    /// A command that wrote also announces [`BoardChanged`](ToileEvent::BoardChanged),
    /// ahead of that refresh.
    pub fn handle_command(&mut self, cmd: ToileCommand) -> ToileOutcome {
        let mut outcome = self.dispatch_command(cmd);
        if outcome.update.is_none() {
            return outcome;
        }
        match outcome.display.pop() {
            Some(last @ ToileEvent::UndoState { .. }) => {
                outcome.display.push(ToileEvent::BoardChanged);
                outcome.display.push(last);
            }
            last => {
                outcome.display.extend(last);
                outcome.display.push(ToileEvent::BoardChanged);
                outcome.display.push(self.undo_state_event());
            }
        }
        outcome
    }

    /// Execute one command and return its raw outcome, before the central
    /// [`UndoState`](ToileEvent::UndoState) refresh applied by
    /// [`handle_command`](Self::handle_command).
    fn dispatch_command(&mut self, cmd: ToileCommand) -> ToileOutcome {
        match cmd {
            ToileCommand::CreateElement {
                kind,
                at,
                size,
                content,
                fill,
                correlate,
            } => self.create_element(kind, at, size, content, fill, correlate),
            ToileCommand::MoveElements { ids, delta } => self.move_elements(&ids, delta),
            ToileCommand::AlignElements { ids, op } => self.align_elements(&ids, op),
            ToileCommand::ResizeElement { id, new } => self.resize_element(&id, new),
            ToileCommand::RotateElement { id, radians } => self.rotate_element(&id, radians),
            ToileCommand::RestyleElements { ids, style } => self.restyle_elements(&ids, style),
            ToileCommand::DeleteElements { ids } => self.delete_elements(&ids),
            ToileCommand::SetText { id, base, text } => self.set_text(&id, &base, &text),
            ToileCommand::LockElements { ids, locked } => self.lock_elements(&ids, locked),
            ToileCommand::Reorder { node, to } => self.reorder(node, to),
            ToileCommand::GroupElements { ids } => self.group_elements(&ids),
            ToileCommand::UngroupElements { groups } => self.ungroup_elements(&groups),
            ToileCommand::SetFrameChildren { frame, add, remove } => {
                self.set_frame_children(&frame, &add, &remove)
            }
            ToileCommand::RemoveFromFrames { ids } => self.remove_from_frames(&ids),
            ToileCommand::RenameFrame { frame, name } => self.rename_frame(&frame, &name),
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
            // One answer per address the document resolves. An address it does
            // not carry earns no answer, so the element keeps the placeholder it
            // already paints (`toile-media`).
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
            ToileCommand::SetConnectorLabel { id, base, text } => {
                self.set_connector_label(id, &base, &text)
            }
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
            ToileCommand::RequestSelectionStats { ids, kind } => self.selection_stats(&ids, kind),
            ToileCommand::RequestSelectionData { ids } => self.selection_data(&ids),
            ToileCommand::RequestBoardIds => self.board_ids(),
            ToileCommand::ImportSvg { bytes, at } => self.import_svg_bytes(&bytes, at),
            ToileCommand::ImportImage { bytes, mime: _, at } => self.import_image_bytes(&bytes, at),
            ToileCommand::ExportSvg => self.export_svg_document(),
            // Served by the Logic thread, which holds the base font faces the
            // render walk needs (`toile-pdf-export`, `TOILE-IO-029`). Its
            // dispatcher intercepts the command before the document lookup. Only
            // a caller driving `ToileState` directly therefore lands here, and
            // this arm answers rather than falling silent.
            ToileCommand::ExportPdf => ToileOutcome::display_only(vec![ToileEvent::ExportFailed(
                "the deck is exported by the Logic thread, not by the board".to_owned(),
            )]),
            // Served by the Logic thread for a different reason: the container's
            // header carries the document's identity, which a board does not
            // know (`ambre-spec-docfile.md`). Same shape of answer, same reason
            // for answering -- a silent arm is indistinguishable from a gesture
            // that never arrived.
            ToileCommand::ExportAmbreFile | ToileCommand::ExportAmbreFileSealed { .. } => {
                ToileOutcome::display_only(vec![ToileEvent::ExportFailed(
                    "the file is written by the Logic thread, not by the board".to_owned(),
                )])
            }
            ToileCommand::ImportFigma { .. } | ToileCommand::ImportMiro { .. } => {
                // Figma / Miro converters land in a later interop slice, and
                // until then nothing here touches the board (atomicity).
                ToileOutcome::display_only(vec![ToileEvent::ImportFailed(
                    "Figma / Miro import is not yet available".to_owned(),
                )])
            }
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
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        if let Err(e) = self.doc.import(update) {
            tracing::warn!("Toile: remote import failed: {e}");
            return vec![ToileEvent::Error(ToileError::RemoteImportFailed)];
        }
        self.absorb_imported();
        self.rebuild_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        // A delta can carry a register this build cannot decode just as a
        // snapshot can, and the user deserves the same word (`SCHEMA-003`).
        events.extend(self.undecodable_notice());
        events
    }

    /// Apply a batch of remote deltas (a server history replay on open) as one
    /// `import` sweep plus a single cache rebuild. The UI therefore repaints
    /// once instead of once per delta. Mirrors
    /// [`apply_remote`](Self::apply_remote) but amortizes the derived-cache
    /// rebuild over the whole batch. A delta that fails to import is skipped,
    /// and the batch then reports the failure instead of answering success
    /// (`TOILE-215`).
    pub fn apply_remote_batch(&mut self, updates: &[Vec<u8>]) -> Vec<ToileEvent> {
        if updates.is_empty() {
            return Vec::new();
        }
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        let mut any_ok = false;
        let mut any_failed = false;
        for update in updates {
            match self.doc.import(update) {
                Ok(_) => any_ok = true,
                Err(e) => {
                    tracing::warn!("Toile: remote batch import failed: {e}");
                    any_failed = true;
                }
            }
        }
        if !any_ok {
            return vec![ToileEvent::Error(ToileError::RemoteImportFailed)];
        }
        self.absorb_imported();
        self.rebuild_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        if any_failed {
            events.push(ToileEvent::Error(ToileError::RemoteImportFailed));
        }
        events.extend(self.undecodable_notice());
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
    /// Toile half of `DocState::import_snapshot`. **Production code, ungated**
    /// (`FIXTURE-007`): the enum method dispatches here for the Toile variant.
    /// Putting this behind the `fixtures` feature would make an ordinary client
    /// build fail to compile and take Toile's local restore with it.
    ///
    /// Loro's `import` ingests snapshots and incremental updates alike, so this
    /// is [`apply_remote`](Self::apply_remote)'s body under a name a loader can
    /// address uniformly across apps. That is the whole reason it exists:
    /// `TrameState::import_snapshot` already spelled it that way, `ToileState`
    /// spelled it `apply_remote`, and a loader cannot dispatch on two names.
    ///
    /// * One thing it does **not** share with `apply_remote`: it leaves the
    ///   export cursor alone (`TOILE-236`). A restore reads what this client
    ///   wrote before it closed, and the operations it wrote offline reached no
    ///   peer. A reopen mints a fresh peer id, so the cursor no longer counts
    ///   those operations as ours. Absorbing them would drop a session's work
    ///   for good.
    /// * Leaving the cursor put re-offers the restored history on the first
    ///   flush. The relay already holds most of it and drops what it knows.
    ///
    /// # Errors
    /// Every failure of the schema gate.
    ///
    /// * A board a newer client wrote (`SCHEMA-008`).
    /// * A migration the registry cannot run.
    /// * Bytes the CRDT engine refuses.
    ///
    /// On any of them the board keeps what it held (`SCHEMA-012`). The gate
    /// reads the payload in a scratch document precisely so a refusal costs the
    /// caller nothing.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Result<Vec<ToileEvent>, SchemaError> {
        // Before the caller touches a single container (`SCHEMA-006`). What
        // comes back is the payload itself at the current version, or a migrated
        // snapshot.
        let payload = schema::gate(payload)?;
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        if let Err(e) = self.doc.import(&payload) {
            tracing::warn!("Toile: snapshot import failed: {e}");
            return Err(SchemaError::PayloadRejected);
        }
        self.rebuild_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        events.extend(self.undecodable_notice());
        Ok(events)
    }

    /// The schema version this board carries (`SCHEMA-004`), for a report rather
    /// than for a decision. The fixture manifest records the version of its own
    /// artefact.
    ///
    /// A register this build cannot read comes back as
    /// [`SCHEMA_VERSION_ABSENT`](ambre_core::schema::SCHEMA_VERSION_ABSENT)
    /// here. Every path that *decides* anything goes through `schema::gate`,
    /// which refuses it instead (`SCHEMA-013`). A caller reaches this one on a
    /// document it just generated, where the case cannot arise.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        schema::read_version(&self.doc).unwrap_or(ambre_core::schema::SCHEMA_VERSION_ABSENT)
    }

    /// Record that the decoder refused a register (`SCHEMA-003`).
    ///
    /// An **empty** register is not a failure: an element with no style carries
    /// no bytes, and that decodes to the default by design. Only a non-empty
    /// blob the decoder refuses is a record this build cannot read.
    pub(crate) fn note_undecodable(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.undecodable.set(true);
        }
    }

    /// Drain the flag into the one event that carries it to the user, or nothing
    /// when every register read cleanly (`SCHEMA-003`).
    ///
    /// Drained rather than read, so the notice reaches the user once per import
    /// instead of once per repaint.
    fn undecodable_notice(&mut self) -> Option<ToileEvent> {
        self.undecodable
            .replace(false)
            .then_some(ToileEvent::UndecodableRecords)
    }

    // -- undo / redo --------------------------------------------------

    /// Apply one undo or redo step. `undo()`/`redo()` commit the inverse ops,
    /// then this method reseeds the derived caches and broadcasts the delta like
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
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        self.rebuild_caches();
        // `undo_state_event` last, so the central refresh in `handle_command`
        // sees it and skips the duplicate (`UNDO-006`).
        let mut events = vec![self.viewport_event()];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        events.push(self.undo_state_event());
        self.emit_with_update(events)
    }

    /// `DeckChanged` when the deck is no longer `before` (`toile-presentation`).
    /// Every write that can move the deck without naming it calls this: a
    /// history step, a remote delta, a deleted frame. The panel holds no
    /// authoritative state, and it would otherwise show a deck nobody has.
    pub(super) fn deck_change(&self, before: &[FrameId]) -> Option<ToileEvent> {
        (self.slide_order() != before).then_some(ToileEvent::DeckChanged)
    }

    /// One `DanglingConnector` per connector that was whole before this write
    /// and is not any more (`TOILE-165`). Every write that can strand an arrow
    /// without naming it calls this: a remote delta, a history step, a peer's
    /// deletion. The re-aim path announces its own.
    pub(super) fn newly_dangling(&self, before: &HashSet<ConnectorId>) -> Vec<ToileEvent> {
        let mut fresh: Vec<ConnectorId> = self
            .dangling_connectors()
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        // The board is a set, so the order it walks is the engine's, not the
        // document's. Sorted, two peers announce the same arrows in one order.
        fresh.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        fresh
            .into_iter()
            .map(ToileEvent::DanglingConnector)
            .collect()
    }

    /// The speaker notes of the slide on screen, read before a write this
    /// viewer did not make (`TOILE-UI-316`). `None` when no slide resolves.
    pub(super) fn active_notes(&self) -> Option<(FrameId, String)> {
        let frame = self.slide_order().get(self.playback.active).cloned()?;
        let notes = self.notes(&frame);
        Some((frame, notes))
    }

    /// `NotesChanged` when the note on screen is no longer `before`
    /// (`TOILE-UI-316`). Every write that can move it without naming it calls
    /// this: a remote delta, a history step. The editor is otherwise seeded by
    /// the slide event alone, so a co-author's writing stays invisible until
    /// the slide changes.
    pub(super) fn notes_change(&self, before: &Option<(FrameId, String)>) -> Option<ToileEvent> {
        let (frame, notes) = self.active_notes()?;
        match before {
            Some((f, n)) if *f == frame && *n == notes => None,
            _ => Some(ToileEvent::NotesChanged { frame, notes }),
        }
    }

    pub(super) fn undo_state_event(&self) -> ToileEvent {
        ToileEvent::UndoState {
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
        }
    }

    // -- selection aggregates -----------------------------------

    fn selection_stats(&self, ids: &[ElementId], kind: StatsRequestKind) -> ToileOutcome {
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
                kind,
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
            kind,
        })])
    }

    /// The full display data of the requested ids, off-screen included
    /// (`toile-menus`, `toile-styles`). The UI thread asks here because the
    /// culled viewport omits the selection past the edge (`TOILE-UI-070`,
    /// `TOILE-UI-134`). It also names the ids a frame holds, which the menu
    /// cannot read off a box (`TOILE-UI-384`).
    fn selection_data(&self, ids: &[ElementId]) -> ToileOutcome {
        let elements = ids
            .iter()
            .filter_map(|id| self.display_element(id))
            .collect();
        let framed = self.framed_among(ids);
        ToileOutcome::display_only(vec![ToileEvent::SelectionData { elements, framed }])
    }

    /// Every element id on the board, off-screen included, for select-all
    /// (`toile-selection`, `TOILE-UI-033`). A pick excludes a locked element
    /// and a group node (`TOILE-UI-006`, `TOILE-UI-224`); select-all obeys the
    /// same exclusions (`TOILE-UI-241`).
    fn board_ids(&self) -> ToileOutcome {
        // The element map, never the `layers` order: an element the order omits
        // is on the board, and the paint walk already draws it (`TOILE-058`).
        // Reading the order here would leave select-all short of exactly the
        // elements that fallback exists for.
        let elems = self.doc.get_map(ELEMENTS);
        let LoroValue::Map(entries) = elems.get_value() else {
            return ToileOutcome::display_only(vec![ToileEvent::BoardIds(Vec::new())]);
        };
        let mut ids: Vec<ElementId> = entries
            .keys()
            .filter_map(|key| {
                let id = ElementId(key.to_string());
                let em = self.element_map(&id)?;
                if codec::bool_of(&em, "locked") || codec::str_of(&em, "type") == "group" {
                    None
                } else {
                    Some(id)
                }
            })
            .collect();
        // By id, so every peer answers the same list (`TOILE-121`). The map's
        // own iteration order carries no such promise.
        ids.sort();
        ToileOutcome::display_only(vec![ToileEvent::BoardIds(ids)])
    }

    // -- outcome plumbing ------------------------------------------------------

    /// Wrap the current export delta (since `exported_vv`) into the outcome.
    /// Every mutating command routes through here.
    pub(super) fn emit_with_update(&mut self, display: Vec<ToileEvent>) -> ToileOutcome {
        ToileOutcome {
            display,
            update: self.flush_delta(),
            ephemeral: None,
            requests: Vec::new(),
        }
    }

    /// Export every operation since the export cursor, and advance the cursor
    /// only if that export succeeded (`TOILE-090`).
    ///
    /// An export that fails leaves the cursor where it is, so the next flush
    /// offers the same range again. Advancing regardless, which is what
    /// swallowing the error with `.ok()` did, dropped those operations for good.
    /// No later export would ever carry them, and the two peers stayed apart
    /// until someone sent a full snapshot. This is the shape Trame's
    /// `flush_delta` already had (`features/ambre-spec-trame-engine.md`).
    fn flush_delta(&mut self) -> Option<Vec<u8>> {
        let current = self.doc.oplog_vv();
        if current == self.exported_vv {
            return None;
        }
        let exported = self.doc.export(ExportMode::updates(&self.exported_vv));
        Self::take_delta(&mut self.exported_vv, current, exported)
    }

    /// Take the export cursor past `current` -- and every operation the board
    /// holds up to it -- **only** on a successful export (`TOILE-090`).
    ///
    /// Split out of [`flush_delta`](Self::flush_delta) so a test can drive the
    /// rule against a failure. Loro's exporter has no seam a test can break. A
    /// rule about error handling is worth nothing when no test can reach the
    /// error. An empty successful export still advances the cursor:
    /// the range held nothing to broadcast, which is not a failure.
    fn take_delta<E: std::fmt::Debug>(
        cursor: &mut VersionVector,
        current: VersionVector,
        exported: Result<Vec<u8>, E>,
    ) -> Option<Vec<u8>> {
        match exported {
            Ok(delta) => {
                *cursor = current;
                (!delta.is_empty()).then_some(delta)
            }
            Err(e) => {
                tracing::error!(
                    "Toile: export failed, the range is kept for the next flush: {e:?}"
                );
                None
            }
        }
    }

    /// Take the export cursor past everything the board now holds, keeping this
    /// peer's own counter where it was.
    ///
    /// Called after an import (a peer's delta, a history replay, a restore). The
    /// imported operations are not ours to broadcast. The cursor must clear
    /// them, or the next local edit would send a peer its own work back.
    ///
    /// * Our own counter is the exception, and Trame learned it the hard way
    ///   (`features/ambre-spec-trame-engine.md`). Every board opens by
    ///   committing its scaffolding under a fresh random peer id, and the first
    ///   import lands before any flush.
    /// * Swallowing those genesis operations here would make the next local
    ///   delta depend, causally, on operations no other peer holds. A peer that
    ///   cannot resolve a dependency keeps the edit pending forever rather than
    ///   showing it.
    fn absorb_imported(&mut self) {
        let ours = self.exported_vv.get(&self.peer).copied();
        self.exported_vv = self.doc.oplog_vv();
        match ours {
            Some(counter) => {
                self.exported_vv.insert(self.peer, counter);
            }
            None => {
                self.exported_vv.remove(&self.peer);
            }
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

    /// How many times the whole element map has been resolved. Test helper.
    pub fn board_walks(&self) -> u64 {
        self.board_walks.get()
    }

    /// Materialized paint order. Test helper.
    pub fn layer_order(&self) -> &[NodeId] {
        &self.layers
    }

    /// Derived world AABB of an element. Test helper.
    pub fn bounds_of(&self, id: &ElementId) -> Option<Rect> {
        self.bounds.get(id).copied()
    }

    /// The decoded style register of an element, or the default when absent.
    /// Test helper.
    pub fn style_of(&self, id: &ElementId) -> ElementStyle {
        self.element_map(id)
            .map(|em| self.style_read(&codec::bin_of(&em, "style")))
            .unwrap_or_default()
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

    /// The raw strings of the `layers` list, decode-or-not. Test helper: the
    /// only way to assert an entry this build cannot decode survives a delete
    /// (`TOILE-129`).
    pub fn debug_raw_layers(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let LoroValue::List(cur) = self.doc.get_movable_list(LAYERS).get_deep_value() {
            for v in cur.iter() {
                if let LoroValue::String(s) = v {
                    out.push(s.as_str().to_owned());
                }
            }
        }
        out
    }

    /// Push one raw string onto the `layers` list, decode-or-not. Test helper:
    /// it forges an entry a future version wrote and this build cannot read
    /// (`TOILE-129`).
    pub fn debug_push_layer_raw(&mut self, s: &str) {
        let _ = self.doc.get_movable_list(LAYERS).push(s);
        self.rematerialize_layers();
    }

    /// Remove one node's entry from the `layers` list, leaving the node in the
    /// document. Test helper: it forges the concurrent drop `TOILE-058` names, a
    /// state no command of ours can produce.
    pub fn debug_remove_layer_entry(&mut self, node: &NodeId) {
        let list = self.doc.get_movable_list(LAYERS);
        if let LoroValue::List(cur) = list.get_deep_value() {
            for (i, v) in cur.iter().enumerate() {
                if let LoroValue::String(s) = v
                    && NodeId::decode(s).as_ref().is_some_and(|n| n == node)
                {
                    let _ = list.delete(i, 1);
                    break;
                }
            }
        }
        self.rematerialize_layers();
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

#[cfg(test)]
mod schema_tests {
    //! The two schema rules that need a live board rather than a payload.
    //!
    //! * A refused read must leave the board it refused exactly as it was
    //!   (`SCHEMA-012`).
    //! * A register this build cannot decode must reach the user
    //!   (`SCHEMA-003`).

    use kurbo::{Point, Size};
    use loro::{ExportMode, LoroDoc};

    use ambre_bus::app::toile::ToileEvent;
    use ambre_core::schema::{SCHEMA_META_MAP, SCHEMA_VERSION_KEY, SchemaError};

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::schema::SCHEMA_VERSION;
    use crate::logic::state::ToileState;

    fn board_with_one_rect() -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        });
        st
    }

    /// `SCHEMA-008` + `SCHEMA-012`: the refusal is the whole document, and the
    /// board the user was already looking at survives it intact.
    #[test]
    fn a_board_from_a_newer_client_is_refused_and_changes_nothing() {
        // A snapshot stamped one version ahead of this build.
        let newer = {
            let doc = LoroDoc::new();
            doc.get_map(SCHEMA_META_MAP)
                .insert(SCHEMA_VERSION_KEY, i64::from(SCHEMA_VERSION + 1))
                .expect("stamp");
            doc.get_map("elements")
                .insert_container("xx", loro::LoroMap::new())
                .expect("an element the refusal must not import");
            doc.commit();
            doc.export(ExportMode::Snapshot).expect("snapshot")
        };

        let mut st = board_with_one_rect();
        let before = st.export_snapshot();
        let err = st.import_snapshot(&newer).expect_err("refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION + 1,
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(st.element_count(), 1, "nothing was merged in");
        assert_eq!(
            st.export_snapshot(),
            before,
            "a refused read must not write the document back"
        );
    }

    /// `SCHEMA-003`: the painter unavoidably draws a style register this build
    /// cannot decode plain. The user hears about it, instead of reading a
    /// document that looks intact and is not.
    #[test]
    fn a_register_that_cannot_be_decoded_reaches_the_user() {
        let source = board_with_one_rect();
        let crate::logic::model::NodeId::Element(id) = source.layer_order()[0].clone() else {
            panic!("the board holds one element");
        };
        let em = source.element_map(&id).expect("the rect is there");
        // A blob no `ElementStyle` layout can produce: a register written by a
        // client this build does not know.
        em.insert("style", loro::LoroValue::Binary(vec![0xFFu8; 8].into()))
            .expect("overwrite the style register");
        source.doc.commit();
        let payload = source.export_snapshot();

        let mut st = ToileState::new();
        let events = st.import_snapshot(&payload).expect("the board still opens");

        assert!(
            events
                .iter()
                .any(|e| matches!(e, ToileEvent::UndecodableRecords)),
            "an unreadable register must be reported, not defaulted in silence"
        );
        // Once per import, not once per repaint: the read drains the flag.
        assert!(st.undecodable_notice().is_none());
    }

    /// The same board, written by this build, says nothing -- otherwise the
    /// notice is noise and the user learns to ignore it.
    #[test]
    fn a_board_this_build_wrote_raises_no_notice() {
        let payload = board_with_one_rect().export_snapshot();
        let mut st = ToileState::new();
        let events = st.import_snapshot(&payload).expect("opens");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ToileEvent::UndecodableRecords))
        );
    }
}

#[cfg(test)]
mod cursor_tests {
    use loro::VersionVector;

    use super::ToileState;

    /// `TOILE-090`: a failed export keeps the cursor where it is, so the next
    /// flush offers the range again. A successful one takes the cursor past
    /// everything it exported.
    #[test]
    fn a_failed_export_keeps_the_range_for_the_next_flush() {
        // A cursor the board has actually reached, so the two vectors differ
        // the way they do in a real flush.
        let st = ToileState::new();
        let current = st.doc.oplog_vv();
        let mut cursor = VersionVector::default();

        let out = ToileState::take_delta(
            &mut cursor,
            current.clone(),
            Err::<Vec<u8>, &str>("the exporter said no"),
        );
        assert!(out.is_none(), "a failed export broadcasts nothing");
        assert_eq!(
            cursor,
            VersionVector::default(),
            "a failed export must not advance the cursor"
        );

        // The next flush offers the same range, and this time it lands.
        let out =
            ToileState::take_delta(&mut cursor, current.clone(), Ok::<_, &str>(vec![1, 2, 3]));
        assert_eq!(out.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(cursor, current, "a successful export advances the cursor");
    }

    /// An export that succeeds with nothing in it still advances the cursor.
    /// An empty range is not a failure, and it holds nothing to offer again.
    #[test]
    fn an_empty_export_is_not_a_failure() {
        let st = ToileState::new();
        let current = st.doc.oplog_vv();
        let mut cursor = VersionVector::default();

        let out = ToileState::take_delta(&mut cursor, current.clone(), Ok::<_, &str>(Vec::new()));
        assert!(out.is_none(), "an empty delta is not broadcast");
        assert_eq!(cursor, current);
    }

    /// A flush with nothing new answers `None` and touches nothing: the guard
    /// that keeps an idle command off the exporter.
    #[test]
    fn a_flush_with_nothing_new_exports_nothing() {
        let mut st = ToileState::new();
        assert!(
            st.flush_delta().is_some(),
            "the scaffolding is ours to send"
        );
        assert!(st.flush_delta().is_none(), "and it is sent only once");
    }
}

#[cfg(test)]
mod undo_state_tests {
    //! `UNDO-006`: every mutating command refreshes the Undo/Redo availability,
    //! and a read-only command does not.

    use kurbo::{Point, Size};

    use ambre_bus::app::toile::ToileEvent;

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::state::ToileState;

    /// Every mutating command refreshes the Undo/Redo availability, so the
    /// chrome enables the Undo button right after the first edit. It does not
    /// wait for the user to trigger an undo. A read-only command emits no
    /// `UndoState`.
    #[test]
    fn mutating_command_reports_undo_state() {
        let mut st = ToileState::new();

        let evts = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: 0,
        });
        assert!(
            matches!(
                evts.display.last(),
                Some(ToileEvent::UndoState {
                    can_undo: true,
                    can_redo: false
                }),
            ),
            "an edit enables Undo: {:?}",
            evts.display.last(),
        );

        // A read-only command produces no CRDT delta -> no UndoState refresh.
        let evts = st.handle_command(ToileCommand::ReadDeck);
        assert!(
            !evts
                .display
                .iter()
                .any(|e| matches!(e, ToileEvent::UndoState { .. })),
            "a read-only command does not touch the history stacks",
        );
    }
}
