//! What the comments read off the document
//! (`features/ambre-spec-prose-comments.md`).
//!
//! A thread is an entry of `annots` holding its replies, and its anchor is a
//! mark of its own key, `cm:{id}`, over the passage. The mark is the position;
//! the anchor fields of the entry are a cache of it (`PROSE-FEAT-075`).
//! Nothing in this module writes.

use ambre_core::StoredAsI64;
use std::collections::HashMap;

use ambre_bus::app::prose::{DocCount, DocOffset};
use ambre_core::id::{Comment, Id, Reply, User};
use loro::{Container, LoroDoc, LoroList, TreeID, ValueOrContainer};

use crate::doc::{self, AnnotKey, Fields, ReplyKey};
use ambre_core::ThreadState;

use crate::events::{AnchorState, CommentReply, CommentThread, block_id};
use crate::mark::MarkRoot;
use ambre_bus::app::prose::MarkKey;

/// The mark key of thread `id`.
#[must_use]
pub(crate) fn mark_key(id: &Id<Comment>) -> MarkKey {
    MarkKey::new(format!(
        "{}:{}",
        MarkRoot::Comment.as_str(),
        id.to_hyphenated()
    ))
}

/// The thread a mark key names, when it is a comment's.
fn thread_of_key(key: &str) -> Option<Id<Comment>> {
    let (root, id) = key.split_once(':')?;
    if MarkRoot::parse(root)? != MarkRoot::Comment {
        return None;
    }
    Id::<Comment>::parse_str(id).ok()
}

/// Where a mark sits: a block and a span in its own offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub block: TreeID,
    pub start: DocOffset,
    pub end: DocOffset,
}

/// The entry of thread `id`, when `annots` holds one of that kind.
#[must_use]
pub(crate) fn entry(document: &LoroDoc, id: &Id<Comment>) -> Option<Fields<AnnotKey>> {
    let annots = doc::annots(document);
    let Some(ValueOrContainer::Container(Container::Map(map))) = annots.get(&id.to_hyphenated())
    else {
        return None;
    };
    let map = Fields::of(map);
    (doc::kind_of(&map) == Some(doc::AnnotKind::Comment)).then_some(map)
}

/// The reply list of an entry.
#[must_use]
pub(crate) fn replies_of(entry: &Fields<AnnotKey>) -> Option<LoroList> {
    match entry.get(AnnotKey::Replies) {
        Some(ValueOrContainer::Container(Container::List(list))) => Some(list),
        _ => None,
    }
}

ambre_core::define_index! {
    /// The place of a reply in its thread's list, the head being zero.
    pub struct ReplySlot(usize);
    /// A number of replies of one thread.
    pub struct ReplySlotCount;
    /// The signed gap between two replies of one thread.
    pub struct ReplySlotDelta(isize);
    /// A stretch of replies of one thread.
    pub struct ReplySlotSpan;
    /// A list indexed by reply of one thread.
    pub struct ReplySlotVec;
}

/// Every place of a reply list, the head first.
#[must_use]
pub(crate) fn reply_slots(list: &LoroList) -> ReplySlotSpan {
    ReplySlotSpan::from_count(ReplySlot::ZERO, ReplySlotCount::from_len(list.len()))
}

/// Reply `slot` of a list.
#[must_use]
pub(crate) fn reply_at(list: &LoroList, slot: ReplySlot) -> Option<Fields<ReplyKey>> {
    match list.get(slot.to_usize()) {
        Some(ValueOrContainer::Container(Container::Map(map))) => Some(Fields::of(map)),
        _ => None,
    }
}

/// One reply as the view reads it, or `None` for a map missing a field.
fn read_reply(map: &Fields<ReplyKey>) -> Option<CommentReply> {
    let text = match map.get(ReplyKey::Text) {
        Some(ValueOrContainer::Container(Container::Text(text))) => text.to_string(),
        _ => String::new(),
    };
    Some(CommentReply {
        reply_id: Id::<Reply>::parse_str(&doc::read_string(map, ReplyKey::Id)?).ok()?,
        author: doc::read_string(map, ReplyKey::Author)
            .and_then(|name| ambre_core::DisplayName::new(&name).ok()),
        user_id: Id::<User>::parse_str(&doc::read_string(map, ReplyKey::User)?).ok()?,
        created_at: ambre_bus::Instant::<ambre_bus::Authored>::from_i64(
            doc::read_i64(map, ReplyKey::Created).unwrap_or(0),
        ),
        text,
    })
}

/// The replies of a thread, in the list's own order (`PROSE-FEAT-186`).
#[must_use]
pub(crate) fn replies(entry: &Fields<AnnotKey>) -> Vec<CommentReply> {
    let Some(list) = replies_of(entry) else {
        return Vec::new();
    };
    reply_slots(&list)
        .into_iter()
        .filter_map(|slot| reply_at(&list, slot))
        .filter_map(|map| read_reply(&map))
        .collect()
}

/// Every thread the document holds, marked or not.
#[must_use]
pub(crate) fn all(document: &LoroDoc) -> Vec<Id<Comment>> {
    let annots = doc::annots(document);
    let mut ids: Vec<Id<Comment>> = annots
        .keys()
        .filter_map(|key| Id::<Comment>::parse_str(&key).ok())
        .filter(|id| entry(document, id).is_some())
        .collect();
    ids.sort_by_key(Id::<Comment>::to_hyphenated);
    ids
}

/// The cached anchor of an entry.
#[must_use]
pub(crate) fn cached(entry: &Fields<AnnotKey>) -> Option<Span> {
    let block = TreeID::try_from(doc::read_string(entry, AnnotKey::AnchorBlock)?.as_str()).ok()?;
    let start = DocOffset::new(u32::try_from(doc::read_i64(entry, AnnotKey::AnchorStart)?).ok()?);
    let end = DocOffset::new(u32::try_from(doc::read_i64(entry, AnnotKey::AnchorEnd)?).ok()?);
    Some(Span { block, start, end })
}

/// Every run of a comment mark in `blocks`, in the order given: the thread,
/// the block, and the run's span.
#[must_use]
pub(crate) fn runs(document: &LoroDoc, blocks: &[TreeID]) -> Vec<(Id<Comment>, Span)> {
    let mut out = Vec::new();
    for id in blocks {
        let mut offset = DocOffset::ZERO;
        for delta in doc::get_block_text(document, id).to_delta() {
            let loro::TextDelta::Insert { insert, attributes } = delta else {
                continue;
            };
            let length = DocCount::of(&insert);
            let end = offset + length;
            for key in attributes.iter().flat_map(|attributes| attributes.keys()) {
                if let Some(thread) = thread_of_key(key) {
                    out.push((
                        thread,
                        Span {
                            block: *id,
                            start: offset,
                            end,
                        },
                    ));
                }
            }
            offset = end;
        }
    }
    out
}

/// The live span of each marked thread: the first block its mark reaches, from
/// the mark's first character there to its last.
#[must_use]
pub(crate) fn live_spans(document: &LoroDoc, blocks: &[TreeID]) -> HashMap<Id<Comment>, Span> {
    spans_of(&runs(document, blocks))
}

/// The live span of each thread among `runs`, as [`live_spans`] reads it.
fn spans_of(runs: &[(Id<Comment>, Span)]) -> HashMap<Id<Comment>, Span> {
    let mut out: HashMap<Id<Comment>, Span> = HashMap::new();
    for &(thread, run) in runs {
        match out.get_mut(&thread) {
            None => {
                out.insert(thread, run);
            }
            Some(span) if span.block == run.block => {
                span.start = span.start.min(run.start);
                span.end = span.end.max(run.end);
            }
            Some(_) => {}
        }
    }
    out
}

/// Every live block, in document order. The bands are left out: a thread is
/// never opened there.
#[must_use]
pub(crate) fn text_blocks(document: &LoroDoc) -> Vec<TreeID> {
    doc::flow(&doc::blocks(document))
}

/// Every thread as the view lists it: the marked ones in the order their
/// anchors appear, then the orphans, kept for review (`PROSE-FEAT-202`).
#[must_use]
pub fn threads(document: &LoroDoc) -> Vec<CommentThread> {
    let runs = runs(document, &text_blocks(document));
    let spans = spans_of(&runs);
    let mut order: Vec<Id<Comment>> = Vec::new();
    for (thread, _) in &runs {
        if !order.contains(thread) {
            order.push(*thread);
        }
    }
    let ids = all(document);
    let marked = order.iter().filter(|id| ids.contains(id)).copied();
    let orphans = ids.iter().filter(|id| !spans.contains_key(id)).copied();
    marked
        .chain(orphans)
        .filter_map(|id| {
            let entry = entry(document, &id)?;
            let live = spans.get(&id).copied();
            let anchor = live.or_else(|| cached(&entry));
            Some(CommentThread {
                comment_id: id,
                anchor_block: anchor.map(|span| block_id(span.block)),
                anchor_start: anchor.map_or(DocOffset::ZERO, |span| span.start),
                anchor_end: anchor.map_or(DocOffset::ZERO, |span| span.end),
                orphaned: AnchorState::from_bool(live.is_none()),
                resolved: ThreadState::resolved_if(
                    doc::read_bool(&entry, AnnotKey::Resolved) == Some(true),
                ),
                replies: replies(&entry),
            })
        })
        .collect()
}
