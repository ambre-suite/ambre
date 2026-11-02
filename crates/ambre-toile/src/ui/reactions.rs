// ambre-toile/src/ui/reactions.rs
// ============================================================================
// Reaction badge overlay (toile-reactions). Each element carrying a derived
// reaction tally (in `DisplayViewport.reactions`) gets a row of chips painted
// just below its bottom edge. They are drawn in screen space above all content
// and never culled -- the same overlay pass as presence. Badges are ALWAYS
// visible (edit and present modes).
//
// One chip per distinct emoji shows the color emoji glyph followed by its count,
// such as "thumbs-up 3". The emoji renders in full color through the shared
// color atlas (ambre-canvas), the digits as ordinary text, both in one layout.
// Votes collapse into one trailing badge: the signed weight sum and the
// voter count.
//
// Chips are a constant screen size regardless of Toile zoom, like presence
// chips. The element rect is mapped to screen, but the chip metrics are in
// logical pixels. Pure over `DrawSurface`, so it is host-testable against
// `RecordingSurface` with no GPU.
// ============================================================================

use ambre_bus::app::toile::{DisplayNode, DisplayViewport, ReactionTally, VoteCount, VoteTotal};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Point, PositionedLayoutItem, Rect,
};
use kurbo::{RoundedRect, Shape, Vec2};

use super::viewport::ViewportTransform;

/// Chip label font size, in logical pixels.
const CHIP_FONT_SIZE: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(13.0);
/// The text padding inside a chip: across, then down.
const CHIP_PAD: Vec2 = Vec2::new(6.0, 3.0);
/// Gap between adjacent chips, in logical pixels.
const CHIP_GAP: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(4.0);
/// Gap between the element's bottom edge and its chip row, in logical pixels.
const ROW_MARGIN: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(6.0);
/// Corner radius of a chip pill, in logical pixels.
const CHIP_RADIUS: ambre_core::Px<ambre_core::View> = ambre_core::Px::<ambre_core::View>::new(8.0);

/// Paint the reaction badge row for every element with a tally. Pure over
/// `DrawSurface`: host-testable. Called from the overlay pass, unconditionally
/// (badges are always visible), after presence so chips sit above ghosts.
pub fn paint_reactions<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) {
    if vp.reactions.is_empty() {
        return;
    }
    let w2s = cam.world_to_screen();
    // Walk the display nodes (not the map) so chips paint in z-order and only for
    // elements actually on screen this frame.
    for node in &vp.nodes {
        let DisplayNode::Element(e) = node else {
            continue;
        };
        let Some(tally) = vp.reactions.get(&e.id) else {
            continue;
        };
        let rect = rect_to_screen(e.bounds, &w2s);
        paint_chip_row(surface, tally, rect, font_cx, text_cache, palette);
    }
}

/// Lay out one element's chips left-to-right below its bottom-left corner.
fn paint_chip_row<S: DrawSurface>(
    surface: &mut S,
    tally: &ReactionTally,
    rect: Rect,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) {
    let mut x = rect.x0;
    let y = rect.y1 + f64::from(ROW_MARGIN.get());

    // Emoji chips first, in the tally's stable sorted order.
    for (emoji, count) in &tally.emoji {
        let label = format!("{} {count}", emoji.as_str());
        let edge = paint_chip(
            surface,
            &label,
            Point::new(x, y),
            font_cx,
            text_cache,
            palette,
        );
        x = f64::from((edge + CHIP_GAP).get());
    }

    // A single trailing vote badge when any vote exists: signed weight sum
    // and voter count, for example "+3 (5)".
    if tally.vote_count > VoteCount::ZERO {
        let label = vote_label(tally.vote_total, tally.vote_count);
        paint_chip(
            surface,
            &label,
            Point::new(x, y),
            font_cx,
            text_cache,
            palette,
        );
    }
}

/// The trailing vote chip's label: the signed total, then the number of
/// voters in parentheses (`TOILE-UI-304`).
fn vote_label(total: VoteTotal, count: VoteCount) -> String {
    format!("{total:+} ({count})")
}

/// Paint one chip (rounded pill + label) with its top-left at `at`.
/// Returns the chip's right edge x, a view coordinate, so the caller can place
/// the next one.
fn paint_chip<S: DrawSurface>(
    surface: &mut S,
    label: &str,
    at: Point,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) -> ambre_core::Px<ambre_core::View> {
    let layout = text_cache.get_or_build(
        font_cx,
        label,
        CHIP_FONT_SIZE,
        ambre_core::Px::<ambre_core::View>::new(1000.0),
        Alignment::Start,
        ambre_canvas::Weight::Regular,
        ambre_canvas::Slant::Upright,
    );
    let text_w = layout.width() as f64;
    let text_h = layout.height() as f64;
    let chip_w = text_w + 2.0 * CHIP_PAD.x;
    let chip_h = text_h + 2.0 * CHIP_PAD.y;

    // Rounded pill background + hairline border, in the theme's header colours.
    let pill = RoundedRect::from_rect(
        Rect::new(at.x, at.y, at.x + chip_w, at.y + chip_h),
        f64::from(CHIP_RADIUS.get()),
    );
    let path = pill.to_path(0.1);
    surface.fill_path(&path, &Brush::Solid(palette.header_bg));
    surface.stroke_path(
        &path,
        &Brush::Solid(palette.header_border),
        &ambre_draw::Stroke::new(1.0),
    );

    // Label: color emoji glyphs route to the color atlas, digits to the mask
    // atlas -- both from a single layout via the shared `draw_glyphs` chokepoint.
    let brush = Brush::Solid(text_color(palette));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = at.x + CHIP_PAD.x + run.offset() as f64;
            let ty = at.y + CHIP_PAD.y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
    ambre_core::Px::<ambre_core::View>::new((at.x + chip_w) as f32)
}

/// The non-emoji label colour (vote badges, counts). Emoji ignore it (they carry
/// their own colours), so this only tints the digits and vote text.
fn text_color(palette: &Palette) -> Color {
    palette.cell_text
}

/// Map a world-space rectangle to screen space through `w2s`.
fn rect_to_screen(b: Rect, w2s: &Affine) -> Rect {
    let tl = *w2s * Point::new(b.x0, b.y0);
    let br = *w2s * Point::new(b.x1, b.y1);
    Rect::new(tl.x, tl.y, br.x, br.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, ElementKind, ElementStyle, ReactionCount, ShapeKind, Viewport,
    };
    use ambre_canvas::theme::LIGHT;
    use ambre_core::id::{Element, Id};
    use ambre_core::{Board, Px};
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use std::collections::BTreeMap;

    fn elem(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn vp_with(
        nodes: Vec<DisplayNode>,
        reactions: BTreeMap<Id<Element>, ReactionTally>,
    ) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: Px::<Board>::new(20.0),
            pins: Vec::new(),
            reactions,
            viewport: Viewport::default(),
            nodes,
        }
    }

    /// Paint `vp` through `cam` and return the screen box of every pill, in
    /// paint order. A pill is the one path stroked in the header border.
    fn pills(vp: &DisplayViewport, cam: &ViewportTransform) -> Vec<Rect> {
        let mut font_cx = FontContext::new();
        let mut text_cache = TextCache::new();
        let mut surface = RecordingSurface::new();
        paint_reactions(&mut surface, vp, cam, &mut font_cx, &mut text_cache, &LIGHT);
        surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokePath { color, bounds, .. } if *color == Some(LIGHT.header_border) => {
                    Some(*bounds)
                }
                _ => None,
            })
            .collect()
    }

    fn tally(emoji: &[(&str, u32)], vote_total: i64, vote_count: u32) -> ReactionTally {
        ReactionTally {
            emoji: emoji
                .iter()
                .map(|(e, n)| (ambre_bus::Emoji::from(*e), ReactionCount::new(*n)))
                .collect(),
            vote_total: VoteTotal::new(vote_total),
            vote_count: VoteCount::new(vote_count),
        }
    }

    /// `TOILE-UI-302`: of a tallied and an untallied element, only the tallied
    /// one paints a chip. Exactly one pill is painted, and it sits under the
    /// tallied element.
    #[test]
    fn chip_row_paints_only_tallied_elements() {
        let a = Rect::new(0.0, 0.0, 40.0, 40.0);
        let mut reactions = BTreeMap::new();
        reactions.insert(
            Id::<Element>::from_text("a".into()),
            tally(&[("x", 2)], 0, 0),
        );
        let vp = vp_with(
            vec![elem("a", a), elem("b", Rect::new(80.0, 0.0, 120.0, 40.0))],
            reactions,
        );
        let cam = ViewportTransform::new(ambre_draw::Size::new(200.0, 200.0));

        let pills = pills(&vp, &cam);
        let a_on_screen = rect_to_screen(a, &cam.world_to_screen());
        assert_eq!(pills.len(), 1, "one chip, for the tallied element");
        assert!(
            (pills[0].x0 - a_on_screen.x0).abs() < 1.0 && pills[0].y0 > a_on_screen.y1,
            "the chip sits under the tallied element: {pills:?}"
        );
    }

    /// `TOILE-UI-080`: the same tally paints the same pills at 1x and at 4x.
    /// Only where the row starts follows the zoom.
    #[test]
    fn a_badge_keeps_one_screen_size_through_the_zoom() {
        let mut reactions = BTreeMap::new();
        reactions.insert(
            Id::<Element>::from_text("a".into()),
            tally(&[("x", 2)], 1, 1),
        );
        let vp = vp_with(vec![elem("a", Rect::new(0.0, 0.0, 40.0, 40.0))], reactions);
        let near = ViewportTransform::new(ambre_draw::Size::new(800.0, 800.0));
        let far = ViewportTransform {
            zoom: ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(4.0),
            ..near
        };

        let (at_1x, at_4x) = (pills(&vp, &near), pills(&vp, &far));
        assert_eq!(at_1x.len(), 2);
        assert_eq!(at_4x.len(), 2);
        for (one, four) in at_1x.iter().zip(&at_4x) {
            assert!(
                (one.width() - four.width()).abs() < 1e-6
                    && (one.height() - four.height()).abs() < 1e-6,
                "one size at both zooms: {one:?} against {four:?}"
            );
        }
        assert_ne!(at_1x[0].y0, at_4x[0].y0, "the row follows the element");
    }

    /// `TOILE-UI-304`: votes collapse into one trailing chip after the emoji
    /// chips. Its label is the signed total, then the voter count.
    #[test]
    fn votes_collapse_into_one_trailing_chip() {
        let mut reactions = BTreeMap::new();
        reactions.insert(
            Id::<Element>::from_text("a".into()),
            tally(&[("x", 1), ("y", 2)], -2, 3),
        );
        let vp = vp_with(vec![elem("a", Rect::new(0.0, 0.0, 40.0, 40.0))], reactions);
        let pills = pills(
            &vp,
            &ViewportTransform::new(ambre_draw::Size::new(400.0, 400.0)),
        );

        assert_eq!(pills.len(), 3, "two emoji chips and one vote chip");
        assert!(
            pills.windows(2).all(|w| w[0].x1 < w[1].x0),
            "left to right, the vote chip last: {pills:?}"
        );
        let label = |total, count| vote_label(VoteTotal::new(total), VoteCount::new(count));
        assert_eq!(label(-2, 3), "-2 (3)");
        assert_eq!(label(4, 2), "+4 (2)");
        assert_eq!(label(0, 2), "+0 (2)", "a zero total keeps its sign");
    }

    /// No reactions -> no draw ops at all (cheap early out).
    #[test]
    fn empty_reactions_paint_nothing() {
        let mut font_cx = FontContext::new();
        let mut text_cache = TextCache::new();
        let mut surface = RecordingSurface::new();
        let vp = vp_with(
            vec![elem("a", Rect::new(0.0, 0.0, 40.0, 40.0))],
            BTreeMap::new(),
        );
        let cam = ViewportTransform::new(ambre_draw::Size::new(200.0, 200.0));
        paint_reactions(
            &mut surface,
            &vp,
            &cam,
            &mut font_cx,
            &mut text_cache,
            &LIGHT,
        );
        assert!(surface.ops().is_empty());
    }
}
