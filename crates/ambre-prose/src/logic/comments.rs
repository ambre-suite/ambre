//! The seven comment commands (`features/ambre-spec-prose-comments.md`).
//!
//! A thread is an entry of `annots` and a mark of its own key over the
//! passage. The identity is minted once, by the peer that opens the thread
//! (`PROSE-FEAT-001`). Every command commits under [`COMMENT_ORIGIN`], which
//! the undo manager excludes: `Ctrl+Z` never retracts a message a collaborator
//! may have read.

use std::collections::HashMap;

use ambre_core::Uuid;
use loro::{LoroList, LoroMap, LoroText, TreeID, UpdateOptions};

use crate::comment::{self, Span};
use crate::doc;
use crate::events::{ProseEvent, ProseOutcome, ProseRefusal};

use super::ProseState;

/// The commit origin of every comment command, and of the anchor cache.
pub(super) const COMMENT_ORIGIN: &str = "comment";

impl ProseState {
    /// A thread over `range` in `block`, opened by a reply holding `text`
    /// (`PROSE-FEAT-188`).
    ///
    /// A header or a footer is refused: a band is drawn on every page, and a
    /// thread anchored there would sit on all of them. An empty range, or one
    /// the block does not hold, is refused as well.
    pub(super) fn add_comment(
        &mut self,
        block: &TreeID,
        range: (u32, u32),
        text: &str,
        now_ms: u64,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::is_band(&tree, block) {
            return ProseOutcome::refused(ProseRefusal::CommentInBand);
        }
        if doc::is_note(&tree, block) {
            return ProseOutcome::refused(ProseRefusal::CommentInNote);
        }
        let Some(body) = self.body_of(block) else {
            return Self::err_no_block(block);
        };
        let (start, end) = range;
        if start >= end || end as usize > body.len_unicode() {
            return Self::err_bad_range();
        }
        if self.my_user_id.is_none() {
            return ProseOutcome::refused(ProseRefusal::NoIdentity);
        }
        let id = self.ids.mint();
        let span = Span {
            block: *block,
            start,
            end,
        };
        let written = self.write_thread(&id, span).and_then(|entry| {
            let replies = comment::replies_of(&entry).ok_or("no reply list")?;
            self.write_reply(&replies, text, now_ms)
        });
        if let Err(e) = written {
            return Self::err_write(&e);
        }
        let key = comment::mark_key(&id);
        if let Err(e) = body.mark(start as usize..end as usize, &key, id.to_hyphenated()) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_comments()
    }

    /// A reply at the end of thread `id`, which reopens it
    /// (`PROSE-FEAT-190`, `PROSE-FEAT-079`).
    pub(super) fn reply_to_comment(&mut self, id: &Uuid, text: &str, now_ms: u64) -> ProseOutcome {
        let Some(entry) = comment::entry(&self.doc, id) else {
            return ProseOutcome::refused(ProseRefusal::NoSuchComment(*id));
        };
        if self.my_user_id.is_none() {
            return ProseOutcome::refused(ProseRefusal::NoIdentity);
        }
        let Some(replies) = comment::replies_of(&entry) else {
            return Self::err_write("no reply list");
        };
        if let Err(e) = self.write_reply(&replies, text, now_ms) {
            return Self::err_write(&e);
        }
        if let Err(e) = entry.insert(doc::ANNOT_RESOLVED, false) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_comments()
    }

    /// Close or reopen thread `id`, whoever opened it (`PROSE-FEAT-187`).
    pub(super) fn set_resolved(&mut self, id: &Uuid, resolved: bool) -> ProseOutcome {
        let Some(entry) = comment::entry(&self.doc, id) else {
            return ProseOutcome::refused(ProseRefusal::NoSuchComment(*id));
        };
        if let Err(e) = entry.insert(doc::ANNOT_RESOLVED, resolved) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_comments()
    }

    /// Remove thread `id` and its mark, and leave the text under it
    /// (`PROSE-FEAT-200`, `PROSE-FEAT-279`).
    ///
    /// Only this thread's key is taken off, so another thread over the same
    /// characters keeps its span (`PROSE-FEAT-201`).
    pub(super) fn delete_comment(&mut self, id: &Uuid) -> ProseOutcome {
        if comment::entry(&self.doc, id).is_none() {
            return ProseOutcome::refused(ProseRefusal::NoSuchComment(*id));
        }
        if let Err(e) = self.remove_thread(id) {
            return Self::err_write(&e);
        }
        self.commit_comments()
    }

    /// Remove reply `reply` of thread `id`, or the thread when the reply is
    /// its head (`PROSE-FEAT-197`, `PROSE-FEAT-081`). Only its author may
    /// (`PROSE-FEAT-198`).
    pub(super) fn delete_reply(&mut self, id: &Uuid, reply: &Uuid) -> ProseOutcome {
        let (list, index) = match self.own_reply(id, reply) {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        let removed = if index == 0 {
            self.remove_thread(id)
        } else {
            list.delete(index, 1).map_err(|e| format!("{e:?}"))
        };
        if let Err(e) = removed {
            return Self::err_write(&e);
        }
        self.commit_comments()
    }

    /// Rewrite reply `reply` of thread `id`. Only its author may
    /// (`PROSE-FEAT-195`, `PROSE-FEAT-080`).
    pub(super) fn edit_reply(&mut self, id: &Uuid, reply: &Uuid, text: &str) -> ProseOutcome {
        let (list, index) = match self.own_reply(id, reply) {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        let body = comment::reply_at(&list, index).and_then(|map| match map.get(doc::ANNOT_TEXT) {
            Some(loro::ValueOrContainer::Container(loro::Container::Text(text))) => Some(text),
            _ => None,
        });
        let Some(body) = body else {
            return Self::err_write("the reply holds no text");
        };
        if let Err(e) = body.update(text, UpdateOptions::default()) {
            return Self::err_write(&format!("{e:?}"));
        }
        self.commit_comments()
    }

    /// Every thread, marked or orphaned, as the view lists it.
    #[must_use]
    pub fn comments(&self) -> Vec<crate::events::CommentThread> {
        comment::threads(&self.doc)
    }

    /// Rewrite the cached anchor of every thread whose mark sits elsewhere
    /// than the cache says (`PROSE-FEAT-076`). A thread whose mark is gone
    /// keeps the last position it held (`PROSE-FEAT-203`).
    ///
    /// A keystroke reads the blocks the caches name, never the whole text. The
    /// whole text is read only for a thread missing from its block and not yet
    /// known to be an orphan. That is a mark a split carried away, or a thread
    /// that just lost its text. Returns whether anything was written.
    pub(super) fn refresh_anchors(&mut self) -> bool {
        let ids = comment::all(&self.doc);
        if ids.is_empty() {
            return false;
        }
        let tree = doc::blocks(&self.doc);
        let mut blocks: Vec<TreeID> = ids
            .iter()
            .filter_map(|id| comment::entry(&self.doc, id))
            .filter_map(|entry| comment::cached(&entry))
            .map(|span| span.block)
            .filter(|block| doc::node_of(&tree, block).is_some())
            .collect();
        blocks.sort();
        blocks.dedup();
        let mut spans = comment::live_spans(&self.doc, &blocks);
        let lost = ids
            .iter()
            .any(|id| !spans.contains_key(id) && !self.orphans.contains(id));
        if lost {
            spans = comment::live_spans(&self.doc, &comment::text_blocks(&self.doc));
            self.orphans = ids
                .iter()
                .filter(|id| !spans.contains_key(id))
                .copied()
                .collect();
        }
        self.write_anchors(&ids, &spans)
    }

    fn write_anchors(&self, ids: &[Uuid], spans: &HashMap<Uuid, Span>) -> bool {
        let mut wrote = false;
        for id in ids {
            let (Some(entry), Some(span)) = (comment::entry(&self.doc, id), spans.get(id)) else {
                continue;
            };
            if comment::cached(&entry).as_ref() == Some(span) {
                continue;
            }
            if let Err(e) = Self::write_anchor(&entry, *span) {
                tracing::error!("Prose: the anchor of comment {id} was not cached: {e}");
                continue;
            }
            wrote = true;
        }
        wrote
    }

    /// The entry of a new thread: its kind, its anchor, open, no reply yet.
    fn write_thread(&self, id: &Uuid, span: Span) -> Result<LoroMap, String> {
        let annots = self.doc.get_map(doc::ANNOTS);
        let entry = annots
            .insert_container(&id.to_hyphenated(), LoroMap::new())
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_KIND, comment::KIND_COMMENT)
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_RESOLVED, false)
            .map_err(|e| format!("{e:?}"))?;
        Self::write_anchor(&entry, span)?;
        entry
            .insert_container(doc::ANNOT_REPLIES, LoroList::new())
            .map_err(|e| format!("{e:?}"))?;
        Ok(entry)
    }

    fn write_anchor(entry: &LoroMap, span: Span) -> Result<(), String> {
        entry
            .insert(doc::ANNOT_ANCHOR_BLOCK, span.block.to_string())
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_ANCHOR_START, i64::from(span.start))
            .map_err(|e| format!("{e:?}"))?;
        entry
            .insert(doc::ANNOT_ANCHOR_END, i64::from(span.end))
            .map_err(|e| format!("{e:?}"))
    }

    /// One reply at the end of `list`, stamped from the session identity and
    /// never from the command (`PROSE-FEAT-077`).
    fn write_reply(&self, list: &LoroList, text: &str, now_ms: u64) -> Result<(), String> {
        let user = self.my_user_id.ok_or("no identity")?;
        let reply = list
            .push_container(LoroMap::new())
            .map_err(|e| format!("{e:?}"))?;
        reply
            .insert(doc::REPLY_ID, self.ids.mint().to_hyphenated())
            .map_err(|e| format!("{e:?}"))?;
        reply
            .insert(doc::REPLY_AUTHOR, self.my_name.clone().unwrap_or_default())
            .map_err(|e| format!("{e:?}"))?;
        reply
            .insert(doc::REPLY_USER, user.to_string())
            .map_err(|e| format!("{e:?}"))?;
        reply
            .insert(
                doc::REPLY_CREATED,
                i64::try_from(now_ms).unwrap_or(i64::MAX),
            )
            .map_err(|e| format!("{e:?}"))?;
        let body = reply
            .insert_container(doc::ANNOT_TEXT, LoroText::new())
            .map_err(|e| format!("{e:?}"))?;
        body.insert(0, text).map_err(|e| format!("{e:?}"))
    }

    /// The list holding reply `reply` of thread `id`, and its place, when the
    /// caller wrote it.
    ///
    /// The check is a guard for an honest interface and not a protection: a
    /// peer running other code writes to the CRDT directly.
    fn own_reply(&self, id: &Uuid, reply: &Uuid) -> Result<(LoroList, usize), ProseOutcome> {
        let entry = comment::entry(&self.doc, id)
            .ok_or_else(|| ProseOutcome::refused(ProseRefusal::NoSuchComment(*id)))?;
        let list = comment::replies_of(&entry)
            .ok_or_else(|| ProseOutcome::refused(ProseRefusal::NoSuchReply(*reply)))?;
        let wanted = reply.to_hyphenated();
        let found = (0..list.len()).find_map(|index| {
            let map = comment::reply_at(&list, index)?;
            (doc::read_string(&map, doc::REPLY_ID)? == wanted).then_some((index, map))
        });
        let Some((index, map)) = found else {
            return Err(ProseOutcome::refused(ProseRefusal::NoSuchReply(*reply)));
        };
        let mine = self.my_user_id.map(|user| user.to_string());
        if mine.is_none() || doc::read_string(&map, doc::REPLY_USER) != mine {
            return Err(ProseOutcome::refused(ProseRefusal::NotYourReply));
        }
        Ok((list, index))
    }

    /// Take thread `id`'s mark off every run it covers, then its entry.
    fn remove_thread(&self, id: &Uuid) -> Result<(), String> {
        let key = comment::mark_key(id);
        let runs = comment::runs(&self.doc, &comment::text_blocks(&self.doc));
        for (_, span) in runs.iter().filter(|(thread, _)| thread == id) {
            doc::get_block_text(&self.doc, &span.block)
                .unmark(span.start as usize..span.end as usize, &key)
                .map_err(|e| format!("{e:?}"))?;
        }
        self.doc
            .get_map(doc::ANNOTS)
            .delete(&id.to_hyphenated())
            .map_err(|e| format!("{e:?}"))
    }

    /// Commit a comment command out of the undo stack, and answer the list
    /// it leaves.
    fn commit_comments(&mut self) -> ProseOutcome {
        let threads = comment::threads(&self.doc);
        self.doc.set_next_commit_origin(COMMENT_ORIGIN);
        self.commit_with(vec![ProseEvent::CommentsUpdated { threads }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BlockId, CommentThread, ProseCommand};
    use ambre_core::UserId;
    use proptest::prelude::*;

    const ALICE: UserId = UserId::from_bytes([1; 16]);
    const BOB: UserId = UserId::from_bytes([2; 16]);

    fn refusal(outcome: &ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    /// A one-paragraph document holding `text`, written by `user`.
    fn document(peer: u64, user: UserId, text: &str) -> (ProseState, BlockId) {
        let mut state = ProseState::with_peer(peer).expect("a document");
        state.set_user_id(user);
        state.set_display_name(format!("user {peer}"));
        let id = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertText {
            block_id: id,
            pos: 0,
            text: text.to_owned(),
        });
        (state, id)
    }

    /// A second peer holding `from`'s document, signed in as `user`.
    fn join(from: &ProseState, peer: u64, user: UserId) -> ProseState {
        let mut state = ProseState::with_peer(peer).expect("a document");
        state.set_user_id(user);
        state
            .import_snapshot(&from.export_snapshot())
            .expect("the seed imports");
        state
    }

    fn send(from: &mut ProseState, to: &mut ProseState, command: ProseCommand) -> ProseOutcome {
        let outcome = from.handle_command(command);
        if let Some(delta) = &outcome.update {
            to.apply_remote(delta);
        }
        outcome
    }

    fn open(state: &mut ProseState, block: BlockId, range: (u32, u32), text: &str) -> Uuid {
        let before: Vec<Uuid> = state.comments().iter().map(|t| t.comment_id).collect();
        state.handle_command(ProseCommand::AddComment {
            block_id: block,
            range,
            text: text.to_owned(),
            now_ms: 10,
        });
        state
            .comments()
            .into_iter()
            .map(|t| t.comment_id)
            .find(|id| !before.contains(id))
            .expect("a new thread")
    }

    fn thread(state: &ProseState, id: Uuid) -> CommentThread {
        state
            .comments()
            .into_iter()
            .find(|t| t.comment_id == id)
            .expect("the thread")
    }

    fn span(state: &ProseState, id: Uuid) -> (u32, u32) {
        let t = thread(state, id);
        (t.anchor_start, t.anchor_end)
    }

    fn reply(state: &mut ProseState, id: Uuid, text: &str, now_ms: u64) -> ProseOutcome {
        state.handle_command(ProseCommand::ReplyToComment {
            comment_id: id,
            text: text.to_owned(),
            now_ms,
        })
    }

    fn texts(state: &ProseState, id: Uuid) -> Vec<String> {
        thread(state, id)
            .replies
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    fn cache(state: &ProseState, id: Uuid) -> Option<Span> {
        comment::entry(&state.doc, &id).and_then(|entry| comment::cached(&entry))
    }

    /// Opening a thread writes one entry, and a mark over the range and no
    /// character outside it (`PROSE-FEAT-188`).
    #[test]
    fn a_thread_marks_its_range_and_no_character_outside_it() {
        let (mut state, block) = document(1, ALICE, "Hello brave world");
        let id = open(&mut state, block, (6, 11), "Why brave?");
        let runs: Vec<(u32, u32)> = comment::runs(&state.doc, &comment::text_blocks(&state.doc))
            .into_iter()
            .filter(|(thread, _)| *thread == id)
            .map(|(_, s)| (s.start, s.end))
            .collect();
        assert_eq!(runs, vec![(6, 11)]);
        assert_eq!(comment::all(&state.doc), vec![id]);
        let t = thread(&state, id);
        assert_eq!(t.anchor_block, Some(block));
        assert!(!t.orphaned && !t.resolved);
        assert_eq!(texts(&state, id), vec!["Why brave?".to_owned()]);
    }

    /// Opening a thread leaves the text and every other thread as they were
    /// (`PROSE-FEAT-189`).
    #[test]
    fn opening_a_thread_touches_neither_the_text_nor_another_thread() {
        let (mut state, block) = document(1, ALICE, "Hello brave world");
        let first = open(&mut state, block, (0, 11), "First.");
        let before = thread(&state, first);
        open(&mut state, block, (6, 17), "Second.");
        assert_eq!(state.text_of(&block).as_deref(), Some("Hello brave world"));
        assert_eq!(thread(&state, first), before);
    }

    /// A band, an empty range, a range past the text, and a session with no
    /// user are refused, and write nothing (`PROSE-FEAT-295`).
    #[test]
    fn a_thread_is_refused_in_a_band_on_an_empty_range_or_with_no_user() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let add = |state: &mut ProseState, block, range| {
            refusal(&state.handle_command(ProseCommand::AddComment {
                block_id: block,
                range,
                text: "x".to_owned(),
                now_ms: 0,
            }))
        };
        assert_eq!(add(&mut state, block, (2, 2)), Some(ProseRefusal::BadRange));
        assert_eq!(add(&mut state, block, (2, 9)), Some(ProseRefusal::BadRange));
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        let header = state.bands().header.expect("a header");
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: 0,
            text: "Title".to_owned(),
        });
        assert_eq!(
            add(&mut state, header, (0, 2)),
            Some(ProseRefusal::CommentInBand)
        );
        let mut anonymous = ProseState::with_peer(2).expect("a document");
        let root = anonymous.root_blocks()[0];
        anonymous.handle_command(ProseCommand::InsertText {
            block_id: root,
            pos: 0,
            text: "Hello".to_owned(),
        });
        assert_eq!(
            add(&mut anonymous, root, (0, 2)),
            Some(ProseRefusal::NoIdentity)
        );
        assert!(state.comments().is_empty() && anonymous.comments().is_empty());
    }

    /// The author is the session's, never the command's: the command carries
    /// none (`PROSE-FEAT-077`).
    #[test]
    fn the_author_is_stamped_from_the_session() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let id = open(&mut state, block, (0, 5), "Mine.");
        let head = thread(&state, id).replies.remove(0);
        assert_eq!(head.user_id, ALICE);
        assert_eq!(head.author, "user 1");
        assert_eq!(head.created_at, 10);
    }

    /// A reply lands last, in the list's order and not the stamp's
    /// (`PROSE-FEAT-190`, `PROSE-FEAT-186`).
    #[test]
    fn a_reply_lands_last_whatever_its_stamp() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let id = open(&mut state, block, (0, 5), "One.");
        reply(&mut state, id, "Two.", 5);
        reply(&mut state, id, "Three.", 1);
        assert_eq!(texts(&state, id), vec!["One.", "Two.", "Three."]);
    }

    /// A reply to a closed thread reopens it (`PROSE-FEAT-079`).
    #[test]
    fn a_reply_reopens_a_resolved_thread() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let id = open(&mut state, block, (0, 5), "One.");
        state.handle_command(ProseCommand::ResolveComment { comment_id: id });
        assert!(thread(&state, id).resolved);
        reply(&mut state, id, "Two.", 11);
        assert!(!thread(&state, id).resolved);
    }

    /// A reply moves no mark, rewrites no earlier reply and touches no other
    /// thread (`PROSE-FEAT-191`).
    #[test]
    fn a_reply_touches_nothing_but_its_own_list() {
        let (mut state, block) = document(1, ALICE, "Hello world");
        let id = open(&mut state, block, (0, 5), "One.");
        let other = open(&mut state, block, (6, 11), "Other.");
        let before = thread(&state, other);
        reply(&mut state, id, "Two.", 11);
        assert_eq!(span(&state, id), (0, 5));
        assert_eq!(texts(&state, id)[0], "One.");
        assert_eq!(thread(&state, other), before);
    }

    /// Resolving closes, reopening opens with no reply added, and neither
    /// touches a span or a reply (`PROSE-FEAT-192`, `PROSE-FEAT-193`,
    /// `PROSE-FEAT-194`).
    #[test]
    fn resolving_and_reopening_flip_the_flag_alone() {
        let (mut state, block) = document(1, ALICE, "Hello world");
        let id = open(&mut state, block, (0, 5), "One.");
        let other = open(&mut state, block, (3, 11), "Other.");
        let before = thread(&state, other);
        state.handle_command(ProseCommand::ResolveComment { comment_id: id });
        assert!(thread(&state, id).resolved);
        state.handle_command(ProseCommand::ReopenComment { comment_id: id });
        let t = thread(&state, id);
        assert!(!t.resolved);
        assert_eq!(texts(&state, id), vec!["One."]);
        assert_eq!((t.anchor_start, t.anchor_end), (0, 5));
        assert_eq!(thread(&state, other), before);
    }

    /// A thread is everyone's: another member resolves, reopens and deletes
    /// it (`PROSE-FEAT-187`).
    #[test]
    fn another_member_may_close_reopen_and_delete_a_thread() {
        let (mut alice, block) = document(1, ALICE, "Hello");
        let id = open(&mut alice, block, (0, 5), "Mine.");
        let mut bob = join(&alice, 2, BOB);
        for command in [
            ProseCommand::ResolveComment { comment_id: id },
            ProseCommand::ReopenComment { comment_id: id },
            ProseCommand::DeleteComment { comment_id: id },
        ] {
            assert_eq!(refusal(&bob.handle_command(command)), None);
        }
        assert!(bob.comments().is_empty());
    }

    /// Editing a reply rewrites its text in place, its author's alone, and
    /// leaves the flag, the mark and the stamp (`PROSE-FEAT-195`,
    /// `PROSE-FEAT-080`, `PROSE-FEAT-196`).
    #[test]
    fn a_reply_is_edited_in_place_by_its_author_alone() {
        let (mut alice, block) = document(1, ALICE, "Hello");
        let id = open(&mut alice, block, (0, 5), "One.");
        reply(&mut alice, id, "Two.", 11);
        alice.handle_command(ProseCommand::ResolveComment { comment_id: id });
        let second = thread(&alice, id).replies[1].reply_id;
        let edit = |text: &str| ProseCommand::EditCommentReply {
            comment_id: id,
            reply_id: second,
            new_text: text.to_owned(),
        };
        let mut bob = join(&alice, 2, BOB);
        assert_eq!(
            refusal(&bob.handle_command(edit("Forged."))),
            Some(ProseRefusal::NotYourReply)
        );
        assert_eq!(texts(&bob, id), vec!["One.", "Two."]);
        alice.handle_command(edit("Deux."));
        let t = thread(&alice, id);
        assert_eq!(texts(&alice, id), vec!["One.", "Deux."]);
        assert!(t.resolved);
        assert_eq!((t.anchor_start, t.anchor_end), (0, 5));
        assert_eq!(t.replies[1].user_id, ALICE);
        assert_eq!(t.replies[1].created_at, 11);
    }

    /// Deleting a reply takes it out and keeps the order around it, its
    /// author's alone, and leaves the mark and the flag (`PROSE-FEAT-197`,
    /// `PROSE-FEAT-198`, `PROSE-FEAT-199`).
    #[test]
    fn a_reply_is_deleted_by_its_author_alone() {
        let (mut alice, block) = document(1, ALICE, "Hello");
        let id = open(&mut alice, block, (0, 5), "One.");
        for (text, at) in [("Two.", 11), ("Three.", 12)] {
            reply(&mut alice, id, text, at);
        }
        alice.handle_command(ProseCommand::ResolveComment { comment_id: id });
        let second = thread(&alice, id).replies[1].reply_id;
        let delete = ProseCommand::DeleteReply {
            comment_id: id,
            reply_id: second,
        };
        let mut bob = join(&alice, 2, BOB);
        assert_eq!(
            refusal(&bob.handle_command(delete.clone())),
            Some(ProseRefusal::NotYourReply)
        );
        alice.handle_command(delete);
        let t = thread(&alice, id);
        assert_eq!(texts(&alice, id), vec!["One.", "Three."]);
        assert!(t.resolved);
        assert_eq!((t.anchor_start, t.anchor_end), (0, 5));
    }

    /// Deleting the opening reply deletes the thread (`PROSE-FEAT-081`).
    #[test]
    fn deleting_the_head_deletes_the_thread() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let id = open(&mut state, block, (0, 5), "One.");
        reply(&mut state, id, "Two.", 11);
        let head = thread(&state, id).replies[0].reply_id;
        state.handle_command(ProseCommand::DeleteReply {
            comment_id: id,
            reply_id: head,
        });
        assert!(state.comments().is_empty());
        assert!(comment::runs(&state.doc, &comment::text_blocks(&state.doc)).is_empty());
    }

    /// Deleting a thread removes its entry and its mark, leaves the text, and
    /// leaves another thread over the same characters whole
    /// (`PROSE-FEAT-200`, `PROSE-FEAT-279`, `PROSE-FEAT-201`).
    #[test]
    fn deleting_a_thread_leaves_the_text_and_the_thread_beside_it() {
        let (mut state, block) = document(1, ALICE, "Hello brave world");
        let gone = open(&mut state, block, (0, 11), "Gone.");
        let kept = open(&mut state, block, (6, 17), "Kept.");
        state.handle_command(ProseCommand::DeleteComment { comment_id: gone });
        assert_eq!(comment::all(&state.doc), vec![kept]);
        assert_eq!(span(&state, kept), (6, 17));
        assert!(!thread(&state, kept).orphaned);
        assert_eq!(state.text_of(&block).as_deref(), Some("Hello brave world"));
        assert!(
            comment::runs(&state.doc, &comment::text_blocks(&state.doc))
                .iter()
                .all(|(thread, _)| *thread == kept)
        );
    }

    /// The cache follows the mark on an edit to the block, and the mark wins
    /// a cache that disagrees (`PROSE-FEAT-076`, `PROSE-FEAT-075`,
    /// `PROSE-FEAT-185`).
    #[test]
    fn the_cache_follows_the_mark_and_never_overrules_it() {
        let (mut state, block) = document(1, ALICE, "Hello world");
        let id = open(&mut state, block, (6, 11), "World.");
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "Oh, ".to_owned(),
        });
        let moved = Span {
            block: crate::events::tree_id(block),
            start: 10,
            end: 15,
        };
        assert_eq!(cache(&state, id), Some(moved));
        assert_eq!(span(&state, id), (10, 15));
        let entry = comment::entry(&state.doc, &id).expect("the entry");
        entry
            .insert(doc::ANNOT_ANCHOR_START, 0_i64)
            .expect("a write");
        assert_eq!(span(&state, id), (10, 15), "the mark wins");
    }

    /// Deleting the whole range keeps the thread, orphaned, at the last place
    /// its mark held, with its flag and replies (`PROSE-FEAT-202`,
    /// `PROSE-FEAT-203`, `PROSE-FEAT-204`).
    #[test]
    fn deleting_the_range_orphans_the_thread_where_it_last_was() {
        let (mut state, block) = document(1, ALICE, "Hello brave world");
        let id = open(&mut state, block, (6, 11), "Brave?");
        let other = open(&mut state, block, (12, 17), "World.");
        state.handle_command(ProseCommand::ResolveComment { comment_id: id });
        let before = thread(&state, other);
        state.handle_command(ProseCommand::DeleteText {
            block_id: block,
            range: (5, 11),
        });
        let t = thread(&state, id);
        assert!(t.orphaned);
        assert!(t.resolved);
        assert_eq!(texts(&state, id), vec!["Brave?"]);
        assert_eq!((t.anchor_start, t.anchor_end), (6, 11));
        assert_eq!(span(&state, other), (6, 11));
        assert_eq!(thread(&state, other).replies, before.replies);
    }

    /// An edit, a reply and a remote delta leave the thread's identity
    /// (`PROSE-FEAT-001`).
    #[test]
    fn a_thread_keeps_its_identity_through_edits_and_peers() {
        let (mut alice, block) = document(1, ALICE, "Hello");
        let id = open(&mut alice, block, (0, 5), "One.");
        let mut bob = join(&alice, 2, BOB);
        send(
            &mut alice,
            &mut bob,
            ProseCommand::InsertText {
                block_id: block,
                pos: 0,
                text: "Oh ".to_owned(),
            },
        );
        send(
            &mut bob,
            &mut alice,
            ProseCommand::ReplyToComment {
                comment_id: id,
                text: "Two.".to_owned(),
                now_ms: 11,
            },
        );
        let ids = |state: &ProseState| -> Vec<Uuid> {
            state.comments().iter().map(|t| t.comment_id).collect()
        };
        assert_eq!(ids(&alice), vec![id]);
        assert_eq!(ids(&bob), vec![id]);
    }

    /// Two peers commenting over shared characters at once keep two threads,
    /// each on its own span; resolving one leaves the other's span
    /// (`PROSE-FEAT-078`).
    #[test]
    fn two_threads_over_shared_characters_keep_their_own_spans() {
        let (mut alice, block) = document(1, ALICE, "Hello brave world");
        let mut bob = join(&alice, 2, BOB);
        let mine = alice.handle_command(ProseCommand::AddComment {
            block_id: block,
            range: (0, 11),
            text: "Alice.".to_owned(),
            now_ms: 1,
        });
        let theirs = bob.handle_command(ProseCommand::AddComment {
            block_id: block,
            range: (6, 17),
            text: "Bob.".to_owned(),
            now_ms: 1,
        });
        bob.apply_remote(&mine.update.expect("a delta"));
        alice.apply_remote(&theirs.update.expect("a delta"));
        let spans = |state: &ProseState| -> Vec<(u32, u32)> {
            let mut out: Vec<(u32, u32)> = state
                .comments()
                .iter()
                .map(|t| (t.anchor_start, t.anchor_end))
                .collect();
            out.sort_unstable();
            out
        };
        assert_eq!(spans(&alice), vec![(0, 11), (6, 17)]);
        assert_eq!(spans(&bob), spans(&alice));
        let alices = alice
            .comments()
            .into_iter()
            .find(|t| t.anchor_start == 0)
            .expect("Alice's thread")
            .comment_id;
        send(
            &mut alice,
            &mut bob,
            ProseCommand::ResolveComment { comment_id: alices },
        );
        assert_eq!(spans(&bob), vec![(0, 11), (6, 17)]);
    }

    /// `Ctrl+Z` retracts the typing and never the comment
    /// (`PROSE-FEAT-296`).
    #[test]
    fn an_undo_never_retracts_a_comment() {
        let (mut state, block) = document(1, ALICE, "Hello");
        let id = open(&mut state, block, (0, 5), "One.");
        reply(&mut state, id, "Two.", 11);
        state.handle_command(ProseCommand::Undo);
        assert_eq!(texts(&state, id), vec!["One.", "Two."]);
        assert_eq!(state.text_of(&block).as_deref(), Some(""));
        assert!(thread(&state, id).orphaned, "the undone text took the mark");
    }

    /// A reply that races the deletion of its thread is lost, on both peers
    /// alike (`PROSE-FEAT-297`).
    #[test]
    fn a_reply_racing_the_deletion_of_its_thread_is_lost() {
        let (mut alice, block) = document(1, ALICE, "Hello");
        let id = open(&mut alice, block, (0, 5), "One.");
        let head = thread(&alice, id).replies[0].reply_id;
        let mut bob = join(&alice, 2, BOB);
        let deleted = alice.handle_command(ProseCommand::DeleteReply {
            comment_id: id,
            reply_id: head,
        });
        let replied = reply(&mut bob, id, "Two.", 11);
        bob.apply_remote(&deleted.update.expect("a delta"));
        alice.apply_remote(&replied.update.expect("a delta"));
        assert!(alice.comments().is_empty());
        assert!(bob.comments().is_empty());
    }

    proptest! {
        /// Whatever two peers type and delete around it, a thread ends on
        /// the text it was opened on, or orphaned. It never lands on other
        /// text, and both peers agree (`PROSE-FEAT-009`).
        #[test]
        fn a_thread_stays_on_its_text_under_concurrent_edits(
            edits in proptest::collection::vec((any::<bool>(), any::<bool>(), 0_u32..40, 1_u32..4), 1..12)
        ) {
            let (mut alice, block) = document(1, ALICE, "aaaa bbbb cccc");
            let id = open(&mut alice, block, (5, 9), "On b.");
            let mut bob = join(&alice, 2, BOB);
            let mut pending: Vec<(bool, Vec<u8>)> = Vec::new();
            for (by_alice, insert, at, len) in edits {
                let state = if by_alice { &mut alice } else { &mut bob };
                let body = state.text_of(&block).unwrap_or_default().chars().count() as u32;
                let command = if insert || body == 0 {
                    ProseCommand::InsertText {
                        block_id: block,
                        pos: at.min(body),
                        text: "x".repeat(len as usize),
                    }
                } else {
                    let start = at.min(body - 1);
                    ProseCommand::DeleteText {
                        block_id: block,
                        range: (start, (start + len).min(body)),
                    }
                };
                if let Some(delta) = state.handle_command(command).update {
                    pending.push((by_alice, delta));
                }
            }
            for (by_alice, delta) in pending {
                let other = if by_alice { &mut bob } else { &mut alice };
                other.apply_remote(&delta);
            }
            let (here, there) = (thread(&alice, id), thread(&bob, id));
            let whole = alice.text_of(&block).unwrap_or_default();
            let left = whole.chars().filter(|c| *c == 'b').count();
            prop_assert_eq!(here.orphaned, there.orphaned);
            // A thread is orphaned only once its text is gone. It may outlive
            // every `b` when a peer typed inside the passage meanwhile.
            prop_assert!(!here.orphaned || left == 0);
            if !here.orphaned {
                prop_assert_eq!(span(&alice, id), span(&bob, id));
                let text: String = whole.chars()
                    .skip(here.anchor_start as usize)
                    .take((here.anchor_end - here.anchor_start) as usize)
                    .collect();
                // Typing inside the passage joins it, and no text around it
                // ever does.
                prop_assert!(text.chars().all(|c| c == 'b' || c == 'x'), "on {}", text);
                prop_assert_eq!(text.chars().filter(|c| *c == 'b').count(), left);
            }
        }
    }
}
