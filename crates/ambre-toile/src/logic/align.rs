//! Align and distribute a selection (`toile-elements section 1.7`).
//!
//! This module is **pure and deterministic**: it takes the boxes of the selected
//! nodes, in selection order, and returns one translation per box. It never reads
//! the document and never writes it. The handler
//! (`state::elements::align_elements`) owns the boxes it feeds in and the
//! registers it writes, so the geometry can be tested without a CRDT.
//!
//! ## Semantics (pinned; the spec fixes the intent, not the arithmetic)
//!
//! - The **reference** is the union of the boxes handed in, never the viewport
//!   and never a frame. Aligning left puts every box on the left edge of what is
//!   selected, so the selection as a whole never moves.
//! - An **alignment** moves on one axis and leaves the other untouched.
//! - A **distribution** pins the two extreme boxes and spreads the ones between
//!   them so the gaps between consecutive boxes are equal. That is edge to
//!   edge, not centre to centre (see `toile-elements section 5`). The extremes
//!   are the boxes with the smallest and largest near edge, in the order fed
//!   in.
//! - **Fewer than two boxes** align to nothing, and **fewer than three** have
//!   nothing to distribute. Both answer all-zero translations, and the handler
//!   reads that as "write nothing".
//! - A selection whose boxes **overlap** (their extents exceed the span) yields
//!   a negative gap rather than pushing an extreme. Pinning the extremes is
//!   what makes the operation idempotent, and a user who overlaps three shapes
//!   gets an even overlap.

use ambre_core::{Board, Px};
use kurbo::{Rect, Vec2};

pub use crate::message::AlignOp;

/// One translation per input box, in the order fed in.
///
/// The returned vector always has `boxes.len()` entries; a box the operation
/// leaves alone gets [`Vec2::ZERO`]. See the module docs for the pinned
/// semantics of each [`AlignOp`].
pub fn align_deltas(boxes: &[Rect], op: AlignOp) -> Vec<Vec2> {
    let zero = vec![Vec2::ZERO; boxes.len()];
    let Some((first, rest)) = boxes.split_first().filter(|(_, rest)| !rest.is_empty()) else {
        return zero;
    };
    let union = rest.iter().fold(*first, |union, b| union.union(*b));
    match op {
        AlignOp::Left => shift(boxes, |b| Vec2::new(union.x0 - b.x0, 0.0)),
        AlignOp::HCenter => shift(boxes, |b| Vec2::new(union.center().x - b.center().x, 0.0)),
        AlignOp::Right => shift(boxes, |b| Vec2::new(union.x1 - b.x1, 0.0)),
        AlignOp::Top => shift(boxes, |b| Vec2::new(0.0, union.y0 - b.y0)),
        AlignOp::VCenter => shift(boxes, |b| Vec2::new(0.0, union.center().y - b.center().y)),
        AlignOp::Bottom => shift(boxes, |b| Vec2::new(0.0, union.y1 - b.y1)),
        AlignOp::DistributeH => distribute(boxes, union, Axis::Horizontal),
        AlignOp::DistributeV => distribute(boxes, union, Axis::Vertical),
    }
}

/// The axis a distribution works on.
#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// The near edge of a board box along the axis.
    fn near(self, r: &Rect) -> Px<Board> {
        Px::<Board>::new(match self {
            Self::Horizontal => r.x0,
            Self::Vertical => r.y0,
        })
    }

    /// The extent of a board box along the axis.
    fn extent(self, r: &Rect) -> Px<Board> {
        Px::<Board>::new(match self {
            Self::Horizontal => r.width(),
            Self::Vertical => r.height(),
        })
    }

    /// The translation that moves `d` along the axis and not at all across it.
    fn delta(self, d: Px<Board>) -> Vec2 {
        match self {
            Self::Horizontal => Vec2::new(d.get(), 0.0),
            Self::Vertical => Vec2::new(0.0, d.get()),
        }
    }
}

fn shift(boxes: &[Rect], delta: impl Fn(&Rect) -> Vec2) -> Vec<Vec2> {
    boxes.iter().map(delta).collect()
}

/// Pin the extremes and equalize the gaps between the boxes in between.
fn distribute(boxes: &[Rect], union: Rect, axis: Axis) -> Vec<Vec2> {
    if boxes.len() < 3 {
        return vec![Vec2::ZERO; boxes.len()];
    }
    // Selection order is not paint order and not screen order. Sort by near
    // edge so the walk lays the boxes out left to right (top to bottom).
    // Keep the original index to answer in the order fed in.
    let mut order: Vec<(usize, &Rect)> = boxes.iter().enumerate().collect();
    order.sort_by(|(_, a), (_, b)| {
        axis.near(a)
            .partial_cmp(&axis.near(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let span = axis.extent(&union);
    let occupied: Px<Board> = boxes.iter().map(|b| axis.extent(b)).sum();
    // A selection is far below 2^53 boxes, so the count is exact as a float.
    let gap = (span - occupied) / (boxes.len() - 1) as f64;

    let mut out = vec![Vec2::ZERO; boxes.len()];
    let mut cursor = order
        .first()
        .map_or(Px::<Board>::ZERO, |(_, b)| axis.near(b));
    for (i, b) in order {
        if let Some(slot) = out.get_mut(i) {
            *slot = axis.delta(cursor - axis.near(b));
        }
        cursor += axis.extent(b) + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, x + w, y + h)
    }

    /// Applying the answer to the input, the way the handler does.
    fn moved(boxes: &[Rect], op: AlignOp) -> Vec<Rect> {
        align_deltas(boxes, op)
            .iter()
            .zip(boxes)
            .map(|(d, b)| *b + *d)
            .collect()
    }

    /// `TOILE-277`: aligning left puts every box on the left edge of the union.
    #[test]
    fn aligning_left_puts_every_box_on_the_union_edge() {
        let boxes = [r(10.0, 0.0, 20.0, 10.0), r(50.0, 40.0, 5.0, 10.0)];
        let out = moved(&boxes, AlignOp::Left);
        assert_eq!(out[0].x0, 10.0);
        assert_eq!(out[1].x0, 10.0);
        // The other axis is untouched.
        assert_eq!(out[1].y0, 40.0);
    }

    /// `TOILE-277`: aligning on the horizontal centre puts every box on one
    /// centre line, and keeps each width.
    #[test]
    fn aligning_centers_shares_one_axis_and_keeps_the_widths() {
        let boxes = [r(0.0, 0.0, 100.0, 10.0), r(0.0, 20.0, 10.0, 10.0)];
        let out = moved(&boxes, AlignOp::HCenter);
        assert_eq!(out[0].center().x, out[1].center().x);
        assert_eq!(out[1].width(), 10.0);
    }

    /// `TOILE-278`: an alignment run a second time leaves every box where the
    /// first run put it.
    #[test]
    fn aligning_is_idempotent() {
        let boxes = [r(10.0, 0.0, 20.0, 10.0), r(50.0, 40.0, 5.0, 10.0)];
        let alignments = [
            AlignOp::Left,
            AlignOp::HCenter,
            AlignOp::Right,
            AlignOp::Top,
            AlignOp::VCenter,
            AlignOp::Bottom,
        ];
        for op in alignments {
            let once = moved(&boxes, op);
            assert_eq!(moved(&once, op), once, "{op:?} moved a box twice");
            // And the pass that changes nothing says so.
            assert!(
                align_deltas(&once, op).iter().all(|d| *d == Vec2::ZERO),
                "{op:?} answered a translation on its second run"
            );
        }
    }

    /// `TOILE-279`: a distribution equalizes the gaps, and the two extreme boxes
    /// keep their place.
    #[test]
    fn distributing_equalizes_the_gaps_and_pins_the_extremes() {
        // Widths 10, 30, 10 over a span of 100 leaves 50 for two gaps.
        let boxes = [
            r(0.0, 0.0, 10.0, 5.0),
            r(20.0, 0.0, 30.0, 5.0),
            r(90.0, 0.0, 10.0, 5.0),
        ];
        let out = moved(&boxes, AlignOp::DistributeH);
        assert_eq!(out[0].x0, 0.0);
        assert_eq!(out[2].x1, 100.0);
        assert_eq!(out[1].x0 - out[0].x1, out[2].x0 - out[1].x1);
        assert_eq!(out[1].x0, 35.0);
    }

    /// `TOILE-280`: a distribution orders the boxes by their place on the axis,
    /// never by selection order.
    #[test]
    fn distributing_reads_screen_order_not_selection_order() {
        let boxes = [
            r(90.0, 0.0, 10.0, 5.0),
            r(0.0, 0.0, 10.0, 5.0),
            r(20.0, 0.0, 30.0, 5.0),
        ];
        let out = moved(&boxes, AlignOp::DistributeH);
        // The extremes are the leftmost and rightmost box, whatever the order
        // the selection was built in.
        assert_eq!(out[1].x0, 0.0);
        assert_eq!(out[0].x1, 100.0);
        assert_eq!(out[2].x0, 35.0);
    }

    /// `TOILE-204`: a vertical distribution leaves every box where it was on the
    /// horizontal axis.
    #[test]
    fn distributing_on_the_vertical_axis_leaves_x_alone() {
        let boxes = [
            r(0.0, 0.0, 10.0, 10.0),
            r(7.0, 20.0, 10.0, 10.0),
            r(3.0, 90.0, 10.0, 10.0),
        ];
        let out = moved(&boxes, AlignOp::DistributeV);
        assert_eq!([out[0].x0, out[1].x0, out[2].x0], [0.0, 7.0, 3.0]);
        assert_eq!(out[1].y0 - out[0].y1, out[2].y0 - out[1].y1);
    }

    #[test]
    fn a_pair_has_nothing_to_distribute() {
        let boxes = [r(0.0, 0.0, 10.0, 10.0), r(90.0, 0.0, 10.0, 10.0)];
        assert!(
            align_deltas(&boxes, AlignOp::DistributeH)
                .iter()
                .all(|d| *d == Vec2::ZERO)
        );
    }

    #[test]
    fn a_lone_box_is_already_aligned() {
        let boxes = [r(4.0, 9.0, 10.0, 10.0)];
        assert_eq!(align_deltas(&boxes, AlignOp::Right), vec![Vec2::ZERO]);
        assert_eq!(align_deltas(&[], AlignOp::Top), Vec::<Vec2>::new());
    }

    /// `TOILE-279`: boxes that overlap get an even overlap, and the two extreme
    /// boxes keep their place.
    #[test]
    fn overlapping_boxes_get_an_even_overlap_rather_than_a_pushed_extreme() {
        // Three 50-wide boxes over a 60-wide span: the gap is negative.
        let boxes = [
            r(0.0, 0.0, 50.0, 5.0),
            r(5.0, 0.0, 50.0, 5.0),
            r(10.0, 0.0, 50.0, 5.0),
        ];
        let out = moved(&boxes, AlignOp::DistributeH);
        assert_eq!(out[0].x0, 0.0);
        assert_eq!(out[2].x1, 60.0);
        assert_eq!(out[1].x0 - out[0].x1, out[2].x0 - out[1].x1);
    }
}
