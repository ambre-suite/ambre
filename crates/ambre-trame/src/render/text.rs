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
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

/// Spreadsheet column label for a 0-based column index (0 -> "A", 26 -> "AA").
pub fn col_label(col: u32) -> String {
    let mut label = String::new();
    let mut n = col + 1;
    while n > 0 {
        n -= 1;
        label.push(char::from_u32(b'A' as u32 + n % 26).unwrap());
        n /= 26;
    }
    label.chars().rev().collect()
}
