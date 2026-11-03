// ambre-toile/src/ui/render.rs
// ============================================================================
// The Toile screen passes (toile-board-rendering). Toile owns only "what to
// draw": it emits ABSTRACT `DrawSurface` calls. Tessellation, the glyph atlas
// and GPU submission are `ambre-canvas`'s job. So the passes are host-testable
// against `RecordingSurface` (a mock `DrawSurface`): assert the ordered
// `DrawOp` sequence with no GPU.
//
// Two passes:
//   paint_content -- board bg + adaptive grid, then the board walk
//                    (`walk.rs`) under the camera transform, then the frame
//                    titles in screen space.
//   paint_overlay -- selection bounding box + transform handles (screen space).
// ============================================================================

use std::collections::HashMap;

use ambre_bus::app::toile::{
    DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, PresentMode, ShapeKind,
};
use ambre_canvas::{Palette, TextCache};
use ambre_core::id::{Connector, Element, Id};
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, DocumentWalk, DrawSurface, FontContext, Point,
    PositionedLayoutItem, Rect, Stroke,
};

use crate::logic::animate::AnimatedElement;

use super::grid;
use super::tool::{
    HANDLE_SIZE, Preview, is_group, resize_handle_points, rotate_handle_point, selection_bounds,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;
use super::walk::{
    BoardView, BoardWalk, ellipse_path, polyline_path, rect_path, roundrect_path, triangle_path,
};
pub use super::walk::{DecodedImage, ImageStore};
pub(crate) use super::walk::{paint_label_plate, rotate_about};

/// The media addresses a display list names and the view cannot paint: not in
/// the store, not already asked for, each listed once.
///
/// It is the culled list that is walked, so only what survived the viewport
/// query is asked for. A board of two hundred photographs transfers the ones
/// on screen (`toile-media`).
pub fn missing_media(
    nodes: &[DisplayNode],
    images: &ImageStore,
    requested: &std::collections::HashSet<ambre_core::ContentHash>,
) -> Vec<ambre_core::ContentHash> {
    let mut out: Vec<ambre_core::ContentHash> = Vec::new();
    for node in nodes {
        let DisplayNode::Element(de) = node else {
            continue;
        };
        let DisplayGeom::Image(Some(hash)) = &de.geom else {
            continue;
        };
        if images.contains_key(hash) || requested.contains(hash) || out.iter().any(|h| h == hash) {
            continue;
        }
        out.push(hash.clone());
    }
    out
}

/// Stroke width, in screen pixels, of the line traced over a selected arrow.
/// Heavier than the arrow itself so the selection reads at any zoom.
const ARROW_SELECT_W: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(3.0);

/// Paint the persistent content layer: board background + adaptive grid, then
/// the board walk under the camera transform, then the frame titles.
///
/// The walk draws what the PDF page draws, from the same view, at board units
/// (`CANVAS-074`). The camera is the only thing the screen adds to it.
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
    anim: &HashMap<Id<Element>, AnimatedElement>,
    // Presentation focus mode: the UI hides the board grid so the audience
    // sees only the slide content.
    mode: PresentMode,
) {
    // 1. Background + adaptive grid (screen space). Focus mode drops the grid.
    if mode.is_presenting() {
        grid::paint_background(surface, cam, palette);
    } else {
        grid::paint_grid(surface, cam, palette, vp.grid_unit);
    }

    // 2. The board walk, in board units under the camera. The body or the
    // caption under inline edit is left out: the editor overlay paints its
    // live buffer instead.
    let view = BoardView {
        viewport: vp,
        palette,
        images,
        anim,
        muted_body: editing.and_then(|t| t.element()),
        muted_caption: editing.and_then(|t| t.connector()),
    };
    surface.push_transform(cam.world_to_screen());
    BoardWalk {
        font_cx,
        text_cache,
    }
    .walk(&view, cam.visible_world(), surface);
    surface.pop_transform();

    // 3. Frame titles, in screen space, so a title reads the same at every
    // zoom. A title is board structure, not content: the page does not print
    // it, and a presentation drops it with the grid (`TOILE-255`).
    if !mode.is_presenting() {
        paint_frame_titles(surface, vp, cam, palette, font_cx, text_cache, editing);
    }
}

/// The screen-space rotation (radians) to apply to the selection box + handles of
/// a single selected element. That is its live gesture angle when a resize /
/// rotate / move is in flight, else the angle baked into its display transform.
/// `0.0` when the id is not in the culled set.
fn selection_rotation(
    id: &Id<Element>,
    vp: &DisplayViewport,
    gesture: &HashMap<Id<Element>, AnimatedElement>,
) -> ambre_core::Radians {
    if let Some(a) = gesture.get(id) {
        return a.rotation;
    }
    vp.nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Element(e) if &e.id == id => {
                let c = e.transform.as_coeffs();
                Some(ambre_core::Radians::new(c[1].atan2(c[0])))
            }
            _ => None,
        })
        .unwrap_or(ambre_core::Radians::ZERO)
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
    selection: &[Id<Element>],
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    preview: Option<&Preview>,
    find_hits: &[Id<Element>],
    find_focus: Option<&Id<Element>>,
    // Live transform overrides for an in-flight resize / rotate / move. When an
    // entry exists for a selected id, the selection box + handles follow its
    // preview bounds instead of the committed geometry. They stay glued to the
    // object during the drag. Empty at rest.
    gesture: &HashMap<Id<Element>, AnimatedElement>,
    // The selected arrow, if one is (`toile-connectors`). Exclusive with
    // `selection`, so at most one of the two boxes is drawn.
    arrow: Option<&Id<Connector>>,
    mode: PresentMode,
) {
    if mode.is_presenting() {
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
    let theta = single.map_or(ambre_core::Radians::ZERO, |only| {
        selection_rotation(only, vp, gesture)
    });
    let place = |p: Point| {
        if theta == ambre_core::Radians::ZERO {
            p
        } else {
            rotate_about(b.center(), theta) * p
        }
    };

    // Bounding box. Axis-aligned: the crisp `stroke_rect` fast path. Rotated: four
    // `stroke_line` edges between the rotated corners (stroke_rect would degrade to
    // an upright AABB on the WgpuSurface, see above).
    if theta == ambre_core::Radians::ZERO {
        surface.stroke_rect(
            b,
            palette.selection,
            ambre_core::Px::<ambre_core::Surface>::new(1.5),
        );
    } else {
        let c = [
            place(Point::new(b.x0, b.y0)),
            place(Point::new(b.x1, b.y0)),
            place(Point::new(b.x1, b.y1)),
            place(Point::new(b.x0, b.y1)),
        ];
        for (a, d) in c.iter().zip(c.iter().cycle().skip(1)) {
            surface.stroke_line(
                ambre_draw::Line::new((a.x, a.y), (d.x, d.y)),
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(1.5),
            );
        }
    }

    // Rotate handle: a short stem above the top edge + the knob.
    let stem_base = place(Point::new(b.center().x, b.y0));
    let knob = place(rotate_handle_point(b));
    surface.stroke_line(
        ambre_draw::Line::new((stem_base.x, stem_base.y), (knob.x, knob.y)),
        palette.selection,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
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
        surface.stroke_rect(
            s,
            palette.selection,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        return;
    }

    let brush = Brush::Solid(palette.selection);
    // A screen-constant width: `n` view pixels, in board units at this zoom.
    let px = |n: f32| {
        Stroke::new(
            cam.to_board(ambre_core::Px::<ambre_core::View>::new(n))
                .get(),
        )
    };
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
fn element_bounds(vp: &DisplayViewport, id: &Id<Element>) -> Option<Rect> {
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
    selection: &[Id<Element>],
    vp: &DisplayViewport,
    gesture: &HashMap<Id<Element>, AnimatedElement>,
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
    hits: &[Id<Element>],
    focus: Option<&Id<Element>>,
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
        surface.stroke_rect(sb, color, ambre_core::Px::<ambre_core::Surface>::new(width));
    }
}

/// Trace a selected arrow and put a handle on each end. The arrow has no box.
/// A rectangle around a diagonal line would cover half the diagram and say
/// nothing about which end a drag would move. The line itself is the
/// selection feedback (`toile-connectors`).
fn paint_arrow_selection<S: DrawSurface>(
    surface: &mut S,
    id: &Id<Connector>,
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
        &Stroke::new(cam.to_board(ARROW_SELECT_W).get()),
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
            ambre_draw::Line::new((cx - r * 1.6, cy + r), (cx + r * 0.4, cy - r)),
            palette.content_bg,
            ambre_core::Px::<ambre_core::Surface>::new(1.5),
        );
        surface.stroke_line(
            ambre_draw::Line::new((cx - r * 0.4, cy + r), (cx + r * 1.6, cy - r)),
            palette.content_bg,
            ambre_core::Px::<ambre_core::Surface>::new(1.5),
        );
    }
}

/// Height of a frame's title band in screen pixels, and the font size inside
/// it (`TOILE-253`). Screen space, so a title reads the same at every zoom,
/// like the link badges and the comment pins.
pub const FRAME_TITLE_HEIGHT: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(14.0);

/// Gap between the title's baseline band and the frame's own top edge.
const FRAME_TITLE_GAP: ambre_core::Px<ambre_core::View> =
    ambre_core::Px::<ambre_core::View>::new(4.0);

/// What a frame's band shows, and whether it is the frame's own title
/// (`TOILE-253`).
///
/// A frame nobody named shows a placeholder instead of nothing, so there is
/// always something to read and something to double-click. The placeholder
/// names no rank. The deck panel numbers a frame by its place in the deck, and
/// a second numbering here would call one frame two different things.
pub fn frame_title_label(title: &str) -> FrameTitle<'_> {
    let title = title.trim();
    if title.is_empty() {
        FrameTitle::Untitled
    } else {
        FrameTitle::Named(title)
    }
}

/// What a frame's band shows: the title somebody typed, or the placeholder
/// of a frame nobody named.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FrameTitle<'a> {
    /// The frame's own title, trimmed.
    Named(&'a str),
    /// No title: the band shows [`FrameTitle::PLACEHOLDER`].
    Untitled,
}

impl<'a> FrameTitle<'a> {
    /// What the band of an unnamed frame reads.
    pub const PLACEHOLDER: &'static str = "Untitled frame";

    /// The text the band draws.
    pub fn label(self) -> &'a str {
        match self {
            Self::Named(title) => title,
            Self::Untitled => Self::PLACEHOLDER,
        }
    }
}

/// A typed title prints as its length alone (`ZK-006`).
impl std::fmt::Debug for FrameTitle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = match self {
            Self::Named(title) => Some(ambre_core::Redacted::new(title)),
            Self::Untitled => None,
        };
        f.debug_tuple("FrameTitle").field(&len).finish()
    }
}

/// Every frame of the visible set, with the band it draws and what is in it, in
/// paint order (`TOILE-253`). One source for the painter, the hit test and the
/// editor, so the three cannot disagree on where a title is.
pub fn frame_bands<'a>(
    nodes: &'a [DisplayNode],
    cam: &'a ViewportTransform,
) -> impl Iterator<Item = (&'a DisplayElement, Rect, FrameTitle<'a>)> {
    nodes.iter().filter_map(move |node| {
        let DisplayNode::Element(e) = node else {
            return None;
        };
        let DisplayGeom::Frame(title) = &e.geom else {
            return None;
        };
        let shown = frame_title_label(title);
        let band = frame_title_box(world_rect_to_screen(e.bounds, cam), shown)?;
        Some((e, band, shown))
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
pub fn frame_title_box(screen: Rect, title: FrameTitle<'_>) -> Option<Rect> {
    let title = title.label().trim();
    if title.is_empty() {
        return None;
    }
    // ~0.6 em per character, the same estimate the SVG import seeds a text box
    // with. A title is one short line, so the error is a few pixels of hit
    // target rather than a layout.
    let tall = f64::from(FRAME_TITLE_HEIGHT.get());
    let w = (title.chars().count() as f64) * tall * 0.6;
    let bottom = screen.y0 - f64::from(FRAME_TITLE_GAP.get());
    Some(Rect::new(screen.x0, bottom - tall, screen.x0 + w, bottom))
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
    for (e, band, shown) in frame_bands(&vp.nodes, cam) {
        // The title under edit is painted by the editor overlay instead, as an
        // element's body is (`TOILE-261`).
        if editing == Some(&e.id) {
            continue;
        }
        let layout = text_cache.get_or_build_font(
            font_cx,
            shown.label(),
            FRAME_TITLE_HEIGHT,
            ambre_core::Px::<ambre_core::View>::new(f32::MAX),
            Alignment::Start,
            ambre_canvas::Weight::Regular,
            ambre_canvas::Slant::Upright,
            None,
            ambre_canvas::WordBreak::Whole,
        );
        // A placeholder reads as the absence it is: the secondary ink, not the
        // ink of a title somebody typed.
        let brush = Brush::Solid(match shown {
            FrameTitle::Named(_) => palette.cell_text,
            FrameTitle::Untitled => palette.header_text,
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
    let h = f64::from(HANDLE_SIZE.get()) / 2.0;
    surface.fill_rect(
        Rect::new(p.x - h, p.y - h, p.x + h, p.y + h),
        palette.selection,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address that repeats the hex `digit` sixty-four times.
    fn hh(digit: char) -> ambre_core::ContentHash {
        ambre_core::ContentHash::parse(&digit.to_string().repeat(64)).expect("a hex digit")
    }
    use crate::ui::walk::{
        STICKY_INK, STICKY_SHADOW, TextHost, anim_transform, contrast, resolved_text_ink,
        sticky_font_size, sticky_or_style_font_size, style_color,
    };
    use ambre_bus::app::toile::{
        DisplayConnector, DisplayElement, ElementKind, ElementStyle, ShapeKind,
    };
    use ambre_core::Px;
    use ambre_core::Rgba;
    use ambre_draw::Color;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn elem(id: &str, geom: DisplayGeom, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style: ElementStyle::default(),
            geom,
            link: None,
            group: None,
        })
    }

    fn cam() -> ViewportTransform {
        ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0))
    }

    /// `TOILE-UI-420`: the direct cubics draw the shape kurbo's arcs drew. They
    /// hold the same box and, to a thousandth, the same area, at every radius
    /// a card takes.
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

    /// `TOILE-UI-420`: the four direct cubics draw the ellipse kurbo's arcs
    /// drew. The box holds within 0.1 and the area to a thousandth, on both
    /// sides of the switch.
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
        let band =
            frame_title_box(screen, FrameTitle::Named("Agenda")).expect("a named frame has a band");
        assert!(
            band.y1 <= screen.y0,
            "the title covers the frame: {band:?} over {screen:?}"
        );
        assert_eq!(band.x0, screen.x0, "the band starts at the frame's corner");
        assert_eq!(band.height(), f64::from(FRAME_TITLE_HEIGHT.get()));
        // A longer title reserves a wider band, which is what the click target
        // is measured against.
        let wider =
            frame_title_box(screen, FrameTitle::Named("Agenda and next steps")).expect("a band");
        assert!(wider.width() > band.width());
    }

    /// `TOILE-253`, the other side: a frame nobody named reads as a
    /// placeholder rather than as nothing. The board then always offers
    /// somewhere to type a first name.
    #[test]
    fn an_unnamed_frame_reads_as_a_placeholder() {
        assert_eq!(frame_title_label("Agenda"), FrameTitle::Named("Agenda"));
        assert_eq!(frame_title_label("  Agenda "), FrameTitle::Named("Agenda"));
        let blank = frame_title_label("   ");
        assert_eq!(blank, FrameTitle::Untitled, "a blank title is not a title");
        let shown = blank.label();
        assert!(!shown.is_empty(), "the band would have nothing to show");
    }

    /// `TOILE-289`: a frame title's debug form prints the title's length, none of
    /// it.
    #[test]
    fn a_frame_title_debug_form_prints_no_title() {
        let form = format!("{:?}", frame_title_label("Q3 layoffs"));
        assert!(!form.contains("layoffs"), "{form}");
        assert!(form.contains("FrameTitle(Some(10 chars))"), "{form}");
        assert_eq!(format!("{:?}", FrameTitle::Untitled), "FrameTitle(None)");
    }

    #[test]
    fn content_walk_balances_transform_and_paints_each_node() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
        let mut tc = TextCache::new();
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
            PresentMode::Editing,
        );

        // Transform stack must be balanced.
        assert!(surface.transforms_balanced());
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            Id::<Element>::from_text("a".into()),
            AnimatedElement {
                id: Id::<Element>::from_text("a".into()),
                bounds: Rect::new(20.0, 20.0, 70.0, 70.0),
                rotation: ambre_core::Radians::ZERO,
                opacity: ambre_core::Bounded::<ambre_core::Opacity>::new(0.5).unwrap(),
                fill: Some(Rgba::from_bits(0x0000_FFFF)),
            },
        );
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
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
            PresentMode::Presenting, // focus mode: grid suppressed
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
        fill: Option<Rgba>,
    ) -> DisplayNode {
        let style = ElementStyle {
            fill,
            stroke: None,
            ..ElementStyle::default()
        };
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind,
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, 100.0, 80.0),
            locked: ambre_bus::app::toile::LockState::Unlocked,
            style,
            geom,
            link: None,
            group: None,
        })
    }

    fn paint(vp: &DisplayViewport) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
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
            PresentMode::Editing,
        );
        surface
    }

    /// `TOILE-UI-346`: a caption sits on a plate in the board colour, and the
    /// plate is painted after the element bodies. A caption that crosses a
    /// sticky is therefore drawn over the card and stays readable.
    #[test]
    fn a_caption_plate_is_painted_over_the_element_bodies() {
        let fill = Rgba::from_bits(0xFEF0_8AFF);
        let mut path = ambre_draw::BezPath::new();
        path.move_to((-200.0, 40.0));
        path.line_to((300.0, 40.0));
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                DisplayNode::Connector(DisplayConnector {
                    id: Id::<Connector>::from_text("c".into()),
                    path,
                    style: ElementStyle::default(),
                    arrows: ambre_bus::app::toile::EndCaps {
                        start: ambre_bus::app::toile::Cap::Plain,
                        end: ambre_bus::app::toile::Cap::Arrow,
                    },
                    label: "yes".to_owned(),
                }),
                styled_elem(
                    "s",
                    ElementKind::Sticky,
                    DisplayGeom::Sticky("hi".into()),
                    Some(fill),
                ),
            ],
        };
        let surface = paint(&vp);
        let ops = surface.ops();
        let card = ops
            .iter()
            .position(
                |op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == style_color(fill, ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE)),
            )
            .expect("the sticky paints its card");
        let plates: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                matches!(op, DrawOp::FillPath { color: Some(c) } if *c == LIGHT_PALETTE.content_bg)
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(plates.len(), 1, "one plate for the one caption");
        assert!(plates[0] > card, "the plate is painted over the card");
        // Font-availability agnostic: any caption glyphs come after the plate.
        for (i, op) in ops.iter().enumerate() {
            if let DrawOp::Glyphs { color: Some(c), .. } = op
                && *c == LIGHT_PALETTE.cell_text
            {
                assert!(i > plates[0], "the words sit on the plate");
            }
        }
    }

    // A sticky note paints a coloured card in the geometry pass: exactly the bug
    // behind "clicking Sticky does nothing". An empty sticky used to derive as
    // Text and paint no background, so it was invisible (toile-styles).
    #[test]
    fn sticky_paints_a_colored_card() {
        let fill = Rgba::from_bits(0xFEF0_8AFF); // sticky yellow
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
        let card = surface.ops().iter().any(
            |op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == style_color(fill, ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE)),
        );
        assert!(
            card,
            "a sticky must paint a coloured card in the geometry pass"
        );
        // A drop shadow is painted behind the card, in the translucent shadow colour.
        let shadow = surface.ops().iter().any(
            |op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == style_color(STICKY_SHADOW, ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE)),
        );
        assert!(shadow, "a sticky must paint a drop shadow behind its card");
        // Any glyphs the body produces use the fixed ink, never the card colour
        // (the ink is independent of `fill`,). Font-availability agnostic:
        // if no glyphs are shaped this simply holds vacuously.
        let card_color = style_color(fill, ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE);
        let ink = style_color(
            STICKY_INK,
            ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE,
        );
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
                size: ambre_bus::app::toile::FontSizeChoice::AUTO,
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        // Sticky on an automatic size -> size derived from the card.
        assert_eq!(
            sticky_or_style_font_size(TextHost::Sticky, &auto, bounds),
            sticky_font_size(bounds)
        );
        // A positive size overrides the auto-sizing.
        let manual = ElementStyle {
            font: ambre_bus::app::toile::FontSpec {
                size: ambre_bus::app::toile::FontSizeChoice::from_whole_points(Some(30)),
                ..Default::default()
            },
            ..ElementStyle::default()
        };
        // The style holds points: 30 points lay out at 40 px.
        assert_eq!(
            sticky_or_style_font_size(TextHost::Sticky, &manual, bounds),
            ambre_core::Px::<ambre_core::Doc>::new(40.0)
        );
        // A text element has no card to track. Auto draws it at the default
        // size: 12 points lay out at 16 px, never at zero.
        assert_eq!(
            sticky_or_style_font_size(TextHost::Text, &auto, bounds),
            ambre_core::Px::<ambre_core::Doc>::new(16.0)
        );
    }

    #[test]
    fn text_ink_precedence() {
        // Explicit text_color always wins, for sticky and text alike.
        assert_eq!(
            resolved_text_ink(
                TextHost::Sticky,
                Some(Rgba::from_bits(0x0012_3400)),
                Some(Rgba::from_bits(0x0099_9900))
            ),
            Some(Rgba::from_bits(0x0012_3400))
        );
        assert_eq!(
            resolved_text_ink(
                TextHost::Text,
                Some(Rgba::from_bits(0x0012_3400)),
                Some(Rgba::from_bits(0x0099_9900))
            ),
            Some(Rgba::from_bits(0x0012_3400))
        );
        // No text_color: a sticky uses its ink; a text element uses its fill.
        assert_eq!(
            resolved_text_ink(TextHost::Sticky, None, Some(Rgba::from_bits(0x0099_9900))),
            Some(STICKY_INK)
        );
        assert_eq!(
            resolved_text_ink(TextHost::Text, None, Some(Rgba::from_bits(0x0099_9900))),
            Some(Rgba::from_bits(0x0099_9900))
        );
        // A text element with neither falls back to the theme (None).
        assert_eq!(resolved_text_ink(TextHost::Text, None, None), None);
    }

    #[test]
    fn sticky_font_size_tracks_the_card_and_is_clamped() {
        // Bigger card -> bigger font.
        let small = sticky_font_size(Rect::new(0.0, 0.0, 60.0, 60.0));
        let big = sticky_font_size(Rect::new(0.0, 0.0, 240.0, 240.0));
        assert!(big > small, "font must grow with the note");
        // Clamped to a legible range at the extremes.
        assert_eq!(
            sticky_font_size(Rect::new(0.0, 0.0, 4.0, 4.0)),
            ambre_core::Px::<ambre_core::Doc>::new(8.0)
        );
        assert_eq!(
            sticky_font_size(Rect::new(0.0, 0.0, 4000.0, 4000.0)),
            ambre_core::Px::<ambre_core::Doc>::new(48.0)
        );
        // A square 120x120 (the default) sits in-range.
        let default = sticky_font_size(Rect::new(0.0, 0.0, 120.0, 120.0));
        assert!(
            (ambre_core::Px::<ambre_core::Doc>::new(8.0)
                ..=ambre_core::Px::<ambre_core::Doc>::new(48.0))
                .contains(&default)
        );
    }

    // A free `Text` element has no card. The geometry pass paints nothing for it,
    // so it emits no FillPath (only its glyphs, in the text pass). This is what
    // keeps Sticky and Text visually distinct (toile-styles).
    #[test]
    fn text_element_paints_no_card() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![styled_elem(
                "t",
                ElementKind::Text,
                DisplayGeom::Text("hi".into()),
                Some(Rgba::from_bits(0x0000_FFFF)),
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
            elem("a", DisplayGeom::Image(Some(hh('1'))), b),
            elem("b", DisplayGeom::Image(Some(hh('2'))), b),
            // The same picture twice: one request, not two.
            elem("c", DisplayGeom::Image(Some(hh('2'))), b),
            // Not a picture: it names no address at all.
            elem("d", DisplayGeom::Rect, b),
        ];
        let mut images = ImageStore::new();
        images.insert(
            hh('1'),
            DecodedImage {
                pixels: vec![0u8; 4].into(),
                size: ambre_core::Size::new(Px::new(1), Px::new(1)),
            },
        );
        let asked = missing_media(&nodes, &images, &std::collections::HashSet::new());
        assert_eq!(asked, vec![hh('2')]);
    }

    /// `TOILE-IO-105`: a picture already asked for is not asked for again. That stops a
    /// request going out on every frame that fails to paint it. That includes
    /// an address the document cannot resolve and never answers.
    #[test]
    fn a_pending_request_is_not_repeated() {
        let b = Rect::new(0.0, 0.0, 10.0, 10.0);
        let nodes = vec![elem("a", DisplayGeom::Image(Some(hh('1'))), b)];
        let mut pending = std::collections::HashSet::new();
        pending.insert(hh('1'));
        assert!(missing_media(&nodes, &ImageStore::new(), &pending).is_empty());
        // With nothing pending, the same list does ask.
        assert_eq!(
            missing_media(
                &nodes,
                &ImageStore::new(),
                &std::collections::HashSet::new()
            ),
            vec![hh('1')]
        );
    }

    // An `Image` element whose hash is in the store paints real pixels
    // (`DrawImage`), scaled to its AABB. One whose hash is absent falls back to
    // a placeholder box (a path fill), per toile-media.
    #[test]
    fn image_paints_pixels_when_resolved_else_placeholder() {
        let bounds = Rect::new(0.0, 0.0, 40.0, 20.0);
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("has", DisplayGeom::Image(Some(hh('1'))), bounds),
                elem("missing", DisplayGeom::Image(Some(hh('2'))), bounds),
            ],
        };
        // Only "h1" is decoded; "h2" is still pending.
        let mut images = ImageStore::new();
        images.insert(
            hh('1'),
            DecodedImage {
                pixels: vec![255u8; 2 * 2 * 4].into(),
                size: ambre_core::Size::new(Px::new(2), Px::new(2)),
            },
        );
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
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
            PresentMode::Editing,
        );

        // Exactly one real image paint, sized to the element AABB.
        let draws: Vec<_> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::DrawImage { size, dest } => {
                    Some((size.width.get(), size.height.get(), *dest))
                }
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 1, "one resolved image paints pixels");
        assert_eq!(draws[0], (2, 2, bounds));
        // The unresolved image falls back to a placeholder path fill.
        let fills = surface.count(|o| matches!(o, DrawOp::FillPath { .. }));
        assert_eq!(fills.get(), 1, "the pending image paints a placeholder box");
        assert!(surface.transforms_balanced());
    }

    #[test]
    fn overlay_draws_box_and_handles_for_single_selection() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            &[Id::<Element>::from_text("r".into())],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            PresentMode::Editing,
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![elem("g", DisplayGeom::Group, bounds)],
        };
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[Id::<Element>::from_text("g".into())],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            PresentMode::Editing,
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

    /// `TOILE-UI-230`: a multi-selection wears one upright union box and no
    /// resize handle, even when a member is turned. The box is the crisp
    /// upright rect, not four rotated edges, and the knob is the one square.
    #[test]
    fn a_multi_selection_wears_an_upright_union_box_and_no_resize_handle() {
        let DisplayNode::Element(mut turned) =
            elem("b", DisplayGeom::Rect, Rect::new(100.0, 0.0, 150.0, 50.0))
        else {
            panic!("elem builds an element")
        };
        turned.transform = Affine::rotate_about(0.5, Point::new(125.0, 25.0));
        let vp = board(vec![
            elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
            DisplayNode::Element(turned),
        ]);
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[
                Id::<Element>::from_text("a".into()),
                Id::<Element>::from_text("b".into()),
            ],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            PresentMode::Editing,
        );
        let count = |f: fn(&DrawOp) -> bool| surface.ops().iter().filter(|o| f(o)).count();
        assert_eq!(count(|o| matches!(o, DrawOp::StrokeRect { .. })), 1);
        assert_eq!(
            count(|o| matches!(o, DrawOp::StrokeLine { .. })),
            1,
            "the stem"
        );
        assert_eq!(
            count(|o| matches!(o, DrawOp::FillRect { .. })),
            1,
            "the knob"
        );
        let union = surface
            .ops()
            .iter()
            .find_map(|o| match o {
                DrawOp::StrokeRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("the box");
        let a = world_rect_to_screen(Rect::new(0.0, 0.0, 50.0, 50.0), &cam());
        assert!(union.x0 <= a.x0 && union.y0 <= a.y0, "the box misses a");
        assert!(union.width() > a.width() * 2.0, "the box misses b");
    }

    /// `TOILE-UI-120`: a group paints nothing of its own. The node the engine
    /// derives for it, painted alone, draws no fill, no border and no title.
    #[test]
    fn a_derived_group_paints_nothing_of_its_own() {
        use crate::logic::{InitContent, ToileCommand, ToileEvent, ToileState};
        let mut st = ToileState::new();
        let mut ids = Vec::new();
        for x in [0.0, 100.0] {
            let out = st.handle_command(ToileCommand::CreateElement {
                kind: ElementKind::Shape(ShapeKind::Rect),
                at: Point::new(x, 0.0),
                size: kurbo::Size::new(50.0, 50.0),
                content: InitContent::None,
                fill: None,
                correlate: None,
            });
            ids.extend(out.display.iter().filter_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
                _ => None,
            }));
        }
        st.handle_command(ToileCommand::GroupElements { ids });
        let group = st
            .debug_viewport()
            .nodes
            .into_iter()
            .find(|n| matches!(n, DisplayNode::Element(e) if matches!(e.geom, DisplayGeom::Group)))
            .expect("the group is derived as a node");
        // The background and the grid are drawn either way; the group adds
        // nothing to them. A transform pushed and popped around nothing
        // draws nothing, so the stack ops are left out of the comparison.
        let drawn = |nodes: Vec<DisplayNode>| -> Vec<DrawOp> {
            paint_pass(&board(nodes), &cam(), &Pass::default())
                .ops()
                .iter()
                .filter(|o| !matches!(o, DrawOp::PushTransform { .. } | DrawOp::PopTransform))
                .cloned()
                .collect()
        };
        assert_eq!(
            drawn(vec![group]),
            drawn(Vec::new()),
            "a group painted something"
        );
    }

    /// `TOILE-253`, the glyphs: a named frame's title is shaped and drawn in
    /// the title ink, above the frame's top edge. A frame nobody named draws
    /// its placeholder in the secondary ink instead, so it is never blank.
    #[test]
    fn a_frame_title_is_drawn_above_the_frame_in_its_ink() {
        let bounds = Rect::new(0.0, 0.0, 300.0, 200.0);
        let top = world_rect_to_screen(bounds, &cam()).y0;
        for (title, ink) in [
            ("Agenda", LIGHT_PALETTE.cell_text),
            ("  ", LIGHT_PALETTE.header_text),
        ] {
            let vp = board(vec![elem("f", DisplayGeom::Frame(title.into()), bounds)]);
            let painted = runs(&paint_pass(&vp, &cam(), &Pass::default()));
            assert!(!painted.is_empty(), "{title:?}: no title was shaped");
            for (color, tf, depth) in painted {
                assert_eq!(color, Some(ink), "{title:?}");
                assert_eq!(depth, 0, "{title:?}: a title under the camera transform");
                assert!(
                    tf.translation().y <= top,
                    "{title:?}: the title covers the frame"
                );
            }
        }
    }

    /// `TOILE-UI-083`: presentation mode paints no editing surface. The same
    /// selection that draws a box and five handles above draws neither while
    /// the mode holds.
    #[test]
    fn overlay_paints_no_editing_surface_while_presenting() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            &[Id::<Element>::from_text("r".into())],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[Id::<Element>::from_text("r".into())],
            None,
            &HashMap::new(),
            None,
            PresentMode::Presenting,
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            Id::<Element>::from_text("r".into()),
            AnimatedElement {
                id: Id::<Element>::from_text("r".into()),
                bounds: preview_bounds,
                rotation: ambre_core::Radians::ZERO,
                opacity: ambre_core::Bounded::<ambre_core::Opacity>::new(1.0).unwrap(),
                fill: None,
            },
        );
        let cam = cam();
        let mut surface = RecordingSurface::new();
        paint_overlay(
            &mut surface,
            &[Id::<Element>::from_text("r".into())],
            &vp,
            &cam,
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &gesture,
            None,
            PresentMode::Editing,
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![DisplayNode::Connector(DisplayConnector {
                id: Id::<Connector>::from_text("c".into()),
                path,
                style: ElementStyle::default(),
                arrows: ambre_bus::app::toile::EndCaps {
                    start: ambre_bus::app::toile::Cap::Plain,
                    end: ambre_bus::app::toile::Cap::Arrow,
                },
                label: String::new(),
            })],
        };
        let mut surface = RecordingSurface::new();
        let selected = Id::<Connector>::from_text("c".into());
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
            PresentMode::Editing,
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

    /// `TOILE-UI-406`: a linked element gets a badge outside its top-right
    /// corner; an unlinked one gets none (`toile-links`).
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            PresentMode::Editing,
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: Vec::new(),
        };
        let mut surface = RecordingSurface::new();
        let selected = Id::<Connector>::from_text("gone".into());
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
            PresentMode::Editing,
        );
        assert!(surface.ops().is_empty(), "nothing is painted");
    }

    #[test]
    fn overlay_outlines_visible_find_hits() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
                elem("b", DisplayGeom::Rect, Rect::new(60.0, 0.0, 110.0, 50.0)),
            ],
        };
        let mut surface = RecordingSurface::new();
        let hits = vec![
            Id::<Element>::from_text("a".into()),
            Id::<Element>::from_text("b".into()),
        ];
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &hits,
            Some(&Id::<Element>::from_text("b".into())),
            &HashMap::new(),
            None,
            PresentMode::Editing,
        );
        // No selection => no handles, only the two hit outlines (one per match).
        let strokes = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::StrokeRect { .. }))
            .count();
        assert_eq!(strokes, 2);
    }

    /// `TOILE-UI-112`, `TOILE-UI-357`: the current match wears the selection
    /// accent at a heavier width, and every other match a softer tint.
    #[test]
    fn the_current_match_is_outlined_apart_from_the_rest() {
        let vp = DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes: vec![
                elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
                elem("b", DisplayGeom::Rect, Rect::new(60.0, 0.0, 110.0, 50.0)),
                elem("c", DisplayGeom::Rect, Rect::new(120.0, 0.0, 170.0, 50.0)),
            ],
        };
        let mut surface = RecordingSurface::new();
        let hits = ["a", "b", "c"].map(|s| Id::<Element>::from_text(s.into()));
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &hits,
            Some(&Id::<Element>::from_text("b".into())),
            &HashMap::new(),
            None,
            PresentMode::Editing,
        );
        let outlines: Vec<(Color, f64)> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::StrokeRect { color, width, .. } => Some((*color, width.get())),
                _ => None,
            })
            .collect();
        let [a, b, c] = outlines[..] else {
            panic!("one outline per match: {outlines:?}");
        };
        assert_eq!(a, c, "the other matches look alike");
        assert_eq!(
            b.0, LIGHT_PALETTE.selection,
            "the current one in the accent"
        );
        assert_ne!(a.0, b.0);
        assert!(b.1 > a.1, "and heavier");
    }

    // Build a node rotated `theta` radians about its AABB center, mirroring the
    // Logic derive so the overlay / text passes see a realistic transform.
    fn rotated_elem(id: &str, geom: DisplayGeom, bounds: Rect, theta: f64) -> DisplayNode {
        let c = bounds.center();
        let transform = Affine::translate(c.to_vec2())
            * Affine::rotate(theta)
            * Affine::translate(-c.to_vec2());
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Sticky,
            transform,
            bounds,
            locked: ambre_bus::app::toile::LockState::Unlocked,
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
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
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
            &[Id::<Element>::from_text("r".into())],
            &vp,
            &cam,
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            PresentMode::Editing,
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
        let expect_se = super::rotate_about(b.center(), ambre_core::Radians::new(FRAC_PI_4))
            * Point::new(b.x1, b.y1);
        let half = f64::from(HANDLE_SIZE.get()) / 2.0;
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

    /// A context holding the shipped Noto Sans face alone, read from the tree.
    /// Every host then shapes the same runs, so a test on the glyphs cannot
    /// pass on a surface that drew no text.
    fn shipped_fonts() -> FontContext {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ambre-app/fonts/NotoSans-Regular.ttf"
        );
        let face = std::fs::read(path).expect("the shipped face is in the tree");
        ambre_canvas::fonts::font_context_from_bytes([
            ambre_core::Payload::<ambre_core::FontBytes>::new(face),
        ])
    }

    fn board(nodes: Vec<DisplayNode>) -> DisplayViewport {
        DisplayViewport {
            version: ambre_core::Generation::ZERO,
            grid_unit: grid::BASE_GRID,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Default::default(),
            nodes,
        }
    }

    /// What one content pass needs beyond the board and the camera.
    #[derive(Default)]
    struct Pass {
        editing: Option<crate::ui::edit::EditTarget>,
        images: ImageStore,
        anim: HashMap<Id<Element>, AnimatedElement>,
        present: PresentMode,
    }

    /// One content pass with the shipped face.
    fn paint_pass(vp: &DisplayViewport, cam: &ViewportTransform, pass: &Pass) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = shipped_fonts();
        let mut tc = TextCache::new();
        paint_content(
            &mut surface,
            vp,
            cam,
            &LIGHT_PALETTE,
            &mut font_cx,
            &mut tc,
            pass.editing.as_ref(),
            &pass.images,
            &pass.anim,
            pass.present,
        );
        surface
    }

    /// Each glyph run painted, as its colour and its origin transform, with
    /// the depth of the transform stack it was painted under.
    fn runs(surface: &RecordingSurface) -> Vec<(Option<Color>, Affine, usize)> {
        let mut depth = 0usize;
        let mut out = Vec::new();
        for op in surface.ops() {
            match op {
                DrawOp::PushTransform { .. } => depth += 1,
                DrawOp::PopTransform => depth -= 1,
                DrawOp::Glyphs {
                    color, transform, ..
                } => out.push((*color, *transform, depth)),
                _ => {}
            }
        }
        out
    }

    fn filled(id: &str, fill: Rgba, bounds: Rect) -> DisplayNode {
        let DisplayNode::Element(mut e) = elem(id, DisplayGeom::Rect, bounds) else {
            panic!("elem builds an element")
        };
        e.style.fill = Some(fill);
        e.style.stroke = None;
        DisplayNode::Element(e)
    }

    fn arrow(id: &str, stroke: Rgba, from: (f64, f64), to: (f64, f64)) -> DisplayNode {
        let mut path = BezPath::new();
        path.move_to(from);
        path.line_to(to);
        DisplayNode::Connector(DisplayConnector {
            id: Id::<Connector>::from_text(id.into()),
            path,
            style: ElementStyle {
                stroke: Some(stroke),
                ..ElementStyle::default()
            },
            arrows: ambre_bus::app::toile::EndCaps {
                start: ambre_bus::app::toile::Cap::Plain,
                end: ambre_bus::app::toile::Cap::Arrow,
            },
            label: String::new(),
        })
    }

    fn note(id: &str, body: &str, fill: Rgba, bounds: Rect) -> DisplayNode {
        let DisplayNode::Element(mut e) = styled_elem(
            id,
            ElementKind::Sticky,
            DisplayGeom::Sticky(body.into()),
            Some(fill),
        ) else {
            panic!("styled_elem builds an element")
        };
        e.bounds = bounds;
        DisplayNode::Element(e)
    }

    /// `TOILE-UI-043`: the walk paints the nodes in the order it is handed,
    /// an arrow between the elements around it. Reversed, the paint reverses.
    #[test]
    fn the_walk_paints_in_the_order_it_is_handed() {
        let (red, blue, green) = (
            Rgba::from_bits(0xFF00_00FF),
            Rgba::from_bits(0x0000_FFFF),
            Rgba::from_bits(0x00FF_00FF),
        );
        let a = filled("a", red, Rect::new(0.0, 0.0, 50.0, 50.0));
        let b = filled("b", green, Rect::new(20.0, 20.0, 70.0, 70.0));
        let c = arrow("c", blue, (-40.0, 30.0), (120.0, 30.0));
        let order = |nodes: Vec<DisplayNode>| {
            let surface = paint_pass(&board(nodes), &cam(), &Pass::default());
            let at = |want: Rgba| {
                surface
                    .ops()
                    .iter()
                    .position(|op| match op {
                        DrawOp::FillPath { color: Some(k) } => {
                            *k == style_color(
                                want,
                                ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE,
                            )
                        }
                        DrawOp::StrokePath { color: Some(k), .. } => {
                            *k == style_color(
                                want,
                                ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE,
                            )
                        }
                        _ => false,
                    })
                    .expect("every node paints")
            };
            [at(red), at(blue), at(green)]
        };
        let [r, k, g] = order(vec![a.clone(), c.clone(), b.clone()]);
        assert!(r < k && k < g, "the arrow paints between: {r} {k} {g}");
        let [r, k, g] = order(vec![b, c, a]);
        assert!(g < k && k < r, "the order is the one handed: {r} {k} {g}");
    }

    /// `TOILE-UI-262`: a note's body is drawn in the ink chosen for its card,
    /// never in the card colour, and it reads on every card. The runs are
    /// shaped with the shipped face, so the loop cannot run empty.
    #[test]
    fn a_note_body_is_drawn_in_an_ink_that_reads_on_its_card() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 160.0);
        let cards = [
            Rgba::from_bits(0xFEF0_8AFF),
            Rgba::from_bits(0xFFFF_FFFF),
            Rgba::from_bits(0x0000_00FF),
            STICKY_INK,
            Rgba::from_bits(0x1E3A_8AFF),
            Rgba::from_bits(0x7777_77FF),
            Rgba::from_bits(0xE539_35FF),
        ];
        for card in cards {
            let vp = board(vec![note("s", "Plan the week", card, bounds)]);
            let painted = runs(&paint_pass(&vp, &cam(), &Pass::default()));
            assert!(!painted.is_empty(), "card {card:08x}: no run was shaped");
            let ink =
                resolved_text_ink(TextHost::Sticky, None, Some(card)).expect("a note has an ink");
            assert_ne!(ink, card, "card {card:08x}");
            assert!(contrast(ink, card) >= 4.0, "card {card:08x}");
            for (color, _, _) in painted {
                assert_eq!(
                    color,
                    Some(style_color(
                        ink,
                        ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE
                    )),
                    "card {card:08x}"
                );
            }
        }
    }

    /// `TOILE-UI-264`: a group paints nothing between its own push and pop;
    /// an embed and a nested document each paint the box that holds their place.
    #[test]
    fn a_group_paints_nothing_and_an_embed_holds_its_place() {
        let bounds = Rect::new(0.0, 0.0, 80.0, 60.0);
        let inner = |geom: DisplayGeom| {
            let surface = paint_pass(
                &board(vec![elem("x", geom, bounds)]),
                &cam(),
                &Pass::default(),
            );
            let ops = surface.ops();
            // The camera push, then the element's own push.
            let open = ops
                .iter()
                .enumerate()
                .filter(|(_, o)| matches!(o, DrawOp::PushTransform { .. }))
                .nth(1)
                .map(|(i, _)| i)
                .expect("the element pushes its transform");
            let close = open
                + ops[open..]
                    .iter()
                    .position(|o| matches!(o, DrawOp::PopTransform))
                    .expect("and pops it");
            ops[open + 1..close].to_vec()
        };
        assert!(
            inner(DisplayGeom::Group).is_empty(),
            "a group paints nothing"
        );
        let doc = DisplayGeom::DocEmbed {
            document_id: Id::<ambre_core::id::Document>::mint(),
            doc_type: ambre_bus::DocType::Trame,
            focused: ambre_bus::app::toile::InputFocus::Board,
        };
        for geom in [
            DisplayGeom::Embed(
                ambre_bus::app::toile::WebAddress::parse("https://example.org").ok(),
            ),
            doc,
        ] {
            let ops = inner(geom.clone());
            assert!(
                matches!(ops[..], [DrawOp::FillPath { .. }, ..]),
                "{geom:?} paints a box: {ops:?}"
            );
        }
    }

    /// `TOILE-UI-044`: a body is laid out in board units and painted under the
    /// camera transform, the page's layout. Its runs sit at the same board
    /// point at every zoom, below the padded top of the card. The surface
    /// rasterizes them at their screen size.
    #[test]
    fn a_body_is_laid_out_in_board_units_under_the_camera() {
        let bounds = Rect::new(-100.0, -80.0, 100.0, 80.0);
        let vp = board(vec![note(
            "s",
            "Plan",
            Rgba::from_bits(0xFEF0_8AFF),
            bounds,
        )]);
        let origin = |zoom: f64| {
            let mut cam = cam();
            cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom);
            let painted = runs(&paint_pass(&vp, &cam, &Pass::default()));
            let (_, first, depth) = *painted.first().expect("the body is shaped");
            assert_eq!(depth, 1, "zoom {zoom}: a run under the camera transform");
            first.translation()
        };
        let one = origin(1.0);
        assert_eq!(origin(2.0), one, "the layout does not follow the zoom");
        assert_eq!(one.x, bounds.x0 + crate::ui::walk::TEXT_PAD.get());
        assert!(one.y > bounds.y0 + crate::ui::walk::TEXT_PAD.get());
    }

    /// `TOILE-UI-265`: a rotated note rotates its body and its rules with it.
    /// Each run and each rule carries the element's angle.
    #[test]
    fn a_rotated_note_carries_its_body_and_rules_round() {
        use std::f64::consts::FRAC_PI_6;
        let DisplayNode::Element(mut e) = rotated_elem(
            "s",
            DisplayGeom::Sticky("Plan the week".into()),
            Rect::new(0.0, 0.0, 200.0, 160.0),
            FRAC_PI_6,
        ) else {
            panic!("rotated_elem builds an element")
        };
        e.style.font.underline = ambre_core::Underline::Underlined;
        e.style.font.strikethrough = ambre_core::Strikethrough::Struck;
        let surface = paint_pass(
            &board(vec![DisplayNode::Element(e)]),
            &cam(),
            &Pass::default(),
        );
        let painted = runs(&surface);
        assert!(!painted.is_empty(), "the body is shaped");
        let ink = painted[0].0;
        for (_, tf, _) in painted {
            let c = tf.as_coeffs();
            assert!(
                (c[1].atan2(c[0]) - FRAC_PI_6).abs() < 1e-9,
                "a run turned {c:?}"
            );
        }
        let rules: Vec<f64> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                // The rules are drawn in the body's ink; other lines are chrome.
                DrawOp::StrokeLine { line, color, .. } if Some(*color) == ink => {
                    Some((line.p1.y - line.p0.y).atan2(line.p1.x - line.p0.x))
                }
                _ => None,
            })
            .collect();
        assert!(
            rules.len() >= 2,
            "an underline and a strikethrough: {rules:?}"
        );
        for angle in rules {
            assert!((angle - FRAC_PI_6).abs() < 1e-9, "a rule at {angle}");
        }
    }

    /// `TOILE-UI-277` and `TOILE-UI-358`: the overridden element paints at the
    /// override's bounds and rotation; the other one paints where it committed.
    #[test]
    fn an_override_moves_its_own_element_and_no_other() {
        let (at_a, at_b) = (
            Rect::new(0.0, 0.0, 40.0, 20.0),
            Rect::new(100.0, 0.0, 140.0, 20.0),
        );
        let tween = AnimatedElement {
            id: Id::<Element>::from_text("a".into()),
            bounds: Rect::new(20.0, 30.0, 80.0, 60.0),
            rotation: ambre_core::Radians::new(0.5),
            opacity: ambre_core::Bounded::<ambre_core::Opacity>::new(1.0).unwrap(),
            fill: None,
        };
        let mut pass = Pass::default();
        pass.images.insert(
            hh('f'),
            DecodedImage {
                pixels: vec![255u8; 2 * 2 * 4].into(),
                size: ambre_core::Size::new(Px::new(2), Px::new(2)),
            },
        );
        pass.anim.insert(tween.id.clone(), tween.clone());
        let vp = board(vec![
            elem("a", DisplayGeom::Image(Some(hh('f'))), at_a),
            elem("b", DisplayGeom::Image(Some(hh('f'))), at_b),
        ]);
        let surface = paint_pass(&vp, &cam(), &pass);
        let dests: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::DrawImage { dest, .. } => Some(*dest),
                _ => None,
            })
            .collect();
        assert_eq!(dests, vec![tween.bounds, at_b]);
        let pushes: Vec<Affine> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::PushTransform { transform } => Some(*transform),
                _ => None,
            })
            .collect();
        // The camera, then each element's own transform.
        assert_eq!(pushes[1..], [anim_transform(&tween), Affine::IDENTITY]);
    }

    /// `TOILE-UI-266`: the note under the inline editor paints its card and
    /// no body. Its neighbour keeps its own, and nothing else changes.
    #[test]
    fn the_note_under_the_editor_paints_no_body() {
        let vp = board(vec![
            note(
                "a",
                "First",
                Rgba::from_bits(0xFEF0_8AFF),
                Rect::new(0.0, 0.0, 160.0, 120.0),
            ),
            note(
                "b",
                "Second note",
                Rgba::from_bits(0xFEF0_8AFF),
                Rect::new(200.0, 0.0, 360.0, 120.0),
            ),
        ]);
        let all = paint_pass(&vp, &cam(), &Pass::default());
        let editing = paint_pass(
            &vp,
            &cam(),
            &Pass {
                editing: Some(crate::ui::edit::EditTarget::Element(
                    Id::<Element>::from_text("a".into()),
                )),
                ..Pass::default()
            },
        );
        // The bodies are laid out in board units, under the camera.
        let b_left = 200.0;
        let (before, after) = (runs(&all), runs(&editing));
        assert!(
            before.iter().any(|r| r.1.translation().x < b_left),
            "a has a body"
        );
        assert!(!after.is_empty(), "b keeps its body");
        assert!(
            after.iter().all(|r| r.1.translation().x >= b_left),
            "a body under the editor was painted"
        );
        let rest = |s: &RecordingSurface| -> Vec<DrawOp> {
            s.ops()
                .iter()
                .filter(|o| !matches!(o, DrawOp::Glyphs { .. }))
                .cloned()
                .collect()
        };
        assert_eq!(rest(&all), rest(&editing), "the cards are untouched");
    }

    /// `TOILE-UI-045` and `TOILE-UI-267`: the content pass opens on the board
    /// background over the whole canvas. The grid lines follow on the board,
    /// and presentation focus paints none.
    #[test]
    fn the_background_comes_first_and_focus_drops_the_grid() {
        let vp = board(vec![filled(
            "a",
            Rgba::from_bits(0xFF00_00FF),
            Rect::new(0.0, 0.0, 50.0, 50.0),
        )]);
        let canvas = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let lines = |s: &RecordingSurface| {
            s.count(|o| matches!(o, DrawOp::StrokeLine { color, .. } if *color == LIGHT_PALETTE.grid_line))
        };
        for present in [PresentMode::Editing, PresentMode::Presenting] {
            let surface = paint_pass(
                &vp,
                &cam(),
                &Pass {
                    present,
                    ..Pass::default()
                },
            );
            assert_eq!(
                surface.ops().first(),
                Some(&DrawOp::FillRect {
                    rect: canvas,
                    color: LIGHT_PALETTE.content_bg
                }),
                "present {present:?}"
            );
            if present.is_presenting() {
                assert_eq!(lines(&surface).get(), 0, "focus paints no grid line");
            } else {
                assert!(lines(&surface).get() > 0, "the board paints its grid");
            }
        }
    }

    /// `TOILE-UI-268`: several selected elements show their union box and the
    /// rotate knob, and no corner square.
    #[test]
    fn several_selected_elements_show_their_union_box_alone() {
        let vp = board(vec![
            elem("a", DisplayGeom::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
            elem("b", DisplayGeom::Rect, Rect::new(100.0, 80.0, 150.0, 120.0)),
        ]);
        let mut surface = RecordingSurface::new();
        let selection = [
            Id::<Element>::from_text("a".into()),
            Id::<Element>::from_text("b".into()),
        ];
        paint_overlay(
            &mut surface,
            &selection,
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &[],
            None,
            &HashMap::new(),
            None,
            PresentMode::Editing,
        );
        let union = world_rect_to_screen(Rect::new(0.0, 0.0, 150.0, 120.0), &cam());
        let boxes: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::StrokeRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(boxes, vec![union], "one box, the union");
        let squares: Vec<Point> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::FillRect { rect, .. } => Some(rect.center()),
                _ => None,
            })
            .collect();
        assert_eq!(squares, vec![rotate_handle_point(union)], "the knob alone");
    }

    /// `TOILE-UI-269`: a match the culled set does not hold is out of view,
    /// and it gets no outline. The match in view gets its own.
    #[test]
    fn a_match_out_of_view_gets_no_outline() {
        let vp = board(vec![elem(
            "a",
            DisplayGeom::Rect,
            Rect::new(0.0, 0.0, 50.0, 50.0),
        )]);
        let mut surface = RecordingSurface::new();
        let hits = [
            Id::<Element>::from_text("far".into()),
            Id::<Element>::from_text("a".into()),
        ];
        paint_overlay(
            &mut surface,
            &[],
            &vp,
            &cam(),
            &LIGHT_PALETTE,
            None,
            &hits,
            Some(&hits[0]),
            &HashMap::new(),
            None,
            PresentMode::Editing,
        );
        let outlines: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|o| match o {
                DrawOp::StrokeRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        let a = world_rect_to_screen(Rect::new(0.0, 0.0, 50.0, 50.0), &cam());
        assert_eq!(outlines.len(), 1, "{outlines:?}");
        assert!(outlines[0].contains(a.center()), "the outline is a's");
    }

    /// `TOILE-UI-270`: each preview paints over an empty overlay, on the
    /// geometry it will commit. The marquee is a screen rectangle; the rest
    /// are world paths under the camera, at a screen-constant width.
    #[test]
    fn every_preview_tracks_the_geometry_it_will_commit() {
        let mut cam = cam();
        cam.zoom = ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(2.0);
        let r = Rect::new(10.0, 20.0, 110.0, 70.0);
        let mut pen = BezPath::new();
        pen.move_to((0.0, 0.0));
        pen.curve_to((10.0, 40.0), (60.0, 40.0), (70.0, 0.0));
        let pts = vec![
            Point::new(5.0, 5.0),
            Point::new(45.0, 25.0),
            Point::new(85.0, 5.0),
        ];
        let paint = |p: &Preview| {
            let mut surface = RecordingSurface::new();
            paint_overlay(
                &mut surface,
                &[],
                &board(Vec::new()),
                &cam,
                &LIGHT_PALETTE,
                Some(p),
                &[],
                None,
                &HashMap::new(),
                None,
                PresentMode::Editing,
            );
            surface.ops().to_vec()
        };
        let world = cam.world_to_screen();
        let stroked = |ops: &[DrawOp], bounds: Rect, px: f64| match ops {
            [
                DrawOp::PushTransform { transform },
                DrawOp::StrokePath {
                    color,
                    bounds: b,
                    width,
                },
                DrawOp::PopTransform,
            ] => {
                assert_eq!(*transform, world, "under the camera");
                assert_eq!(*color, Some(LIGHT_PALETTE.selection));
                assert_eq!(*b, bounds);
                assert_eq!(width.get() * cam.zoom.get(), px, "a screen-constant width");
            }
            other => panic!("one stroked path: {other:?}"),
        };
        let ops = paint(&Preview::Marquee(r));
        assert_eq!(
            ops,
            vec![DrawOp::StrokeRect {
                rect: world_rect_to_screen(r, &cam),
                color: LIGHT_PALETTE.selection,
                width: ambre_core::Px::<ambre_core::Surface>::new(1.0)
            }]
        );
        for kind in [
            ShapeKind::Rect,
            ShapeKind::Ellipse,
            ShapeKind::Triangle,
            ShapeKind::Line,
        ] {
            stroked(&paint(&Preview::Shape { kind, rect: r }), r, 1.5);
        }
        let (from, to) = (Point::new(-30.0, 10.0), Point::new(90.0, 60.0));
        stroked(
            &paint(&Preview::Connector { from, to }),
            Rect::from_points(from, to),
            1.5,
        );
        stroked(&paint(&Preview::Pen(pen.clone())), pen.control_box(), 1.5);
        stroked(
            &paint(&Preview::Stroke(pts)),
            Rect::new(5.0, 5.0, 85.0, 25.0),
            2.0,
        );
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
