// ambre-toile/src/ui/render.rs
// ============================================================================
// The Toile content walk (toile-ui §2). Toile owns only "what to draw": it walks
// the culled, z-ordered `DisplayViewport` and emits ABSTRACT `DrawSurface` calls;
// tessellation, the glyph atlas and GPU submission are `ambre-canvas`'s job. So
// the walk is host-testable against `RecordingSurface` (a mock `DrawSurface`):
// assert the ordered `DrawOp` sequence with no GPU.
//
// Two passes:
//   paint_content -- board bg + adaptive grid, then one z-ordered walk of the
//                    nodes (shapes / paths / strokes / connectors) under the
//                    camera transform, then a screen-space text pass.
//   paint_overlay -- selection bounding box + transform handles (screen space).
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementId,
    ElementStyle, ShapeKind,
};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DrawSurface, FontContext, ImageSource, Point,
    PositionedLayoutItem, Rect, Stroke,
};

use crate::logic::animate::AnimatedElement;

use super::grid;
use super::hit::triangle_path;
use super::tool::{
    HANDLE_SIZE_PX, Preview, resize_handle_points, rotate_handle_point, selection_bounds,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;

/// A decoded raster image ready to paint: non-premultiplied RGBA8 pixels
/// (row-major, `width * height * 4` bytes) plus native dimensions.
///
/// Keyed by the element's blake2b media hash (`DisplayGeom::Image`). The
/// screen renderer fills this from its media cache and the PDF export path
/// decodes it from the `media` LoroMap (`toile-interop §5.B`); a hash absent
/// from the map means the pixels have not arrived yet, so the element paints a
/// placeholder box instead (§5.B) until a later frame carries them.
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Resolved image pixels for this frame, keyed by media hash. Empty when no
/// image element is present (or its media has not been decoded yet).
pub type ImageStore = HashMap<String, DecodedImage>;

/// Fixed ink (ARGB) for a sticky-note body: a near-black that stays readable on
/// any pastel card colour, so the body is legible regardless of the card fill
/// (`toile-logic §2.C`). Independent of the theme (the card is not). Shared with
/// the inline editor so an in-edit sticky uses the same ink (`edit.rs`).
pub(crate) const STICKY_INK: u32 = 0xFF1A_1A1A;

/// Sticky drop-shadow colour (ARGB): a soft translucent black offset to the
/// bottom-right so the note reads as a physical card (§2.C, feedback).
const STICKY_SHADOW: u32 = 0x3300_0000;
/// Drop-shadow offset in world units (bottom-right).
const STICKY_SHADOW_OFFSET: f64 = 4.0;

/// The body font size (world units) for a sticky of the given bounds: it scales
/// with the note so the text "adapts to the size of the note" (feedback). A
/// fraction of the shorter side, clamped to a legible range. Shared with the
/// inline editor so the in-edit size matches the committed one (`edit.rs`).
pub(crate) fn sticky_font_size(bounds: Rect) -> f32 {
    let shorter = bounds.width().min(bounds.height()) as f32;
    (shorter * 0.16).clamp(8.0, 48.0)
}

/// The effective body font size (world units) for a sticky / text element: a
/// sticky auto-sizes from its bounds while `style.font.size` is the auto sentinel
/// (`STICKY_AUTO_FONT_SIZE`, i.e. not positive), but a positive size set from the
/// style inspector overrides it; a text element always uses its style size (§6).
pub(crate) fn sticky_or_style_font_size(
    is_sticky: bool,
    style: &ElementStyle,
    bounds: Rect,
) -> f32 {
    if is_sticky && style.font.size <= 0.0 {
        sticky_font_size(bounds)
    } else {
        style.font.size
    }
}

/// Resolve a sticky / text body's ink (ARGB), or `None` to fall back to the theme
/// text colour (§2.E). An explicit `text_color` always wins; otherwise a sticky
/// uses its fixed ink (its `fill` is the card background, not the text), and a free
/// text element falls back to its `fill` (legacy: a text element's fill was its
/// glyph colour) then, if unset, the theme colour. Pure and host-tested; shared
/// with the inline editor via the same precedence in `edit.rs`.
pub(crate) fn resolved_text_ink(
    is_sticky: bool,
    text_color: Option<u32>,
    fill: Option<u32>,
) -> Option<u32> {
    match (text_color, is_sticky) {
        (Some(tc), _) => Some(tc),
        (None, true) => Some(STICKY_INK),
        (None, false) => fill,
    }
}

/// Convert an ARGB `u32` (Toile style register, §2.E) to a `peniko::Color`,
/// pre-multiplying the element `opacity` into the alpha channel.
fn argb(v: u32, opacity: f32) -> Color {
    let a = ((v >> 24) & 0xff) as f32 / 255.0;
    let r = ((v >> 16) & 0xff) as u8;
    let g = ((v >> 8) & 0xff) as u8;
    let b = (v & 0xff) as u8;
    let alpha = (a * opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(r, g, b, alpha)
}

/// A closed rectangle as a path (so it maps through the active transform and
/// honors element rotation, unlike the axis-aligned `fill_rect` fast path).
fn rect_path(b: Rect) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((b.x0, b.y0));
    p.line_to((b.x1, b.y0));
    p.line_to((b.x1, b.y1));
    p.line_to((b.x0, b.y1));
    p.close_path();
    p
}

/// An axis-aligned ellipse inscribed in `b`, as a flattened path.
fn ellipse_path(b: Rect) -> BezPath {
    let c = b.center();
    let ell = kurbo::Ellipse::new(c, (b.width() / 2.0, b.height() / 2.0), 0.0);
    let mut p = BezPath::new();
    p.extend(kurbo::Shape::path_elements(&ell, 0.1));
    p
}

/// A rounded rectangle inscribed in `b`, as a flattened path. Used for the
/// sticky-note card and its shadow; with `radius = 0` (the sticky default) it is
/// a plain rectangle. The radius is clamped to half the shorter side so a tiny
/// card stays a valid shape.
fn roundrect_path(b: Rect, radius: f64) -> BezPath {
    let r = radius.min(b.width() / 2.0).min(b.height() / 2.0).max(0.0);
    let rr = kurbo::RoundedRect::from_rect(b, r);
    let mut p = BezPath::new();
    p.extend(kurbo::Shape::path_elements(&rr, 0.1));
    p
}

/// A freehand-stroke polyline as an open path.
fn polyline_path(pts: &[Point]) -> BezPath {
    let mut p = BezPath::new();
    let mut it = pts.iter();
    if let Some(first) = it.next() {
        p.move_to(*first);
        for pt in it {
            p.line_to(*pt);
        }
    }
    p
}

/// Fill then stroke a path from an element's style.
fn fill_and_stroke<S: DrawSurface>(surface: &mut S, path: &BezPath, style: &ElementStyle) {
    if let Some(fill) = style.fill {
        surface.fill_path(path, &Brush::Solid(argb(fill, style.opacity)));
    }
    if let Some(stroke) = style.stroke
        && style.stroke_w > 0.0
    {
        surface.stroke_path(
            path,
            &Brush::Solid(argb(stroke, style.opacity)),
            &Stroke::new(style.stroke_w as f64),
        );
    }
}

/// Paint the persistent content layer: board background + adaptive grid, then the
/// z-ordered node walk under the camera transform, then a screen-space text pass.
#[allow(clippy::too_many_arguments)]
pub fn paint_content<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    editing: Option<&ElementId>,
    // Decoded image pixels for this frame, keyed by media hash. An `Image`
    // element whose hash is present paints real pixels via `draw_image`; one
    // absent paints a placeholder box (`toile-interop §5.B`). Empty on a board
    // with no images or whose media has not yet been decoded.
    images: &ImageStore,
    // Smart Animate overrides for an in-flight slide transition (§1.C): keyed by
    // element id, each replaces the element's transform / bounds / opacity / fill
    // for this frame. Empty when no transition is active. Fade-only ids (a new /
    // gone element) still appear here so their opacity ramp is honored.
    anim: &HashMap<ElementId, AnimatedElement>,
    // Presentation focus mode (§1.D): the UI hides the board grid so the audience
    // sees only the slide content.
    present: bool,
) {
    // 1. Background + adaptive grid (screen space). Focus mode drops the grid.
    if present {
        grid::paint_background(surface, cam, palette);
    } else {
        grid::paint_grid(surface, cam, palette);
    }

    // 2. Shapes / paths / strokes / connectors, in world space under the camera.
    surface.push_transform(cam.world_to_screen());
    for node in &vp.nodes {
        match node {
            DisplayNode::Element(e) => paint_element(surface, e, images, anim.get(&e.id)),
            DisplayNode::Connector(c) => paint_connector(surface, c),
        }
    }
    surface.pop_transform();

    // 3. Text pass in screen space (crisp glyphs; font size scales with zoom).
    // Both sticky notes and free text carry a body string here; a sticky draws
    // its body in a fixed readable ink (its `fill` is the card colour), a text
    // element draws in its `fill` (§2.C).
    for node in &vp.nodes {
        let DisplayNode::Element(e) = node else {
            continue;
        };
        let (text, is_sticky) = match &e.geom {
            DisplayGeom::Sticky(t) => (t, true),
            DisplayGeom::Text(t) => (t, false),
            _ => continue,
        };
        // Skip the element under inline edit: its live buffer is painted by the
        // editor overlay instead (§6).
        if editing == Some(&e.id) {
            continue;
        }
        paint_text(
            surface,
            e,
            text,
            is_sticky,
            cam,
            palette,
            font_cx,
            text_cache,
            anim.get(&e.id),
        );
    }
}

/// The world transform of an element whose geometry an animation override
/// replaces: rotation about the (interpolated) AABB center, mirroring
/// [`display_element`](crate::logic::state) so the tween matches the static pass.
fn anim_transform(a: &AnimatedElement) -> Affine {
    let c = a.bounds.center();
    Affine::translate(c.to_vec2()) * Affine::rotate(a.rotation) * Affine::translate(-c.to_vec2())
}

/// The style an element paints with this frame: its own, or the base overlaid
/// with the animation override's opacity / fill (§1.C interpolates only those
/// visual attributes; the rest of the register is held).
fn anim_style(base: &ElementStyle, a: Option<&AnimatedElement>) -> ElementStyle {
    match a {
        None => base.clone(),
        Some(a) => ElementStyle {
            opacity: a.opacity,
            fill: a.fill,
            ..base.clone()
        },
    }
}

/// Paint one element under the (already pushed) camera transform. Rotation is
/// applied by pushing the element's own transform, so geometry is expressed in
/// the unrotated world frame (matching hit-testing, `hit.rs`). When `anim` is
/// `Some`, the element is mid-transition: its transform, bounds and opacity /
/// fill come from the Smart Animate override instead of the static display (§1.C).
fn paint_element<S: DrawSurface>(
    surface: &mut S,
    e: &DisplayElement,
    images: &ImageStore,
    anim: Option<&AnimatedElement>,
) {
    let transform = anim.map(anim_transform).unwrap_or(e.transform);
    let bounds = anim.map(|a| a.bounds).unwrap_or(e.bounds);
    let style = anim_style(&e.style, anim);
    surface.push_transform(transform);
    match &e.geom {
        DisplayGeom::Rect | DisplayGeom::Frame => {
            fill_and_stroke(surface, &rect_path(bounds), &style);
        }
        DisplayGeom::Ellipse => fill_and_stroke(surface, &ellipse_path(bounds), &style),
        DisplayGeom::Triangle => fill_and_stroke(surface, &triangle_path(bounds), &style),
        DisplayGeom::Path(p) => fill_and_stroke(surface, p, &style),
        DisplayGeom::Stroke(pts) => {
            // A stroke has no fill; paint the smoothed polyline outline only.
            if let Some(stroke) = style.stroke {
                surface.stroke_path(
                    &polyline_path(pts),
                    &Brush::Solid(argb(stroke, style.opacity)),
                    &Stroke::new(style.stroke_w.max(1.0) as f64),
                );
            }
        }
        // An image paints its decoded pixels when they have arrived, scaled to
        // the element AABB (the rect is stored at the imported aspect, §5.B); a
        // hash not yet in the store paints a placeholder box until a later frame
        // carries the media. `draw_image` maps through the active element
        // transform, so a rotated image rotates with its box.
        DisplayGeom::Image(hash) => match images.get(hash) {
            Some(img) => surface.draw_image(
                &ImageSource {
                    pixels: &img.pixels,
                    width: img.width,
                    height: img.height,
                },
                bounds,
            ),
            None => fill_and_stroke(surface, &rect_path(bounds), &style),
        },
        // Embeds (iframe) and doc-embeds paint a placeholder box; their content
        // is composited outside the canvas walk.
        DisplayGeom::Embed(_) | DisplayGeom::DocEmbed { .. } => {
            fill_and_stroke(surface, &rect_path(bounds), &style);
        }
        // A sticky note paints a small drop shadow, then its coloured card here;
        // the body string is drawn on top in the screen-space text pass (§2.C). A
        // free `Text` element has no card, so it paints nothing in the geometry
        // pass. `style.corner` is 0 (Miro-style rectangle), so `roundrect_path`
        // degenerates to a plain rectangle.
        DisplayGeom::Sticky(_) => {
            let shadow = bounds + ambre_draw::Vec2::new(STICKY_SHADOW_OFFSET, STICKY_SHADOW_OFFSET);
            surface.fill_path(
                &roundrect_path(shadow, style.corner as f64),
                &Brush::Solid(argb(STICKY_SHADOW, style.opacity)),
            );
            fill_and_stroke(
                surface,
                &roundrect_path(bounds, style.corner as f64),
                &style,
            );
        }
        DisplayGeom::Text(_) => {}
        DisplayGeom::Group => {} // paints nothing (§2.F bis)
    }
    surface.pop_transform();
}

fn paint_connector<S: DrawSurface>(surface: &mut S, c: &DisplayConnector) {
    let color = c.style.stroke.unwrap_or(0xFF33_3333);
    surface.stroke_path(
        &c.path,
        &Brush::Solid(argb(color, c.style.opacity)),
        &Stroke::new(c.style.stroke_w.max(1.0) as f64),
    );
}

/// Draw a sticky / text body in screen space, laid out within the element's
/// screen rectangle at a zoom-scaled font size.
#[allow(clippy::too_many_arguments)]
fn paint_text<S: DrawSurface>(
    surface: &mut S,
    e: &DisplayElement,
    text: &str,
    is_sticky: bool,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    anim: Option<&AnimatedElement>,
) {
    if text.is_empty() {
        return;
    }
    // Mid-transition, the body follows the tweened bounds and opacity (§1.C); the
    // font attributes and (when unset) color are held from the static style.
    let bounds = anim.map(|a| a.bounds).unwrap_or(e.bounds);
    let opacity = anim.map(|a| a.opacity).unwrap_or(e.style.opacity);
    let fill = anim.map(|a| a.fill).unwrap_or(e.style.fill);
    let sb = world_rect_to_screen(bounds, cam);
    // A sticky's font tracks the note size unless the style carries an explicit
    // override (size > 0); a text element uses its own style size. Zoom is applied
    // on top for crisp glyphs (§6).
    let base_size = sticky_or_style_font_size(is_sticky, &e.style, bounds);
    let font_size = (base_size as f64 * cam.zoom) as f32;
    // Text (foreground) colour, §2.E (see `resolved_text_ink`): `None` means "use
    // the theme text colour" (only a text element with neither text_color nor fill).
    let color = resolved_text_ink(is_sticky, e.style.text_color, fill)
        .map(|c| argb(c, opacity))
        .unwrap_or(palette.cell_text);
    let layout = text_cache.get_or_build_font(
        font_cx,
        text,
        font_size,
        sb.width() as f32,
        Alignment::Start,
        e.style.font.weight >= 700,
        e.style.font.italic,
        Some(e.style.font.family.as_str()),
        false,
    );
    let brush = Brush::Solid(color);
    let underline = e.style.font.underline;
    let strikethrough = e.style.font.strikethrough;
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = sb.x0 + run.offset() as f64;
            let ty = sb.y0 + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            // Per-run text decorations, drawn in the glyph ink (matches Trame's
            // cell decorations): a 1px underline just below the baseline and a 1px
            // strikethrough through the x-height (~0.3em above the baseline).
            if underline || strikethrough {
                let adv = run.advance() as f64;
                if underline {
                    surface.stroke_line(tx, ty + 2.0, tx + adv, ty + 2.0, color, 1.0);
                }
                if strikethrough {
                    let sy = ty - (font_size as f64) * 0.3;
                    surface.stroke_line(tx, sy, tx + adv, sy, color, 1.0);
                }
            }
        }
    }
}

/// Paint the transient overlay: find highlights, the selection bounding box + 8
/// resize handles + the rotate handle, in screen space (toile-ui §2 pass 3, §7,
/// §5quater). The selection box is skipped for an empty selection, but find
/// highlights are painted regardless (a search can be active with nothing
/// selected).
#[allow(clippy::too_many_arguments)]
pub fn paint_overlay<S: DrawSurface>(
    surface: &mut S,
    selection: &[ElementId],
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    preview: Option<&Preview>,
    find_hits: &[ElementId],
    find_focus: Option<&ElementId>,
) {
    // Live gesture preview (world -> screen), a thin outline / trail.
    if let Some(p) = preview {
        paint_preview(surface, p, cam, palette);
    }

    // Find highlights (§5quater): outline every matching element visible in the
    // culled set, with a heavier outline on the focused hit. Off-screen hits
    // simply have no box until the camera (recentered on `focus`) brings them in.
    paint_find_highlights(surface, find_hits, find_focus, vp, cam, palette);

    let Some(wb) = selection_bounds(selection, vp) else {
        return;
    };
    let b = world_rect_to_screen(wb, cam);
    // Bounding box.
    surface.stroke_rect(b, palette.selection, 1.5);

    // Rotate handle: a short stem + the knob.
    let rot = rotate_handle_point(b);
    surface.stroke_line(b.center().x, b.y0, rot.x, rot.y, palette.selection, 1.0);
    draw_handle(surface, rot, palette);

    // 8 resize handles (only for a single-element selection, matching the tool's
    // handle picking; a multi-selection shows the box only in Phase 0).
    if selection.len() == 1 {
        for (_, p) in resize_handle_points(b) {
            draw_handle(surface, p, palette);
        }
    }
}

/// Paint an in-flight gesture preview (§5). Marquee is a thin screen-space
/// outline; the shape / connector / pen / stroke previews are drawn in world
/// space under the camera transform with a zoom-compensated 1.5 px line so they
/// track the geometry they will commit to.
fn paint_preview<S: DrawSurface>(
    surface: &mut S,
    preview: &Preview,
    cam: &ViewportTransform,
    palette: &Palette,
) {
    if let Preview::Marquee(m) = preview {
        let s = world_rect_to_screen(*m, cam);
        surface.stroke_rect(s, palette.selection, 1.0);
        return;
    }

    let brush = Brush::Solid(palette.selection);
    let px = |n: f64| Stroke::new(n / cam.zoom.max(1e-3));
    surface.push_transform(cam.world_to_screen());
    match preview {
        Preview::Marquee(_) => {} // handled above
        Preview::Shape { kind, rect } => match kind {
            ShapeKind::Rect => surface.stroke_path(&rect_path(*rect), &brush, &px(1.5)),
            ShapeKind::Ellipse => surface.stroke_path(&ellipse_path(*rect), &brush, &px(1.5)),
            ShapeKind::Triangle => surface.stroke_path(&triangle_path(*rect), &brush, &px(1.5)),
            ShapeKind::Line => {
                let mut p = BezPath::new();
                p.move_to((rect.x0, rect.y0));
                p.line_to((rect.x1, rect.y1));
                surface.stroke_path(&p, &brush, &px(1.5));
            }
            ShapeKind::Path => {}
        },
        Preview::Connector { from, to } => {
            let mut p = BezPath::new();
            p.move_to(*from);
            p.line_to(*to);
            surface.stroke_path(&p, &brush, &px(1.5));
        }
        Preview::Pen(path) => surface.stroke_path(path, &brush, &px(1.5)),
        Preview::Stroke(pts) => surface.stroke_path(&polyline_path(pts), &brush, &px(2.0)),
    }
    surface.pop_transform();
}

/// The world-space AABB of a displayed element, or `None` when the id is not in
/// the culled set (an off-screen match).
fn element_bounds(vp: &DisplayViewport, id: &ElementId) -> Option<Rect> {
    vp.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => Some(e.bounds),
        _ => None,
    })
}

/// Outline the find matches present in the culled set (§5quater). Every hit gets
/// a light box; the focused hit gets a heavier one in the selection accent so it
/// stands out as the current match.
fn paint_find_highlights<S: DrawSurface>(
    surface: &mut S,
    hits: &[ElementId],
    focus: Option<&ElementId>,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
) {
    for id in hits {
        let Some(wb) = element_bounds(vp, id) else {
            continue;
        };
        let sb = world_rect_to_screen(wb, cam);
        let is_focus = focus == Some(id);
        // The focused match reads as "current" in the selection accent; the rest
        // use the header-active amber as a softer match tint.
        let (color, width) = if is_focus {
            (palette.selection, 2.5)
        } else {
            (palette.header_active, 1.5)
        };
        surface.stroke_rect(sb, color, width);
    }
}

/// A small filled square handle centered on `p`.
fn draw_handle<S: DrawSurface>(surface: &mut S, p: Point, palette: &Palette) {
    let h = HANDLE_SIZE_PX / 2.0;
    surface.fill_rect(
        Rect::new(p.x - h, p.y - h, p.x + h, p.y + h),
        palette.selection,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{DisplayElement, ElementId, ElementKind, ElementStyle, ShapeKind};
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn elem(id: &str, geom: DisplayGeom, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom,
            group: None,
        })
    }

    fn cam() -> ViewportTransform {
        ViewportTransform::new(1000.0, 800.0)
    }

    #[test]
    fn content_walk_balances_transform_and_paints_each_node() {
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("r", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
                elem("e", DisplayGeom::Ellipse, Rect::new(60.0, 0.0, 110.0, 50.0)),
            ],
        };
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        paint_content(
            &mut surface,
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            &mut font_cx,
            &mut tc,
            None,
            &ImageStore::new(),
            &HashMap::new(),
            false,
        );

        // Transform stack must be balanced.
        assert_eq!(surface.transform_depth(), 0);
        // One outer camera push/pop wraps the node walk; each element also pushes
        // its own transform. So total PushTransform == 1 + node_count.
        let pushes = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::PushTransform { .. }))
            .count();
        assert_eq!(pushes, 3);
        // Both shapes fill via a path (default style has a fill).
        let fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillPath { .. }))
            .count();
        assert_eq!(fills, 2);
    }

    // The §6 "camera pass" test, host-run against the mock surface: with a Smart
    // Animate override in the map, the animated element paints with the tweened
    // opacity / fill instead of its static style, while an un-overridden element
    // keeps its own. Exercises the render side of the playback -> camera wiring.
    #[test]
    fn content_walk_applies_animation_override() {
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
                elem("b", DisplayGeom::Rect, Rect::new(60.0, 0.0, 110.0, 50.0)),
            ],
        };
        // Override "a" with a half-opacity blue fill at a tweened rect; leave "b".
        let mut anim = HashMap::new();
        anim.insert(
            ElementId("a".into()),
            AnimatedElement {
                id: ElementId("a".into()),
                bounds: Rect::new(20.0, 20.0, 70.0, 70.0),
                rotation: 0.0,
                opacity: 0.5,
                fill: Some(0xFF00_00FF),
            },
        );
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        paint_content(
            &mut surface,
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            &mut font_cx,
            &mut tc,
            None,
            &ImageStore::new(),
            &anim,
            true, // focus mode: grid suppressed
        );

        // The overridden element fills with the tweened blue at half alpha
        // (0.5 * 0xFF -> ~128); a color the default white style never produces.
        let tweened = Color::from_rgba8(0, 0, 255, 128);
        let has_tweened = surface
            .ops()
            .iter()
            .any(|op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == tweened));
        assert!(
            has_tweened,
            "animated element must paint its interpolated fill/opacity"
        );
    }

    /// Build an element with an explicit kind + geom + fill, for the sticky/text
    /// card tests (the render path keys the card off the geom, the ink off the
    /// `is_sticky` flag derived from it).
    fn styled_elem(
        id: &str,
        kind: ElementKind,
        geom: DisplayGeom,
        fill: Option<u32>,
    ) -> DisplayNode {
        let style = ElementStyle {
            fill,
            stroke: None,
            ..ElementStyle::default()
        };
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind,
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 100.0, 80.0),
            locked: false,
            style,
            geom,
            group: None,
        })
    }

    fn paint(vp: &DisplayViewport) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        paint_content(
            &mut surface,
            vp,
            &cam(),
            &LIGHT_PALETTE,
            &mut font_cx,
            &mut tc,
            None,
            &ImageStore::new(),
            &HashMap::new(),
            false,
        );
        surface
    }

    // A sticky note paints a coloured card in the geometry pass: exactly the bug
    // behind "clicking Sticky does nothing" -- an empty sticky used to derive as
    // Text and paint no background, so it was invisible (toile-logic §2.C).
    #[test]
    fn sticky_paints_a_colored_card() {
        let fill = 0xFFFE_F08A; // sticky yellow
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![styled_elem(
                "s",
                ElementKind::Sticky,
                DisplayGeom::Sticky("hi".into()),
                Some(fill),
            )],
        };
        let surface = paint(&vp);
        // The card is a FillPath in the sticky's own colour.
        let card = surface
            .ops()
            .iter()
            .any(|op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == argb(fill, 1.0)));
        assert!(
            card,
            "a sticky must paint a coloured card in the geometry pass"
        );
        // A drop shadow is painted behind the card, in the translucent shadow colour.
        let shadow = surface.ops().iter().any(
            |op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == argb(STICKY_SHADOW, 1.0)),
        );
        assert!(shadow, "a sticky must paint a drop shadow behind its card");
        // Any glyphs the body produces use the fixed ink, never the card colour
        // (the ink is independent of `fill`, §2.C). Font-availability agnostic:
        // if no glyphs are shaped this simply holds vacuously.
        let card_color = argb(fill, 1.0);
        let ink = argb(STICKY_INK, 1.0);
        for op in surface.ops() {
            if let DrawOp::Glyphs { color: Some(c), .. } = op {
                assert_eq!(*c, ink, "sticky body must use the ink, not the card colour");
                assert_ne!(*c, card_color);
            }
        }
    }

    #[test]
    fn sticky_font_size_auto_unless_overridden() {
        let bounds = Rect::new(0.0, 0.0, 120.0, 120.0);
        let auto = ElementStyle {
            font: ambre_bus::app::toile::FontSpec {
                size: 0.0,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        // Sticky with the auto sentinel -> size derived from the card.
        assert_eq!(
            sticky_or_style_font_size(true, &auto, bounds),
            sticky_font_size(bounds)
        );
        // A positive size overrides the auto-sizing.
        let manual = ElementStyle {
            font: ambre_bus::app::toile::FontSpec {
                size: 30.0,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        assert_eq!(sticky_or_style_font_size(true, &manual, bounds), 30.0);
        // A text element always uses its style size, never auto.
        assert_eq!(sticky_or_style_font_size(false, &auto, bounds), 0.0);
    }

    #[test]
    fn text_ink_precedence() {
        // Explicit text_color always wins, for sticky and text alike.
        assert_eq!(
            resolved_text_ink(true, Some(0x1234), Some(0x9999)),
            Some(0x1234)
        );
        assert_eq!(
            resolved_text_ink(false, Some(0x1234), Some(0x9999)),
            Some(0x1234)
        );
        // No text_color: a sticky uses its ink; a text element uses its fill.
        assert_eq!(
            resolved_text_ink(true, None, Some(0x9999)),
            Some(STICKY_INK)
        );
        assert_eq!(resolved_text_ink(false, None, Some(0x9999)), Some(0x9999));
        // A text element with neither falls back to the theme (None).
        assert_eq!(resolved_text_ink(false, None, None), None);
    }

    #[test]
    fn sticky_font_size_tracks_the_card_and_is_clamped() {
        // Bigger card -> bigger font.
        let small = sticky_font_size(Rect::new(0.0, 0.0, 60.0, 60.0));
        let big = sticky_font_size(Rect::new(0.0, 0.0, 240.0, 240.0));
        assert!(big > small, "font must grow with the note");
        // Clamped to a legible range at the extremes.
        assert_eq!(sticky_font_size(Rect::new(0.0, 0.0, 4.0, 4.0)), 8.0);
        assert_eq!(sticky_font_size(Rect::new(0.0, 0.0, 4000.0, 4000.0)), 48.0);
        // A square 120x120 (the default) sits in-range.
        let default = sticky_font_size(Rect::new(0.0, 0.0, 120.0, 120.0));
        assert!((8.0..=48.0).contains(&default));
    }

    // A free `Text` element has no card: the geometry pass paints nothing for it,
    // so it emits no FillPath (only its glyphs, in the text pass). This is what
    // keeps Sticky and Text visually distinct (toile-logic §2.C).
    #[test]
    fn text_element_paints_no_card() {
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![styled_elem(
                "t",
                ElementKind::Text,
                DisplayGeom::Text("hi".into()),
                Some(0xFF00_00FF),
            )],
        };
        let surface = paint(&vp);
        let fills = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::FillPath { .. }))
            .count();
        assert_eq!(fills, 0, "a text element must not paint a card background");
    }

    // An `Image` element whose hash is in the store paints real pixels
    // (`DrawImage`), scaled to its AABB; one whose hash is absent falls back to
    // a placeholder box (a path fill), per toile-interop §5.B.
    #[test]
    fn image_paints_pixels_when_resolved_else_placeholder() {
        let bounds = Rect::new(0.0, 0.0, 40.0, 20.0);
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("has", DisplayGeom::Image("h1".into()), bounds),
                elem("missing", DisplayGeom::Image("h2".into()), bounds),
            ],
        };
        // Only "h1" is decoded; "h2" is still pending.
        let mut images = ImageStore::new();
        images.insert(
            "h1".into(),
            DecodedImage {
                pixels: vec![255u8; 2 * 2 * 4],
                width: 2,
                height: 2,
            },
        );
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        paint_content(
            &mut surface,
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            &mut font_cx,
            &mut tc,
            None,
            &images,
            &HashMap::new(),
            false,
        );

        // Exactly one real image paint, sized to the element AABB.
        let draws: Vec<_> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::DrawImage {
                    width,
                    height,
                    dest,
                } => Some((*width, *height, *dest)),
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 1, "one resolved image paints pixels");
        assert_eq!(draws[0], (2, 2, bounds));
        // The unresolved image falls back to a placeholder path fill.
        let fills = surface.count(|o| matches!(o, DrawOp::FillPath { .. }));
        assert_eq!(fills, 1, "the pending image paints a placeholder box");
        assert_eq!(surface.transform_depth(), 0);
    }

    #[test]
    fn overlay_draws_box_and_handles_for_single_selection() {
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![elem(
                "r",
                DisplayGeom::Rect,
                Rect::new(0.0, 0.0, 50.0, 50.0),
            )],
        };
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[ElementId("r".into())],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
        );
        // 8 resize handles + 1 rotate handle = 9 filled squares.
        let handle_fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handle_fills, 9);
    }

    #[test]
    fn overlay_outlines_visible_find_hits() {
        let vp = DisplayViewport {
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
                elem("b", DisplayGeom::Rect, Rect::new(60.0, 0.0, 110.0, 50.0)),
            ],
        };
        let mut surface = RecordingSurface::new();
        let hits = vec![ElementId("a".into()), ElementId("b".into())];
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &hits,
            Some(&ElementId("b".into())),
        );
        // No selection => no handles, only the two hit outlines (one per match).
        let strokes = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::StrokeRect { .. }))
            .count();
        assert_eq!(strokes, 2);
    }

    // A minimal palette for host tests (the generated LIGHT lives behind the
    // canvas build script; construct the fields the renderer reads).
    const LIGHT_PALETTE: Palette = Palette {
        content_bg: Color::from_rgba8(255, 255, 255, 255),
        content_pending: Color::from_rgba8(235, 237, 240, 255),
        header_bg: Color::from_rgba8(242, 243, 245, 255),
        header_active: Color::from_rgba8(245, 231, 200, 255),
        header_hover: Color::from_rgba8(230, 232, 236, 255),
        grid_line: Color::from_rgba8(210, 213, 218, 255),
        header_border: Color::from_rgba8(184, 188, 196, 255),
        selection: Color::from_rgba8(176, 111, 10, 255),
        cell_text: Color::from_rgba8(32, 32, 32, 255),
        header_text: Color::from_rgba8(95, 99, 104, 255),
        caret: Color::from_rgba8(20, 20, 20, 255),
        cell_error: Color::from_rgba8(204, 68, 68, 255),
    };
}
