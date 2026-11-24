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

use crate::message::{DeckFrame, Slide};
use ambre_core::id::{Frame, Id};
use ambre_core::index::Pos;

/// What a deck row reads (`TOILE-UI-317`). The frame's title, or its rank when
/// the frame has never been named, so a deck of unnamed frames is still
/// navigable. `rank` is 0-based and shown 1-based, like the slide counter.
pub fn deck_label(frame: &DeckFrame, rank: Pos<Slide>) -> String {
    let name = frame.name.trim();
    if name.is_empty() {
        format!("Frame {}", rank.number())
    } else {
        name.to_owned()
    }
}

/// Append `frame` to the deck, or answer `None` when the deck already names
/// it. The engine de-duplicates, and a no-op write would only cost an undo
/// item.
pub fn add(order: &[Id<Frame>], frame: &Id<Frame>) -> Option<Vec<Id<Frame>>> {
    if order.contains(frame) {
        return None;
    }
    let mut next = order.to_vec();
    next.push(frame.clone());
    Some(next)
}

/// Drop the slide at `at` from the deck, or `None` when `at` names no slide.
/// The frame itself is untouched: leaving the deck is not a deletion.
pub fn remove(order: &[Id<Frame>], at: Pos<Slide>) -> Option<Vec<Id<Frame>>> {
    let at = at.to_index().filter(|at| *at < order.len())?;
    let mut next = order.to_vec();
    next.remove(at);
    Some(next)
}

/// Swap the slide at `at` with its neighbour one step towards `to`
/// (`Move::Up` earlier, `Move::Down` later), or `None` when it is already at
/// that end. One swap is one `SetSlideOrder`, so one reorder is one undo item.
pub fn shift(order: &[Id<Frame>], at: Pos<Slide>, to: Move) -> Option<Vec<Id<Frame>>> {
    let other = match to {
        Move::Up => at.prev()?,
        Move::Down => at.next(),
    };
    let len = order.len();
    let at = at.to_index().filter(|at| *at < len)?;
    let other = other
        .to_index()
        .filter(|other| *other < len && *other != at)?;
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
/// command queue. The view turns each into one `SetSlideOrder`, except
/// `Goto`, which is local playback.
#[derive(Clone, PartialEq)]
pub enum DeckIntent {
    /// Add a frame the deck does not name yet, at the end.
    Add(Id<Frame>),
    /// Drop the slide at this rank.
    Remove(Pos<Slide>),
    /// Move the slide at this rank one step.
    Shift(Pos<Slide>, Move),
    /// Move this viewer's playback to a slide (`TOILE-FEAT-081`).
    Goto(Pos<Slide>),
    /// Name a frame, deck member or not (`TOILE-247`). It names the frame
    /// rather than a rank, because the panel's second list holds frames no
    /// rank addresses.
    Rename(Id<Frame>, String),
    /// Select a frame, from the frames section of the context panel
    /// (`CHROME-251`).
    Select(Id<Frame>),
}

/// A frame name prints as its length alone (`ZK-006`).
impl std::fmt::Debug for DeckIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add(frame) => f.debug_tuple("Add").field(frame).finish(),
            Self::Remove(at) => f.debug_tuple("Remove").field(at).finish(),
            Self::Shift(at, step) => f.debug_tuple("Shift").field(at).field(step).finish(),
            Self::Goto(at) => f.debug_tuple("Goto").field(at).finish(),
            Self::Rename(frame, name) => f
                .debug_tuple("Rename")
                .field(frame)
                .field(&ambre_core::Redacted::new(name))
                .finish(),
            Self::Select(frame) => f.debug_tuple("Select").field(frame).finish(),
        }
    }
}

/// The signals the canvas view and the DOM panel share (`toile-presentation`
/// section 4). The panel holds no board state of its own. The view writes what
/// it read back from Logic, and reads the intents the panel emits, exactly like
/// the comments panel.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct DeckSignals {
    /// Whether the panel is open (the Present menu sets it).
    pub panel: leptos::prelude::RwSignal<super::PanelState>,
    /// The board's frames, deck first: re-read on every deck change.
    pub view: leptos::prelude::RwSignal<crate::message::DeckView>,
    /// The panel's next intent, drained by the view each frame.
    pub intent: leptos::prelude::RwSignal<Option<DeckIntent>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TOILE-289`: a rename's debug form prints the name's length, none of it.
    #[test]
    fn a_rename_debug_form_prints_no_name() {
        let form = format!(
            "{:?}",
            DeckIntent::Rename(
                Id::<Frame>::from_text("f".to_owned()),
                "Q3 layoffs".to_owned()
            )
        );
        assert!(!form.contains("layoffs"), "{form}");
        assert!(form.contains(", 10 chars)"), "{form}");
    }

    fn frame(id: &str, name: &str) -> DeckFrame {
        DeckFrame {
            id: Id::<Frame>::from_text(id.to_owned()),
            name: name.to_owned(),
            items: ambre_core::Count::<crate::message::Elements>::new(0),
        }
    }

    fn rank(at: u32) -> Pos<Slide> {
        Pos::<Slide>::new(at)
    }

    fn ids(names: &[&str]) -> Vec<Id<Frame>> {
        names
            .iter()
            .map(|n| Id::<Frame>::from_text((*n).to_owned()))
            .collect()
    }

    #[test]
    fn an_unnamed_frame_reads_by_rank() {
        assert_eq!(deck_label(&frame("a", ""), Pos::<Slide>::new(0)), "Frame 1");
        assert_eq!(
            deck_label(&frame("a", "   "), Pos::<Slide>::new(2)),
            "Frame 3"
        );
    }

    #[test]
    fn a_named_frame_reads_by_name() {
        assert_eq!(
            deck_label(&frame("a", "Agenda"), Pos::<Slide>::new(4)),
            "Agenda"
        );
    }

    #[test]
    fn adding_appends_and_a_frame_already_on_the_deck_writes_nothing() {
        let order = ids(&["a", "b"]);
        assert_eq!(
            add(&order, &Id::<Frame>::from_text("c".to_owned())),
            Some(ids(&["a", "b", "c"]))
        );
        assert_eq!(add(&order, &Id::<Frame>::from_text("b".to_owned())), None);
    }

    #[test]
    fn removing_drops_that_rank_alone() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(remove(&order, rank(1)), Some(ids(&["a", "c"])));
        assert_eq!(
            remove(&order, rank(3)),
            None,
            "a rank past the deck is no slide"
        );
    }

    #[test]
    fn shifting_swaps_with_the_neighbour() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(
            shift(&order, rank(1), Move::Up),
            Some(ids(&["b", "a", "c"]))
        );
        assert_eq!(
            shift(&order, rank(1), Move::Down),
            Some(ids(&["a", "c", "b"]))
        );
    }

    #[test]
    fn shifting_past_either_end_writes_nothing() {
        let order = ids(&["a", "b"]);
        assert_eq!(shift(&order, rank(0), Move::Up), None);
        assert_eq!(shift(&order, rank(1), Move::Down), None);
        assert_eq!(shift(&order, rank(7), Move::Up), None);
    }
}
