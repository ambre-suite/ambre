// ambre-toile/src/ui/render.rs
// ============================================================================
// The Toile content walk (toile-board-rendering). Toile owns only "what to draw":
// it walks the culled, z-ordered `DisplayViewport` and emits ABSTRACT
// `DrawSurface` calls. Tessellation, the glyph atlas and GPU submission are
// `ambre-canvas`'s job. So the walk is host-testable against `RecordingSurface`
// (a mock `DrawSurface`): assert the ordered `DrawOp` sequence with no GPU.
//
// Two passes:
//   paint_content -- board bg + adaptive grid, then one z-ordered walk of
//                    the nodes under the camera transform (shapes, paths,
//                    strokes, connectors). Then a screen-space text pass.
//   paint_overlay -- selection bounding box + transform handles (screen space).
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{
    ConnectorId, DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport,
    ElementId, ElementStyle, ShapeKind,
};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DrawSurface, FontContext, ImageKey, ImageSource,
    Point, PositionedLayoutItem, Rect, Size, Stroke,
};

use crate::logic::animate::AnimatedElement;

use super::grid;
use super::hit::triangle_path;
use super::tool::{
    HANDLE_SIZE_PX, Preview, is_group, resize_handle_points, rotate_handle_point, selection_bounds,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;

/// The media addresses a display list names and the view cannot paint: not in
/// the store, not already asked for, each listed once.
///
/// It is the culled list that is walked, so only what survived the viewport
/// query is asked for. A board of two hundred photographs transfers the ones
/// on screen (`toile-media`).
pub fn missing_media(
    nodes: &[DisplayNode],
    images: &ImageStore,
    requested: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for node in nodes {
        let DisplayNode::Element(de) = node else {
            continue;
        };
        let DisplayGeom::Image(hash) = &de.geom else {
            continue;
        };
        if images.contains_key(hash) || requested.contains(hash) || out.iter().any(|h| h == hash) {
            continue;
        }
        out.push(hash.clone());
    }
    out
}

/// A decoded raster image ready to paint: non-premultiplied RGBA8 pixels
/// (row-major, `width * height * 4` bytes) plus native dimensions.
///
/// Keyed by the element's blake2b media hash (`DisplayGeom::Image`). A hash
/// absent from the map means the pixels have not arrived. The element paints a
/// placeholder box instead until a later frame carries them.
///
/// **Only the PDF export fills this today**, from the thumbnail each entry
/// carries in the `media` LoroMap (`toile-media`). The screen path builds it in
/// `AppView::new` and never inserts, so every picture on every board paints the
/// placeholder. `WgpuSurface` implements no `draw_image` either, so a resident
/// entry would paint nothing at all. Both halves land together
/// (`ambre-roadmap.md` E6).
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Resolved image pixels for this frame, keyed by media hash. Empty when no
/// image element is present (or its media has not been decoded yet).
pub type ImageStore = HashMap<String, DecodedImage>;

/// Fixed ink (ARGB) for a sticky-note body: a near-black that stays readable on
/// any pastel card colour. The body is legible regardless of the card fill
/// (`toile-styles`). Independent of the theme (the card is not). Shared with
/// the inline editor so an in-edit sticky uses the same ink (`edit.rs`).
pub(crate) const STICKY_INK: u32 = 0xFF1A_1A1A;

/// Sticky drop-shadow colour (ARGB): a soft translucent black offset to the
/// bottom-right so the note reads as a physical card (feedback).
const STICKY_SHADOW: u32 = 0x3300_0000;
/// Drop-shadow offset in world units (bottom-right).
const STICKY_SHADOW_OFFSET: f64 = 4.0;

/// The body font size (world units) for a sticky of the given bounds. It scales
/// with the note so the text "adapts to the size of the note" (feedback). A
/// fraction of the shorter side, clamped to a legible range. Shared with the
/// inline editor so the in-edit size matches the committed one (`edit.rs`).
pub(crate) fn sticky_font_size(bounds: Rect) -> f32 {
    let shorter = bounds.width().min(bounds.height()) as f32;
    (shorter * 0.16).clamp(8.0, 48.0)
}

/// The effective body font size (world units) for a sticky / text element. A sticky
/// auto-sizes from its bounds while `style.font.size` is the auto sentinel
/// (`STICKY_AUTO_FONT_SIZE`, that is, not positive). A positive size set from the
/// style inspector overrides it. A text element always uses its style size.
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
/// text colour. An explicit `text_color` always wins. Otherwise a sticky uses its
/// fixed ink (its `fill` is the card background, not the text). A free text element
/// falls back to its `fill` (legacy: a text element's fill was its glyph colour)
/// then, if unset, the theme colour. Pure and host-tested; shared with the inline
/// editor via the same precedence in `edit.rs`.
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

/// Convert an ARGB `u32` (Toile style register,) to a `peniko::Color`,
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
    let mut p = BezPath::with_capacity(5);
    p.move_to((b.x0, b.y0));
    p.line_to((b.x1, b.y0));
    p.line_to((b.x1, b.y1));
    p.line_to((b.x0, b.y1));
    p.close_path();
    p
}

/// The control-point distance of a cubic quarter circle, as a share of r.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// The widest radius four cubics draw within 0.1 of the true ellipse. Their
/// error is 0.027 % of the radius.
const ELLIPSE_FOUR_CUBICS_MAX_R: f64 = 370.0;

/// An axis-aligned ellipse inscribed in `b`, as a path.
///
/// Up to `ELLIPSE_FOUR_CUBICS_MAX_R`, four cubics are written directly. kurbo
/// pays a `sincos` per arc, the largest cost of a busy board (`CANVAS-005`).
fn ellipse_path(b: Rect) -> BezPath {
    let c = b.center();
    let (rx, ry) = (b.width().abs() / 2.0, b.height().abs() / 2.0);
    if rx.max(ry) > ELLIPSE_FOUR_CUBICS_MAX_R {
        let ell = kurbo::Ellipse::new(c, (rx, ry), 0.0);
        let mut p = BezPath::new();
        p.extend(kurbo::Shape::path_elements(&ell, 0.1));
        return p;
    }
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let mut p = BezPath::with_capacity(6);
    p.move_to((c.x + rx, c.y));
    p.curve_to((c.x + rx, c.y + ky), (c.x + kx, c.y + ry), (c.x, c.y + ry));
    p.curve_to((c.x - kx, c.y + ry), (c.x - rx, c.y + ky), (c.x - rx, c.y));
    p.curve_to((c.x - rx, c.y - ky), (c.x - kx, c.y - ry), (c.x, c.y - ry));
    p.curve_to((c.x + kx, c.y - ry), (c.x + rx, c.y - ky), (c.x + rx, c.y));
    p.close_path();
    p
}

/// A rounded rectangle inscribed in `b`, as a path. Used for the
/// sticky-note card and its shadow; with `radius = 0` (the sticky default) it is
/// a plain rectangle. The radius is clamped to half the shorter side so a tiny
/// card stays a valid shape.
///
/// Each corner is one cubic quarter-arc, written directly. kurbo's arcs cost a
/// `sincos` per corner even at radius zero, which was half of a busy board's
/// frame (`CANVAS-005`).
fn roundrect_path(b: Rect, radius: f64) -> BezPath {
    let r = radius.min(b.width() / 2.0).min(b.height() / 2.0).max(0.0);
    let (x0, y0, x1, y1) = (b.x0, b.y0, b.x1, b.y1);
    let mut p = BezPath::with_capacity(10);
    if r <= 0.0 {
        p.move_to((x0, y0));
        p.line_to((x1, y0));
        p.line_to((x1, y1));
        p.line_to((x0, y1));
        p.close_path();
        return p;
    }
    let k = r * (1.0 - KAPPA);
    p.move_to((x0 + r, y0));
    p.line_to((x1 - r, y0));
    p.curve_to((x1 - k, y0), (x1, y0 + k), (x1, y0 + r));
    p.line_to((x1, y1 - r));
    p.curve_to((x1, y1 - k), (x1 - k, y1), (x1 - r, y1));
    p.line_to((x0 + r, y1));
    p.curve_to((x0 + k, y1), (x0, y1 - k), (x0, y1 - r));
    p.line_to((x0, y0 + r));
    p.curve_to((x0, y0 + k), (x0 + k, y0), (x0 + r, y0));
    p.close_path();
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

/// Stroke width, in screen pixels, of the line traced over a selected arrow.
/// Heavier than the arrow itself so the selection reads at any zoom.
const ARROW_SELECT_W: f64 = 3.0;

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
    editing: Option<&super::edit::EditTarget>,
    // Decoded image pixels for this frame, keyed by media hash. An `Image`
    // element whose hash is present paints real pixels via `draw_image`; one
    // absent paints a placeholder box (`toile-media`). Empty on a board
    // with no images or whose media has not yet been decoded.
    images: &ImageStore,
    // Smart Animate overrides for an in-flight slide transition. Keyed by element
    // id, each replaces the element's transform / bounds / opacity / fill for
    // this frame. Empty when no transition is active. Fade-only ids (a new / gone
    // element) still appear here so their opacity ramp is honored.
    anim: &HashMap<ElementId, AnimatedElement>,
    // Presentation focus mode: the UI hides the board grid so the audience
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
    // Both sticky notes and free text carry a body string here. A sticky draws
    // its body in a fixed readable ink (its `fill` is the card colour); a text
    // element draws in its `fill`.
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
        // editor overlay instead.
        if editing.and_then(|t| t.element()) == Some(&e.id) {
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

    // 3b. Frame titles, in the same screen-space pass, so a title reads the
    // same at every zoom. A presentation shows the board's content and not its
    // structure, so the mode drops them with the grid (`TOILE-255`).
    if !present {
        paint_frame_titles(surface, vp, cam, palette, font_cx, text_cache, editing);
    }

    // 4. Connector captions, in the same screen-space pass. They come after the
    // element bodies so a caption stays readable over whatever the arrow
    // crosses (`toile-connectors`).
    for node in &vp.nodes {
        let DisplayNode::Connector(c) = node else {
            continue;
        };
        if c.label.is_empty() || editing.and_then(|t| t.connector()) == Some(&c.id) {
            continue;
        }
        paint_label(surface, c, cam, palette, font_cx, text_cache);
    }
}

/// The board-coloured plate a caption sits on, so the words are readable where
/// they cross the arrow. Shared by the static pass and the inline editor, which
/// is why it is not inlined into either.
pub(crate) fn paint_label_plate<S: DrawSurface>(surface: &mut S, plate: Rect, palette: &Palette) {
    surface.fill_path(
        &roundrect_path(plate, 3.0),
        &Brush::Solid(palette.content_bg),
    );
}

/// Draw a connector's caption in screen space, centred on the arrow's mid-arc
/// point. A plate in the board colour keeps the words readable where they cross
/// the line they belong to (`toile-connectors`).
fn paint_label<S: DrawSurface>(
    surface: &mut S,
    c: &DisplayConnector,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) {
    let Some(world) = super::label::label_rect(&c.path) else {
        return;
    };
    let sb = world_rect_to_screen(world, cam);
    let pad = super::edit::InlineEditor::pad(cam.zoom);
    let wrap_w = (sb.width() - 2.0 * pad).max(1.0) as f32;
    let font_size = (super::label::LABEL_FONT_SIZE as f64 * cam.zoom) as f32;
    let style = ElementStyle::default();
    let layout = text_cache.get_or_build_font(
        font_cx,
        &c.label,
        font_size,
        wrap_w,
        Alignment::Center,
        false,
        false,
        Some(style.font.family.as_str()),
        false,
    );
    // The plate hugs the laid-out text rather than the (fixed) caption box, so a
    // short caption does not punch a wide hole in the board.
    let (tw, th) = (layout.width() as f64, layout.height() as f64);
    paint_label_plate(
        surface,
        Rect::from_center_size(sb.center(), Size::new(tw + 2.0 * pad, th + 2.0 * pad)),
        palette,
    );
    let brush = Brush::Solid(palette.cell_text);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = sb.x0 + pad + run.offset() as f64;
            let ty = sb.y0 + pad + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// The world transform of an element whose geometry an animation override
/// replaces: rotation about the (interpolated) AABB center. This mirrors
/// [`display_element`](crate::logic::state), so the tween matches the static
/// pass.
fn anim_transform(a: &AnimatedElement) -> Affine {
    let c = a.bounds.center();
    Affine::translate(c.to_vec2()) * Affine::rotate(a.rotation) * Affine::translate(-c.to_vec2())
}

/// A rotation of `theta` radians about `center` (screen space). The camera carries
/// no rotation (pan + zoom only), so an element's world rotation is also its
/// screen rotation. The text / overlay passes, which run in screen space, rotate
/// about the element's screen-space center with this affine.
pub(crate) fn rotate_about(center: Point, theta: f64) -> Affine {
    Affine::translate(center.to_vec2())
        * Affine::rotate(theta)
        * Affine::translate(-center.to_vec2())
}

/// The screen-space rotation (radians) of an element's text: the live gesture /
/// animation angle when overridden, else the angle baked into its display
/// transform. That transform is a pure rotation about the AABB center, so `atan2(b,
/// a)` recovers it.
fn text_rotation(e: &DisplayElement, anim: Option<&AnimatedElement>) -> f64 {
    match anim {
        Some(a) => a.rotation,
        None => {
            let c = e.transform.as_coeffs();
            c[1].atan2(c[0])
        }
    }
}

/// The screen-space rotation (radians) to apply to the selection box + handles of
/// a single selected element. That is its live gesture angle when a resize /
/// rotate / move is in flight, else the angle baked into its display transform.
/// `0.0` when the id is not in the culled set.
fn selection_rotation(
    id: &ElementId,
    vp: &DisplayViewport,
    gesture: &HashMap<ElementId, AnimatedElement>,
) -> f64 {
    if let Some(a) = gesture.get(id) {
        return a.rotation;
    }
    vp.nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Element(e) if &e.id == id => {
                let c = e.transform.as_coeffs();
                Some(c[1].atan2(c[0]))
            }
            _ => None,
        })
        .unwrap_or(0.0)
}

/// The style an element paints with this frame: its own, or the base overlaid
/// with the animation override's opacity / fill. Only those visual attributes
/// interpolate; the rest of the register is held.
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
/// `Some`, the element is mid-transition. Its transform, bounds and opacity /
/// fill come from the Smart Animate override instead of the static display.
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
        DisplayGeom::Rect | DisplayGeom::Frame(_) => {
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
        // the element AABB (the rect is stored at the imported aspect). A hash
        // not yet in the store paints a placeholder box until a later frame
        // carries the media. `draw_image` maps through the active element
        // transform, so a rotated image rotates with its box.
        DisplayGeom::Image(hash) => match images.get(hash) {
            Some(img) => surface.draw_image(
                &ImageSource {
                    // The media address already identifies these pixels, so the
                    // back end recognises the texture it uploaded on an earlier
                    // frame without hashing the buffer again.
                    key: ImageKey::from_address(hash),
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
        // the body string is drawn on top in the screen-space text pass. A
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
        DisplayGeom::Group => {} // paints nothing
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
    // Mid-transition, the body follows the tweened bounds and opacity; the
    // font attributes and (when unset) color are held from the static style.
    let bounds = anim.map(|a| a.bounds).unwrap_or(e.bounds);
    let opacity = anim.map(|a| a.opacity).unwrap_or(e.style.opacity);
    let fill = anim.map(|a| a.fill).unwrap_or(e.style.fill);
    let sb = world_rect_to_screen(bounds, cam);
    // A sticky's font tracks the note size unless the style carries an explicit
    // override (size > 0); a text element uses its own style size. Zoom is applied
    // on top for crisp glyphs.
    let base_size = sticky_or_style_font_size(is_sticky, &e.style, bounds);
    let font_size = (base_size as f64 * cam.zoom) as f32;
    // Body inset: the same padding the inline editor applies, so the static text
    // and the in-edit text share one origin + wrap width. The body does not jump
    // / re-wrap when entering or leaving edit. Single source of truth.
    let pad = super::edit::InlineEditor::pad(cam.zoom);
    let wrap_w = (sb.width() - 2.0 * pad).max(1.0) as f32;
    // Text (foreground) colour, (see `resolved_text_ink`): `None` means "use
    // the theme text colour" (only a text element with neither text_color nor fill).
    let color = resolved_text_ink(is_sticky, e.style.text_color, fill)
        .map(|c| argb(c, opacity))
        .unwrap_or(palette.cell_text);
    let layout = text_cache.get_or_build_font(
        font_cx,
        text,
        font_size,
        wrap_w,
        Alignment::Start,
        e.style.font.weight >= 700,
        e.style.font.italic,
        Some(e.style.font.family.as_str()),
        false,
    );
    let brush = Brush::Solid(color);
    let underline = e.style.font.underline;
    let strikethrough = e.style.font.strikethrough;
    // A rotated element rotates its body too: the glyph run origins and the
    // decoration endpoints are mapped through a rotation about the note's
    // screen-space center. The text follows the card instead of staying
    // axis-aligned (toile-board-rendering). `None` (no rotation) keeps the fast
    // pure-translation path.
    let rot = text_rotation(e, anim);
    let rot_tf = (rot != 0.0).then(|| rotate_about(sb.center(), rot));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = sb.x0 + pad + run.offset() as f64;
            let ty = sb.y0 + pad + run.baseline() as f64;
            let base = Affine::translate((tx, ty));
            let glyph_tf = rot_tf.map_or(base, |r| r * base);
            surface.draw_glyphs(&run, &brush, glyph_tf);
            // Per-run text decorations, drawn in the glyph ink (matches Trame's
            // cell decorations). A 1px underline sits just below the baseline and a
            // 1px strikethrough through the x-height (~0.3em above the baseline).
            // The endpoints ride the same rotation so the rules stay glued to the
            // text.
            if underline || strikethrough {
                let adv = run.advance() as f64;
                let mut line = |ay: f64| {
                    let (mut a, mut b) = (Point::new(tx, ay), Point::new(tx + adv, ay));
                    if let Some(r) = rot_tf {
                        a = r * a;
                        b = r * b;
                    }
                    surface.stroke_line(a.x, a.y, b.x, b.y, color, 1.0);
                };
                if underline {
                    line(ty + 2.0);
                }
                if strikethrough {
                    line(ty - (font_size as f64) * 0.3);
                }
            }
        }
    }
}

/// Paint the transient overlay: find highlights, the selection bounding box + 8
/// resize handles + the rotate handle, in screen space (`toile-selection`, pass 3).
/// The selection box is skipped for an empty selection, but find
/// highlights are painted regardless (a search can be active with nothing
/// selected).
///
/// `present` is presentation mode. It skips every editing surface of this pass:
/// the gesture preview, the arrow box, the find highlights, the selection box
/// and its handles (`TOILE-UI-083`). A slide is what the room is looking at.
/// The link badges stay: following a link is navigation, and a presenter uses
/// it.
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
    // Live transform overrides for an in-flight resize / rotate / move. When an
    // entry exists for a selected id, the selection box + handles follow its
    // preview bounds instead of the committed geometry. They stay glued to the
    // object during the drag. Empty at rest.
    gesture: &HashMap<ElementId, AnimatedElement>,
    // The selected arrow, if one is (`toile-connectors`). Exclusive with
    // `selection`, so at most one of the two boxes is drawn.
    arrow: Option<&ConnectorId>,
    present: bool,
) {
    if present {
        // Navigation only: a badge says a slide carries a link.
        paint_link_badges(surface, vp, cam, palette);
        return;
    }
    // Live gesture preview (world -> screen), a thin outline / trail.
    if let Some(p) = preview {
        paint_preview(surface, p, cam, palette);
    }

    // A selected arrow gets its line traced in the selection colour and a handle
    // on each end, which is what a re-aim grabs (`toile-connectors`).
    if let Some(id) = arrow {
        paint_arrow_selection(surface, id, vp, cam, palette);
    }

    // Link badges (`toile-links`): one per linked element, in screen space so
    // they keep their size through the zoom. They are always painted rather
    // than shown on hover: a link nobody can see is a link nobody follows.
    paint_link_badges(surface, vp, cam, palette);

    // Find highlights: outline every matching element visible in the
    // culled set, with a heavier outline on the focused hit. Off-screen hits
    // simply have no box until the camera (recentered on `focus`) brings them in.
    paint_find_highlights(surface, find_hits, find_focus, vp, cam, palette);

    let Some(wb) = selection_preview_bounds(selection, vp, gesture) else {
        return;
    };
    let b = world_rect_to_screen(wb, cam);
    // A single selected element carries the box + handles rotated with it; a
    // multi-selection keeps an axis-aligned union box (no shared rotation). The
    // camera has no rotation, so the element's world angle is also its screen
    // angle.
    //
    // The rotation is applied by mapping each corner / handle point through a
    // screen-space rotation about `b`'s center, NOT via `push_transform`. The
    // WgpuSurface's `stroke_rect` / `fill_rect` degrade a rotated rect to its
    // axis-aligned bounding box, since they only carry an AABB. A pushed rotation
    // would paint a bigger upright box instead. `stroke_line` maps its endpoints
    // through the transform faithfully, so the box is drawn as four rotated edges.
    // Each handle square is drawn axis-aligned (screen-aligned, Miro-style) at its
    // rotated center.
    let single = match selection {
        [only] => Some(only),
        _ => None,
    };
    let theta = single.map_or(0.0, |only| selection_rotation(only, vp, gesture));
    let place = |p: Point| {
        if theta == 0.0 {
            p
        } else {
            rotate_about(b.center(), theta) * p
        }
    };

    // Bounding box. Axis-aligned: the crisp `stroke_rect` fast path. Rotated: four
    // `stroke_line` edges between the rotated corners (stroke_rect would degrade to
    // an upright AABB on the WgpuSurface, see above).
    if theta == 0.0 {
        surface.stroke_rect(b, palette.selection, 1.5);
    } else {
        let c = [
            place(Point::new(b.x0, b.y0)),
            place(Point::new(b.x1, b.y0)),
            place(Point::new(b.x1, b.y1)),
            place(Point::new(b.x0, b.y1)),
        ];
        for (a, d) in c.iter().zip(c.iter().cycle().skip(1)) {
            surface.stroke_line(a.x, a.y, d.x, d.y, palette.selection, 1.5);
        }
    }

    // Rotate handle: a short stem above the top edge + the knob.
    let stem_base = place(Point::new(b.center().x, b.y0));
    let knob = place(rotate_handle_point(b));
    surface.stroke_line(
        stem_base.x,
        stem_base.y,
        knob.x,
        knob.y,
        palette.selection,
        1.0,
    );
    draw_handle(surface, knob, palette);

    // 4 corner resize handles (only for a single-element selection, matching the
    // tool's corners-only picking; a multi-selection shows the box only in Phase 0).
    // A group wears none: its box is the union of its members (`TOILE-UI-391`).
    if single.is_some_and(|only| !is_group(only, vp)) {
        for (_, p) in resize_handle_points(b) {
            draw_handle(surface, place(p), palette);
        }
    }
}

/// Paint an in-flight gesture preview. Marquee is a thin screen-space outline.
/// The shape / connector / pen / stroke previews are drawn in world space under
/// the camera transform with a zoom-compensated 1.5 px line. They track the
/// geometry they will commit to.
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

/// World union AABB of the selection for the overlay box, honoring any in-flight
/// gesture override. A selected id present in `gesture` contributes its live
/// preview bounds, otherwise its committed display bounds. `None` when nothing
/// selected is in the culled set.
fn selection_preview_bounds(
    selection: &[ElementId],
    vp: &DisplayViewport,
    gesture: &HashMap<ElementId, AnimatedElement>,
) -> Option<Rect> {
    if gesture.is_empty() {
        return selection_bounds(selection, vp);
    }
    let mut acc: Option<Rect> = None;
    for id in selection {
        let b = gesture
            .get(id)
            .map(|a| a.bounds)
            .or_else(|| element_bounds(vp, id));
        if let Some(b) = b {
            acc = Some(match acc {
                Some(r) => r.union(b),
                None => b,
            });
        }
    }
    acc
}

/// Outline the find matches present in the culled set. Every hit gets
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

/// Trace a selected arrow and put a handle on each end. The arrow has no box.
/// A rectangle around a diagonal line would cover half the diagram and say
/// nothing about which end a drag would move. The line itself is the
/// selection feedback (`toile-connectors`).
fn paint_arrow_selection<S: DrawSurface>(
    surface: &mut S,
    id: &ConnectorId,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
) {
    let Some(path) = super::arrow::path_of(vp, id) else {
        return; // the arrow left the culled set, or an endpoint is gone
    };
    let Some((from, to)) = super::arrow::endpoints(path) else {
        return;
    };
    surface.push_transform(cam.world_to_screen());
    surface.stroke_path(
        path,
        &Brush::Solid(palette.selection),
        &Stroke::new(ARROW_SELECT_W / cam.zoom.max(1e-3)),
    );
    surface.pop_transform();
    let w2s = cam.world_to_screen();
    draw_handle(surface, w2s * from, palette);
    draw_handle(surface, w2s * to, palette);
}

/// Draw the badge of every linked element in the culled frame. Each is a
/// small filled square outside the top-right corner, with a chain-link notch
/// cut as two short strokes. The notch makes it read as a link and not as a
/// resize handle (`toile-links`).
fn paint_link_badges<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
) {
    for e in super::link::linked(vp) {
        let b = super::link::badge_rect(world_rect_to_screen(e.bounds, cam));
        surface.fill_path(&roundrect_path(b, 3.0), &Brush::Solid(palette.selection));
        // Two short bars on the diagonal: the smallest mark that reads as a
        // chain at this size.
        let (cx, cy) = (b.center().x, b.center().y);
        let r = b.width() * 0.22;
        surface.stroke_line(
            cx - r * 1.6,
            cy + r,
            cx + r * 0.4,
            cy - r,
            palette.content_bg,
            1.5,
        );
        surface.stroke_line(
            cx - r * 0.4,
            cy + r,
            cx + r * 1.6,
            cy - r,
            palette.content_bg,
            1.5,
        );
    }
}

/// Height of a frame's title band in screen pixels, and the font size inside
/// it (`TOILE-253`). Screen space, so a title reads the same at every zoom,
/// like the link badges and the comment pins.
pub const FRAME_TITLE_PX: f64 = 14.0;

/// Gap between the title's baseline band and the frame's own top edge.
const FRAME_TITLE_GAP: f64 = 4.0;

/// What a frame's band shows, and whether it is the frame's own title
/// (`TOILE-253`).
///
/// A frame nobody named shows a placeholder instead of nothing, so there is
/// always something to read and something to double-click. The placeholder
/// names no rank. The deck panel numbers a frame by its place in the deck, and
/// a second numbering here would call one frame two different things.
pub fn frame_title_label(title: &str) -> (&str, bool) {
    let title = title.trim();
    if title.is_empty() {
        ("Untitled frame", false)
    } else {
        (title, true)
    }
}

/// Every frame of the visible set, with the band it draws and what is in it, in
/// paint order (`TOILE-253`). One source for the painter, the hit test and the
/// editor, so the three cannot disagree on where a title is.
pub fn frame_bands<'a>(
    nodes: &'a [DisplayNode],
    cam: &'a ViewportTransform,
) -> impl Iterator<Item = (&'a DisplayElement, Rect, &'a str, bool)> {
    nodes.iter().filter_map(move |node| {
        let DisplayNode::Element(e) = node else {
            return None;
        };
        let DisplayGeom::Frame(title) = &e.geom else {
            return None;
        };
        let (label, named) = frame_title_label(title);
        let band = frame_title_box(world_rect_to_screen(e.bounds, cam), label)?;
        Some((e, band, label, named))
    })
}

/// The screen-space box a frame's title occupies, above its top-left corner
/// (`TOILE-253`), or `None` when there is nothing to draw.
///
/// `screen` is the frame's box already projected. The band sits outside the
/// frame, so it never covers the content a user put inside it. Its width is an
/// estimate of the glyphs. The exact advance is known only once the text is
/// shaped, and the hit target has to exist without a font at all
/// (`TOILE-254`).
pub fn frame_title_box(screen: Rect, title: &str) -> Option<Rect> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    // ~0.6 em per character, the same estimate the SVG import seeds a text box
    // with. A title is one short line, so the error is a few pixels of hit
    // target rather than a layout.
    let w = (title.chars().count() as f64) * FRAME_TITLE_PX * 0.6;
    let bottom = screen.y0 - FRAME_TITLE_GAP;
    Some(Rect::new(
        screen.x0,
        bottom - FRAME_TITLE_PX,
        screen.x0 + w,
        bottom,
    ))
}

/// Paint each visible frame's title above it, in screen space (`TOILE-253`).
///
/// Skipped in presentation mode: a slide shows the board's content, not the
/// board's structure (`TOILE-255`).
fn paint_frame_titles<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    editing: Option<&super::edit::EditTarget>,
) {
    let editing = editing.and_then(|t| t.frame_title());
    for (e, band, label, named) in frame_bands(&vp.nodes, cam) {
        // The title under edit is painted by the editor overlay instead, as an
        // element's body is (`TOILE-261`).
        if editing == Some(&e.id) {
            continue;
        }
        let layout = text_cache.get_or_build_font(
            font_cx,
            label,
            FRAME_TITLE_PX as f32,
            f32::MAX,
            Alignment::Start,
            false,
            false,
            None,
            false,
        );
        // A placeholder reads as the absence it is: the secondary ink, not the
        // ink of a title somebody typed.
        let brush = Brush::Solid(if named {
            palette.cell_text
        } else {
            palette.header_text
        });
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let tx = band.x0 + run.offset() as f64;
                let ty = band.y0 + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
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
            link: None,
            group: None,
        })
    }

    fn cam() -> ViewportTransform {
        ViewportTransform::new(1000.0, 800.0)
    }

    /// The direct cubics draw the shape kurbo's arcs drew: the same box and,
    /// to a thousandth, the same area, at every radius a card takes.
    #[test]
    fn a_rounded_card_keeps_the_shape_of_its_arcs() {
        use kurbo::Shape;
        let b = Rect::new(10.0, 20.0, 210.0, 140.0);
        for radius in [0.0, 3.0, 12.0, 60.0, 500.0] {
            let ours = roundrect_path(b, radius);
            let r = radius.min(60.0);
            let theirs = kurbo::RoundedRect::from_rect(b, r).to_path(0.1);
            let (o, t) = (ours.bounding_box(), theirs.bounding_box());
            let drift = [o.x0 - t.x0, o.y0 - t.y0, o.x1 - t.x1, o.y1 - t.y1];
            assert!(
                drift.iter().all(|d| d.abs() < 1e-9),
                "radius {radius}: {o:?}"
            );
            let (a, e) = (ours.area().abs(), theirs.area().abs());
            assert!((a - e).abs() / e < 1e-3, "radius {radius}: {a} against {e}");
        }
    }

    /// The four direct cubics draw the ellipse kurbo's arcs drew. The box holds
    /// within 0.1 and the area to a thousandth, on both sides of the switch.
    #[test]
    fn an_ellipse_keeps_the_shape_of_its_arcs() {
        use kurbo::Shape;
        for (w, h) in [(20.0, 20.0), (200.0, 80.0), (740.0, 300.0), (2000.0, 900.0)] {
            let b = Rect::new(10.0, 20.0, 10.0 + w, 20.0 + h);
            let ours = ellipse_path(b);
            let ell = kurbo::Ellipse::new(b.center(), (w / 2.0, h / 2.0), 0.0);
            let theirs = ell.to_path(0.1);
            let (o, t) = (ours.bounding_box(), theirs.bounding_box());
            let drift = [o.x0 - t.x0, o.y0 - t.y0, o.x1 - t.x1, o.y1 - t.y1];
            assert!(drift.iter().all(|d| d.abs() < 0.1), "{w}x{h}: {o:?}");
            let (a, e) = (ours.area().abs(), theirs.area().abs());
            assert!((a - e).abs() / e < 1e-3, "{w}x{h}: {a} against {e}");
        }
    }

    /// `TOILE-253`: a named frame carries a title band above its own top-left
    /// corner, outside the frame, so it never covers what a user put inside.
    /// Screen space, so it keeps one size through the zoom.
    #[test]
    fn a_named_frame_reserves_a_title_band_above_itself() {
        let screen = Rect::new(100.0, 200.0, 400.0, 500.0);
        let band = frame_title_box(screen, "Agenda").expect("a named frame has a band");
        assert!(
            band.y1 <= screen.y0,
            "the title covers the frame: {band:?} over {screen:?}"
        );
        assert_eq!(band.x0, screen.x0, "the band starts at the frame's corner");
        assert_eq!(band.height(), FRAME_TITLE_PX);
        // A longer title reserves a wider band, which is what the click target
        // is measured against.
        let wider = frame_title_box(screen, "Agenda and next steps").expect("a band");
        assert!(wider.width() > band.width());
    }

    /// `TOILE-253`, the other side: a frame nobody named reads as a
    /// placeholder rather than as nothing. The board then always offers
    /// somewhere to type a first name.
    #[test]
    fn an_unnamed_frame_reads_as_a_placeholder() {
        assert_eq!(frame_title_label("Agenda"), ("Agenda", true));
        assert_eq!(frame_title_label("  Agenda "), ("Agenda", true));
        let (label, named) = frame_title_label("   ");
        assert!(!named, "a blank title is not a title");
        assert!(!label.is_empty(), "the band would have nothing to show");
    }

    #[test]
    fn content_walk_balances_transform_and_paints_each_node() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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

    // The "camera pass" test, host-run against the mock surface. With a Smart
    // Animate override in the map, the animated element paints with the tweened
    // opacity / fill instead of its static style. An un-overridden element keeps
    // its own. Exercises the render side of the playback -> camera wiring.
    #[test]
    fn content_walk_applies_animation_override() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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
    /// card tests. The render path keys the card off the geom, the ink off the
    /// `is_sticky` flag derived from it.
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
            link: None,
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
    // behind "clicking Sticky does nothing". An empty sticky used to derive as
    // Text and paint no background, so it was invisible (toile-styles).
    #[test]
    fn sticky_paints_a_colored_card() {
        let fill = 0xFFFE_F08A; // sticky yellow
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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
        // (the ink is independent of `fill`,). Font-availability agnostic:
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

    // A free `Text` element has no card. The geometry pass paints nothing for it,
    // so it emits no FillPath (only its glyphs, in the text pass). This is what
    // keeps Sticky and Text visually distinct (toile-styles).
    #[test]
    fn text_element_paints_no_card() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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

    /// The walk names the pictures it cannot paint, once each, and skips the
    /// one already resident (`TOILE-IO-034`). Only these addresses cross the
    /// bus, so what is off screen is never transferred.
    #[test]
    fn only_the_unresolved_pictures_are_asked_for() {
        let b = Rect::new(0.0, 0.0, 10.0, 10.0);
        let nodes = vec![
            elem("a", DisplayGeom::Image("h1".into()), b),
            elem("b", DisplayGeom::Image("h2".into()), b),
            // The same picture twice: one request, not two.
            elem("c", DisplayGeom::Image("h2".into()), b),
            // Not a picture: it names no address at all.
            elem("d", DisplayGeom::Rect, b),
        ];
        let mut images = ImageStore::new();
        images.insert(
            "h1".into(),
            DecodedImage {
                pixels: vec![0u8; 4],
                width: 1,
                height: 1,
            },
        );
        let asked = missing_media(&nodes, &images, &std::collections::HashSet::new());
        assert_eq!(asked, vec!["h2".to_owned()]);
    }

    /// A picture already asked for is not asked for again. That stops a
    /// request going out on every frame that fails to paint it. That includes
    /// an address the document cannot resolve and never answers.
    #[test]
    fn a_pending_request_is_not_repeated() {
        let b = Rect::new(0.0, 0.0, 10.0, 10.0);
        let nodes = vec![elem("a", DisplayGeom::Image("h1".into()), b)];
        let mut pending = std::collections::HashSet::new();
        pending.insert("h1".to_owned());
        assert!(missing_media(&nodes, &ImageStore::new(), &pending).is_empty());
        // With nothing pending, the same list does ask.
        assert_eq!(
            missing_media(
                &nodes,
                &ImageStore::new(),
                &std::collections::HashSet::new()
            ),
            vec!["h1".to_owned()]
        );
    }

    // An `Image` element whose hash is in the store paints real pixels
    // (`DrawImage`), scaled to its AABB. One whose hash is absent falls back to
    // a placeholder box (a path fill), per toile-media.
    #[test]
    fn image_paints_pixels_when_resolved_else_placeholder() {
        let bounds = Rect::new(0.0, 0.0, 40.0, 20.0);
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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
            version: 0,
            pins: Vec::new(),
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
            &HashMap::new(),
            None,
            false,
        );
        // 4 corner handles + 1 rotate handle = 5 filled squares (corners-only).
        let handle_fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handle_fills, 5);
    }

    /// `TOILE-UI-391`: a selected group wears its box and the rotate handle,
    /// and no corner square.
    #[test]
    fn overlay_draws_no_corner_square_for_a_selected_group() {
        let bounds = Rect::new(0.0, 0.0, 50.0, 50.0);
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![elem("g", DisplayGeom::Group, bounds)],
        };
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[ElementId("g".into())],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            false,
        );
        let handle_fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handle_fills, 1, "the rotate knob alone");
        let box_strokes = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::StrokeRect { .. }))
            .count();
        assert_eq!(box_strokes, 1, "the group still wears its box");
    }

    /// `TOILE-UI-083`: presentation mode paints no editing surface. The same
    /// selection that draws a box and five handles above draws neither while
    /// the mode holds.
    #[test]
    fn overlay_paints_no_editing_surface_while_presenting() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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
            &[ElementId("r".into())],
            None,
            &HashMap::new(),
            None,
            true,
        );
        assert!(
            surface.ops().is_empty(),
            "a slide took a selection box, a handle or a find outline: {:?}",
            surface.ops()
        );
    }

    #[test]
    fn overlay_box_follows_gesture_override_bounds() {
        // The element is committed at (0,0)-(50,50) but a live resize override
        // stretches it to (0,0)-(90,70); the selection box must track the preview.
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![elem(
                "r",
                DisplayGeom::Rect,
                Rect::new(0.0, 0.0, 50.0, 50.0),
            )],
        };
        let preview_bounds = Rect::new(0.0, 0.0, 90.0, 70.0);
        let mut gesture = HashMap::new();
        gesture.insert(
            ElementId("r".into()),
            AnimatedElement {
                id: ElementId("r".into()),
                bounds: preview_bounds,
                rotation: 0.0,
                opacity: 1.0,
                fill: None,
            },
        );
        let cam = cam();
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[ElementId("r".into())],
            &vp,
            &cam,
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &gesture,
            None,
            false,
        );
        // The selection box is stroked at the preview bounds, not the committed rect.
        let expected = world_rect_to_screen(preview_bounds, &cam);
        let has_box = surface
            .ops()
            .iter()
            .any(|o| matches!(o, DrawOp::StrokeRect { rect, .. } if *rect == expected));
        assert!(has_box, "selection box must follow the live preview bounds");
    }

    /// A selected arrow is traced and gets exactly two handles, one per end, and
    /// no selection box. An arrow has no rectangle to draw (`toile-connectors`).
    #[test]
    fn a_selected_arrow_gets_two_handles_and_no_box() {
        let mut path = ambre_draw::BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![DisplayNode::Connector(DisplayConnector {
                id: ConnectorId("c".into()),
                path,
                style: ElementStyle::default(),
                arrows: ambre_bus::app::toile::EndCaps {
                    start: false,
                    end: true,
                },
                label: String::new(),
            })],
        };
        let mut surface = RecordingSurface::new();
        let selected = ConnectorId("c".into());
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            Some(&selected),
            false,
        );
        let handles = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handles, 2, "one handle per end");
        assert!(
            !surface
                .ops()
                .iter()
                .any(|o| matches!(o, DrawOp::StrokeRect { .. })),
            "an arrow carries no selection box"
        );
        assert!(
            surface
                .ops()
                .iter()
                .any(|o| matches!(o, DrawOp::StrokePath { .. })),
            "the arrow's own line is traced"
        );
    }

    /// A linked element gets a badge outside its top-right corner; an unlinked
    /// one gets none (`toile-links`).
    #[test]
    fn a_linked_element_gets_a_badge() {
        let bounds = Rect::new(0.0, 0.0, 50.0, 50.0);
        let mut linked = elem("l", DisplayGeom::Rect, bounds);
        if let DisplayNode::Element(e) = &mut linked {
            e.link = Some(ambre_bus::app::toile::LinkTarget::Url(
                "https://x.test/".into(),
            ));
        }
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![linked, elem("p", DisplayGeom::Rect, bounds)],
        };
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            false,
        );
        let badges = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillPath { .. }))
            .count();
        assert_eq!(badges, 1, "one badge, for the one linked element");
    }

    /// An arrow the display no longer holds paints nothing: a stale selection
    /// leaves no handles floating on the board.
    #[test]
    fn a_vanished_arrow_paints_nothing() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: Vec::new(),
        };
        let mut surface = RecordingSurface::new();
        let selected = ConnectorId("gone".into());
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            Some(&selected),
            false,
        );
        assert!(surface.ops().is_empty(), "nothing is painted");
    }

    #[test]
    fn overlay_outlines_visible_find_hits() {
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
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
            &HashMap::new(),
            None,
            false,
        );
        // No selection => no handles, only the two hit outlines (one per match).
        let strokes = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::StrokeRect { .. }))
            .count();
        assert_eq!(strokes, 2);
    }

    // Build a node rotated `theta` radians about its AABB center, mirroring the
    // Logic derive so the overlay / text passes see a realistic transform.
    fn rotated_elem(id: &str, geom: DisplayGeom, bounds: Rect, theta: f64) -> DisplayNode {
        let c = bounds.center();
        let transform = Affine::translate(c.to_vec2())
            * Affine::rotate(theta)
            * Affine::translate(-c.to_vec2());
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Sticky,
            transform,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom,
            link: None,
            group: None,
        })
    }

    #[test]
    fn overlay_rotates_box_and_handles_for_a_rotated_element() {
        use std::f64::consts::FRAC_PI_4;
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![rotated_elem(
                "r",
                DisplayGeom::Rect,
                Rect::new(0.0, 0.0, 50.0, 50.0),
                FRAC_PI_4,
            )],
        };
        let cam = cam();
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[ElementId("r".into())],
            &vp,
            &cam,
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            false,
        );
        // The box must NOT be a `stroke_rect` (that degrades to an upright AABB on
        // the WgpuSurface); it is drawn as rotated `stroke_line` edges instead.
        assert!(
            !surface
                .ops()
                .iter()
                .any(|o| matches!(o, DrawOp::StrokeRect { .. })),
            "a rotated box must not use stroke_rect"
        );
        // Still 5 handle squares (4 corners + rotate knob), each an axis-aligned
        // (screen-aligned) square positioned at its rotated center.
        let b = world_rect_to_screen(Rect::new(0.0, 0.0, 50.0, 50.0), &cam);
        let expect_se = super::rotate_about(b.center(), FRAC_PI_4) * Point::new(b.x1, b.y1);
        let half = HANDLE_SIZE_PX / 2.0;
        let se_square = Rect::new(
            expect_se.x - half,
            expect_se.y - half,
            expect_se.x + half,
            expect_se.y + half,
        );
        let has_rotated_se_handle = surface.ops().iter().any(|o| {
            matches!(o, DrawOp::FillRect { rect, .. }
                if (rect.x0 - se_square.x0).abs() < 1e-6 && (rect.y0 - se_square.y0).abs() < 1e-6)
        });
        assert!(
            has_rotated_se_handle,
            "the SE handle must sit at its rotated position"
        );
        let handle_fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handle_fills, 5);
    }

    #[test]
    fn rotated_sticky_body_rotates_its_glyphs() {
        use std::f64::consts::FRAC_PI_4;
        let vp = DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![rotated_elem(
                "s",
                DisplayGeom::Sticky("hi".into()),
                Rect::new(0.0, 0.0, 120.0, 120.0),
                FRAC_PI_4,
            )],
        };
        let surface = paint(&vp);
        // Font-availability agnostic. If the headless host shapes no glyphs this
        // holds vacuously; when it does, every run carries the element rotation
        // rather than a pure translation.
        for op in surface.ops() {
            if let DrawOp::Glyphs { transform, .. } = op {
                let c = transform.as_coeffs();
                assert!(
                    c[1].abs() > 1e-6,
                    "a rotated sticky must rotate its glyph runs"
                );
            }
        }
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
        cell_border: Color::from_rgba8(96, 96, 96, 255),
    };
}
