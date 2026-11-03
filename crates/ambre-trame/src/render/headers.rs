// ambre-trame/src/render/headers.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;
use ambre_core::index::{Len, Pos};

// -----------------------------------------------------------------------------
// draw_col_headers_on / draw_row_headers_on -- fixed header strips
// -----------------------------------------------------------------------------

/// Draw the top column-header strip. The order is the opaque background, then
/// the hover tint (when `hover_col` is visible and not the active column). Then
/// the active-column tint (when `active_col` is visible), the bottom separator,
/// and the A/B/C labels.
///
/// `active_col == None` (PDF) skips the highlight; `hover_col == None` skips the
/// hover tint.
#[allow(clippy::too_many_arguments)]
pub fn draw_col_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    active_col: Option<Pos<Col>>,
    hover_col: Option<Pos<Col>>,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_pass: TextPass,
) {
    // The surface draws in bare f64: the lengths unwrap once, here.
    let w = layout.width.get();
    let hh = layout.header_height.get();
    let top = layout.top.get();

    // Opaque background over the full top strip (corner included).
    surface.fill_rect(Rect::new(0.0, top, w, top + hh), palette.header_bg);

    // Hover tint, under the active tint so the active column always wins when a
    // hover lands on it. Skipped when the hovered column is the active one.
    if let Some(col) = hover_col
        && Some(col) != active_col
        && col >= layout.origin_col
    {
        let x = layout.col_left(col).get();
        if x < w {
            surface.fill_rect(
                Rect::new(x, top, x + layout.col_w(col).get(), top + hh),
                palette.header_hover,
            );
        }
    }

    // Active-column tint, over the bg and under the labels.
    if let Some(col) = active_col
        && col >= layout.origin_col
    {
        let x = layout.col_left(col).get();
        if x < w {
            surface.fill_rect(
                Rect::new(x, top, x + layout.col_w(col).get(), top + hh),
                palette.header_active,
            );
        }
    }

    // Bottom separator.
    surface.stroke_line(
        ambre_draw::Line::new((0.0, top + hh), (w, top + hh)),
        palette.header_border,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );

    // Per-column separators: a vertical hairline at each column's right edge, so
    // the letter cells read as discrete buttons (`trame-headers`). Walks the same
    // visible columns as the label loop below, independent of `text_pass`.
    {
        let mut col = layout.origin_col;
        let mut col_x = layout.header_width.get();
        while col < layout.total_cols.as_end() && col_x < w {
            let edge = col_x + layout.col_w(col).get();
            surface.stroke_line(
                ambre_draw::Line::new((edge, top), (edge, top + hh)),
                palette.header_border,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            col_x = edge;
            col = col.next();
        }
    }

    if text_pass == TextPass::Shapes {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let mut col = layout.origin_col;
    let mut col_x = layout.header_width.get();
    loop {
        if col >= layout.total_cols.as_end() || col_x >= w {
            break;
        }
        let cw = layout.col_w(col).get();
        let label = col_label(col);
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx,
            &label,
            HEADER_SIZE.zoomed(layout.zoom),
            Px::<View>::new(cw as f32),
            Alignment::Center,
            ambre_canvas::Weight::Regular,
            ambre_canvas::Slant::Upright,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((hh - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > hh;
        if needs_clip {
            surface.push_clip(Rect::new(col_x, top, col_x + cw, top + hh));
        }
        draw_layout_runs(surface, lyt, Point::new(col_x, top + y_off), &brush);
        if needs_clip {
            surface.pop_clip();
        }
        col_x += cw;
        col = col.next();
    }
}

ambre_core::define_flag! {
    /// Whether the pointer rests on the top-left select-all corner.
    pub enum CornerHover {
        /// The pointer is elsewhere: the corner draws plain.
        Plain,
        /// The pointer rests on the corner: it draws the hover tint.
        Hovered,
    }
}

/// Draw the left row-header strip. The order is the opaque background, then the
/// hover tint (when `hover_row` is visible and not the active row). Then the
/// active-row tint (when `active_row` is visible), the right separator, and the
/// 1/2/3 numbers.
///
/// `active_row == None` (PDF) skips the highlight; `hover_row == None` skips the
/// row hover tint; `hover_corner` tints the top-left select-all corner.
#[allow(clippy::too_many_arguments)]
pub fn draw_row_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    active_row: Option<Pos<Row>>,
    hover_row: Option<Pos<Row>>,
    hover_corner: CornerHover,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_pass: TextPass,
) {
    // The surface draws in bare f64: the lengths unwrap once, here.
    let h = layout.height.get();
    let hw = layout.header_width.get();
    let top = layout.top.get();
    let cells_top = layout.cells_top().get();

    // Opaque background over the full left strip (below the formula bar band).
    surface.fill_rect(Rect::new(0.0, top, hw, h), palette.header_bg);

    // Hover tint on the top-left corner (select-all affordance). The corner is
    // the last bg drawn over this region, so the tint sits cleanly on top.
    if hover_corner == CornerHover::Hovered {
        surface.fill_rect(Rect::new(0.0, top, hw, cells_top), palette.header_hover);
    }

    // Hover tint, under the active tint so the active row always wins when a
    // hover lands on it. Skipped when the hovered row is the active one or hidden.
    if let Some(row) = hover_row
        && Some(row) != active_row
        && row >= layout.origin_row
        && !layout.is_hidden(row)
    {
        let y = layout.row_top(row).get();
        if y < h {
            surface.fill_rect(
                Rect::new(0.0, y, hw, y + layout.row_h(row).get()),
                palette.header_hover,
            );
        }
    }

    // Active-row tint, over the bg and under the numbers. Positioned in collapsed
    // (filtered) row space; skipped when the active row is itself hidden.
    if let Some(row) = active_row
        && row >= layout.origin_row
        && !layout.is_hidden(row)
    {
        let y = layout.row_top(row).get();
        if y < h {
            surface.fill_rect(
                Rect::new(0.0, y, hw, y + layout.row_h(row).get()),
                palette.header_active,
            );
        }
    }

    // Right separator.
    surface.stroke_line(
        ambre_draw::Line::new((hw, top), (hw, h)),
        palette.header_border,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );

    // Per-row separators: a horizontal hairline at each visible row's bottom edge,
    // so the numbers read as discrete buttons (`trame-headers`). Walks the collapsed
    // (filtered) row space like the number loop, independent of `text_pass`.
    {
        let mut slot = Len::<Row>::ZERO;
        let mut row_y = cells_top;
        while let Some(row) = layout.row_at_slot(slot) {
            if row_y >= h {
                break;
            }
            let edge = row_y + layout.row_h(row).get();
            surface.stroke_line(
                ambre_draw::Line::new((0.0, edge), (hw, edge)),
                palette.header_border,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            row_y = edge;
            slot += Len::<Row>::ONE;
        }
    }

    if text_pass == TextPass::Shapes {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let max_width = (hw - 8.0) as f32;
    // Clip every row label to the strip. The last visible row is usually
    // partial. Its vertically centred number would otherwise spill below the
    // grid bottom and bleed through onto the sheet-tab strip
    // (`trame-sheets`).
    surface.push_clip(Rect::new(0.0, cells_top, hw, h));
    // Walk the visible rows, accumulating their variable heights. `slot` only
    // tracks how many visible rows we have drawn (for `row_at_slot`); `row_y` is
    // the running pixel top.
    let mut slot = Len::<Row>::ZERO;
    let mut row_y = cells_top;
    while let Some(row) = layout.row_at_slot(slot) {
        if row_y >= h {
            break;
        }
        let rh = layout.row_h(row).get();
        let label = row_label(row);
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx,
            &label,
            HEADER_SIZE.zoomed(layout.zoom),
            Px::<View>::new(max_width),
            Alignment::End,
            ambre_canvas::Weight::Regular,
            ambre_canvas::Slant::Upright,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((rh - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > rh;
        if needs_clip {
            surface.push_clip(Rect::new(0.0, row_y, hw, row_y + rh));
        }
        draw_layout_runs(surface, lyt, Point::new(4.0, row_y + y_off), &brush);
        if needs_clip {
            surface.pop_clip();
        }
        row_y += rh;
        slot += Len::<Row>::ONE;
    }
    surface.pop_clip();
}
