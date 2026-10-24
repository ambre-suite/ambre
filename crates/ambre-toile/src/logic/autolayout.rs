//! Auto-layout constraint solver (`toile-auto-layout`).
//!
//! A **frame** can carry an [`AutoLayout`] spec so its children are positioned by
//! this solver rather than by absolute coordinates (the Figma model). The **spec is
//! CRDT**: a single LWW register under the frame element map's `autolayout` key,
//! shared design intent (see `state::autolayout`). The **solved positions are
//! derived, never persisted**, like connector geometry (`toile-connectors`).
//! Persisting solved positions is the rejected alternative: redundant derived state
//! that could desync from the spec.
//!
//! This module is **pure and deterministic**. Given the frame rect, the layout spec
//! and the children sizes (in `children` LoroMovableList order), it returns one
//! [`kurbo::Rect`] per child. Determinism is mandatory: every peer derives
//! identical positions from identical CRDT state. It runs at derivation on every
//! child edit, so it stays allocation-light for the < 1 ms / 200-child budget.
//!
//! ## Semantics (pinned; the spec leaves the fine details to the solver)
//!
//! - **Content box** = `frame` inset by `padding`, one length a side.
//!   Children lay out inside it; the origin is the content-box top-left.
//! - **Main axis** = X for [`Direction::Horizontal`] / [`Direction::Wrap`],
//!   Y for [`Direction::Vertical`]. The **cross axis** is the other one.
//! - **`gap`** = space between consecutive children on the main axis (and between
//!   rows for `Wrap`).
//! - **[`Align`]** distributes children along the **main axis** in the free
//!   space. `Start` packs at the start, `Center` centers the packed block and
//!   `End` packs at the end. `SpaceBetween` pins first/last to the edges and
//!   splits the remaining space evenly between them (falls back to `Start` for
//!   a run of `< 2` children).
//! - **[`Sizing`]** governs the **main-axis** child extent. `Fixed` / `Hug`
//!   keep each child's own size. From the solver's view Hug == Fixed, since
//!   the frame rect is an input, not something the solver grows. `Fill` grows
//!   children equally to consume the leftover main-axis space, and stretches
//!   them to the content-box extent on the cross axis. `Fill` supersedes
//!   `SpaceBetween` when both are set (no leftover space remains to
//!   distribute).
//! - **Cross axis**: children keep their own cross size and are
//!   **start-aligned**, the single [`Align`] knob being spent on the main axis.
//!   `Fill` also stretches the cross size to the content box.
//! - **Edge cases**: empty `children` -> empty `Vec`. When children overflow
//!   the box, distribution never produces negative spacing (clamped to `>= 0`),
//!   and the children simply extend past the box.

use kurbo::{Rect, Size};

// The auto-layout spec value types (`AutoLayout`, `Direction`, `Align`,
// `Sizing`) live in `ambre-bus` so `ToileCommand::SetAutoLayout` can carry them
// across `AppCommand::Toile` without a dependency cycle (mirrors `model.rs`).
// This module owns the pure `solve_layout` solver over them.
pub use ambre_bus::app::toile::{Align, AutoLayout, Direction, Padding, Sizing};

/// Solve the auto-layout of `frame` for the given `children` sizes (in
/// `children` order), returning one world-space [`Rect`] per child.
///
/// Pure and deterministic. See the module docs for the pinned semantics of each
/// [`Direction`] / [`Align`] / [`Sizing`] combination.
pub fn solve_layout(frame: Rect, layout: &AutoLayout, children: &[Size]) -> Vec<Rect> {
    if children.is_empty() {
        return Vec::new();
    }

    // Content box: the frame inset by the padding.
    let pad = layout.padding;
    let content = Rect::new(
        frame.x0 + pad.left.get(),
        frame.y0 + pad.top.get(),
        frame.x1 - pad.right.get(),
        frame.y1 - pad.bottom.get(),
    );

    match layout.direction {
        Direction::Horizontal => solve_axis(content, layout, children, Axis::X),
        Direction::Vertical => solve_axis(content, layout, children, Axis::Y),
        Direction::Wrap => solve_wrap(content, layout, children),
    }
}

/// The main axis of a single-line layout.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Lay out a single run of `children` along `axis` inside `content`. Shared by
/// [`Direction::Horizontal`] / [`Direction::Vertical`] and by each `Wrap` row
/// (with a per-row `content` band).
fn solve_axis(content: Rect, layout: &AutoLayout, children: &[Size], axis: Axis) -> Vec<Rect> {
    // Main / cross extents of a child size, and of the content box.
    let main_of = |s: Size| match axis {
        Axis::X => s.width,
        Axis::Y => s.height,
    };
    let cross_of = |s: Size| match axis {
        Axis::X => s.height,
        Axis::Y => s.width,
    };
    let (content_main, content_cross) = match axis {
        Axis::X => (content.width(), content.height()),
        Axis::Y => (content.height(), content.width()),
    };

    let n = children.len();
    let fill = layout.sizing == Sizing::Fill;
    // The gap in the float kurbo lays the rects out in.
    let gap = layout.gap.get();

    // Main-axis size of each child: Fill shares the leftover equally, otherwise
    // each keeps its own size.
    let total_gap = gap * (n.saturating_sub(1)) as f64;
    let main_sizes: Vec<f64> = if fill {
        let intrinsic: f64 = children.iter().map(|s| main_of(*s)).sum();
        let leftover = (content_main - total_gap - intrinsic).max(0.0);
        let extra = leftover / n as f64;
        children.iter().map(|s| main_of(*s) + extra).collect()
    } else {
        children.iter().map(|s| main_of(*s)).collect()
    };

    // Main-axis start offset and inter-child spacing, from the alignment. Fill
    // consumes all leftover, so its distribution is always packed from the start.
    let used: f64 = main_sizes.iter().sum::<f64>() + total_gap;
    let free = (content_main - used).max(0.0);
    let (mut cursor, spacing) = if fill {
        (0.0, gap)
    } else {
        match layout.align {
            Align::Start => (0.0, gap),
            Align::Center => (free / 2.0, gap),
            Align::End => (free, gap),
            Align::SpaceBetween => {
                if n < 2 {
                    (0.0, gap)
                } else {
                    // First at start, last at end: spread the free space across
                    // the (n - 1) interior gaps, on top of the base gap.
                    (0.0, gap + free / (n - 1) as f64)
                }
            }
        }
    };

    children
        .iter()
        .zip(main_sizes)
        .map(|(size, main)| {
            // Fill stretches a child across, but never shrinks it below its
            // own cross size (`TOILE-FEAT-045`).
            let cross = if fill {
                content_cross.max(cross_of(*size))
            } else {
                cross_of(*size)
            };
            let rect = match axis {
                Axis::X => Rect::new(
                    content.x0 + cursor,
                    content.y0,
                    content.x0 + cursor + main,
                    content.y0 + cross,
                ),
                Axis::Y => Rect::new(
                    content.x0,
                    content.y0 + cursor,
                    content.x0 + cross,
                    content.y0 + cursor + main,
                ),
            };
            cursor += main + spacing;
            rect
        })
        .collect()
}

/// Horizontal wrapping flow: pack children left-to-right, breaking to a new
/// row when the next child would overflow the content width. Rows stack
/// top-to-bottom separated by `gap`; each row's band height is its tallest child,
/// and [`Align`] applies per row on the main axis.
fn solve_wrap(content: Rect, layout: &AutoLayout, children: &[Size]) -> Vec<Rect> {
    // Partition the children into rows by greedy width packing. Indices are kept
    // so the flat output preserves `children` order.
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut row_main = 0.0_f64;
    for (i, s) in children.iter().enumerate() {
        let next = if row.is_empty() {
            s.width
        } else {
            row_main + layout.gap.get() + s.width
        };
        if !row.is_empty() && next > content.width() {
            rows.push(std::mem::take(&mut row));
            row_main = s.width;
        } else {
            row_main = next;
        }
        row.push(i);
    }
    if !row.is_empty() {
        rows.push(row);
    }

    let mut out = vec![Rect::ZERO; children.len()];
    let mut y = content.y0;
    for row in rows {
        let row_sizes: Vec<Size> = row
            .iter()
            .filter_map(|&i| children.get(i).copied())
            .collect();
        let row_height = row_sizes.iter().map(|s| s.height).fold(0.0_f64, f64::max);
        // Solve this row in a one-row-tall band; Fill stretches to that band.
        let band = Rect::new(content.x0, y, content.x1, y + row_height);
        let solved = solve_axis(band, layout, &row_sizes, Axis::X);
        for (&i, r) in row.iter().zip(solved) {
            if let Some(slot) = out.get_mut(i) {
                *slot = r;
            }
        }
        y += row_height + layout.gap.get();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, x + w, y + h)
    }

    fn size(w: f64, h: f64) -> Size {
        Size::new(w, h)
    }

    /// A frame with no padding, gap g, given direction/align/sizing.
    fn layout(direction: Direction, gap: f64, align: Align, sizing: Sizing) -> AutoLayout {
        AutoLayout {
            direction,
            gap: ambre_core::BoardLength::new(gap),
            padding: Padding::ZERO,
            align,
            sizing,
        }
    }

    #[test]
    fn horizontal_start_fixed_with_gap() {
        let frame = rect(0.0, 0.0, 200.0, 100.0);
        let l = layout(Direction::Horizontal, 5.0, Align::Start, Sizing::Fixed);
        let children = [size(20.0, 10.0), size(30.0, 40.0)];
        let out = solve_layout(frame, &l, &children);
        // First at x=0, second after 20 + gap 5 = 25; cross size preserved, y at top.
        assert_eq!(
            out,
            vec![rect(0.0, 0.0, 20.0, 10.0), rect(25.0, 0.0, 30.0, 40.0)]
        );
    }

    #[test]
    fn vertical_start_fixed_with_gap() {
        let frame = rect(0.0, 0.0, 100.0, 200.0);
        let l = layout(Direction::Vertical, 8.0, Align::Start, Sizing::Fixed);
        let children = [size(10.0, 20.0), size(40.0, 30.0)];
        let out = solve_layout(frame, &l, &children);
        // First at y=0, second after 20 + gap 8 = 28; cross size preserved, x at left.
        assert_eq!(
            out,
            vec![rect(0.0, 0.0, 10.0, 20.0), rect(0.0, 28.0, 40.0, 30.0)]
        );
    }

    #[test]
    fn padding_offsets_origin() {
        let frame = rect(0.0, 0.0, 200.0, 100.0);
        let l = AutoLayout {
            direction: Direction::Horizontal,
            gap: ambre_core::BoardLength::ZERO,
            padding: Padding {
                top: ambre_core::BoardLength::new(5.0),
                right: ambre_core::BoardLength::new(10.0),
                bottom: ambre_core::BoardLength::new(15.0),
                left: ambre_core::BoardLength::new(20.0),
            },
            align: Align::Start,
            sizing: Sizing::Fixed,
        };
        let children = [size(30.0, 40.0)];
        let out = solve_layout(frame, &l, &children);
        // Content origin at (left=20, top=5).
        assert_eq!(out, vec![rect(20.0, 5.0, 30.0, 40.0)]);
    }

    #[test]
    fn align_center() {
        let frame = rect(0.0, 0.0, 100.0, 50.0);
        let l = layout(Direction::Horizontal, 0.0, Align::Center, Sizing::Fixed);
        let children = [size(20.0, 10.0), size(20.0, 10.0)];
        // Used = 40, free = 60, half = 30 offset.
        let out = solve_layout(frame, &l, &children);
        assert_eq!(
            out,
            vec![rect(30.0, 0.0, 20.0, 10.0), rect(50.0, 0.0, 20.0, 10.0)]
        );
    }

    #[test]
    fn align_end() {
        let frame = rect(0.0, 0.0, 100.0, 50.0);
        let l = layout(Direction::Horizontal, 0.0, Align::End, Sizing::Fixed);
        let children = [size(20.0, 10.0), size(30.0, 10.0)];
        // Used = 50, free = 50 -> block flush right.
        let out = solve_layout(frame, &l, &children);
        assert_eq!(
            out,
            vec![rect(50.0, 0.0, 20.0, 10.0), rect(70.0, 0.0, 30.0, 10.0)]
        );
    }

    #[test]
    fn align_space_between() {
        let frame = rect(0.0, 0.0, 100.0, 50.0);
        let l = layout(
            Direction::Horizontal,
            0.0,
            Align::SpaceBetween,
            Sizing::Fixed,
        );
        let children = [size(20.0, 10.0), size(20.0, 10.0), size(20.0, 10.0)];
        // Used = 60, free = 40, split over 2 interior gaps = 20 each.
        let out = solve_layout(frame, &l, &children);
        assert_eq!(
            out,
            vec![
                rect(0.0, 0.0, 20.0, 10.0),
                rect(40.0, 0.0, 20.0, 10.0),
                rect(80.0, 0.0, 20.0, 10.0),
            ]
        );
    }

    #[test]
    fn space_between_single_child_falls_back_to_start() {
        let frame = rect(0.0, 0.0, 100.0, 50.0);
        let l = layout(
            Direction::Horizontal,
            0.0,
            Align::SpaceBetween,
            Sizing::Fixed,
        );
        let children = [size(20.0, 10.0)];
        let out = solve_layout(frame, &l, &children);
        assert_eq!(out, vec![rect(0.0, 0.0, 20.0, 10.0)]);
    }

    #[test]
    fn sizing_fill_shares_leftover() {
        let frame = rect(0.0, 0.0, 100.0, 50.0);
        let l = layout(Direction::Horizontal, 10.0, Align::Start, Sizing::Fill);
        let children = [size(10.0, 5.0), size(10.0, 5.0)];
        // Intrinsic 20, gap 10, leftover 70 -> +35 each => widths 45.
        // Cross stretched to content height 50.
        let out = solve_layout(frame, &l, &children);
        assert_eq!(
            out,
            vec![rect(0.0, 0.0, 45.0, 50.0), rect(55.0, 0.0, 45.0, 50.0)]
        );
        // Sum of mains + gap fills the main axis exactly.
        assert!((out[0].width() + out[1].width() + 10.0 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn sizing_hug_equals_fixed() {
        let frame = rect(0.0, 0.0, 200.0, 100.0);
        let children = [size(20.0, 10.0), size(30.0, 40.0)];
        let hug = solve_layout(
            frame,
            &layout(Direction::Horizontal, 5.0, Align::Start, Sizing::Hug),
            &children,
        );
        let fixed = solve_layout(
            frame,
            &layout(Direction::Horizontal, 5.0, Align::Start, Sizing::Fixed),
            &children,
        );
        assert_eq!(hug, fixed);
    }

    #[test]
    fn wrap_breaks_rows() {
        let frame = rect(0.0, 0.0, 100.0, 200.0);
        let l = layout(Direction::Wrap, 10.0, Align::Start, Sizing::Fixed);
        // Row 1: 60 + 10 + 30 = 100 fits; adding 40 -> 140 > 100 wraps.
        let children = [size(60.0, 20.0), size(30.0, 25.0), size(40.0, 15.0)];
        let out = solve_layout(frame, &l, &children);
        // Row 1 height = max(20, 25) = 25; row 2 starts at y = 25 + gap 10 = 35.
        assert_eq!(out[0], rect(0.0, 0.0, 60.0, 20.0));
        assert_eq!(out[1], rect(70.0, 0.0, 30.0, 25.0));
        assert_eq!(out[2], rect(0.0, 35.0, 40.0, 15.0));
    }

    #[test]
    fn wrap_align_per_row() {
        let frame = rect(0.0, 0.0, 100.0, 200.0);
        let l = layout(Direction::Wrap, 0.0, Align::End, Sizing::Fixed);
        // Row 1: 60 + 50 = 110 > 100 -> each child on its own row here (60 then wrap).
        // Use widths that pack two-per-row deterministically: 40 + 40 = 80 fits, +40 wraps.
        let children = [size(40.0, 10.0), size(40.0, 10.0), size(40.0, 10.0)];
        let out = solve_layout(frame, &l, &children);
        // Row 1 = [c0, c1] (80 <= 100), End: free = 20 -> flush right.
        assert_eq!(out[0], rect(20.0, 0.0, 40.0, 10.0));
        assert_eq!(out[1], rect(60.0, 0.0, 40.0, 10.0));
        // Row 2 = [c2] alone, End: free = 60 -> flush right, y = 10.
        assert_eq!(out[2], rect(60.0, 10.0, 40.0, 10.0));
    }

    #[test]
    fn empty_children_is_empty() {
        let frame = rect(0.0, 0.0, 100.0, 100.0);
        let l = layout(Direction::Horizontal, 5.0, Align::Center, Sizing::Fill);
        assert!(solve_layout(frame, &l, &[]).is_empty());
    }

    #[test]
    fn overflow_does_not_produce_negative_spacing() {
        let frame = rect(0.0, 0.0, 40.0, 50.0);
        let l = layout(
            Direction::Horizontal,
            0.0,
            Align::SpaceBetween,
            Sizing::Fixed,
        );
        // Children wider than the box: no negative free space, packed from start.
        let children = [size(30.0, 10.0), size(30.0, 10.0)];
        let out = solve_layout(frame, &l, &children);
        assert_eq!(
            out,
            vec![rect(0.0, 0.0, 30.0, 10.0), rect(30.0, 0.0, 30.0, 10.0)]
        );
    }
    /// `TOILE-FEAT-040`: the far padding bounds the room too. A run packed far
    /// stops at the inner right edge, and a wrap breaks where the padded row
    /// ends rather than at the frame.
    #[test]
    fn the_far_padding_bounds_the_room() {
        let frame = rect(0.0, 0.0, 200.0, 100.0);
        let mut l = layout(Direction::Horizontal, 0.0, Align::End, Sizing::Fixed);
        l.padding = Padding {
            top: ambre_core::BoardLength::new(10.0),
            right: ambre_core::BoardLength::new(20.0),
            bottom: ambre_core::BoardLength::new(10.0),
            left: ambre_core::BoardLength::new(30.0),
        };
        let out = solve_layout(frame, &l, &[size(40.0, 10.0), size(40.0, 10.0)]);
        assert_eq!(out[1].x1, 180.0, "the run stops at the inner edge");
        assert_eq!(out[0].x0, 100.0);
        l.direction = Direction::Wrap;
        l.align = Align::Start;
        // The padded row is 150 wide: two 80-wide children do not share it.
        let out = solve_layout(frame, &l, &[size(80.0, 10.0), size(80.0, 10.0)]);
        assert_eq!(out[0], rect(30.0, 10.0, 80.0, 10.0));
        assert_eq!(out[1], rect(30.0, 20.0, 80.0, 10.0), "the second wraps");
    }

    /// `TOILE-FEAT-045`: a filled frame too small for its children shrinks
    /// none of them. Each keeps its own size on both axes and overflows.
    #[test]
    fn a_filled_frame_shrinks_no_child() {
        let frame = rect(0.0, 0.0, 50.0, 40.0);
        let l = layout(Direction::Horizontal, 5.0, Align::Start, Sizing::Fill);
        let out = solve_layout(frame, &l, &[size(40.0, 60.0), size(40.0, 60.0)]);
        assert_eq!(
            out,
            vec![rect(0.0, 0.0, 40.0, 60.0), rect(45.0, 0.0, 40.0, 60.0)]
        );
    }
}
