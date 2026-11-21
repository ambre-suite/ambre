// ambre-toile/src/ui/cover.rs
// ============================================================================
// What the shell's floating surfaces cover of the board, as the chrome last
// measured it. A fit keeps the framed content out of those strips
// (`ZOOM-011`). The chrome measures, and the view reads the latest measure
// when a fit reply lands. No figure is declared twice.
// ============================================================================

use ambre_core::{Css, Point, Px};

/// The gap a fit leaves between the framed content and a surface beside it.
pub const COVER_GAP: f64 = 8.0;

/// What the floating surfaces cover, as the chrome last measured it. `None`
/// on a side means nothing covers it, and the fit takes that side whole. The
/// surface is then not mounted, hidden, or not measured yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cover {
    /// The right edge of the tool palette, on the page.
    pub palette_right: Option<Px<Css>>,
    /// How far the selection bar reaches above the selection's top edge: its
    /// own height and the lift the stylesheet gives it.
    pub bar_reach: Option<Px<Css>>,
}

impl Cover {
    /// The strips of a canvas whose top-left corner sits at `canvas_origin` on
    /// the page that a fit keeps the framed content out of (`ZOOM-011`).
    ///
    /// The palette covers the canvas up to its right edge. The selection bar
    /// floats above the framed selection, so the room starts below its reach.
    /// A side nothing covers keeps no strip.
    #[must_use]
    pub fn clearance(self, canvas_origin: Point<Px<Css>>) -> kurbo::Insets {
        let left = self.palette_right.map_or(0.0, |right| {
            (f64::from(right.get()) - f64::from(canvas_origin.x.get())).max(0.0) + COVER_GAP
        });
        let top = self
            .bar_reach
            .map_or(0.0, |reach| f64::from(reach.get()).max(0.0) + COVER_GAP);
        kurbo::Insets {
            x0: left,
            y0: top,
            x1: 0.0,
            y1: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css(v: f32) -> Px<Css> {
        Px::<Css>::new(v)
    }

    #[test]
    fn nothing_measured_leaves_the_whole_canvas() {
        let clear = Cover::default().clearance(Point::new(css(0.0), css(80.0)));
        assert_eq!(clear, kurbo::Insets::ZERO);
    }

    #[test]
    fn the_palette_covers_the_canvas_up_to_its_right_edge() {
        let cover = Cover {
            palette_right: Some(css(70.0)),
            bar_reach: None,
        };
        let clear = cover.clearance(Point::new(css(0.0), css(80.0)));
        assert!((clear.x0 - (70.0 + COVER_GAP)).abs() < 1e-9);
        // A canvas that starts right of the palette is not covered by it.
        let beside = cover.clearance(Point::new(css(100.0), css(80.0)));
        assert!((beside.x0 - COVER_GAP).abs() < 1e-9);
    }

    #[test]
    fn the_bar_reach_is_kept_above_the_frame() {
        let cover = Cover {
            palette_right: None,
            bar_reach: Some(css(79.0)),
        };
        let clear = cover.clearance(Point::new(css(0.0), css(80.0)));
        assert!((clear.y0 - (79.0 + COVER_GAP)).abs() < 1e-9);
        assert!(clear.x0.abs() < 1e-9 && clear.x1.abs() < 1e-9 && clear.y1.abs() < 1e-9);
    }
}
