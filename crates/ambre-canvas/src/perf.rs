// ambre-canvas/src/perf.rs
// ============================================================================
// Perf snapshot: the canvas->DOM channel for the performance HUD (fps + frame
// timings), the symmetric counterpart of `AnchorPublisher` for debug metrics.
//
// The perf HUD used to be shaped as glyphs on the wgpu overlay every frame --
// pure waste for a developer read-out. The canvas view now publishes a
// `PerfSnapshot` each frame; the DOM status bar mirrors it into a reactive
// signal and renders it as plain text (an `fps` chip plus a collapsible panel).
//
// Pure value plumbing (an `Rc<Cell>`): compiles on every target, depends on no
// UI framework.
// ============================================================================

use std::cell::Cell;
use std::rc::Rc;

use crate::view::PerfSnapshot;

/// One-slot channel: the view writes the current frame's perf snapshot, the
/// chrome reads it from its per-frame pump. Cheap to clone (shared `Rc<Cell>`);
/// lives on the UI thread only, so it stays `!Send`.
#[derive(Clone, Default)]
pub struct PerfPublisher {
    slot: Rc<Cell<PerfSnapshot>>,
}

impl PerfPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the latest per-frame perf snapshot.
    pub fn publish(&self, snapshot: PerfSnapshot) {
        self.slot.set(snapshot);
    }

    /// Read the latest published snapshot.
    pub fn read(&self) -> PerfSnapshot {
        self.slot.get()
    }
}
