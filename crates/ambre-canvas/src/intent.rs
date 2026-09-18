// ambre-canvas/src/intent.rs
// ============================================================================
// One-shot UI intents: canvas->DOM channel for "open a floating chrome surface
// now" pulses (`trame-shared-filter`). Unlike `AnchorPublisher`, which the view
// re-publishes every frame and the chrome reads non-destructively, these carry
// a single edge. The view raises the flag on a click. The launcher consumes it
// once per frame and resets it.
//
// First user: the in-canvas autofilter button. A left-click on the button can
// not reach the DOM `FilterPopover` directly (the view only owns canvas input).
// It selects the button's column and raises this flag. The launcher then opens
// the popover through the same path as the context-menu entry.
//
// Pure value plumbing (an `Rc<Cell<bool>>`): compiles on every target, depends
// on no UI framework. Lives on the UI thread only, so it stays `!Send`.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

/// One-slot, take-once channel: the view raises the flag (`publish`), the chrome
/// consumes it once per frame (`take`, which resets it). Cheap to clone (shared
/// `Rc<Cell>`).
#[derive(Clone, Default)]
pub struct FilterOpenPublisher {
    slot: Rc<Cell<bool>>,
}

impl FilterOpenPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Raise the "open the filter popover" intent.
    pub fn publish(&self) {
        self.slot.set(true);
    }

    /// Consume the intent, returning `true` exactly once per `publish`.
    pub fn take(&self) -> bool {
        self.slot.replace(false)
    }
}
