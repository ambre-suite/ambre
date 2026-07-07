// ambre-trame/src/ui/renderer/edit.rs
// Inline cell editor rendering (caret, selection highlight, preview text).

use super::*;

/// Width in pixels of `text` shaped at the body size, or 0 for empty text.
pub(crate) fn measure(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    bold: bool,
    italic: bool,
    zoom: f64,
) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let layout: &Layout<ParleyBrush> = cache.get_or_build(
        font_cx,
        text,
        BODY_SIZE * zoom as f32,
        f32::MAX,
        Alignment::Start,
        bold,
        italic,
    );
    layout.width() as f64
}

/// Map a text-relative pixel x to the nearest char boundary in `text`. Walks
/// boundaries left to right and returns the one whose mid-glyph crossing the
/// target is closest -- i.e. the caret snaps to whichever gap the click is
/// nearer to. `text` is short (one cell), so the linear measure scan is cheap.
pub(crate) fn offset_at_x(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    bold: bool,
    italic: bool,
    target: f64,
    zoom: f64,
) -> usize {
    let mut prev_w = 0.0;
    let mut prev_off = 0;
    for (off, c) in text.char_indices() {
        let next_off = off + c.len_utf8();
        let w = measure(font_cx, cache, &text[..next_off], bold, italic, zoom);
        if target <= (prev_w + w) / 2.0 {
            return prev_off;
        }
        prev_w = w;
        prev_off = next_off;
    }
    text.len()
}

/// Draw the active edit session over the cell: a white field, the buffer with
/// its IME composition preview, the caret, and a dotted underline under the
/// composition. Called on the overlay layer after `draw_overlay`, so it sits
/// above the active-cell border and the grid. Clipped to the cell, which is
/// itself clipped to the scrollable area (never under the header strip).
pub fn draw_edit<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    editor: &mut CellEditor,
    caret_visible: bool,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
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

    // Resolve a deferred pointer hit now that we have font metrics: map its
    // physical x to a byte offset and apply the click/drag/word-select intent.
    let pad = CELL_PAD_X * state.zoom;
    if let Some(hit) = editor.pending.take() {
        let text_x = (hit.x - (x + pad)).max(0.0);
        let off = offset_at_x(
            font_cx,
            cache,
            &editor.buffer,
            bold,
            italic,
            text_x,
            state.zoom,
        );
        match hit.kind {
            HitKind::Place => editor.place_caret(off),
            HitKind::Extend => editor.extend_to(off),
            HitKind::Word => editor.select_word(off),
        }
    }

    let cell = Rect::new(x, y, x + cw, y + ch);
    surface.push_clip(cell);

    // Content-coloured field over the underlying cell + active border interior.
    surface.fill_rect(cell, palette.content_bg);

    // Selection highlight behind the text: a translucent band from the start to
    // the end of the selected range.
    if let Some((s, e)) = editor.selection() {
        let sx = x
            + pad
            + measure(
                font_cx,
                cache,
                &editor.buffer[..s],
                bold,
                italic,
                state.zoom,
            );
        let ex = x
            + pad
            + measure(
                font_cx,
                cache,
                &editor.buffer[..e],
                bold,
                italic,
                state.zoom,
            );
        surface.fill_rect(
            Rect::new(sx, y + 2.0, ex, y + ch - 2.0),
            palette.selection.with_alpha(0.30),
        );
    }

    // Text: prefix + preview + suffix, baseline-centred in the row.
    let display = editor.display();
    if !display.is_empty() {
        let layout: &Layout<ParleyBrush> = cache.get_or_build(
            font_cx,
            &display,
            BODY_SIZE * state.zoom as f32,
            f32::MAX,
            Alignment::Start,
            bold,
            italic,
        );
        let text_h = layout.height() as f64;
        let y_off = ((ch - text_h) / 2.0).max(0.0);
        let brush = Brush::Solid(palette.cell_text);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let tx = x + pad + run.offset() as f64;
                let ty = y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
    }

    // Composition underline: a dotted line under the preview run.
    if !editor.preview.is_empty() {
        let pre_w = measure(
            font_cx,
            cache,
            &editor.buffer[..editor.caret],
            bold,
            italic,
            state.zoom,
        );
        let prev_w = measure(font_cx, cache, &editor.preview, bold, italic, state.zoom);
        let ux0 = x + pad + pre_w;
        let ux1 = ux0 + prev_w;
        let uy = (y + ch - 3.0).min(y + ch);
        // Dotted: 3px dash, 2px gap.
        let mut dx = ux0;
        while dx < ux1 {
            let seg_end = (dx + 3.0).min(ux1);
            surface.stroke_line(dx, uy, seg_end, uy, palette.selection, 1.0);
            dx += 5.0;
        }
    }

    // Caret: a 1px vertical bar after the prefix + preview, drawn only on the
    // visible half of the blink cycle.
    if caret_visible {
        let caret_w = measure(
            font_cx,
            cache,
            &editor.before_caret(),
            bold,
            italic,
            state.zoom,
        );
        let caret_x = x + pad + caret_w;
        surface.stroke_line(caret_x, y + 3.0, caret_x, y + ch - 3.0, palette.caret, 1.0);
    }

    surface.pop_clip();
}
