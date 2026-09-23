//! What the footnotes read off the document
//! (`features/ambre-spec-prose-footnotes.md`).
//!
//! A note is an entry of `annots` and a call is a carrier character marked
//! with the entry's identity. The number a note shows is counted here, from the
//! calls in document order, and never stored (`PROSE-FEAT-013`,
//! `PROSE-FEAT-095`). Nothing in this module writes.

use std::collections::HashSet;

use ambre_core::Uuid;
use loro::{Container, LoroDoc, LoroMap, LoroText, LoroValue, ValueOrContainer};

use crate::doc;
use crate::events::FootnoteEntry;

/// The kind a footnote's entry declares. A value of `kind`, not a key, so it
/// stays out of `doc.rs`, whose string constants are the key vocabulary.
pub(crate) const KIND_FOOTNOTE: &str = "footnote";

/// The entry of footnote `id`, when `annots` holds one of that kind.
#[must_use]
pub(crate) fn entry(document: &LoroDoc, id: &Uuid) -> Option<LoroMap> {
    let annots = document.get_map(doc::ANNOTS);
    let Some(ValueOrContainer::Container(Container::Map(map))) = annots.get(&id.to_hyphenated())
    else {
        return None;
    };
    let kind = map.get(doc::ANNOT_KIND);
    let is_note = matches!(
        kind,
        Some(ValueOrContainer::Value(LoroValue::String(kind))) if kind.as_str() == KIND_FOOTNOTE
    );
    is_note.then_some(map)
}

/// The body of footnote `id`.
#[must_use]
pub(crate) fn body(document: &LoroDoc, id: &Uuid) -> Option<LoroText> {
    match entry(document, id)?.get(doc::ANNOT_TEXT) {
        Some(ValueOrContainer::Container(Container::Text(text))) => Some(text),
        _ => None,
    }
}

/// Every footnote the document holds, whether a call reaches it or not.
#[must_use]
pub(crate) fn all(document: &LoroDoc) -> Vec<Uuid> {
    let annots = document.get_map(doc::ANNOTS);
    let mut ids: Vec<Uuid> = annots
        .keys()
        .filter_map(|key| Uuid::parse_str(&key).ok())
        .filter(|id| entry(document, id).is_some())
        .collect();
    ids.sort_by_key(Uuid::to_hyphenated);
    ids
}

/// Each reachable note and its number, in the order of its first call
/// (`PROSE-FEAT-013`, `PROSE-068`).
///
/// The first call in document order is note one, and the count leaves no gap.
/// A call whose note is gone counts for nothing. Two peers holding one text
/// therefore count one set of numbers (`PROSE-FEAT-097`).
#[must_use]
pub fn ordinals(document: &LoroDoc) -> Vec<(Uuid, u32)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for anchor in doc::anchors(document, doc::K_FOOTNOTE) {
        let Ok(id) = Uuid::parse_str(&anchor.value) else {
            continue;
        };
        if seen.contains(&id) || entry(document, &id).is_none() {
            continue;
        }
        seen.insert(id);
        let ordinal = u32::try_from(out.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        out.push((id, ordinal));
    }
    out
}

/// Every note as the view lists it: the numbered ones in order, then the
/// orphans no call reaches, kept for review (`PROSE-FEAT-098`).
#[must_use]
pub fn entries(document: &LoroDoc) -> Vec<FootnoteEntry> {
    let numbered = ordinals(document);
    let text_of = |id: &Uuid| {
        body(document, id)
            .map(|t| t.to_string())
            .unwrap_or_default()
    };
    let reached: HashSet<Uuid> = numbered.iter().map(|(id, _)| *id).collect();
    let mut out: Vec<FootnoteEntry> = numbered
        .iter()
        .map(|(id, ordinal)| FootnoteEntry {
            footnote_id: *id,
            ordinal: Some(*ordinal),
            text: text_of(id),
        })
        .collect();
    out.extend(
        all(document)
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
