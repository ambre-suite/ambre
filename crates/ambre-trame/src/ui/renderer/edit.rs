// ambre-trame/src/ui/renderer/edit.rs
// Inline cell editor rendering: delegates to the shared `ambre_canvas::TextEditor`
// core (parley PlainEditor). The core lays out the buffer and resolves any queued
// pointer hits against that layout. It paints the selection band, the glyphs (with
// the live IME preedit) and the caret. This module only supplies the cell geometry
// + pinned style and paints the edit field background under it.

use super::*;

use ambre_canvas::TextEditorStyle;

/// Draw the active edit session over the cell: a content-coloured field, then the
/// shared editor overlay (selection + text + caret). Called on the overlay layer
/// after `draw_overlay`, so it sits above the active-cell border and the grid.
/// Clipped to the cell, itself clipped to the scrollable area (never under the
/// header strip).
///
/// Answers where the caret was laid out, in canvas pixels, so the host can
/// place the input pipe on it (`CHROME-007`).
pub fn draw_edit<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    editor: &mut CellEditor,
    caret_visible: bool,
    font_cx: &mut FontContext,
) -> Option<Rect> {
    // An active cell scrolled out of view: nothing to draw.
    let (x, y, cw, ch) = state.cell_pixel_rect(editor.at)?;

    // Match the cell's pinned formatting so the edit looks like the result.
    let (bold, italic) = state
        .cell_cache
        .get(&editor.at)
        .map(|c| (c.bold, c.italic))
        .unwrap_or((false, false));
    editor.core.set_style(TextEditorStyle {
        weight: ambre_core::FontWeight::of_bold(bold),
        italic,
        family: "Sans".to_owned(),
        base_size: ambre_core::DocPx::new(BODY_SIZE),
        color: palette.cell_text,
        underline: false,
        strikethrough: false,
    });

    // Apply queued intents against this frame's layout. The text wraps to the
    // cell's content width and Alt+Enter inserts explicit newlines, so the edit
    // is multi-line (Excel/Sheets in-cell editor).
    let pad = CELL_PAD_X.zoomed_on_surface(state.zoom).get();
    let wrap_w = (cw - 2.0 * pad).max(1.0) as f32;
    editor.core.flush(font_cx, wrap_w, state.zoom);

    // The field grows downward to fit every line, overflowing the cell like a
    // spreadsheet, and is never shorter than the row. A layout that fits the row
    // stays vertically centred, matching the static cell. A taller one top-aligns
    // with a small pad, so line 0 reads at the cell top.
    let text_h = editor.core.layout_height().unwrap_or(0.0) as f64;
    let (y_off, box_h) = if text_h + 2.0 * pad <= ch {
        (((ch - text_h) / 2.0).max(0.0), ch)
    } else {
        (pad, text_h + 2.0 * pad)
    };
    let origin = ambre_draw::Point::new(x + pad, y + y_off);

    // The field may extend below the cell; clip to it (intersected with the live
    // clip) so the overflowing lines show. Downward growth only ever covers the
    // cells below, never the header strip above.
    let field = Rect::new(x, y, x + cw, y + box_h);
    surface.push_clip(field);
    // Content-coloured field over the underlying cell + active border interior.
    surface.fill_rect(field, palette.content_bg);
    // Selection band + glyphs (incl. IME preedit) + blinking caret.
    editor.core.render(
        surface,
        origin,
        palette.selection,
        ambre_canvas::Caret::shown_if(caret_visible),
    );
    surface.pop_clip();
    let caret = editor.core.caret_local()?;
    Some(Rect::new(
        origin.x + caret.x0,
        origin.y + caret.y0,
        origin.x + caret.x1,
        origin.y + caret.y1,
    ))
}
