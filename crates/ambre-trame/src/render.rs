// ambre-trame/src/render.rs
// ============================================================================
// Shared, surface-generic rendering primitives for the Trame grid.
//
// These functions are parameterized over the `DrawSurface` trait (defined in
// `ambre-draw`) so the *same* drawing code serves both:
//
//   - the UI thread, via `WgpuSurface` (live GPU rendering), called from
//     `ui::renderer` inside its dirty-rect / layer orchestration;
//   - the Logic thread, via `KrillaSurface` (vector PDF export, lazy-loaded),
//     once the PDF pipeline lands (see spec-ambre-trame-logic.md 9 / 14).
//
// Everything here is geometry-driven through `GridLayout` and plain cell data
// (`CellContent`), with no knowledge of scroll state, dirty rects, the HUD, or
// any other screen-only concern. Those live in `ui::renderer`, which composes
// these primitives in the right z-order.
//
// `GridLayout` is uniform-metrics for the MVP (one column width, one row
// height) so it stays O(1) over the virtual 100k grid -- never materializing a
// per-index offset vector. Variable column widths / row heights (resize) will
// extend `GridLayout` later without changing these primitive signatures.
// ============================================================================

use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

/// Cell text left padding (also the caret's x origin inside a cell).
pub const CELL_PAD_X: f64 = 4.0;
/// Body font size, shared by the cell renderer and the edit overlay.
pub const BODY_SIZE: f32 = 12.0;
/// Header label font size (column letters + row numbers).
pub const HEADER_SIZE: f32 = 11.0;

// -----------------------------------------------------------------------------
// GridLayout -- index <-> pixel mapping
// -----------------------------------------------------------------------------

/// Maps absolute grid indices to pixel rectangles on a target surface.
///
/// `origin_row` / `origin_col` are the absolute grid indices pinned to the
/// top-left corner of the content area (i.e. the scroll position, cell-aligned
/// for the MVP). For PDF export the origin is `(0, 0)`.
///
/// MVP: uniform `col_width` / `row_height`. The fields are deliberately shaped
/// so a future variable-size layout can add per-index offset slices without
/// breaking callers.
#[derive(Clone, Copy, Debug)]
pub struct GridLayout<'a> {
    /// Absolute grid rows hidden by the shared filter (`-ui.md` sec.5.D), sorted
    /// ascending. Such rows occupy no vertical space: the row axis collapses over
    /// them, so a hidden row's successors slide up by one `row_height`. Empty
    /// (the common case and PDF export) means a plain uniform layout. The slice
    /// only needs to cover the visible window -- rows beyond it are treated as
    /// visible.
    pub hidden_rows: &'a [u32],
    /// Width of the fixed row-number strip on the left.
    pub header_width: f64,
    /// Height of the fixed column-letter strip on the top.
    pub header_height: f64,
    /// Vertical offset (physical px) of the whole grid from the surface top:
    /// the height of any band reserved above the column-header strip (the
    /// formula bar on screen, `0.0` for PDF export).
    pub top: f64,
    /// Uniform column width (content area).
    pub col_width: f64,
    /// Uniform row height (content area).
    pub row_height: f64,
    /// Absolute grid column at the content area's left edge.
    pub origin_col: u32,
    /// Absolute grid row at the content area's top edge.
    pub origin_row: u32,
    /// Full surface width in device pixels (header strip included).
    pub width: f64,
    /// Full surface height in device pixels (header strip included).
    pub height: f64,
    /// Total columns in the grid (iteration bound).
    pub total_cols: u32,
    /// Total rows in the grid (iteration bound).
    pub total_rows: u32,
}

impl GridLayout<'_> {
    /// The scrollable content area (everything below + right of the headers).
    pub fn content_area(&self) -> Rect {
        Rect::new(self.header_width, self.top + self.header_height, self.width, self.height)
    }

    /// True if `row` is collapsed by the shared filter (binary search over the
    /// sorted [`hidden_rows`](Self::hidden_rows) slice).
    pub fn is_hidden(&self, row: u32) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Signed number of *visible* row slots between [`origin_row`](Self::origin_row)
    /// and `row`: the vertical offset (in `row_height` units) at which `row` is
    /// drawn, with the collapsed rows removed. Equals `row - origin_row` when no
    /// row in between is hidden.
    pub fn visible_offset(&self, row: u32) -> i64 {
        if self.hidden_rows.is_empty() {
            return row as i64 - self.origin_row as i64;
        }
        if row >= self.origin_row {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= self.origin_row && r < row)
                .count() as i64;
            (row - self.origin_row) as i64 - hidden
        } else {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= row && r < self.origin_row)
                .count() as i64;
            -((self.origin_row - row) as i64 - hidden)
        }
    }

    /// The absolute grid row drawn at visible slot `slot` (0 = the row at
    /// `origin_row`, skipping hidden rows). `None` once the grid is exhausted.
    pub fn row_at_slot(&self, slot: u32) -> Option<u32> {
        let mut row = self.origin_row;
        let mut seen = 0u32;
        while row < self.total_rows {
            if !self.is_hidden(row) {
                if seen == slot {
                    return Some(row);
                }
                seen += 1;
            }
            row += 1;
        }
        None
    }

    /// Pixel rect of cell `(row, col)` relative to the surface top-left.
    ///
    /// The result is unclipped: a cell scrolled left of / above the origin has
    /// a negative-offset (smaller-than-header) coordinate. Callers clip against
    /// `content_area()` or push their own clip. The vertical position accounts
    /// for collapsed (filtered) rows via [`visible_offset`](Self::visible_offset).
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        let x = self.header_width + (col as i64 - self.origin_col as i64) as f64 * self.col_width;
        let y = self.top + self.header_height + self.visible_offset(row) as f64 * self.row_height;
        Rect::new(x, y, x + self.col_width, y + self.row_height)
    }

    /// Pixel rect of a merge master `(row, col)` spanning `col_span` columns and
    /// `row_span` rows (both >= 1). Like [`cell_rect`](Self::cell_rect) the
    /// result is unclipped. The height counts only the *visible* rows in the span
    /// (a merge across a filtered row shrinks accordingly).
    pub fn merge_rect(&self, row: u32, col: u32, col_span: u32, row_span: u32) -> Rect {
        let base = self.cell_rect(row, col);
        let vis_rows = (row..row + row_span.max(1))
            .filter(|&r| !self.is_hidden(r))
            .count()
            .max(1) as f64;
        Rect::new(
            base.x0,
            base.y0,
            base.x0 + col_span.max(1) as f64 * self.col_width,
            base.y0 + vis_rows * self.row_height,
        )
    }
}

// -----------------------------------------------------------------------------
// Cell content view
// -----------------------------------------------------------------------------

/// A borrowed view of one cell's renderable content. Both the screen cache
/// (`ui::grid::CellData`) and a future `DisplayViewport` map onto this, so the
/// primitives never depend on either concrete type.
#[derive(Clone, Copy)]
pub struct CellContent<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub italic: bool,
    /// Underline run drawn under the glyphs (`-ui.md` sec.5bis).
    pub underline: bool,
    /// Strikethrough run drawn through the glyphs.
    pub strikethrough: bool,
    /// Horizontal alignment ([`HAlign::Auto`] resolves numbers right, text left).
    pub h_align: ambre_bus::HAlign,
    /// Vertical alignment within the cell (or the span rect for a merge master).
    pub v_align: ambre_bus::VAlign,
    /// Text colour, packed `0xRRGGBB` (`None` = the palette default).
    pub fg: Option<u32>,
    /// Fill colour, packed `0xRRGGBB` (`None` = the palette `content_bg`).
    pub bg: Option<u32>,
    /// Font family name (`None` = the grid default).
    pub font: Option<&'a str>,
    /// Font size in points (`None` = the grid default [`BODY_SIZE`]).
    pub size: Option<u16>,
    /// Wrap onto multiple lines inside the cell instead of overflowing.
    pub wrap: bool,
    /// Per-edge border (style + colour each); `None` edges are not drawn.
    pub borders: ambre_bus::Borders,
}

impl<'a> CellContent<'a> {
    /// A plain cell view (no rich style): the common case for tests and any caller
    /// that only carries text + bold/italic.
    pub fn plain(text: &'a str, bold: bool, italic: bool) -> Self {
        Self {
            text,
            bold,
            italic,
            underline: false,
            strikethrough: false,
            h_align: ambre_bus::HAlign::Auto,
            v_align: ambre_bus::VAlign::Middle,
            fg: None,
            bg: None,
            font: None,
            size: None,
            wrap: false,
            borders: ambre_bus::Borders::default(),
        }
    }
}

/// Unpack a `0xRRGGBB` colour into a [`Color`].
fn rgb(packed: u32) -> Color {
    Color::from_rgb8(
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

/// Resolve a cell's parley alignment from its [`HAlign`], deferring `Auto` to the
/// numeric-vs-text heuristic on `text`.
fn cell_alignment(h: ambre_bus::HAlign, text: &str) -> Alignment {
    match h {
        ambre_bus::HAlign::Left => Alignment::Start,
        ambre_bus::HAlign::Center => Alignment::Center,
        ambre_bus::HAlign::Right => Alignment::End,
        ambre_bus::HAlign::Auto => {
            if text.parse::<f64>().is_ok() {
                Alignment::End
            } else {
                Alignment::Start
            }
        }
    }
}

/// Vertical text offset inside a box of height `box_h` for a text block of height
/// `text_h`, honouring the cell's [`VAlign`](ambre_bus::VAlign). Clamped to `>= 0`
/// so oversized text never lifts above the box top.
fn cell_v_offset(v: ambre_bus::VAlign, box_h: f64, text_h: f64) -> f64 {
    match v {
        ambre_bus::VAlign::Top => 0.0,
        ambre_bus::VAlign::Middle => ((box_h - text_h) / 2.0).max(0.0),
        ambre_bus::VAlign::Bottom => (box_h - text_h).max(0.0),
    }
}

/// The default border colour (a mid grey), darker than the grid line so a border
/// reads as a deliberate edge.
const BORDER_DEFAULT: Color = Color::from_rgb8(0x60, 0x60, 0x60);

/// Resolve a [`BorderStyle`] into a stroke `(width, dash_period)`; `dash_period`
/// of `0.0` means a solid stroke. `Double` is handled by the caller (two passes).
fn border_stroke(style: ambre_bus::BorderStyle) -> (f64, f64) {
    use ambre_bus::BorderStyle as Bs;
    match style {
        Bs::Solid => (1.0, 0.0),
        Bs::Dashed => (1.0, 6.0),
        Bs::Dotted => (1.0, 2.5),
        Bs::Medium => (2.0, 0.0),
        Bs::Thick => (3.0, 0.0),
        Bs::Double => (1.0, 0.0),
    }
}

/// Stroke a straight axis-aligned segment in the given border `style`. Dashed and
/// dotted patterns are drawn as a run of short segments because [`DrawSurface`]
/// has no dash parameter; solid/medium/thick are a single stroke; `Double` draws
/// two thin parallel lines straddling the edge.
fn styled_segment<S: DrawSurface>(
    surface: &mut S,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: Color,
    style: ambre_bus::BorderStyle,
) {
    use ambre_bus::BorderStyle as Bs;
    let (width, period) = border_stroke(style);
    if style == Bs::Double {
        // Two 1px lines offset by 1px on each side of the edge.
        let (dx, dy) = if (y1 - y0).abs() < f64::EPSILON { (0.0, 1.0) } else { (1.0, 0.0) };
        surface.stroke_line(x0 - dx, y0 - dy, x1 - dx, y1 - dy, color, 1.0);
        surface.stroke_line(x0 + dx, y0 + dy, x1 + dx, y1 + dy, color, 1.0);
        return;
    }
    if period <= 0.0 {
        surface.stroke_line(x0, y0, x1, y1, color, width);
        return;
    }
    // Dashed / dotted: step along the segment drawing `on` runs.
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    if len <= 0.0 {
        return;
    }
    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
    let on = period * 0.6;
    let mut t = 0.0;
    while t < len {
        let s = t;
        let e = (t + on).min(len);
        surface.stroke_line(x0 + ux * s, y0 + uy * s, x0 + ux * e, y0 + uy * e, color, width);
        t += period;
    }
}

/// Stroke a cell's set border edges, each in its own style/colour, over `rect`.
fn draw_cell_border<S: DrawSurface>(surface: &mut S, rect: Rect, content: &CellContent<'_>) {
    let b = &content.borders;
    let mut edge = |cb: Option<ambre_bus::CellBorder>, x0, y0, x1, y1| {
        if let Some(cb) = cb {
            let color = cb.color.map(rgb).unwrap_or(BORDER_DEFAULT);
            styled_segment(surface, x0, y0, x1, y1, color, cb.style);
        }
    };
    edge(b.top, rect.x0, rect.y0, rect.x1, rect.y0);
    edge(b.bottom, rect.x0, rect.y1, rect.x1, rect.y1);
    edge(b.left, rect.x0, rect.y0, rect.x0, rect.y1);
    edge(b.right, rect.x1, rect.y0, rect.x1, rect.y1);
}

/// Whether any of the four edges carries a border (drives the border pass).
fn has_border(b: &ambre_bus::Borders) -> bool {
    b.top.is_some() || b.right.is_some() || b.bottom.is_some() || b.left.is_some()
}

/// Draw the border layer for each supplied `(rect, content)`. Called **after** the
/// grid lines (and the merge repaint) so a deliberate border is never overpainted;
/// `rect` is the owning cell's rect, or the full span rect for a merge master.
pub fn draw_cell_borders_on<'a, S, I>(surface: &mut S, cells: I)
where
    S: DrawSurface,
    I: IntoIterator<Item = (Rect, CellContent<'a>)>,
{
    for (rect, content) in cells {
        if has_border(&content.borders) {
            draw_cell_border(surface, rect, &content);
        }
    }
}

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
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
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

// -----------------------------------------------------------------------------
// draw_cells_on -- cell backgrounds + body text
// -----------------------------------------------------------------------------

/// Draw the background fill and body text for each supplied cell.
///
/// `cells` yields `(row, col, content)` for the cells that exist (on screen:
/// the cells received from the Logic thread; for PDF: every used cell). Each
/// gets an opaque `palette.content_bg` fill -- on screen this is what lifts a
/// cell out of the "pending" grey -- followed by its text when `text_enabled`.
///
/// Grid lines are *not* drawn here: call [`draw_grid_lines_on`] afterwards so
/// the separators sit on top of the fills. Returns the number of cells that
/// actually had non-empty text drawn (font-independent tally for stats/tests).
#[allow(clippy::too_many_arguments)]
pub fn draw_cells_on<'a, S, I>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    cells: I,
) -> u32
where
    S: DrawSurface,
    I: IntoIterator<Item = (u32, u32, CellContent<'a>)>,
{
    let cell_max_width = (layout.col_width - 8.0) as f32;
    let mut drawn = 0u32;

    for (row, col, content) in cells {
        let rect = layout.cell_rect(row, col);
        // Lift the cell out of the pending grey with its fill (the manual fill
        // colour if set, else the opaque content background).
        let bg = content.bg.map(rgb).unwrap_or(palette.content_bg);
        surface.fill_rect(rect, bg);

        // Borders are a separate pass (draw_cell_borders_on) AFTER the grid lines,
        // so a deliberate border is never overpainted by a faint separator.

        if !text_enabled || content.text.is_empty() {
            continue;
        }

        // Wrap constrains the layout to the cell width so parley breaks lines;
        // otherwise the text lays out unbounded (Excel-style overflow). We always
        // shape left-aligned and position the run horizontally ourselves: parley's
        // own alignment needs a finite advance, and an unbounded (non-wrap) layout
        // would push centred/right text off-screen.
        let max_w = if content.wrap { cell_max_width } else { f32::MAX };
        let font_size = content.size.map(|s| s as f32).unwrap_or(BODY_SIZE);
        let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build_font(
            font_cx,
            content.text,
            font_size,
            max_w,
            Alignment::Start,
            content.bold,
            content.italic,
            content.font,
        );
        let text_h = layout_text.height() as f64;
        let text_w = layout_text.width() as f64;
        let y_off = cell_v_offset(content.v_align, layout.row_height, text_h);
        // Horizontal alignment within the column content width (Auto resolves
        // numbers right, text left). Overflow keeps the text pinned to the start.
        let avail = cell_max_width as f64;
        let x_off = match cell_alignment(content.h_align, content.text) {
            Alignment::Center => ((avail - text_w) / 2.0).max(0.0),
            Alignment::End | Alignment::Right => (avail - text_w).max(0.0),
            _ => 0.0,
        };
        // Clip wrapped or vertically oversized text to the cell box; a non-wrapped
        // single line is allowed to overflow into empty neighbours.
        let needs_clip = text_h > layout.row_height || content.wrap;
        if needs_clip {
            surface.push_clip(rect);
        }
        let brush = Brush::Solid(content.fg.map(rgb).unwrap_or(palette.cell_text));
        for line in layout_text.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = rect.x0 + CELL_PAD_X + x_off + run.offset() as f64;
                let ty = rect.y0 + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                let dec_color = content.fg.map(rgb).unwrap_or(palette.cell_text);
                let adv = run.advance() as f64;
                // Underline run: a 1px line just below the glyph baseline.
                if content.underline {
                    surface.stroke_line(tx, ty + 2.0, tx + adv, ty + 2.0, dec_color, 1.0);
                }
                // Strikethrough run: a 1px line through the x-height.
                if content.strikethrough {
                    let sy = ty - (font_size as f64) * 0.3;
                    surface.stroke_line(tx, sy, tx + adv, sy, dec_color, 1.0);
                }
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        drawn += 1;
    }

    drawn
}

// -----------------------------------------------------------------------------
// draw_grid_lines_on -- 1px cell separators
// -----------------------------------------------------------------------------

/// Stroke the grid separators inside `clip`, intersected with the content area.
///
/// Drawn *after* the cell fills so the 1px boundaries stay visible between
/// adjacent filled cells.
pub fn draw_grid_lines_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    color: Color,
    clip: Rect,
) {
    let area = clip.intersect(layout.content_area());
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }

    // Vertical lines at each visible column boundary.
    let cells_top = layout.top + layout.header_height;
    let line_y0 = area.y0.max(cells_top);
    let line_y1 = area.y1.min(layout.height);
    if line_y1 > line_y0 {
        let mut i = 0u32;
        loop {
            let col = layout.origin_col + i;
            if col > layout.total_cols {
                break;
            }
            let x = layout.header_width + i as f64 * layout.col_width;
            if x > layout.width {
                break;
            }
            if x >= area.x0 && x <= area.x1 {
                surface.stroke_line(x, line_y0, x, line_y1, color, 1.0);
            }
            i += 1;
        }
    }

    // Horizontal lines at each visible row boundary.
    let line_x0 = area.x0.max(layout.header_width);
    let line_x1 = area.x1.min(layout.width);
    if line_x1 > line_x0 {
        let mut i = 0u32;
        loop {
            let row = layout.origin_row + i;
            if row > layout.total_rows {
                break;
            }
            let y = cells_top + i as f64 * layout.row_height;
            if y > layout.height {
                break;
            }
            if y >= area.y0 && y <= area.y1 {
                surface.stroke_line(line_x0, y, line_x1, y, color, 1.0);
            }
            i += 1;
        }
    }
}

// -----------------------------------------------------------------------------
// draw_merges_on -- merge masters spanning their covered area
// -----------------------------------------------------------------------------

/// Draw the merge masters over the base cell + grid-line layer.
///
/// `merges` yields `(row, col, col_span, row_span, content)` for the visible
/// merge masters (a covered cell is *not* a master and must not appear here --
/// it renders blank under the master's span). For each master:
///
///   1. the full span gets an opaque `content_bg` fill, painting over the
///      interior grid separators drawn through the covered cells;
///   2. the master's text is laid out across the spanned width, vertically
///      centered in the spanned height;
///   3. the span's outer border is stroked in `grid_line` so the merged block
///      reads as one cell.
///
/// Call *after* [`draw_cells_on`] + [`draw_grid_lines_on`]. Returns the number
/// of masters that had non-empty text drawn (font-independent tally).
#[allow(clippy::too_many_arguments)]
pub fn draw_merges_on<'a, S, I>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    merges: I,
) -> u32
where
    S: DrawSurface,
    I: IntoIterator<Item = (u32, u32, u32, u32, CellContent<'a>)>,
{
    let mut drawn = 0u32;

    for (row, col, col_span, row_span, content) in merges {
        let rect = layout.merge_rect(row, col, col_span, row_span);

        // 1. Fill the whole span with the master's fill (or the themed bg),
        // hiding the interior separators drawn through the covered cells.
        let bg = content.bg.map(rgb).unwrap_or(palette.content_bg);
        surface.fill_rect(rect, bg);
        // Re-stroke the span perimeter in the grid-line colour so the block reads
        // as one cell (the fill above paints over the perimeter separators). A
        // user-set border draws on top of this in the dedicated border pass.
        surface.stroke_rect(rect, palette.grid_line, 1.0);

        if text_enabled && !content.text.is_empty() {
            // Honour the master's full style (size, font, colour, alignment,
            // underline/strike) exactly like a plain cell, but always clipped to
            // and aligned within the span rect.
            let avail = (rect.width() - 2.0 * CELL_PAD_X).max(0.0);
            let font_size = content.size.map(|s| s as f32).unwrap_or(BODY_SIZE);
            let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build_font(
                font_cx,
                content.text,
                font_size,
                avail as f32,
                Alignment::Start,
                content.bold,
                content.italic,
                content.font,
            );
            let text_h = layout_text.height() as f64;
            let text_w = layout_text.width() as f64;
            let y_off = cell_v_offset(content.v_align, rect.height(), text_h);
            let x_off = match cell_alignment(content.h_align, content.text) {
                Alignment::Center => ((avail - text_w) / 2.0).max(0.0),
                Alignment::End | Alignment::Right => (avail - text_w).max(0.0),
                _ => 0.0,
            };
            let brush = Brush::Solid(content.fg.map(rgb).unwrap_or(palette.cell_text));
            surface.push_clip(rect);
            for line in layout_text.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let tx = rect.x0 + CELL_PAD_X + x_off + run.offset() as f64;
                    let ty = rect.y0 + y_off + run.baseline() as f64;
                    surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                    let dec = content.fg.map(rgb).unwrap_or(palette.cell_text);
                    let adv = run.advance() as f64;
                    if content.underline {
                        surface.stroke_line(tx, ty + 2.0, tx + adv, ty + 2.0, dec, 1.0);
                    }
                    if content.strikethrough {
                        let sy = ty - (font_size as f64) * 0.3;
                        surface.stroke_line(tx, sy, tx + adv, sy, dec, 1.0);
                    }
                }
            }
            surface.pop_clip();
            drawn += 1;
        }
    }

    drawn
}

// -----------------------------------------------------------------------------
// draw_col_headers_on / draw_row_headers_on -- fixed header strips
// -----------------------------------------------------------------------------

/// Draw the top column-header strip: opaque background, the active-column tint
/// (when `active_col` is visible), the bottom separator, then the A/B/C labels.
///
/// `active_col == None` (PDF) skips the highlight.
#[allow(clippy::too_many_arguments)]
pub fn draw_col_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    active_col: Option<u32>,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
) {
    let w = layout.width;
    let hh = layout.header_height;
    let top = layout.top;

    // Opaque background over the full top strip (corner included).
    surface.fill_rect(Rect::new(0.0, top, w, top + hh), palette.header_bg);

    // Active-column tint, over the bg and under the labels.
    if let Some(col) = active_col
        && col >= layout.origin_col
    {
        let x = layout.header_width + (col - layout.origin_col) as f64 * layout.col_width;
        if x < w {
            surface.fill_rect(
                Rect::new(x, top, x + layout.col_width, top + hh),
                palette.header_active,
            );
        }
    }

    // Bottom separator.
    surface.stroke_line(0.0, top + hh, w, top + hh, palette.header_border, 1.0);

    if !text_enabled {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let mut i = 0u32;
    loop {
        let col = layout.origin_col + i;
        if col >= layout.total_cols {
            break;
        }
        let col_x = layout.header_width + i as f64 * layout.col_width;
        if col_x >= w {
            break;
        }
        let label = col_label(col);
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx, &label, HEADER_SIZE, layout.col_width as f32, Alignment::Center, false, false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((hh - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > hh;
        if needs_clip {
            surface.push_clip(Rect::new(col_x, top, col_x + layout.col_width, top + hh));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = col_x + run.offset() as f64;
                let ty = top + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        i += 1;
    }
}

/// Draw the left row-header strip: opaque background, the active-row tint (when
/// `active_row` is visible), the right separator, then the 1/2/3 numbers.
///
/// `active_row == None` (PDF) skips the highlight.
#[allow(clippy::too_many_arguments)]
pub fn draw_row_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    active_row: Option<u32>,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
) {
    let h = layout.height;
    let hw = layout.header_width;
    let top = layout.top;
    let cells_top = top + layout.header_height;

    // Opaque background over the full left strip (below the formula bar band).
    surface.fill_rect(Rect::new(0.0, top, hw, h), palette.header_bg);

    // Active-row tint, over the bg and under the numbers. Positioned in collapsed
    // (filtered) row space; skipped when the active row is itself hidden.
    if let Some(row) = active_row
        && row >= layout.origin_row
        && !layout.is_hidden(row)
    {
        let y = cells_top + layout.visible_offset(row) as f64 * layout.row_height;
        if y < h {
            surface.fill_rect(Rect::new(0.0, y, hw, y + layout.row_height), palette.header_active);
        }
    }

    // Right separator.
    surface.stroke_line(hw, top, hw, h, palette.header_border, 1.0);

    if !text_enabled {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let max_width = (hw - 8.0) as f32;
    let mut i = 0u32;
    loop {
        // The label at visible slot `i` is the i-th non-hidden row from the
        // origin: filtered rows are collapsed out of the number strip too.
        let Some(row) = layout.row_at_slot(i) else {
            break;
        };
        let row_y = cells_top + i as f64 * layout.row_height;
        if row_y >= h {
            break;
        }
        let label = (row + 1).to_string();
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx, &label, HEADER_SIZE, max_width, Alignment::End, false, false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((layout.row_height - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > layout.row_height;
        if needs_clip {
            surface.push_clip(Rect::new(0.0, row_y, hw, row_y + layout.row_height));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = 4.0 + run.offset() as f64;
                let ty = row_y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        i += 1;
    }
}

// -----------------------------------------------------------------------------
// Headless tests -- mock DrawSurface.
// -----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use ambre_draw::FontContext;

    /// 800x600 surface, uniform 80x20 cells, 50x24 headers, origin at (0,0).
    fn layout() -> GridLayout<'static> {
        GridLayout {
            hidden_rows: &[],
            header_width: 50.0,
            header_height: 24.0,
            top: 0.0,
            col_width: 80.0,
            row_height: 20.0,
            origin_col: 0,
            origin_row: 0,
            width: 800.0,
            height: 600.0,
            total_cols: 1000,
            total_rows: 100_000,
        }
    }

    fn harness() -> (RecordingSurface, FontContext, TextCache) {
        (RecordingSurface::new(), FontContext::new(), TextCache::new(1.0))
    }

    #[test]
    fn cell_rect_maps_index_to_pixels() {
        let l = layout();
        let r = l.cell_rect(0, 0);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
        let r = l.cell_rect(2, 1);
        assert_eq!((r.x0, r.y0), (50.0 + 80.0, 24.0 + 2.0 * 20.0));
    }

    #[test]
    fn cell_rect_collapses_over_hidden_rows() {
        let mut l = layout();
        // Rows 2 and 3 are filtered out (sorted slice).
        l.hidden_rows = &[2, 3];
        assert!(l.is_hidden(2) && l.is_hidden(3) && !l.is_hidden(4));
        // Row 4 draws on the 3rd slot (rows 0,1 above; 2,3 collapsed).
        assert_eq!(l.visible_offset(4), 2);
        let r = l.cell_rect(4, 0);
        assert_eq!(r.y0, 24.0 + 2.0 * 20.0);
        // Slot 2 maps back to absolute row 4, not row 2.
        assert_eq!(l.row_at_slot(2), Some(4));
    }

    #[test]
    fn cell_rect_accounts_for_scroll_origin() {
        let mut l = layout();
        l.origin_col = 3;
        l.origin_row = 5;
        // The origin cell sits at the content top-left.
        let r = l.cell_rect(5, 3);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
    }

    #[test]
    fn draw_cells_on_fills_each_and_counts_nonempty_text() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        let cells = vec![
            (0, 0, CellContent::plain("hi", false, false)),
            (1, 1, CellContent::plain("", false, false)),
        ];
        let drawn = draw_cells_on(&mut s, &l, palette, &mut fc, &mut tc, true, cells);

        // Both cells get a content_bg fill...
        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.content_bg)),
            2,
        );
        // ...but only the non-empty one is tallied.
        assert_eq!(drawn, 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn draw_grid_lines_on_strokes_with_grid_color() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, _fc, _tc) = harness();

        draw_grid_lines_on(&mut s, &l, palette.grid_line, l.content_area());

        assert!(
            s.count(|op| matches!(op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line)) > 0
        );
    }

    #[test]
    fn col_headers_draw_bg_separator_and_active_tint() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        draw_col_headers_on(&mut s, &l, palette, Some(0), &mut fc, &mut tc, true);

        // The full-width header strip.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        // The active-column tint.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_active
        )) >= 1);
        // The bottom separator.
        assert!(s.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.header_border
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn row_headers_skip_tint_when_inactive() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        // No active row -> no header_active fill, just bg + separator.
        draw_row_headers_on(&mut s, &l, palette, None, &mut fc, &mut tc, true);

        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.header_active)),
            0,
        );
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn col_label_matches_spreadsheet_convention() {
        assert_eq!(col_label(0), "A");
        assert_eq!(col_label(25), "Z");
        assert_eq!(col_label(26), "AA");
        assert_eq!(col_label(701), "ZZ");
        assert_eq!(col_label(702), "AAA");
    }
}

// -----------------------------------------------------------------------------
// Formula bar action types (shared between the UI thread and the Leptos chrome)
// -----------------------------------------------------------------------------

/// Direction the active cell moves after a formula bar commit via Enter or Tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommitDirection {
    Down,
    Right,
    Stay,
}

/// Action emitted by the HTML formula bar input: commit the edit with the
/// cursor-navigation direction, fill the whole selection with the value
/// (Ctrl/Cmd+Enter), or cancel it without writing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FormulaAction {
    Commit(CommitDirection),
    /// Ctrl/Cmd+Enter: write the value to every cell of the current selection,
    /// keeping the selection and active cell put.
    CommitFill,
    Cancel,
}
