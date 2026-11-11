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
mod names;
mod pptx_export;
mod presence;
pub use import::{MediaEntries, PendingImports};
pub use presence::PRESENCE_TTL;
pub(crate) mod presentation;
mod reactions;
/// The schema shape belt (`SCHEMA-018`): the key set, the value types and the
/// `postcard` layouts a build writes, frozen per `SCHEMA_VERSION`. Test-only,
/// so no client build carries it.
#[cfg(test)]
mod schema_shape;

/// The schema reading belt (`SCHEMA-017`): what each stored value means, in
/// absolute units, frozen per `SCHEMA_VERSION`. Test-only, like the shape belt.
#[cfg(test)]
mod schema_reading;

use ambre_core::Count;
use ambre_core::hash::{HashMap, HashSet};
use std::cell::Cell;

use kurbo::Rect;
use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

use crate::message::{Elements, IdCounter};
use ambre_core::id::{Connector, Element, Frame, Id, Thread, User};
use ambre_core::schema::SchemaError;
use ambre_core::{HistoryStep, Rgba};

use crate::logic::command::{
    Broadcast, PresencePayload, PresentMode, SelectionStats, StatsRequestKind, ToileCommand,
    ToileError, ToileEvent, ToileOutcome,
};
use crate::logic::index::{SpatialIndex, Viewport};
use crate::logic::model::{ElementKind, ElementStyle, NodeId, PlaybackState};
use crate::logic::schema;
use crate::logic::text_merge::Edit;
use crate::message::{ToileExport, ToileImport, ToileImportFormat};
use ambre_bus::interop::{Direction, InteropAnswer, InteropError, InteropRequest};

use ambre_core::{Instant, PageLoad};
pub(crate) use names::{BoardKey, ElementKey, Root};
#[cfg(test)]
pub(crate) use names::{
    ConfigKey, ConnectorKey, Fields, MediaKey, MessageKey, ThreadKey, board, entry, root_map,
};

/// How many times the whole element map has been resolved: a cost probe
/// (`TOILE-056`).
pub enum BoardWalks {}

impl ambre_core::CountKind for BoardWalks {
    type Repr = u32;
    const NAME: &'static str = "BoardWalkCount";
}

/// Per-document Toile engine.
pub struct ToileState {
    doc: LoroDoc,
    index: SpatialIndex,
    /// Rotation-expanded world AABB per element; feeds culling + connector
    /// routing. Derived, never authoritative.
    bounds: HashMap<Id<Element>, Rect>,
    /// Materialized paint order, mirrored from the `layers` list.
    layers: Vec<NodeId>,
    /// The groups and frames whose `children` list names each element, one
    /// entry per occurrence, in discovery order. Derived, never authoritative:
    /// a read skips an entry whose parent the board no longer holds, so
    /// finding an element's parent never walks the board (`TOILE-056`).
    parents: HashMap<Id<Element>, Vec<Id<Element>>>,
    /// The reverse of `parents`: what each parent's `children` list names.
    /// A remote change to a list relinks its parent from it (`TOILE-092`).
    members: HashMap<Id<Element>, Vec<Id<Element>>>,
    /// What the document was written since the caches last read it, noted by
    /// the subscription below.
    touched: caches::TouchedSink,
    /// Held so the subscription lives as long as the state.
    _watch: loro::Subscription,
    /// Every arrow's ends and reach, for the cull (`TOILE-045`). Brought in
    /// step on read from what the subscription noted, hence the `RefCell`:
    /// the derivation that reads it takes `&self`.
    wires: std::cell::RefCell<HashMap<Id<Connector>, connectors::Wire>>,
    /// The arrows the cache dropped since the last answer named them.
    wires_gone: std::cell::RefCell<Vec<Id<Connector>>>,
    /// How many times the whole element map has been resolved. A cost probe a
    /// test reads to show a read walks no board (`TOILE-056`); a `Cell`, since
    /// the reads that would walk take `&self`.
    board_walks: Cell<Count<BoardWalks>>,
    /// Per-session monotonic id-minting counter.
    counter: IdCounter,
    peer: ambre_core::PeerId,
    /// Last viewport the UI reported; drives culling.
    viewport: Viewport,
    /// How many viewport requests this state applied. Stamped on every frame
    /// it derives, so the view can drop a stale one (`BUS-017`).
    viewport_version: ambre_core::Seq<ambre_bus::ViewportVersion>,
    /// Version vector already exported to Sync; deltas are `export(Updates{from})`.
    exported_vv: VersionVector,
    /// Local-only undo/redo: undoes only this peer's operations, one
    /// item per atomic user action (one `doc.commit()` per mutating command).
    /// Each commit records its own inverse (`ambre-spec-undo.md section 3`).
    history: ambre_crdt::history::UndoHistory,
    /// Canonical per-user identifier, seeded from `LogicEvent::SessionReady`
    /// by the dispatcher. `None` until the session carries an authenticated
    /// user. A stable `Id<User>`, NOT `peer`, which is per-session / per-device
    /// and only mints element ids. It keys this peer's own persisted reactions
    /// (`toile-reactions`) and tags outbound presence with a stable identity.
    /// Same field as `trame-engine`.
    my_user_id: Option<Id<User>>,
    /// This session's display name, seeded from `SessionReady` beside the
    /// `Id<User>` and stamped on every outbound presence sample
    /// (`TOILE-UI-076`). Cosmetic: it decides neither identity nor colour
    /// (`PRESENCE-004`).
    my_name: Option<ambre_core::Validated<ambre_core::DisplayName>>,
    /// Remote peers' latest presence (cursors, live strokes, lasers, selections)
    /// and the local clock each was last seen at, keyed by canonical `Id<User>`.
    /// Ephemeral: never in Loro, never persisted, rebuilt from inbound samples.
    /// The local peer's own presence is not stored here (the UI draws its own
    /// cursor). The map drops a peer whose last sample is older than the TTL,
    /// which is how a departure travels with no departure signal
    /// (`TOILE-185`).
    presence: HashMap<Id<User>, (PresencePayload, Instant<PageLoad>)>,
    /// The latest wall clock a local `Presence` command carried, and the only
    /// clock this thread has (`toile-presence`). Monotonic: it never moves
    /// backwards, whatever the machine's own clock does.
    now: Instant<PageLoad>,
    /// Presentation (focus) mode toggle (`toile-presentation`). Local/per-viewer,
    /// never in Loro and never synced -- only the slide sequence travels.
    present: PresentMode,
    /// Local playback cursor + focus flag. Momentary-gesture state, like
    /// [`present`](Self::present); reset on `EnterPresent`, clamped by
    /// `SetSlideOrder`.
    playback: PlaybackState,
    /// "Follow presenter" broadcast toggle: when set, the playback commands
    /// additionally emit a presence sample carrying the active slide index so
    /// followers glide along. Local role, never in Loro and never synced.
    broadcasting: Broadcast,
    /// The presenter this viewer is following, or `None`. When `Some(u)`,
    /// an inbound presence sample from `u` that carries a `present_slide` moves this
    /// viewer's active slide; a manual navigation clears it ("take control").
    /// Local role, never in Loro and never synced.
    following: Option<Id<User>>,
    /// Imports awaiting media-upload confirmations (`toile-media`). An
    /// import that carries raster media waits here until every blob's real
    /// blake2b returns from Sync. The whole import therefore stays one atomic
    /// undo group. Rare and short-lived; dies with the `DocState`. See
    /// `state::import`.
    pending_imports: Vec<import::PendingImport>,
    /// The import being applied in slices (`TOILE-IO-028`). The Logic thread
    /// resumes it one slice per pass; a command that writes the board lands it
    /// whole first. `None` while no import runs.
    import_job: Option<Box<import::ImportJob>>,
    /// A read met a register this build cannot decode (`SCHEMA-003`). The decode
    /// helpers set it, and every import drains it. A `Cell`, because a caller
    /// reads a board through `&self` and the read is where the failure surfaces.
    ///
    /// * A flag rather than a count. Every later read meets the same unreadable
    ///   register, so a count would measure how often the board redrew.
    /// * An import drains it, a repaint never does. A register a display-only
    ///   read met first therefore reaches the user at the next import, not at
    ///   once. Late, and never wrong: the register is still unreadable.
    undecodable: std::cell::Cell<bool>,
    /// This PeerID's counter just past its registry entry, while the entry is
    /// all the session wrote. No delta goes out for it alone; the first edit
    /// carries it (`AUTHOR-009`). Set by [`adopt_session`](Self::adopt_session).
    entry_end: Option<ambre_core::OpCounter>,
    /// The drawing the images module read for the import in flight, set by
    /// `handle_conversion` for the length of one dispatch.
    converted: Option<import::ConvertedFile>,
}

/// Write a merge's edits into the text it was computed against, in order. The
/// merge already knows which characters change, so the text is spared the
/// whole-body diff `LoroText::update` would run (`TOILE-UI-383`).
///
/// # Errors
///
/// The first edit the text refuses. Every position was computed against the
/// text as read, so a refusal means it changed between the read and the write.
fn write_edits(text: &loro::LoroText, edits: &[Edit]) -> loro::LoroResult<()> {
    for e in edits {
        if !e.delete.is_zero() {
            text.delete(e.at.to_usize(), e.delete.to_usize())?;
        }
        if !e.insert.is_empty() {
            text.insert(e.at.to_usize(), &e.insert)?;
        }
    }
    Ok(())
}

/// The board background the scaffolding stores: a dark grey, opaque.
const BOARD_BG: Rgba = Rgba::from_bits(0x1E1E_1EFF);

/// The board scaffolding, which every peer writes on open, before any edit.
fn write_scaffold(doc: &LoroDoc) {
    let board = names::board(doc);
    let _ = board.insert(BoardKey::BgColor, i64::from(BOARD_BG.bits()));
    let _ = board.insert(BoardKey::GridSize, 32.0);
    let _ = board.insert(BoardKey::GridSnap, false);
    // The presentation defaults sit in a root map, so they are registers on
    // a container every peer already addresses (`TOILE-029`).
    presentation::seed_presentation_config(doc);
    // The schema version is board scaffolding too, and it rides the same
    // commit so no board ever persists without it (`SCHEMA-004`).
    schema::stamp(doc);
}

/// The identity the scaffolding is written under: a hash of the scaffolding
/// this build writes (`TOILE-265`). Two builds that scaffold alike share it,
/// and two that differ never write two changes under one identifier.
#[expect(
    clippy::expect_used,
    reason = "a fresh document with no op takes a peer id and exports"
)]
fn scaffold_identity() -> ambre_core::PeerId {
    static PEER: std::sync::OnceLock<ambre_core::PeerId> = std::sync::OnceLock::new();
    *PEER.get_or_init(|| {
        let scratch = LoroDoc::new();
        scratch
            .set_peer_id(0)
            .expect("a fresh document takes a peer id");
        write_scaffold(&scratch);
        scratch.commit();
        let bytes = scratch
            .export(ExportMode::all_updates())
            .expect("a fresh document exports");
        ambre_core::schema::scaffold_peer(&bytes)
    })
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
    ///   caches [`peer`](Self::peer) and mints it into every [`Id<Element>`] /
    ///   [`Id<Connector>`], so an unpinned peer changes the board's *content*, not
    ///   merely its encoding.
    /// * **Commit timestamps** -- Loro records one per commit unless told not to.
    ///
    /// Both land before the board-scaffolding commit of [`new`](Self::new).
    /// That commit alone runs under the shared scaffolding identity, and
    /// `peer` owns every op after it (`TOILE-265`).
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
    #[expect(
        clippy::expect_used,
        reason = "a fresh document with no op takes a peer id"
    )]
    pub fn with_peer(peer: ambre_core::PeerId) -> Self {
        let doc = LoroDoc::new();
        // `set_peer_id` before any op: every minted element id captures it.
        doc.set_peer_id(peer.get())
            .expect("a fresh LoroDoc accepts a peer id");
        // No wall clock in the ops: a recorded timestamp would differ between two
        // runs of the same generator and defeat the byte diff of `fixtures`.
        doc.set_record_timestamp(false);
        Self::from_doc(doc)
    }

    /// Shared body of [`new`](Self::new) and [`with_peer`](Self::with_peer): the
    /// board scaffolding and every derived cache, over a `LoroDoc` the caller has
    /// already configured.
    #[expect(
        clippy::expect_used,
        reason = "a document outside a transaction takes a peer id"
    )]
    fn from_doc(doc: LoroDoc) -> Self {
        let peer = doc.peer_id();
        // Every peer writes the scaffolding as one shared identity, so every
        // copy is one change. Under the board's own identity, a reader that has
        // sent nothing would import each change against the whole history
        // (`TOILE-265`).
        doc.set_peer_id(scaffold_identity().get())
            .expect("a document outside a transaction takes a peer id");
        write_scaffold(&doc);
        doc.commit();
        doc.set_peer_id(peer)
            .expect("a document outside a transaction takes a peer id");
        // Open the history AFTER the initial commit. The board scaffolding is
        // document setup, not a user edit, so it must not sit on the stack.
        // Every comment write stays off the stack (`TOILE-FEAT-017`): undo
        // reverts what the user drew, never a message a teammate has read. The
        // registry entry stays off it too (`AUTHOR-009`), as for every
        // application.
        let history = ambre_crdt::history::UndoHistory::watch_nested(
            &doc,
            &[ambre_core::crdt::CommitOrigin::Comment],
            None,
        );
        let (touched, watch) = caches::watch(&doc);
        Self {
            doc,
            index: SpatialIndex::default(),
            bounds: HashMap::default(),
            layers: Vec::new(),
            parents: HashMap::default(),
            members: HashMap::default(),
            touched,
            _watch: watch,
            wires: std::cell::RefCell::default(),
            wires_gone: std::cell::RefCell::default(),
            board_walks: Cell::new(Count::<BoardWalks>::ZERO),
            counter: IdCounter::ZERO,
            peer: ambre_core::PeerId::new(peer),
            viewport: Viewport::default(),
            viewport_version: ambre_core::Seq::ZERO,
            exported_vv: VersionVector::default(),
            history,
            my_user_id: None,
            my_name: None,
            presence: HashMap::default(),
            now: Instant::<PageLoad>::new(0.0),
            present: PresentMode::Editing,
            playback: PlaybackState::default(),
            broadcasting: Broadcast::Private,
            following: None,
            pending_imports: Vec::new(),
            import_job: None,
            undecodable: std::cell::Cell::new(false),
            entry_end: None,
            converted: None,
        }
    }

    // -- session identity -------------------------------------------------

    /// Write under the session identity Sync issued for this board
    /// (`AUTHOR-002`), and write its registry entry first (`AUTHOR-009`).
    ///
    /// The export cursor moves past everything the board held before. Those
    /// operations are not this session's to send (`AUTHOR-008`). The entry is
    /// written now, before any edit opens an undo group it would split. Every
    /// id minted from here on carries the new PeerID. `false` when the CRDT
    /// engine refused the identity, and then nothing changed.
    pub fn adopt_session(
        &mut self,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
    ) -> bool {
        self.doc.commit();
        if let Err(e) = self.doc.set_peer_id(peer.get()) {
            tracing::error!("Toile: the session identity was refused: {e:?}");
            return false;
        }
        self.peer = peer;
        self.exported_vv = self.doc.oplog_vv();
        self.entry_end = match self
            .doc
            .get_map(ambre_core::crdt::FoundationRoot::Peers.as_str())
            .insert(ambre_core::PeerKey::of(peer).as_str(), entry.to_bytes())
        {
            Ok(()) => {
                self.doc.commit_with(
                    loro::CommitOptions::new()
                        .origin(ambre_core::crdt::CommitOrigin::Peers.as_str()),
                );
                self.doc
                    .oplog_vv()
                    .get(&peer.get())
                    .copied()
                    .map(ambre_core::OpCounter::new)
            }
            Err(e) => {
                tracing::error!("Toile: the registry entry was refused: {e:?}");
                None
            }
        };
        true
    }

    /// Seed the canonical `Id<User>` from `LogicEvent::SessionReady`. Called
    /// by the dispatcher once the session carries an authenticated user;
    /// idempotent (a re-login with the same account is a no-op). Keys this
    /// peer's reactions (`toile-reactions`) and tags outbound presence.
    pub fn set_user_id(&mut self, user_id: Id<User>) {
        self.my_user_id = Some(user_id);
    }

    /// The canonical `Id<User>` for this session, when a user has signed in.
    pub fn user_id(&self) -> Option<Id<User>> {
        self.my_user_id
    }

    /// Seed this session's display name, from the same `SessionReady` that
    /// carries the `Id<User>`. It rides every outbound presence sample so a
    /// collaborator reads a name rather than a hex prefix (`TOILE-UI-076`), and
    /// it decides nothing (`PRESENCE-004`).
    pub fn set_display_name(&mut self, name: ambre_core::Validated<ambre_core::DisplayName>) {
        self.my_name = Some(name);
    }

    /// Whether this viewer shares its playback position. Test / integration
    /// accessor.
    pub fn broadcast(&self) -> Broadcast {
        self.broadcasting
    }

    /// The presenter this viewer is currently following, if any. Test /
    /// integration accessor.
    pub fn following(&self) -> Option<Id<User>> {
        self.following
    }

    // -- id minting -----------------------------------------------------

    /// The next id part of this session. A session past `u32::MAX` mints
    /// nothing: a repeated id would overwrite the element that holds it.
    fn next_counter(&mut self) -> Result<IdCounter, ToileError> {
        let next = self.counter.next().ok_or(ToileError::IdsExhausted)?;
        self.counter = next;
        Ok(next)
    }

    pub(super) fn mint_element_id(&mut self) -> Result<Id<Element>, ToileError> {
        Ok(Id::<Element>::mint_from(self.peer, self.next_counter()?))
    }

    pub(super) fn mint_connector_id(&mut self) -> Result<Id<Connector>, ToileError> {
        Ok(Id::<Connector>::mint_from(self.peer, self.next_counter()?))
    }

    pub(super) fn mint_thread_id(&mut self) -> Result<Id<Thread>, ToileError> {
        Ok(Id::<Thread>::mint_from(self.peer, self.next_counter()?))
    }

    // -- command dispatch ------------------------------------------------------

    /// Handle a UI command and return a typed outcome.
    ///
    /// Any command that produced a CRDT delta also moved the undo history, so the chrome's Undo/Redo affordances must be refreshed
    /// (`UNDO-006`). Undo and Redo already append their own `UndoState`; skip
    /// the duplicate.
    /// A command that wrote also announces [`BoardChanged`](ToileEvent::BoardChanged),
    /// ahead of that refresh.
    ///
    /// A command that writes or reads the board waits for a running import to
    /// land whole first (`TOILE-IO-028`). Its delta then carries the rest of
    /// the import, and the import stays one undo step. A camera move does not
    /// wait.
    pub fn handle_command(&mut self, cmd: ToileCommand) -> ToileOutcome {
        let landed = if waits_for_import(&cmd) {
            self.finish_running_import()
        } else {
            Vec::new()
        };
        let mut outcome = self.dispatch_command(cmd);
        // A local command keeps the caches in step itself, so what its commits
        // noted is dropped here. The set would otherwise grow with every edit
        // until the next refresh, and that refresh would pay for it.
        self.forget_touched();
        if !landed.is_empty() {
            outcome.display.splice(0..0, landed);
            if outcome.update.is_none() {
                outcome.update = self.flush_delta();
            }
        }
        // First, so the undo state stays the answer's last event (`UNDO-006`).
        if let Some(gone) = self.arrows_gone() {
            outcome.display.insert(0, gone);
        }
        self.announce_write(outcome)
    }

    /// Land a running import whole, now, and answer its last delta
    /// (`TOILE-IO-028`). For the exports the Logic thread serves itself, which
    /// never reach [`handle_command`](Self::handle_command).
    pub fn settle(&mut self) -> ToileOutcome {
        let mut display = self.finish_running_import();
        display.extend(self.arrows_gone());
        let outcome = self.emit_with_update(display);
        self.announce_write(outcome)
    }

    /// Append [`BoardChanged`](ToileEvent::BoardChanged) and the undo state to
    /// an outcome that wrote the board.
    pub(super) fn announce_write(&self, mut outcome: ToileOutcome) -> ToileOutcome {
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
            ToileCommand::LockElements { ids, lock } => self.lock_elements(&ids, lock),
            ToileCommand::Reorder { node, to } => self.reorder(node, to),
            ToileCommand::GroupElements { ids } => self.group_elements(&ids),
            ToileCommand::UngroupElements { groups } => self.ungroup_elements(&groups),
            ToileCommand::SetFrameChildren { frame, add, remove } => {
                self.set_frame_children(&frame, &add, &remove)
            }
            ToileCommand::RemoveFromFrames { ids } => self.remove_from_frames(&ids),
            ToileCommand::RenameFrame { frame, name } => self.rename_frame(&frame, &name),
            ToileCommand::SetDocumentLocale { locale } => self.set_document_locale(&locale),
            ToileCommand::SetSlideOrder { order } => self.set_slide_order(&order),
            ToileCommand::NextSlide => self.next_slide(),
            ToileCommand::PrevSlide => self.prev_slide(),
            ToileCommand::GotoSlide(idx) => self.goto_slide(idx),
            ToileCommand::EnterPresent { focus } => self.enter_present(focus),
            ToileCommand::ExitPresent => self.exit_present(),
            ToileCommand::SetPresentBroadcast(on) => self.set_present_broadcast(on),
            ToileCommand::FollowPresenter(who) => self.follow_presenter(who),
            ToileCommand::EditNotes { frame, base, delta } => {
                self.edit_notes(&frame, &base, &delta)
            }
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
            ToileCommand::ResolveThread { thread, state } => self.resolve_thread(&thread, state),
            ToileCommand::ReadComments { thread } => self.read_comments(thread.as_ref()),
            ToileCommand::BooleanOp { op, ids, sources } => self.boolean_op(op, &ids, sources),
            ToileCommand::ReadDeck => self.read_deck(),
            // One answer per address the document resolves. An address it does
            // not carry earns no answer, so the element keeps the placeholder it
            // already paints (`toile-media`).
            ToileCommand::RequestMedia { hashes } => ToileOutcome::display_only(
                hashes
                    .iter()
                    .filter_map(|h| {
                        self.decoded_thumbnail(h)
                            .map(|decoded| ToileEvent::MediaDecoded {
                                hash: h.clone(),
                                size: decoded.size,
                                pixels: decoded.pixels,
                            })
                    })
                    .collect(),
            ),
            ToileCommand::Undo => self.apply_history(HistoryStep::Undo),
            ToileCommand::Redo => self.apply_history(HistoryStep::Redo),
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
                from,
                direction,
            } => self.find(query.as_ref(), from.as_ref(), direction),
            ToileCommand::RequestSelectionStats { ids, kind } => self.selection_stats(&ids, kind),
            ToileCommand::RequestSelectionData { ids } => self.selection_data(&ids),
            ToileCommand::RequestBoardIds => self.board_ids(),
            ToileCommand::Interop(InteropRequest::Import(ToileImport { format, data, at })) => {
                match format {
                    ToileImportFormat::Svg => {
                        // The images module read the file first
                        // (`handle_conversion`). A caller that reaches this
                        // arm with nothing read is answered by name.
                        let _ = data;
                        let read = match self.converted.take() {
                            Some(import::ConvertedFile::Drawing(read)) => read,
                            _ => Err("the images converter is not loaded".to_owned()),
                        };
                        self.import_svg_drawing(read, at)
                    }
                    // A WebP picture arrives decoded by the images module
                    // (`handle_conversion`) and lands as PNG; any other
                    // picture is read here, as it came.
                    ToileImportFormat::Image(_) => match self.converted.take() {
                        Some(import::ConvertedFile::Picture(Ok(pixels))) => {
                            match crate::logic::interop::png_of(&pixels) {
                                Ok(png) => self.import_image_bytes(&png, at),
                                Err(e) => Self::interop_failed(Direction::Import, e.to_string()),
                            }
                        }
                        Some(import::ConvertedFile::Picture(Err(reason))) => {
                            Self::interop_failed(Direction::Import, reason)
                        }
                        _ => self.import_image_bytes(&data, at),
                    },
                    // No converter reads either format yet. The refusal names
                    // the one asked for and touches nothing (`TOILE-IO-094`).
                    ToileImportFormat::Figma => {
                        Self::interop_failed(Direction::Import, "Figma import is not yet available")
                    }
                    ToileImportFormat::Miro => {
                        Self::interop_failed(Direction::Import, "Miro import is not yet available")
                    }
                }
            }
            ToileCommand::Interop(InteropRequest::Export(format)) => match format {
                ToileExport::Svg => self.export_svg_document(),
                // Served by the Logic thread, which holds the base font faces
                // the render walk needs (`toile-pdf-export`, `TOILE-IO-029`).
                // Its dispatcher intercepts the command before the document
                // lookup. Only a caller driving `ToileState` directly therefore
                // lands here, and this arm answers rather than falling silent.
                ToileExport::Pdf => Self::interop_failed(
                    Direction::Export,
                    "the deck is exported by the Logic thread, not by the board",
                ),
                // The writer runs in the Office module, which reaches the board
                // through `handle_conversion` alone. A caller driving the
                // state without a module lands here, and learns why.
                ToileExport::Pptx => Self::interop_failed(
                    Direction::Export,
                    "the PowerPoint writer runs in its converter module",
                ),
                // Served by the Logic thread for a different reason: the
                // container's header carries the document's identity, which a
                // board does not know (`ambre-spec-docfile.md`). A silent arm
                // is indistinguishable from a gesture that never arrived.
                ToileExport::Ambre | ToileExport::AmbreSealed { .. } => Self::interop_failed(
                    Direction::Export,
                    "the file is written by the Logic thread, not by the board",
                ),
            },
            ToileCommand::SetViewport(vp) => {
                self.viewport = vp;
                self.viewport_version = self.viewport_version.next();
                ToileOutcome::display_only(vec![self.viewport_event()])
            }
            ToileCommand::Presence { payload, now } => self.broadcast_presence(&payload, now),
        }
    }

    // -- remote / persistence --------------------------------------------

    /// Apply a remote delta, refresh the derived caches it touched
    /// (`TOILE-092`), and return the display events to repaint. A remote
    /// import is never re-broadcast.
    pub fn apply_remote(
        &mut self,
        update: &ambre_core::Payload<ambre_core::DeltaBytes>,
    ) -> Vec<ToileEvent> {
        self.import_remote(update.into())
    }

    /// A snapshot whose contents are trusted, merged the way a remote delta
    /// is. A board opened from a file or from the local store goes through
    /// [`import_snapshot`](Self::import_snapshot) instead, which gates its
    /// schema.
    pub fn apply_remote_snapshot(
        &mut self,
        snapshot: &ambre_core::Payload<ambre_core::SnapshotBytes>,
    ) -> Vec<ToileEvent> {
        self.import_remote(snapshot.into())
    }

    /// The events that draw this board whole, and write nothing: the
    /// viewport, the board and deck changes every panel re-reads on, and the
    /// undo state. A view that lost an event takes them in place of what it
    /// missed (`ambre-spec-message-bus.md`, `BUS-042`).
    #[must_use]
    pub fn whole_state(&self) -> Vec<ToileEvent> {
        vec![
            self.viewport_event(),
            ToileEvent::BoardChanged,
            ToileEvent::DeckChanged,
            self.undo_state_event(),
        ]
    }

    /// The tail of [`apply_remote`](Self::apply_remote): `doc.import` reads a
    /// delta and a snapshot alike.
    fn import_remote(&mut self, payload: ambre_core::LoroPayloadRef<'_>) -> Vec<ToileEvent> {
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        self.forget_touched();
        if let Err(e) = self.doc.import(payload.as_bytes()) {
            tracing::warn!("Toile: remote import failed: {e}");
            return vec![ToileEvent::Error(ToileError::RemoteImportFailed)];
        }
        self.absorb_imported();
        self.refresh_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        events.extend(self.arrows_gone());
        // A delta can carry a register this build cannot decode just as a
        // snapshot can, and the user deserves the same word (`SCHEMA-003`).
        events.extend(self.undecodable_notice());
        events
    }

    /// Apply a batch of remote deltas (a server history replay on open) as one
    /// `import` sweep plus a single cache refresh. The UI therefore repaints
    /// once instead of once per delta. Mirrors
    /// [`apply_remote`](Self::apply_remote) but amortizes the derived-cache
    /// refresh over the whole batch. A delta that fails to import is skipped,
    /// and the batch then reports the failure instead of answering success
    /// (`TOILE-215`).
    pub fn apply_remote_batch(
        &mut self,
        updates: &[ambre_core::Payload<ambre_core::DeltaBytes>],
    ) -> Vec<ToileEvent> {
        if updates.is_empty() {
            return Vec::new();
        }
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        let mut any_ok = false;
        let mut any_failed = false;
        self.forget_touched();
        for update in updates {
            match self.doc.import(update.as_bytes()) {
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
        self.refresh_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        events.extend(self.arrows_gone());
        if any_failed {
            events.push(ToileEvent::Error(ToileError::RemoteImportFailed));
        }
        events.extend(self.undecodable_notice());
        events
    }

    /// The CRDT document, read-only. A snapshot check compares its history
    /// with a peer's snapshot and never writes (`AUTHOR-018`).
    #[must_use]
    pub fn crdt(&self) -> &loro::LoroDoc {
        &self.doc
    }

    /// Full CRDT snapshot for local persistence / compaction / SnapshotNeeded.
    pub fn export_snapshot(&self) -> ambre_core::Payload<ambre_core::SnapshotBytes> {
        ambre_core::Payload::<ambre_core::SnapshotBytes>::new(
            self.doc.export(ExportMode::Snapshot).unwrap_or_default(),
        )
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
    pub fn import_snapshot(
        &mut self,
        payload: &ambre_core::Payload<ambre_core::SnapshotBytes>,
    ) -> Result<Vec<ToileEvent>, SchemaError> {
        // Before the caller touches a single container (`SCHEMA-006`). What
        // comes back is the payload itself at the current version, or a migrated
        // snapshot.
        let payload = schema::gate(payload)?;
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        if let Err(e) = self.doc.import(payload.as_bytes()) {
            tracing::warn!("Toile: snapshot import failed: {e}");
            return Err(SchemaError::PayloadRejected);
        }
        self.rebuild_caches();
        let mut events = vec![self.viewport_event(), ToileEvent::BoardChanged];
        events.extend(self.deck_change(&deck_before));
        events.extend(self.notes_change(&notes_before));
        events.extend(self.newly_dangling(&dangling_before));
        events.extend(self.arrows_gone());
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
    pub fn schema_version(&self) -> ambre_core::Version<ambre_core::schema::Schema> {
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

    /// Apply one undo or redo step. The history writes the inverse ops. The
    /// commit lands them under the replay origin, so they record no step. This
    /// method then refreshes the caches of what the step wrote and broadcasts
    /// the delta like any structural edit. A no-op (empty stack) only reports availability.
    fn apply_history(&mut self, step: HistoryStep) -> ToileOutcome {
        // Read before the step: the deck and the notes come off the document,
        // which the step has already moved once it returns.
        let deck_before = self.slide_order();
        let notes_before = self.active_notes();
        let dangling_before = self.dangling_connectors();
        // An empty stack changes nothing, so it refreshes nothing
        // (`UNDO-009`).
        self.doc.commit();
        // The local command before this step kept the caches in step itself,
        // so the refresh reads only what the step's own commit writes.
        self.forget_touched();
        if self.history.write(&self.doc, step) == ambre_crdt::history::Replayed::Nothing {
            return ToileOutcome::display_only(Vec::new());
        }
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Recorded.as_str());
        self.doc.commit();
        // The document's events name what the step wrote, as for a remote
        // change (`UNDO-005`, `TOILE-292`).
        self.refresh_caches();
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
    pub(super) fn deck_change(&self, before: &[Id<Frame>]) -> Option<ToileEvent> {
        (self.slide_order() != before).then_some(ToileEvent::DeckChanged)
    }

    /// One `DanglingConnector` per connector that was whole before this write
    /// and is not any more (`TOILE-165`). Every write that can strand an arrow
    /// without naming it calls this: a remote delta, a history step, a peer's
    /// deletion. The re-aim path announces its own.
    pub(super) fn newly_dangling(&self, before: &HashSet<Id<Connector>>) -> Vec<ToileEvent> {
        let mut fresh: Vec<Id<Connector>> = self
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
    pub(super) fn active_notes(&self) -> Option<(Id<Frame>, String)> {
        let frame = self
            .slide_order()
            .get(self.playback.active.to_index()?)
            .cloned()?;
        // A slide with no notes yet reads as empty ones.
        let notes = self.notes(&frame).unwrap_or_default();
        Some((frame, notes))
    }

    /// `NotesChanged` when the note on screen is no longer `before`
    /// (`TOILE-UI-316`). Every write that can move it without naming it calls
    /// this: a remote delta, a history step. The editor is otherwise seeded by
    /// the slide event alone, so a co-author's writing stays invisible until
    /// the slide changes.
    pub(super) fn notes_change(&self, before: &Option<(Id<Frame>, String)>) -> Option<ToileEvent> {
        let (frame, notes) = self.active_notes()?;
        match before {
            Some((f, n)) if *f == frame && *n == notes => None,
            _ => Some(ToileEvent::NotesChanged { frame, notes }),
        }
    }

    pub(super) fn undo_state_event(&self) -> ToileEvent {
        ToileEvent::UndoState {
            undo: self.history.can_undo(),
            redo: self.history.can_redo(),
        }
    }

    // -- selection aggregates -----------------------------------

    fn selection_stats(&self, ids: &[Id<Element>], kind: StatsRequestKind) -> ToileOutcome {
        let mut union: Option<Rect> = None;
        let mut count = Count::<Elements>::ZERO;
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
                count = count.incremented();
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
                count = count.incremented();
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
    /// cannot read off a box (`TOILE-UI-384`). Last, it names the ids the board
    /// no longer holds, which the view drops from its selection (`TOILE-UI-385`).
    fn selection_data(&self, ids: &[Id<Element>]) -> ToileOutcome {
        let elements = ids
            .iter()
            .filter_map(|id| self.display_element(id))
            .collect();
        let framed = self.framed_among(ids);
        // Read from the element map, not from the display answer: an element
        // the board holds is never reported gone (`TOILE-UI-385`).
        let missing = ids
            .iter()
            .filter(|id| self.element_map(id).is_none())
            .cloned()
            .collect();
        ToileOutcome::display_only(vec![ToileEvent::SelectionData {
            elements,
            framed,
            missing,
        }])
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
        let elems = self.doc.get_map(Root::Elements.as_str());
        let LoroValue::Map(entries) = elems.get_value() else {
            return ToileOutcome::display_only(vec![ToileEvent::BoardIds(Vec::new())]);
        };
        let mut ids: Vec<Id<Element>> = entries
            .keys()
            .filter_map(|key| {
                let id = Id::<Element>::from_text(key.to_string());
                let em = self.element_map(&id)?;
                if codec::locked(&em) || codec::has_type(&em, ElementKind::Group) {
                    None
                } else {
                    Some(id)
                }
            })
            .collect();
        // By id, so every peer answers the same list (`TOILE-121`). The map's
        // own iteration order carries no such promise.
        ids.sort_unstable();
        ToileOutcome::display_only(vec![ToileEvent::BoardIds(ids)])
    }

    // -- outcome plumbing ------------------------------------------------------

    /// Wrap the current export delta (since `exported_vv`) into the outcome.
    /// Every mutating command routes through here.
    /// The answer to a conversion that produced nothing: the board is as it
    /// was, and the user reads why (`INTEROP-011`).
    pub(super) fn interop_failed(direction: Direction, reason: impl Into<String>) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Interop(InteropAnswer::failed(
            direction,
            InteropError::Stopped(reason.into()),
        ))])
    }

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
    fn flush_delta(&mut self) -> Option<ambre_core::Payload<ambre_core::DeltaBytes>> {
        let current = self.doc.oplog_vv();
        if current == self.exported_vv {
            return None;
        }
        // The registry entry alone is no edit: it waits for the first one and
        // travels with it (`AUTHOR-009`).
        if let Some(end) = self.entry_end {
            if current.get(&self.peer.get()) == Some(&end.get()) {
                return None;
            }
            self.entry_end = None;
        }
        let exported = self
            .doc
            .export(ExportMode::updates(&self.exported_vv))
            .map(ambre_core::Payload::<ambre_core::DeltaBytes>::new);
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
        exported: Result<ambre_core::Payload<ambre_core::DeltaBytes>, E>,
    ) -> Option<ambre_core::Payload<ambre_core::DeltaBytes>> {
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
        let ours = self.exported_vv.get(&self.peer.get()).copied();
        self.exported_vv = self.doc.oplog_vv();
        match ours {
            Some(counter) => {
                self.exported_vv.insert(self.peer.get(), counter);
            }
            None => {
                self.exported_vv.remove(&self.peer.get());
            }
        }
    }

    pub(super) fn err_no_element(&self, id: &Id<Element>) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchElement(
            id.clone(),
        ))])
    }

    pub(super) fn err_no_connector(&self, id: &Id<Connector>) -> ToileOutcome {
        ToileOutcome::display_only(vec![ToileEvent::Error(ToileError::NoSuchConnector(
            id.clone(),
        ))])
    }

    // -- test / integration accessors -----------------------------------------

    /// Number of elements currently in the CRDT. Test helper.
    pub fn element_count(&self) -> ambre_core::Count<ambre_core::Elements> {
        ambre_core::Count::<ambre_core::Elements>::of(self.bounds.len())
    }

    /// How many times the whole element map has been resolved. Test helper.
    pub fn board_walks(&self) -> Count<BoardWalks> {
        self.board_walks.get()
    }

    /// Every derived cache, in an order that does not depend on how it was
    /// filled: bounds, leaves, both membership maps and the paint order. Two
    /// states whose caches agree print the same text. Test helper.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn debug_caches(&self) -> String {
        fn sorted<K: Ord + std::fmt::Debug, V: std::fmt::Debug>(
            it: impl Iterator<Item = (K, V)>,
        ) -> Vec<String> {
            let mut v: Vec<(K, V)> = it.collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v.into_iter().map(|(k, v)| format!("{k:?}={v:?}")).collect()
        }
        let lists = |m: &HashMap<Id<Element>, Vec<Id<Element>>>| {
            sorted(m.iter().map(|(k, v)| {
                let mut v = v.clone();
                v.sort();
                (k.clone(), v)
            }))
        };
        let mut leaves: Vec<Id<Element>> = self.index.ids().collect();
        leaves.sort();
        format!(
            "bounds {:?}\nleaves {leaves:?}\nparents {:?}\nmembers {:?}\nlayers {:?}",
            sorted(self.bounds.iter().map(|(k, r)| (k.clone(), *r))),
            lists(&self.parents),
            lists(&self.members),
            self.layers,
        )
    }

    /// The history this board holds, as a version vector. Test helper.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn history(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    /// How many times a call walked every leaf of the spatial index. Test helper.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn index_walks(&self) -> usize {
        self.index.walks()
    }

    /// Materialized paint order. Test helper.
    pub fn layer_order(&self) -> &[NodeId] {
        &self.layers
    }

    /// Derived world AABB of an element. Test helper.
    pub fn bounds_of(&self, id: &Id<Element>) -> Option<Rect> {
        self.bounds.get(id).copied()
    }

    /// The decoded style register of an element, or the default when absent.
    /// Test helper.
    pub fn style_of(&self, id: &Id<Element>) -> ElementStyle {
        self.element_map(id)
            .map(|em| self.style_read(codec::bin_of(&em, ElementKey::Style).as_deref()))
            .unwrap_or_default()
    }

    /// Membership of a frame / group, in `children` order. Test helper.
    pub fn membership_of(&self, id: &Id<Element>) -> Vec<Id<Element>> {
        self.children_of(id)
    }

    /// The culled, z-ordered `DisplayViewport` for the current viewport. Test
    /// helper (the production path returns it inside a `ToileEvent`).
    pub fn debug_viewport(&self) -> crate::logic::command::DisplayViewport {
        self.derive_viewport()
    }

    /// Overwrite one raw string field of an element's map, behind the engine's
    /// back. Test helper: it exists to forge the state a newer or broken peer
    /// could merge into us, which no command of ours can produce. Its one
    /// caller, `convergence_tests`, is host-only, so the helper is too.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn debug_set_element_field(
        &mut self,
        id: &Id<Element>,
        key: ElementKey,
        value: &str,
    ) {
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
        if let LoroValue::List(cur) = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .get_deep_value()
        {
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
        let _ = self.doc.get_movable_list(Root::Layers.as_str()).push(s);
        self.rematerialize_layers();
    }

    /// Remove one node's entry from the `layers` list, leaving the node in the
    /// document. Test helper: it forges the concurrent drop `TOILE-058` names, a
    /// state no command of ours can produce.
    pub fn debug_remove_layer_entry(&mut self, node: &NodeId) {
        let list = self.doc.get_movable_list(Root::Layers.as_str());
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
    pub fn connector_ids(&self) -> Vec<Id<Connector>> {
        let mut ids = Vec::new();
        if let LoroValue::Map(m) = self.doc.get_map(Root::Connectors.as_str()).get_deep_value() {
            ids.extend(m.keys().map(|k| Id::<Connector>::from_text(k.clone())));
        }
        ids.sort_unstable();
        ids
    }
}

/// Whether a command must wait for a running import to land whole
/// (`TOILE-IO-028`). Only a camera move, a cursor and slide navigation pass:
/// they read nothing the import is still writing. Every other command writes
/// the board or reads it, and a half-applied import would answer it wrongly.
fn waits_for_import(cmd: &ToileCommand) -> bool {
    !matches!(
        cmd,
        ToileCommand::SetViewport(_)
            | ToileCommand::Presence { .. }
            | ToileCommand::NextSlide
            | ToileCommand::PrevSlide
            | ToileCommand::GotoSlide(_)
    )
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

    use crate::message::ToileEvent;
    use ambre_core::schema::SchemaError;

    use crate::logic::command::{InitContent, ToileCommand};
    use crate::logic::model::{ElementKind, ShapeKind};
    use crate::logic::schema::SCHEMA_VERSION;
    use crate::logic::state::{ElementKey, Root, ToileState};

    fn board_with_one_rect() -> ToileState {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
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
            doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
                .insert(
                    ambre_core::schema::MetaKey::SchemaVersion.as_str(),
                    i64::from(SCHEMA_VERSION.next().get()),
                )
                .expect("stamp");
            doc.get_map(Root::Elements.as_str())
                .insert_container("xx", loro::LoroMap::new())
                .expect("an element the refusal must not import");
            doc.commit();
            doc.export(ExportMode::Snapshot).expect("snapshot")
        };

        let mut st = board_with_one_rect();
        let before = st.export_snapshot();
        let err = st
            .import_snapshot(&ambre_core::Payload::<ambre_core::SnapshotBytes>::new(
                newer.to_vec(),
            ))
            .expect_err("refused");

        assert_eq!(
            err,
            SchemaError::DocumentTooNew {
                document: SCHEMA_VERSION.next(),
                binary: SCHEMA_VERSION,
            }
        );
        assert_eq!(
            st.element_count(),
            ambre_core::Count::<ambre_core::Elements>::new(1),
            "nothing was merged in"
        );
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
        em.insert(
            ElementKey::Style,
            loro::LoroValue::Binary(vec![0xFFu8; 8].into()),
        )
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
            Err::<ambre_core::Payload<ambre_core::DeltaBytes>, &str>("the exporter said no"),
        );
        assert!(out.is_none(), "a failed export broadcasts nothing");
        assert_eq!(
            cursor,
            VersionVector::default(),
            "a failed export must not advance the cursor"
        );

        // The next flush offers the same range, and this time it lands.
        let out = ToileState::take_delta(
            &mut cursor,
            current.clone(),
            Ok::<_, &str>(vec![1, 2, 3].into()),
        );
        assert_eq!(out, Some(vec![1, 2, 3].into()));
        assert_eq!(cursor, current, "a successful export advances the cursor");
    }

    /// An export that succeeds with nothing in it still advances the cursor.
    /// An empty range is not a failure, and it holds nothing to offer again.
    #[test]
    fn an_empty_export_is_not_a_failure() {
        let st = ToileState::new();
        let current = st.doc.oplog_vv();
        let mut cursor = VersionVector::default();

        let out = ToileState::take_delta(
            &mut cursor,
            current.clone(),
            Ok::<_, &str>(ambre_core::Payload::<ambre_core::DeltaBytes>::default()),
        );
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

    use crate::message::ToileEvent;

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
            correlate: None,
        });
        assert!(
            matches!(
                evts.display.last(),
                Some(ToileEvent::UndoState {
                    undo: ambre_core::Availability::Enabled,
                    redo: ambre_core::Availability::Disabled
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

    /// `UNDO-009`: a command that changes nothing refreshes no affordance, the
    /// history steps on an empty stack included.
    #[test]
    fn a_command_that_changes_nothing_reports_no_undo_state() {
        let mut st = ToileState::new();
        st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        for no_op in [
            ToileCommand::Redo,
            ToileCommand::ReadDeck,
            ToileCommand::DeleteElements { ids: Vec::new() },
            ToileCommand::MoveElements {
                ids: Vec::new(),
                delta: kurbo::Vec2::new(5.0, 5.0),
            },
            ToileCommand::LockElements {
                ids: Vec::new(),
                lock: crate::message::LockState::Locked,
            },
        ] {
            let label = format!("{no_op:?}");
            let evts = st.handle_command(no_op);
            assert!(evts.update.is_none(), "{label} wrote");
            assert!(
                !evts
                    .display
                    .iter()
                    .any(|e| matches!(e, ToileEvent::UndoState { .. })),
                "{label} refreshed the history",
            );
        }
        st.handle_command(ToileCommand::Undo);
        let evts = st.handle_command(ToileCommand::Undo);
        assert!(evts.update.is_none() && evts.display.is_empty());
    }
}
