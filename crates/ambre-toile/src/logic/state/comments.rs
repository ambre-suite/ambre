//! Comment threads for [`ToileState`](super::ToileState) (`toile-comments`).
//! A peer pins a remark on an element or on empty board, and a teammate answers
//! it hours later.
//!
//! **Physical layout.** A `comments` root `LoroMap` holds one entry per thread,
//! under a minted thread id:
//!
//! ```text
//! comments (LoroMap)
//! `-- {thread_id} (LoroMap): anchor: Binary(postcard(CommentAnchor)), resolved: bool
//!     `-- messages (LoroMovableList)
//!         `-- [n] (LoroMap): author: String, ts: i64
//!             `-- body (LoroText)
//! ```
//!
//! **Why the engine mints the id.** Two peers who comment on one spot must both
//! get a thread. Under a key derived from the anchor, each peer mints a rival
//! entry and Loro keeps one (`TOILE-FEAT-011`).
//!
//! **Why one write seeds the whole thread.** A parent creates its own container
//! under a fixed key (`TOILE-029`). The seed creates `messages` and the first
//! body, so no later reply mints a rival container.
//!
//! **Why the messages are a list.** Two readers see the order the CRDT
//! converges on. A machine owns its own clock, so the engine shows `ts` to the
//! reader and orders nothing by it (`TOILE-FEAT-012`).
//!
//! **Why a body is a `LoroText`.** Two authors rarely edit one message. A
//! whole-string register drops one of the two edits in silence
//! (`TOILE-FEAT-013`).
//!
//! **Why a comment leaves no undo item.** `Ctrl+Z` reverts what the user drew.
//! A teammate has already read the message, so a withdrawal is a deletion
//! nobody asked for. Every write here commits under
//! [`CommitOrigin::Comment`](ambre_core::crdt::CommitOrigin::Comment), which the `UndoManager` excludes
//! (`TOILE-FEAT-017`).

use std::collections::HashSet;

use loro::{LoroMap, LoroMovableList, LoroText, LoroValue};

use ambre_bus::UserId;
use ambre_core::CharCount;

use crate::logic::command::{
    CommentAnchor, CommentMessage, CommentPin, CommentSummary, CommentThread, TextDelta,
    ThreadMessageIndex, ToileEvent, ToileOutcome, Viewport,
};
use crate::logic::model::{ElementId, ThreadId};

use super::ToileState;
use super::names::{FieldValues, Fields, MessageKey, Root, ThreadKey, entry, root_map};

/// How many characters of the opening message the board-wide list carries. The
/// panel shows a row per thread, and a row is a line: the rest of the
/// conversation arrives when the thread is opened (`TOILE-205`).
const PREVIEW_CHARS: usize = 80;

impl ToileState {
    /// Open a thread at `anchor`, seeded with its first message.
    ///
    /// The engine mints the thread id, so two peers who comment on one spot get
    /// two threads (`TOILE-FEAT-011`). An empty body writes nothing. A session
    /// with no `UserId` writes nothing either: a message carries the person who
    /// wrote it (`TOILE-FEAT-014`).
    pub(super) fn create_comment(
        &mut self,
        anchor: &CommentAnchor,
        body: &str,
        ts: ambre_core::Timestamp,
    ) -> ToileOutcome {
        // A session with no identity has no author to write under. The UI cannot
        // reach this pre-`SessionReady`; guard anyway.
        let Some(author) = self.my_user_id else {
            return ToileOutcome::display_only(Vec::new());
        };
        if body.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let bytes = match postcard::to_allocvec(anchor) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Toile: comment anchor encode failed: {e}");
                return ToileOutcome::display_only(Vec::new());
            }
        };
        let thread = self.mint_element_id();
        let seeded = self.seed_thread(&thread, bytes, author, ts, body).is_some();
        // Commit in both outcomes. Loro carries an uncommitted op into the next
        // commit, so a partial seed would join the next user edit and land on
        // its undo item.
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Comment.as_str());
        self.doc.commit();
        if !seeded {
            tracing::warn!("Toile: comment thread seed failed");
            return self.emit_with_update(Vec::new());
        }
        self.emit_with_update(vec![
            ToileEvent::CommentsUpdated(thread.clone()),
            ToileEvent::CommentThreadOpened(thread),
        ])
    }

    /// Write the whole thread under `thread`: the anchor, the resolved flag, the
    /// message list and the first message, in one pass (`TOILE-029`). `None`
    /// when Loro refuses a container; the caller then commits the partial write.
    fn seed_thread(
        &mut self,
        thread: &ThreadId,
        anchor: Vec<u8>,
        author: UserId,
        ts: ambre_core::Timestamp,
        body: &str,
    ) -> Option<()> {
        let map = self
            .doc
            .get_map(Root::Comments.as_str())
            .insert_container(thread.as_str(), LoroMap::new())
            .map(Fields::<ThreadKey>::of)
            .ok()?;
        map.insert(ThreadKey::Anchor, anchor).ok()?;
        map.insert(ThreadKey::Resolved, false).ok()?;
        let messages = map
            .insert_container(ThreadKey::Messages, LoroMovableList::new())
            .ok()?;
        push_message(&messages, author, ts, body)
    }

    /// Append a message to `thread`. A missing thread, an empty body and a
    /// session with no `UserId` write nothing. Outside the undo stack, like
    /// every comment write (`TOILE-FEAT-017`).
    pub(super) fn reply_comment(
        &mut self,
        thread: &ThreadId,
        body: &str,
        ts: ambre_core::Timestamp,
    ) -> ToileOutcome {
        let Some(author) = self.my_user_id else {
            return ToileOutcome::display_only(Vec::new());
        };
        if body.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let Some(messages) = self.message_list(thread) else {
            return self.err_no_element(thread);
        };
        if push_message(&messages, author, ts, body).is_none() {
            tracing::warn!("Toile: comment reply write failed");
        }
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Comment.as_str());
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::CommentsUpdated(thread.clone())])
    }

    /// Splice the body of the author's own message at index `message`.
    ///
    /// The body is a `LoroText`, so two devices of one author converge character
    /// by character (`TOILE-FEAT-013`). The engine clamps the splice indices to
    /// the current length, like the speaker-notes editor. An edit on another
    /// author's message writes nothing. No rule says who may edit whose message
    /// (`features/ambre-spec-toile-comments.md section 6`), so the engine takes
    /// the case that needs no rule.
    pub(super) fn edit_comment(
        &mut self,
        thread: &ThreadId,
        message: ThreadMessageIndex,
        delta: &TextDelta,
    ) -> ToileOutcome {
        let Some(me) = self.my_user_id else {
            return ToileOutcome::display_only(Vec::new());
        };
        // A pure no-op produces neither a commit nor a delta.
        if delta.delete.is_zero() && delta.insert.is_empty() {
            return ToileOutcome::display_only(Vec::new());
        }
        let Some(map) = self.message_map(thread, message) else {
            return self.err_no_element(thread);
        };
        if author_of(&map) != Some(me) {
            return ToileOutcome::display_only(Vec::new());
        }
        let Some(text) = body_text(&map) else {
            return self.err_no_element(thread);
        };
        // Clamp to the current length: `at` and the deletion span never run past
        // the end of the buffer.
        let (at, del) = delta.clamped(CharCount::from_len(text.len_unicode()));
        if !del.is_zero() {
            let _ = text.delete(at.to_usize(), del.to_usize());
        }
        if !delta.insert.is_empty() {
            let _ = text.insert(at.to_usize(), &delta.insert);
        }
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Comment.as_str());
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::CommentsUpdated(thread.clone())])
    }

    /// Resolve `thread`, or reopen it. Idempotent: a flag already at `resolved`
    /// produces neither a commit nor a delta. The thread and its messages
    /// survive both ways (`TOILE-FEAT-015`).
    pub(super) fn resolve_thread(&mut self, thread: &ThreadId, resolved: bool) -> ToileOutcome {
        let Some(map) = self.thread_map(thread) else {
            return self.err_no_element(thread);
        };
        if matches!(map.value(ThreadKey::Resolved), Some(LoroValue::Bool(cur)) if cur == resolved) {
            return ToileOutcome::display_only(Vec::new());
        }
        let _ = map.insert(ThreadKey::Resolved, resolved);
        self.doc
            .set_next_commit_origin(ambre_core::crdt::CommitOrigin::Comment.as_str());
        self.doc.commit();
        self.emit_with_update(vec![ToileEvent::CommentsUpdated(thread.clone())])
    }

    // -- reads ------------------------------------------------------------

    /// Every thread of the board, in id order. The panel's board-wide list reads
    /// them. A thread whose anchor element is gone stays in the list, and a
    /// reader still opens it. Only the pin derivation skips it
    /// (`TOILE-FEAT-016`).
    pub fn comment_thread_ids(&self) -> Vec<ThreadId> {
        let LoroValue::Map(threads) = self.doc.get_map(Root::Comments.as_str()).get_deep_value()
        else {
            return Vec::new();
        };
        let mut ids: Vec<ThreadId> = threads.keys().map(|k| ElementId(k.to_string())).collect();
        // Loro fixes no iteration order on a map, so the read sorts. Two peers
        // then list the same threads in the same order.
        ids.sort();
        ids
    }

    /// One whole thread, read on demand. Derived on read and never stored, so it
    /// cannot drift from the containers; the display viewport carries the pins
    /// alone (`TOILE-026`). `None` when the thread does not exist or its anchor
    /// fails to decode.
    pub fn comment_thread(&self, id: &ThreadId) -> Option<CommentThread> {
        let LoroValue::Map(fields) = self.thread_map(id)?.map().get_deep_value() else {
            return None;
        };
        let fields = FieldValues::<ThreadKey>::of(&fields);
        let Some(LoroValue::Binary(bytes)) = fields.get(ThreadKey::Anchor) else {
            return None;
        };
        let anchor: CommentAnchor = postcard::from_bytes(bytes).ok()?;
        let resolved = matches!(fields.get(ThreadKey::Resolved), Some(LoroValue::Bool(true)));
        let messages = match fields.get(ThreadKey::Messages) {
            Some(LoroValue::List(items)) => items.iter().filter_map(read_message).collect(),
            _ => Vec::new(),
        };
        Some(CommentThread {
            id: id.clone(),
            anchor,
            resolved,
            messages,
        })
    }

    /// Every thread of the board as a list row (`TOILE-UI-023`): the panel
    /// renders these and keeps none of its own. A thread whose anchor element is
    /// gone is in the list, because the discussion outlives the drawing
    /// (`TOILE-FEAT-016`); only the pin derivation skips it (`TOILE-025`).
    pub fn comment_summaries(&self) -> Vec<CommentSummary> {
        self.comment_thread_ids()
            .into_iter()
            .filter_map(|id| {
                let thread = self.comment_thread(&id)?;
                let preview = thread
                    .messages
                    .first()
                    .map(|m| m.body.chars().take(PREVIEW_CHARS).collect())
                    .unwrap_or_default();
                Some(CommentSummary {
                    id: thread.id,
                    anchor: thread.anchor,
                    resolved: thread.resolved,
                    messages: thread.messages.len() as u32,
                    preview,
                })
            })
            .collect()
    }

    /// The pins to paint for camera `vp` (`TOILE-026`): one per thread whose
    /// anchor resolves here. An element anchor resolves to the element's
    /// top-right corner, so the pin follows the element through every move
    /// (`TOILE-024`). A point anchor resolves when the camera contains it. A
    /// thread whose anchor element is gone yields no pin and is left untouched
    /// (`TOILE-025`). No body travels (`TOILE-205`).
    ///
    /// Walked in thread-id order, so two peers on one document derive the same
    /// sequence.
    pub(super) fn comment_pins(
        &self,
        visible: &HashSet<ElementId>,
        vp: &Viewport,
    ) -> Vec<CommentPin> {
        let camera = vp.to_world_rect();
        self.comment_thread_ids()
            .into_iter()
            .filter_map(|id| {
                let thread = self.comment_thread(&id)?;
                let at = match &thread.anchor {
                    CommentAnchor::Element(element) => {
                        // Culled, or deleted: a culled element is off-camera and a
                        // deleted one has no rect at all (`TOILE-025`).
                        if !visible.contains(element) {
                            return None;
                        }
                        let rect = self.element_rect(element)?;
                        kurbo::Point::new(rect.x1, rect.y0)
                    }
                    CommentAnchor::Point { x, y } => {
                        let at = kurbo::Point::new(*x, *y);
                        if !camera.contains(at) {
                            return None;
                        }
                        at
                    }
                };
                Some(CommentPin {
                    thread: thread.id,
                    at,
                    messages: thread.messages.len() as u32,
                    resolved: thread.resolved,
                })
            })
            .collect()
    }

    /// Answer the panel's read (`TOILE-UI-023`): one whole thread, or the
    /// board-wide list. A pure read -- no commit, no delta, no undo item. An
    /// unknown thread answers nothing rather than an empty thread, so the panel
    /// keeps showing what it has.
    pub(super) fn read_comments(&self, thread: Option<&ThreadId>) -> ToileOutcome {
        let events = match thread {
            Some(id) => match self.comment_thread(id) {
                Some(t) => vec![ToileEvent::CommentThreadData(t)],
                None => Vec::new(),
            },
            None => vec![ToileEvent::CommentList(self.comment_summaries())],
        };
        ToileOutcome::display_only(events)
    }

    /// The thread's map container, or `None` when no thread carries `id`.
    fn thread_map(&self, id: &ThreadId) -> Option<Fields<ThreadKey>> {
        entry(&root_map(&self.doc, Root::Comments), id.as_str())
    }

    /// The thread's message list container.
    fn message_list(&self, thread: &ThreadId) -> Option<LoroMovableList> {
        self.thread_map(thread)?
            .get(ThreadKey::Messages)?
            .into_container()
            .ok()?
            .into_movable_list()
            .ok()
    }

    /// The map of one message, addressed by its position in the list. An
    /// out-of-range index yields `None`.
    fn message_map(
        &self,
        thread: &ThreadId,
        index: ThreadMessageIndex,
    ) -> Option<Fields<MessageKey>> {
        self.message_list(thread)?
            .get(index.to_usize())?
            .into_container()
            .ok()?
            .into_map()
            .ok()
            .map(Fields::of)
    }
}

/// Append one message to a thread's list: the author, the clock and the body
/// text container, in the same write as the message map (`TOILE-029`). `None`
/// when Loro refuses one of the writes.
fn push_message(
    messages: &LoroMovableList,
    author: UserId,
    ts: ambre_core::Timestamp,
    body: &str,
) -> Option<()> {
    let map = Fields::<MessageKey>::of(messages.push_container(LoroMap::new()).ok()?);
    map.insert(MessageKey::Author, author.to_string()).ok()?;
    map.insert(MessageKey::Ts, ts.to_i64_saturating()).ok()?;
    let text = map
        .insert_container(MessageKey::Body, LoroText::new())
        .ok()?;
    text.insert(0, body).ok()?;
    Some(())
}

/// The canonical `UserId` a message map carries, or `None` when the field is
/// absent or unparsable.
fn author_of(map: &Fields<MessageKey>) -> Option<UserId> {
    let LoroValue::String(author) = map.value(MessageKey::Author)? else {
        return None;
    };
    UserId::parse_str(&author).ok()
}

/// The `LoroText` body of a message map. The seed creates the container with the
/// message, so a missing body means a corrupt document, never a first edit.
fn body_text(map: &Fields<MessageKey>) -> Option<LoroText> {
    map.get(MessageKey::Body)?
        .into_container()
        .ok()?
        .into_text()
        .ok()
}

/// Decode one message of the thread's deep value. The reader drops a message
/// whose author does not parse, rather than show it under a wrong name.
fn read_message(value: &LoroValue) -> Option<CommentMessage> {
    let LoroValue::Map(fields) = value else {
        return None;
    };
    let fields = FieldValues::<MessageKey>::of(fields);
    let Some(LoroValue::String(author)) = fields.get(MessageKey::Author) else {
        return None;
    };
    let author = UserId::parse_str(author).ok()?;
    let ts = match fields.get(MessageKey::Ts) {
        Some(LoroValue::I64(ts)) => ambre_core::Timestamp::from_i64(*ts),
        _ => ambre_core::Timestamp::default(),
    };
    let body = match fields.get(MessageKey::Body) {
        Some(LoroValue::String(body)) => body.to_string(),
        _ => String::new(),
    };
    Some(CommentMessage { author, ts, body })
}
