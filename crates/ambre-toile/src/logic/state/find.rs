//! Board text search for [`ToileState`](super::ToileState).
//!
//! `Find` is **non-mutating** and runs entirely on the Logic thread. The UI only
//! holds the culled `DisplayViewport`, so a match on an off-screen sticky is
//! invisible to it. The handler walks the text-bearing elements (sticky / text
//! bodies, frame titles) in z-order and returns their ids plus the next hit to
//! recenter on. Matches are local view state, never synced and never an undo
//! item.

use ambre_core::StepDirection;

use crate::logic::command::{FindOptions, FindQuery, FindResults, ToileEvent, ToileOutcome};
use crate::logic::model::NodeId;
use ambre_core::id::{Element, Id};

use super::ToileState;
use super::codec::has_type;
use super::codec::{body_of, str_of};
use super::names::ElementKey;
use crate::logic::model::ElementKind;

impl ToileState {
    /// Scan the text-bearing elements for `query`, returning the matching ids in
    /// z-order and the recenter target. Read-only: no CRDT mutation,
    /// no sync payload. No query yields no hits (the UI's "clear" signal).
    pub(super) fn find(
        &self,
        query: Option<&FindQuery>,
        from: Option<&Id<Element>>,
        direction: StepDirection,
    ) -> ToileOutcome {
        let hits = query.map_or_else(Vec::new, |query| self.find_hits(query));
        let focus = next_focus(&hits, from, direction);
        ToileOutcome::display_only(vec![ToileEvent::FindResults(FindResults { hits, focus })])
    }

    /// Text-bearing element ids matching `query`, in paint order. The
    /// `layers` walk comes first, then any element absent from `layers`,
    /// appended in `Id<Element>` order (the well-formedness fallback). A
    /// concurrently dropped `layers` entry therefore never hides a hit.
    fn find_hits(&self, query: &FindQuery) -> Vec<Id<Element>> {
        let mut hits = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for node in &self.layers {
            if let NodeId::Element(id) = node
                && seen.insert(id.clone())
                && self.element_matches(id, query)
            {
                hits.push(id.clone());
            }
        }
        // Fallback: text-bearing elements missing from `layers`, id-ordered.
        let mut orphans: Vec<Id<Element>> = self
            .bounds
            .keys()
            .filter(|id| !seen.contains(*id))
            .filter(|id| self.element_matches(id, query))
            .cloned()
            .collect();
        orphans.sort();
        hits.extend(orphans);
        hits
    }

    /// Whether the element's searchable text (sticky / text body, frame title)
    /// matches `query` under the given options. Non-text elements never match.
    fn element_matches(&self, id: &Id<Element>, query: &FindQuery) -> bool {
        let Some(em) = self.element_map(id) else {
            return false;
        };
        let body = if has_type(&em, ElementKind::Sticky) || has_type(&em, ElementKind::Text) {
            // A text element that stores no body holds no text to match.
            body_of(&em).unwrap_or_default()
        } else if has_type(&em, ElementKind::Frame) {
            // A frame title lives in its `name` field (empty until named).
            str_of(&em, ElementKey::Name).unwrap_or_default()
        } else {
            return false;
        };
        text_matches(&body, query)
    }
}

/// The hit to recenter on. That is the first hit for a fresh search, otherwise
/// the hit adjacent to `from` in the requested direction, wrapping around.
/// A `Backward` direction steps to the previous hit, else the next. A `from`
/// that is no longer a hit restarts at the first.
fn next_focus(
    hits: &[Id<Element>],
    from: Option<&Id<Element>>,
    direction: StepDirection,
) -> Option<Id<Element>> {
    let first = hits.first().cloned();
    let Some(from) = from else {
        return first;
    };
    let len = hits.len();
    match (hits.iter().position(|h| h == from), direction) {
        // `+ len` before subtracting keeps the step unsigned and wraps correctly
        // for the backward case at index 0.
        (Some(idx), StepDirection::Backward) => hits.get((idx + len - 1) % len).cloned(),
        (Some(idx), StepDirection::Forward) => hits.get((idx + 1) % len).cloned(),
        (None, _) => first,
    }
}

/// Substring test of the query text, honoring its case and word options. Case
/// folding is a plain lowercase on both sides (adequate for the Latin-script
/// MVP).
fn text_matches(haystack: &str, query: &FindQuery) -> bool {
    let options = query.options;
    // The matcher below runs on raw or case-folded text alike, so the newtype
    // ends here.
    let needle = query.text.as_str();
    if needle.is_empty() {
        return false;
    }
    if options.case == ambre_core::CaseMatch::Sensitive {
        contains_match(haystack, needle, options)
    } else {
        contains_match(&haystack.to_lowercase(), &needle.to_lowercase(), options)
    }
}

/// `haystack.contains(needle)`, refined to word boundaries when the options
/// ask for whole words. The case folding is the caller's.
fn contains_match(haystack: &str, needle: &str, options: FindOptions) -> bool {
    if options.word == ambre_core::WordMatch::Anywhere {
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
