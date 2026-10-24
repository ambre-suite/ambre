// ambre-canvas/src/anchor.rs
// ============================================================================
// Selection anchor: the canvas->DOM signal for floating chrome glued to content
// that pans and zooms with it (`ambre-spec-ui.md`, the canvas/DOM boundary).
//
// The canvas view owns the scroll/zoom and the selection, so it -- and only it
// -- knows where the selection sits on screen. It publishes that point each
// frame; the chrome mirrors it into a reactive signal and transform-syncs a
// floating toolbar onto it. As the viewport pans, the published point moves and
// the DOM element follows by `transform` alone (no layout, no per-frame DOM
// reads in the renderer).
//
// Pure value plumbing (an `Rc<Cell>`): compiles on every target, depends on no
// UI framework -- the symmetric counterpart of `TextInput` for output.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

use ambre_core::ViewPoint;

/// A point relative to the canvas content cell's top-left, that the chrome
/// can glue floating DOM UI to. Conventionally the horizontal centre and top
/// edge of the selection, so a toolbar centres above it.
pub type Anchor = ViewPoint;

/// One-slot signal: the view writes the current selection anchor (or `None` to
/// hide), the chrome reads it from its per-frame pump. Cheap to clone (shared
/// `Rc<Cell>`); lives on the UI thread only, so it stays `!Send`.
#[derive(Clone, Default)]
pub struct AnchorPublisher {
    slot: Rc<Cell<Option<Anchor>>>,
}

impl AnchorPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the current anchor (`None` hides the floating UI).
    pub fn publish(&self, anchor: Option<Anchor>) {
        self.slot.set(anchor);
    }

    /// Read the latest published anchor.
    pub fn read(&self) -> Option<Anchor> {
        self.slot.get()
    }
}
