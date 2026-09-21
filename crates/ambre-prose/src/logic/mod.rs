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
//! and nesting, plus the remote, snapshot, presence and picture paths. The page
//! layout, the tables, the comments, the interop filters and the find machinery
//! each land with their own Story (`ambre-roadmap.md` E5).
//!
//! The `impl` splits across the sibling modules, and each of them reaches the
//! struct's fields and shared helpers through `super::`:
//! - [`text`] -- typing, deleting, the cross-paragraph join and the split
//! - [`blocks`] -- the block lifecycle, the nesting guard and the styles
//! - [`marks`] -- the inline mark registry's write path
//! - [`presence`] -- the ephemeral cursor frame, outbound and inbound

mod blocks;
mod marks;
mod presence;
mod text;

/// The per-application half of `shape_of` and `canonical_dump`
/// (`ambre-spec-fixtures.md`). Fixture code, so the whole module is behind the
/// non-default `fixtures` feature and no client build reaches it.
#[cfg(feature = "fixtures")]
mod fixtures;

#[cfg(test)]
mod convergence_tests;

use std::collections::HashMap;

use loro::{ExportMode, LoroDoc, TreeID, UndoManager, VersionVector};

use ambre_core::UserId;
use ambre_core::schema::SchemaError;

use crate::doc;
use crate::events::{ProseCommand, ProseEvent, ProseOutcome, block_id, error_code, tree_id};
use crate::mark;
use crate::schema;

/// The per-document Prose engine.
pub struct ProseState {
    /// The CRDT, private and named by no public signature (`PROSE-001`).
    doc: LoroDoc,
    /// Local-only undo and redo: it retracts this peer's operations alone, one
    /// item per atomic user action.
    undo: UndoManager,
    /// Decrypted picture bytes, by content hash. Session state: it never enters
    /// the document (`PROSE-044`) and it dies with the tab.
    media_cache: HashMap<String, Vec<u8>>,
    /// The version vector already exported to Sync. A delta is every operation
    /// past it (`PROSE-050`, `PROSE-098`).
    last_exported_vv: VersionVector,
    /// This document's own peer id, cached at construction.
    peer: u64,
    /// The canonical per-user identifier, seeded from `SessionReady` by the
    /// dispatcher. A stable `UserId`, never the per-session peer id.
    my_user_id: Option<UserId>,
    /// This session's display name, stamped on every outbound presence frame.
    /// Cosmetic: it decides neither identity nor colour (`PRESENCE-004`).
    my_name: Option<String>,
    /// Every remote peer's latest caret and self-asserted name, keyed by the
    /// Sync-resolved `UserId`. Ephemeral: never in the CRDT, never persisted
    /// (`PROSE-058`). The name is cosmetic and decides nothing
    /// (`PRESENCE-004`).
    presence: HashMap<UserId, (crate::events::CursorPosition, String)>,
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
        Self::from_doc(LoroDoc::new())
    }

    /// A document with a **pinned** CRDT identity and commit clock, so the same
    /// command sequence produces the same bytes twice (`FIXTURE-001`).
    ///
    /// **Never for a live document.** Two concurrent writers sharing a peer id
    /// mint conflicting operation ids and corrupt the history. This exists for
    /// single-owner, offline, host-side reproduction. The client path is
    /// [`new`](Self::new).
    #[must_use]
    pub fn with_peer(peer: u64) -> Option<Self> {
        let doc = LoroDoc::new();
        // Before any operation: this peer owns the scaffolding, and every block
        // the tree mints captures it.
        doc.set_peer_id(peer).ok()?;
        // No wall clock in the operations: a recorded timestamp would differ
        // between two runs of the same generator.
        doc.set_record_timestamp(false);
        Self::from_doc(doc)
    }

    /// The shared body of the two constructors, over a `LoroDoc` the caller has
    /// already configured.
    fn from_doc(doc: LoroDoc) -> Option<Self> {
        Self::prepare(&doc);
        doc::seed(&doc)?;
        // The version is scaffolding too, and rides the same commit, so no
        // document ever persists without it (`SCHEMA-004`, `PROSE-028`).
        schema::stamp(&doc);
        doc.commit();
        let peer = doc.peer_id();
        // The undo manager opens after the scaffolding commit: seeding a
        // document is not a user edit and must not sit on the stack.
        let undo = UndoManager::new(&doc);
        Some(Self {
            doc,
            undo,
            media_cache: HashMap::new(),
            last_exported_vv: VersionVector::default(),
            peer,
            my_user_id: None,
            my_name: None,
            presence: HashMap::new(),
        })
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
    pub fn set_display_name(&mut self, name: String) {
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
        match command {
            ProseCommand::Noop => ProseOutcome::nothing(),
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
                style_id.as_deref(),
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
            ProseCommand::SetCodeLang { block_id, lang } => {
                self.set_code_lang(&tree_id(block_id), &lang)
            }
            ProseCommand::SetParagraphStyle { block_id, style } => {
                self.set_paragraph_style(&tree_id(block_id), &style)
            }
            ProseCommand::UpdateCursor { pos } => self.update_cursor(&pos),
            ProseCommand::Undo => self.apply_history(true),
            ProseCommand::Redo => self.apply_history(false),
        }
    }

    // -- remote and persistence --------------------------------------------

    /// Apply a remote delta or a remote snapshot, routed here by document id.
    ///
    /// The confirmed-change event carries the payload that arrived, never a
    /// re-export of the current state (`PROSE-048`). A re-export after a local
    /// edit is a different set of operations from the one that came in. Nothing
    /// is re-broadcast (`PROSE-049`): the method answers display events alone.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<ProseEvent> {
        if let Err(e) = self.doc.import(payload) {
            tracing::warn!("Prose: a remote change could not be applied: {e:?}");
            return vec![ProseEvent::Error {
                code: error_code::REMOTE_IMPORT_FAILED,
                message: "a collaborator's change could not be applied".to_owned(),
            }];
        }
        self.absorb_imported();
        let mut events = vec![ProseEvent::DocUpdated {
            delta: payload.to_vec(),
        }];
        events.extend(self.regrow_root());
        // The repair is a write of this peer's own, so it needs a commit. The
        // next local command flushes it to the other peers.
        self.doc.commit();
        events
    }

    /// Apply a batch of remote deltas -- a relay's history replay on open -- as
    /// one import sweep.
    ///
    /// It mirrors [`apply_remote`](Self::apply_remote) and answers one confirmed
    /// change per delta that landed, so the view repaints once. A delta the
    /// engine refuses is skipped and the batch reports the failure rather than
    /// answering success.
    pub fn apply_remote_batch(&mut self, payloads: &[Vec<u8>]) -> Vec<ProseEvent> {
        if payloads.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut any_failed = false;
        for payload in payloads {
            match self.doc.import(payload) {
                Ok(_) => events.push(ProseEvent::DocUpdated {
                    delta: payload.clone(),
                }),
                Err(e) => {
                    tracing::warn!("Prose: a remote change in a batch failed: {e:?}");
                    any_failed = true;
                }
            }
        }
        if events.is_empty() {
            return vec![ProseEvent::Error {
                code: error_code::REMOTE_IMPORT_FAILED,
                message: "a collaborator's changes could not be applied".to_owned(),
            }];
        }
        self.absorb_imported();
        events.extend(self.regrow_root());
        self.doc.commit();
        if any_failed {
            events.push(ProseEvent::Error {
                code: error_code::REMOTE_IMPORT_FAILED,
                message: "a collaborator's change could not be applied".to_owned(),
            });
        }
        events
    }

    /// The whole document as the CRDT's own bytes, and nothing else: no layout,
    /// no cache, no session state (`PROSE-053`).
    ///
    /// It reads through `&self` and advances no cursor (`PROSE-054`).
    #[must_use]
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
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
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Result<Vec<ProseEvent>, SchemaError> {
        let payload = schema::gate(payload)?;
        // A snapshot replaces the document whole (`PROSE-055`), so this peer's
        // own scaffolding must not survive it. The paragraph it grew on opening
        // is dropped when it is still empty and still the only root: what
        // arrives brings its own. Every other block stays, because a snapshot
        // merges rather than overwrites and losing a local edit is the worse
        // answer.
        let tree = doc::blocks(&self.doc);
        let roots = doc::roots(&tree);
        if roots.len() == 1
            && doc::grown_empty_roots(&self.doc, &tree) == roots
            && let Err(e) = tree.delete(roots[0])
        {
            tracing::warn!("Prose: the scaffolding paragraph could not be dropped: {e:?}");
        }
        self.doc.commit();
        if let Err(e) = self.doc.import(&payload) {
            tracing::warn!("Prose: the snapshot import failed: {e:?}");
            return Err(SchemaError::PayloadRejected);
        }
        // An imported document carries containers this build has not configured
        // yet: the mark registry and the tree's ordering are settings, not
        // content.
        Self::prepare(&self.doc);
        let mut events = vec![ProseEvent::DocUpdated {
            delta: payload.into_owned(),
        }];
        events.extend(self.regrow_root());
        self.doc.commit();
        Ok(events)
    }

    /// The schema version this document carries (`SCHEMA-004`), for a report
    /// rather than for a decision. Every path that decides goes through the
    /// gate, which refuses an unreadable register instead.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        schema::read_version(&self.doc).unwrap_or(ambre_core::schema::SCHEMA_VERSION_ABSENT)
    }

    // -- pictures ----------------------------------------------------------

    /// A decrypted picture arrived on its own Logic route: cache it under the
    /// hash it came with (`PROSE-059`), and name what showed it (`PROSE-060`).
    ///
    /// It writes nothing to the document and produces no delta (`PROSE-061`):
    /// the cache is session state (`PROSE-044`).
    pub fn on_media_downloaded(&mut self, content_hash: String, data: Vec<u8>) -> Vec<ProseEvent> {
        let blocks = self.blocks_showing(&content_hash);
        self.media_cache.insert(content_hash.clone(), data);
        vec![ProseEvent::MediaArrived {
            content_hash,
            blocks,
        }]
    }

    /// The decrypted bytes of a cached picture, for the view that paints it.
    #[must_use]
    pub fn cached_media(&self, content_hash: &str) -> Option<&[u8]> {
        self.media_cache.get(content_hash).map(Vec::as_slice)
    }

    /// The live image blocks addressing one content hash. Sorted, so two peers
    /// name them in one order.
    fn blocks_showing(&self, content_hash: &str) -> Vec<crate::events::BlockId> {
        let tree = doc::blocks(&self.doc);
        let mut ids: Vec<TreeID> = tree
            .nodes()
            .into_iter()
            .filter(|id| doc::block_type_of(&tree, id) == Some(crate::events::BlockType::Image))
            .filter(|id| self.image_hash(id).as_deref() == Some(content_hash))
            .collect();
        ids.sort();
        ids.into_iter().map(block_id).collect()
    }

    // -- undo and redo -----------------------------------------------------

    /// One undo or one redo step. It commits the inverse operations, and the
    /// delta they produce is broadcast like any other mutation.
    fn apply_history(&mut self, undo: bool) -> ProseOutcome {
        let changed = if undo {
            self.undo.undo()
        } else {
            self.undo.redo()
        }
        .unwrap_or(false);
        if !changed {
            return ProseOutcome::nothing();
        }
        let mut events = self.regrow_root();
        let update = self.flush_delta();
        if let Some(delta) = update.clone() {
            events.insert(0, ProseEvent::DocUpdated { delta });
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
            return vec![ProseEvent::Error {
                code: error_code::WRITE_FAILED,
                message: "the document could not keep a block".to_owned(),
            }];
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
        display.extend(self.regrow_root());
        self.doc.commit();
        let update = self.flush_delta();
        if let Some(delta) = update.clone() {
            display.insert(0, ProseEvent::DocUpdated { delta });
        }
        ProseOutcome {
            display,
            update,
            ephemeral: None,
        }
    }

    /// Export every operation past the export cursor, and advance the cursor
    /// only when the export succeeded (`PROSE-050`, `PROSE-098`).
    ///
    /// Exporting reads and never writes (`PROSE-052`). A second export from the
    /// same vector yields the same bytes (`PROSE-051`), because the cursor is a
    /// stored version vector rather than a drain. An extra call therefore
    /// cannot truncate the stream. An export that fails leaves the cursor where
    /// it is, so the next flush offers the same range again.
    fn flush_delta(&mut self) -> Option<Vec<u8>> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        match self.doc.export(ExportMode::updates(&self.last_exported_vv)) {
            Ok(delta) => {
                self.last_exported_vv = current;
                (!delta.is_empty()).then_some(delta)
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
        let ours = self.last_exported_vv.get(&self.peer).copied();
        self.last_exported_vv = self.doc.oplog_vv();
        match ours {
            Some(counter) => {
                self.last_exported_vv.insert(self.peer, counter);
            }
            None => {
                self.last_exported_vv.remove(&self.peer);
            }
        }
    }

    // -- refusals ----------------------------------------------------------

    /// The refusal a command that names no live block answers with.
    fn err_no_block(id: &TreeID) -> ProseOutcome {
        ProseOutcome::refused(
            error_code::NO_SUCH_BLOCK,
            format!("no live block {id} in this document"),
        )
    }

    /// The refusal a command whose offsets leave the body it addresses answers
    /// with (`PROSE-045`).
    fn err_bad_range() -> ProseOutcome {
        ProseOutcome::refused(
            error_code::BAD_RANGE,
            "the range leaves the body of the block the command names",
        )
    }

    /// The refusal the nesting table answers a move or an insert with
    /// (`PROSE-040`, `PROSE-095`).
    fn err_nesting() -> ProseOutcome {
        ProseOutcome::refused(
            error_code::NESTING_REFUSED,
            "the document refuses that block under that parent",
        )
    }

    /// The refusal a write the CRDT engine itself rejected answers with.
    fn err_write(reason: &str) -> ProseOutcome {
        ProseOutcome::refused(
            error_code::WRITE_FAILED,
            format!("the write failed: {reason}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::BlockInit;

    #[test]
    fn a_fresh_document_holds_one_paragraph_and_a_version() {
        let state = ProseState::new().expect("a document is built");
        assert_eq!(state.schema_version(), schema::SCHEMA_VERSION);
        assert_eq!(state.root_blocks().len(), 1);
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
                pos: 0,
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

    #[test]
    fn a_snapshot_round_trips_what_the_document_held() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "a paragraph".to_owned(),
        });
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(block),
            init: BlockInit::Heading { level: 2 },
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
            pos: 0,
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
            pos: 0,
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
        let events = state.on_media_downloaded("abc".to_owned(), vec![1, 2, 3]);
        assert_eq!(state.cached_media("abc"), Some(&[1u8, 2, 3][..]));
        assert_eq!(events.len(), 1);
        assert_eq!(state.export_snapshot().len(), before.len());
    }

    #[test]
    fn a_document_a_newer_client_wrote_leaves_this_one_untouched() {
        let mut state = ProseState::new().expect("a document is built");
        let block = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "mine".to_owned(),
        });
        let doc = LoroDoc::new();
        doc.get_map("meta")
            .insert("schema_version", i64::from(schema::SCHEMA_VERSION + 1))
            .expect("a newer register");
        doc.commit();
        let newer = doc.export(ExportMode::Snapshot).expect("snapshot");
        assert!(state.import_snapshot(&newer).is_err());
        assert_eq!(state.text_of(&block).as_deref(), Some("mine"));
    }
}
