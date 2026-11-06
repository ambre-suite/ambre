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
// Pure value plumbing (an `Rc<Cell<BusyLevel>>`) that compiles on every
// target, reaches no UI framework, and stays `!Send` on the UI thread.
//
// The level is a type of this crate. The renderer knows no thread protocol
// (`ARCH-071`), so it takes the outbox's answer to `is_busy` as a plain
// predicate, through `BusyLevel::from_busy`. That one constructor is the whole
// seam: no view writes a conversion of its own.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

/// Whether a view can take more input (`BUS-016`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BusyLevel {
    /// The view takes input.
    #[default]
    Free,
    /// The view refuses input: its outbox holds a command the Logic thread
    /// has not taken.
    Busy,
}

impl BusyLevel {
    /// The level of an outbox that answers `busy` to `is_busy` (`BUS-016`).
    pub fn from_busy(busy: bool) -> Self {
        if busy { Self::Busy } else { Self::Free }
    }
}

/// The level signal a view uses to say it cannot take more input.
///
/// The view calls [`publish`](Self::publish) from its frame hook; the chrome
/// reads [`is_busy`](Self::is_busy) and shows the notice. Cheap to clone (shared
/// `Rc<Cell>`).
#[derive(Clone, Default)]
pub struct BusyPublisher {
    slot: Rc<Cell<BusyLevel>>,
}

impl BusyPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// State the current level. Returns `true` when the level changed, so the
    /// chrome raises one notice per episode instead of one per frame.
    pub fn publish(&self, level: BusyLevel) -> bool {
        self.slot.replace(level) != level
    }

    /// Whether the view is refusing input right now.
    pub fn is_busy(&self) -> bool {
        self.slot.get() == BusyLevel::Busy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_frame_of_an_episode_reports_a_change() {
        let busy = BusyPublisher::new();
        assert!(!busy.is_busy());
        assert!(busy.publish(BusyLevel::Busy));
        assert!(!busy.publish(BusyLevel::Busy));
        assert!(busy.is_busy());
        assert!(busy.publish(BusyLevel::Free));
        assert!(!busy.publish(BusyLevel::Free));
    }

    #[test]
    fn a_busy_outbox_reads_as_a_busy_level() {
        let outbox_busy = [false, true];
        let levels = outbox_busy.map(BusyLevel::from_busy);
        assert_eq!(levels, [BusyLevel::Free, BusyLevel::Busy]);
    }
}
