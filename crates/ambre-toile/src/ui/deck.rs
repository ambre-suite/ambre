// ambre-toile/src/ui/deck.rs
// ============================================================================
// The deck panel's pure model (toile-presentation). The engine can already write
// a slide sequence (`SetSlideOrder`), and this is what turns a click into one.
// Every edit here answers with the WHOLE new order, because that is the only
// shape the command takes. The deck is small, and a full rewrite converges where
// a positional patch would not.
//
// No DOM and no Loro, so each edit is an ordinary unit test. The wasm-only
// Leptos panel (`chrome::deck`) is a thin renderer over these functions and the
// signals declared at the bottom.
// ============================================================================

use ambre_bus::app::toile::{DeckFrame, FrameId};

/// What a deck row reads (`TOILE-UI-317`). The frame's title, or its rank when
/// the frame has never been named, so a deck of unnamed frames is still
/// navigable. `rank` is 0-based and shown 1-based, like the slide counter.
pub fn deck_label(frame: &DeckFrame, rank: usize) -> String {
    let name = frame.name.trim();
    if name.is_empty() {
        format!("Frame {}", rank + 1)
    } else {
        name.to_owned()
    }
}

/// Append `frame` to the deck, or answer `None` when the deck already names
/// it. The engine de-duplicates, and a no-op write would only cost an undo
/// item.
pub fn add(order: &[FrameId], frame: &FrameId) -> Option<Vec<FrameId>> {
    if order.contains(frame) {
        return None;
    }
    let mut next = order.to_vec();
    next.push(frame.clone());
    Some(next)
}

/// Drop the slide at `at` from the deck, or `None` when `at` names no slide.
/// The frame itself is untouched: leaving the deck is not a deletion.
pub fn remove(order: &[FrameId], at: usize) -> Option<Vec<FrameId>> {
    if at >= order.len() {
        return None;
    }
    let mut next = order.to_vec();
    next.remove(at);
    Some(next)
}

/// Swap the slide at `at` with its neighbour one step towards `to`
/// (`Move::Up` earlier, `Move::Down` later), or `None` when it is already at
/// that end. One swap is one `SetSlideOrder`, so one reorder is one undo item.
pub fn shift(order: &[FrameId], at: usize, to: Move) -> Option<Vec<FrameId>> {
    let other = match to {
        Move::Up => at.checked_sub(1)?,
        Move::Down => at + 1,
    };
    if at >= order.len() || other >= order.len() {
        return None;
    }
    let mut next = order.to_vec();
    next.swap(at, other);
    Some(next)
}

/// Which way a reorder moves a slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
}

/// What the deck panel asks the canvas view to do (`TOILE-FEAT-026`). The DOM
/// holds no board state, so every intent is drained by the view, which owns the
/// command channel. The view turns each into one `SetSlideOrder`, except
/// `Goto`, which is local playback.
#[derive(Clone, Debug, PartialEq)]
pub enum DeckIntent {
    /// Add a frame the deck does not name yet, at the end.
    Add(FrameId),
    /// Drop the slide at this rank.
    Remove(usize),
    /// Move the slide at this rank one step.
    Shift(usize, Move),
    /// Move this viewer's playback to a slide (`TOILE-FEAT-081`).
    Goto(usize),
}

/// The signals the canvas view and the DOM panel share (`toile-presentation`
/// section 4). The panel holds no board state of its own. The view writes what
/// it read back from Logic, and reads the intents the panel emits, exactly like
/// the comments panel.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct DeckSignals {
    /// Whether the panel is open (the Present menu sets it).
    pub panel: leptos::prelude::RwSignal<bool>,
    /// The board's frames, deck first: re-read on every deck change.
    pub view: leptos::prelude::RwSignal<ambre_bus::app::toile::DeckView>,
    /// The panel's next intent, drained by the view each frame.
    pub intent: leptos::prelude::RwSignal<Option<DeckIntent>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::ElementId;

    fn frame(id: &str, name: &str) -> DeckFrame {
        DeckFrame {
            id: ElementId(id.to_owned()),
            name: name.to_owned(),
        }
    }

    fn ids(names: &[&str]) -> Vec<FrameId> {
        names.iter().map(|n| ElementId((*n).to_owned())).collect()
    }

    #[test]
    fn an_unnamed_frame_reads_by_rank() {
        assert_eq!(deck_label(&frame("a", ""), 0), "Frame 1");
        assert_eq!(deck_label(&frame("a", "   "), 2), "Frame 3");
    }

    #[test]
    fn a_named_frame_reads_by_name() {
        assert_eq!(deck_label(&frame("a", "Agenda"), 4), "Agenda");
    }

    #[test]
    fn adding_appends_and_a_frame_already_on_the_deck_writes_nothing() {
        let order = ids(&["a", "b"]);
        assert_eq!(
            add(&order, &ElementId("c".to_owned())),
            Some(ids(&["a", "b", "c"]))
        );
        assert_eq!(add(&order, &ElementId("b".to_owned())), None);
    }

    #[test]
    fn removing_drops_that_rank_alone() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(remove(&order, 1), Some(ids(&["a", "c"])));
        assert_eq!(remove(&order, 3), None, "a rank past the deck is no slide");
    }

    #[test]
    fn shifting_swaps_with_the_neighbour() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(shift(&order, 1, Move::Up), Some(ids(&["b", "a", "c"])));
        assert_eq!(shift(&order, 1, Move::Down), Some(ids(&["a", "c", "b"])));
    }

    #[test]
    fn shifting_past_either_end_writes_nothing() {
        let order = ids(&["a", "b"]);
        assert_eq!(shift(&order, 0, Move::Up), None);
        assert_eq!(shift(&order, 1, Move::Down), None);
        assert_eq!(shift(&order, 7, Move::Up), None);
    }
}
