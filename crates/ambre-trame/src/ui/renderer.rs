// ambre-trame/src/ui/renderer.rs
// ============================================================================
// Trame grid renderer -- UI Thread.
//
// Split into two layers to support dirty-rect rendering:
//
//   draw_cells (CELLS layer, persistent offscreen):
//       white bg + pending bg + received-cell fills + grid lines + cell text.
//       Either full-canvas redraw or strip-only (during scroll), controlled by
//       the `strip_rect` argument.
//
//   draw_overlay (OVERLAY layer, swapchain):
//       column/row header backgrounds + active tints + separator lines
//       + active cell border + column/row header labels + on-canvas HUD.
//
// The orchestrator (`TrameView::plan_repaint`, driven by the launcher's FrameHost)
// detects what changed since the previous frame and selects the appropriate
// combination:
//   - Full repaint:      draw_cells(strip=None) + draw_overlay
//   - Pure scroll:       draw_cells(strip=Some(strip_rect)) + draw_overlay
//   - Overlay-only diff: draw_overlay
// ============================================================================

use ambre_draw::{
    Affine, Alignment, Brush, Clock, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

use ambre_canvas::{Palette, TextCache};

use super::grid::{CellEditor, COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT};
use super::TrameUiState;
use crate::render::{self, BODY_SIZE, CELL_PAD_X, CellContent, GridLayout};
use crate::{TOTAL_COLS, TOTAL_ROWS};

/// Build the shared [`GridLayout`] for the current screen viewport. Uniform
/// metrics for the MVP; the cell-aligned scroll position is the layout origin.
fn screen_layout(state: &TrameUiState) -> GridLayout {
    GridLayout {
        header_width: HEADER_WIDTH,
        header_height: HEADER_HEIGHT,
        col_width: COL_WIDTH,
        row_height: ROW_HEIGHT,
        origin_col: state.left_col,
        origin_row: state.top_row,
        width: state.canvas_width,
        height: state.canvas_height,
        total_cols: TOTAL_COLS,
        total_rows: TOTAL_ROWS,
    }
}

// -----------------------------------------------------------------------------
// Colors
// -----------------------------------------------------------------------------
//
// Product colours come from the active `Palette` (generated from tokens.ron,
// theme-switched at runtime). Only the debug HUD keeps hardcoded colours -- it
// is a developer overlay, not themed product chrome.

// -----------------------------------------------------------------------------
// DrawStats — per-frame breakdown
// -----------------------------------------------------------------------------

/// Sub-phase timings for one frame.
///
/// All times are wall-clock milliseconds measured via `performance.now()`.
/// The GPU work (copy_texture, blit pass, present) happens after these CPU
/// phases return and is measured separately as `last_gpu_ms` in `UiState`.
#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    /// Cells layer non-text geometry: pending bg, received-cell fills,
    /// grid lines. Zero on overlay-only frames.
    pub bg_ms: f32,
    /// Cells layer text (parley shaping + glyph atlas + push_glyph per cell).
    /// Zero on overlay-only frames or when `text_enabled` is false.
    pub cells_ms: f32,
    /// Overlay layer non-HUD: header backgrounds, active tints, separator
    /// lines, active border, header labels.
    pub hdrs_ms: f32,
    /// Overlay layer HUD overlay itself.
    pub hud_ms: f32,
    /// Number of cells with non-empty text actually drawn this frame.
    pub cells_drawn: u32,
    /// Latest measured input->caret latency (keydown to caret paint), in ms.
    /// Target < 50 ms, acceptable < 100 ms (ambre-spec-chrome 7, Phase 3).
    pub input_latency_ms: f32,
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Compute the inclusive cell-coordinate range that intersects `strip` in the
/// cell area. Returns `(row_start, row_end_exclusive, col_start, col_end_exclusive)`.
fn rows_cols_in_strip(
    state: &TrameUiState,
    strip: Rect,
) -> (u32, u32, u32, u32) {
    let row_start = if strip.y0 > HEADER_HEIGHT {
        ((strip.y0 - HEADER_HEIGHT) / ROW_HEIGHT).floor() as u32 + state.top_row
    } else {
        state.top_row
    };
    let row_end_excl = (((strip.y1 - HEADER_HEIGHT) / ROW_HEIGHT).ceil() as u32
        + state.top_row)
        .min(TOTAL_ROWS);
    let col_start = if strip.x0 > HEADER_WIDTH {
        ((strip.x0 - HEADER_WIDTH) / COL_WIDTH).floor() as u32 + state.left_col
    } else {
        state.left_col
    };
    let col_end_excl = (((strip.x1 - HEADER_WIDTH) / COL_WIDTH).ceil() as u32
        + state.left_col)
        .min(TOTAL_COLS);
    (row_start, row_end_excl, col_start, col_end_excl)
}

// -----------------------------------------------------------------------------
// draw_cells -- persistent CELLS layer
// -----------------------------------------------------------------------------

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
    surface:      &mut S,
    state:        &TrameUiState,
    palette:      &Palette,
    font_cx:      &mut FontContext,
    text_cache:   &mut TextCache,
    text_enabled:   bool,
    strip_rect:     Option<Rect>,
    clock:        &dyn Clock,
    stats:        &mut DrawStats,
) {
    let w = state.canvas_width;
    let h = state.canvas_height;

    // Cell area = full canvas minus the fixed-position header strip.
    let cell_area = Rect::new(HEADER_WIDTH, HEADER_HEIGHT, w, h);
    let strip = match strip_rect {
        Some(r) => r.intersect(cell_area),
        None    => cell_area,
    };
    if strip.width() <= 0.0 || strip.height() <= 0.0 {
        stats.bg_ms      = 0.0;
        stats.cells_ms   = 0.0;
        stats.cells_drawn = 0;
        return;
    }

    let layout = screen_layout(state);
    let t0 = clock.now_ms();

    // -- 1. Pending grey over the strip (screen-only "not yet received"). -------
    surface.fill_rect(strip, palette.content_pending);
    let t_pending = clock.now_ms();

    // -- 2. Received-cell backgrounds + body text, via the shared primitive. ---
    // The cell area is virtual, so we feed only the cached cells inside the
    // strip rather than a full DisplayViewport.
    let (row_start, row_end, col_start, col_end) = rows_cols_in_strip(state, strip);
    let cells = (row_start..row_end).flat_map(move |row| {
        (col_start..col_end).filter_map(move |col| {
            state.cell_cache.get(&(row, col)).map(|c| {
                (row, col, CellContent { text: &c.text, bold: c.bold, italic: c.italic })
            })
        })
    });
    let cells_drawn =
        render::draw_cells_on(surface, &layout, palette, font_cx, text_cache, text_enabled, cells);
    let t_cells = clock.now_ms();

    // -- 3. Grid lines on top of the fills, so separators stay visible. --------
    render::draw_grid_lines_on(surface, &layout, palette.grid_line, strip);
    let t_lines = clock.now_ms();

    stats.bg_ms       = ((t_pending - t0) + (t_lines - t_cells)) as f32;
    stats.cells_ms    = (t_cells - t_pending) as f32;
    stats.cells_drawn = cells_drawn;
}

// -----------------------------------------------------------------------------
// draw_overlay -- transient OVERLAY layer
// -----------------------------------------------------------------------------

/// Draw the overlay layer (headers + active border + header labels + HUD).
/// This is re-rendered every dirty frame onto the swapchain after the cells
/// layer has been blitted from its persistent offscreen.
#[allow(clippy::too_many_arguments)]
pub fn draw_overlay<S: DrawSurface>(
    surface:        &mut S,
    state:          &TrameUiState,
    palette:        &Palette,
    font_cx:        &mut FontContext,
    text_cache:     &mut TextCache,
    text_enabled:   bool,
    render_fps:     f32,
    last_frame_ms:  f32,
    last_drain_ms:  f32,
    last_draw_ms:   f32,
    last_gpu_ms:    f32,
    clock:          &dyn Clock,
    stats:          &mut DrawStats,
) {
    let h = state.canvas_height;

    let t0 = clock.now_ms();

    let layout = screen_layout(state);

    // -- 1. Fixed header strips (bg + active tint + separator + labels), via
    //       the shared primitives. The primitives clip the active tint to the
    //       visible range, so passing the raw active index is safe. -----------
    render::draw_col_headers_on(
        surface, &layout, palette, Some(state.active_col), font_cx, text_cache, text_enabled,
    );
    render::draw_row_headers_on(
        surface, &layout, palette, Some(state.active_row), font_cx, text_cache, text_enabled,
    );

    // -- 2. Active cell border. -----------------------------------------------
    // Lives in the overlay so it tracks the active cell on every frame
    // regardless of scroll shifts in the persistent layer.
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        surface.stroke_rect(Rect::new(x, y, x + cw, y + ch), palette.selection, 2.0);
    }
    let t_after_hdrs = clock.now_ms();

    // -- 6. HUD overlay. ------------------------------------------------------
    let fps_bg = Rect::new(4.0, h - 38.0, 520.0, h - 4.0);
    surface.fill_rect(fps_bg, Color::from_rgba8(0, 0, 0, 160));

    let text_tag  = if text_enabled { "" } else { " [T=text]" };
    let line1 = format!(
        "{render_fps:.0}fps {last_frame_ms:.1}ms  \
         drain:{last_drain_ms:.1} draw:{last_draw_ms:.1} gpu:{last_gpu_ms:.1}  \
         miss:{}{text_tag}",
        text_cache.frame_misses,
    );
    let line2 = format!(
        "bg:{:.2} cells:{:.2} hdrs:{:.2} hud:{:.2}  drawn:{}  inlat:{:.1}ms",
        stats.bg_ms, stats.cells_ms, stats.hdrs_ms, stats.hud_ms, stats.cells_drawn,
        stats.input_latency_ms,
    );
    render::draw_text(
        surface, font_cx, text_cache,
        &line1, 11.0, 514.0, Alignment::Start,
        6.0, h - 37.0,
        Color::from_rgba8(255, 220, 0, 255),
    );
    render::draw_text(
        surface, font_cx, text_cache,
        &line2, 11.0, 514.0, Alignment::Start,
        6.0, h - 21.0,
        Color::from_rgba8(180, 220, 255, 255),
    );

    let t_end = clock.now_ms();
    stats.hdrs_ms = (t_after_hdrs - t0) as f32;
    stats.hud_ms  = (t_end - t_after_hdrs) as f32;
}

// -----------------------------------------------------------------------------
// draw_edit -- in-progress cell edit (overlay, above everything)
// -----------------------------------------------------------------------------

/// Width in pixels of `text` shaped at the body size, or 0 for empty text.
fn measure(
    font_cx: &mut FontContext,
    cache:   &mut TextCache,
    text:    &str,
    bold:    bool,
    italic:  bool,
) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let layout: &Layout<ParleyBrush> =
        cache.get_or_build(font_cx, text, BODY_SIZE, f32::MAX, Alignment::Start, bold, italic);
    layout.width() as f64
}

/// Draw the active edit session over the cell: a white field, the buffer with
/// its IME composition preview, the caret, and a dotted underline under the
/// composition. Called on the overlay layer after `draw_overlay`, so it sits
/// above the active-cell border and the grid. Clipped to the cell, which is
/// itself clipped to the scrollable area (never under the header strip).
pub fn draw_edit<S: DrawSurface>(
    surface:       &mut S,
    state:         &TrameUiState,
    palette:       &Palette,
    editor:        &CellEditor,
    caret_visible: bool,
    font_cx:       &mut FontContext,
    cache:         &mut TextCache,
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

    let cell = Rect::new(x, y, x + cw, y + ch);
    surface.push_clip(cell);

    // Content-coloured field over the underlying cell + active border interior.
    surface.fill_rect(cell, palette.content_bg);

    // Text: prefix + preview + suffix, baseline-centred in the row.
    let display = editor.display();
    if !display.is_empty() {
        let layout: &Layout<ParleyBrush> = cache.get_or_build(
            font_cx, &display, BODY_SIZE, f32::MAX, Alignment::Start, bold, italic,
        );
        let text_h = layout.height() as f64;
        let y_off  = ((ch - text_h) / 2.0).max(0.0);
        let brush  = Brush::Solid(palette.cell_text);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = x + CELL_PAD_X + run.offset() as f64;
                let ty = y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
    }

    // Composition underline: a dotted line under the preview run.
    if !editor.preview.is_empty() {
        let pre_w  = measure(font_cx, cache, &editor.buffer[..editor.caret], bold, italic);
        let prev_w = measure(font_cx, cache, &editor.preview, bold, italic);
        let ux0 = x + CELL_PAD_X + pre_w;
        let ux1 = ux0 + prev_w;
        let uy  = (y + ch - 3.0).min(y + ch);
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
        let caret_w = measure(font_cx, cache, &editor.before_caret(), bold, italic);
        let caret_x = x + CELL_PAD_X + caret_w;
        surface.stroke_line(caret_x, y + 3.0, caret_x, y + ch - 3.0, palette.caret, 1.0);
    }

    surface.pop_clip();
}

// -----------------------------------------------------------------------------
// Headless tests -- mock DrawSurface (ambre-spec-canvas P0 "Headless tests").
// -----------------------------------------------------------------------------
//
// These drive the real grid renderer against `RecordingSurface` (a recording
// `DrawSurface`) and `FixedClock`, entirely off the browser main thread. They
// assert on the *sequence* of draw calls -- geometry, palette colours, clip
// balance -- not on pixels. This is the unit-test counterpart to the on-screen
// wgpu path; it needs no GPU, no canvas, no `performance.now()`.
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::FontContext;
    use ambre_draw::testing::{DrawOp, FixedClock, RecordingSurface};

    use super::super::grid::CellData;

    /// A view state large enough to expose several rows/cols, with the given
    /// cells pre-populated in the cache.
    fn fixture(cells: &[(u32, u32, &str)]) -> TrameUiState {
        let mut state = TrameUiState::new(800.0, 600.0);
        for &(row, col, text) in cells {
            state.cell_cache.insert(
                (row, col),
                CellData { text: text.to_owned(), bold: false, italic: false, merge_span: None },
            );
        }
        state
    }

    /// Shared harness: a recording surface, a fixed clock, a real (empty) font
    /// context and text cache. Text shaping may yield zero glyphs on a host with
    /// no fonts, so tests never assert glyph *counts* -- only structural ops and
    /// the renderer's own `cells_drawn` tally, which is font-independent.
    fn harness() -> (RecordingSurface, FixedClock, FontContext, TextCache, DrawStats) {
        (
            RecordingSurface::new(),
            FixedClock::default(),
            FontContext::new(),
            TextCache::new(1.0),
            DrawStats::default(),
        )
    }

    #[test]
    fn draw_cells_full_repaint_paints_pending_then_grid() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface, &state, palette, &mut font_cx, &mut cache,
            true, None, &clock, &mut stats,
        );

        // First op is the full-area pending-grey fill.
        assert_eq!(
            surface.ops().first(),
            Some(&DrawOp::FillRect {
                rect: Rect::new(HEADER_WIDTH, HEADER_HEIGHT, 800.0, 600.0),
                color: palette.content_pending,
            }),
        );
        // Grid lines are stroked with the palette grid colour.
        assert!(surface.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line
        )) > 0);
        // The clip stack is balanced and no cell bg was painted (empty cache).
        assert_eq!(surface.clip_depth(), 0);
        assert_eq!(stats.cells_drawn, 0);
    }

    #[test]
    fn draw_cells_fills_cached_cell_and_counts_text() {
        // Two cells in view: one with text, one empty (pending stays, but its bg
        // is filled because it is "received").
        let state = fixture(&[(0, 0, "hello"), (1, 1, "")]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface, &state, palette, &mut font_cx, &mut cache,
            true, None, &clock, &mut stats,
        );

        // Both received cells get a content_bg fill...
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.content_bg
            )),
            2,
        );
        // ...but only the non-empty one is tallied as drawn text.
        assert_eq!(stats.cells_drawn, 1);
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn draw_cells_strip_skips_when_outside_grid() {
        // A strip entirely inside the header corner produces no cell drawing.
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        let header_corner = Rect::new(0.0, 0.0, HEADER_WIDTH, HEADER_HEIGHT);
        draw_cells(
            &mut surface, &state, palette, &mut font_cx, &mut cache,
            true, Some(header_corner), &clock, &mut stats,
        );

        assert!(surface.ops().is_empty());
        assert_eq!(stats.cells_drawn, 0);
    }

    #[test]
    fn draw_overlay_draws_headers_and_active_border() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface, &state, palette, &mut font_cx, &mut cache,
            true, 60.0, 16.0, 1.0, 1.0, 1.0, &clock, &mut stats,
        );

        // Header backgrounds use the palette header colour.
        assert!(surface.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 2);
        // The active cell (0,0) border is stroked with the selection colour.
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeRect { color, width, .. }
                    if *color == palette.selection && *width == 2.0
            )),
            1,
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn renderer_uses_only_palette_colors_for_fills() {
        // Guards the "no hardcoded product colour" invariant on the cells layer:
        // every fill colour must come from the active palette (the HUD, which
        // does use literals, lives only in the overlay).
        let state = fixture(&[(0, 0, "x")]);
        let palette = Theme::Light.palette();
        let allowed = [palette.content_pending, palette.content_bg];
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface, &state, palette, &mut font_cx, &mut cache,
            true, None, &clock, &mut stats,
        );

        for color in surface.fill_colors() {
            assert!(allowed.contains(&color), "unexpected fill colour {color:?}");
        }
    }
}
