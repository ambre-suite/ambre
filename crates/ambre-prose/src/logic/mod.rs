//! `ProseState` -- the per-document Prose engine, Logic thread side
//! (`features/ambre-spec-prose-engine.md`).
//!
//! It owns the single-document `LoroDoc`, the local undo manager, the decrypted
//! picture cache and the export cursor. It turns a typed [`ProseCommand`] into a
//! [`ProseOutcome`], that is display events plus CRDT and ephemeral bytes. The
//! `LoroDoc` never leaves this thread; it exits only as bytes (`PROSE-001`).
//!
//! **The state names no channel.** It holds no sender, no `SyncCommand` and no
//! incoming event type. A handler answers with a value and the `ambre-logic`
//! dispatcher fans it out (`PROSE-024`). A connection change, a sharing change
//! and a revoked member are not document content and never reach this state at
//! all (`PROSE-063`).
//!
//! **Engine-core scope** (`ARCH P2`). The document model: text, blocks, marks
//! and nesting, the tables, plus the remote, snapshot, presence and picture
//! paths. The comments, the interop filters and the find machinery each land
//! with their own Story (`ambre-roadmap.md` E5).
//!
//! The `impl` splits across the sibling modules, and each of them reaches the
//! struct's fields and shared helpers through `super::`:
//! - [`text`] -- typing, deleting, the cross-paragraph join and the split
//! - [`blocks`] -- the block lifecycle, the nesting guard and the styles
//! - [`marks`] -- the inline mark registry's write path
//! - [`presence`] -- the cursor's presence sample, outbound and inbound
//! - [`tables`] -- the grid, its column widths and its merged regions
//! - [`derived`] -- the tables of contents and the word count, read and never
//!   stored
//! - [`bands`] -- the headers, the footers and their fields
//! - [`find`] -- the search session and its two replacements

mod bands;
mod blocks;
mod catchup;
mod clipboard;
mod comments;
mod derived;
mod docfile;
mod find;
mod history;
mod ids;
mod interop;
mod lists;
mod marks;
pub(crate) mod notes;
mod presence;
mod suggestions;
mod sweep;
pub(crate) mod tables;
mod text;
mod touched;
mod xrefs;

/// The per-application half of `shape_of` and `canonical_dump`, and Prose's
/// generators (`ambre-spec-fixtures.md`). Fixture code, so the whole module is
/// behind the non-default `fixtures` feature and no client build reaches it.
#[cfg(feature = "fixtures")]
pub mod fixtures;

#[cfg(test)]
mod convergence_tests;

/// The schema shape belt (`SCHEMA-018`): the key set and the value types a
/// build writes, frozen per `SCHEMA_VERSION`. Test-only, so no client build
/// carries it.
#[cfg(test)]
mod schema_shape;

/// The reading belt (`SCHEMA-017`): what each stored value means, frozen per
/// `SCHEMA_VERSION` over the shape belt's scenario. Test-only, like it.
#[cfg(test)]
mod schema_reading;

use std::collections::HashMap;

use loro::{ExportMode, LoroDoc, TreeID, UndoManager, VersionVector};

use ambre_core::schema::SchemaError;
use ambre_core::{HistoryStep, StepDirection, UserId};

use crate::doc;
use crate::events::{
    PageCount, PageIndex, PageSpan, ProseCommand, ProseError, ProseEvent, ProseOutcome,
    ProseRefusal, block_id, tree_id,
};
use crate::layout::pass::PageLayout;
use crate::layout::reflow::Paginator;
use crate::layout::run::LayoutVersion;
use crate::layout::run::ShapedLayout;
use crate::mark;
use crate::schema;

/// The pages a layout carries before the view asks for any (`PROSE-125`).
pub const WINDOW_PAGES: PageCount = PageCount::new(8);

/// The pages carried on each side of a block the view reveals
/// (`PROSE-UI-212`).
pub const WINDOW_MARGIN: PageCount = PageCount::new(4);

/// The per-document Prose engine.
pub struct ProseState {
    /// The CRDT, private and named by no public signature (`PROSE-001`).
    doc: LoroDoc,
    /// Local-only undo and redo: it retracts this peer's operations alone, one
    /// item per atomic user action.
    undo: UndoManager,
    /// Decrypted picture bytes, by content hash. Session state: it never enters
    /// the document (`PROSE-044`) and it dies with the tab.
    media_cache: HashMap<ambre_core::ContentHash, Vec<u8>>,
    /// The version vector already exported to Sync. A delta is every operation
    /// past it (`PROSE-050`, `PROSE-098`).
    last_exported_vv: VersionVector,
    /// This document's own peer id, cached at construction.
    peer: ambre_core::PeerId,
    /// The canonical per-user identifier, seeded from `SessionReady` by the
    /// dispatcher. A stable `UserId`, never the per-session peer id.
    my_user_id: Option<UserId>,
    /// This session's display name, stamped on every outbound presence sample.
    /// Cosmetic: it decides neither identity nor colour (`PRESENCE-004`).
    my_name: Option<ambre_core::DisplayName>,
    /// The threads the last whole-text walk found with no mark. A keystroke
    /// does not walk the text again for them (`comments::refresh_anchors`).
    orphans: std::collections::HashSet<ambre_core::CommentId>,
    /// Whether this session turns typing and deleting into suggestions.
    /// Per session, never shared (`features/ambre-spec-prose-engine.md`).
    suggesting: bool,
    /// The suggestion the next adjacent edit joins (`PROSE-FEAT-085`).
    burst: Option<suggestions::Burst>,
    /// Every remote peer's latest caret and self-asserted name, keyed by the
    /// Sync-resolved `UserId`. Ephemeral: never in the CRDT, never persisted
    /// (`PROSE-058`). The name is cosmetic and decides nothing
    /// (`PRESENCE-004`).
    presence: HashMap<
        UserId,
        (
            crate::events::CursorPosition,
            Option<ambre_core::DisplayName>,
        ),
    >,
    /// Every user with a session open on this document, as far as this peer
    /// knows. A user joins on any frame that decodes and leaves on the relay's
    /// word alone (`PRESENCE-014`). Apart from `presence`, which drops a caret
    /// it cannot place.
    live: std::collections::HashSet<UserId>,
    /// This session's last presence sample, re-sent to a newcomer.
    last_frame: Option<ambre_core::EphemeralBytes>,
    /// Whether a newcomer has not heard from this session yet.
    owe_greeting: bool,
    /// The paginator, built on the first layout asked for. It is the one place
    /// the document is measured, for the screen and for export alike
    /// (`PROSE-064`). It lives here so measuring runs where the document does
    /// (`PROSE-010`).
    paginator: Option<Paginator>,
    /// The font faces every peer holds, as bytes (`PROSE-114`). A measurer
    /// discovers none on its own, so these are the whole of what pagination
    /// shapes on. They arrive from the Main thread, which is the only one that
    /// can fetch (`LogicCommand::LoadFonts`).
    faces: Vec<ambre_core::FontBytes>,
    /// What the last layout handed to the view was computed from: the
    /// document's state and how many faces it was shaped on. A layout from the
    /// same basis is the same picture, so it is not handed over twice.
    handed: Option<(loro::Frontiers, usize, PageSpan)>,
    /// The pages the view last asked for (`PROSE-125`).
    window: PageSpan,
    /// The version stamped on the last layout handed over. It only grows, even
    /// across a snapshot import that rebuilds the paginator: the view drops a
    /// layout older than the one it holds (`PROSE-UI-016`).
    handed_version: LayoutVersion,
    /// The layouts handed over that the view may still hold. This thread
    /// frees each one once the view has let go of it, so the view never
    /// pays for freeing a whole layout (`PROSE-UI-205`).
    in_view: Vec<std::sync::Arc<crate::layout::run::ShapedLayout>>,
    /// Where the ids of notes, threads, replies and suggestions come from:
    /// fresh on a live document, a sequence on a pinned one (`FIXTURE-002`).
    ids: ids::IdSource,
    /// The five figures as last answered (`PROSE-FEAT-018`). Session state:
    /// the tallies move on a recount and the page count on a layout, and
    /// neither ever reaches the document.
    word_count: crate::events::WordCount,
    /// The open search, if any. Session state: it never enters the document
    /// and no other peer sees it (`FIND-001`, `PROSE-FEAT-148`).
    find: Option<find::FindSession>,
    /// This peer's own operation counter when the last-modified stamp was
    /// last written. A stamp is owed only past it (`PROSE-FEAT-066`).
    stamped_counter: ambre_core::OpCounter,
    /// The blocks written to since the last layout, whoever wrote them.
    touched: touched::Touched,
    /// The changes the view has not drawn yet, and what it drew for the
    /// command in dispatch (`catchup`).
    catchup: catchup::CatchUp,
    /// This PeerID's counter just past its registry entry, while the entry is
    /// all the session wrote. No delta goes out for it alone; the first edit
    /// carries it (`AUTHOR-009`). Set by [`adopt_session`](Self::adopt_session).
    entry_end: Option<ambre_core::OpCounter>,
}

/// The document scaffolding, which every peer writes on open, before any edit.
/// `None` when the CRDT engine refused a write.
fn write_scaffold(doc: &LoroDoc) -> Option<()> {
    doc::seed(doc)?;
    // The version is scaffolding too, and rides the same commit, so no
    // document ever persists without it (`SCHEMA-004`, `PROSE-028`).
    schema::stamp(doc);
    Some(())
}

/// The identity the scaffolding is written under: a hash of the scaffolding
/// this build writes (`PROSE-129`). Two builds that scaffold alike share it,
/// and two that differ never write two changes under one identifier.
fn scaffold_identity() -> Option<ambre_core::PeerId> {
    static PEER: std::sync::OnceLock<Option<ambre_core::PeerId>> = std::sync::OnceLock::new();
    *PEER.get_or_init(|| {
        let scratch = LoroDoc::new();
        scratch.set_peer_id(0).ok()?;
        ProseState::prepare(&scratch);
        write_scaffold(&scratch)?;
        scratch.commit();
        let bytes = scratch.export(ExportMode::all_updates()).ok()?;
        Some(ambre_core::schema::scaffold_peer(&bytes))
    })
}

impl ProseState {
    /// A fresh document: the registers, the mark registry, and the one root
    /// paragraph the tree is never without (`PROSE-027`).
    ///
    /// `None` when the CRDT engine refused the scaffolding. A caller that meets
    /// it holds no document, which is the honest answer: opening a half-seeded
    /// one would break `PROSE-027` on the first read.
    #[must_use]
    pub fn new() -> Option<Self> {
        Self::from_doc(LoroDoc::new(), ids::IdSource::Random)
    }

    /// A document with a **pinned** CRDT identity and commit clock, so the same
    /// command sequence produces the same bytes twice (`FIXTURE-001`).
    ///
    /// **Never for a live document.** Two concurrent writers sharing a peer id
    /// mint conflicting operation ids and corrupt the history. This exists for
    /// single-owner, offline, host-side reproduction. The client path is
    /// [`new`](Self::new).
    #[must_use]
    pub fn with_peer(peer: ambre_core::PeerId) -> Option<Self> {
        let doc = LoroDoc::new();
        // Before any operation: every block the tree mints captures it. The
        // scaffolding is written under the identity every peer shares
        // (`PROSE-129`), so it is the same bytes whatever `peer` is.
        doc.set_peer_id(peer.0).ok()?;
        // No wall clock in the operations: a recorded timestamp would differ
        // between two runs of the same generator.
        doc.set_record_timestamp(false);
        // The ids the engine mints are pinned by the same peer.
        Self::from_doc(doc, ids::IdSource::seeded(peer))
    }

    /// The shared body of the two constructors, over a `LoroDoc` the caller has
    /// already configured.
    fn from_doc(doc: LoroDoc, ids: ids::IdSource) -> Option<Self> {
        Self::prepare(&doc);
        let peer = doc.peer_id();
        // Every peer writes the scaffolding as one shared identity, so every
        // copy is one change, the root paragraph included. Under the
        // document's own identity, a reader that has sent nothing would import
        // each change against the whole history (`PROSE-129`).
        doc.set_peer_id(scaffold_identity()?.0).ok()?;
        write_scaffold(&doc)?;
        doc.commit();
        doc.set_peer_id(peer).ok()?;
        // The undo manager opens after the scaffolding commit: seeding a
        // document is not a user edit and must not sit on the stack.
        let mut undo = UndoManager::new(&doc);
        // Neither is loading one. A full-state read drops the paragraph this
        // peer grew on opening, and that write lands after this manager exists.
        // Excluded by origin, an undo on a restored document retracts the
        // author's own edits and never the load (`UNDO-001`).
        undo.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Load.as_str());
        undo.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Stamp.as_str());
        undo.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Comment.as_str());
        // A removal for good is no gesture either (`PROSE-121`).
        undo.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Sweep.as_str());
        // The registry entry is bookkeeping, not prose: an undo never
        // retracts it (`AUTHOR-009`).
        undo.add_exclude_origin_prefix(ambre_core::crdt::CommitOrigin::Peers.as_str());
        let stamped_counter =
            ambre_core::OpCounter(doc.oplog_vv().get(&peer).copied().unwrap_or(0));
        // After the scaffolding: a document never laid out measures whole.
        let touched = touched::Touched::watch(&doc);
        Some(Self {
            doc,
            undo,
            media_cache: HashMap::new(),
            last_exported_vv: VersionVector::default(),
            peer: ambre_core::PeerId(peer),
            my_user_id: None,
            my_name: None,
            orphans: std::collections::HashSet::new(),
            suggesting: false,
            burst: None,
            presence: HashMap::new(),
            live: std::collections::HashSet::new(),
            last_frame: None,
            owe_greeting: false,
            paginator: None,
            faces: Vec::new(),
            handed: None,
            window: WINDOW_PAGES.span(),
            handed_version: LayoutVersion::FIRST,
            in_view: Vec::new(),
            ids,
            word_count: crate::events::WordCount::default(),
            find: None,
            stamped_counter,
            touched,
            catchup: catchup::CatchUp::default(),
            entry_end: None,
        })
    }

    /// Lay the document out, reusing what the last layout measured.
    ///
    /// The one place a page number, a break position or a measured height
    /// comes from. Each is derived on every call, never read back from the
    /// document (`PROSE-033`, `PROSE-064`). Every block written to since the
    /// last layout is re-measured, whoever wrote it (`PROSE-123`). `dirty`
    /// adds to those, and a layout with nothing written re-measures nothing
    /// (`PROSE-083`). The
    /// first call builds the paginator, since loading the faces costs more
    /// than a document that is never laid out should pay. The page count of
    /// the word count is stamped from the layout taken (`PROSE-FEAT-126`).
    pub fn paginate(&mut self, dirty: &[TreeID]) -> &PageLayout {
        let mut dirty = dirty.to_vec();
        // Loro hands over the events of an import at the next commit, so
        // flush them first. A commit with nothing pending writes nothing.
        self.doc.commit();
        dirty.extend(self.touched.take(&self.doc));
        let structural = self.touched.take_structural();
        let faces = &self.faces;
        let paginator = self.paginator.get_or_insert_with(|| {
            let mut paginator = Paginator::new();
            for bytes in faces {
                paginator.register_faces(bytes.as_bytes().to_vec());
            }
            paginator
        });
        let layout = if structural {
            paginator.reflow(&self.doc, &dirty)
        } else {
            paginator.reflow_text(&self.doc, &dirty)
        };
        self.word_count.pages = layout.page_count.get();
        layout
    }

    /// The layout as it stands, with the shaped runs the view draws on it
    /// (`features/ambre-spec-prose-page-rendering.md section 2`).
    ///
    /// It lays out first, so a caller never hands over a layout older than
    /// the document. Each call carries the paginator's version, and the view
    /// drops one older than what it holds (`PROSE-UI-016`).
    ///
    /// It carries the pages the view last asked for (`PROSE-125`).
    pub fn shaped_layout(&mut self) -> ShapedLayout {
        self.paginate(&[]);
        let window = self.window;
        self.paginator
            .as_ref()
            .map(|paginator| paginator.window(window))
            .unwrap_or_default()
    }

    /// Carry the pages `first` to `last` in the layouts handed from now on.
    fn show_pages(&mut self, first: PageIndex, last: PageIndex) -> ProseOutcome {
        self.window = first.min(last).through(first.max(last));
        ProseOutcome::nothing()
    }

    /// Carry the pages around the one `block` starts on (`PROSE-UI-212`).
    fn show_block(&mut self, block: &TreeID) -> ProseOutcome {
        self.paginate(&[]);
        let page = self
            .paginator
            .as_ref()
            .and_then(|paginator| paginator.page_of(block));
        match page {
            Some(page) => self.show_pages(page - WINDOW_MARGIN, page + WINDOW_MARGIN),
            None => ProseOutcome::nothing(),
        }
    }

    /// The layout for the view, when the document or its faces changed since
    /// the last one handed over (`PROSE-UI-006`).
    ///
    /// The Logic thread calls it after every command, import and face
    /// delivery. The view receives a layout already computed and shaped, and
    /// never lays text out itself. A caret move writes nothing, so it lays
    /// nothing out and sends nothing.
    pub fn layout_event(&mut self) -> Option<ProseEvent> {
        self.doc.commit();
        let basis = (self.doc.state_frontiers(), self.faces.len(), self.window);
        if self.handed.as_ref() == Some(&basis) {
            return None;
        }
        let mut shaped = self.shaped_layout();
        self.handed_version = self.handed_version.next();
        shaped.version = self.handed_version;
        self.handed = Some(basis);
        // A layout no one else holds is freed here, on this thread. One the
        // view still holds is kept, so its last handle is this one.
        self.in_view
            .retain(|layout| std::sync::Arc::strong_count(layout) > 1);
        let shaped = std::sync::Arc::new(shaped);
        self.in_view.push(std::sync::Arc::clone(&shaped));
        Some(ProseEvent::LayoutShaped {
            layout: ambre_bus::app::prose::SharedLayout::from_arc(shaped),
        })
    }

    /// The whole layout the paginator holds, beyond the window handed over.
    #[cfg(test)]
    pub(crate) fn full_layout(&self) -> Option<&crate::layout::pass::PageLayout> {
        self.paginator.as_ref().map(Paginator::layout)
    }

    /// How many runs the paginator has shaped, over its life.
    #[cfg(test)]
    pub(crate) fn shaped_count(&self) -> u64 {
        self.paginator.as_ref().map_or(0, Paginator::shaped)
    }

    /// How many faces this document measures on (`PROSE-115`).
    ///
    /// Zero means every run shapes into tofu: the measuring context is sealed
    /// and nothing has been handed to it yet.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Hand the document the faces pagination measures on (`PROSE-114`).
    ///
    /// A measurer holds nothing until this is called, so a layout taken before
    /// it shapes every run into tofu. A face that arrives after a layout was
    /// measured invalidates it, and the next call recomputes in full
    /// (`PROSE-111`).
    pub fn load_faces(&mut self, faces: &[ambre_core::FontBytes]) {
        for bytes in faces {
            if let Some(paginator) = self.paginator.as_mut() {
                paginator.register_faces(bytes.as_bytes().to_vec());
            }
            self.faces.push(bytes.clone());
        }
    }

    /// The two settings a document needs before its first operation, and after
    /// every import: the mark registry and the tree's ordering.
    ///
    /// Both are configuration rather than content, so a document that arrives
    /// as bytes carries neither and this re-applies them.
    fn prepare(doc: &LoroDoc) {
        mark::configure(doc);
        // A move addresses a sibling position, which Loro answers only with the
        // fractional index enabled. Without it `MoveBlock` could not place a
        // block after another one.
        doc::blocks(doc).enable_fractional_index(0);
    }

    // -- session identity --------------------------------------------------

    /// Write under the session identity Sync issued for this document
    /// (`AUTHOR-002`), and write its registry entry first (`AUTHOR-009`).
    ///
    /// The export cursor moves past everything the document held before. Those
    /// operations are not this session's to send (`AUTHOR-008`). The entry is
    /// written now, before any edit opens an undo group it would split. It is
    /// no edit, so it owes no last-modified stamp. `false` when the CRDT engine
    /// refused the identity, and then nothing changed.
    pub fn adopt_session(
        &mut self,
        peer: ambre_core::PeerId,
        entry: ambre_core::PeerEntry,
    ) -> bool {
        self.doc.commit();
        if let Err(e) = self.doc.set_peer_id(peer.0) {
            tracing::error!("Prose: the session identity was refused: {e:?}");
            return false;
        }
        self.peer = peer;
        self.last_exported_vv = self.doc.oplog_vv();
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
                    .get(&peer.0)
                    .copied()
                    .map(ambre_core::OpCounter)
            }
            Err(e) => {
                tracing::error!("Prose: the registry entry was refused: {e:?}");
                None
            }
        };
        self.stamped_counter = self.entry_end.unwrap_or_default();
        true
    }

    /// Seed the canonical `UserId`, from `LogicEvent::SessionReady`.
    pub fn set_user_id(&mut self, user_id: UserId) {
        self.my_user_id = Some(user_id);
    }

    /// The canonical `UserId` for this session, when a user has signed in.
    #[must_use]
    pub fn user_id(&self) -> Option<UserId> {
        self.my_user_id
    }

    /// Seed this session's display name, from the same `SessionReady`.
    pub fn set_display_name(&mut self, name: ambre_core::DisplayName) {
        self.my_name = Some(name);
    }

    // -- command dispatch --------------------------------------------------

    /// The one entry point a mutation reaches the document through
    /// (`PROSE-FEAT-005`).
    ///
    /// Every arm answers with a value; none of them sends (`PROSE-024`). A
    /// command that wrote nothing exports no delta (`PROSE-047`), because the
    /// export reads the version vector and finds it unchanged.
    pub fn handle_command(&mut self, command: ProseCommand) -> ProseOutcome {
        // The handlers refuse by reason alone. The command that drew the
        // refusal is known here and nowhere deeper, so the name is written in
        // on the way out (`PROSE-105`).
        let refused = command.name();
        self.catchup.predicting = catchup::Prediction::drawn_by(&command);
        let mut outcome = self.dispatch(command).named(refused);
        self.catchup.predicting = catchup::Prediction::None;
        self.report_history(&mut outcome);
        outcome
    }

    /// Tell the chrome what the history holds, after a command that wrote
    /// (`UNDO-006`). A command that wrote nothing exports no delta, and so
    /// sends no refresh (`UNDO-009`).
    fn report_history(&self, outcome: &mut ProseOutcome) {
        if outcome.update.is_some() {
            outcome.display.push(ProseEvent::UndoState {
                can_undo: self.undo.can_undo(),
                can_redo: self.undo.can_redo(),
            });
        }
    }

    /// [`handle_command`](Self::handle_command) for a command the view sent
    /// (`features/ambre-spec-prose-text-editing.md section 3`).
    ///
    /// The view's positions lag behind the changes it had not seen when it
    /// sent the command, so the command is caught up with them first. What
    /// the document did differently from what the view drew is told back.
    pub fn handle_view_command(&mut self, command: ProseCommand) -> ProseOutcome {
        let refused = command.name();
        let command = match self.catch_up(command) {
            Ok(command) => command,
            Err(outcome) => return outcome,
        };
        let mut outcome = self.dispatch(command).named(refused);
        self.settle(&mut outcome);
        self.report_history(&mut outcome);
        outcome
    }

    fn dispatch(&mut self, command: ProseCommand) -> ProseOutcome {
        if self.suggesting {
            if !suggestions::passes_while_suggesting(&command) {
                return ProseOutcome::refused(ProseRefusal::NotWhileSuggesting);
            }
            match command {
                ProseCommand::InsertText {
                    block_id,
                    pos,
                    text,
                } => return self.suggest_insert(&tree_id(block_id), pos, &text),
                ProseCommand::DeleteText { block_id, range } => {
                    let block = tree_id(block_id);
                    return self.suggest_delete((block, range.start), (block, range.end));
                }
                ProseCommand::DeleteRange { anchor, focus } => {
                    return self.suggest_delete(
                        (tree_id(anchor.0), anchor.1),
                        (tree_id(focus.0), focus.1),
                    );
                }
                _ => {}
            }
        }
        match command {
            // Answered by `catch_up` before dispatch.
            ProseCommand::Noop | ProseCommand::Sync { .. } => ProseOutcome::nothing(),
            ProseCommand::ShowPages { first, last } => self.show_pages(first, last),
            ProseCommand::ShowBlock { block_id } => self.show_block(&tree_id(block_id)),
            ProseCommand::InsertText {
                block_id,
                pos,
                text,
            } => self.insert_text(&tree_id(block_id), pos, &text),
            ProseCommand::DeleteText { block_id, range } => {
                self.delete_text(&tree_id(block_id), range)
            }
            ProseCommand::DeleteRange { anchor, focus } => {
                self.delete_range((tree_id(anchor.0), anchor.1), (tree_id(focus.0), focus.1))
            }
            ProseCommand::SplitBlock { block_id, pos } => self.split_block(&tree_id(block_id), pos),
            ProseCommand::ApplyMark {
                block_id,
                range,
                key,
                value,
            } => self.apply_mark(&tree_id(block_id), range, &key, value.as_deref()),
            ProseCommand::RemoveMark {
                block_id,
                range,
                key,
            } => self.remove_mark(&tree_id(block_id), range, &key),
            ProseCommand::ClearInlineMarks { block_id, range } => {
                self.clear_inline_marks(&tree_id(block_id), range)
            }
            ProseCommand::InsertBlock {
                parent_id,
                after_id,
                init,
                style_id,
            } => self.insert_block(
                parent_id.map(tree_id),
                after_id.map(tree_id),
                &init,
                style_id.as_ref(),
            ),
            ProseCommand::DeleteBlock { block_id } => self.delete_block(&tree_id(block_id)),
            ProseCommand::MoveBlock {
                block_id,
                new_parent,
                after_id,
            } => self.move_block(
                &tree_id(block_id),
                new_parent.map(tree_id),
                after_id.map(tree_id),
            ),
            ProseCommand::SetBlockStyle { block_id, style_id } => {
                self.set_block_style(&tree_id(block_id), &style_id)
            }
            ProseCommand::SetHeadingLevel { block_id, level } => {
                self.set_heading_level(&tree_id(block_id), level)
            }
            ProseCommand::SetListStyle { block_id, style } => {
                self.set_list_style(&tree_id(block_id), style)
            }
            ProseCommand::IndentList { block_id } => self.indent_list(&tree_id(block_id)),
            ProseCommand::UnindentList { block_id } => self.unindent_list(&tree_id(block_id)),
            ProseCommand::SetCodeLang { block_id, lang } => {
                self.set_code_lang(&tree_id(block_id), &lang)
            }
            ProseCommand::SetParagraphStyle { block_id, style } => {
                self.set_paragraph_style(&tree_id(block_id), &style)
            }
            ProseCommand::InsertTable {
                parent_id,
                after_id,
                rows,
                cols,
            } => self.insert_table(parent_id.map(tree_id), after_id.map(tree_id), rows, cols),
            ProseCommand::InsertTableRow {
                table_id,
                after_row,
            } => self.insert_table_row(&tree_id(table_id), after_row.map(tree_id)),
            ProseCommand::DeleteTableRow { row_id } => self.delete_table_row(&tree_id(row_id)),
            ProseCommand::InsertTableCol {
                table_id,
                at_col,
                width_pt,
            } => self.insert_table_col(&tree_id(table_id), at_col, width_pt),
            ProseCommand::DeleteTableCol {
                table_id,
                col_index,
            } => self.delete_table_col(&tree_id(table_id), col_index),
            ProseCommand::ResizeTableCol {
                table_id,
                col_index,
                width_pt,
            } => self.resize_table_col(&tree_id(table_id), col_index, width_pt),
            ProseCommand::MergeTableCells {
                table_id,
                start,
                end,
            } => self.merge_table_cells(&tree_id(table_id), start, end),
            ProseCommand::UnmergeTableCell { cell_id } => {
                self.unmerge_table_cell(&tree_id(cell_id))
            }
            ProseCommand::SetCellStyle {
                cell_id,
                bg,
                valign,
                border,
                padding_pt,
            } => self.set_cell_style(&tree_id(cell_id), bg, valign, border, padding_pt),
            ProseCommand::StepTableCell { cell_id, direction } => {
                self.step_table_cell(&tree_id(cell_id), direction)
            }
            ProseCommand::InsertToc {
                parent_id,
                after_id,
                max_level,
                numbered,
            } => self.insert_toc(
                parent_id.map(tree_id),
                after_id.map(tree_id),
                max_level,
                numbered,
            ),
            ProseCommand::UpdateTocConfig {
                block_id,
                max_level,
                numbered,
            } => self.update_toc_config(&tree_id(block_id), max_level, numbered),
            ProseCommand::CountWords => self.count_words(),
            ProseCommand::ReadMarks { anchor, focus } => self.read_marks(anchor, focus),
            ProseCommand::CopyRange { anchor, focus } => self.copy_range(anchor, focus),
            ProseCommand::Paste {
                at,
                text,
                html,
                payload,
            } => self.paste(
                (tree_id(at.0), at.1),
                &text,
                html.as_deref(),
                payload.as_deref(),
            ),
            ProseCommand::FormatSelection {
                anchor,
                focus,
                key,
                action,
            } => self.format_selection(anchor, focus, &key, action),
            ProseCommand::SetHeaderEnabled { enabled } => {
                self.set_side_enabled(bands::Side::Header, enabled)
            }
            ProseCommand::SetFooterEnabled { enabled } => {
                self.set_side_enabled(bands::Side::Footer, enabled)
            }
            ProseCommand::SetDifferentFirstPage { enabled } => {
                self.set_different_first_page(enabled)
            }
            ProseCommand::InsertHeaderField { field, target, pos } => {
                self.insert_field(bands::Side::Header, field, target, pos)
            }
            ProseCommand::InsertFooterField { field, target, pos } => {
                self.insert_field(bands::Side::Footer, field, target, pos)
            }
            ProseCommand::SetBandAlign { block_id, align } => {
                self.set_band_align(&tree_id(block_id), align)
            }
            ProseCommand::StampModified { now_ms } => self.stamp_modified(now_ms),
            ProseCommand::FindStart { query } => self.find_start(query),
            ProseCommand::FindNext => self.find_step(StepDirection::Forward),
            ProseCommand::FindPrev => self.find_step(StepDirection::Backward),
            ProseCommand::FindClose => self.find_close(),
            ProseCommand::ReplaceOne { replacement } => self.replace_one(&replacement),
            ProseCommand::ReplaceAll { replacement } => self.replace_all(&replacement),
            ProseCommand::InsertFootnote {
                block_id,
                pos,
                note_text,
            } => self.insert_footnote(&tree_id(block_id), pos, &note_text),
            ProseCommand::EditFootnote {
                footnote_id,
                new_text,
            } => self.edit_footnote(&footnote_id, &new_text),
            ProseCommand::DeleteFootnote { footnote_id } => self.delete_footnote(&footnote_id),
            ProseCommand::AddComment {
                block_id,
                range,
                text,
                now_ms,
            } => self.add_comment(&tree_id(block_id), range, &text, now_ms),
            ProseCommand::ReplyToComment {
                comment_id,
                text,
                now_ms,
            } => self.reply_to_comment(&comment_id, &text, now_ms),
            ProseCommand::ResolveComment { comment_id } => self.set_resolved(&comment_id, true),
            ProseCommand::ReopenComment { comment_id } => self.set_resolved(&comment_id, false),
            ProseCommand::DeleteComment { comment_id } => self.delete_comment(&comment_id),
            ProseCommand::DeleteReply {
                comment_id,
                reply_id,
            } => self.delete_reply(&comment_id, &reply_id),
            ProseCommand::EditCommentReply {
                comment_id,
                reply_id,
                new_text,
            } => self.edit_reply(&comment_id, &reply_id, &new_text),
            ProseCommand::InsertCrossRef {
                block_id,
                pos,
                target_block_id,
                kind,
            } => self.insert_cross_ref(&tree_id(block_id), pos, &tree_id(target_block_id), &kind),
            ProseCommand::FollowCrossRef { block_id, pos } => {
                self.follow_cross_ref(&tree_id(block_id), pos)
            }
            ProseCommand::EnableTrackChanges { enabled } => self.enable_track_changes(enabled),
            ProseCommand::AcceptSuggestion { suggestion_id } => {
                self.resolve_suggestion(&suggestion_id, suggestions::Verdict::Accept)
            }
            ProseCommand::RejectSuggestion { suggestion_id } => {
                self.resolve_suggestion(&suggestion_id, suggestions::Verdict::Reject)
            }
            ProseCommand::AcceptAllSuggestions => self.resolve_all(suggestions::Verdict::Accept),
            ProseCommand::RejectAllSuggestions => self.resolve_all(suggestions::Verdict::Reject),
            ProseCommand::UpdateCursor { pos } => self.update_cursor(&pos),
            ProseCommand::Undo => self.apply_history(HistoryStep::Undo),
            ProseCommand::Redo => self.apply_history(HistoryStep::Redo),
            // The native header needs the identity the registry holds. So the
            // Logic dispatcher serves that export before the document is
            // reached (`PROSE-IO-028`). A text format is the document's own.
            ProseCommand::ExportDocument { format } => self.export_document(format),
            ProseCommand::ImportDocument { format, data } => self.import_document(format, &data),
        }
    }

    // -- remote and persistence --------------------------------------------

    /// Apply a remote delta, routed here by document id.
    ///
    /// The confirmed-change event carries the payload that arrived, never a
    /// re-export of the current state (`PROSE-048`). A re-export after a local
    /// edit is a different set of operations from the one that came in. Nothing
    /// is re-broadcast (`SYNC-101`): the method answers display events alone.
    pub fn apply_remote(&mut self, payload: &ambre_core::DeltaBytes) -> Vec<ProseEvent> {
        self.import_remote(ambre_core::LoroPayloadRef::Delta(payload))
    }

    /// Apply a snapshot whose contents are trusted the way a remote delta is
    /// applied: [`apply_remote`](Self::apply_remote) and this share one path.
    pub fn apply_remote_snapshot(
        &mut self,
        payload: &ambre_core::SnapshotBytes,
    ) -> Vec<ProseEvent> {
        self.import_remote(ambre_core::LoroPayloadRef::Snapshot(payload))
    }

    /// The path [`apply_remote`](Self::apply_remote) and
    /// [`apply_remote_snapshot`](Self::apply_remote_snapshot) share: the CRDT
    /// engine imports a delta and a snapshot alike.
    fn import_remote(&mut self, payload: ambre_core::LoroPayloadRef<'_>) -> Vec<ProseEvent> {
        let known = self.doc.oplog_vv();
        if let Err(e) = self.doc.import(payload.as_bytes()) {
            tracing::warn!("Prose: a remote change could not be applied: {e:?}");
            return vec![Self::err_remote()];
        }
        // The relay echoes this peer's own changes back. An import that brought
        // no operation changed nothing, so it answers nothing: no repaint, no
        // layout, no save.
        if self.doc.oplog_vv() == known {
            return Vec::new();
        }
        self.absorb_imported();
        let mut events = vec![ProseEvent::DocUpdated {
            payload: payload.to_payload(),
        }];
        events.extend(self.validate_tables());
        events.extend(self.regrow_root());
        // The repair is a write of this peer's own, so it needs a commit. The
        // next local command flushes it to the other peers.
        self.doc.commit();
        events.extend(self.text_changed(catchup::ChangeSource::Import));
        events
    }

    /// Apply a batch of remote deltas -- a relay's history replay on open -- as
    /// one import sweep.
    ///
    /// It mirrors [`apply_remote`](Self::apply_remote) and answers one confirmed
    /// change per delta that landed, so the view repaints once. A delta the
    /// engine refuses is skipped and the batch reports the failure rather than
    /// answering success.
    pub fn apply_remote_batch(&mut self, payloads: &[ambre_core::DeltaBytes]) -> Vec<ProseEvent> {
        if payloads.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut any_failed = false;
        for payload in payloads {
            match self.doc.import(payload.as_bytes()) {
                Ok(_) => events.push(ProseEvent::DocUpdated {
                    payload: ambre_core::LoroPayload::Delta(payload.clone()),
                }),
                Err(e) => {
                    tracing::warn!("Prose: a remote change in a batch failed: {e:?}");
                    any_failed = true;
                }
            }
        }
        if events.is_empty() {
            return vec![Self::err_remote()];
        }
        self.absorb_imported();
        events.extend(self.validate_tables());
        events.extend(self.regrow_root());
        self.doc.commit();
        events.extend(self.text_changed(catchup::ChangeSource::Import));
        if any_failed {
            events.push(Self::err_remote());
        }
        events
    }

    /// The whole document as the CRDT's own bytes, and nothing else: no layout,
    /// no cache, no session state (`PROSE-053`).
    ///
    /// It reads through `&self` and advances no cursor (`PROSE-054`).
    #[must_use]
    pub fn export_snapshot(&self) -> ambre_core::SnapshotBytes {
        ambre_core::SnapshotBytes::new(self.doc.export(ExportMode::Snapshot).unwrap_or_default())
    }

    /// The CRDT document, read-only. A snapshot check compares its history
    /// with a peer's snapshot and never writes (`AUTHOR-018`).
    #[must_use]
    pub fn crdt(&self) -> &loro::LoroDoc {
        &self.doc
    }

    /// Take in a whole document (`PROSE-055`).
    ///
    /// The schema gate runs first, on a scratch document, so a refused read
    /// leaves this one exactly as it was (`SCHEMA-012`, `PROSE-097`). Importing
    /// produces no delta (`PROSE-056`): the method answers display events, and
    /// the export cursor absorbs what arrived.
    ///
    /// # Errors
    /// Every failure of the schema gate: a document a newer client wrote, a
    /// migration the registry cannot run, or bytes the CRDT engine refuses.
    pub fn import_snapshot(
        &mut self,
        payload: &ambre_core::SnapshotBytes,
    ) -> Result<Vec<ProseEvent>, SchemaError> {
        let payload = schema::gate(payload.as_bytes())?;
        // A snapshot replaces the document whole (`PROSE-055`), so this peer's
        // own scaffolding must not survive it. The paragraph it grew on opening
        // is dropped when it is still empty and still the only root: what
        // arrives brings its own. Every other block stays, because a snapshot
        // merges rather than overwrites and losing a local edit is the worse
        // answer. A paragraph the payload carries is not this peer's: it is
        // the shared scaffolding, the author's own (`PROSE-129`).
        let carried = LoroDoc::decode_import_blob_meta(&payload, false)
            .map(|meta| meta.partial_end_vv)
            .unwrap_or_default();
        let tree = doc::blocks(&self.doc);
        let roots = doc::roots(&tree);
        if let [root] = roots[..]
            && doc::grown_empty_roots(&self.doc, &tree) == roots
            && !carried.includes_id(root.id())
            && let Err(e) = tree.delete(root)
        {
            tracing::warn!("Prose: the scaffolding paragraph could not be dropped: {e:?}");
        }
        self.doc.commit_with(
            loro::CommitOptions::new().origin(ambre_core::crdt::CommitOrigin::Load.as_str()),
        );
        if let Err(e) = self.doc.import(&payload) {
            tracing::warn!("Prose: the snapshot import failed: {e:?}");
            return Err(SchemaError::PayloadRejected);
        }
        // The operations that arrived are not ours to broadcast: the next local
        // edit would otherwise send the whole imported history back out
        // (`PROSE-056`).
        self.absorb_imported();
        // A whole document arrived: the layout is dropped and the next call
        // to [`paginate`](Self::paginate) recomputes it whole (`PROSE-082`).
        self.paginator = None;
        // An imported document carries containers this build has not configured
        // yet: the mark registry and the tree's ordering are settings, not
        // content.
        Self::prepare(&self.doc);
        let mut events = vec![ProseEvent::DocUpdated {
            payload: ambre_core::LoroPayload::Snapshot(ambre_core::SnapshotBytes::new(
                payload.into_owned(),
            )),
        }];
        events.extend(self.validate_tables());
        events.extend(self.regrow_root());
        self.doc.commit();
        // A whole document is no run of edits: the layout it brings is the
        // answer, and the moves it recorded go nowhere.
        drop(self.touched.take_moves());
        Ok(events)
    }

    /// The schema version this document carries (`SCHEMA-004`), for a report
    /// rather than for a decision. Every path that decides goes through the
    /// gate, which refuses an unreadable register instead.
    #[must_use]
    pub fn schema_version(&self) -> ambre_core::schema::SchemaVersion {
        schema::read_version(&self.doc).unwrap_or(ambre_core::schema::SCHEMA_VERSION_ABSENT)
    }

    // -- pictures ----------------------------------------------------------

    /// A decrypted picture arrived on its own Logic route: cache it under the
    /// hash it came with (`PROSE-059`), and name what showed it (`PROSE-060`).
    ///
    /// It writes nothing to the document and produces no delta (`PROSE-061`):
    /// the cache is session state (`PROSE-044`).
    pub fn on_media_downloaded(
        &mut self,
        content_hash: ambre_core::ContentHash,
        data: Vec<u8>,
    ) -> Vec<ProseEvent> {
        let blocks = self.blocks_showing(&content_hash);
        self.media_cache.insert(content_hash.clone(), data);
        vec![ProseEvent::MediaArrived {
            content_hash,
            blocks,
        }]
    }

    /// The decrypted bytes of a cached picture, for the view that paints it.
    #[must_use]
    pub fn cached_media(&self, content_hash: &ambre_core::ContentHash) -> Option<&[u8]> {
        self.media_cache.get(content_hash).map(Vec::as_slice)
    }

    /// Mint an image block addressing `content_hash`, for a test that binds the
    /// answer rather than the gesture.
    ///
    /// Test-only: `InsertImage` lands with the `Images` Story
    /// (`ambre-roadmap.md` E5).
    #[cfg(test)]
    fn image_for_test(
        &mut self,
        content_hash: &ambre_core::ContentHash,
    ) -> Option<crate::events::BlockId> {
        let id = doc::create_block(&self.doc, None, crate::events::BlockType::Image)?;
        let tree = doc::blocks(&self.doc);
        let node = doc::node_of(&tree, &id)?;
        let attrs = doc::attrs_at(&node)?;
        attrs
            .insert(doc::AttrKey::ContentHash, content_hash.as_str())
            .ok()?;
        self.doc.commit();
        Some(crate::events::block_id(id))
    }

    /// The live image blocks addressing one content hash. Sorted, so two peers
    /// name them in one order.
    fn blocks_showing(
        &self,
        content_hash: &ambre_core::ContentHash,
    ) -> Vec<crate::events::BlockId> {
        let tree = doc::blocks(&self.doc);
        let mut ids: Vec<TreeID> = tree
            .nodes()
            .into_iter()
            .filter(|id| doc::block_type_of(&tree, id) == Some(crate::events::BlockType::Image))
            .filter(|id| self.image_hash(id).as_ref() == Some(content_hash))
            .collect();
        ids.sort();
        ids.into_iter().map(block_id).collect()
    }

    // -- undo and redo -----------------------------------------------------

    /// One undo or one redo step. It commits the inverse operations, and the
    /// delta they produce is broadcast like any other mutation.
    fn apply_history(&mut self, step: HistoryStep) -> ProseOutcome {
        let before = self.doc.state_frontiers();
        let changed = match step {
            HistoryStep::Undo => self.undo.undo(),
            HistoryStep::Redo => self.undo.redo(),
        }
        .unwrap_or(false);
        if !changed {
            return ProseOutcome::nothing();
        }
        let mut events = self.regrow_root();
        // `regrow_root` leaves its writes to the caller, and the export cursor
        // is about to move past them: an uncommitted repair would never reach a
        // peer.
        self.doc.commit();
        if let Some((block, offset)) = history::edit_point(&self.doc, &before) {
            events.push(ProseEvent::CaretPlaced { block, offset });
        }
        let update = self.flush_delta();
        if let Some(delta) = update.clone() {
            events.insert(
                0,
                ProseEvent::DocUpdated {
                    payload: ambre_core::LoroPayload::Delta(delta),
                },
            );
        }
        ProseOutcome {
            display: events,
            update,
            ephemeral: None,
        }
    }

    // -- the root block the tree is never without --------------------------

    /// Keep the tree to exactly the blocks the document asked for, and never
    /// to none (`PROSE-027`, `PROSE-094`).
    ///
    /// Called after every write and after every import, because the write that
    /// empties the tree may be a collaborator's.
    ///
    /// Loro refuses to raise a tombstone, so a peer that empties the tree grows
    /// a paragraph of its own. Two peers that did so concurrently, and two that
    /// each opened the document with their own scaffolding, therefore merge to
    /// several empty paragraphs. This drops the surplus and keeps the one whose
    /// `TreeID` sorts first. Both halves read only the document, so every peer
    /// reaches the same tree.
    ///
    /// **What it does not repair.** A peer that joins a document whose own grown
    /// paragraph already holds text keeps its empty one. The two are no longer
    /// both empty, and dropping a paragraph a user may have meant to leave
    /// blank is the worse answer. Every peer still reads the same tree.
    ///
    /// The caller commits, so the repair rides the same delta as the write that
    /// caused it.
    fn regrow_root(&mut self) -> Vec<ProseEvent> {
        let tree = doc::blocks(&self.doc);
        // The first grown paragraph stays, whatever else the tree holds: it is
        // the block a document that has nothing else is never without
        // (`PROSE-027`). Only the peers that grew one too many lose theirs.
        for id in doc::grown_empty_roots(&self.doc, &tree).into_iter().skip(1) {
            if let Err(e) = tree.delete(id) {
                tracing::warn!("Prose: the tree refused to drop a grown paragraph: {e:?}");
            }
        }
        if !doc::roots(&tree).is_empty() {
            return Vec::new();
        }
        if doc::grow_root(&self.doc).is_none() {
            return vec![ProseEvent::Error(ProseError::new(
                "keeping a block",
                ProseRefusal::WriteFailed("the document could not keep a block".to_owned()),
            ))];
        }
        Vec::new()
    }

    // -- outcome plumbing --------------------------------------------------

    /// Commit the command's writes, re-grow the tree if the command emptied it,
    /// and wrap the delta into the outcome.
    ///
    /// Every mutating handler ends here, so the confirmed-change event carries
    /// the bytes that were exported and never a re-export (`PROSE-023`).
    fn commit_with(&mut self, mut display: Vec<ProseEvent>) -> ProseOutcome {
        self.doc.commit();
        // The anchor cache follows the edit in a commit of its own, out of the
        // undo stack. An undo takes the edit back, and the next commit
        // re-reads the mark (`PROSE-FEAT-076`).
        if self.refresh_anchors() {
            self.doc
                .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Comment.as_str());
            self.doc.commit();
        }
        // Typed text grows a block and never empties one, so the tree cannot
        // need a repair: the walk over the roots is skipped.
        if !self.touched.only_typed() {
            display.extend(self.regrow_root());
            self.doc.commit();
        }
        let update = self.flush_delta();
        if let Some(delta) = update.clone() {
            display.insert(
                0,
                ProseEvent::DocUpdated {
                    payload: ambre_core::LoroPayload::Delta(delta),
                },
            );
        }
        if let Some(moved) = self.text_changed(catchup::ChangeSource::Command) {
            display.insert(usize::from(update.is_some()), moved);
        }
        ProseOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// What the text did since the last answer, for the view that follows it
    /// (`PROSE-UI-028`). `None` when the change touched no text and deleted
    /// no block. `source` tells a command's change, which the view may have
    /// drawn, from an import.
    fn text_changed(&mut self, source: catchup::ChangeSource) -> Option<ProseEvent> {
        let moves = self.touched.take_moves();
        let removed = self.gone(moves.removed);
        let edits: Vec<crate::events::TextEdit> = moves
            .edits
            .into_iter()
            .map(|(block, at, delta)| crate::events::TextEdit {
                block_id: block_id(block),
                at,
                delta,
            })
            .collect();
        let (local, seq, acked, edits) = self.catchup.stamp(source, edits);
        if edits.is_empty() && removed.is_empty() {
            return None;
        }
        Some(ProseEvent::TextChanged {
            local,
            seq,
            acked,
            edits,
            removed: removed.into_iter().map(block_id).collect(),
        })
    }

    /// The blocks a change deleted, out of those it reported, with every
    /// block inside them.
    ///
    /// A tree diff reports a delete for a node an import re-creates or
    /// moves, so only a block that is gone now counts. Deleting a table hides
    /// the table alone, and its cells go with it.
    fn gone(&self, reported: Vec<TreeID>) -> Vec<TreeID> {
        let tree = doc::blocks(&self.doc);
        let mut out = Vec::new();
        let mut stack: Vec<TreeID> = reported
            .into_iter()
            .filter(|id| doc::node_of(&tree, id).is_none())
            .collect();
        while let Some(id) = stack.pop() {
            if out.contains(&id) {
                continue;
            }
            out.push(id);
            stack.extend(tree.children(id).unwrap_or_default());
        }
        out.sort_unstable();
        out
    }

    /// Export every operation past the export cursor, and advance the cursor
    /// only when the export succeeded (`PROSE-050`, `PROSE-098`).
    ///
    /// Exporting reads and never writes (`PROSE-052`). A second export from the
    /// same vector yields the same bytes (`PROSE-051`), because the cursor is a
    /// stored version vector rather than a drain. An extra call therefore
    /// cannot truncate the stream. An export that fails leaves the cursor where
    /// it is, so the next flush offers the same range again.
    fn flush_delta(&mut self) -> Option<ambre_core::DeltaBytes> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        // The registry entry alone is no edit: it waits for the first one and
        // travels with it (`AUTHOR-009`).
        if let Some(end) = self.entry_end {
            if current.get(&self.peer.0) == Some(&end.0) {
                return None;
            }
            self.entry_end = None;
        }
        match self.doc.export(ExportMode::updates(&self.last_exported_vv)) {
            Ok(delta) => {
                self.last_exported_vv = current;
                (!delta.is_empty()).then_some(ambre_core::DeltaBytes::new(delta))
            }
            Err(e) => {
                tracing::error!("Prose: the export failed, the range is kept: {e:?}");
                None
            }
        }
    }

    /// Take the export cursor past everything the document now holds, keeping
    /// this peer's own counter where it was.
    ///
    /// The imported operations are not ours to broadcast. The cursor must clear
    /// them, or the next local edit sends a peer its own work back. Our own
    /// counter is the exception. A document opens by committing its scaffolding
    /// under a fresh peer id. Swallowing those operations would make the next
    /// local delta depend on operations no other peer holds.
    fn absorb_imported(&mut self) {
        // The layout survives a delta: the document reports the blocks it
        // wrote, and the next layout re-measures those alone.
        // A collaborator can bring a mark back, so no orphan is taken on trust.
        self.orphans.clear();
        let ours = self.last_exported_vv.get(&self.peer.0).copied();
        self.last_exported_vv = self.doc.oplog_vv();
        match ours {
            Some(counter) => {
                self.last_exported_vv.insert(self.peer.0, counter);
            }
            None => {
                self.last_exported_vv.remove(&self.peer.0);
            }
        }
    }

    // -- refusals ----------------------------------------------------------

    /// The refusal a command that names no live block answers with.
    /// The document itself, for a test that reads a container no command
    /// exposes, or that plays an import by writing one directly.
    #[cfg(test)]
    pub(crate) fn document(&self) -> &loro::LoroDoc {
        &self.doc
    }

    /// Whether one of the named blocks holds a band or a note. No
    /// structural command may delete, move, or place a block beside one
    /// (`PROSE-006`, `PROSE-FEAT-305`).
    fn names_a_band(tree: &loro::LoroTree, ids: &[Option<TreeID>]) -> bool {
        ids.iter().flatten().any(|id| doc::is_aside(tree, id))
    }

    fn err_no_block(id: &TreeID) -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::NoSuchBlock(block_id(*id)))
    }

    /// The refusal a command whose offsets leave the body it addresses answers
    /// with (`PROSE-045`).
    fn err_bad_range() -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::BadRange)
    }

    /// The refusal the nesting table answers a move or an insert with
    /// (`PROSE-040`, `PROSE-095`).
    fn err_nesting() -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::NestingRefused)
    }

    /// The refusal a collaborator's change this peer could not apply answers
    /// with. It names the update rather than a command: nothing local was
    /// asked for (`PROSE-105`).
    fn err_remote() -> ProseEvent {
        ProseEvent::Error(ProseError::new(
            "a collaborator's update",
            ProseRefusal::RemoteImportFailed,
        ))
    }

    /// The refusal a write that did not land answers with.
    fn err_write(error: impl Into<doc::WriteError>) -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::from(error.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::HeadingLevel;
    use crate::events::{BlockId, BlockInit, CursorPosition};
    use ambre_bus::app::prose::DocOffset;
    use ambre_bus::app::prose::DocSpan;

    #[test]
    fn a_fresh_document_holds_one_paragraph_and_a_version() {
        let state = ProseState::new().expect("a document is built");
        assert_eq!(state.schema_version(), schema::SCHEMA_VERSION);
        assert_eq!(state.root_blocks().len(), 1);
    }

    /// The one refusal in an outcome, or the test fails on the spot.
    fn refusal(outcome: &ProseOutcome) -> &ProseError {
        let [ProseEvent::Error(error)] = outcome.display.as_slice() else {
            panic!("the outcome carried no single refusal");
        };
        error
    }

    /// A refusal names the command that drew it (`PROSE-105`). The handler
    /// states the reason; the name comes from the entry point.
    #[test]
    fn a_refusal_names_the_command_it_refused() {
        let mut state = ProseState::new().expect("a document is built");
        let absent = BlockId::new(ambre_core::PeerId(9), 9);
        let outcome = state.handle_command(ProseCommand::DeleteText {
            block_id: absent,
            range: DocSpan::from_usize_range(0..1),
        });
        let error = refusal(&outcome);
        assert_eq!(error.refused, "deleting text");
        assert_eq!(error.reason, ProseRefusal::NoSuchBlock(absent));
    }

    /// Two commands refused for one reason MUST NOT read alike (`PROSE-105`).
    /// The reason is what the author can act on least, so the name carries the
    /// message.
    #[test]
    fn two_commands_refused_for_one_reason_name_themselves_apart() {
        let mut state = ProseState::new().expect("a document is built");
        let absent = BlockId::new(ambre_core::PeerId(9), 9);
        let deleting = state.handle_command(ProseCommand::DeleteText {
            block_id: absent,
            range: DocSpan::from_usize_range(0..1),
        });
        let splitting = state.handle_command(ProseCommand::SplitBlock {
            block_id: absent,
            pos: DocOffset::ZERO,
        });
        assert_eq!(refusal(&deleting).reason, refusal(&splitting).reason);
        assert_ne!(refusal(&deleting).refused, refusal(&splitting).refused);
    }

    /// A mark value the key's grammar refuses says so (`PROSE-106`). It used
    /// to travel as a range failure, which named a part of the command that
    /// was correct.
    #[test]
    fn a_mark_value_the_grammar_refuses_is_not_a_range_failure() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "text".to_owned(),
        });
        let outcome = state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: DocSpan::from_usize_range(0..4),
            key: "fs".into(),
            value: Some("huge".to_owned()),
        });
        let error = refusal(&outcome);
        assert_eq!(error.refused, "applying the style");
        assert_eq!(
            error.reason,
            ProseRefusal::BadMarkValue {
                key: "fs".into(),
                value: "huge".to_owned(),
            }
        );
    }

    #[test]
    fn a_command_that_changes_nothing_exports_no_delta() {
        let mut state = ProseState::new().expect("a document is built");
        let outcome = state.handle_command(ProseCommand::Noop);
        assert!(outcome.update.is_none());
        assert!(outcome.display.is_empty());
    }

    #[test]
    fn a_second_export_from_the_same_vector_yields_the_same_bytes() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        let first = state
            .handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
                text: "hello".to_owned(),
            })
            .update
            .expect("the insertion exported a delta");
        // The cursor advanced, so the same range is no longer pending. A peer
        // that replays `first` reaches the same document, which is what the
        // rule protects: the extra call did not truncate the stream.
        let second = state.handle_command(ProseCommand::Noop);
        assert!(second.update.is_none());
        let mut replay = ProseState::new().expect("a document is built");
        replay.apply_remote(&first);
        assert_eq!(replay.text_of(&block).as_deref(), Some("hello"));
    }

    /// A document written the way a build before `PROSE-129` wrote it: the
    /// scaffolding under the author's own identity, so no reader shares it.
    fn a_document_with_its_own_scaffolding(text: &str) -> Vec<u8> {
        let doc = LoroDoc::new();
        ProseState::prepare(&doc);
        let root = doc::seed(&doc).expect("the scaffolding is written");
        schema::stamp(&doc);
        doc.commit();
        doc::get_block_text(&doc, &root)
            .insert(0, text)
            .expect("the text is written");
        doc.commit();
        doc.export(ExportMode::Snapshot).expect("a snapshot")
    }

    /// The reader's own scaffolding does not survive a snapshot that does
    /// not carry it (`PROSE-055`). A peer that opened a document grew one
    /// paragraph for want of any. A document an earlier build wrote brings
    /// its own, so the grown one is dropped rather than left standing above
    /// the imported text.
    #[test]
    fn an_imported_snapshot_drops_the_paragraph_the_reader_grew() {
        let snapshot = a_document_with_its_own_scaffolding("a paragraph");
        let mut reader = ProseState::new().expect("a document is built");
        let grown = reader.root_blocks()[0];
        reader
            .import_snapshot(&ambre_core::SnapshotBytes::new(snapshot.to_vec()))
            .expect("the snapshot imports");
        assert!(!reader.root_blocks().contains(&grown));
        assert_eq!(reader.root_blocks().len(), 1);
        assert_eq!(
            reader.text_of(&reader.root_blocks()[0]).as_deref(),
            Some("a paragraph")
        );
    }

    /// `PROSE-129`: a reader that has sent nothing holds exactly its
    /// author's history. The paragraph it grew is the author's own, so the
    /// import drops nothing, and a later change finds no concurrent
    /// operation to import against.
    #[test]
    fn a_reader_that_sent_nothing_holds_no_history_of_its_own() {
        let mut author = ProseState::new().expect("a document is built");
        let block = author.root_blocks()[0];
        author.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "a paragraph".to_owned(),
        });
        let mut reader = ProseState::new().expect("a document is built");
        reader
            .import_snapshot(&author.export_snapshot())
            .expect("the snapshot imports");
        assert_eq!(
            reader.doc.oplog_vv(),
            author.doc.oplog_vv(),
            "the scaffolding is shared"
        );
        assert_eq!(reader.root_blocks(), vec![block]);
        let delta = author
            .handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
                text: "Still ".to_owned(),
            })
            .update
            .expect("a delta");
        reader.apply_remote(&delta);
        assert_eq!(reader.doc.oplog_vv(), author.doc.oplog_vv());
        assert_eq!(reader.text_of(&block).as_deref(), Some("Still a paragraph"));
    }

    /// A block the reader wrote in is not scaffolding (`PROSE-104`). A
    /// snapshot merges, and dropping the root would lose an edit made while
    /// the whole document was still on its way.
    #[test]
    fn an_imported_snapshot_keeps_a_root_the_reader_wrote_in() {
        // A snapshot that does not carry the reader's root: one it carries is
        // the author's own block, and merges character by character.
        let snapshot = a_document_with_its_own_scaffolding("a paragraph");
        let mut reader = ProseState::new().expect("a document is built");
        let mine = reader.root_blocks()[0];
        reader.handle_command(ProseCommand::InsertText {
            block_id: mine,
            pos: DocOffset::ZERO,
            text: "mine".to_owned(),
        });
        reader
            .import_snapshot(&ambre_core::SnapshotBytes::new(snapshot.to_vec()))
            .expect("the snapshot imports");
        assert!(reader.root_blocks().contains(&mine));
        assert_eq!(reader.text_of(&mine).as_deref(), Some("mine"));
    }

    #[test]
    fn the_layout_is_derived_and_the_document_is_left_alone() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "a paragraph that is measured, never stored".to_owned(),
        });
        let before = state.doc.get_deep_value();
        let pages = state.paginate(&[]).page_count;
        assert!(pages >= PageCount::ONE);
        // `PROSE-033`, `PROSE-065`, `PROSE-079`: laying out reads, and the
        // document holds no page number, no break and no height afterwards.
        assert_eq!(state.doc.get_deep_value(), before);
        assert_eq!(state.paginate(&[]).page_count, pages);
    }

    #[test]
    fn a_snapshot_import_lays_the_document_out_again() {
        let mut source = ProseState::new().expect("a document is built");
        source.load_faces(&crate::layout::fonts::shipped_faces());
        let block = source.root_blocks()[0];
        source.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "a long paragraph ".repeat(400),
        });
        let snapshot = source.export_snapshot();
        let mut target = ProseState::new().expect("a document is built");
        target.load_faces(&crate::layout::fonts::shipped_faces());
        let empty = target.paginate(&[]).page_count;
        target
            .import_snapshot(&snapshot)
            .expect("the snapshot imports");
        // `PROSE-082`: the bytes named no block, so the layout is recomputed
        // whole rather than reusing what the empty document measured.
        assert!(target.paginate(&[]).page_count > empty);
        assert_eq!(
            target.paginate(&[]).page_count,
            source.paginate(&[]).page_count
        );
    }

    #[test]
    fn a_snapshot_round_trips_what_the_document_held() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "a paragraph".to_owned(),
        });
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(block),
            init: BlockInit::Heading {
                level: HeadingLevel::H2,
            },
            style_id: None,
        });
        let snapshot = state.export_snapshot();
        let mut restored = ProseState::new().expect("a document is built");
        restored
            .import_snapshot(&snapshot)
            .expect("the snapshot imports");
        assert_eq!(restored.text_of(&block).as_deref(), Some("a paragraph"));
        assert_eq!(restored.root_blocks().len(), state.root_blocks().len());
    }

    #[test]
    fn answering_with_a_snapshot_advances_no_cursor() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "x".to_owned(),
        });
        let before = state.last_exported_vv.clone();
        let _ = state.export_snapshot();
        let _ = state.export_snapshot();
        assert_eq!(state.last_exported_vv, before);
    }

    #[test]
    fn importing_a_snapshot_produces_no_delta() {
        let mut source = ProseState::new().expect("a document is built");
        let block = source.root_blocks()[0];
        source.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "x".to_owned(),
        });
        let snapshot = source.export_snapshot();
        let mut target = ProseState::new().expect("a document is built");
        let events = target.import_snapshot(&snapshot).expect("it imports");
        // The events repaint; none of them is an outcome, so nothing reaches
        // Sync from this path.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProseEvent::DocUpdated { .. }))
        );
    }

    #[test]
    fn a_picture_arriving_writes_nothing_to_the_document() {
        let mut state = ProseState::new().expect("a document is built");
        let before = state.export_snapshot();
        let events =
            state.on_media_downloaded(ambre_core::ContentHash("abc".to_owned()), vec![1, 2, 3]);
        assert_eq!(state.cached_media(&"abc".into()), Some(&[1u8, 2, 3][..]));
        assert_eq!(events.len(), 1);
        assert_eq!(state.export_snapshot().len(), before.len());
    }

    /// `PROSE-060`: the picture arriving names what showed it.
    ///
    /// The image block is built through the format, because no command inserts
    /// a picture until the images Story lands. The rule binds the answer, not
    /// the gesture.
    #[test]
    fn a_picture_arriving_names_every_block_that_shows_it() {
        let mut state = ProseState::new().expect("a document is built");
        let shown = state
            .image_for_test(&"a-content-hash".into())
            .expect("an image block");
        let other = state
            .image_for_test(&"another-hash".into())
            .expect("an image block");

        let events = state.on_media_downloaded(
            ambre_core::ContentHash("a-content-hash".to_owned()),
            vec![9],
        );
        let named = events
            .iter()
            .find_map(|e| match e {
                ProseEvent::MediaArrived { blocks, .. } => Some(blocks.clone()),
                _ => None,
            })
            .expect("the picture named its blocks");
        assert_eq!(named, vec![shown]);
        assert!(!named.contains(&other));
    }

    /// `PROSE-044`: the picture cache and the local history are session state.
    ///
    /// Neither is a container of the document, so neither survives the bytes.
    /// The cache is read against the snapshot directly. The history is read
    /// through a restore. An undo stack that had entered the document would
    /// come back with it, and the restored state could then retract an edit it
    /// never made.
    ///
    /// The page layout has no code to test. Nothing can carry it into the
    /// document while it has none, and the pagination Story owns that half.
    #[test]
    fn the_picture_cache_and_the_history_never_enter_the_document() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(11)).expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "same text".to_owned(),
        });
        let before = state.export_snapshot();
        state.on_media_downloaded(
            ambre_core::ContentHash("a-hash".to_owned()),
            vec![1, 2, 3, 4],
        );
        assert!(state.cached_media(&"a-hash".into()).is_some());
        // The cache is session state: the bytes the document exports are the
        // ones it exported before the picture arrived.
        assert_eq!(state.export_snapshot(), before);

        // The history is session state too. A restored document carries the
        // text and no stack, so an undo on it retracts nothing.
        let mut restored =
            ProseState::with_peer(ambre_core::PeerId(12)).expect("a document is built");
        restored.import_snapshot(&before).expect("it imports");
        assert_eq!(restored.text_of(&block).as_deref(), Some("same text"));
        let outcome = restored.handle_command(ProseCommand::Undo);
        assert!(
            outcome.update.is_none(),
            "a restored stack retracts nothing"
        );
        assert_eq!(restored.text_of(&block).as_deref(), Some("same text"));
        assert!(restored.cached_media(&"a-hash".into()).is_none());
    }

    /// The history state each command reports, if any.
    fn history_of(outcome: &ProseOutcome) -> Vec<(bool, bool)> {
        outcome
            .display
            .iter()
            .filter_map(|event| match event {
                ProseEvent::UndoState { can_undo, can_redo } => Some((*can_undo, *can_redo)),
                _ => None,
            })
            .collect()
    }

    /// `UNDO-006`, `UNDO-009`: a command that writes reports the history state
    /// once, read from the stack. A command that writes nothing reports none,
    /// so the menu is not refreshed for it.
    #[test]
    fn a_write_reports_the_history_and_a_no_op_reports_none() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(13)).expect("a document is built");
        let block = state.root_blocks()[0];
        let insert = ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "text".to_owned(),
        };
        assert_eq!(history_of(&state.handle_command(insert)), [(true, false)]);
        assert_eq!(
            history_of(&state.handle_command(ProseCommand::Undo)),
            [(false, true)]
        );
        assert_eq!(
            history_of(&state.handle_command(ProseCommand::Redo)),
            [(true, false)]
        );

        // Nothing to redo, a move of the caret, and an empty deletion.
        for no_op in [
            ProseCommand::Redo,
            ProseCommand::UpdateCursor {
                pos: CursorPosition {
                    block_id: block,
                    offset: DocOffset::new(1),
                    anchor: None,
                },
            },
            ProseCommand::DeleteText {
                block_id: block,
                range: DocSpan::from_usize_range(0..0),
            },
        ] {
            let outcome = state.handle_command(no_op);
            assert!(outcome.update.is_none());
            assert_eq!(history_of(&outcome), []);
        }
    }

    /// `UNDO-003`, `UNDO-008`: an undo with nothing to revert writes nothing
    /// and emits no delta. This holds on a fresh page and on a page undone to
    /// its start.
    #[test]
    fn an_undo_with_nothing_to_revert_writes_nothing() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(13)).expect("a document is built");
        let block = state.root_blocks()[0];
        let undo_on_nothing = |state: &mut ProseState| {
            let before = state.document().oplog_vv();
            assert!(state.handle_command(ProseCommand::Undo).update.is_none());
            assert_eq!(state.document().oplog_vv(), before, "the undo wrote");
        };
        undo_on_nothing(&mut state);
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "text".to_owned(),
        });
        assert!(state.handle_command(ProseCommand::Undo).update.is_some());
        undo_on_nothing(&mut state);
    }

    /// `UNDO-007`: an undo reverts this peer's own text and leaves the text a
    /// remote peer wrote in place.
    #[test]
    fn an_undo_leaves_a_remote_edit_alone() {
        let mut alice = ProseState::with_peer(ambre_core::PeerId(13)).expect("a document is built");
        let mut bob = ProseState::with_peer(ambre_core::PeerId(14)).expect("a document is built");
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        let block = alice.root_blocks()[0];
        let write = |state: &mut ProseState, text: &str| {
            state
                .handle_command(ProseCommand::InsertText {
                    block_id: block,
                    pos: DocOffset::ZERO,
                    text: text.to_owned(),
                })
                .update
                .expect("a delta")
        };
        write(&mut alice, "mine ");
        let theirs = write(&mut bob, "theirs ");
        alice.apply_remote(&theirs);
        assert!(alice.handle_command(ProseCommand::Undo).update.is_some());
        assert_eq!(alice.text_of(&block).as_deref(), Some("theirs "));
    }

    #[test]
    fn a_document_a_newer_client_wrote_leaves_this_one_untouched() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "mine".to_owned(),
        });
        let doc = LoroDoc::new();
        doc.get_map(ambre_core::crdt::FoundationRoot::Meta.as_str())
            .insert(
                ambre_core::schema::MetaKey::SchemaVersion.as_str(),
                i64::from(schema::SCHEMA_VERSION.next().get()),
            )
            .expect("a newer register");
        doc.commit();
        let newer = doc.export(ExportMode::Snapshot).expect("snapshot");
        assert!(
            state
                .import_snapshot(&ambre_core::SnapshotBytes::new(newer.to_vec()))
                .is_err()
        );
        assert_eq!(state.text_of(&block).as_deref(), Some("mine"));
    }

    /// An import used to leave the export cursor behind the operations it
    /// brought in. The next local edit then broadcast the whole imported
    /// history back out (`PROSE-056`).
    #[test]
    fn the_first_edit_after_an_import_exports_only_what_this_peer_wrote() {
        let mut origin = ProseState::new().expect("a document is built");
        let block = origin.root_blocks()[0];
        origin.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "an imported paragraph".to_owned(),
        });
        let snapshot = origin.export_snapshot();

        let mut peer = ProseState::new().expect("a document is built");
        peer.import_snapshot(&snapshot)
            .expect("the snapshot imports");
        let delta = peer
            .handle_command(ProseCommand::InsertText {
                block_id: block,
                pos: DocOffset::ZERO,
                text: "mine".to_owned(),
            })
            .update
            .expect("a delta");

        // A third peer that never saw the snapshot is the measure: a delta
        // carrying the imported history would hand it the origin's paragraph.
        let mut stranger = ProseState::new().expect("a document is built");
        stranger.apply_remote(&delta);
        let held: String = stranger
            .root_blocks()
            .iter()
            .filter_map(|id| stranger.text_of(id))
            .collect();
        assert!(
            !held.contains("an imported paragraph"),
            "the delta replayed the imported history: {held}"
        );
    }
}
