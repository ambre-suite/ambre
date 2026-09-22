//! Pagination: the document laid out on measured pages
//! (`features/ambre-spec-prose-pagination.md`).
//!
//! Everything here runs in the Logic thread (`PROSE-010`) and only reads the
//! document (`PROSE-079`). Nothing it computes is written back, so a page
//! number, a break position and a measured height exist here alone
//! (`PROSE-033`, `PROSE-065`).
//!
//! * [`geometry`] -- the page box and the two bands cut out of it.
//! * [`fonts`] -- every family label resolved to a face the shaper holds.
//! * [`measure`] -- the effective style of one block, and the height its
//!   shaped lines take.
//! * [`place`] -- measured blocks laid onto pages, under the constraint order.
//! * [`pass`] -- the walk that measures the whole document, and the layout it
//!   assembles.
//! * [`reflow`] -- the layout held between edits, and the reflow that updates
//!   it in place.

pub mod fonts;
pub mod geometry;
pub mod measure;
pub mod pass;
pub mod place;
pub mod reflow;
