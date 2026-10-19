//! The pages' document walk (`CANVAS-074`).
//!
//! The walk draws what a page shows of the document: the glyphs, the inline
//! highlights, the tables, the bands, the footnotes and their separator. It
//! reads a [`PageView`] the screen resolves before the call, in column
//! coordinates: the pages stacked down the column at zoom 1. It knows no
//! scroll position, no zoom, no caret and no selection. The screen draws the
//! sheets and the selections before the walk, and the carets and the
//! composition after it.

use ambre_draw::{Affine, Brush, Color, DocumentWalk, DrawSurface, PositionedLayoutItem, Rect};

use crate::layout::run::{CellStyle, ParleyRunData, ShapedRun};
use crate::view::cache::{RunCache, RunKey};
use crate::view::{Highlight, HighlightKind};

/// The colours a page is painted with, filled by the host from the generated
/// theme tokens (`ambre-spec-design-tokens.md`). The painter holds no colour
/// of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePalette {
    /// The sheet, its border and its shadow: the `surfaces` group.
    pub sheet: Color,
    pub sheet_border: Color,
    pub shadow: Color,
    /// The surface's foreground: body text, bands and footnotes.
    pub text: Color,
    /// The caret, and the local selection at a fifth.
    pub accent: Color,
    /// A comment highlight, at two fifths.
    pub warning: Color,
    /// A suggested insertion, at three tenths.
    pub success: Color,
    /// A suggested deletion, at three tenths and struck through.
    pub error: Color,
}

impl PagePalette {
    pub(crate) fn highlight(&self, kind: HighlightKind) -> Color {
        match kind {
            HighlightKind::Comment => self.warning.multiply_alpha(0.4),
            HighlightKind::Insertion => self.success.multiply_alpha(0.3),
            HighlightKind::Deletion => self.error.multiply_alpha(0.3),
            HighlightKind::Find => self.accent.multiply_alpha(0.3),
        }
    }
}

/// Where a run is drawn on its page: its origin, and the clip of the part
/// the page shows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub origin_x: f64,
    pub origin_y: f64,
    pub clip: Rect,
}

/// A box of a run, moved to where the run is drawn.
pub(crate) fn to_rect(placed: &Placed, rect: &ambre_draw::BoundingBox) -> Rect {
    Rect::new(
        placed.origin_x + rect.x0,
        placed.origin_y + rect.y0,
        placed.origin_x + rect.x1,
        placed.origin_y + rect.y1,
    )
}

/// A text the view draws on a line ahead of the layout: the composition.
/// The glyphs after its point move right by its width.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Insertion {
    pub line: usize,
    pub x: f64,
    pub width: f64,
}

/// One block of text a page shows.
#[derive(Clone, Debug)]
pub(crate) struct BlockPaint {
    /// The key its drawing state is cached under.
    pub key: RunKey,
    pub run: ShapedRun,
    pub placed: Placed,
    /// The composition the glyphs after it make room for.
    pub insertion: Option<Insertion>,
    /// Its inline highlights: comments, suggestions, find matches.
    pub highlights: Vec<Highlight>,
}

/// One table cell: its box, then the blocks it holds.
#[derive(Clone, Debug)]
pub(crate) struct CellPaint {
    pub style: CellStyle,
    pub bounds: Rect,
    pub blocks: Vec<BlockPaint>,
}

/// One thing a page draws, in page coordinates.
#[derive(Clone, Debug)]
pub(crate) enum PageItem {
    /// A block of text.
    Block(BlockPaint),
    /// A table, clipped to the part of it the page shows.
    Table { clip: Rect, cells: Vec<CellPaint> },
    /// A text shaped ahead of the layout and owned by no block yet: the
    /// paragraph a split opens.
    Shape { run: ShapedRun, placed: Placed },
    /// The separator above the footnotes.
    Rule { x0: f64, x1: f64, y: f64 },
}

/// One page as the walk draws it.
#[derive(Clone, Debug)]
pub(crate) struct PagePaint {
    /// The top of the page down the column.
    pub top: f64,
    /// What it shows, in the order it is drawn.
    pub items: Vec<PageItem>,
}

/// The pages as the walk reads them.
pub struct PageView<'v> {
    pub(crate) palette: &'v PagePalette,
    /// The size of a page.
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) pages: &'v [PagePaint],
}

/// The pages' walk: the drawing state of the runs it keeps between calls.
pub struct PageWalk<'t> {
    pub(crate) runs: &'t mut RunCache,
    /// The highlights the last call drew.
    pub(crate) highlights_walked: u64,
}

impl DocumentWalk for PageWalk<'_> {
    type View<'v> = PageView<'v>;

    fn walk<S: DrawSurface>(&mut self, view: &PageView<'_>, region: Rect, surface: &mut S) {
        self.highlights_walked = 0;
        surface.push_clip(region);
        for page in view.pages {
            let bounds = Rect::new(0.0, page.top, view.width, page.top + view.height);
            let hit = bounds.intersect(region);
            if hit.width() <= 0.0 || hit.height() <= 0.0 {
                continue;
            }
            surface.push_transform(Affine::translate((0.0, page.top)));
            for item in &page.items {
                self.item(surface, view.palette, item);
            }
            surface.pop_transform();
        }
        surface.pop_clip();
    }
}

impl PageWalk<'_> {
    fn item<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette, item: &PageItem) {
        match item {
            PageItem::Block(block) => self.block(surface, palette, block),
            PageItem::Table { clip, cells } => {
                surface.push_clip(*clip);
                for cell in cells {
                    paint_cell_box(surface, palette, &cell.style, cell.bounds);
                    for block in &cell.blocks {
                        self.block(surface, palette, block);
                    }
                }
                surface.pop_clip();
            }
            PageItem::Shape { run, placed } => {
                surface.push_clip(placed.clip);
                draw_shape(surface, run, placed.origin_x, placed.origin_y, palette.text);
                surface.pop_clip();
            }
            PageItem::Rule { x0, x1, y } => {
                surface.stroke_line(*x0, *y, *x1, *y, palette.sheet_border, 1.0);
            }
        }
    }

    /// A block's glyphs, then its inline highlights over them.
    fn block<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        palette: &PagePalette,
        block: &BlockPaint,
    ) {
        let data = block.run.data();
        let placed = &block.placed;
        surface.push_clip(placed.clip);
        draw_run(
            surface,
            self.runs,
            block.key,
            &block.run,
            placed,
            palette.text,
            block.insertion.as_slice(),
        );
        for mark in &block.highlights {
            self.highlights_walked = self.highlights_walked.saturating_add(1);
            for rect in data.range_boxes(mark.start, mark.end) {
                let rect = to_rect(placed, &rect);
                surface.fill_rect(rect, palette.highlight(mark.kind));
                // A suggestion reads in the text itself: an insertion is
                // underlined, a deletion struck through (`PROSE-UI-051`).
                match mark.kind {
                    HighlightKind::Deletion => {
                        let middle = (rect.y0 + rect.y1) / 2.0;
                        surface.stroke_line(rect.x0, middle, rect.x1, middle, palette.error, 1.0);
                    }
                    HighlightKind::Insertion => {
                        let base = rect.y1 - 1.0;
                        surface.stroke_line(rect.x0, base, rect.x1, base, palette.success, 1.0);
                    }
                    HighlightKind::Comment | HighlightKind::Find => {}
                }
            }
        }
        surface.pop_clip();
    }
}

/// Far enough past any page that a clip band never cuts a glyph vertically.
const BAND: f64 = 1.0e7;

/// Draw the glyphs of a run, building its drawing state on its first paint.
///
/// A line that carries insertions is drawn in slices, each clipped at an
/// insertion point and moved right by the widths before it (`PROSE-UI-199`).
fn draw_run<S: DrawSurface>(
    surface: &mut S,
    cache: &mut RunCache,
    key: RunKey,
    run: &ShapedRun,
    placed: &Placed,
    color: Color,
    insertions: &[Insertion],
) {
    let data: &ParleyRunData = run.data();
    let brush = Brush::Solid(color);
    for (line_index, item_index, _) in &cache.draw_state(key, run).glyph_runs {
        let Some((line, shift)) = data.line(*line_index) else {
            continue;
        };
        // A line shaped in a later segment is moved down to its place
        // (`PROSE-128`).
        let origin_y = placed.origin_y + f64::from(shift);
        let transform = Affine::translate((placed.origin_x, origin_y));
        let Some(PositionedLayoutItem::GlyphRun(glyphs)) = line.items().nth(*item_index) else {
            continue;
        };
        let mut cuts = insertions
            .iter()
            .filter(|cut| cut.line == *line_index)
            .peekable();
        if cuts.peek().is_none() {
            surface.draw_glyphs(&glyphs, &brush, transform);
            draw_lines(surface, &glyphs, placed.origin_x, origin_y, color);
            continue;
        }
        let (mut from, mut shift) = (-BAND, 0.0);
        let slice = |surface: &mut S, from: f64, to: f64, shift: f64| {
            let left = placed.origin_x + from + shift;
            let right = placed.origin_x + to + shift;
            surface.push_clip(Rect::new(left, -BAND, right, BAND));
            let moved = Affine::translate((placed.origin_x + shift, origin_y));
            surface.draw_glyphs(&glyphs, &brush, moved);
            draw_lines(surface, &glyphs, placed.origin_x + shift, origin_y, color);
            surface.pop_clip();
        };
        for cut in cuts {
            if cut.x > from {
                slice(surface, from, cut.x, shift);
                from = cut.x;
            }
            shift += cut.width;
        }
        slice(surface, from, BAND, shift);
    }
}

/// Draw the underline and the strike-through a glyph run carries, at the
/// offsets and thicknesses its font gives (`PROSE-UI-209`).
fn draw_lines<S: DrawSurface>(
    surface: &mut S,
    glyphs: &parley::GlyphRun<'_, ambre_draw::ParleyBrush>,
    x: f64,
    y: f64,
    color: Color,
) {
    let style = glyphs.style();
    let metrics = glyphs.run().font_metrics();
    let lines = [
        (
            style.underline.as_ref(),
            metrics.underline_offset,
            metrics.underline_size,
        ),
        (
            style.strikethrough.as_ref(),
            metrics.strikethrough_offset,
            metrics.strikethrough_size,
        ),
    ];
    let left = x + f64::from(glyphs.offset());
    let right = left + f64::from(glyphs.advance());
    for (line, offset, size) in lines {
        let Some(line) = line else {
            continue;
        };
        let size = line.size.unwrap_or(size);
        let offset = line.offset.unwrap_or(offset);
        let at = y + f64::from(glyphs.baseline() - offset + size / 2.0);
        surface.stroke_line(left, at, right, at, color, f64::from(size));
    }
}

/// Draw every glyph of a shaped text at `(x, y)`.
pub(crate) fn draw_shape<S: DrawSurface>(
    surface: &mut S,
    shape: &ShapedRun,
    x: f64,
    y: f64,
    color: Color,
) {
    let brush = Brush::Solid(color);
    for (line, shift) in shape.data().lines() {
        let y = y + f64::from(shift);
        let transform = Affine::translate((x, y));
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyphs) = item {
                surface.draw_glyphs(&glyphs, &brush, transform);
                draw_lines(surface, &glyphs, x, y, color);
            }
        }
    }
}

/// A cell's fill, then its border. A stored border draws each edge at its own
/// width, inside the cell; none draws the theme's hairline (`PROSE-UI-179`,
/// `PROSE-UI-181`).
fn paint_cell_box<S: DrawSurface>(
    surface: &mut S,
    palette: &PagePalette,
    style: &CellStyle,
    bounds: Rect,
) {
    if let Some(fill) = style.fill {
        surface.fill_rect(bounds, packed(fill));
    }
    let Some(border) = style.border else {
        surface.stroke_rect(bounds, palette.sheet_border, 1.0);
        return;
    };
    let colour = border.color.map_or(palette.sheet_border, packed);
    let (top, bottom) = (f64::from(border.top), f64::from(border.bottom));
    let (left, right) = (f64::from(border.left), f64::from(border.right));
    let edges = [
        Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + top),
        Rect::new(bounds.x0, bounds.y1 - bottom, bounds.x1, bounds.y1),
        Rect::new(bounds.x0, bounds.y0, bounds.x0 + left, bounds.y1),
        Rect::new(bounds.x1 - right, bounds.y0, bounds.x1, bounds.y1),
    ];
    for edge in edges {
        if edge.width() > 0.0 && edge.height() > 0.0 {
            surface.fill_rect(edge, colour);
        }
    }
}

/// A colour packed as `0xRRGGBBAA`.
fn packed(rgba: u32) -> Color {
    let [r, g, b, a] = rgba.to_be_bytes();
    Color::from_rgba8(r, g, b, a)
}
