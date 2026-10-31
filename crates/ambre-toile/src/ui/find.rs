// ambre-toile/src/ui/find.rs
// ============================================================================
// Find-on-board view helper (toile-find). The DOM `FindPanel` (chrome)
// owns the query and options; Logic runs the scan and replies with
// `FindResults { hits, focus }`, stepping `focus` forward or backward itself
// (toile-find). The panel only needs to know where `focus` sits in
// `hits` for its "k/n" counter -- that is this helper.
//
// Kept in the always-compiled `ui` module, not the wasm-only `chrome`, so it is
// host-unit-tested. The chrome panel is Leptos/DOM and only reachable under the
// wasm target.
// ============================================================================

use ambre_bus::app::find::MatchIndex;
use ambre_commands::{Commands, StaticKey};
use ambre_core::id::{Element, Id};

/// Open the Find panel. A UI-only intent: the launcher intercepts it to toggle
/// the panel open rather than dispatching to Logic (the panel emits
/// `ToileCommand::Find` itself).
///
/// Declared here rather than in the chrome, which is wasm-only: an identity the
/// registry describes must be reachable from a host test
/// (`ambre-spec-commands.md`).
pub const FIND: StaticKey<Commands> = StaticKey::<Commands>::new("toile.find.open");

/// Index of `focus` within `hits`, or `None` when there is no focus or it is no
/// longer a hit. A query change can drop the previously focused element. The
/// panel renders the index's ordinal over `hits.len()` as its match counter.
pub fn active_index(hits: &[Id<Element>], focus: Option<&Id<Element>>) -> Option<MatchIndex> {
    let focus = focus?;
    hits.iter()
        .position(|h| h == focus)
        .map(MatchIndex::from_usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<Id<Element>> {
        names
            .iter()
            .map(|n| Id::<Element>::from_text((*n).to_owned()))
            .collect()
    }

    #[test]
    fn active_index_locates_focus_or_none() {
        let hits = ids(&["a", "b", "c"]);
        assert_eq!(
            active_index(&hits, Some(&Id::<Element>::from_text("b".into()))),
            Some(MatchIndex::new(1))
        );
        assert_eq!(active_index(&hits, None), None);
        // A focus no longer in the hit set (query changed) reads as no index.
        assert_eq!(
            active_index(&hits, Some(&Id::<Element>::from_text("z".into()))),
            None
        );
        assert_eq!(
            active_index(&[], Some(&Id::<Element>::from_text("a".into()))),
            None
        );
    }
}
