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

use ambre_core::index::Span;
use ambre_core::{Px, Surface};
use ambre_draw::{Clock, Color, DrawSurface, FontContext, Rect};

use ambre_canvas::Palette;

use super::TrameUiState;
use super::grid::{self, COL_WIDTH, CellEditor, ROW_HEIGHT};
use crate::render::{self, BODY_SIZE, CELL_PAD_X, GridLayout, TextPen};
use crate::{CellPos, Col, LAST_COL, LAST_ROW, Row, TOTAL_COLS, TOTAL_ROWS};

// Rendering entry points are split by layer; shared layout/stats/helpers stay
// here and reach the sub-modules via the glob mesh.
mod cells;
mod edit;
mod overlay;

pub use cells::draw_cells;
pub use edit::draw_edit;
pub use overlay::draw_overlay;
// Exposed to the test module (which calls it as `super::mdi_filter_polygon`).
use ambre_core::{FracMs, TimeSpan};
#[cfg(test)]
use overlay::mdi_filter_polygon;

/// Build the shared [`GridLayout`] for the current screen viewport. Carries the
/// per-axis width/height maps (`trame-resize`) so resized axes reflow; the
/// cell-aligned scroll position is the layout origin.
fn screen_layout(state: &TrameUiState) -> GridLayout<'_> {
    GridLayout {
        // Collapse the row axis over filter-hidden rows (`trame-shared-filter`).
        hidden_rows: &state.hidden_rows,
        // Header strips zoom with the grid: pass the already-scaled sizes (the
        // layout multiplies only the content-area metrics by `zoom`).
        header_width: state.header_w(),
        header_height: state.header_h(),
        top: ambre_core::Px::<ambre_core::Surface>::ZERO,
        col_width: COL_WIDTH,
        row_height: ROW_HEIGHT,
        col_widths: &state.col_widths,
        row_heights: &state.row_heights,
        origin_col: state.left_col,
        origin_row: state.top_row,
        width: state.canvas_width,
        // The grid stops above the bottom sheet-tab strip (`trame-sheets`).
        height: state.grid_bottom(),
        total_cols: TOTAL_COLS,
        total_rows: TOTAL_ROWS,
        zoom: state.zoom,
    }
}

/// Sub-phase timings for one frame.
///
/// All times are spans of the page clock, read through `performance.now()`.
/// The GPU work (copy_texture, blit pass, present) happens after these CPU
/// phases return and is measured separately as `last_gpu` in `UiState`.
#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    /// Cells layer non-text geometry: pending bg, received-cell fills,
    /// grid lines. Zero on overlay-only frames.
    pub background: TimeSpan<FracMs>,
    /// Cells layer text (parley shaping + glyph atlas + push_glyph per cell).
    /// Zero on overlay-only frames or when `text_pass` is false.
    pub cells: TimeSpan<FracMs>,
    /// Overlay layer non-HUD: header backgrounds, active tints, separator
    /// lines, active border, header labels.
    pub headers: TimeSpan<FracMs>,
    /// Number of cells with non-empty text actually drawn this frame.
    pub cells_drawn: ambre_core::Count<ambre_canvas::CellsDrawn>,
    /// Latest measured input->caret latency (keydown to caret paint).
    /// Read against the keystroke-to-draft budget of `ambre-spec-budgets.md`.
    pub input_latency: TimeSpan<FracMs>,
}

/// Opacity of the veil painted over a dirty (awaiting-recompute) cell. The
/// content_bg-coloured veil fades the optimistic value toward the background.
const DIRTY_VEIL_ALPHA: f32 = 0.5;

/// Leg length, in logical pixels, of the right-triangle error marker drawn in
/// a cell's top-right corner.
const ERROR_MARK_SIZE: Px<Surface> = Px::<Surface>::new(7.0);

/// The half-open rows and columns covering a repaint strip. Bundling both
/// spans keeps the per-cell helpers' argument lists short.
#[derive(Clone, Copy)]
struct CellSpan {
    rows: Span<Row>,
    cols: Span<Col>,
}

impl CellSpan {
    /// True when `at` falls inside the span.
    fn contains(&self, at: CellPos) -> bool {
        self.rows.contains(at.row) && self.cols.contains(at.col)
    }
}

/// Compute the cell-coordinate range that intersects `strip` in the cell area.
fn rows_cols_in_strip(state: &TrameUiState, strip: Rect) -> CellSpan {
    let (x0, x1) = (Px::<Surface>::new(strip.x0), Px::<Surface>::new(strip.x1));
    let (y0, y1) = (Px::<Surface>::new(strip.y0), Px::<Surface>::new(strip.y1));
    let cells_top = state.header_h();
    // Walk the variable column widths from the scroll origin: skip columns wholly
    // left of the strip, then extend until the strip's right edge is covered.
    let mut col_start = state.left_col;
    let mut cx = state.header_w();
    while col_start < LAST_COL && cx + state.col_width_of(col_start) <= x0 {
        cx += state.col_width_of(col_start);
        col_start = col_start.next();
    }
    let mut col_end = col_start;
    let mut ex = cx;
    while col_end < TOTAL_COLS.as_end() && ex < x1 {
        ex += state.col_width_of(col_end);
        col_end = col_end.next();
    }
    let col_end = col_end.min(TOTAL_COLS.as_end());

    // Same walk over the *visible* rows' variable heights (hidden rows take no
    // vertical space; the cell iterator skips them too).
    let mut row_start = state.top_row;
    let mut cy = cells_top;
    while row_start < LAST_ROW {
        if state.is_row_hidden(row_start) {
            row_start = row_start.next();
            continue;
        }
        let h = state.row_height_of(row_start);
        if cy + h <= y0 {
            cy += h;
            row_start = row_start.next();
        } else {
            break;
        }
    }
    let mut row_end = row_start;
    let mut ey = cy;
    while row_end < TOTAL_ROWS.as_end() && ey < y1 {
        if !state.is_row_hidden(row_end) {
            ey += state.row_height_of(row_end);
        }
        row_end = row_end.next();
    }
    let row_end = row_end.min(TOTAL_ROWS.as_end());
    CellSpan {
        rows: Span::<Row>::new(row_start, row_end),
        cols: Span::<Col>::new(col_start, col_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::TextPass;
    use ambre_core::index::{Delta, Pos};

    fn pointer(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
        ambre_core::Point::new(
            ambre_core::Px::<ambre_core::View>::new(x as f32),
            ambre_core::Px::<ambre_core::View>::new(y as f32),
        )
    }
    // Header strip metrics: only the tests still reference the raw constants
    // (production code goes through `state.header_w()` / `header_h()`).
    use super::grid::{HEADER_HEIGHT, HEADER_WIDTH};
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::FontContext;
    use ambre_draw::testing::{DrawOp, FixedClock, RecordingSurface};

    use super::super::grid::CellData;
    use crate::test_support::{at, rect, span};
    use crate::{Col, Row};

    /// A view state large enough to expose several rows/cols, with the given
    /// cells pre-populated in the cache.
    fn fixture(cells: &[(u32, u32, &str)]) -> TrameUiState {
        let mut state = TrameUiState::new(Px::<Surface>::new(800.0), Px::<Surface>::new(600.0));
        for &(row, col, text) in cells {
            state.cell_cache.insert(
                CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)),
                CellData {
                    text: text.to_owned(),
                    raw: crate::formula::text::CellText::<crate::formula::text::DisplayForm>::new(
                        text.to_owned(),
                    ),
                    ..Default::default()
                },
            );
        }
        state
    }

    /// `TRAME-UI-143`: the rect the renderer lays a cell in is the rect hit
    /// testing walks, under zoom, resized axes, hidden rows and a scroll. Its
    /// centre hit-tests back to the same cell.
    #[test]
    fn the_drawn_cell_is_the_hit_tested_cell() {
        let mut state = fixture(&[]);
        assert!(
            state.set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(1.25))
        );
        state.col_widths.insert(
            Pos::<Col>::new(4),
            ambre_core::Px::<ambre_core::Doc>::new(150.0),
        );
        state.col_widths.insert(
            Pos::<Col>::new(6),
            ambre_core::Px::<ambre_core::Doc>::new(30.0),
        );
        state.row_heights.insert(
            Pos::<Row>::new(13),
            ambre_core::Px::<ambre_core::Doc>::new(44.0),
        );
        state.hidden_rows = vec![Pos::<Row>::new(12), Pos::<Row>::new(15)];
        assert!(state.scroll_by(Delta::<Row>::new(10), Delta::<Col>::new(3)));
        let layout = screen_layout(&state);
        let mut checked = 0;
        for row in 10..22 {
            for col in 3..9 {
                let Some((x, y, w, h)) = state
                    .cell_pixel_rect(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
                    .map(|r| (r.x0, r.y0, r.width(), r.height()))
                else {
                    assert!(
                        layout.is_hidden(Pos::<Row>::new(row)),
                        "({row}, {col}) is drawn and not hit"
                    );
                    continue;
                };
                let drawn =
                    layout.cell_rect(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)));
                for (a, b) in [
                    (x, drawn.x0),
                    (y, drawn.y0),
                    (w, drawn.width()),
                    (h, drawn.height()),
                ] {
                    assert!((a - b).abs() < 1e-9, "({row}, {col}): hit {a} drawn {b}");
                }
                let centre = drawn.center();
                assert_eq!(
                    state.cell_at(pointer(centre.x, centre.y)),
                    Some(at(row, col))
                );
                checked += 1;
            }
        }
        assert!(checked > 40, "{checked} cells compared");
    }

    /// Shared harness: a recording surface, a fixed clock, a real (empty) font
    /// context and text cache. Text shaping may yield zero glyphs on a host with
    /// no fonts. Tests therefore never assert glyph *counts*, only structural
    /// ops and the renderer's own `cells_drawn` count, which is
    /// font-independent.
    fn harness() -> (
        RecordingSurface,
        FixedClock,
        FontContext,
        TextCache,
        DrawStats,
    ) {
        (
            RecordingSurface::new(),
            FixedClock::default(),
            FontContext::new(),
            TextCache::new(),
            DrawStats::default(),
        )
    }

    // `proptest` is a host-only dev-dependency (see `Cargo.toml`).
    #[cfg(not(target_arch = "wasm32"))]
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(32))]

        /// Whatever the zoom, the scroll and the canvas size, the cells layer
        /// draws the sheet's walk for the region it shows, call for call
        /// (`CANVAS-074`). The dirty veil and the error marker are drawn
        /// apart. The surface exposes no density, so the walk cannot read one.
        #[test]
        fn the_screen_draws_the_walk_at_any_camera(
            zoom in 0.5f64..2.0,
            left_col in 0u32..6,
            top_row in 0u32..6,
            width in 300.0f64..1600.0,
            height in 300.0f64..1200.0,
        ) {
            use crate::render::{SheetView, SheetWalk};
            use ambre_draw::DocumentWalk;

            let mut state = fixture(&[
                (2, 2, "wide text that overflows"),
                (3, 4, "12"),
                (4, 3, "merged"),
                (6, 6, "x"),
                (9, 1, "hidden"),
            ]);
            state.cell_cache.get_mut(&at(4, 3)).unwrap().merge_span = Some(span(2, 2));
            state.col_widths.insert(Pos::<Col>::new(3), ambre_core::Px::<ambre_core::Doc>::new(150.0));
            state.row_heights.insert(Pos::<Row>::new(5), ambre_core::Px::<ambre_core::Doc>::new(44.0));
            state.hidden_rows = vec![Pos::<Row>::new(9)];
            state.dirty_cells.insert(CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(2)));
            state.resize(Px::<Surface>::new(width), Px::<Surface>::new(height));
            state.set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(zoom));
            state.left_col = Pos::<Col>::new(left_col);
            state.top_row = Pos::<Row>::new(top_row);
            let palette = Theme::Light.palette();
            let (mut screen, clock, mut font_cx, mut cache, mut stats) = harness();
            draw_cells(
                &mut screen,
                &state,
                palette,
                TextPen { font_cx: &mut font_cx, cache: &mut cache, pass: TextPass::WithText },
                None,
                &clock,
                &mut stats,
            );

            let cell_area = Rect::new(
                state.header_w().get(),
                state.header_h().get(),
                state.canvas_width.get(),
                state.grid_bottom().get(),
            );
            let region = cells::sheet_camera(&state)
                .inverse()
                .transform_rect_bbox(cell_area);
            let cells = cells::CacheCells(&state.cell_cache);
            let view = SheetView {
                cells: &cells,
                axes: cells::sheet_axes(&state),
                palette,
                text_pass: TextPass::WithText,
            };
            let mut walked = RecordingSurface::new();
            SheetWalk {
                font_cx: &mut font_cx,
                text_cache: &mut cache,
                cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
            }
            .walk(&view, region, &mut walked);
            proptest::prop_assert!(screen.holds_walk(&walked));
        }
    }

    /// A border the user did not colour follows the active theme (`TRAME-101`):
    /// its stroke is the palette's `cell_border`, and the two themes differ.
    #[test]
    fn uncoloured_border_follows_the_theme() {
        let mut state = fixture(&[(0, 0, "x")]);
        if let Some(cell) = state.cell_cache.get_mut(&at(0, 0)) {
            cell.borders.top = Some(ambre_bus::CellBorder {
                style: ambre_bus::BorderStyle::Solid,
                color: None,
            });
        }
        let stroked = |theme: Theme| {
            let palette = theme.palette();
            let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();
            draw_cells(
                &mut surface,
                &state,
                palette,
                TextPen {
                    font_cx: &mut font_cx,
                    cache: &mut cache,
                    pass: TextPass::WithText,
                },
                None,
                &clock,
                &mut stats,
            );
            surface.count(|op| {
                matches!(
                    op, DrawOp::StrokeLine { color, .. } if *color == palette.cell_border
                )
            })
        };
        assert_ne!(
            Theme::Light.palette().cell_border,
            Theme::Dark.palette().cell_border
        );
        assert_eq!(stroked(Theme::Light).get(), 1);
        assert_eq!(stroked(Theme::Dark).get(), 1);
    }

    #[test]
    fn draw_cells_full_repaint_paints_pending_then_grid() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        // First op is the full-area pending-grey fill.
        assert_eq!(
            surface.ops().first(),
            Some(&DrawOp::FillRect {
                rect: Rect::new(
                    HEADER_WIDTH.get(),
                    HEADER_HEIGHT.get(),
                    800.0,
                    state.grid_bottom().get(),
                ),
                color: palette.content_pending,
            }),
        );
        // Grid lines are stroked with the palette grid colour.
        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line
                ))
                .get()
                > 0
        );
        // The clip stack is balanced and no cell bg was painted (empty cache).
        assert!(surface.clips_balanced());
        assert_eq!(
            stats.cells_drawn,
            ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO
        );
    }

    #[test]
    fn draw_cells_scroll_strip_base_is_blank_not_pending_grey() {
        // A scroll strip repaints a band entering the viewport on top of the
        // shifted pixels. Its base fill must be the blank content background, not
        // the grey "pending" veil. A cell scrolled in faster than the Logic
        // round-trip should read as empty until its data lands, never flash grey.
        // That grey flash is the Arrow/Page Up/wheel scroll flicker.
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        // One row entering at the top: [HEADER_HEIGHT, HEADER_HEIGHT + ROW_HEIGHT].
        let strip = Rect::new(
            HEADER_WIDTH.get(),
            HEADER_HEIGHT.get(),
            800.0,
            HEADER_HEIGHT.get() + f64::from(ROW_HEIGHT.get()),
        );
        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            Some(strip),
            &clock,
            &mut stats,
        );

        // The strip base fill is content_bg, and the grey pending veil never
        // appears during a scroll repaint.
        assert_eq!(
            surface.ops().first(),
            Some(&DrawOp::FillRect {
                rect: strip,
                color: palette.content_bg,
            }),
        );
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == palette.content_pending
                ))
                .get(),
            0,
        );
    }

    #[test]
    fn draw_cells_fills_cached_cell_and_counts_text() {
        // Two cells in view: one with text, one empty (pending stays, but its bg
        // is filled because it is "received").
        let state = fixture(&[(0, 0, "hello"), (1, 1, "")]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        // Both received cells get a content_bg fill...
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == palette.content_bg
                ))
                .get(),
            2,
        );
        // ...but only the non-empty one is counted as drawn text.
        assert_eq!(
            stats.cells_drawn,
            ambre_core::Count::<ambre_canvas::CellsDrawn>::new(1)
        );
        assert!(surface.clips_balanced());
    }

    #[test]
    fn draw_cells_renders_merge_master_over_its_span() {
        // A 2x2 merge master at (0,0) plus its three blank covered cells.
        let mut state = fixture(&[(0, 0, "merged"), (0, 1, ""), (1, 0, ""), (1, 1, "")]);
        state.cell_cache.get_mut(&at(0, 0)).unwrap().merge_span = Some(span(2, 2));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        // The merge pass strokes the span's outer border with the grid colour as
        // a rect (the per-cell separators are only ever stroked as lines). The
        // walk draws in sheet coordinates: the camera maps the span onto the
        // screen layout's rect.
        let span_rect = Rect::new(
            0.0,
            0.0,
            2.0 * f64::from(COL_WIDTH.get()),
            2.0 * f64::from(ROW_HEIGHT.get()),
        );
        assert_eq!(
            cells::sheet_camera(&state).transform_rect_bbox(span_rect),
            screen_layout(&state).merge_rect(at(0, 0), span(2, 2)),
        );
        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeRect { rect, color, .. }
                        if *color == palette.grid_line && *rect == span_rect
                ))
                .get()
                == 1
        );
        // The span gets an opaque content_bg fill covering its full rectangle.
        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { rect, color }
                        if *color == palette.content_bg && *rect == span_rect
                ))
                .get()
                == 1
        );
        assert!(surface.clips_balanced());
    }

    /// A border a user asked for is drawn after every grid line and after the
    /// merge repaint, so neither can paint over it (`TRAME-100`).
    #[test]
    fn a_border_is_drawn_after_the_grid_and_the_merges() {
        let edge = Some(ambre_bus::CellBorder {
            style: ambre_bus::BorderStyle::Solid,
            color: None,
        });
        let mut state = fixture(&[(0, 0, "m"), (0, 1, ""), (1, 0, ""), (1, 1, ""), (3, 3, "x")]);
        let master = state.cell_cache.get_mut(&at(0, 0)).unwrap();
        master.merge_span = Some(span(2, 2));
        master.borders = ambre_bus::Borders {
            top: edge,
            right: edge,
            bottom: edge,
            left: edge,
        };
        state.cell_cache.get_mut(&at(3, 3)).unwrap().borders.top = edge;
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();
        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );
        let span_rect = screen_layout(&state).merge_rect(at(0, 0), span(2, 2));
        let ops = surface.ops();
        let is_border = |op: &DrawOp| matches!(op, DrawOp::StrokeLine { color, .. } if *color == palette.cell_border);
        let first_border = ops.iter().position(is_border).expect("a border");
        let last_grid = ops
            .iter()
            .rposition(|op| match op {
                DrawOp::StrokeLine { color, .. } | DrawOp::StrokeRect { color, .. } => {
                    *color == palette.grid_line
                }
                DrawOp::FillRect { rect, .. } => *rect == span_rect,
                _ => false,
            })
            .expect("a grid line");
        assert!(last_grid < first_border, "{last_grid} >= {first_border}");
        // Four edges round the span, one on the plain cell.
        assert_eq!(surface.count(is_border).get(), 5);
    }

    #[test]
    fn draw_cells_strip_skips_when_outside_grid() {
        // A strip entirely inside the header corner produces no cell drawing.
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        let header_corner = Rect::new(0.0, 0.0, HEADER_WIDTH.get(), HEADER_HEIGHT.get());
        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            Some(header_corner),
            &clock,
            &mut stats,
        );

        assert!(surface.ops().is_empty());
        assert_eq!(
            stats.cells_drawn,
            ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO
        );
    }

    #[test]
    fn draw_overlay_draws_headers_and_active_border() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            &clock,
            &mut stats,
        );

        // Header backgrounds use the palette header colour.
        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
                ))
                .get()
                >= 2
        );
        // The active cell (0,0) border is stroked with the selection colour.
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeRect { color, width, .. }
                        if *color == palette.selection && *width == ambre_core::Px::<ambre_core::Surface>::new(2.0)
                ))
                .get(),
            1,
        );
        assert!(surface.clips_balanced());
    }

    /// A lone selected cell shows its border and no range fill
    /// (`TRAME-UI-131`). The tint is the one a range takes.
    #[test]
    fn a_lone_selected_cell_shows_no_range_fill() {
        let mut state = fixture(&[]);
        state.select(CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(3)));
        assert!(!state.selection.is_multi());
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();
        draw_overlay(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            &clock,
            &mut stats,
        );
        let tint = palette.selection.with_alpha(0.16);
        assert_eq!(
            surface
                .count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == tint))
                .get(),
            0,
        );
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeRect { color, width, .. }
                        if *color == palette.selection && *width == ambre_core::Px::<ambre_core::Surface>::new(2.0)
                ))
                .get(),
            1,
        );
    }

    #[test]
    fn autofilter_buttons_drawn_on_region_header_row() {
        // A two-column data block around the active cell (0,0): each column draws
        // one autofilter button (bordered with the grid colour) on the band row.
        let state = fixture(&[(0, 0, "a"), (0, 1, "b")]);
        assert_eq!(state.current_region(), Some(rect(0, 0, 0, 1)));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            &clock,
            &mut stats,
        );

        // One bordered button per region column (cell grid lines are stroke_line,
        // so a grid-colour StrokeRect is unambiguously an autofilter button).
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeRect { color, .. } if *color == palette.grid_line
                ))
                .get(),
            2,
        );
        assert!(surface.clips_balanced());
    }

    #[test]
    fn autofilter_button_shows_funnel_on_filtered_column() {
        // With a filter active on column 0, that button paints the MdiFilter
        // funnel glyph as selection-coloured scanline FillRects (the GPU surface
        // has no fill_path). The secondary letter-header badge fills it too. An
        // unfiltered fixture draws no selection-coloured fills here.
        let mut state = fixture(&[(0, 0, "a"), (0, 1, "b")]);
        state.filtered_col = Some(Pos::<Col>::new(0));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            &clock,
            &mut stats,
        );

        // Several selection-coloured fill spans make up the funnel silhouette.
        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == palette.selection
                ))
                .get()
                > 2
        );
        assert!(surface.clips_balanced());
    }

    #[test]
    fn mdi_filter_polygon_parses_into_unit_box() {
        // The embedded MdiFilter path flattens to at least one non-empty contour,
        // normalized inside the unit box so the scanline fill maps cleanly.
        let contours = super::mdi_filter_polygon();
        assert!(contours.iter().any(|c| c.len() >= 3));
        for c in contours {
            for &kurbo::Point { x, y } in c {
                assert!((-1e-6..=1.0 + 1e-6).contains(&x), "x out of unit box: {x}");
                assert!((-1e-6..=1.0 + 1e-6).contains(&y), "y out of unit box: {y}");
            }
        }
    }

    #[test]
    fn multi_cell_selection_fills_and_outlines_each_range() {
        // A live range (0,0)..=(1,1) plus a disjoint single cell (3,3). Both get
        // a translucent selection fill, and no 2px active-cell border is drawn;
        // the range outline alone marks the selection, as in Excel.
        let mut state = fixture(&[]);
        state.select(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)));
        state.select_extend(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)));
        state.select_disjoint(CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(3)));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            &clock,
            &mut stats,
        );

        // One translucent fill per selected rectangle (2 rects).
        let tint = palette.selection.with_alpha(0.16);
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == tint
                ))
                .get(),
            2,
        );
        // No 2px active-cell border in a multi-cell selection.
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeRect { color, width, .. }
                        if *color == palette.selection && *width == ambre_core::Px::<ambre_core::Surface>::new(2.0)
                ))
                .get(),
            0,
        );
    }

    /// A whole-axis selection tints its band in the strip (`TRAME-UI-047`):
    /// columns tint the column strip, rows the row strip.
    #[test]
    fn a_whole_axis_selection_tints_its_strip() {
        use super::super::grid::{AxisPos, SelKind, Selection};
        let palette = Theme::Light.palette();
        for kind in [SelKind::Cols, SelKind::Rows] {
            let mut state = fixture(&[]);
            let (from, to) = match kind {
                SelKind::Cols => (
                    AxisPos::Col(Pos::<Col>::new(1)),
                    AxisPos::Col(Pos::<Col>::new(2)),
                ),
                _ => (
                    AxisPos::Row(Pos::<Row>::new(1)),
                    AxisPos::Row(Pos::<Row>::new(2)),
                ),
            };
            state.selection = Selection::axis(from, to);
            let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();
            draw_overlay(
                &mut surface,
                &state,
                palette,
                TextPen {
                    font_cx: &mut font_cx,
                    cache: &mut cache,
                    pass: TextPass::WithText,
                },
                &clock,
                &mut stats,
            );
            let (x0, x1, y0, y1) = match kind {
                SelKind::Cols => (
                    state.col_x(Pos::<Col>::new(1)).get(),
                    state.col_x(Pos::<Col>::new(2)).get()
                        + state.col_width_of(Pos::<Col>::new(2)).get(),
                    0.0,
                    state.header_h().get(),
                ),
                _ => (
                    0.0,
                    state.header_w().get(),
                    state.row_y(Pos::<Row>::new(1)).get(),
                    state.row_y(Pos::<Row>::new(2)).get()
                        + state.row_height_of(Pos::<Row>::new(2)).get(),
                ),
            };
            assert_eq!(
                surface
                    .count(|op| matches!(
                        op, DrawOp::FillRect { color, rect }
                            if *color == palette.header_active
                                && *rect == Rect::new(x0, y0, x1, y1)
                    ))
                    .get(),
                1,
                "{kind:?}",
            );
        }
    }

    #[test]
    fn dirty_cell_gets_a_translucent_veil() {
        // An optimistic edit marks (0,0) dirty -> a content_bg veil at reduced
        // alpha is painted over it, on top of the opaque cell fill.
        let mut state = fixture(&[(0, 0, "x")]);
        state.set_cell_text(
            CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
            "5".to_owned(),
        );
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        let veil = palette.content_bg.with_alpha(super::DIRTY_VEIL_ALPHA);
        assert_eq!(
            surface
                .count(|op| matches!(
                    op, DrawOp::FillRect { color, .. } if *color == veil
                ))
                .get(),
            1,
        );
        assert_ne!(
            veil, palette.content_bg,
            "veil must differ from the opaque fill"
        );
    }

    #[test]
    fn errored_cell_gets_a_corner_marker() {
        // A formula error on (1,1) draws the corner marker with the error colour;
        // a clean cell draws none.
        let mut state = fixture(&[(1, 1, "")]);
        state.set_formula_error(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        assert!(
            surface
                .count(|op| matches!(
                    op, DrawOp::StrokeLine { color, .. } if *color == palette.cell_error
                ))
                .get()
                > 0
        );
    }

    #[test]
    fn renderer_uses_only_palette_colors_for_fills() {
        // Guards the "no hardcoded product colour" invariant on the cells layer.
        // Every fill colour must come from the active palette; the HUD, which
        // does use literals, lives only in the overlay.
        let state = fixture(&[(0, 0, "x")]);
        let palette = Theme::Light.palette();
        let allowed = [palette.content_pending, palette.content_bg];
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            TextPen {
                font_cx: &mut font_cx,
                cache: &mut cache,
                pass: TextPass::WithText,
            },
            None,
            &clock,
            &mut stats,
        );

        for color in surface.fill_colors() {
            assert!(allowed.contains(&color), "unexpected fill colour {color:?}");
        }
    }
}
