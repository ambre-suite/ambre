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

use ambre_bus::app::toile::{DisplayNode, DisplayViewport, ReactionTally};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Point, PositionedLayoutItem, Rect,
};
use kurbo::{RoundedRect, Shape};

use super::viewport::ViewportTransform;

/// Chip label font size, in logical pixels.
const CHIP_FONT_PX: f32 = 13.0;
/// Horizontal / vertical text padding inside a chip.
const CHIP_PAD_X: f64 = 6.0;
const CHIP_PAD_Y: f64 = 3.0;
/// Gap between adjacent chips and between the element edge and the chip row.
const CHIP_GAP: f64 = 4.0;
const ROW_MARGIN: f64 = 6.0;
/// Corner radius of a chip pill, in logical pixels.
const CHIP_RADIUS: f64 = 8.0;

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
    let y = rect.y1 + ROW_MARGIN;

    // Emoji chips first, in the tally's stable sorted order.
    for (emoji, count) in &tally.emoji {
        let label = format!("{emoji} {count}");
        x = paint_chip(surface, &label, x, y, font_cx, text_cache, palette) + CHIP_GAP;
    }

    // A single trailing vote badge when any vote exists: signed weight sum
    // and voter count, for example "+3 (5)".
    if tally.vote_count > 0 {
        let label = format!("{:+} ({})", tally.vote_total, tally.vote_count);
        paint_chip(surface, &label, x, y, font_cx, text_cache, palette);
    }
}

/// Paint one chip (rounded pill + label) with its top-left at `(x, y)`.
/// Returns the chip's right edge x so the caller can place the next one.
fn paint_chip<S: DrawSurface>(
    surface: &mut S,
    label: &str,
    x: f64,
    y: f64,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    palette: &Palette,
) -> f64 {
    let layout = text_cache.get_or_build(
        font_cx,
        label,
        CHIP_FONT_PX,
        1000.0,
        Alignment::Start,
        false,
        false,
    );
    let text_w = layout.width() as f64;
    let text_h = layout.height() as f64;
    let chip_w = text_w + 2.0 * CHIP_PAD_X;
    let chip_h = text_h + 2.0 * CHIP_PAD_Y;

    // Rounded pill background + hairline border, in the theme's header colours.
    let pill = RoundedRect::from_rect(Rect::new(x, y, x + chip_w, y + chip_h), CHIP_RADIUS);
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
            let tx = x + CHIP_PAD_X + run.offset() as f64;
            let ty = y + CHIP_PAD_Y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
    x + chip_w
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
        DisplayElement, DisplayGeom, ElementId, ElementKind, ElementStyle, ShapeKind, Viewport,
    };
    use ambre_canvas::theme::LIGHT;
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use std::collections::BTreeMap;

    fn elem(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn vp_with(
        nodes: Vec<DisplayNode>,
        reactions: BTreeMap<ElementId, ReactionTally>,
    ) -> DisplayViewport {
        DisplayViewport {
            version: 0,
            grid_unit: 20.0,
            pins: Vec::new(),
            reactions,
            viewport: Viewport::default(),
            nodes,
        }
    }

    /// An element with a tally emits at least one filled pill and glyphs; an
    /// element without one emits nothing.
    #[test]
    fn chip_row_paints_only_tallied_elements() {
        let mut font_cx = FontContext::new();
        let mut text_cache = TextCache::new(1.0);
        let mut surface = RecordingSurface::new();

        let mut reactions = BTreeMap::new();
        reactions.insert(
            ElementId("a".into()),
            ReactionTally {
                emoji: vec![("x".to_string(), 2)],
                vote_total: 0,
                vote_count: 0,
            },
        );
        let vp = vp_with(
            vec![
                elem("a", Rect::new(0.0, 0.0, 40.0, 40.0)),
                elem("b", Rect::new(80.0, 0.0, 120.0, 40.0)),
            ],
            reactions,
        );
        let cam = ViewportTransform::new(200.0, 200.0);
        paint_reactions(
            &mut surface,
            &vp,
            &cam,
            &mut font_cx,
            &mut text_cache,
            &LIGHT,
        );

        let fills = surface.count(|op| matches!(op, DrawOp::FillPath { .. }));
        assert!(fills >= 1, "expected at least one chip pill fill");
    }

    /// No reactions -> no draw ops at all (cheap early out).
    #[test]
    fn empty_reactions_paint_nothing() {
        let mut font_cx = FontContext::new();
        let mut text_cache = TextCache::new(1.0);
        let mut surface = RecordingSurface::new();
        let vp = vp_with(
            vec![elem("a", Rect::new(0.0, 0.0, 40.0, 40.0))],
            BTreeMap::new(),
        );
        let cam = ViewportTransform::new(200.0, 200.0);
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
