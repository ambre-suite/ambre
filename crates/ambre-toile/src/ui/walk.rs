// ambre-toile/src/ui/walk.rs
// ============================================================================
// The board walk (`CANVAS-074`): what the screen and the page both draw.
//
// It walks the culled, z-ordered `DisplayViewport` in board units and emits
// abstract `DrawSurface` calls. The shapes, paths, strokes and connectors come
// first, then the sticky and text bodies, then the connector captions. The
// screen pushes its camera before the call. The page pushes the transform that
// fits its frame. So the walk never reads a zoom, a density or a pointer. The text is
// laid out in board units too, and the surface scales the glyphs with the
// transform it draws them under.
//
// Screen-only layers live in `render.rs`: the grid, the frame titles, the
// selection and its handles. This module imports no screen module, which
// `crate_boundaries.py` holds.
// ============================================================================

use std::collections::HashMap;
use std::sync::LazyLock;

use ambre_bus::app::toile::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementStyle,
};
use ambre_canvas::{Palette, TextCache};
use ambre_core::id::{Connector, Element, Id};
use ambre_core::{Board, Doc, Px, Rgba, StoredBoard};
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DocumentWalk, DrawSurface, FontContext, ImageKey,
    ImageSource, Point, PositionedLayoutItem, Rect, Size, Stroke,
};

use crate::logic::animate::AnimatedElement;

use super::label::{LABEL_FONT_SIZE, label_rect};

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
    pub pixels: ambre_core::Payload<ambre_core::RgbaPixels>,
    pub size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
}

/// Resolved image pixels for this frame, keyed by media hash. Empty when no
/// image element is present (or its media has not been decoded yet).
pub type ImageStore = HashMap<ambre_core::ContentHash, DecodedImage>;

/// Inner padding, in board units, between an element's border and its body,
/// and between a caption's plate and its words. The inline editor applies the
/// same inset, so a body does not move when an edit opens or closes.
pub(crate) const TEXT_PAD: Px<Board> = Px::<Board>::new(4.0);

/// The board as the walk reads it.
pub struct BoardView<'v> {
    /// The culled, z-ordered nodes, already derived for the region drawn.
    pub viewport: &'v DisplayViewport,
    /// The theme the board paints in.
    pub palette: &'v Palette,
    /// Decoded image pixels, keyed by media hash (`toile-media`).
    pub images: &'v ImageStore,
    /// Smart Animate or gesture overrides, keyed by element id. Each replaces
    /// the element's transform, bounds, opacity and fill. Empty at rest.
    pub anim: &'v HashMap<Id<Element>, AnimatedElement>,
    /// The body this view leaves out, because the inline editor paints it.
    pub muted_body: Option<&'v Id<Element>>,
    /// The caption this view leaves out, for the same reason.
    pub muted_caption: Option<&'v Id<Connector>>,
}

/// The board walk, holding the shaping context and the layout cache it lays
/// the bodies out with.
pub struct BoardWalk<'t> {
    pub font_cx: &'t mut FontContext,
    pub text_cache: &'t mut TextCache,
}

impl DocumentWalk for BoardWalk<'_> {
    type View<'v> = BoardView<'v>;

    /// Draw the board inside `region`, clipped to it. The page passes its
    /// frame, so neighbouring content never bleeds onto a slide; the screen
    /// passes what it shows.
    fn walk<S: DrawSurface>(&mut self, view: &BoardView<'_>, region: Rect, surface: &mut S) {
        surface.push_clip(region);
        let nodes = &view.viewport.nodes;
        // 1. Shapes, paths, strokes and connectors, in z-order.
        for node in nodes {
            match node {
                DisplayNode::Element(e) => {
                    paint_element(surface, e, view.images, view.anim.get(&e.id));
                }
                DisplayNode::Connector(c) => paint_connector(surface, c),
            }
        }
        // 2. Sticky and text bodies. A sticky draws its body in an ink chosen
        // for its card (its `fill` is the card colour); a text element draws
        // in its `fill`.
        for node in nodes {
            let DisplayNode::Element(e) = node else {
                continue;
            };
            let (text, host) = match &e.geom {
                DisplayGeom::Sticky(t) => (t, TextHost::Sticky),
                DisplayGeom::Text(t) => (t, TextHost::Text),
                _ => continue,
            };
            if view.muted_body == Some(&e.id) {
                continue;
            }
            paint_text(
                surface,
                e,
                text,
                host,
                view.palette,
                self,
                view.anim.get(&e.id),
            );
        }
        // 3. Connector captions, after the bodies, so a caption stays readable
        // over whatever the arrow crosses (`toile-connectors`).
        for node in nodes {
            let DisplayNode::Connector(c) = node else {
                continue;
            };
            if c.label.is_empty() || view.muted_caption == Some(&c.id) {
                continue;
            }
            paint_label(surface, c, view.palette, self);
        }
        surface.pop_clip();
    }
}

/// The dark ink (RGBA) of a sticky-note body: a near-black, readable on a light
/// card. Independent of the theme (the card is not).
pub(crate) const STICKY_INK: Rgba = Rgba::from_bits(0x1A1A_1AFF);

/// The light ink (RGBA) of a sticky-note body, for a dark card.
pub(crate) const STICKY_INK_ON_DARK: Rgba = Rgba::from_bits(0xFFFF_FFFF);

/// The linear value of each 8-bit sRGB channel (WCAG 2), built once. A sticky
/// body weighs four colours, and computing their powers on every frame cost an
/// eighth of the board walk.
static LINEAR: LazyLock<[f64; 256]> = LazyLock::new(|| {
    std::array::from_fn(|v| {
        let c = v as f64 / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
});

/// The relative luminance of an RGBA colour, alpha ignored (WCAG 2).
fn luminance(colour: Rgba) -> f64 {
    // A byte is always inside the table.
    let channel = |c: u8| LINEAR.get(usize::from(c)).copied().unwrap_or(0.0);
    let [r, g, b] = colour.rgb().channels();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG 2 contrast ratio of two RGBA colours, from 1 to 21.
pub(crate) fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The ink of a sticky body on `card`: of the two inks, the one that contrasts
/// more with the card (`TOILE-UI-262`). The worst card, a mid grey, still
/// reads at four to one. A note with no fill paints the dark ink.
fn sticky_ink(card: Option<Rgba>) -> Rgba {
    match card {
        Some(c) if contrast(c, STICKY_INK_ON_DARK) > contrast(c, STICKY_INK) => STICKY_INK_ON_DARK,
        _ => STICKY_INK,
    }
}

/// Sticky drop-shadow colour (RGBA): a soft translucent black offset to the
/// bottom-right so the note reads as a physical card (feedback).
pub(crate) const STICKY_SHADOW: Rgba = Rgba::from_bits(0x0000_0033);
/// Drop-shadow offset in world units (bottom-right).
const STICKY_SHADOW_OFFSET: Px<Board> = Px::<Board>::new(4.0);

/// The body font size (world units) for a sticky of the given bounds. It scales
/// with the note so the text "adapts to the size of the note" (feedback). A
/// fraction of the shorter side, clamped to a legible range. Shared with the
/// inline editor so the in-edit size matches the committed one (`edit.rs`).
pub(crate) fn sticky_font_size(bounds: Rect) -> Px<Doc> {
    let shorter = bounds.width().min(bounds.height()) as f32;
    Px::<Doc>::new((shorter * 0.16).clamp(8.0, 48.0))
}

/// The element a body of text belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextHost {
    /// A free text element: its `fill` is its glyph colour.
    Text,
    /// A sticky note: its `fill` is the card, and the body auto-sizes.
    Sticky,
}

/// The effective body font size (world units) for a sticky / text element. A size
/// the style names wins. An automatic one sizes a sticky from its bounds. A text
/// element has no card to track, so it draws at the default size, which the
/// export also writes (`TOILE-IO-088`). The style holds points, and a world unit is a
/// CSS pixel at zoom 1, so the size goes through
/// [`FontSize::to_px`](ambre_core::FontSize::to_px).
pub(crate) fn sticky_or_style_font_size(
    host: TextHost,
    style: &ElementStyle,
    bounds: Rect,
) -> Px<Doc> {
    match (host, style.font.size.as_fixed()) {
        (TextHost::Sticky, None) => sticky_font_size(bounds),
        (TextHost::Text, _) | (TextHost::Sticky, Some(_)) => style.font.size.or_default().to_px(),
    }
}

/// Resolve a sticky / text body's ink (RGBA), or `None` to fall back to the theme
/// text colour. An explicit `text_color` always wins. Otherwise a sticky uses the
/// ink chosen for its card (its `fill` is the card background, not the text). A free text element
/// falls back to its `fill` (legacy: a text element's fill was its glyph colour)
/// then, if unset, the theme colour. Pure and host-tested; shared with the inline
/// editor via the same precedence in `edit.rs`.
pub(crate) fn resolved_text_ink(
    host: TextHost,
    text_color: Option<Rgba>,
    fill: Option<Rgba>,
) -> Option<Rgba> {
    match (text_color, host) {
        (Some(tc), _) => Some(tc),
        (None, TextHost::Sticky) => Some(sticky_ink(fill)),
        (None, TextHost::Text) => fill,
    }
}

/// Convert a Toile style colour to a `peniko::Color`, pre-multiplying the
/// element `opacity` into the alpha channel.
pub(crate) fn style_color(v: Rgba, opacity: ambre_core::Bounded<ambre_core::Opacity>) -> Color {
    let [r, g, b, a] = v.channels();
    let alpha = opacity.fade(ambre_core::Alpha::new(a));
    Color::from_rgba8(r, g, b, alpha.get())
}

/// A closed rectangle as a path (so it maps through the active transform and
/// honors element rotation, unlike the axis-aligned `fill_rect` fast path).
pub(crate) fn rect_path(b: Rect) -> BezPath {
    let mut p = BezPath::with_capacity(5);
    p.move_to((b.x0, b.y0));
    p.line_to((b.x1, b.y0));
    p.line_to((b.x1, b.y1));
    p.line_to((b.x0, b.y1));
    p.close_path();
    p
}

/// The Phase-0 triangle inscribed in `bounds`: apex at the top-center, base along
/// the bottom edge. The hit-test reads the same path, so a click lands where the
/// paint is (`hit.rs`).
pub fn triangle_path(bounds: Rect) -> BezPath {
    let mut p = BezPath::with_capacity(4);
    p.move_to((bounds.center().x, bounds.y0));
    p.line_to((bounds.x1, bounds.y1));
    p.line_to((bounds.x0, bounds.y1));
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
pub(crate) fn ellipse_path(b: Rect) -> BezPath {
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
pub(crate) fn roundrect_path(b: Rect, radius: f64) -> BezPath {
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
pub(crate) fn polyline_path(pts: &[Point]) -> BezPath {
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
        surface.fill_path(path, &Brush::Solid(style_color(fill, style.opacity)));
    }
    if let Some(stroke) = style.stroke
        && style.stroke_w > Px::<StoredBoard>::ZERO
    {
        surface.stroke_path(
            path,
            &Brush::Solid(style_color(stroke, style.opacity)),
            &Stroke::new(f64::from(style.stroke_w.get())),
        );
    }
}

/// The board-coloured plate a caption sits on, so the words are readable where
/// they cross the arrow. Shared by the walk and the inline editor, which is why
/// it is not inlined into either.
pub(crate) fn paint_label_plate<S: DrawSurface>(surface: &mut S, plate: Rect, palette: &Palette) {
    surface.fill_path(
        &roundrect_path(plate, 3.0),
        &Brush::Solid(palette.content_bg),
    );
}

/// Draw a connector's caption, centred on the arrow's mid-arc point, in board
/// units. A plate in the board colour keeps the words readable where they
/// cross the line they belong to (`toile-connectors`).
fn paint_label<S: DrawSurface>(
    surface: &mut S,
    c: &DisplayConnector,
    palette: &Palette,
    walk: &mut BoardWalk<'_>,
) {
    let Some(world) = label_rect(&c.path) else {
        return;
    };
    let pad = TEXT_PAD.get();
    let wrap_w = Px::<Doc>::new((world.width() - 2.0 * pad).max(1.0) as f32);
    let style = ElementStyle::default();
    let layout = walk.text_cache.get_or_build_font(
        walk.font_cx,
        &c.label,
        LABEL_FONT_SIZE,
        wrap_w,
        Alignment::Center,
        ambre_canvas::Weight::Regular,
        ambre_canvas::Slant::Upright,
        Some(style.font.face()),
        ambre_canvas::WordBreak::Whole,
    );
    // The plate hugs the laid-out text rather than the (fixed) caption box, so a
    // short caption does not punch a wide hole in the board.
    let (tw, th) = (layout.width() as f64, layout.height() as f64);
    paint_label_plate(
        surface,
        Rect::from_center_size(world.center(), Size::new(tw + 2.0 * pad, th + 2.0 * pad)),
        palette,
    );
    let brush = Brush::Solid(palette.cell_text);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = world.x0 + pad + run.offset() as f64;
            let ty = world.y0 + pad + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// The world transform of an element whose geometry an animation override
/// replaces: rotation about the (interpolated) AABB center. This mirrors
/// [`display_element`](crate::logic::state), so the tween matches the static
/// pass.
pub(crate) fn anim_transform(a: &AnimatedElement) -> Affine {
    let c = a.bounds.center();
    Affine::translate(c.to_vec2())
        * Affine::rotate(a.rotation.get())
        * Affine::translate(-c.to_vec2())
}

/// A rotation of `theta` about `center`. The camera carries no
/// rotation (pan + zoom only), so an element's world rotation is also its
/// screen rotation. A pass in either space rotates about the element's center
/// in that space with this affine.
pub(crate) fn rotate_about(center: Point, theta: ambre_core::Radians) -> Affine {
    Affine::translate(center.to_vec2())
        * Affine::rotate(theta.get())
        * Affine::translate(-center.to_vec2())
}

/// The rotation (radians) of an element's text: the live gesture / animation
/// angle when overridden, else the angle baked into its display transform.
/// That transform is a pure rotation about the AABB center, so `atan2(b, a)`
/// recovers it.
fn text_rotation(e: &DisplayElement, anim: Option<&AnimatedElement>) -> ambre_core::Radians {
    match anim {
        Some(a) => a.rotation,
        None => {
            let c = e.transform.as_coeffs();
            ambre_core::Radians::new(c[1].atan2(c[0]))
        }
    }
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

/// Paint one element in board units. Rotation is applied by pushing the
/// element's own transform, so geometry is expressed in the unrotated world
/// frame (matching hit-testing, `hit.rs`). When `anim` is `Some`, the element
/// is mid-transition. Its transform, bounds and opacity / fill come from the
/// Smart Animate override instead of the static display.
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
                    &Brush::Solid(style_color(stroke, style.opacity)),
                    &Stroke::new(f64::from(style.stroke_w.get().max(1.0))),
                );
            }
        }
        // An image paints its decoded pixels when they have arrived, scaled to
        // the element AABB (the rect is stored at the imported aspect). A hash
        // not yet in the store paints a placeholder box until a later frame
        // carries the media. `draw_image` maps through the active element
        // transform, so a rotated image rotates with its box.
        DisplayGeom::Image(hash) => match hash
            .as_ref()
            .and_then(|hash| Some((hash, images.get(hash)?)))
        {
            Some((hash, img)) => surface.draw_image(
                &ImageSource {
                    // The media address already identifies these pixels, so the
                    // back end recognises the texture it uploaded on an earlier
                    // frame without hashing the buffer again.
                    key: ImageKey::from_address(hash),
                    pixels: &img.pixels,
                    size: img.size,
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
        // the body string is drawn on top in the body pass. A free `Text`
        // element has no card, so it paints nothing in the geometry pass.
        // `style.corner` is 0 (Miro-style rectangle), so `roundrect_path`
        // degenerates to a plain rectangle.
        DisplayGeom::Sticky(_) => {
            let offset = STICKY_SHADOW_OFFSET.get();
            let shadow = bounds + ambre_draw::Vec2::new(offset, offset);
            surface.fill_path(
                &roundrect_path(shadow, f64::from(style.corner.get())),
                &Brush::Solid(style_color(STICKY_SHADOW, style.opacity)),
            );
            fill_and_stroke(
                surface,
                &roundrect_path(bounds, f64::from(style.corner.get())),
                &style,
            );
        }
        DisplayGeom::Text(_) => {}
        DisplayGeom::Group => {} // paints nothing
    }
    surface.pop_transform();
}

fn paint_connector<S: DrawSurface>(surface: &mut S, c: &DisplayConnector) {
    let color = c.style.stroke.unwrap_or(Rgba::from_bits(0x3333_33FF));
    surface.stroke_path(
        &c.path,
        &Brush::Solid(style_color(color, c.style.opacity)),
        &Stroke::new(f64::from(c.style.stroke_w.get().max(1.0))),
    );
}

/// Draw a sticky / text body in board units, laid out within the element's
/// bounds at its own font size. The transform pushed before the walk scales
/// the glyphs, so a body reads at the same size relative to its card at every
/// zoom.
fn paint_text<S: DrawSurface>(
    surface: &mut S,
    e: &DisplayElement,
    text: &str,
    host: TextHost,
    palette: &Palette,
    walk: &mut BoardWalk<'_>,
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
    // A sticky's font tracks the note size unless the style carries an explicit
    // override (size > 0); a text element uses its own style size.
    let font_size = sticky_or_style_font_size(host, &e.style, bounds);
    // Body inset: the same padding the inline editor applies, so the static text
    // and the in-edit text share one origin + wrap width. The body does not jump
    // / re-wrap when entering or leaving edit.
    let pad = TEXT_PAD.get();
    let wrap_w = Px::<Doc>::new((bounds.width() - 2.0 * pad).max(1.0) as f32);
    // Text (foreground) colour, (see `resolved_text_ink`): `None` means "use
    // the theme text colour" (only a text element with neither text_color nor fill).
    let color = resolved_text_ink(host, e.style.text_color, fill)
        .map(|c| style_color(c, opacity))
        .unwrap_or(palette.cell_text);
    let layout = walk.text_cache.get_or_build_font(
        walk.font_cx,
        text,
        font_size,
        wrap_w,
        Alignment::Start,
        ambre_canvas::Weight::from_bool(e.style.font.weight.is_bold()),
        e.style.font.italic,
        Some(e.style.font.face()),
        ambre_canvas::WordBreak::Whole,
    );
    let brush = Brush::Solid(color);
    let underline = e.style.font.underline == ambre_core::Underline::Underlined;
    let strikethrough = e.style.font.strikethrough == ambre_core::Strikethrough::Struck;
    // A rotated element rotates its body too: the glyph run origins and the
    // decoration endpoints are mapped through a rotation about the note's
    // center. The text follows the card instead of staying axis-aligned
    // (toile-board-rendering). `None` (no rotation) keeps the fast
    // pure-translation path.
    let rot = text_rotation(e, anim);
    let rot_tf = (rot != ambre_core::Radians::ZERO).then(|| rotate_about(bounds.center(), rot));
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = bounds.x0 + pad + run.offset() as f64;
            let ty = bounds.y0 + pad + run.baseline() as f64;
            let base = Affine::translate((tx, ty));
            let glyph_tf = rot_tf.map_or(base, |r| r * base);
            surface.draw_glyphs(&run, &brush, glyph_tf);
            // Per-run text decorations, drawn in the glyph ink (matches Trame's
            // cell decorations). An underline sits just below the baseline and
            // a strikethrough through the x-height (~0.3em above the baseline).
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
                    surface.stroke_line(
                        ambre_draw::Line::new((a.x, a.y), (b.x, b.y)),
                        color,
                        ambre_core::Px::<ambre_core::Surface>::new(1.0),
                    );
                };
                if underline {
                    line(ty + 2.0);
                }
                if strikethrough {
                    line(ty - f64::from(font_size.get()) * 0.3);
                }
            }
        }
    }
}
