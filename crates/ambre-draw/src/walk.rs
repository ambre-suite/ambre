//! The one walk an application draws its document with (`CANVAS-074`).
//!
//! The screen and the page differ only in the surface they hand the walk and
//! in the transform they push before the call. The walk itself reads a view
//! of the document and a region of it, both in document coordinates. It reads
//! nothing else: no camera, no density, no hover, no frame clock. So a page
//! drawn from the walk shows what the screen shows, at any zoom and on any
//! screen (`CANVAS-029`).
//!
//! Screen-only layers (a selection, a caret, a grid, a handle) are drawn by
//! the screen before or after the walk, never inside it.

use crate::{DrawSurface, Rect};

/// An application's document walk.
///
/// The implementing type holds what the walk may keep between calls and
/// must not draw from: the shaping contexts and the caches built on them. The
/// view holds what is drawn. `crate_boundaries.py` holds both to geometry,
/// paint, typesetting and document types, and refuses a screen module
/// imported where the walk is implemented.
pub trait DocumentWalk {
    /// The document as the walk reads it: the content, its paint, and what
    /// this view of it leaves out.
    type View<'v>;

    /// Draw the part of `view` inside `region`, in document coordinates, onto
    /// `surface`. The caller pushes the transform from document coordinates to
    /// its own before the call, and pops it after.
    fn walk<S: DrawSurface>(&mut self, view: &Self::View<'_>, region: Rect, surface: &mut S);
}
