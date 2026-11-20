//! What the footnotes read off the document
//! (`features/ambre-spec-prose-footnotes.md`).
//!
//! A note is an entry of `annots` and a call is a carrier character marked
//! with the entry's identity. The number a note shows is counted here, from the
//! calls in document order, and never stored (`PROSE-FEAT-013`,
//! `PROSE-FEAT-095`). Nothing in this module writes.

use ambre_core::Ordinal;
use std::collections::HashSet;

use ambre_core::id::{Footnote, Id};
use loro::{Container, LoroDoc, LoroText, TreeID, ValueOrContainer};

use crate::doc;
use crate::events::{FootnoteCall, FootnoteEntry};
use crate::logic::Roots;

/// The entry of footnote `id`, when `annots` holds one of that kind.
#[must_use]
pub(crate) fn entry(
    document: &LoroDoc,
    roots: &Roots,
    id: &Id<Footnote>,
) -> Option<doc::Fields<doc::AnnotKey>> {
    let annots = doc::annots_read(document, roots)?;
    let Some(ValueOrContainer::Container(Container::Map(map))) = annots.get(&id.to_hyphenated())
    else {
        return None;
    };
    let map = doc::Fields::new(map);
    (doc::kind_of(&map) == Some(doc::AnnotKind::Footnote)).then_some(map)
}

/// The block holding the text of footnote `id`, when its entry names a live
/// one (`PROSE-FEAT-305`).
#[must_use]
pub(crate) fn block(document: &LoroDoc, roots: &Roots, id: &Id<Footnote>) -> Option<TreeID> {
    let named = doc::read_string(&entry(document, roots, id)?, doc::AnnotKey::NoteBlock)?;
    let block = TreeID::try_from(named.as_str()).ok()?;
    let tree = doc::blocks_read(document, roots)?;
    doc::is_note(&tree, &block).then_some(block)
}

/// The body of footnote `id`: the text of its block.
#[must_use]
pub(crate) fn body(document: &LoroDoc, roots: &Roots, id: &Id<Footnote>) -> Option<LoroText> {
    block(document, roots, id).map(|block| doc::get_block_text(document, &block))
}

/// Every footnote the document holds, whether a call reaches it or not.
#[must_use]
pub(crate) fn all(document: &LoroDoc, roots: &Roots) -> Vec<Id<Footnote>> {
    let Some(annots) = doc::annots_read(document, roots) else {
        return Vec::new();
    };
    let mut ids: Vec<Id<Footnote>> = annots
        .keys()
        .filter_map(|key| Id::<Footnote>::parse_str(&key).ok())
        .filter(|id| entry(document, roots, id).is_some())
        .collect();
    ambre_core::order::sort_by_id(&mut ids, |id| *id);
    ids
}

/// Every footnote call of the document, in document order, with the note it
/// names. A call whose value names no note id is left out.
#[must_use]
pub(crate) fn footnote_calls(document: &LoroDoc, roots: &Roots) -> Vec<doc::Anchor<Id<Footnote>>> {
    doc::anchors(document, roots, crate::mark::MarkRoot::Footnote, |value| {
        Id::<Footnote>::parse_str(value).ok()
    })
}

/// Each reachable note and its number, in the order of its first call
/// (`PROSE-FEAT-013`, `PROSE-068`).
///
/// The first call in document order is note one, and the count leaves no gap.
/// A call whose note is gone counts for nothing. Two peers holding one text
/// therefore count one set of numbers (`PROSE-FEAT-097`).
#[must_use]
pub fn ordinals(document: &LoroDoc, roots: &Roots) -> Vec<(Id<Footnote>, Ordinal<FootnoteCall>)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for anchor in footnote_calls(document, roots) {
        let id = anchor.value;
        if seen.contains(&id) || entry(document, roots, &id).is_none() {
            continue;
        }
        seen.insert(id);
        out.push((id, Ordinal::<FootnoteCall>::from_position(out.len())));
    }
    out
}

/// Every note as the view lists it: the numbered ones in order, then the
/// orphans no call reaches, kept for review (`PROSE-FEAT-098`).
#[must_use]
pub fn entries(document: &LoroDoc, roots: &Roots) -> Vec<FootnoteEntry> {
    let numbered = ordinals(document, roots);
    let text_of = |id: &Id<Footnote>| {
        body(document, roots, id)
            .map(|t| t.to_string())
            .unwrap_or_default()
    };
    let reached: HashSet<Id<Footnote>> = numbered.iter().map(|(id, _)| *id).collect();
    let mut out: Vec<FootnoteEntry> = numbered
        .iter()
        .map(|(id, ordinal)| FootnoteEntry {
            footnote_id: *id,
            ordinal: Some(*ordinal),
            text: text_of(id),
        })
        .collect();
    out.extend(
        all(document, roots)
            .into_iter()
            .filter(|id| !reached.contains(id))
            .map(|id| FootnoteEntry {
                footnote_id: id,
                ordinal: None,
                text: text_of(&id),
            }),
    );
    out
}
