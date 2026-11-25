// ambre-client-core/src/cover.rs
// ============================================================================
// What the floating surfaces of the shell cover of the canvas, as each one
// last measured itself (`ambre-spec-shell.md` section 2, `CHROME-171`). A fit
// keeps the framed content out of the strips this model derives (`ZOOM-011`).
// The model is pure: the chrome measures, the shell holds one cover per page,
// and a view reads it when a fit lands. No size of a surface is written twice.
// ============================================================================

use ambre_core::{Css, Point, Px};

/// The gap a fit leaves between the framed content and a surface beside it.
pub const COVER_GAP: Px<Css> = Px::<Css>::new(8.0);

/// One floating surface of the shell. The set is closed: a surface the shell
/// floats over a canvas is a variant here, and publishes through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The creation bar: the board's tool palette today.
    CreationBar,
    /// The context bar over the selection: the board's selection bar, the
    /// format bar of Prose and the mark bar of Murmure.
    ContextBar,
    /// The context panel. It docks beside the canvas on a desktop, so it
    /// covers the canvas only as the sheet of a phone.
    ContextPanel,
    /// The call bubble, away from the channel of the call.
    CallBubble,
}

impl Surface {
    /// Every surface, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::CreationBar,
        Self::ContextBar,
        Self::ContextPanel,
        Self::CallBubble,
    ];
}

/// A box on the page, in CSS pixels, from its top-left to its bottom-right
/// corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageBox {
    /// The top-left corner.
    pub top_left: Point<Px<Css>>,
    /// The bottom-right corner.
    pub bottom_right: Point<Px<Css>>,
}

impl PageBox {
    /// Whether the two boxes share an area. Boxes that only touch share none.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.top_left.x < other.bottom_right.x
            && other.top_left.x < self.bottom_right.x
            && self.top_left.y < other.bottom_right.y
            && other.top_left.y < self.bottom_right.y
    }
}

/// What one surface covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Covered {
    /// A surface that keeps its place on the page while the view moves: its
    /// box on the page.
    Fixed(PageBox),
    /// A surface that rides above the selection: how far it reaches above
    /// the selection's top edge. A fit puts the selection under it, so its
    /// place before the fit does not matter.
    Above(Px<Css>),
}

/// The strips of a canvas that a fit keeps the framed content out of, each
/// measured from its edge of the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clearance {
    /// The strip along the left edge.
    pub left: Px<Css>,
    /// The strip along the top edge.
    pub top: Px<Css>,
    /// The strip along the right edge.
    pub right: Px<Css>,
    /// The strip along the bottom edge.
    pub bottom: Px<Css>,
}

impl Clearance {
    /// Widen each strip to the wider of the two.
    fn widen(&mut self, other: Self) {
        self.left = self.left.max(other.left);
        self.top = self.top.max(other.top);
        self.right = self.right.max(other.right);
        self.bottom = self.bottom.max(other.bottom);
    }
}

/// What the floating surfaces cover, as each last measured itself. `None` for
/// a surface means it covers nothing: it is not mounted, hidden, or not
/// measured yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cover {
    creation_bar: Option<Covered>,
    context_bar: Option<Covered>,
    context_panel: Option<Covered>,
    call_bubble: Option<Covered>,
}

impl Cover {
    /// What `surface` covers now.
    #[must_use]
    pub fn of(&self, surface: Surface) -> Option<Covered> {
        match surface {
            Surface::CreationBar => self.creation_bar,
            Surface::ContextBar => self.context_bar,
            Surface::ContextPanel => self.context_panel,
            Surface::CallBubble => self.call_bubble,
        }
    }

    /// Record what `surface` covers now.
    pub fn set(&mut self, surface: Surface, covered: Option<Covered>) {
        let slot = match surface {
            Surface::CreationBar => &mut self.creation_bar,
            Surface::ContextBar => &mut self.context_bar,
            Surface::ContextPanel => &mut self.context_panel,
            Surface::CallBubble => &mut self.call_bubble,
        };
        *slot = covered;
    }

    /// The strips of the canvas at `canvas` on the page that a fit keeps the
    /// framed content out of (`ZOOM-011`).
    ///
    /// A fixed surface takes the one strip, along an edge of the canvas, that
    /// holds its box and costs the least room. A surface beside the canvas
    /// takes none. A surface above the selection keeps its reach clear at
    /// the top. Each strip a surface takes carries the gap.
    #[must_use]
    pub fn clearance(&self, canvas: PageBox) -> Clearance {
        Surface::ALL
            .iter()
            .filter_map(|surface| self.of(*surface))
            .map(|covered| strip(covered, canvas))
            .fold(Clearance::default(), |mut all, one| {
                all.widen(one);
                all
            })
    }
}

/// The one strip of `canvas` that `covered` takes.
fn strip(covered: Covered, canvas: PageBox) -> Clearance {
    match covered {
        Covered::Above(reach) => Clearance {
            top: reach.max(Px::<Css>::ZERO) + COVER_GAP,
            ..Clearance::default()
        },
        Covered::Fixed(at) if at.overlaps(canvas) => {
            let left = at.bottom_right.x - canvas.top_left.x;
            let top = at.bottom_right.y - canvas.top_left.y;
            let right = canvas.bottom_right.x - at.top_left.x;
            let bottom = canvas.bottom_right.y - at.top_left.y;
            let least = left.min(top).min(right).min(bottom);
            // The first strip of least room wins, so a tie picks one side.
            if least == left {
                Clearance {
                    left: left + COVER_GAP,
                    ..Clearance::default()
                }
            } else if least == top {
                Clearance {
                    top: top + COVER_GAP,
                    ..Clearance::default()
                }
            } else if least == right {
                Clearance {
                    right: right + COVER_GAP,
                    ..Clearance::default()
                }
            } else {
                Clearance {
                    bottom: bottom + COVER_GAP,
                    ..Clearance::default()
                }
            }
        }
        Covered::Fixed(_) => Clearance::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn css(v: f32) -> Px<Css> {
        Px::<Css>::new(v)
    }

    fn at(left: f32, top: f32, right: f32, bottom: f32) -> PageBox {
        PageBox {
            top_left: Point::new(css(left), css(top)),
            bottom_right: Point::new(css(right), css(bottom)),
        }
    }

    /// A canvas of 1000 by 600 under a header of 80.
    const CANVAS: PageBox = PageBox {
        top_left: Point::new(Px::<Css>::new(0.0), Px::<Css>::new(80.0)),
        bottom_right: Point::new(Px::<Css>::new(1000.0), Px::<Css>::new(680.0)),
    };

    fn gap(v: f32) -> Px<Css> {
        css(v) + COVER_GAP
    }

    #[test]
    fn nothing_measured_leaves_the_whole_canvas() {
        assert_eq!(Cover::default().clearance(CANVAS), Clearance::default());
    }

    #[test]
    fn the_palette_covers_the_canvas_up_to_its_right_edge() {
        let mut cover = Cover::default();
        cover.set(
            Surface::CreationBar,
            Some(Covered::Fixed(at(8.0, 200.0, 70.0, 560.0))),
        );
        let clear = cover.clearance(CANVAS);
        assert_eq!(
            clear,
            Clearance {
                left: gap(70.0),
                ..Clearance::default()
            }
        );
    }

    #[test]
    fn a_surface_beside_the_canvas_covers_none_of_it() {
        let mut cover = Cover::default();
        // A desktop panel docks right of the canvas, and touches its edge.
        cover.set(
            Surface::ContextPanel,
            Some(Covered::Fixed(at(1000.0, 80.0, 1300.0, 680.0))),
        );
        assert_eq!(cover.clearance(CANVAS), Clearance::default());
    }

    #[test]
    fn a_phone_sheet_covers_the_bottom_strip() {
        let mut cover = Cover::default();
        cover.set(
            Surface::ContextPanel,
            Some(Covered::Fixed(at(0.0, 380.0, 1000.0, 680.0))),
        );
        assert_eq!(
            cover.clearance(CANVAS),
            Clearance {
                bottom: gap(300.0),
                ..Clearance::default()
            }
        );
    }

    #[test]
    fn a_bubble_in_a_corner_takes_the_strip_that_costs_least() {
        let mut cover = Cover::default();
        // Wide and short in the bottom-right corner: the bottom strip.
        cover.set(
            Surface::CallBubble,
            Some(Covered::Fixed(at(700.0, 610.0, 984.0, 664.0))),
        );
        assert_eq!(
            cover.clearance(CANVAS),
            Clearance {
                bottom: gap(70.0),
                ..Clearance::default()
            }
        );
    }

    #[test]
    fn the_bar_reach_is_kept_above_the_frame() {
        let mut cover = Cover::default();
        cover.set(Surface::ContextBar, Some(Covered::Above(css(79.0))));
        assert_eq!(
            cover.clearance(CANVAS),
            Clearance {
                top: gap(79.0),
                ..Clearance::default()
            }
        );
    }

    #[test]
    fn each_side_keeps_the_widest_strip_of_every_surface() {
        let mut cover = Cover::default();
        cover.set(
            Surface::CreationBar,
            Some(Covered::Fixed(at(8.0, 200.0, 70.0, 560.0))),
        );
        cover.set(Surface::ContextBar, Some(Covered::Above(css(40.0))));
        cover.set(
            Surface::CallBubble,
            Some(Covered::Fixed(at(700.0, 610.0, 984.0, 664.0))),
        );
        cover.set(
            Surface::ContextPanel,
            Some(Covered::Fixed(at(0.0, 380.0, 1000.0, 680.0))),
        );
        assert_eq!(
            cover.clearance(CANVAS),
            Clearance {
                left: gap(70.0),
                top: gap(40.0),
                right: Px::<Css>::ZERO,
                bottom: gap(300.0),
            }
        );
        // A surface that leaves gives its strip back.
        cover.set(Surface::ContextPanel, None);
        assert_eq!(cover.clearance(CANVAS).bottom, gap(70.0));
    }

    #[test]
    fn a_surface_records_and_reads_back_alone() {
        let mut cover = Cover::default();
        for surface in Surface::ALL {
            cover.set(surface, Some(Covered::Above(css(12.0))));
            assert_eq!(cover.of(surface), Some(Covered::Above(css(12.0))));
            let mut others = Surface::ALL.iter().filter(|s| **s != surface);
            assert!(others.all(|s| cover.of(*s).is_none()));
            cover.set(surface, None);
        }
    }
}
