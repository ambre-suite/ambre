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

use ambre_bus::app::toile::{
    DisplayConnector, DisplayElement, DisplayGeom, DisplayNode, DisplayViewport, ElementId,
    ElementStyle, ShapeKind,
};
use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, BezPath, Brush, Color, DrawSurface, FontContext, Point,
    PositionedLayoutItem, Rect, Stroke,
};

use super::grid;
use super::hit::triangle_path;
use super::tool::{
    HANDLE_SIZE_PX, Preview, resize_handle_points, rotate_handle_point, selection_bounds,
    world_rect_to_screen,
};
use super::viewport::ViewportTransform;

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
pub fn paint_content<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    editing: Option<&ElementId>,
) {
    // 1. Background + adaptive grid (screen space).
    grid::paint_grid(surface, cam, palette);

    // 2. Shapes / paths / strokes / connectors, in world space under the camera.
    surface.push_transform(cam.world_to_screen());
    for node in &vp.nodes {
        match node {
            DisplayNode::Element(e) => paint_element(surface, e),
            DisplayNode::Connector(c) => paint_connector(surface, c),
        }
    }
    surface.pop_transform();

    // 3. Text pass in screen space (crisp glyphs; font size scales with zoom).
    for node in &vp.nodes {
        if let DisplayNode::Element(e) = node
            && let DisplayGeom::Text(text) = &e.geom
            // Skip the element under inline edit: its live buffer is painted by
            // the editor overlay instead (§6).
            && editing != Some(&e.id)
        {
            paint_text(surface, e, text, cam, palette, font_cx, text_cache);
        }
    }
}

/// Paint one element under the (already pushed) camera transform. Rotation is
/// applied by pushing the element's own transform, so geometry is expressed in
/// the unrotated world frame (matching hit-testing, `hit.rs`).
fn paint_element<S: DrawSurface>(surface: &mut S, e: &DisplayElement) {
    surface.push_transform(e.transform);
    match &e.geom {
        DisplayGeom::Rect | DisplayGeom::Frame => {
            fill_and_stroke(surface, &rect_path(e.bounds), &e.style);
        }
        DisplayGeom::Ellipse => fill_and_stroke(surface, &ellipse_path(e.bounds), &e.style),
        DisplayGeom::Triangle => fill_and_stroke(surface, &triangle_path(e.bounds), &e.style),
        DisplayGeom::Path(p) => fill_and_stroke(surface, p, &e.style),
        DisplayGeom::Stroke(pts) => {
            // A stroke has no fill; paint the smoothed polyline outline only.
            if let Some(stroke) = e.style.stroke {
                surface.stroke_path(
                    &polyline_path(pts),
                    &Brush::Solid(argb(stroke, e.style.opacity)),
                    &Stroke::new(e.style.stroke_w.max(1.0) as f64),
                );
            }
        }
        // Media / embeds paint a placeholder box in Phase 0 (pixels + iframe land
        // in toile-interop). Text is handled in the screen-space pass.
        DisplayGeom::Image(_) | DisplayGeom::Embed(_) | DisplayGeom::DocEmbed { .. } => {
            fill_and_stroke(surface, &rect_path(e.bounds), &e.style);
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
fn paint_text<S: DrawSurface>(
    surface: &mut S,
    e: &DisplayElement,
    text: &str,
    cam: &ViewportTransform,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
) {
    if text.is_empty() {
        return;
    }
    let sb = world_rect_to_screen(e.bounds, cam);
    let font_size = (e.style.font.size as f64 * cam.zoom) as f32;
    let color = e
        .style
        .fill
        .map(|f| argb(f, e.style.opacity))
        .unwrap_or(palette.cell_text);
    let layout = text_cache.get_or_build(
        font_cx,
        text,
        font_size,
        sb.width() as f32,
        Alignment::Start,
        e.style.font.weight >= 700,
        e.style.font.italic,
    );
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = sb.x0 + run.offset() as f64;
            let ty = sb.y0 + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// Paint the transient overlay: the selection bounding box + 8 resize handles +
/// the rotate handle, in screen space (toile-ui §2 pass 3, §7). Nothing is drawn
/// for an empty selection.
pub fn paint_overlay<S: DrawSurface>(
    surface: &mut S,
    selection: &[ambre_bus::app::toile::ElementId],
    vp: &DisplayViewport,
    cam: &ViewportTransform,
    palette: &Palette,
    preview: Option<&Preview>,
) {
    // Live gesture preview (world -> screen), a thin outline / trail.
    if let Some(p) = preview {
        paint_preview(surface, p, cam, palette);
    }

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

    #[test]
    fn overlay_draws_box_and_handles_for_single_selection() {
        let vp = DisplayViewport {
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
        );
        // 8 resize handles + 1 rotate handle = 9 filled squares.
        let handle_fills = surface
            .ops()
            .iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count();
        assert_eq!(handle_fills, 9);
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
