//! What the comments read off the document
//! (`features/ambre-spec-prose-comments.md`).
//!
//! A thread is an entry of `annots` holding its replies, and its anchor is a
//! mark of its own key, `cm:{id}`, over the passage. The mark is the position;
//! the anchor fields of the entry are a cache of it (`PROSE-FEAT-075`).
//! Nothing in this module writes.

use std::collections::HashMap;

use ambre_bus::app::prose::{DocCount, DocOffset};
use ambre_core::{CommentId, ReplyId, UserId};
use loro::{Container, LoroDoc, LoroList, LoroMap, LoroValue, TreeID, ValueOrContainer};

use crate::doc;
use crate::events::{CommentReply, CommentThread, block_id};

/// The kind a comment's entry declares.
pub(crate) const KIND_COMMENT: &str = "comment";

/// The mark key of thread `id`.
#[must_use]
pub(crate) fn mark_key(id: &CommentId) -> String {
    format!("{}:{}", doc::K_COMMENT, id.to_hyphenated())
}

/// The thread a mark key names, when it is a comment's.
fn thread_of_key(key: &str) -> Option<CommentId> {
    let (root, id) = key.split_once(':')?;
    if root != doc::K_COMMENT {
        return None;
    }
    CommentId::parse_str(id).ok()
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
pub(crate) fn entry(document: &LoroDoc, id: &CommentId) -> Option<LoroMap> {
    let annots = document.get_map(doc::ANNOTS);
    let Some(ValueOrContainer::Container(Container::Map(map))) = annots.get(&id.to_hyphenated())
    else {
        return None;
    };
    let is_comment = matches!(
        map.get(doc::ANNOT_KIND),
        Some(ValueOrContainer::Value(LoroValue::String(kind))) if kind.as_str() == KIND_COMMENT
    );
    is_comment.then_some(map)
}

/// The reply list of an entry.
#[must_use]
pub(crate) fn replies_of(entry: &LoroMap) -> Option<LoroList> {
    match entry.get(doc::ANNOT_REPLIES) {
        Some(ValueOrContainer::Container(Container::List(list))) => Some(list),
        _ => None,
    }
}

/// Reply `index` of a list.
#[must_use]
pub(crate) fn reply_at(list: &LoroList, index: usize) -> Option<LoroMap> {
    match list.get(index) {
        Some(ValueOrContainer::Container(Container::Map(map))) => Some(map),
        _ => None,
    }
}

/// One reply as the view reads it, or `None` for a map missing a field.
fn read_reply(map: &LoroMap) -> Option<CommentReply> {
    let text = match map.get(doc::ANNOT_TEXT) {
        Some(ValueOrContainer::Container(Container::Text(text))) => text.to_string(),
        _ => String::new(),
    };
    Some(CommentReply {
        reply_id: ReplyId::parse_str(&doc::read_string(map, doc::REPLY_ID)?).ok()?,
        author: doc::read_string(map, doc::REPLY_AUTHOR).unwrap_or_default(),
        user_id: UserId::parse_str(&doc::read_string(map, doc::REPLY_USER)?).ok()?,
        created_at: ambre_bus::Timestamp::from_i64(
            doc::read_i64(map, doc::REPLY_CREATED).unwrap_or(0),
        ),
        text,
    })
}

/// The replies of a thread, in the list's own order (`PROSE-FEAT-186`).
#[must_use]
pub(crate) fn replies(entry: &LoroMap) -> Vec<CommentReply> {
    let Some(list) = replies_of(entry) else {
        return Vec::new();
    };
    (0..list.len())
        .filter_map(|index| reply_at(&list, index))
        .filter_map(|map| read_reply(&map))
        .collect()
}

/// Every thread the document holds, marked or not.
#[must_use]
pub(crate) fn all(document: &LoroDoc) -> Vec<CommentId> {
    let annots = document.get_map(doc::ANNOTS);
    let mut ids: Vec<CommentId> = annots
        .keys()
        .filter_map(|key| CommentId::parse_str(&key).ok())
        .filter(|id| entry(document, id).is_some())
        .collect();
    ids.sort_by_key(CommentId::to_hyphenated);
    ids
}

/// The cached anchor of an entry.
#[must_use]
pub(crate) fn cached(entry: &LoroMap) -> Option<Span> {
    let block =
        TreeID::try_from(doc::read_string(entry, doc::ANNOT_ANCHOR_BLOCK)?.as_str()).ok()?;
    let start = DocOffset::new(u32::try_from(doc::read_i64(entry, doc::ANNOT_ANCHOR_START)?).ok()?);
    let end = DocOffset::new(u32::try_from(doc::read_i64(entry, doc::ANNOT_ANCHOR_END)?).ok()?);
    Some(Span { block, start, end })
}

/// Every run of a comment mark in `blocks`, in the order given: the thread,
/// the block, and the run's span.
#[must_use]
pub(crate) fn runs(document: &LoroDoc, blocks: &[TreeID]) -> Vec<(CommentId, Span)> {
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
pub(crate) fn live_spans(document: &LoroDoc, blocks: &[TreeID]) -> HashMap<CommentId, Span> {
    spans_of(&runs(document, blocks))
}

/// The live span of each thread among `runs`, as [`live_spans`] reads it.
fn spans_of(runs: &[(CommentId, Span)]) -> HashMap<CommentId, Span> {
    let mut out: HashMap<CommentId, Span> = HashMap::new();
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
    let mut order: Vec<CommentId> = Vec::new();
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
                orphaned: live.is_none(),
                resolved: doc::read_bool(&entry, doc::ANNOT_RESOLVED) == Some(true),
                replies: replies(&entry),
            })
        })
        .collect()
}
