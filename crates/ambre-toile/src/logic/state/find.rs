//! Board text search for [`ToileState`](super::ToileState).
//!
//! `Find` is **non-mutating** and runs entirely on the Logic thread. The UI only
//! holds the culled `DisplayViewport`, so a match on an off-screen sticky is
//! invisible to it. The handler walks the text-bearing elements (sticky / text
//! bodies, frame titles) in z-order and returns their ids plus the next hit to
//! recenter on. Matches are local view state, never synced and never an undo
//! item.

use crate::logic::command::{FindOptions, FindResults, ToileEvent, ToileOutcome};
use crate::logic::model::{ElementId, NodeId};

use super::ToileState;
use super::codec::{body_of, str_of};

impl ToileState {
    /// Scan the text-bearing elements for `query`, returning the matching ids in
    /// z-order and the recenter target. Read-only: no CRDT mutation,
    /// no sync payload. An empty query yields no hits (the UI's "clear" signal).
    pub(super) fn find(
        &self,
        query: &str,
        options: FindOptions,
        from: Option<&ElementId>,
        backward: bool,
    ) -> ToileOutcome {
        let hits = if query.is_empty() {
            Vec::new()
        } else {
            self.find_hits(query, options)
        };
        let focus = next_focus(&hits, from, backward);
        ToileOutcome::display_only(vec![ToileEvent::FindResults(FindResults { hits, focus })])
    }

    /// Text-bearing element ids matching `query`, in paint order. The
    /// `layers` walk comes first, then any element absent from `layers`,
    /// appended in `ElementId` order (the well-formedness fallback). A
    /// concurrently dropped `layers` entry therefore never hides a hit.
    fn find_hits(&self, query: &str, options: FindOptions) -> Vec<ElementId> {
        let mut hits = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for node in &self.layers {
            if let NodeId::Element(id) = node
                && seen.insert(id.clone())
                && self.element_matches(id, query, options)
            {
                hits.push(id.clone());
            }
        }
        // Fallback: text-bearing elements missing from `layers`, id-ordered.
        let mut orphans: Vec<ElementId> = self
            .bounds
            .keys()
            .filter(|id| !seen.contains(*id))
            .filter(|id| self.element_matches(id, query, options))
            .cloned()
            .collect();
        orphans.sort();
        hits.extend(orphans);
        hits
    }

    /// Whether the element's searchable text (sticky / text body, frame title)
    /// matches `query` under the given options. Non-text elements never match.
    fn element_matches(&self, id: &ElementId, query: &str, options: FindOptions) -> bool {
        let Some(em) = self.element_map(id) else {
            return false;
        };
        let body = match str_of(&em, "type").as_str() {
            "sticky" | "text" => body_of(&em),
            // A frame title lives in its `name` field (empty until named).
            "frame" => str_of(&em, "name"),
            _ => return false,
        };
        text_matches(&body, query, options)
    }
}

/// The hit to recenter on. That is the first hit for a fresh search, otherwise
/// the hit adjacent to `from` in the requested direction, wrapping around.
/// `backward` steps to the previous hit, else the next. A `from` that is no
/// longer a hit restarts at the first.
fn next_focus(hits: &[ElementId], from: Option<&ElementId>, backward: bool) -> Option<ElementId> {
    let first = hits.first().cloned();
    let Some(from) = from else {
        return first;
    };
    let len = hits.len();
    match hits.iter().position(|h| h == from) {
        // `+ len` before subtracting keeps the step unsigned and wraps correctly
        // for the backward case at index 0.
        Some(idx) if backward => hits.get((idx + len - 1) % len).cloned(),
        Some(idx) => hits.get((idx + 1) % len).cloned(),
        None => first,
    }
}

/// Substring test honoring `match_case` and `whole_word`. Case folding is a
/// plain lowercase on both sides (adequate for the Latin-script MVP).
fn text_matches(haystack: &str, needle: &str, options: FindOptions) -> bool {
    if needle.is_empty() {
        return false;
    }
    if options.match_case {
        contains_match(haystack, needle, options.whole_word)
    } else {
        contains_match(
            &haystack.to_lowercase(),
            &needle.to_lowercase(),
            options.whole_word,
        )
    }
}

/// `haystack.contains(needle)`, refined to word boundaries when `whole_word`.
fn contains_match(haystack: &str, needle: &str, whole_word: bool) -> bool {
    if !whole_word {
        return haystack.contains(needle);
    }
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        // Advance one byte past this start to find a later, better-bounded hit.
        from = start + 1;
        if from > haystack.len() {
            break;
        }
    }
    false
}
