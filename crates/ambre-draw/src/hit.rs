//! Content hit-testing, which the drawing surface owns (`UI-THREAD-001`).
//!
//! Each application's view state implements [`HitTest`] over its own layout.
//! The host reads the pointer off the canvas and hands the view a point; the
//! view alone turns it into content. No interface element reads the canvas
//! pointer, so none can rebuild a hit-test from its own geometry.

/// The content under a point of the canvas.
pub trait HitTest {
    /// What a point lands on: a place in the text, a cell, an element.
    type Target;

    /// The content under `at`, a point of the canvas. `None` over no content.
    fn hit_test(
        &self,
        at: ambre_core::Point<ambre_core::Px<ambre_core::View>>,
    ) -> Option<Self::Target>;
}
