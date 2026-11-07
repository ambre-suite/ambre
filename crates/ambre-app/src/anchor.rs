//! The selection anchor, mirrored from the canvas into the chrome.
//!
//! The view publishes the active cell's screen position, and the frame loop
//! mirrors it into the chrome's `anchor_pos` signal. The floating selection
//! toolbar transform-syncs onto that signal.

use ambre_core::{Point, Px, View};
use leptos::prelude::*;

/// The mirror of the anchor: the view's publisher, the chrome's signal, and
/// the last position written.
pub(crate) struct AnchorMirror {
    publisher: ambre_canvas::AnchorPublisher,
    signal: RwSignal<Option<Point<Px<View>>>>,
    last: Option<Point<Px<View>>>,
}

impl AnchorMirror {
    /// A mirror that has written nothing yet.
    pub(crate) fn new(
        publisher: ambre_canvas::AnchorPublisher,
        signal: RwSignal<Option<Point<Px<View>>>>,
    ) -> Self {
        Self {
            publisher,
            signal,
            last: None,
        }
    }

    /// Mirror the view's published selection anchor into the chrome signal,
    /// only on change so the floating toolbar repositions just when it moves.
    pub(crate) fn step(&mut self) {
        let next = self.publisher.read();
        if next != self.last {
            self.last = next;
            self.signal.set(next);
        }
    }
}
