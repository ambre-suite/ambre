// ambre-trame/src/ui/renderer/cells.rs
// Cell-layer rendering: full/scroll/reuse repaint of the persistent grid.

use super::*;

/// Draw the cells layer (white bg + pending fill + cell text + grid lines).
///
/// `strip_rect == None`: full-canvas repaint. The caller must have started
/// the cells pass with LoadOp::Clear(WHITE) so the background is already
/// white; this function only adds the pending grey + cell fills + lines + text.
///
/// `strip_rect == Some(r)`: only the area inside `r` is repainted, on top of
/// existing (shifted) pixels. Pending grey covers the strip first, then cells
/// and grid lines are drawn over.
#[allow(clippy::too_many_arguments)]
pub fn draw_cells<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    strip_rect: Option<Rect>,
    clock: &dyn Clock,
    stats: &mut DrawStats,
) {
    let w = state.canvas_width;
    let h = state.grid_bottom();

    // Cell area = canvas minus the formula bar band + fixed header strip
    // (top/left) and the bottom sheet-tab strip.
    let cell_area = Rect::new(state.header_w(), state.header_h(), w, h);
    // The base fill under the cells differs by repaint kind:
    //  - Full (strip_rect == None): the grey "pending" veil, the first-load cue
    //    shown until the initial ViewportData lands.
    //  - Scroll (strip_rect == Some): the band entering the viewport sits on top
    //    of the shifted pixels. Fill it with the blank content background, NOT
    //    grey. The prefetch margin is meant to have these cells cached before
    //    they scroll in, but it can never win the async Logic round-trip every
    //    time (fast wheel, held arrow, a full-viewport Page Up jump). When it
    //    loses the race a grey block flashes for a frame -- the flicker. White is
    //    the graceful fallback: a not-yet-received cell reads as empty (which it
    //    usually is) until its data lands, never a flashing grey strip.
    let (strip, base_fill) = match strip_rect {
        Some(r) => (r.intersect(cell_area), palette.content_bg),
        None => (cell_area, palette.content_pending),
    };
    if strip.width() <= 0.0 || strip.height() <= 0.0 {
        stats.bg_ms = 0.0;
        stats.cells_ms = 0.0;
        stats.cells_drawn = 0;
        return;
    }

    let layout = screen_layout(state);
    let t0 = clock.now_ms();

    // -- 1. Base fill over the strip (see `base_fill` above). -------------------
    surface.fill_rect(strip, base_fill);
    let t_pending = clock.now_ms();

    // -- 2. Received-cell backgrounds + body text, via the shared primitive. ---
    // The cell area is virtual, so we feed only the cached cells inside the
    // strip rather than a full DisplayViewport.
    let span = rows_cols_in_strip(state, strip);
    let cells = (span.row_start..span.row_end)
        // A collapsed (filtered) row occupies no slot: skip it so its successors
        // are not double-drawn over the next visible row.
        .filter(move |row| !state.is_row_hidden(*row))
        .flat_map(move |row| {
            (span.col_start..span.col_end).filter_map(move |col| {
                state.cell_cache.get(&(row, col)).map(|c| {
                    // A merge master's text is laid out by the merge pass over the
                    // full span; drawing it here too would double it (single-cell
                    // baseline + span baseline). Pass empty text so only its bg fill
                    // lands here. Covered cells already carry empty text.
                    let text = if c.merge_span.is_some() {
                        ""
                    } else {
                        c.text.as_str()
                    };
                    (row, col, c.content(text))
                })
            })
        });
    // A neighbour is "empty" -- so an overflowing non-wrapped line may spill over
    // it -- when the cache has no entry for it or its text is blank. A merge
    // master keeps its (non-empty) text, so overflow stops at a merged block.
    let is_empty = |row: u32, col: u32| {
        state
            .cell_cache
            .get(&(row, col))
            .is_none_or(|c| c.text.is_empty())
    };
    let cells_drawn = render::draw_cells_on(
        surface,
        &layout,
        palette,
        font_cx,
        text_cache,
        text_enabled,
        cells,
        is_empty,
    );
    let t_cells = clock.now_ms();

    // -- 3. Grid lines on top of the fills, so separators stay visible. --------
    render::draw_grid_lines_on(surface, &layout, palette.grid_line, strip);

    // -- 3bis. Merge masters: paint over the interior separators of the covered
    // cells, lay the master's text across the span, re-stroke the outer border.
    // Iterates the (bounded) cache rather than the strip so a master pinned just
    // outside the strip still repaints the part of its span that is inside.
    let merges = state.cell_cache.iter().filter_map(|(&(row, col), c)| {
        c.merge_span.and_then(|(col_span, row_span)| {
            let hit = layout
                .merge_rect(row, col, col_span, row_span)
                .intersect(strip);
            (hit.width() > 0.0 && hit.height() > 0.0).then_some((
                row,
                col,
                col_span,
                row_span,
                c.content(&c.text),
            ))
        })
    });
    render::draw_merges_on(
        surface,
        &layout,
        palette,
        font_cx,
        text_cache,
        text_enabled,
        merges,
    );

    // -- 3ter. Cell borders LAST, so a deliberate border is never overpainted by a
    // faint grid separator nor by the merge repaint (`-ui.md` sec.5bis). A merged
    // master's border wraps the whole span (merge_rect); plain cells use their own
    // rect. Second pass over the strip (the first iterator was consumed by
    // draw_cells_on); plus the cache for merge masters pinned just outside it.
    let bspan = rows_cols_in_strip(state, strip);
    let strip_borders = (bspan.row_start..bspan.row_end)
        .filter(move |row| !state.is_row_hidden(*row))
        .flat_map(move |row| {
            (bspan.col_start..bspan.col_end).filter_map(move |col| {
                let c = state.cell_cache.get(&(row, col))?;
                // Merge masters are handled by the cache pass below (full span);
                // skip them here so a single-cell rect is not drawn under the span.
                if c.merge_span.is_some() {
                    return None;
                }
                Some((layout.cell_rect(row, col), c.content("")))
            })
        });
    render::draw_cell_borders_on(surface, strip_borders);
    // Merge masters: border around the full span rect.
    let merge_borders = state.cell_cache.iter().filter_map(|(&(row, col), c)| {
        let (col_span, row_span) = c.merge_span?;
        let rect = layout.merge_rect(row, col, col_span, row_span);
        let hit = rect.intersect(strip);
        (hit.width() > 0.0 && hit.height() > 0.0).then(|| (rect, c.content("")))
    });
    render::draw_cell_borders_on(surface, merge_borders);

    // -- 4. UI-only layers tied to cell content (dirty veil + error marker). ---
    // These live on the persistent cells layer so they scroll with the data,
    // and they are intentionally absent from the shared `render` primitives:
    // a PDF export never shows "pending recompute" or transient error state.
    draw_ui_cell_layers(surface, state, &layout, palette, span);
    let t_lines = clock.now_ms();

    stats.bg_ms = ((t_pending - t0) + (t_lines - t_cells)) as f32;
    stats.cells_ms = (t_cells - t_pending) as f32;
    stats.cells_drawn = cells_drawn;
}

/// Paint the per-cell UI-only layers for the cells intersecting the strip:
/// a reduced-opacity veil over dirty cells, and a corner marker over errored
/// cells. Iterates the (small) dirty/error sets rather than the visible grid,
/// rejecting entries outside the `span` already computed by the caller.
pub(crate) fn draw_ui_cell_layers<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    layout: &GridLayout,
    palette: &Palette,
    span: CellSpan,
) {
    // Dirty veil: a translucent content_bg quad fades the optimistic value.
    let veil = palette.content_bg.with_alpha(DIRTY_VEIL_ALPHA);
    for &(row, col) in &state.dirty_cells {
        if span.contains(row, col) && !state.is_row_hidden(row) {
            surface.fill_rect(layout.cell_rect(row, col), veil);
        }
    }

    // Error marker: a filled right-triangle in the cell's top-right corner,
    // assembled from 1px horizontal lines so it renders on the GPU surface
    // (WgpuSurface does not implement fill_path).
    let err = palette.cell_error;
    for &(row, col) in state.formula_errors.keys() {
        if !span.contains(row, col) || state.is_row_hidden(row) {
            continue;
        }
        let rect = layout.cell_rect(row, col);
        let mut dy = 0.0;
        while dy < ERROR_MARK_SIZE {
            // Row `dy` from the top spans the hypotenuse to the right edge.
            let x0 = rect.x1 - (ERROR_MARK_SIZE - dy);
            let y = rect.y0 + dy + 0.5;
            surface.stroke_line(x0, y, rect.x1, y, err, 1.0);
            dy += 1.0;
        }
    }
}
