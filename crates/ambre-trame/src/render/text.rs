// ambre-trame/src/render/text.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;

// -----------------------------------------------------------------------------
// Text helper
// -----------------------------------------------------------------------------

/// Shape `text` and emit its glyph runs at `(origin_x, origin_y)` (the layout's
/// top-left). Shared by the headers, the HUD, and any one-shot label.
#[allow(clippy::too_many_arguments)]
pub fn draw_text<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    font_size: f32,
    max_width: f32,
    align: Alignment,
    origin_x: f64,
    origin_y: f64,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let layout: &Layout<ParleyBrush> =
        cache.get_or_build(font_cx, text, font_size, max_width, align, false, false);
    let brush = Brush::Solid(color);
    draw_layout_runs(surface, layout, origin_x, origin_y, &brush);
}

/// Emit every glyph run of an already-built `layout` at `(origin_x, origin_y)`
/// (the layout's top-left) in a single `brush`. This is the one place the
/// `lines() -> items() -> GlyphRun` walk lives; the headers, the inline editor
/// and any decoration-free label all delegate here instead of re-inlining the
/// same nested loop.
pub fn draw_layout_runs<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    origin_x: f64,
    origin_y: f64,
    brush: &Brush,
) {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, brush, Affine::translate((tx, ty)));
        }
    }
}

/// Like [`draw_layout_runs`] but also paints per-run underline / strikethrough
/// decorations in `deco_color`. Shared by the plain-cell and merged-cell text
/// paths, which both draw a 1px underline just below the baseline and a 1px
/// strikethrough at ~0.3em above it.
#[allow(clippy::too_many_arguments)]
pub fn draw_layout_runs_decorated<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    origin_x: f64,
    origin_y: f64,
    brush: &Brush,
    deco_color: Color,
    font_size: f32,
    underline: bool,
    strikethrough: bool,
) {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, brush, Affine::translate((tx, ty)));
            if !underline && !strikethrough {
                continue;
            }
            let adv = run.advance() as f64;
            // Underline run: a 1px line just below the glyph baseline.
            if underline {
                surface.stroke_line(tx, ty + 2.0, tx + adv, ty + 2.0, deco_color, 1.0);
            }
            // Strikethrough run: a 1px line through the x-height.
            if strikethrough {
                let sy = ty - (font_size as f64) * 0.3;
                surface.stroke_line(tx, sy, tx + adv, sy, deco_color, 1.0);
            }
        }
    }
}

/// Spreadsheet column label for a 0-based column index (0 -> "A", 26 -> "AA").
pub fn col_label(col: u32) -> String {
    crate::columns::col_to_alpha(col)
}
