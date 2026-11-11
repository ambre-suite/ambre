// ambre-trame/src/ui/renderer/overlay.rs
// Overlay rendering: headers, selection, marching ants, badges, autofilter buttons.

use super::*;
use crate::CellPos;

/// Marching-ants dash geometry (logical px): 6px dash + 4px gap. The animation
/// crawls the dashes by advancing a phase across this period each frame.
const MARCHING_DASH: Px<Surface> = Px::<Surface>::new(6.0);

const MARCHING_GAP: Px<Surface> = Px::<Surface>::new(4.0);

const MARCHING_PERIOD: Px<Surface> = Px::<Surface>::new(MARCHING_DASH.get() + MARCHING_GAP.get());

/// Radius of the inline insert pill (`trame-headers`).
const BADGE_RADIUS: Px<Surface> = Px::<Surface>::new(7.5);

/// Fill a disc of radius `radius` centred on `center` with solid `color`. It
/// is painted as 1px-tall horizontal `fill_rect` spans, since the GPU surface
/// only fills axis-aligned quads, so the circle is reproduced scanline by
/// scanline. The half-pixel span height keeps the rim from looking ragged at
/// small radii.
pub(crate) fn fill_disc<S: DrawSurface>(
    s: &mut S,
    center: kurbo::Point,
    radius: Px<Surface>,
    color: Color,
) {
    // The surface draws in bare f64: the lengths unwrap once, here.
    let (cx, cy, r) = (center.x, center.y, radius.get());
    if r <= 0.0 {
        return;
    }
    let mut y = (cy - r).floor() + 0.5;
    while y < cy + r {
        let dy = y - cy;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        if half > 0.0 {
            s.fill_rect(Rect::new(cx - half, y - 0.5, cx + half, y + 0.5), color);
        }
        y += 1.0;
    }
}

/// Inline insert badge: an Excel-style filled pill floating on the hovered header
/// boundary (`trame-headers`). A solid `fill` disc with a 1px contrasting `ring` halo
/// (so it reads on both header and content backgrounds) and a centred `glyph`-
/// coloured "+". Sized to sit comfortably on a header border without overlapping
/// the neighbouring labels.
pub(crate) fn draw_insert_badge<S: DrawSurface>(
    s: &mut S,
    center: kurbo::Point,
    fill: Color,
    glyph: Color,
) {
    let r = BADGE_RADIUS;
    // Halo: a slightly larger disc in the glyph colour gives the pill a crisp
    // outline against the header. The pill then floats rather than smearing
    // into it.
    fill_disc(s, center, r + Px::<Surface>::new(1.0), glyph);
    fill_disc(s, center, r, fill);
    // The "+" sign, in the contrasting glyph colour.
    let (cx, cy) = (center.x, center.y);
    let arm = 4.0;
    let t = 1.0;
    s.fill_rect(Rect::new(cx - arm, cy - t, cx + arm, cy + t), glyph);
    s.fill_rect(Rect::new(cx - t, cy - arm, cx + t, cy + arm), glyph);
}

/// The Material Design `MdiFilter` funnel path (`viewBox 0 0 24 24`), the exact
/// `d` of `icondata::MdiFilter` used by the DOM chrome (`chrome/commands.rs`).
/// Embedded as a const because `icondata` is a wasm-only dependency, whereas the
/// renderer also compiles on the host for tests. This keeps the canvas glyph and
/// the DOM icon a single, identical silhouette.
const MDI_FILTER_PATH: &str = "M14,12V19.88C14.04,20.18 13.94,20.5 13.71,20.71C13.32,21.1 12.69,21.1 12.3,20.71L10.29,18.7C10.06,18.47 9.96,18.16 10,17.87V12H9.97L4.21,4.62C3.87,4.19 3.95,3.56 4.38,3.22C4.57,3.08 4.78,3 5,3V3H19V3C19.22,3 19.43,3.08 19.62,3.22C20.05,3.56 20.13,4.19 19.79,4.62L14.03,12H14Z";

/// The `MdiFilter` outline, flattened to polylines and normalized into the unit box
/// `[0,1]^2` (aspect-preserving, centred), computed once. Each inner `Vec` is one closed
/// contour as points of the unit box. The GPU surface has no `fill_path`, so the
/// glyph is reproduced by scanline-filling these contours (`trame-shared-filter`).
pub(crate) fn mdi_filter_polygon() -> &'static Vec<Vec<kurbo::Point>> {
    static POLY: std::sync::OnceLock<Vec<Vec<kurbo::Point>>> = std::sync::OnceLock::new();
    POLY.get_or_init(|| {
        // The path is a constant a host test parses; a failure paints no glyph.
        let Ok(path) = kurbo::BezPath::from_svg(MDI_FILTER_PATH) else {
            return Vec::new();
        };
        // Flatten curves to line segments, grouping into contours at each MoveTo.
        let mut contours: Vec<Vec<kurbo::Point>> = Vec::new();
        kurbo::flatten(path.iter(), 0.05, |el| match el {
            kurbo::PathEl::MoveTo(p) => contours.push(vec![p]),
            kurbo::PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push(p);
                }
            }
            // The scanline fill closes each contour implicitly; nothing to do.
            _ => {}
        });
        // Normalize by the path bounding box, scaling both axes by the larger
        // dimension so the funnel keeps its proportions, then centre in [0,1].
        let bb = kurbo::Shape::bounding_box(&path);
        let scale = bb.width().max(bb.height()).max(f64::EPSILON);
        let (cx, cy) = (bb.center().x, bb.center().y);
        for contour in &mut contours {
            for p in contour.iter_mut() {
                p.x = 0.5 + (p.x - cx) / scale;
                p.y = 0.5 + (p.y - cy) / scale;
            }
        }
        contours
    })
}

/// Scanline-fill the unit-box `contours` into the box `bounds` with solid
/// `color`, using 1px-tall horizontal `fill_rect` spans (even-odd rule). The GPU
/// surface only fills axis-aligned quads, so an arbitrary silhouette is painted
/// row by row. `MdiFilter` is a single hole-less contour, so even-odd == nonzero.
pub(crate) fn fill_unit_polygon<S: DrawSurface>(
    s: &mut S,
    contours: &[Vec<kurbo::Point>],
    bounds: Rect,
    color: Color,
) {
    let (bx, by, w, h) = (bounds.x0, bounds.y0, bounds.width(), bounds.height());
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let mut xs: Vec<f64> = Vec::new();
    let mut y = by + 0.5;
    while y < by + h {
        let uy = (y - by) / h; // scanline in unit space
        xs.clear();
        for contour in contours {
            // Each point pairs with the next; the cycle wraps = implicit close.
            let next = contour.iter().cycle().skip(1);
            for (a, b) in contour.iter().zip(next) {
                let (x0, y0, x1, y1) = (a.x, a.y, b.x, b.y);
                // Half-open crossing test avoids double-counting shared vertices.
                if (y0 <= uy && y1 > uy) || (y1 <= uy && y0 > uy) {
                    let t = (uy - y0) / (y1 - y0);
                    xs.push(x0 + t * (x1 - x0));
                }
            }
        }
        ambre_core::order::sort_partial(&mut xs);
        for [a, b] in xs.as_chunks::<2>().0 {
            let xa = bx + a.clamp(0.0, 1.0) * w;
            let xb = bx + b.clamp(0.0, 1.0) * w;
            if xb > xa {
                s.fill_rect(Rect::new(xa, y - 0.5, xb, y + 0.5), color);
            }
        }
        y += 1.0;
    }
}

/// Paint the `MdiFilter` funnel glyph (filled) inside the box `(bx, by, w, h)`:
/// the "filter active" cue (`trame-shared-filter`). A faithful reproduction of the DOM
/// `icondata::MdiFilter`, fitted aspect-preserving into a centred square so the
/// proportions hold regardless of the box aspect.
pub(crate) fn draw_funnel<S: DrawSurface>(s: &mut S, bounds: Rect, c: Color) {
    let (bx, by, w, h) = (bounds.x0, bounds.y0, bounds.width(), bounds.height());
    let side = w.min(h);
    let ox = bx + (w - side) / 2.0;
    let oy = by + (h - side) / 2.0;
    let square = Rect::new(ox, oy, ox + side, oy + side);
    fill_unit_polygon(s, mdi_filter_polygon(), square, c);
}

/// Stroke a downward chevron inside the box `bounds`: the "filter available,
/// none active" cue on an autofilter button (`trame-shared-filter`).
pub(crate) fn draw_chevron<S: DrawSurface>(s: &mut S, bounds: Rect, c: Color) {
    let (bx, by, w, h) = (bounds.x0, bounds.y0, bounds.width(), bounds.height());
    let cx = bx + w / 2.0;
    s.stroke_line(
        ambre_draw::Line::new((bx, by), (cx, by + h)),
        c,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );
    s.stroke_line(
        ambre_draw::Line::new((bx + w, by), (cx, by + h)),
        c,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );
}

/// Stroke the horizontal `line` dashed, left to right, the dashes offset left
/// by `phase` (the marching-ants crawl, `trame-clipboard`).
pub(crate) fn dashed_h<S: DrawSurface>(
    s: &mut S,
    line: ambre_draw::Line,
    c: Color,
    phase: Px<Surface>,
) {
    let (x0, x1, y) = (line.p0.x, line.p1.x, line.p0.y);
    let mut t = x0 - phase.get();
    while t < x1 {
        let a = t.max(x0);
        let b = (t + MARCHING_DASH.get()).min(x1);
        if b > a {
            s.stroke_line(
                ambre_draw::Line::new((a, y), (b, y)),
                c,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
        }
        t += MARCHING_PERIOD.get();
    }
}

/// Stroke the vertical `line` dashed, top to bottom (see [`dashed_h`]).
pub(crate) fn dashed_v<S: DrawSurface>(
    s: &mut S,
    line: ambre_draw::Line,
    c: Color,
    phase: Px<Surface>,
) {
    let (y0, y1, x) = (line.p0.y, line.p1.y, line.p0.x);
    let mut t = y0 - phase.get();
    while t < y1 {
        let a = t.max(y0);
        let b = (t + MARCHING_DASH.get()).min(y1);
        if b > a {
            s.stroke_line(
                ambre_draw::Line::new((x, a), (x, b)),
                c,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
        }
        t += MARCHING_PERIOD.get();
    }
}

/// Draw the overlay layer (headers + active border + header labels + HUD).
/// This is re-rendered every dirty frame onto the swapchain after the cells
/// layer has been blitted from its persistent offscreen.
pub fn draw_overlay<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    pen: TextPen<'_>,
    clock: &dyn Clock,
    stats: &mut DrawStats,
) {
    let TextPen {
        font_cx,
        cache: text_cache,
        pass: text_pass,
    } = pen;
    let t0 = clock.now();

    let layout = screen_layout(state);

    // -- 1. Fixed header strips (bg + active tint + separator + labels), via
    //       the shared primitives. The primitives clip the active tint to the
    //       visible range, so passing the raw active index is safe. -----------
    let hover_col = match state.hover_header {
        Some(grid::HeaderHit::ColBody(c)) => Some(c),
        _ => None,
    };
    let hover_row = match state.hover_header {
        Some(grid::HeaderHit::RowBody(r)) => Some(r),
        _ => None,
    };
    let hover_corner =
        render::CornerHover::from_bool(matches!(state.hover_header, Some(grid::HeaderHit::Corner)));
    render::draw_col_headers_on(
        surface,
        &layout,
        palette,
        Some(state.active_col()),
        hover_col,
        TextPen {
            font_cx,
            cache: text_cache,
            pass: text_pass,
        },
    );
    render::draw_row_headers_on(
        surface,
        &layout,
        palette,
        Some(state.active_row()),
        hover_row,
        hover_corner,
        TextPen {
            font_cx,
            cache: text_cache,
            pass: text_pass,
        },
    );

    // -- 1b. Funnel badge on the filtered column's letter header (`trame-shared-filter`), kept
    //        as a secondary cue alongside the first-row autofilter buttons below.
    //        Drawn only when the column is within the visible span.
    if let Some(fcol) = state.filtered_col
        && fcol >= layout.origin_col
        && fcol <= layout.origin_col + state.viewport_cols
    {
        let cell = layout.cell_rect(CellPos::new(layout.origin_row, fcol));
        // Anchor the badge to the header strip, right-aligned in the cell. The
        // glyph scales with the grid zoom so it tracks the zoomed header letters.
        let z = state.zoom.get();
        let w = 8.0 * z;
        let h = 7.0 * z;
        let bx = (cell.x1 - w - 3.0 * z).max(cell.x0 + 2.0);
        let by = layout.top.get() + (layout.header_height.get() - h) / 2.0;
        draw_funnel(
            surface,
            Rect::new(bx, by, bx + w, by + h),
            palette.selection,
        );
    }

    // -- 1b'. Excel-style autofilter buttons on the current region's header row
    //         (`trame-shared-filter`): each column of the contiguous data block around the
    //         active cell draws a small button at the right edge of its first-row
    //         cell -- a plain chevron when the column is unfiltered, a funnel
    //         when the shared filter targets it. Clicking opens the same
    //         FilterPopover as the context-menu entry (one filter code path).
    if let Some(region) = state.current_region() {
        let band_row = region.top_left.row;
        for col in region.cols() {
            let Some(r) = state.filter_button_rect(CellPos::new(band_row, col)) else {
                continue;
            };
            let (bx, by, bw, bh) = (r.x0, r.y0, r.width(), r.height());
            surface.fill_rect(r, palette.header_bg);
            surface.stroke_rect(
                r,
                palette.grid_line,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            // Glyph inset so it reads inside the button, scaled with the zoom
            // (the button itself is zoom-scaled) so the inset stays proportional.
            let pad = 3.0 * state.zoom.get();
            let gx = bx + pad;
            let gy = by + pad;
            let gw = (bw - 2.0 * pad).max(0.0);
            let gh = (bh - 2.0 * pad).max(0.0);
            if state.filtered_col == Some(col) {
                draw_funnel(
                    surface,
                    Rect::new(gx, gy, gx + gw, gy + gh),
                    palette.selection,
                );
            } else {
                let chevron = Rect::new(gx, gy, gx + gw, gy + gh * 0.7);
                draw_chevron(surface, chevron, palette.header_text);
            }
        }
    }

    // -- 1c. Find matches (`trame-find-replace`): a translucent highlight behind each matched
    //        cell so every hit is visible at a glance; the focused match is the
    //        active cell, so it also carries the active border drawn below.
    if !state.find_matches.is_empty() {
        for &at in &state.find_matches {
            if let Some(cell) = state.cell_pixel_rect(at) {
                surface.fill_rect(cell, palette.selection.with_alpha(0.28));
            }
        }
    }

    // -- 1d. Whole-axis selection band (`trame-selection`). A virtual column/row
    //        selection has no on-screen rectangle bounding it, so fill the whole
    //        band over the grid area and tint its headers opaquely.
    let sel = &state.selection;
    if sel.kind != grid::SelKind::Cells {
        let (band_x0, band_x1, band_y0, band_y1, hx0, hx1, hy0, hy1) = match sel.kind {
            grid::SelKind::Cols => {
                let (c1, c2) = (
                    sel.anchor.col.min(sel.active.col),
                    sel.anchor.col.max(sel.active.col),
                );
                let x0 = state.col_x(c1).max(state.header_w());
                let x1 = (state.col_x(c2) + state.col_width_of(c2)).min(state.canvas_width);
                // Band over the grid body, and the header tint over the strip.
                (
                    x0.get(),
                    x1.get(),
                    state.header_h().get(),
                    state.grid_bottom().get(),
                    x0.get(),
                    x1.get(),
                    0.0,
                    state.header_h().get(),
                )
            }
            _ => {
                let (r1, r2) = (
                    sel.anchor.row.min(sel.active.row),
                    sel.anchor.row.max(sel.active.row),
                );
                let y0 = state.row_y(r1).max(state.header_h());
                let y1 = (state.row_y(r2) + state.row_height_of(r2)).min(state.grid_bottom());
                (
                    state.header_w().get(),
                    state.canvas_width.get(),
                    y0.get(),
                    y1.get(),
                    0.0,
                    state.header_w().get(),
                    y0.get(),
                    y1.get(),
                )
            }
        };
        if band_x1 > band_x0 && band_y1 > band_y0 {
            surface.fill_rect(
                Rect::new(band_x0, band_y0, band_x1, band_y1),
                palette.selection.with_alpha(0.16),
            );
        }
        if hx1 > hx0 && hy1 > hy0 {
            surface.fill_rect(Rect::new(hx0, hy0, hx1, hy1), palette.header_active);
        }
    }

    // -- 2. Selection (range fills + outlines) and active-cell border. --------
    // Lives in the overlay so it tracks the selection on every frame regardless
    // of scroll shifts in the persistent layer. A lone single cell shows only
    // the active border (no fill); any multi-cell or disjoint selection fills
    // each rectangle with a translucent tint and outlines it. Skipped for a
    // whole-axis band (drawn above): its `bounds()` collapses to row/col 0.
    if sel.kind == grid::SelKind::Cells && sel.is_multi() {
        for rect in state.selection_rects() {
            if let Some(rect) = state.range_pixel_rect(rect) {
                surface.fill_rect(rect, palette.selection.with_alpha(0.16));
                surface.stroke_rect(
                    rect,
                    palette.selection,
                    ambre_core::Px::<ambre_core::Surface>::new(1.0),
                );
            }
        }
    }
    // The active-cell 2px border is drawn only for a lone single-cell selection
    // (Excel behaviour). In a multi-cell / disjoint selection the range fill +
    // outline already mark the selection as one block. A separate border on the
    // active cell would then read as a spurious second selection following the
    // cursor. When the active cell sits in a merged block the border wraps the
    // whole block, so the merge reads as one selected cell.
    if sel.kind == grid::SelKind::Cells
        && !sel.is_multi()
        && let Some(cell) = state
            .active_merge_rect()
            .or_else(|| state.cell_pixel_rect(state.active_cell()))
    {
        surface.stroke_rect(
            cell,
            palette.selection,
            ambre_core::Px::<ambre_core::Surface>::new(2.0),
        );
    }

    // -- 2b. Clipboard marching-ants marquee (`trame-clipboard`). A dashed rectangle around
    //        the copied/cut range, its dashes crawling so the marquee animates.
    if let Some(rect) = state.clipboard_anchor
        && let Some(range) = state.range_pixel_rect(rect)
    {
        let (x, y, w, ht) = (range.x0, range.y0, range.width(), range.height());
        let phase = Px::<Surface>::new((clock.now().get() / 40.0) % MARCHING_PERIOD.get());
        let c = palette.selection;
        dashed_h(surface, ambre_draw::Line::new((x, y), (x + w, y)), c, phase);
        dashed_h(
            surface,
            ambre_draw::Line::new((x, y + ht), (x + w, y + ht)),
            c,
            phase,
        );
        dashed_v(
            surface,
            ambre_draw::Line::new((x, y), (x, y + ht)),
            c,
            phase,
        );
        dashed_v(
            surface,
            ambre_draw::Line::new((x + w, y), (x + w, y + ht)),
            c,
            phase,
        );
    }

    // -- 2c. Autofill (`trame-autofill`): a dashed preview outline of the destination
    //        rectangle while the handle is dragged, then the small blue handle
    //        square at the selection's bottom-right corner.
    if let Some(rect) = state.fill_preview
        && let Some(range) = state.range_pixel_rect(rect)
    {
        let (x, y, w, ht) = (range.x0, range.y0, range.width(), range.height());
        let c = palette.selection;
        let still = Px::<Surface>::ZERO;
        dashed_h(surface, ambre_draw::Line::new((x, y), (x + w, y)), c, still);
        dashed_h(
            surface,
            ambre_draw::Line::new((x, y + ht), (x + w, y + ht)),
            c,
            still,
        );
        dashed_v(
            surface,
            ambre_draw::Line::new((x, y), (x, y + ht)),
            c,
            still,
        );
        dashed_v(
            surface,
            ambre_draw::Line::new((x + w, y), (x + w, y + ht)),
            c,
            still,
        );
    }
    if let Some(r) = state.fill_handle_rect() {
        surface.fill_rect(r, palette.selection);
        surface.stroke_rect(
            r,
            palette.content_bg,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
    }

    // -- 2d. Header resize guide (`trame-resize`): a full-height/width line at the
    //        prospective boundary while a header border is dragged.
    match state.resize_preview {
        Some(grid::ResizePreview::Col(x)) => {
            let (x, bottom) = (x.get(), state.grid_bottom().get());
            surface.stroke_line(
                ambre_draw::Line::new((x, layout.top.get()), (x, bottom)),
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
        }
        Some(grid::ResizePreview::Row(y)) => {
            let (y, right) = (y.get(), state.canvas_width.get());
            surface.stroke_line(
                ambre_draw::Line::new((0.0, y), (right, y)),
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
        }
        None => {}
    }

    // -- 2d bis. Header reorder guide: a bold full-height/width line at the
    //            prospective insertion boundary while a selected column/row is
    //            dragged to a new position (Excel-style reorder preview).
    match state.reorder_preview {
        Some(grid::AxisPos::Col(i)) => {
            let (x, bottom) = (state.col_x(i).get(), state.grid_bottom().get());
            surface.stroke_line(
                ambre_draw::Line::new((x, layout.top.get()), (x, bottom)),
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(2.5),
            );
        }
        Some(grid::AxisPos::Row(i)) => {
            let (y, right) = (state.row_y(i).get(), state.canvas_width.get());
            surface.stroke_line(
                ambre_draw::Line::new((0.0, y), (right, y)),
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(2.5),
            );
        }
        None => {}
    }

    // -- 2e. Inline insert "+" badge on the hovered header boundary (`trame-headers`).
    match state.insert_hint {
        // ColInsert(i) inserts left of column i, that is, at column i's left edge.
        Some(grid::HeaderHit::ColInsert(i)) => {
            let bx = state.col_x(i).get();
            // At the top of the column separator: the pill hugs the header's top
            // edge rather than floating in its centre.
            let cy = layout.top.get() + BADGE_RADIUS.get() + 1.0;
            let center = kurbo::Point::new(bx, cy);
            draw_insert_badge(surface, center, palette.selection, palette.content_bg);
        }
        Some(grid::HeaderHit::RowInsert(i)) => {
            let by = state.row_y(i).get();
            // At the left of the row separator: the pill hugs the row header's
            // left edge.
            let cx = BADGE_RADIUS.get() + 1.0;
            let center = kurbo::Point::new(cx, by);
            draw_insert_badge(surface, center, palette.selection, palette.content_bg);
        }
        _ => {}
    }
    let t_after_hdrs = clock.now();

    // The perf HUD no longer paints here. Fps and frame timings are published
    // to the DOM status bar (`ambre_canvas::PerfPublisher`) and rendered as
    // plain text. We never shape debug glyphs on the GPU. Only the timing the
    // view feeds the snapshot is recorded.
    stats.headers = t_after_hdrs - t0;
}
