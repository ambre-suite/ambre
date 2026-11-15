//! The PDF export of a sheet (`features/ambre-spec-trame-pdf-export.md
//! section 3`).
//!
//! The export cuts the printed range into pages at row and column edges. It
//! draws each page with the screen's own sheet walk onto a recorded page
//! (`PDF-001`). The PDF converter module writes the recorded pages. The walk
//! runs one page per slice, so the Logic thread serves the view between two
//! pages (`TRAME-IO-076`, `ARCH-030`).
//!
//! The export reads the document and writes nothing to it (`PDF-002`). Each
//! slice packs the cells of its own page through the viewport's packer, so a
//! page holds what the screen shows. A command other than a scroll or a
//! selection lands the running export whole first, and so does a remote
//! delta. The document therefore stands still for the length of the walk.

use std::collections::{BTreeMap, VecDeque};

use ambre_bus::interop::{
    ConversionProgress, Direction, ExportedFile, FileFormat, InteropAnswer, InteropError, Steps,
};
use ambre_canvas::{TextCache, Theme};
use ambre_convert::{Conversion, Converted, Converter};
use ambre_core::index::{Pos, Span};
use ambre_core::{Count, Mm, PrintLength, PrintUnit, Pt, Scale, StopCause, Surface, View};
use ambre_draw::list::{ListRecorder, PrintDocument, PrintPage, RecorderTables};
use ambre_draw::{Affine, DocumentWalk, DrawSurface, FontContext, Rect};

use super::*;
use crate::message::{PdfExport, PrintFit, PrintRange, TitleRow, TrameReport};
use crate::render::{
    CellContent, SheetAxes, SheetCell, SheetCells, SheetView, SheetWalk, TextPass,
};
use crate::ui::grid::CellData;
use crate::{COL_WIDTH, ROW_HEIGHT};

/// The size of the paper a sheet prints on: A4, portrait, until the sheet
/// stores its own page setup (`TRAME-IO-022`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PrintPaper {
    /// The width of the sheet of paper.
    pub(crate) width: PrintLength<Mm>,
    /// The height of the sheet of paper.
    pub(crate) height: PrintLength<Mm>,
}

/// The paper every export prints on.
pub(crate) const PRINT_PAPER: PrintPaper = PrintPaper {
    width: PrintLength::<Mm>::new(210.0),
    height: PrintLength::<Mm>::new(297.0),
};

/// The blank band on each edge of the paper: half an inch.
pub(crate) const PRINT_MARGIN: PrintLength<Mm> = PrintLength::<Mm>::new(12.7);

/// How much of a running export one call walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintPace {
    /// One page, then the call returns (`ARCH-030`).
    Slice,
    /// Every page left, then the file (`ARCH-031`).
    Whole,
}

/// One page of a printed range: the columns and the rows it holds, and the
/// row that heads it when the title row repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageCut {
    /// The columns of the page, a band of whole columns.
    pub(crate) cols: Span<Col>,
    /// The rows of the page, a band of whole rows.
    pub(crate) rows: Span<Row>,
    /// The title row drawn above the page's own rows, if any.
    pub(crate) title: Option<Pos<Row>>,
}

/// A range cut into pages, and the zoom every page draws at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pagination {
    /// The pixels of a page per pixel of the sheet.
    pub(crate) zoom: Scale<Doc, View>,
    /// The pages, down the rows first, then across the columns.
    pub(crate) pages: Vec<PageCut>,
}

/// The part of a page the cells may cover: the paper less its margins, in
/// document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PrintArea {
    /// The width the cells may cover.
    pub(crate) width: Px<Doc>,
    /// The height the cells may cover.
    pub(crate) height: Px<Doc>,
}

impl PrintArea {
    /// The area of `paper` inside `margin` on each edge.
    pub(crate) fn of(paper: PrintPaper, margin: PrintLength<Mm>) -> Self {
        let twice = margin + margin;
        Self {
            width: (paper.width - twice).to_doc_px(),
            height: (paper.height - twice).to_doc_px(),
        }
    }
}

/// A slack under which a band still fits: a sum of widths may miss an exact
/// fit by a rounding error.
const FIT_SLACK: Px<Surface> = Px::<Surface>::new(1e-6);

/// The room each band of an axis may fill.
#[derive(Clone, Copy)]
struct BandRoom {
    /// The room of the first band.
    first: Px<Surface>,
    /// The room of every later band.
    rest: Px<Surface>,
}

/// Cut `range` into pages of `area`, on `axes`, as `options` asks
/// (`TRAME-IO-072` to `TRAME-IO-074`).
///
/// A band of columns takes whole columns while it fits the width of the area.
/// A band of rows takes whole rows while it fits the height. A column or a row
/// too large for the area alone takes a page of its own, which clips it. A
/// hidden row takes no height and never opens a page.
pub(crate) fn paginate(
    range: CellRect,
    axes: &SheetAxes<'_>,
    area: PrintArea,
    options: PdfExport,
) -> Pagination {
    let col_width = |col: Pos<Col>| {
        Px::<Surface>::new(f64::from(
            axes.col_widths
                .get(&col)
                .copied()
                .unwrap_or(axes.col_width)
                .get(),
        ))
    };
    let row_height = |row: Pos<Row>| {
        if axes.is_hidden(row) {
            Px::<Surface>::ZERO
        } else {
            Px::<Surface>::new(f64::from(
                axes.row_heights
                    .get(&row)
                    .copied()
                    .unwrap_or(axes.row_height)
                    .get(),
            ))
        }
    };
    let total_width: Px<Surface> = range.cols().into_iter().map(col_width).sum();
    let area_width = Px::<Surface>::new(f64::from(area.width.get()));
    let zoom = match options.fit {
        PrintFit::Width if total_width > area_width => area_width.get() / total_width.get(),
        PrintFit::ActualSize | PrintFit::Width => 1.0,
    };
    let room_width = Px::<Surface>::new(area_width.get() / zoom);
    let room_height = Px::<Surface>::new(f64::from(area.height.get()) / zoom);

    let first = range.top_left.row;
    let title_height = row_height(first);
    // The title row repeats only where it leaves room for a row of the page.
    let title = (options.title_row == TitleRow::Repeated
        && range.row_count().get() > 1
        && title_height > Px::<Surface>::ZERO
        && title_height < room_height)
        .then_some(first);

    let col_bands = bands(
        range.cols(),
        col_width,
        BandRoom {
            first: room_width,
            rest: room_width,
        },
    );
    let row_bands = bands(
        range.rows(),
        row_height,
        BandRoom {
            first: room_height,
            rest: match title {
                Some(_) => room_height - title_height,
                None => room_height,
            },
        },
    );
    let pages = col_bands
        .iter()
        .flat_map(|&cols| {
            row_bands
                .iter()
                .enumerate()
                .map(move |(index, &rows)| PageCut {
                    cols,
                    rows,
                    title: title.filter(|_| index > 0),
                })
        })
        .collect();
    Pagination {
        zoom: Scale::<Doc, View>::positive(zoom),
        pages,
    }
}

/// Cut `span` into bands of whole positions. A band takes positions while
/// their sizes sum within its room, and always takes one.
fn bands<A: ambre_core::index::Axis>(
    span: Span<A>,
    size: impl Fn(Pos<A>) -> Px<Surface>,
    room: BandRoom,
) -> Vec<Span<A>> {
    let mut out = Vec::new();
    let mut start = span.start;
    let mut used = Px::<Surface>::ZERO;
    for pos in span {
        let size = size(pos);
        let fits = if out.is_empty() {
            room.first
        } else {
            room.rest
        };
        if pos != start && used + size > fits + FIT_SLACK {
            out.push(Span::new(start, pos));
            start = pos;
            used = Px::<Surface>::ZERO;
        }
        used += size;
    }
    out.push(Span::new(start, span.end));
    out
}

/// The cells of one page as the walk reads them, in the order of the grid.
/// The order makes two walks of one page record the same calls (`PDF-004`).
pub(crate) struct PrintCells(BTreeMap<CellPos, CellData>);

impl PrintCells {
    /// The walk's view of the packed cells `cells`.
    pub(crate) fn new(cells: Vec<DisplayCell>) -> Self {
        // One insert per cell, not `collect`. A collect into a map sorts
        // the pairs first, and that stable sort is one more large function in
        // Main (`ambre-spec-budgets.md section 5`). A later cell at a position
        // replaces an earlier one, as the collect did.
        let mut map = BTreeMap::new();
        for cell in cells {
            map.insert(CellPos::new(cell.row, cell.col), CellData::from(cell));
        }
        Self(map)
    }
}

impl SheetCells for PrintCells {
    fn cell(&self, at: CellPos) -> Option<SheetCell<'_>> {
        self.0.get(&at).map(|c| SheetCell {
            content: c.content(&c.text),
            merge_span: c.merge_span,
        })
    }

    fn for_each_merge(&self, f: &mut dyn FnMut(CellPos, MergeSpan, CellContent<'_>)) {
        for (&at, c) in &self.0 {
            if let Some(span) = c.merge_span {
                f(at, span, c.content(&c.text));
            }
        }
    }
}

/// Draw the page `cut` onto any surface: the title row first when the page
/// repeats it, then the page's own rows below it. Each band is the screen's
/// sheet walk over its region of the sheet, under a camera that places the
/// region on the paper (`PDF-001`). The surface counts in points.
pub(crate) fn paint_page<S: DrawSurface>(
    surface: &mut S,
    cut: &PageCut,
    view: &SheetView<'_>,
    walk: &mut SheetWalk<'_>,
    zoom: Scale<Doc, View>,
) {
    let axes = &view.axes;
    let area = PrintArea::of(PRINT_PAPER, PRINT_MARGIN);
    let margin = f64::from(PRINT_MARGIN.to_doc_px().get());
    let x0 = axes.col_x(cut.cols.start).get();
    let x1 = axes.col_x(cut.cols.end).get();
    let band = |rows: Span<Row>| {
        Rect::new(
            x0,
            axes.row_y(rows.start).get(),
            x1,
            axes.row_y(rows.end).get(),
        )
    };
    surface.push_transform(Affine::scale(1.0 / f64::from(Pt::DOC_PX)));
    surface.push_clip(Rect::new(
        margin,
        margin,
        margin + f64::from(area.width.get()),
        margin + f64::from(area.height.get()),
    ));
    let mut top = margin;
    let regions = cut
        .title
        .map(|row| band(Span::new(row, row.next())))
        .into_iter()
        .chain(std::iter::once(band(cut.rows)));
    for region in regions {
        surface.push_transform(
            Affine::translate((margin, top))
                * Affine::scale(zoom.get())
                * Affine::translate((-region.x0, -region.y0)),
        );
        walk.walk(view, region, surface);
        surface.pop_transform();
        top += region.height() * zoom.get();
    }
    surface.pop_clip();
    surface.pop_transform();
}

/// A PDF export between two slices: the pages left to draw, the recorded
/// file so far, and the caches the walk keeps between pages.
pub(super) struct PrintJob {
    /// The pages left, in print order.
    pages: VecDeque<PageCut>,
    /// The zoom every page draws at.
    zoom: Scale<Doc, View>,
    /// The rows the shared filter hides, sorted, as the walk reads them.
    hidden_rows: Vec<Pos<Row>>,
    /// The pages recorded so far, and every face once.
    document: PrintDocument,
    tables: RecorderTables,
    /// The shaped text of the cells, kept from page to page.
    text_cache: TextCache,
    /// The name the file is handed to the browser under.
    filename: ambre_core::ExportFilename,
    /// The pages drawn so far, as the progress counts them.
    printed: Count<Steps>,
    /// Every page of the export.
    total: Count<Steps>,
}

impl TrameState {
    /// Whether a PDF export runs (`TRAME-IO-076`).
    #[must_use]
    pub fn is_printing(&self) -> bool {
        self.print_job.is_some()
    }

    /// Start a PDF export of the active sheet under `options`. The pages are
    /// cut now, and [`print_slice`](Self::print_slice) draws them.
    ///
    /// # Errors
    ///
    /// [`StopCause::NothingToConvert`] when the range holds no cell to print
    /// (`TRAME-IO-075`).
    pub fn print_start(&mut self, options: PdfExport) -> Result<(), StopCause> {
        let Some(used) = self.print_extent() else {
            return Err(StopCause::NothingToConvert);
        };
        let range = match options.range {
            PrintRange::UsedRange => used,
            PrintRange::Selection => {
                let selection = self.live_selection_rect();
                let top_left = selection.top_left;
                let bottom_right = CellPos::new(
                    selection.bottom_right.row.min(used.bottom_right.row),
                    selection.bottom_right.col.min(used.bottom_right.col),
                );
                if top_left.row > bottom_right.row || top_left.col > bottom_right.col {
                    return Err(StopCause::NothingToConvert);
                }
                CellRect::spanning(top_left, bottom_right)
            }
        };
        let mut hidden_rows: Vec<Pos<Row>> = self.hidden_rows.iter().copied().collect();
        hidden_rows.sort_unstable();
        let axes = SheetAxes {
            col_width: COL_WIDTH,
            row_height: ROW_HEIGHT,
            col_widths: &self.col_widths,
            row_heights: &self.row_heights,
            hidden_rows: &hidden_rows,
        };
        let Pagination { zoom, pages } = paginate(
            range,
            &axes,
            PrintArea::of(PRINT_PAPER, PRINT_MARGIN),
            options,
        );
        let total = Count::<Steps>::of(pages.len());
        let filename = ambre_core::ExportFilename::from_title(
            self.sheet_title(&self.active_sheet).as_str(),
            "sheet",
            ambre_core::ExportExtension::Pdf,
        );
        self.print_job = Some(Box::new(PrintJob {
            pages: pages.into(),
            zoom,
            hidden_rows,
            document: PrintDocument::new(PrintPage {
                width: PRINT_PAPER.width.in_unit::<Pt>(),
                height: PRINT_PAPER.height.in_unit::<Pt>(),
            }),
            tables: RecorderTables::default(),
            text_cache: TextCache::new(),
            filename,
            printed: Count::ZERO,
            total,
        }));
        Ok(())
    }

    /// The cells of the active sheet a print covers. They run from the first
    /// cell to the last one that holds a value, a style or a merged block. `None` on
    /// a sheet with none of the three.
    fn print_extent(&self) -> Option<CellRect> {
        let index = self.index.clone();
        let values = self.used_extent(&self.active_sheet, &index);
        let styled = self.styles.keys().copied();
        let merged = self.merges.values().copied();
        let last = values
            .into_iter()
            .chain(styled)
            .chain(merged)
            .reduce(|a, b| CellPos::new(a.row.max(b.row), a.col.max(b.col)))?;
        Some(CellRect::spanning(CellPos::ORIGIN, last))
    }

    /// Walk `pace` of the running export: one page, or every page left. The
    /// slice that draws the last page hands the recorded file to `converter`
    /// and answers with the bytes (`PDF-003`). A slice short of the end
    /// answers its progress. No slice answers a delta (`PDF-002`).
    pub fn print_slice(
        &mut self,
        font_cx: &mut FontContext,
        converter: &Converter,
        pace: PrintPace,
    ) -> TrameOutcome {
        let Some(mut job) = self.print_job.take() else {
            return TrameOutcome::display_only(Vec::new());
        };
        let indices = self.engine_indices();
        self.engine.compute_all_dirty(&indices);
        let palette = Theme::Light.palette();
        while let Some(cut) = job.pages.pop_front() {
            let cells = PrintCells::new(self.pack_page(&cut));
            let view = SheetView {
                cells: &cells,
                axes: SheetAxes {
                    col_width: COL_WIDTH,
                    row_height: ROW_HEIGHT,
                    col_widths: &self.col_widths,
                    row_heights: &self.row_heights,
                    hidden_rows: &job.hidden_rows,
                },
                palette,
                text_pass: TextPass::WithText,
            };
            let mut walk = SheetWalk {
                font_cx: &mut *font_cx,
                text_cache: &mut job.text_cache,
                cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
            };
            let mut recorder = ListRecorder::new(&mut job.document, &mut job.tables);
            paint_page(&mut recorder, &cut, &view, &mut walk, job.zoom);
            recorder.finish();
            job.printed = job.printed.incremented();
            if pace == PrintPace::Slice {
                break;
            }
        }
        if !job.pages.is_empty() {
            let progress = ConversionProgress::new(Direction::Export, job.printed, job.total);
            self.print_job = Some(job);
            return TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::Progress(
                progress,
            ))]);
        }
        let PrintJob {
            document, filename, ..
        } = *job;
        let answer = match converter.convert(&Conversion::WritePdf { document }) {
            Ok(Converted::File(data)) => InteropAnswer::ExportReady {
                file: ExportedFile {
                    filename,
                    media_type: ambre_core::MediaType::from("application/pdf"),
                    data,
                },
                // The page holds what the screen's walk draws, so the file
                // loses nothing the grid shows.
                report: TrameReport::new(Direction::Export, FileFormat::Pdf),
            },
            Ok(_) => InteropAnswer::failed(
                Direction::Export,
                InteropError::Stopped(StopCause::ConverterAnswered),
            ),
            Err(error) => {
                InteropAnswer::failed(Direction::Export, InteropError::Stopped(error.stop_cause()))
            }
        };
        TrameOutcome::display_only(vec![TrameEvent::Interop(answer)])
    }

    /// Stop the running export with `cause`, and answer it: the converter
    /// module or the faces it needs are gone.
    pub fn print_stop(&mut self, cause: StopCause) -> TrameOutcome {
        if self.print_job.take().is_none() {
            return TrameOutcome::display_only(Vec::new());
        }
        TrameOutcome::display_only(vec![TrameEvent::Interop(InteropAnswer::failed(
            Direction::Export,
            InteropError::Stopped(cause),
        ))])
    }

    /// The display cells page `cut` draws: its band, its title row, and the
    /// master of each merged block that reaches into the page from outside.
    fn pack_page(&self, cut: &PageCut) -> Vec<DisplayCell> {
        let corner = |rows: Span<Row>| {
            let last_row = rows.last()?;
            let last_col = cut.cols.last()?;
            Some(CellRect::spanning(
                CellPos::new(rows.start, cut.cols.start),
                CellPos::new(last_row, last_col),
            ))
        };
        let band = corner(cut.rows);
        let title = cut.title.and_then(|row| corner(Span::new(row, row.next())));
        let windows: Vec<CellRect> = band.into_iter().chain(title).collect();
        let mut cells: Vec<DisplayCell> = windows
            .iter()
            .flat_map(|&window| self.pack_cells(window))
            .collect();
        let mut masters: Vec<CellPos> = self
            .merges
            .iter()
            .filter(|&(&master, &end)| {
                let block = CellRect::spanning(master, end);
                windows
                    .iter()
                    .any(|w| w.overlaps(block) && !w.contains(master))
            })
            .map(|(&master, _)| master)
            .collect();
        masters.sort_unstable();
        cells.extend(
            masters
                .into_iter()
                .flat_map(|master| self.pack_cells(CellRect::cell(master))),
        );
        cells
    }
}

// The converter engine, the extractor and proptest are host dev-dependencies.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use ambre_core::index::Len;
    use ambre_draw::list::DrawOp as Op;
    use ambre_draw::testing::{RecordingSurface, shipped_font_context};

    /// The converter the Logic thread's host build hands in: the engine,
    /// linked in place.
    fn converter() -> Converter {
        Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine")
    }

    fn col(n: u32) -> Pos<Col> {
        Pos::<Col>::new(n)
    }

    fn row(n: u32) -> Pos<Row> {
        Pos::<Row>::new(n)
    }

    /// The range from A1 to the cell at `rows` x `cols`.
    fn range(rows: u32, cols: u32) -> CellRect {
        CellRect::spanning(CellPos::ORIGIN, CellPos::new(row(rows - 1), col(cols - 1)))
    }

    /// The axes of a sheet with default sizes, but for `widths`, `heights`
    /// and `hidden`.
    fn axes<'a>(
        widths: &'a HashMap<Pos<Col>, Px<Doc>>,
        heights: &'a HashMap<Pos<Row>, Px<Doc>>,
        hidden: &'a [Pos<Row>],
    ) -> SheetAxes<'a> {
        SheetAxes {
            col_width: COL_WIDTH,
            row_height: ROW_HEIGHT,
            col_widths: widths,
            row_heights: heights,
            hidden_rows: hidden,
        }
    }

    fn area() -> PrintArea {
        PrintArea::of(PRINT_PAPER, PRINT_MARGIN)
    }

    fn options(fit: PrintFit, title_row: TitleRow) -> PdfExport {
        PdfExport {
            range: PrintRange::UsedRange,
            fit,
            title_row,
        }
    }

    /// `TRAME-IO-072`, `TRAME-IO-077`: a range of default cells breaks after
    /// the last whole column and row a page holds. The pages run down the
    /// rows first, then across the columns.
    #[test]
    fn breaks_fall_on_row_and_column_edges() {
        let (w, h) = (HashMap::default(), HashMap::default());
        let cut = paginate(
            range(200, 30),
            &axes(&w, &h, &[]),
            area(),
            options(PrintFit::ActualSize, TitleRow::Off),
        );
        // 697.7 px of width hold 8 columns of 80 px; 1026.5 px of height
        // hold 51 rows of 20 px.
        let cols = [(0, 8), (8, 16), (16, 24), (24, 30)];
        let rows = [(0, 51), (51, 102), (102, 153), (153, 200)];
        let expected: Vec<PageCut> = cols
            .iter()
            .flat_map(|&(c0, c1)| {
                rows.iter().map(move |&(r0, r1)| PageCut {
                    cols: Span::new(col(c0), col(c1)),
                    rows: Span::new(row(r0), row(r1)),
                    title: None,
                })
            })
            .collect();
        assert_eq!(cut.pages, expected);
        assert_eq!(cut.zoom.get(), 1.0);
    }

    /// `TRAME-IO-072`: a column wider than the page takes a page of its own,
    /// and a hidden row takes no height, so it opens no page.
    #[test]
    fn a_wide_column_stands_alone_and_a_hidden_row_opens_no_page() {
        let mut w = HashMap::default();
        w.insert(col(1), Px::<Doc>::new(2000.0));
        let h = HashMap::default();
        let hidden: Vec<Pos<Row>> = (51..60).map(row).collect();
        let cut = paginate(
            range(60, 3),
            &axes(&w, &h, &hidden),
            area(),
            options(PrintFit::ActualSize, TitleRow::Off),
        );
        let bands: Vec<(Span<Col>, Span<Row>)> =
            cut.pages.iter().map(|p| (p.cols, p.rows)).collect();
        let all = Span::new(row(0), row(60));
        assert_eq!(
            bands,
            [
                (Span::new(col(0), col(1)), all),
                (Span::new(col(1), col(2)), all),
                (Span::new(col(2), col(3)), all),
            ]
        );
    }

    /// `TRAME-IO-073`, `TRAME-IO-078`: fit to width shrinks a wide range
    /// until one band holds every column, and leaves a narrow range at its
    /// size.
    #[test]
    fn fit_to_width_shrinks_a_wide_range_and_never_enlarges() {
        let (w, h) = (HashMap::default(), HashMap::default());
        let wide = paginate(
            range(10, 30),
            &axes(&w, &h, &[]),
            area(),
            options(PrintFit::Width, TitleRow::Off),
        );
        assert!(wide.pages.iter().all(|p| p.cols == range(10, 30).cols()));
        let expected = f64::from(area().width.get()) / 2400.0;
        assert!((wide.zoom.get() - expected).abs() < 1e-9);
        let narrow = paginate(
            range(10, 2),
            &axes(&w, &h, &[]),
            area(),
            options(PrintFit::Width, TitleRow::Off),
        );
        assert_eq!(narrow.zoom.get(), 1.0);
    }

    /// `TRAME-IO-074`: the repeated first row heads every page but the first
    /// of each column band, and takes its height from each such page.
    #[test]
    fn the_title_row_heads_every_page_below_the_first() {
        let (w, h) = (HashMap::default(), HashMap::default());
        let cut = paginate(
            range(120, 10),
            &axes(&w, &h, &[]),
            area(),
            options(PrintFit::ActualSize, TitleRow::Repeated),
        );
        let heads: Vec<(Option<Pos<Row>>, Span<Row>)> =
            cut.pages.iter().map(|p| (p.title, p.rows)).collect();
        let column = [
            (None, Span::new(row(0), row(51))),
            (Some(row(0)), Span::new(row(51), row(101))),
            (Some(row(0)), Span::new(row(101), row(120))),
        ];
        assert_eq!(heads, [column, column].concat());
    }

    proptest::proptest! {
        /// `TRAME-IO-072`: whatever the sizes, the bands of an axis cover the
        /// range once, in order, and each fits its room or holds one position.
        #[test]
        fn bands_cover_the_range_once_and_each_fits(
            widths in proptest::collection::vec(1.0f32..1500.0, 1..60),
        ) {
            let w: HashMap<Pos<Col>, Px<Doc>> = widths
                .iter()
                .enumerate()
                .map(|(i, &px)| (col(i as u32), Px::<Doc>::new(px)))
                .collect();
            let h = HashMap::default();
            let cut = paginate(
                range(1, widths.len() as u32),
                &axes(&w, &h, &[]),
                area(),
                options(PrintFit::ActualSize, TitleRow::Off),
            );
            let room = f64::from(area().width.get());
            let mut next = col(0);
            for page in &cut.pages {
                proptest::prop_assert_eq!(page.cols.start, next);
                let used: f64 = page.cols.into_iter().map(|c| f64::from(widths[c.get() as usize])).sum();
                proptest::prop_assert!(used <= room + 1e-6 || page.cols.len() == Len::<Col>::new(1));
                next = page.cols.end;
            }
            proptest::prop_assert_eq!(next, col(widths.len() as u32));
        }
    }

    /// A workbook holding `text(r, c)` in each cell of `rows` x `cols`.
    fn sheet(rows: u32, cols: u32, text: impl Fn(u32, u32) -> String) -> TrameState {
        let mut state = TrameState::new();
        let cells = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| (CellPos::new(row(r), col(c)), text(r, c)))
            .collect();
        state.handle_command(TrameCommand::SetCellsBatch { cells });
        state
    }

    /// Merge `rect` in `state`, and check that the merge landed. A merge
    /// over a covered cell that holds a value is refused, so the caller
    /// leaves the covered cells empty.
    fn merge(state: &mut TrameState, rect: CellRect) {
        state.handle_command(TrameCommand::MergeCells { rect });
        assert!(
            state.merges.contains_key(&rect.top_left),
            "the covered cells are empty, so the merge lands"
        );
    }

    /// Whether `(r, c)` is a covered cell of a block whose master is a
    /// cell of `masters` and whose size is two by two.
    fn covered(masters: &[(u32, u32)], r: u32, c: u32) -> bool {
        masters.iter().any(|&(mr, mc)| {
            (mr..=mr + 1).contains(&r) && (mc..=mc + 1).contains(&c) && (r, c) != (mr, mc)
        })
    }

    /// Run a whole export of `state` a slice at a time. Answers every
    /// outcome, and the file the last one carries.
    fn export(
        state: &mut TrameState,
        font_cx: &mut FontContext,
        options: PdfExport,
    ) -> (Vec<TrameOutcome>, Vec<u8>) {
        state.print_start(options).expect("the range holds cells");
        let converter = converter();
        let mut outcomes = Vec::new();
        while state.is_printing() {
            outcomes.push(state.print_slice(font_cx, &converter, PrintPace::Slice));
        }
        let file = outcomes
            .iter()
            .flat_map(|o| &o.display)
            .find_map(|event| match event {
                TrameEvent::Interop(InteropAnswer::ExportReady { file, .. }) => {
                    Some(file.data.as_bytes().to_vec())
                }
                _ => None,
            })
            .expect("the last slice answers the file");
        (outcomes, file)
    }

    /// The words of each page of `file`, read by an extractor that shares
    /// no code with the writer.
    fn page_words(file: &[u8]) -> Vec<Vec<String>> {
        pdf_extract::extract_text_from_mem_by_pages(file)
            .expect("the file reads")
            .iter()
            .map(|page| page.split_whitespace().map(str::to_owned).collect())
            .collect()
    }

    /// `TRAME-IO-074`, `TRAME-IO-076`: a column of 120 labels prints on three
    /// pages, one slice each. The second and the third page start with the
    /// repeated first row, then their own first row.
    #[test]
    fn the_repeated_row_heads_each_printed_page() {
        let mut state = sheet(120, 1, |r, _| format!("r{r}"));
        let mut font_cx = shipped_font_context();
        let (outcomes, file) = export(
            &mut state,
            &mut font_cx,
            options(PrintFit::ActualSize, TitleRow::Repeated),
        );
        assert_eq!(outcomes.len(), 3, "one page per slice");
        let pages = page_words(&file);
        let heads: Vec<&[String]> = pages.iter().map(|p| &p[..2]).collect();
        assert_eq!(heads, [["r0", "r1"], ["r0", "r51"], ["r0", "r101"]]);
        assert_eq!(pages[2].last().map(String::as_str), Some("r119"));
    }

    /// `TRAME-IO-072`, `TRAME-IO-077`: twelve labelled columns print on two
    /// pages at their size, the break after the eighth column. Each label
    /// reads back whole, on one page.
    #[test]
    fn a_wide_row_breaks_at_a_column_edge() {
        let mut state = sheet(1, 12, |_, c| format!("c{c}"));
        let mut font_cx = shipped_font_context();
        let (_, file) = export(
            &mut state,
            &mut font_cx,
            options(PrintFit::ActualSize, TitleRow::Off),
        );
        let pages = page_words(&file);
        let label = |range: std::ops::Range<u32>| -> Vec<String> {
            range.map(|c| format!("c{c}")).collect()
        };
        assert_eq!(pages, [label(0..8), label(8..12)]);
        // Fit to width puts the twelve columns on one page.
        let (_, fitted) = export(
            &mut state,
            &mut font_cx,
            options(PrintFit::Width, TitleRow::Off),
        );
        assert_eq!(page_words(&fitted), [label(0..12)]);
    }

    /// `TRAME-IO-075`: the selection prints alone, cut to the used range, and
    /// a sheet with nothing to print writes no file and says why.
    #[test]
    fn the_selection_prints_alone_and_an_empty_sheet_stops() {
        let mut state = sheet(4, 4, |r, c| format!("v{r}{c}"));
        state.handle_command(TrameCommand::SetActiveCell {
            row: row(1),
            col: col(1),
            anchor_row: row(9),
            anchor_col: col(2),
            extra: vec![],
            kind: crate::SelectionKind::Cells,
        });
        let mut font_cx = shipped_font_context();
        let selection = PdfExport {
            range: PrintRange::Selection,
            ..PdfExport::default()
        };
        let (_, file) = export(&mut state, &mut font_cx, selection);
        assert_eq!(
            page_words(&file),
            [["v11", "v12", "v21", "v22", "v31", "v32"]]
        );
        let mut empty = TrameState::new();
        assert_eq!(
            empty.print_start(PdfExport::default()),
            Err(StopCause::NothingToConvert)
        );
        assert!(!empty.is_printing());
    }

    /// `PDF-004`: two exports of one peer write the same bytes, and so does a
    /// second peer fed the shipped faces. A third peer fed no face writes
    /// another file, which shows the comparison can fail.
    #[test]
    fn two_peers_holding_the_same_faces_export_one_file() {
        // Two merged blocks on one page: a walk that met them in the order
        // of a hash table would draw them in either order.
        let masters = [(2, 0), (40, 0)];
        let mut a = sheet(60, 3, |r, c| {
            if covered(&masters, r, c) {
                String::new()
            } else {
                format!("cell {r} {c}")
            }
        });
        for (top, left) in masters {
            merge(
                &mut a,
                CellRect::spanning(
                    CellPos::new(row(top), col(left)),
                    CellPos::new(row(top + 1), col(left + 1)),
                ),
            );
        }
        let mut b = TrameState::new();
        b.import_snapshot(&a.export_snapshot())
            .expect("a snapshot this build wrote reads back");
        let mut c = TrameState::new();
        c.import_snapshot(&a.export_snapshot())
            .expect("a snapshot this build wrote reads back");
        let options = options(PrintFit::Width, TitleRow::Repeated);
        let (_, first) = export(&mut a, &mut shipped_font_context(), options);
        let (_, again) = export(&mut a, &mut shipped_font_context(), options);
        let (_, peer) = export(&mut b, &mut shipped_font_context(), options);
        let (_, faceless) = export(&mut c, &mut FontContext::new(), options);
        assert!(first.starts_with(b"%PDF"));
        assert_eq!(first, again);
        assert_eq!(first, peer);
        assert_ne!(first, faceless);
    }

    /// `TRAME-IO-079`: an export leaves the active cell, the selection and
    /// the sheet shown as they were.
    #[test]
    fn a_print_leaves_the_selection_and_the_sheet_shown() {
        let mut state = sheet(70, 3, |r, c| format!("{r}{c}"));
        state.handle_command(TrameCommand::SetActiveCell {
            row: row(2),
            col: col(1),
            anchor_row: row(4),
            anchor_col: col(2),
            extra: vec![],
            kind: crate::SelectionKind::Cells,
        });
        let view = |s: &TrameState| (s.active, s.live_selection_rect(), s.active_sheet.clone());
        let before = view(&state);
        let _ = export(
            &mut state,
            &mut shipped_font_context(),
            options(PrintFit::Width, TitleRow::Repeated),
        );
        assert_eq!(view(&state), before);
    }

    /// `PDF-002`: no slice of an export answers a delta, and the document's
    /// version vector reads the same after the export.
    #[test]
    fn a_pdf_export_leaves_the_oplog_unchanged() {
        let mut state = sheet(130, 2, |r, c| format!("{r}:{c}"));
        let before = state.doc.oplog_vv();
        let (outcomes, _) = export(
            &mut state,
            &mut shipped_font_context(),
            options(PrintFit::Width, TitleRow::Repeated),
        );
        assert!(outcomes.len() > 1, "the export runs in slices");
        assert!(outcomes.iter().all(|o| o.update.is_none()));
        assert_eq!(state.doc.oplog_vv(), before);
    }

    /// `PDF-006`: pages of text, fills, borders and a merged block record
    /// glyphs and vector fills, and no image paint.
    #[test]
    fn a_printed_page_paints_no_image() {
        let mut state = sheet(80, 3, |r, c| {
            if covered(&[(5, 0)], r, c) {
                String::new()
            } else {
                format!("{r}{c}")
            }
        });
        merge(
            &mut state,
            CellRect::spanning(CellPos::new(row(5), col(0)), CellPos::new(row(6), col(1))),
        );
        state
            .print_start(options(PrintFit::Width, TitleRow::Off))
            .expect("the range holds cells");
        let converter = converter();
        let mut font_cx = shipped_font_context();
        // Every page but the last, whose slice writes the file.
        let _ = state.print_slice(&mut font_cx, &converter, PrintPace::Slice);
        let job = state.print_job.as_ref().expect("a page is left");
        let ops: Vec<&Op> = job.document.pages.iter().flatten().collect();
        assert!(ops.iter().any(|op| matches!(op, Op::Glyphs { .. })));
        assert!(ops.iter().any(|op| matches!(op, Op::FillRect { .. })));
        assert!(
            !ops.iter()
                .any(|op| matches!(op, Op::Image { .. } | Op::EncodedImage { .. }))
        );
    }

    /// `TRAME-IO-083`: a merged block across a row break and a column break
    /// prints on each of the four pages it reaches. Each page fills the whole
    /// block inside the clip of its own band. So each page shows one quarter,
    /// and the four quarters tile the block.
    #[test]
    fn a_merge_across_a_break_prints_its_part_on_each_page() {
        // The breaks fall after column 8 and after row 51. The block covers
        // rows 50 and 51 and columns 7 and 8: one cell sits on each page.
        let mut state = sheet(60, 10, |r, c| {
            if covered(&[(50, 7)], r, c) {
                String::new()
            } else {
                format!("{r}{c}")
            }
        });
        let master = CellPos::new(row(50), col(7));
        merge(
            &mut state,
            CellRect::spanning(master, CellPos::new(row(51), col(8))),
        );
        let hidden = Vec::new();
        let axes = axes(&state.col_widths, &state.row_heights, &hidden);
        let cut = paginate(
            range(60, 10),
            &axes,
            area(),
            options(PrintFit::ActualSize, TitleRow::Off),
        );
        let x = |c: u32| axes.col_x(col(c)).get();
        let y = |r: u32| axes.row_y(row(r)).get();
        let block = Rect::new(x(7), y(50), x(9), y(52));
        let mut font_cx = shipped_font_context();
        let mut cache = TextCache::new();
        let shown: Vec<Rect> = cut
            .pages
            .iter()
            .map(|page| {
                let cells = PrintCells::new(state.pack_page(page));
                let view = SheetView {
                    cells: &cells,
                    axes,
                    palette: Theme::Light.palette(),
                    text_pass: TextPass::WithText,
                };
                let mut surface = RecordingSurface::new();
                paint_page(
                    &mut surface,
                    page,
                    &view,
                    &mut SheetWalk {
                        font_cx: &mut font_cx,
                        text_cache: &mut cache,
                        cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
                    },
                    Scale::<Doc, View>::ONE,
                );
                // The innermost clip is the walk's band, in sheet coordinates.
                use ambre_draw::testing::DrawOp as Rec;
                let mut clips: Vec<Rect> = Vec::new();
                let mut fill = None;
                for op in surface.ops() {
                    match op {
                        Rec::PushClip { rect } => clips.push(*rect),
                        Rec::PopClip => {
                            clips.pop();
                        }
                        Rec::FillRect { rect, .. } if *rect == block => {
                            fill = clips.last().map(|clip| clip.intersect(block));
                        }
                        _ => {}
                    }
                }
                fill.expect("each page fills the whole block under its band clip")
            })
            .collect();
        assert_eq!(
            shown,
            [
                Rect::new(x(7), y(50), x(8), y(51)),
                Rect::new(x(7), y(51), x(8), y(52)),
                Rect::new(x(8), y(50), x(9), y(51)),
                Rect::new(x(8), y(51), x(9), y(52)),
            ],
            "pages run down, then across: one quarter of the block each"
        );
    }

    /// `PDF-001`, `TRAME-IO-023`: a page draws the screen's own sheet walk
    /// over its region, call for call. A label too wide for its column stops
    /// at its non-empty neighbour. Its whole run lands inside a clip that
    /// ends at the column edge, so no glyph is cut for an ellipsis.
    #[test]
    fn a_page_draws_the_walk_the_screen_draws() {
        let long = "a label far too wide for its column";
        let state = sheet(3, 3, |r, c| match (r, c) {
            (0, 0) => long.to_owned(),
            _ => format!("{r}{c}"),
        });
        let cut = PageCut {
            cols: Span::new(col(0), col(3)),
            rows: Span::new(row(0), row(3)),
            title: None,
        };
        let cells = PrintCells::new(state.pack_page(&cut));
        let hidden = Vec::new();
        let view = SheetView {
            cells: &cells,
            axes: axes(&state.col_widths, &state.row_heights, &hidden),
            palette: Theme::Light.palette(),
            text_pass: TextPass::WithText,
        };
        let mut font_cx = shipped_font_context();
        let mut cache = TextCache::new();
        let mut page = RecordingSurface::new();
        let zoom = Scale::<Doc, View>::ONE;
        paint_page(
            &mut page,
            &cut,
            &view,
            &mut SheetWalk {
                font_cx: &mut font_cx,
                text_cache: &mut cache,
                cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
            },
            zoom,
        );
        let region = Rect::new(
            0.0,
            0.0,
            view.axes.col_x(col(3)).get(),
            view.axes.row_y(row(3)).get(),
        );
        let mut screen = RecordingSurface::new();
        SheetWalk {
            font_cx: &mut font_cx,
            text_cache: &mut cache,
            cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
        }
        .walk(&view, region, &mut screen);
        assert!(page.holds_walk(&screen), "the page draws the screen's walk");

        use ambre_draw::testing::DrawOp as Rec;
        let edge = view.axes.col_x(col(1)).get();
        let clipped = screen.ops().windows(2).any(|pair| {
            matches!(
                pair,
                [Rec::PushClip { rect }, Rec::Glyphs { glyph_count, .. }]
                    if rect.x0 == 0.0 && rect.x1 == edge
                        && glyph_count.get() as usize == long.chars().count()
            )
        });
        assert!(clipped, "the long label is cut at its column edge");
    }
}
