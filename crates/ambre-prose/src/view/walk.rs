//! The pages' document walk (`CANVAS-074`).
//!
//! The walk draws what a page shows of the document: the glyphs, the inline
//! highlights, the tables, the bands, the footnotes and their separator. It
//! reads a [`PageView`] the screen resolves before the call, in column
//! coordinates: the pages stacked down the column at zoom 1. It knows no
//! scroll position, no zoom, no caret and no selection. The screen draws the
//! sheets and the selections before the walk, and the carets and the
//! composition after it.

use ambre_core::Rgba;
use ambre_core::index::{Pos, Span};
use ambre_core::{Count, Px, Surface};
use ambre_draw::{
    Affine, Brush, Color, DocumentWalk, DrawSurface, LayoutText, Point, PositionedLayoutItem, Rect,
};

use crate::layout::run::{CellStyle, ParleyRunData, RunLine, ShapedRun};
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
            HighlightKind::Misspelled => self.error,
        }
    }
}

/// Where a run is drawn on its page: its origin on the surface, and the clip
/// of the part the page shows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub origin: Point,
    pub clip: Rect,
}

/// A box of a run, moved to where the run is drawn.
pub(crate) fn to_rect(placed: &Placed, rect: &ambre_draw::BoundingBox) -> Rect {
    Rect::new(
        placed.origin.x + rect.x0,
        placed.origin.y + rect.y0,
        placed.origin.x + rect.x1,
        placed.origin.y + rect.y1,
    )
}

/// A text the view draws on a line ahead of the layout: the composition.
/// The glyphs after its point move right by its width.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Insertion {
    pub line: Pos<RunLine>,
    /// Where the text goes in, from the start of the run.
    pub x: Px<Surface>,
    /// How far the glyphs after it move right.
    pub width: Px<Surface>,
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
    /// A picture: its box, then the texts drawn over the placeholder and
    /// under the box (`features/ambre-spec-prose-images.md section 4`).
    Picture(PicturePaint),
    /// The separator above the footnotes.
    Rule {
        x0: Px<Surface>,
        x1: Px<Surface>,
        y: Px<Surface>,
    },
}

/// One picture as a page draws it.
#[derive(Clone, Debug)]
pub(crate) struct PicturePaint {
    /// The address its samples are looked up by.
    pub address: Option<ambre_core::Validated<ambre_core::ContentHash>>,
    /// The box, in page coordinates.
    pub frame: Rect,
    /// The alternative text, drawn inside the placeholder and the broken mark.
    pub alt: Option<(ShapedRun, Placed)>,
    /// The caption, drawn under the box.
    pub caption: Option<(ShapedRun, Placed)>,
}

/// One page as the walk draws it.
#[derive(Clone, Debug)]
pub(crate) struct PagePaint {
    /// The top of the page down the column.
    pub top: Px<Surface>,
    /// What it shows, in the order it is drawn.
    pub items: Vec<PageItem>,
}

/// The pages as the walk reads them.
pub struct PageView<'v> {
    pub(crate) palette: &'v PagePalette,
    /// The size of a page.
    pub(crate) width: Px<Surface>,
    pub(crate) height: Px<Surface>,
    pub(crate) pages: &'v [PagePaint],
    /// What each picture address shows now.
    pub(crate) pictures: &'v PictureStore,
}

/// What each picture address shows, as the view or a print holds it.
pub(crate) type PictureStore = std::collections::HashMap<
    ambre_core::Validated<ambre_core::ContentHash>,
    crate::message::PictureShown,
>;

/// The pages' walk: the drawing state of the runs it keeps between calls.
pub struct PageWalk<'t> {
    pub(crate) runs: &'t mut RunCache,
    /// The highlights the last call drew.
    pub(crate) highlights_walked: Count<Highlights>,
}

/// How many highlights a paint read.
pub enum Highlights {}

impl ambre_core::NarrowCount for Highlights {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("HighlightCount");
}

impl DocumentWalk for PageWalk<'_> {
    type View<'v> = PageView<'v>;

    fn walk<S: DrawSurface>(&mut self, view: &PageView<'_>, region: Rect, surface: &mut S) {
        self.highlights_walked = Count::<Highlights>::ZERO;
        surface.push_clip(region);
        for page in view.pages {
            let bounds = Rect::new(
                0.0,
                page.top.get(),
                view.width.get(),
                (page.top + view.height).get(),
            );
            let hit = bounds.intersect(region);
            if hit.width() <= 0.0 || hit.height() <= 0.0 {
                continue;
            }
            surface.push_transform(Affine::translate((0.0, page.top.get())));
            for item in &page.items {
                self.item(surface, view, item);
            }
            surface.pop_transform();
        }
        surface.pop_clip();
    }
}

impl PageWalk<'_> {
    fn item<S: DrawSurface>(&mut self, surface: &mut S, view: &PageView<'_>, item: &PageItem) {
        let palette = view.palette;
        match item {
            PageItem::Picture(picture) => draw_picture(surface, palette, view.pictures, picture),
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
                draw_shape(surface, run, placed.origin, palette.text);
                surface.pop_clip();
            }
            PageItem::Rule { x0, x1, y } => {
                surface.stroke_line(
                    ambre_draw::Line::new((x0.get(), y.get()), (x1.get(), y.get())),
                    palette.sheet_border,
                    ambre_core::Px::<ambre_core::Surface>::new(1.0),
                );
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
            self.highlights_walked = self.highlights_walked.incremented();
            for rect in data.range_boxes(mark.span) {
                let rect = to_rect(placed, &rect);
                // A misspelled word keeps its text bare: the wave alone
                // marks it (`PROSE-UI-247`).
                if mark.kind == HighlightKind::Misspelled {
                    wave(surface, palette.highlight(mark.kind), &rect);
                    continue;
                }
                surface.fill_rect(rect, palette.highlight(mark.kind));
                // A suggestion reads in the text itself: an insertion is
                // underlined, a deletion struck through (`PROSE-UI-051`).
                match mark.kind {
                    HighlightKind::Deletion => {
                        let middle = (rect.y0 + rect.y1) / 2.0;
                        surface.stroke_line(
                            ambre_draw::Line::new((rect.x0, middle), (rect.x1, middle)),
                            palette.error,
                            ambre_core::Px::<ambre_core::Surface>::new(1.0),
                        );
                    }
                    HighlightKind::Insertion => {
                        let base = rect.y1 - 1.0;
                        surface.stroke_line(
                            ambre_draw::Line::new((rect.x0, base), (rect.x1, base)),
                            palette.success,
                            ambre_core::Px::<ambre_core::Surface>::new(1.0),
                        );
                    }
                    HighlightKind::Comment | HighlightKind::Find | HighlightKind::Misspelled => {}
                }
            }
        }
        surface.pop_clip();
    }
}

/// The pitch of the wave under a misspelled word: half a period, in pixels
/// of the surface, and its height.
const WAVE_STEP: Px<Surface> = Px::<Surface>::new(2.0);

/// Draw a wavy line along the bottom of `rect`, one stroke per half period,
/// so a misspelled word reads apart from an underlined one.
fn wave<S: DrawSurface>(surface: &mut S, color: Color, rect: &Rect) {
    let step = WAVE_STEP.get();
    let base = rect.y1 - 1.0;
    let mut x = rect.x0;
    let mut up = true;
    while x < rect.x1 {
        let next = (x + step).min(rect.x1);
        let (from, to) = if up {
            (base, base - step)
        } else {
            (base - step, base)
        };
        surface.stroke_line(
            ambre_draw::Line::new((x, from), (next, to)),
            color,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        x = next;
        up = !up;
    }
}

/// Draw one picture: its samples once decoded, a neutral placeholder while it
/// waits, or a broken mark once it failed for good (`PROSE-UI-046`,
/// `PROSE-UI-146`). The placeholder and the mark carry the alternative text,
/// and the caption sits under the box in every state.
fn draw_picture<S: DrawSurface>(
    surface: &mut S,
    palette: &PagePalette,
    pictures: &PictureStore,
    picture: &PicturePaint,
) {
    let frame = picture.frame;
    let shown = picture
        .address
        .as_ref()
        .and_then(|address| pictures.get(address).map(|shown| (address, shown)));
    match shown {
        Some((address, crate::message::PictureShown::Ready(decoded))) => {
            surface.draw_image(
                &ambre_draw::ImageSource {
                    key: ambre_draw::ImageKey::from_address(address),
                    pixels: &decoded.pixels,
                    size: decoded.size,
                },
                frame,
            );
        }
        Some((address, crate::message::PictureShown::Printed(file))) => {
            let image = ambre_draw::EncodedImage {
                key: ambre_draw::ImageKey::from_address(address),
                format: file.format,
                bytes: &file.bytes,
                size: file.size,
            };
            // The file is drawn upright: its unit square goes to the box
            // through the turn its orientation states (`PROSE-FEAT-326`).
            surface.push_transform(oriented(frame, file.orientation));
            surface.draw_encoded_image(&image, Rect::new(0.0, 0.0, 1.0, 1.0));
            surface.pop_transform();
        }
        Some((_, crate::message::PictureShown::Failed(_))) => {
            surface.fill_rect(frame, palette.error.multiply_alpha(0.08));
            surface.stroke_rect(frame, palette.error, Px::<Surface>::new(1.0));
            // The broken mark: a cross over the box, which no loading
            // placeholder draws (`PROSE-UI-046`).
            for (from, to) in [
                ((frame.x0, frame.y0), (frame.x1, frame.y1)),
                ((frame.x0, frame.y1), (frame.x1, frame.y0)),
            ] {
                surface.stroke_line(
                    ambre_draw::Line::new(from, to),
                    palette.error,
                    Px::<Surface>::new(1.0),
                );
            }
            draw_alt(surface, palette, picture);
        }
        Some((_, crate::message::PictureShown::Waiting)) | None => {
            surface.fill_rect(frame, palette.text.multiply_alpha(0.06));
            surface.stroke_rect(frame, palette.sheet_border, Px::<Surface>::new(1.0));
            draw_alt(surface, palette, picture);
        }
    }
    if let Some((run, placed)) = &picture.caption {
        surface.push_clip(placed.clip);
        draw_shape(surface, run, placed.origin, palette.text);
        surface.pop_clip();
    }
}

/// The map from the unit square of a stored picture to `frame`, the
/// picture turned as `turn` states. A stored point `(u, v)`, both in
/// `0..=1`, lands where the shown picture puts it.
fn oriented(frame: Rect, turn: crate::message::Orientation) -> Affine {
    use crate::message::Orientation;
    let (x, y, w, h) = (frame.x0, frame.y0, frame.width(), frame.height());
    Affine::new(match turn {
        Orientation::Upright => [w, 0.0, 0.0, h, x, y],
        Orientation::FlipHorizontal => [-w, 0.0, 0.0, h, x + w, y],
        Orientation::Rotate180 => [-w, 0.0, 0.0, -h, x + w, y + h],
        Orientation::FlipVertical => [w, 0.0, 0.0, -h, x, y + h],
        Orientation::Rotate90 => [0.0, h, -w, 0.0, x + w, y],
        Orientation::Rotate270 => [0.0, -h, w, 0.0, x, y + h],
        Orientation::Transpose => [0.0, h, w, 0.0, x, y],
        Orientation::Transverse => [0.0, -h, -w, 0.0, x + w, y + h],
    })
}

/// The alternative text inside a box that shows no samples.
fn draw_alt<S: DrawSurface>(surface: &mut S, palette: &PagePalette, picture: &PicturePaint) {
    if let Some((run, placed)) = &picture.alt {
        surface.push_clip(placed.clip.intersect(picture.frame));
        draw_shape(surface, run, placed.origin, palette.text);
        surface.pop_clip();
    }
}

/// Far enough past any page that a clip band never cuts a glyph vertically.
const BAND: Px<Surface> = Px::<Surface>::new(1.0e7);

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
    // The glyph runs of a line follow each other in the order of its items,
    // so one walk of the items reaches them all. Parley walks a line from
    // its start to reach any item. A reach per glyph run then costs the
    // square of the style changes on the line.
    for on_line in cache
        .draw_state(key, run)
        .glyph_runs
        .chunk_by(|a, b| a.line == b.line)
    {
        let Some(first) = on_line.first() else {
            continue;
        };
        let Some((line, shift)) = data.line(first.line) else {
            continue;
        };
        let text = data.line_text(first.line).unwrap_or(LayoutText::none());
        // A line shaped in a later segment is moved down to its place
        // (`PROSE-128`).
        let origin = placed.origin + (0.0, f64::from(shift.get()));
        let mut items = line.items().enumerate();
        for at in on_line {
            let Some((_, PositionedLayoutItem::GlyphRun(glyphs))) =
                items.find(|(index, _)| *index == at.item.to_usize())
            else {
                continue;
            };
            let mut cuts = insertions
                .iter()
                .filter(|cut| cut.line == at.line)
                .peekable();
            if cuts.peek().is_none() {
                surface.draw_glyphs(&glyphs, text, &brush, run_at(&glyphs, origin));
                draw_lines(surface, &glyphs, origin, color);
                continue;
            }
            let band = BAND.get();
            let (mut from, mut shift) = (-band, 0.0);
            let slice = |surface: &mut S, from: f64, to: f64, shift: f64| {
                let left = origin.x + from + shift;
                let right = origin.x + to + shift;
                surface.push_clip(Rect::new(left, -band, right, band));
                let moved = origin + (shift, 0.0);
                surface.draw_glyphs(&glyphs, text, &brush, run_at(&glyphs, moved));
                draw_lines(surface, &glyphs, moved, color);
                surface.pop_clip();
            };
            for cut in cuts {
                let x = cut.x.get();
                if x > from {
                    slice(surface, from, x, shift);
                    from = x;
                }
                shift += cut.width.get();
            }
            slice(surface, from, band, shift);
        }
    }
}

/// Where a glyph run is drawn from: its line's `origin`, moved along the line
/// to the run and down to its baseline. `draw_glyphs` places each glyph from
/// that point, so a line top would draw the glyphs above their line.
fn run_at(glyphs: &parley::GlyphRun<'_, ambre_draw::ParleyBrush>, origin: Point) -> Affine {
    let at = origin + (f64::from(glyphs.offset()), f64::from(glyphs.baseline()));
    Affine::translate(at.to_vec2())
}

/// Draw the underline and the strike-through a glyph run carries, at the
/// offsets and thicknesses its font gives (`PROSE-UI-209`). The run's line
/// is drawn from `origin`.
fn draw_lines<S: DrawSurface>(
    surface: &mut S,
    glyphs: &parley::GlyphRun<'_, ambre_draw::ParleyBrush>,
    origin: Point,
    color: Color,
) {
    let style = glyphs.style();
    let metrics = glyphs.run().metrics();
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
    let left = origin.x + f64::from(glyphs.offset());
    let right = left + f64::from(glyphs.advance());
    for (line, offset, size) in lines {
        let Some(line) = line else {
            continue;
        };
        let size = line.size.unwrap_or(size);
        let offset = line.offset.unwrap_or(offset);
        let at = origin.y + f64::from(glyphs.baseline() - offset + size / 2.0);
        surface.stroke_line(
            ambre_draw::Line::new((left, at), (right, at)),
            color,
            ambre_core::Px::<ambre_core::Surface>::new(f64::from(size)),
        );
    }
}

/// Draw every glyph of a shaped text from `at`, its top-left corner.
pub(crate) fn draw_shape<S: DrawSurface>(
    surface: &mut S,
    shape: &ShapedRun,
    at: Point,
    color: Color,
) {
    let brush = Brush::Solid(color);
    let data = shape.data();
    for index in Span::from_len(Pos::<RunLine>::ZERO, data.line_count()) {
        let Some((line, shift)) = data.line(index) else {
            continue;
        };
        let text = data.line_text(index).unwrap_or(LayoutText::none());
        let origin = at + (0.0, f64::from(shift.get()));
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyphs) = item {
                surface.draw_glyphs(&glyphs, text, &brush, run_at(&glyphs, origin));
                draw_lines(surface, &glyphs, origin, color);
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
        surface.stroke_rect(
            bounds,
            palette.sheet_border,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        return;
    };
    let colour = border.color.map_or(palette.sheet_border, packed);
    let (top, bottom) = (f64::from(border.top.get()), f64::from(border.bottom.get()));
    let (left, right) = (f64::from(border.left.get()), f64::from(border.right.get()));

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
fn packed(rgba: Rgba) -> Color {
    let [r, g, b, a] = rgba.channels();
    Color::from_rgba8(r, g, b, a)
}
