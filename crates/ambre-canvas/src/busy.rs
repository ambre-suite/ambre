// ambre-canvas/src/busy.rs
// ============================================================================
// The busy signal a view raises when its lossless outbox holds a command the
// Logic thread has not taken (`BUS-016`).
//
// A saturated Logic thread used to be invisible: the view logged a line and the
// user kept typing into a document that received nothing. The signal is a level
// rather than a pulse: it stays on for as long as the outbox holds anything. So
// the view restates it every frame and the chrome reads it without consuming it,
// like `AnchorPublisher` and unlike the one-shot `FilterOpenPublisher`.
//
// Pure value plumbing (an `Rc<Cell<bool>>`) that compiles on every target,
// reaches no UI framework, and stays `!Send` on the UI thread.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

/// The level signal a view uses to say it cannot take more input.
///
/// The view calls [`publish`](Self::publish) from its frame hook; the chrome
/// reads [`is_busy`](Self::is_busy) and shows the notice. Cheap to clone (shared
/// `Rc<Cell>`).
#[derive(Clone, Default)]
pub struct BusyPublisher {
    slot: Rc<Cell<bool>>,
}

impl BusyPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// State the current level. Returns `true` when the level changed, so the
    /// chrome raises one notice per episode instead of one per frame.
    pub fn publish(&self, busy: bool) -> bool {
        self.slot.replace(busy) != busy
    }

    /// Whether the view is refusing input right now.
    pub fn is_busy(&self) -> bool {
        self.slot.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_frame_of_an_episode_reports_a_change() {
        let busy = BusyPublisher::new();
        assert!(!busy.is_busy());
        assert!(busy.publish(true));
        assert!(!busy.publish(true));
        assert!(busy.is_busy());
        assert!(busy.publish(false));
        assert!(!busy.publish(false));
    }
}
