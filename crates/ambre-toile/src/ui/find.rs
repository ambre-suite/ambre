// ambre-toile/src/ui/find.rs
// ============================================================================
// Find-on-board view helper (toile-ui §5quater). The DOM `FindPanel` (chrome)
// owns the query and options; Logic runs the scan and replies with
// `FindResults { hits, focus }`, stepping `focus` forward or backward itself
// (toile-logic §8 quater). The panel only needs to know where `focus` sits in
// `hits` for its "k/n" counter -- that is this helper.
//
// Kept in the always-compiled `ui` module (not the wasm-only `chrome`) so it is
// host-unit-tested (§10): the chrome panel is Leptos/DOM and only reachable
// under the wasm target.
// ============================================================================

use ambre_bus::app::toile::ElementId;

/// Index of `focus` within `hits`, or `None` when there is no focus or it is no
/// longer a hit (a query change can drop the previously focused element). The
/// panel renders `active + 1` / `hits.len()` as its match counter.
pub fn active_index(hits: &[ElementId], focus: Option<&ElementId>) -> Option<usize> {
    let focus = focus?;
    hits.iter().position(|h| h == focus)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<ElementId> {
        names.iter().map(|n| ElementId((*n).to_owned())).collect()
    }

    #[test]
    fn active_index_locates_focus_or_none() {
        let hits = ids(&["a", "b", "c"]);
        assert_eq!(active_index(&hits, Some(&ElementId("b".into()))), Some(1));
        assert_eq!(active_index(&hits, None), None);
        // A focus no longer in the hit set (query changed) reads as no index.
        assert_eq!(active_index(&hits, Some(&ElementId("z".into()))), None);
        assert_eq!(active_index(&[], Some(&ElementId("a".into()))), None);
    }
}
