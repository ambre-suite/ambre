// ambre-trame/src/ui/renderer/edit.rs
// Inline cell editor rendering: delegates to the shared `ambre_canvas::TextEditor`
// core (parley PlainEditor). The core lays out the buffer, resolves any queued
// pointer hits against that layout, and paints the selection band, glyphs (with
// the live IME preedit) and caret. This module only supplies the cell geometry +
// pinned style and paints the edit field background under it.

use super::*;

use ambre_canvas::TextEditorStyle;

/// Draw the active edit session over the cell: a content-coloured field, then the
/// shared editor overlay (selection + text + caret). Called on the overlay layer
/// after `draw_overlay`, so it sits above the active-cell border and the grid.
/// Clipped to the cell, itself clipped to the scrollable area (never under the
/// header strip).
pub fn draw_edit<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    editor: &mut CellEditor,
    caret_visible: bool,
    font_cx: &mut FontContext,
) {
    let Some((x, y, cw, ch)) = state.cell_pixel_rect(editor.row, editor.col) else {
        return; // active cell scrolled out of view; nothing to draw
    };

    // Match the cell's pinned formatting so the edit looks like the result.
    let (bold, italic) = state
        .cell_cache
        .get(&(editor.row, editor.col))
        .map(|c| (c.bold, c.italic))
        .unwrap_or((false, false));
    editor.core.set_style(TextEditorStyle {
        weight: if bold { 700 } else { 400 },
        italic,
        family: "Sans".to_owned(),
        base_size: BODY_SIZE,
        color: palette.cell_text,
    });

    // Apply queued intents against this frame's layout. Single-line: a very wide
    // wrap so the text overflows the cell (spreadsheet behaviour) instead of
    // wrapping. Multi-line wrapping lands in a later step.
    let pad = CELL_PAD_X * state.zoom;
    editor.core.flush(font_cx, 1.0e6, state.zoom);

    // Vertically centre the (single) line in the row, like the static cell text.
    let text_h = editor.core.layout_height().unwrap_or(0.0) as f64;
    let y_off = ((ch - text_h) / 2.0).max(0.0);
    let origin = ambre_draw::Point::new(x + pad, y + y_off);

    let cell = Rect::new(x, y, x + cw, y + ch);
    surface.push_clip(cell);
    // Content-coloured field over the underlying cell + active border interior.
    surface.fill_rect(cell, palette.content_bg);
    // Selection band + glyphs (incl. IME preedit) + blinking caret.
    editor
        .core
        .render(surface, origin, palette.selection, caret_visible);
    surface.pop_clip();
}
