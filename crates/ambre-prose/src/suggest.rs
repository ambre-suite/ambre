//! What track changes reads off the document
//! (`features/ambre-spec-prose-track-changes.md`).
//!
//! A suggestion is an entry of `annots` naming its author. A mark of its own
//! key covers the text it proposes: `si:{id}` for an insertion, `sd:{id}` for
//! a deletion. With one key per suggestion, two suggestions over one character
//! never share a value. A span can carry an insertion and a deletion at once
//! (`PROSE-FEAT-258`). Nothing in this module writes.

use ambre_bus::app::prose::{DocCount, DocOffset, DocSpan};
use ambre_core::id::{self, Id, User};
use loro::{Container, LoroDoc, LoroText, TreeID, ValueOrContainer};

use crate::doc;
use crate::events::{Suggestion, SuggestionKind, SuggestionSpan, block_id};
use crate::mark::MarkRoot;
use ambre_bus::app::prose::MarkKey;

/// The mark key of suggestion `id`.
#[must_use]
pub(crate) fn mark_key(kind: SuggestionKind, id: &Id<id::Suggestion>) -> MarkKey {
    let root = match kind {
        SuggestionKind::Insertion => MarkRoot::Insertion,
        SuggestionKind::Deletion => MarkRoot::Deletion,
    };
    MarkKey::new(format!("{}:{}", root.as_str(), id.to_hyphenated()))
}

/// The suggestion a mark key names, when it is one.
#[must_use]
pub(crate) fn of_key(key: &str) -> Option<(SuggestionKind, Id<id::Suggestion>)> {
    let (root, id) = key.split_once(':')?;
    let kind = match MarkRoot::parse(root)? {
        MarkRoot::Insertion => SuggestionKind::Insertion,
        MarkRoot::Deletion => SuggestionKind::Deletion,
        _ => return None,
    };
    Some((kind, Id::<id::Suggestion>::parse_str(id).ok()?))
}

/// The entry of suggestion `id`, when `annots` holds one of that kind.
#[must_use]
pub(crate) fn entry(
    document: &LoroDoc,
    id: &Id<id::Suggestion>,
) -> Option<doc::Fields<doc::AnnotKey>> {
    let annots = doc::annots(document);
    let Some(ValueOrContainer::Container(Container::Map(map))) = annots.get(&id.to_hyphenated())
    else {
        return None;
    };
    let map = doc::Fields::of(map);
    (doc::kind_of(&map) == Some(doc::AnnotKind::Suggestion)).then_some(map)
}

/// The user an entry names as its author (`PROSE-FEAT-260`).
#[must_use]
pub(crate) fn owner(entry: &doc::Fields<doc::AnnotKey>) -> Option<Id<User>> {
    Id::<User>::parse_str(&doc::read_string(entry, doc::AnnotKey::User)?).ok()
}

/// One run of text under the marks it carries, in its block's own offsets.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub start: DocOffset,
    pub end: DocOffset,
    /// Every suggestion mark on the run.
    pub marks: Vec<(SuggestionKind, Id<id::Suggestion>)>,
}

/// The runs of one block, clipped to `start..end`, each with the suggestion
/// marks it carries. A run with no suggestion mark is listed too, with none.
#[must_use]
pub(crate) fn runs(
    document: &LoroDoc,
    block: &TreeID,
    start: DocOffset,
    end: DocOffset,
) -> Vec<Run> {
    let mut out = Vec::new();
    let mut offset = DocOffset::ZERO;
    for delta in doc::get_block_text(document, block).to_delta() {
        let loro::TextDelta::Insert { insert, attributes } = delta else {
            continue;
        };
        let length = DocCount::of(&insert);
        let run_end = offset + length;
        let (from, to) = (offset.max(start), run_end.min(end));
        if from < to {
            let mut marks: Vec<(SuggestionKind, Id<id::Suggestion>)> = attributes
                .iter()
                .flat_map(|attributes| attributes.keys())
                .filter_map(|key| of_key(key))
                .collect();
            // The engine keeps a run's marks in no stable order. Two
            // suggestions starting on one character are listed insertion
            // first, then by identity, alike on every peer.
            marks.sort_by_key(|(kind, id)| (*kind == SuggestionKind::Deletion, id.to_hyphenated()));
            out.push(Run {
                start: from,
                end: to,
                marks,
            });
        }
        offset = run_end;
    }
    out
}

/// Take every suggestion mark off `start..end` of `body`.
///
/// A character inserted between the ends of a mark takes that mark, whatever
/// its expansion. Typed text would then grow a suggestion its typist never
/// made (`PROSE-FEAT-266`). Only the inserted range is read, so a keystroke
/// pays for its own characters and not for the block.
pub(crate) fn strip_inherited(body: &LoroText, span: DocSpan) -> Result<(), doc::WriteError> {
    let (start, end) = (span.start.to_usize(), span.end.to_usize());
    let delta = body.slice_delta(start, end, loro::cursor::PosType::Unicode)?;
    let mut keys: Vec<String> = delta
        .iter()
        .filter_map(|run| match run {
            loro::TextDelta::Insert { attributes, .. } => attributes.as_ref(),
            _ => None,
        })
        .flat_map(|attributes| attributes.keys())
        .filter(|key| of_key(key).is_some())
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        body.unmark(start..end, &key)?;
    }
    Ok(())
}

/// Every span of every suggestion in `blocks`, in the order given, adjacent
/// runs of one suggestion joined.
#[must_use]
pub(crate) fn spans(
    document: &LoroDoc,
    blocks: &[TreeID],
) -> Vec<(
    Id<id::Suggestion>,
    SuggestionKind,
    TreeID,
    DocOffset,
    DocOffset,
)> {
    let mut out: Vec<(
        Id<id::Suggestion>,
        SuggestionKind,
        TreeID,
        DocOffset,
        DocOffset,
    )> = Vec::new();
    for block in blocks {
        for run in runs(document, block, DocOffset::ZERO, DocOffset::MAX) {
            for (kind, id) in run.marks {
                let joined = out
                    .iter_mut()
                    .rev()
                    .find(|(other, other_kind, at, _, end)| {
                        *other == id && *other_kind == kind && at == block && *end == run.start
                    });
                match joined {
                    Some(span) => span.4 = run.end,
                    None => out.push((id, kind, *block, run.start, run.end)),
                }
            }
        }
    }
    out
}

/// Every block a suggestion may sit in, in document order. The bands are left
/// out, since suggestion mode refuses to write there.
#[must_use]
pub(crate) fn text_blocks(document: &LoroDoc) -> Vec<TreeID> {
    doc::flow(&doc::blocks(document))
}

/// Every pending suggestion: an entry with at least one live span, in the
/// order of its first span. Derived each time, never stored
/// (`PROSE-FEAT-261`).
#[must_use]
pub fn pending(document: &LoroDoc) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for (id, kind, block, start, end) in spans(document, &text_blocks(document)) {
        let span = SuggestionSpan {
            block_id: block_id(block),
            start,
            end,
        };
        if let Some(known) = out.iter_mut().find(|s| s.suggestion_id == id) {
            known.spans.push(span);
            continue;
        }
        let Some(entry) = entry(document, &id) else {
            continue;
        };
        let Some(user_id) = owner(&entry) else {
            continue;
        };
        out.push(Suggestion {
            suggestion_id: id,
            kind,
            author: doc::read_string(&entry, doc::AnnotKey::Author)
                .and_then(|name| ambre_core::DisplayName::new(&name).ok()),
            user_id,
            spans: vec![span],
        });
    }
    out
}
