//! Painting the pages (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! The page's content comes from one walk, generic over the drawing surface
//! (`PROSE-UI-018`, `CANVAS-074`). The screen resolves what each page shows
//! and draws the sheets and the selections. It hands the pages to the walk
//! under its camera, then draws the carets and the composition over them. Every colour
//! comes from the palette the host fills from the theme tokens, or from a
//! peer's own colour (`PROSE-UI-014`). A paint reads the layout in force and
//! writes nothing to the document (`PROSE-UI-120`).

use ambre_core::index::{Len, Pos, Span};
use ambre_core::{Byte, Count};
use ambre_core::{Px, Surface};
use ambre_draw::{Affine, DocumentWalk, DrawSurface, Point, Rect};

use crate::events::{BlockId, CursorPosition, block_id};
use crate::layout::measure::{EffectiveStyle, shape_draft};
use crate::layout::place::NOTE_SEPARATOR;
use crate::layout::run::{ParleyRunData, RunChar, RunLine, ShapedRun, TableShape};
use crate::ui::ProseUiState;
use crate::view::cache::RunKey;
use crate::view::caret::Spot;
use crate::view::hit::{Fragment, cell_frame};
pub use crate::view::walk::PagePalette;
use crate::view::walk::{
    BlockPaint, CellPaint, Highlights, Insertion, PageItem, PagePaint, PageView, PageWalk, Placed,
    draw_shape, to_rect,
};

/// How far the shadow falls from the sheet.
const SHADOW_OFFSET: Px<Surface> = Px::<Surface>::new(3.0);
/// The width of the local caret and of a remote one.
const CARET_WIDTH: Px<Surface> = Px::<Surface>::new(1.5);
/// The tab a remote caret carries above its bar, standing for its label.
const LABEL_TAB: Px<Surface> = Px::<Surface>::new(5.0);

/// A block the screen draws around: its selections under the walk, its
/// carets over it.
struct ScreenBlock {
    id: BlockId,
    run: ShapedRun,
    placed: Placed,
    /// The top of its page down the column.
    top: Px<Surface>,
    /// Whether the local caret and selection show in it.
    local: LocalCaret,
    /// Whether the other peers' carets show in it: body text only.
    remote_carets: RemoteCarets,
}

/// Whether a block shows the local caret and selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalCaret {
    Hidden,
    Shown,
}

/// Whether a block shows the other peers' carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteCarets {
    Hidden,
    Shown,
}

/// The content box of a page, on the surface: where its runs are placed.
#[derive(Clone, Copy, Debug)]
struct ContentBox {
    /// The left edge of the content, from the sheet's.
    left: Px<Surface>,
    /// The top of the body, under the header band.
    body_top: Px<Surface>,
    /// The width the text wraps at.
    width: Px<Surface>,
}

/// The caret of the paragraph a split opens, before the document names its
/// block. The clip and the bar are two rects side by side, so each is named.
#[derive(Clone, Copy)]
struct GhostCaret {
    /// The top of its page.
    top: Px<Surface>,
    /// The fragment it is clipped to.
    clip: Rect,
    /// The caret bar itself.
    bar: Rect,
}

/// What one frame resolves before it paints.
#[derive(Default)]
struct Frame {
    pages: Vec<PagePaint>,
    blocks: Vec<ScreenBlock>,
    /// The caret of the paragraph a split opens, before the document names
    /// its block.
    ghost_carets: Vec<GhostCaret>,
}

/// Where a run is drawn: from the left of the content box, at the top of
/// its fragment, clipped to it.
fn place(content: ContentBox, run: &ShapedRun, piece: Fragment) -> Placed {
    let left = content.left.get();
    let clip_top = content.body_top.get() + f64::from(piece.top.get());
    Placed {
        origin: Point::new(
            left + f64::from(run.data().indent().get()),
            clip_top - f64::from(piece.run_top.get()),
        ),
        clip: Rect::new(
            left,
            clip_top,
            left + content.width.get(),
            clip_top + f64::from(piece.height.get()),
        ),
    }
}

/// The code-point range of `block` a selection from `start` to `end` covers,
/// the two ends already in flow order and read on the runs drawn.
fn covered(
    ui: &ProseUiState,
    block: BlockId,
    len: Len<RunChar>,
    ends: ((BlockId, Pos<RunChar>), (BlockId, Pos<RunChar>)),
) -> Option<Span<RunChar>> {
    let ((first, from), (last, to)) = ends;
    let len = len.as_end();
    // Within one block, the flow is not asked: a header or a footer holds
    // its own selection and has no place in it.
    if first == last {
        return (first == block && from < to).then_some(Span::<RunChar>::new(from, to.min(len)));
    }
    let here = ui.flow_index(&crate::events::tree_id(block))?;
    let first_index = ui.flow_index(&crate::events::tree_id(first))?;
    let last_index = ui.flow_index(&crate::events::tree_id(last))?;
    if here < first_index || here > last_index {
        return None;
    }
    let start = if here == first_index {
        from
    } else {
        Pos::<RunChar>::ZERO
    };
    let end = if here == last_index { to } else { len };
    (start < end).then_some(Span::<RunChar>::new(start, end))
}

/// The two ends of a selection in flow order, or `None` for a caret alone.
fn ordered(ui: &ProseUiState, cursor: &CursorPosition) -> Option<(Spot, Spot)> {
    let anchor = cursor.anchor?;
    let focus = (cursor.block_id, cursor.offset);
    let key = |(block, offset): Spot| {
        (
            ui.flow_index(&crate::events::tree_id(block))
                .unwrap_or(Pos::<crate::layout::run::Flow>::MAX),
            offset,
        )
    };
    if key(anchor) <= key(focus) {
        Some((anchor, focus))
    } else {
        Some((focus, anchor))
    }
}

impl ProseUiState {
    /// Shape again the paragraphs the draft or a split changes
    /// (`PROSE-UI-201`), and the composition, on the contexts of the thread
    /// that paints.
    ///
    /// A paragraph shaped here is drawn in place of its run until a layout
    /// carries what it shows. Paragraphs already shaped from the same state
    /// are kept.
    pub fn shape_draft(
        &mut self,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<ambre_draw::ParleyBrush>,
    ) {
        self.shape_provisional(font_cx, layout_cx);
        // The composition is shaped apart: it sits at the caret, wherever the
        // draft is (`PROSE-UI-142`).
        let compose = self
            .cursor
            .filter(|_| !self.composing.is_empty())
            .map(|cursor| {
                let at = self.to_laid(cursor.block_id, cursor.offset);
                (cursor.block_id, at, self.composing.clone())
            });
        let kept = self.compose_shape.take();
        self.compose_shape = self.reshape(kept, compose, font_cx, layout_cx);
    }

    /// Shape a text typed at `at` in the style of its block and the marks
    /// before `at`. A shape already made for the same text is kept.
    fn reshape(
        &self,
        kept: Option<(String, ShapedRun)>,
        wanted: Option<(BlockId, Pos<RunChar>, String)>,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<ambre_draw::ParleyBrush>,
    ) -> Option<(String, ShapedRun)> {
        let (block, at, text) = wanted?;
        let run = self.run_of(block);
        let style = run
            .and_then(ParleyRunData::style)
            .cloned()
            .unwrap_or_else(|| {
                EffectiveStyle::from_parts(
                    &crate::doc::DocDefaults::default(),
                    &crate::message::ParagraphStyle::default(),
                    ambre_core::Weight::Regular,
                )
            });
        // Typed after a marked span, the text lands in it (`PROSE-UI-208`).
        let marks = [crate::layout::measure::MarkedSpan {
            start: Pos::<Byte>::ZERO,
            end: Len::<Byte>::of(text.len()).as_end(),
            ..run.map(|run| run.marks_before(at)).unwrap_or_default()
        }];
        // A shape is kept only in the style and the marks it was made in: the
        // same text typed elsewhere is shaped again (`PROSE-UI-200`).
        if let Some(kept) = kept
            && kept.0 == text
            && kept.1.data().style() == Some(&style)
            && kept.1.data().marks() == marks
        {
            return Some(kept);
        }
        let layout = shape_draft(font_cx, layout_cx, (&text, &marks), &style);
        let run = ShapedRun::marked(&text, layout, &style, &marks);
        Some((text, run))
    }

    /// Paint the pages the canvas shows, and one more on each side
    /// (`PROSE-UI-007`).
    ///
    /// The sheets come first, then the selections under the glyphs. The walk
    /// draws each block's glyphs and inline highlights (`PROSE-UI-117`), then
    /// the bands and the footnotes (`PROSE-UI-115`). The local caret and the
    /// composition follow, and the carets of the other peers come last
    /// (`PROSE-UI-118`).
    pub fn paint<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette) {
        self.caret_drawn = None;
        let Some(pages) = self.column.built_pages(&self.viewport) else {
            return;
        };
        let camera = self.camera();
        let width = Px::<Surface>::new(f64::from(self.shaped.layout.page_width.get()));
        let height = Px::<Surface>::new(f64::from(self.shaped.layout.page_height.get()));
        let mut frame = Frame::default();
        for page in pages {
            let top = Px::<Surface>::new(f64::from(self.column.page_top_y(page).get()));
            self.resolve_page(&mut frame, page, top);
        }

        surface.push_transform(camera);
        for page in &frame.pages {
            paint_sheet(surface, palette, page.top, width, height);
        }
        for block in &frame.blocks {
            surface.push_transform(Affine::translate((0.0, block.top.get())));
            surface.push_clip(block.placed.clip);
            self.paint_selections(surface, palette, block);
            surface.pop_clip();
            surface.pop_transform();
        }

        let region = self.walk_region(camera, &frame);
        let view = PageView {
            palette,
            width,
            height,
            pages: &frame.pages,
        };
        let mut walk = PageWalk {
            runs: &mut self.runs,
            highlights_walked: Count::<Highlights>::ZERO,
        };
        walk.walk(&view, region, surface);
        self.highlights_walked = self.highlights_walked.plus(walk.highlights_walked);

        for block in &frame.blocks {
            self.page_frame = camera * Affine::translate((0.0, block.top.get()));
            surface.push_transform(Affine::translate((0.0, block.top.get())));
            surface.push_clip(block.placed.clip);
            self.paint_caret(surface, palette, block);
            surface.pop_clip();
            surface.pop_transform();
        }
        for &GhostCaret { top, clip, bar } in &frame.ghost_carets {
            surface.push_transform(Affine::translate((0.0, top.get())));
            surface.push_clip(clip);
            surface.fill_rect(bar, palette.accent);
            surface.pop_clip();
            surface.pop_transform();
            self.caret_drawn =
                Some((camera * Affine::translate((0.0, top.get()))).transform_rect_bbox(bar));
        }
        // Last, so a collaborator's caret never hides the text under it.
        for block in frame
            .blocks
            .iter()
            .filter(|block| block.remote_carets == RemoteCarets::Shown)
        {
            surface.push_transform(Affine::translate((0.0, block.top.get())));
            self.paint_carets(surface, block.id, block.run.data(), &block.placed);
            surface.pop_transform();
        }
        surface.pop_transform();
    }

    /// The camera from column coordinates to the canvas: the column's left
    /// edge, the zoom, then the scroll.
    fn camera(&self) -> Affine {
        Affine::translate((f64::from(self.column.left(&self.viewport).get()), 0.0))
            * Affine::scale(f64::from(self.viewport.scale()))
            * Affine::translate((0.0, -f64::from(self.viewport.scroll_y.get())))
    }

    /// The region the walk covers: the canvas, and the pages built on each
    /// side of it. Their runs are then ready when a scroll reaches them.
    fn walk_region(&self, camera: Affine, frame: &Frame) -> Rect {
        let width = f64::from(self.shaped.layout.page_width.get());
        let height = f64::from(self.shaped.layout.page_height.get());
        let canvas = Rect::new(
            0.0,
            0.0,
            f64::from(self.viewport.width.get()),
            f64::from(self.viewport.height.get()),
        );
        frame.pages.iter().fold(
            camera.inverse().transform_rect_bbox(canvas),
            |region, page| {
                region.union(Rect::new(
                    0.0,
                    page.top.get(),
                    width + SHADOW_OFFSET.get(),
                    page.top.get() + height + SHADOW_OFFSET.get(),
                ))
            },
        )
    }

    /// Resolve what `page` shows into the items the walk draws, and the
    /// blocks the screen draws around.
    fn resolve_page(&self, frame: &mut Frame, page: Pos<crate::events::Page>, top: Px<Surface>) {
        let layout = std::sync::Arc::clone(&self.shaped.layout);
        let content = ContentBox {
            left: Px::<Surface>::new(f64::from(layout.margin_left.get())),
            body_top: Px::<Surface>::new(f64::from(layout.body_top().get())),
            width: Px::<Surface>::new(f64::from(layout.content_width.get())),
        };
        let (left, body_top, content_width) = (
            content.left.get(),
            content.body_top.get(),
            content.width.get(),
        );
        let right = left + content_width;
        let mut items = Vec::new();
        let (pieces, ghost_at) = self.pieces(page);
        for piece in pieces {
            let Some(block) = self.shaped.block(piece.index) else {
                continue;
            };
            let fragment = piece.fragment;
            if let Some(table) = piece.table {
                let clip_top = body_top + f64::from(fragment.top.get());
                // A table may be wider than the text: it grows to the right
                // rather than squeezing its columns.
                let table_right = table
                    .cells
                    .iter()
                    .map(|cell| f64::from((cell.x + cell.width).get()))
                    .fold(content_width, f64::max);
                let clip = Rect::new(
                    left,
                    clip_top,
                    left + table_right + 1.0,
                    clip_top + f64::from(fragment.height.get()) + 1.0,
                );
                let origin = Point::new(left, clip_top - f64::from(fragment.run_top.get()));
                let cells = self.table_cells(frame, &table, origin, clip, top);
                items.push(PageItem::Table { clip, cells });
                continue;
            }
            let Some(run) = piece.run else {
                continue;
            };
            let id = block_id(block.block_id);
            let placed = place(content, &run, fragment);
            items.push(PageItem::Block(self.block_paint(
                frame,
                id,
                run,
                placed,
                top,
                (LocalCaret::Shown, RemoteCarets::Shown),
            )));
        }
        // The paragraph a split opens, below the block it parts
        // (`PROSE-UI-203`).
        if let (Some(at), Some(ghost)) = (ghost_at, self.ghost().cloned()) {
            let placed = place(content, &ghost.run, at);
            match self.splitting.and_then(|split| split.new_block) {
                Some(new_block) => items.push(PageItem::Block(self.block_paint(
                    frame,
                    new_block,
                    ghost.run,
                    placed,
                    top,
                    (LocalCaret::Shown, RemoteCarets::Hidden),
                ))),
                None => {
                    if let Some(caret) = ghost.caret {
                        let rect = to_rect(&placed, &ghost.run.data().caret_box(caret));
                        let bar = Rect::new(rect.x0, rect.y0, rect.x0 + CARET_WIDTH.get(), rect.y1);
                        frame.ghost_carets.push(GhostCaret {
                            top,
                            clip: placed.clip,
                            bar,
                        });
                    }
                    items.push(PageItem::Shape {
                        run: ghost.run,
                        placed,
                    });
                }
            }
        }
        // The header and the footer, from their own runs. They draw like body
        // text: the local caret and selection show on the page the caret was
        // placed on (`PROSE-UI-033`).
        let local = if self.band_page == Some(page) {
            LocalCaret::Shown
        } else {
            LocalCaret::Hidden
        };
        for slot in self.band_slots(page) {
            let Some(run) = self.drawn_of(slot.block).cloned() else {
                continue;
            };
            let (from, to) = (f64::from(slot.top.get()), f64::from(slot.bottom.get()));
            let placed = Placed {
                origin: Point::new(left + f64::from(run.data().indent().get()), from),
                clip: Rect::new(left, from, right, to),
            };
            let id = block_id(slot.block);
            items.push(PageItem::Block(self.block_paint(
                frame,
                id,
                run,
                placed,
                top,
                (local, RemoteCarets::Hidden),
            )));
        }
        // The footnote band at the foot of the body: a separator, then each
        // note by ordinal, its number then its text (`PROSE-UI-187`).
        let slots = self.note_slots(page);
        if let Some(first) = slots.first() {
            items.push(PageItem::Rule {
                x0: Px::<Surface>::new(left),
                x1: Px::<Surface>::new(left + content_width / 3.0),
                y: Px::<Surface>::new(f64::from((first.top - NOTE_SEPARATOR / 2.0).get())),
            });
        }
        for slot in slots {
            let (from, to) = (f64::from(slot.top.get()), f64::from(slot.bottom.get()));
            let clip = Rect::new(left, from, right, to);
            if let Some(number) = self.shaped.notes.get(&slot.footnote_id).cloned() {
                items.push(PageItem::Block(BlockPaint {
                    key: RunKey::Note(slot.footnote_id),
                    run: number,
                    placed: Placed {
                        origin: Point::new(left, from),
                        clip,
                    },
                    insertion: None,
                    highlights: Vec::new(),
                }));
            }
            if let Some(text) = self.drawn_of(slot.block).cloned() {
                let placed = Placed {
                    origin: Point::new(left + f64::from(text.data().indent().get()), from),
                    clip,
                };
                let id = block_id(slot.block);
                items.push(PageItem::Block(self.block_paint(
                    frame,
                    id,
                    text,
                    placed,
                    top,
                    (LocalCaret::Shown, RemoteCarets::Hidden),
                )));
            }
        }
        frame.pages.push(PagePaint { top, items });
    }

    /// The cells of a table, each with its fill, its border and its blocks.
    /// A merged region is one cell and is drawn once; a covered cell is not
    /// in the shape, so nothing is drawn for it (`PROSE-UI-147`). The text
    /// sits inside the padding, at the cell's vertical alignment
    /// (`PROSE-UI-180`).
    fn table_cells(
        &self,
        frame: &mut Frame,
        table: &TableShape,
        origin: Point,
        clip: Rect,
        page_top: Px<Surface>,
    ) -> Vec<CellPaint> {
        let mut cells = Vec::with_capacity(table.cells.len().to_usize());
        for cell in &table.cells {
            let Some(cell_box) = cell_frame(table, cell) else {
                continue;
            };
            let (left, top_y) = (origin.x, origin.y);
            let bounds = Rect::new(
                left + f64::from(cell_box.x0.get()),
                top_y + f64::from(cell_box.y0.get()),
                left + f64::from(cell_box.x1.get()),
                top_y + f64::from(cell_box.y1.get()),
            );
            let mut blocks = Vec::with_capacity(cell.blocks.len().to_usize());
            for (block, y, run) in &cell.blocks {
                let placed = Placed {
                    origin: Point::new(
                        left + f64::from((cell_box.text_x + run.data().indent()).get()),
                        top_y + f64::from((cell_box.text_y + *y).get()),
                    ),
                    clip: bounds.intersect(clip),
                };
                let id = block_id(*block);
                blocks.push(self.block_paint(
                    frame,
                    id,
                    run.clone(),
                    placed,
                    page_top,
                    (LocalCaret::Shown, RemoteCarets::Hidden),
                ));
            }
            cells.push(CellPaint {
                style: cell.style,
                bounds,
                blocks,
            });
        }
        cells
    }

    /// The walk's record of one block, and the screen's beside it. `shows`
    /// says whether the local caret and selection show in it, and whether
    /// the other peers' carets do.
    fn block_paint(
        &self,
        frame: &mut Frame,
        id: BlockId,
        run: ShapedRun,
        placed: Placed,
        top: Px<Surface>,
        (local, remote_carets): (LocalCaret, RemoteCarets),
    ) -> BlockPaint {
        // The composition, placed before the run is drawn: the text after it
        // moves right by its width (`PROSE-UI-199`).
        let insertion = self
            .local_caret(id, run.data(), local)
            .zip(self.composition())
            .map(|((line, caret), (width, _))| Insertion {
                line,
                x: Px::<Surface>::new(caret.x0),
                width,
            });
        frame.blocks.push(ScreenBlock {
            id,
            run: run.clone(),
            placed,
            top,
            local,
            remote_carets,
        });
        BlockPaint {
            key: RunKey::Block(crate::events::tree_id(id)),
            run,
            placed,
            insertion,
            highlights: self.highlights.get(&id).cloned().unwrap_or_default(),
        }
    }

    /// The local caret in block `id`, when it is drawn there: its line and
    /// its box in the run.
    fn local_caret(
        &self,
        id: BlockId,
        data: &ParleyRunData,
        local: LocalCaret,
    ) -> Option<(Pos<RunLine>, ambre_draw::BoundingBox)> {
        self.cursor
            .as_ref()
            .filter(|cursor| {
                local == LocalCaret::Shown && cursor.block_id == id && cursor.anchor.is_none()
            })
            // Parted and not yet answered, the block leaves its caret to the
            // paragraph drawn below it (`PROSE-UI-203`).
            .filter(|_| {
                !(self.ghost().is_some()
                    && self
                        .splitting
                        .is_some_and(|split| split.block == id && split.new_block.is_none()))
            })
            .map(|cursor| {
                let laid = self.to_laid(id, cursor.offset);
                (
                    data.line_of(laid, self.caret_affinity),
                    data.caret_box_at(laid, self.caret_affinity),
                )
            })
    }

    /// The composition in progress, shaped for the text composed now, and
    /// its width.
    fn composition(&self) -> Option<(Px<Surface>, ShapedRun)> {
        let (shaped_text, shape) = self.compose_shape.as_ref()?;
        (*shaped_text == self.composing).then(|| {
            let width = Px::<Surface>::new(f64::from(shape.data().full_width().get()));
            (width, shape.clone())
        })
    }

    /// Under the glyphs: the local selection, then each peer's.
    fn paint_selections<S: DrawSurface>(
        &self,
        surface: &mut S,
        palette: &PagePalette,
        block: &ScreenBlock,
    ) {
        let (id, data, placed) = (block.id, block.run.data(), &block.placed);
        // The local selection counts the document, draft included, as the
        // local caret does. A peer's counts what the Logic thread answered.
        let laid = |(at, offset): Spot| (at, self.to_laid(at, offset));
        let answered = |(at, offset): Spot| (at, offset.as_axis::<RunChar>());
        if block.local == LocalCaret::Shown
            && let Some((first, last)) = self
                .cursor
                .as_ref()
                .and_then(|cursor| ordered(self, cursor))
            && let Some(span) = covered(self, id, data.len(), (laid(first), laid(last)))
        {
            for rect in data.range_boxes(span) {
                surface.fill_rect(to_rect(placed, &rect), palette.accent.multiply_alpha(0.2));
            }
        }
        for peer in self.remote_at.get(&id).map_or(&[][..], Vec::as_slice) {
            if let Some((first, last)) = ordered(self, &peer.position)
                && let Some(span) = covered(self, id, data.len(), (answered(first), answered(last)))
            {
                let colour = ambre_draw::peer_color(&peer.user_id).multiply_alpha(1.0 / 6.0);
                for rect in data.range_boxes(span) {
                    surface.fill_rect(to_rect(placed, &rect), colour);
                }
            }
        }
    }

    /// The composition in progress at the local caret, and the caret after
    /// it. It never waits on the document (`PROSE-UI-142`).
    fn paint_caret<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        palette: &PagePalette,
        block: &ScreenBlock,
    ) {
        let Some((_, caret)) = self.local_caret(block.id, block.run.data(), block.local) else {
            return;
        };
        let mut rect = to_rect(&block.placed, &caret);
        if let Some((width, shape)) = self.composition() {
            draw_shape(surface, &shape, rect.origin(), palette.text);
            let base = rect.y1 - 1.0;
            surface.stroke_line(
                ambre_draw::Line::new((rect.x0, base), (rect.x0 + width.get(), base)),
                palette.text,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            rect.x0 += width.get();
        }
        let bar = Rect::new(rect.x0, rect.y0, rect.x0 + CARET_WIDTH.get(), rect.y1);
        surface.fill_rect(bar, palette.accent);
        // The host places the input pipe on this bar (`CHROME-007`).
        self.caret_drawn = Some(self.page_frame.transform_rect_bbox(bar));
    }

    /// The carets of the other peers in one block, each with the tab that
    /// stands for its label.
    fn paint_carets<S: DrawSurface>(
        &self,
        surface: &mut S,
        id: BlockId,
        data: &ParleyRunData,
        placed: &Placed,
    ) {
        for peer in self.remote_at.get(&id).map_or(&[][..], Vec::as_slice) {
            let colour = ambre_draw::peer_color(&peer.user_id);
            let caret = to_rect(
                placed,
                &data.caret_box(peer.position.offset.as_axis::<RunChar>()),
            );
            surface.fill_rect(
                Rect::new(caret.x0, caret.y0, caret.x0 + CARET_WIDTH.get(), caret.y1),
                colour.multiply_alpha(0.8),
            );
            surface.fill_rect(
                Rect::new(
                    caret.x0,
                    caret.y0 - LABEL_TAB.get(),
                    caret.x0 + LABEL_TAB.get(),
                    caret.y0,
                ),
                colour,
            );
        }
    }
}

/// A sheet, its shadow and its border, at `top` down the column. The page
/// itself is the sheet, so the walk never draws one.
fn paint_sheet<S: DrawSurface>(
    surface: &mut S,
    palette: &PagePalette,
    top: Px<Surface>,
    width: Px<Surface>,
    height: Px<Surface>,
) {
    let (width, height) = (width.get(), height.get());
    let sheet = Rect::new(0.0, 0.0, width, height);
    let shadow = Rect::new(
        SHADOW_OFFSET.get(),
        SHADOW_OFFSET.get(),
        width + SHADOW_OFFSET.get(),
        height + SHADOW_OFFSET.get(),
    );
    surface.push_transform(Affine::translate((0.0, top.get())));
    surface.fill_rect(shadow, palette.shadow);
    surface.fill_rect(sheet, palette.sheet);
    surface.stroke_rect(
        sheet,
        palette.sheet_border,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );
    surface.pop_transform();
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::events::HeadingLevel;
    use crate::layout::run::RunChar;
    use crate::message::DocChar;
    use ambre_core::index::{Len, Pos, Span};
    use ambre_core::{PrintLength, Pt};

    use ambre_core::Byte;
    use ambre_core::id::{Id, User};
    use ambre_core::{Count, Doc, Px, Surface, View};
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use ambre_draw::{Color, DocumentWalk};

    use super::{PagePalette, PageView, PageWalk};
    use crate::events::{
        BlockId, BlockInit, CursorPosition, ProseCommand, RemoteCursor, block_id, tree_id,
    };
    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use crate::view::cache::{Builds, RunCache, RunKey};
    use crate::view::tests::{append, down, laid_out};
    use crate::view::{Highlight, HighlightKind};

    fn palette() -> PagePalette {
        PagePalette {
            sheet: Color::from_rgba8(255, 255, 255, 255),
            sheet_border: Color::from_rgba8(200, 200, 200, 255),
            shadow: Color::from_rgba8(0, 0, 0, 40),
            text: Color::from_rgba8(20, 20, 20, 255),
            accent: Color::from_rgba8(0, 90, 200, 255),
            warning: Color::from_rgba8(230, 160, 0, 255),
            success: Color::from_rgba8(0, 160, 60, 255),
            error: Color::from_rgba8(200, 30, 30, 255),
        }
    }

    fn view_of(state: &mut ProseState) -> ProseUiState {
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        ui
    }

    fn paint(ui: &mut ProseUiState) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        ui.paint(&mut surface, &palette());
        surface
    }

    /// A page drawn by the screen's own walk and written as a PDF reads back
    /// as its text (`PDF-005`). The paragraph is edited after its first
    /// layout, so the walk draws lines of a run re-shaped in part. A
    /// ligature and a combining mark ride along.
    #[test]
    fn a_page_written_to_pdf_reads_back_as_its_text() {
        use ambre_draw::list::{ListRecorder, PrintDocument, PrintPage, RecorderTables};
        let compact = |t: &str| t.split_whitespace().collect::<String>();
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let words = "The quick brown fox jumps over the lazy dog, then over another one. ";
        let text = format!("{} office cafe\u{301}", words.repeat(4));
        let block = append(&mut state, BlockInit::Paragraph, &text);
        let _ = state.shaped_layout();
        let inserted = "inserted ";
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::new(150),
            text: inserted.to_owned(),
        });
        let expected = format!("{}{inserted}{}", &text[..150], &text[150..]);
        let mut ui = view_of(&mut state);
        let mut document = PrintDocument::new(PrintPage {
            width: PrintLength::<Pt>::new(1200.0),
            height: PrintLength::<Pt>::new(4000.0),
        });
        let mut tables = RecorderTables::default();
        let mut recorder = ListRecorder::new(&mut document, &mut tables);
        ui.paint(&mut recorder, &palette());
        recorder.finish();
        let converter = ambre_convert::Converter::load(&ambre_core::Payload::new(Vec::new()))
            .expect("the host links the engine");
        let Ok(ambre_convert::Converted::File(bytes)) =
            converter.convert(&ambre_convert::Conversion::WritePdf { document })
        else {
            panic!("the page writes");
        };
        let read = pdf_extract::extract_text_from_mem(bytes.as_bytes()).expect("the file reads");
        assert!(
            compact(&read).contains(&compact(&expected)),
            "read back: {read:?}"
        );
    }

    fn sheets(surface: &RecordingSurface) -> usize {
        let sheet = palette().sheet;
        let count =
            surface.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == sheet));
        count.get() as usize
    }

    /// The first block that carries text: the document opens on an empty one.
    fn first_block(ui: &ProseUiState) -> BlockId {
        let block = ui
            .shaped
            .layout
            .blocks
            .iter()
            .find(|block| !ui.shaped.blocks[&block.block_id].data().is_empty())
            .expect("a block with text");
        block_id(block.block_id)
    }

    /// The position of the first op matching `pred`.
    fn first(surface: &RecordingSurface, pred: impl Fn(&DrawOp) -> bool) -> usize {
        surface
            .ops()
            .iter()
            .position(pred)
            .expect("the op was drawn")
    }

    fn glyphs(op: &DrawOp) -> bool {
        matches!(op, DrawOp::Glyphs { .. })
    }

    #[test]
    fn a_long_document_builds_only_the_pages_around_the_canvas() {
        let (_, shaped) = laid_out(400, 60);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let pages = ui.column.page_count;
        assert!(pages.get() > 20, "the document spans {pages} pages");
        let surface = paint(&mut ui);
        let built = ui.column.built_pages(&ui.viewport).expect("a page");
        assert_eq!(sheets(&surface), built.into_iter().count());
        assert!(sheets(&surface) < 5);
        // No block past the built pages holds drawing state.
        for block in &ui.shaped.layout.blocks {
            let run = &ui.shaped.blocks[&block.block_id];
            if block.page_start > built.end {
                assert!(!ui.runs.holds(RunKey::Block(block.block_id), run));
            }
        }
    }

    #[test]
    fn a_run_is_built_on_its_first_paint_and_reused_after() {
        let (_, shaped) = laid_out(6, 30);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        assert_eq!(ui.runs.built(), Count::<Builds>::ZERO);
        let first_paint = paint(&mut ui);
        let built = ui.runs.built();
        assert!(built >= Count::<Builds>::new(6));
        let second_paint = paint(&mut ui);
        assert_eq!(ui.runs.built(), built);
        // A paint reads the layout and changes nothing: two paints agree.
        assert_eq!(first_paint.ops(), second_paint.ops());
    }

    #[test]
    fn the_run_cache_stays_bounded_over_a_long_scroll() {
        let (mut state, shaped) = laid_out(1200, 8);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let bottom = ui.column.max_scroll(&ui.viewport);
        while ui.viewport.scroll_y < bottom {
            super::super::tests::follow(&mut ui, &mut state);
            paint(&mut ui);
            ui.scroll_by(down(800.0));
        }
        paint(&mut ui);
        assert!(ui.runs.built().get() > super::super::cache::RUN_CACHE_RUNS.get());
        assert!(ui.runs.len() <= super::super::cache::RUN_CACHE_RUNS);
    }

    #[test]
    fn selections_go_under_the_glyphs_and_highlights_over_them() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: Pos::<DocChar>::new(12),
            anchor: Some((id, Pos::<DocChar>::new(2))),
        });
        ui.highlights.insert(
            id,
            vec![Highlight {
                kind: HighlightKind::Comment,
                span: crate::view::answered_span(Pos::<DocChar>::new(5), Pos::<DocChar>::new(15)),
            }],
        );
        let surface = paint(&mut ui);
        let pal = palette();
        let selection = pal.accent.multiply_alpha(0.2);
        let comment = pal.highlight(HighlightKind::Comment);
        let under = first(
            &surface,
            |op| matches!(op, DrawOp::FillRect { color, .. } if *color == selection),
        );
        let text = first(&surface, glyphs);
        let over = first(
            &surface,
            |op| matches!(op, DrawOp::FillRect { color, .. } if *color == comment),
        );
        assert!(under < text && text < over);
    }

    #[test]
    fn a_remote_caret_is_painted_after_every_glyph_of_its_page() {
        let (_, shaped) = laid_out(3, 20);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let user = Id::<User>::from_bytes([7; 16]);
        ui.remote_at.insert(
            id,
            vec![RemoteCursor {
                user_id: user,
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Bob").ok(),
                position: CursorPosition {
                    block_id: id,
                    offset: Pos::<DocChar>::new(3),
                    anchor: None,
                },
            }],
        );
        let surface = paint(&mut ui);
        let caret = ambre_draw::peer_color(&user).multiply_alpha(0.8);
        let at = first(
            &surface,
            |op| matches!(op, DrawOp::FillRect { color, .. } if *color == caret),
        );
        let last_glyphs = surface.ops().iter().rposition(glyphs).expect("glyphs");
        assert!(at > last_glyphs);
    }

    #[test]
    fn every_colour_comes_from_the_palette_or_a_peer() {
        let (_, shaped) = laid_out(4, 30);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let user = Id::<User>::from_bytes([9; 16]);
        ui.remote_at.insert(
            id,
            vec![RemoteCursor {
                user_id: user,
                display_name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Eve").ok(),
                position: CursorPosition {
                    block_id: id,
                    offset: Pos::<DocChar>::new(8),
                    anchor: Some((id, Pos::<DocChar>::new(1))),
                },
            }],
        );
        let kinds = [
            HighlightKind::Comment,
            HighlightKind::Insertion,
            HighlightKind::Deletion,
            HighlightKind::Find,
        ];
        let marks = kinds
            .iter()
            .map(|kind| Highlight {
                kind: *kind,
                span: crate::view::answered_span(Pos::<DocChar>::ZERO, Pos::<DocChar>::new(4)),
            })
            .collect();
        ui.highlights.insert(id, marks);
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: Pos::<DocChar>::new(2),
            anchor: None,
        });
        let surface = paint(&mut ui);
        let pal = palette();
        let peer = ambre_draw::peer_color(&user);
        let mut allowed = vec![
            pal.sheet,
            pal.sheet_border,
            pal.shadow,
            pal.text,
            pal.accent,
            pal.error,
            pal.success,
            pal.accent.multiply_alpha(0.2),
            peer,
            peer.multiply_alpha(0.8),
            peer.multiply_alpha(1.0 / 6.0),
        ];
        allowed.extend(kinds.iter().map(|kind| pal.highlight(*kind)));
        for op in surface.ops() {
            let colour = match op {
                DrawOp::FillRect { color, .. }
                | DrawOp::StrokeRect { color, .. }
                | DrawOp::StrokeLine { color, .. } => Some(*color),
                DrawOp::Glyphs { color, .. } => *color,
                _ => None,
            };
            if let Some(colour) = colour {
                assert!(allowed.contains(&colour), "{colour:?} is off the palette");
            }
        }
    }

    #[test]
    fn a_page_carries_its_bands_and_its_footnotes_below_a_separator() {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let body = append(&mut state, BlockInit::Paragraph, "Body text with a note.");
        state.handle_command(ProseCommand::SetHeaderEnabled {
            shown: crate::events::BandState::On,
        });
        state.handle_command(ProseCommand::SetFooterEnabled {
            shown: crate::events::BandState::On,
        });
        let header = state.bands().header.expect("the header block");
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: Pos::<DocChar>::ZERO,
            text: "Header".to_owned(),
        });
        let footer = state.bands().footer.expect("the footer block");
        state.handle_command(ProseCommand::InsertText {
            block_id: footer,
            pos: Pos::<DocChar>::ZERO,
            text: "Footer".to_owned(),
        });
        state.handle_command(ProseCommand::InsertFootnote {
            block_id: body,
            pos: Pos::<DocChar>::new(4),
            note_text: "The note.".to_owned(),
        });
        let mut ui = view_of(&mut state);
        let surface = paint(&mut ui);
        assert_eq!(sheets(&surface), 1);
        let pal = palette();
        for key in [header, footer] {
            let tree = crate::events::tree_id(key);
            let run = &ui.shaped.blocks[&tree];
            assert!(ui.runs.holds(RunKey::Block(tree), run), "a band was drawn");
        }
        let note = *ui.shaped.notes.keys().next().expect("a shaped note");
        assert!(ui.runs.holds(RunKey::Note(note), &ui.shaped.notes[&note]));
        let separator = surface.count(
            |op| matches!(op, DrawOp::StrokeLine { color, .. } if *color == pal.sheet_border),
        );
        assert_eq!(separator.get(), 1);
    }

    #[test]
    fn a_block_split_across_two_pages_is_drawn_on_both() {
        let (_, shaped) = laid_out(3, 900);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(4000.0));
        ui.apply_layout(shaped);
        let split = ui
            .shaped
            .layout
            .blocks
            .iter()
            .find(|block| !block.split_points.is_empty())
            .expect("a split block")
            .block_id;
        let surface = paint(&mut ui);
        let pages = ui
            .shaped
            .layout
            .blocks
            .iter()
            .find(|block| block.block_id == split)
            .map(|block| (block.page_start, block.continuation_heights.len()))
            .expect("the block");
        assert!(pages.1 >= 1);
        // One clip per page each block shows, so a split block has two.
        let clips = surface.count(|op| matches!(op, DrawOp::PushClip { .. }));
        assert!(clips.get() as usize >= ui.shaped.layout.blocks.len() + pages.1);
    }

    #[test]
    fn highlights_are_walked_on_the_blocks_drawn_and_no_other() {
        let (_, shaped) = laid_out(300, 60);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let mark = Highlight {
            kind: HighlightKind::Comment,
            span: crate::view::answered_span(Pos::<DocChar>::ZERO, Pos::<DocChar>::new(3)),
        };
        let ids: Vec<BlockId> = ui
            .shaped
            .layout
            .blocks
            .iter()
            .map(|block| block_id(block.block_id))
            .collect();
        for id in &ids {
            ui.highlights.insert(*id, vec![mark]);
        }
        paint(&mut ui);
        let built = ui.column.built_pages(&ui.viewport).expect("a page");
        let drawn: usize = built.into_iter().map(|page| ui.blocks_on(page).len()).sum();
        assert_eq!(
            ui.highlights_walked,
            ambre_core::Count::<crate::view::walk::Highlights>::of(drawn)
        );
        assert!(drawn < ids.len());
    }

    /// `PROSE-UI-147`: a merged region is drawn once, as wide as the columns
    /// it covers. A covered cell draws nothing, not even the text it kept.
    #[test]
    fn a_merged_region_is_drawn_as_one_cell_and_a_covered_cell_not_at_all() {
        use crate::logic::tables::tests::{cell_at, document_with_table, type_in};
        let (mut state, table) = document_with_table(ambre_core::PeerId::new(1), 2, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let outcome = state.handle_command(ProseCommand::MergeTableCells {
            table_id: block_id(table),
            start: crate::logic::tables::tests::pos((0, 0)),
            end: crate::logic::tables::tests::pos((0, 1)),
        });
        assert!(outcome.update.is_some(), "the merge is written");
        let covered = cell_at(&state, table, 0, 1);
        for (row, col, text) in [(0, 0, "merged"), (1, 0, "left"), (1, 1, "right")] {
            let cell = cell_at(&state, table, row, col);
            type_in(&mut state, cell, text);
        }
        // The covered cell keeps its text, and the region hides it.
        type_in(&mut state, covered, "hidden");
        let mut ui = view_of(&mut state);
        assert!(
            ui.shaped.tables[&table]
                .cells
                .iter()
                .all(|cell| cell.cell != covered)
        );

        let surface = paint(&mut ui);
        let border = palette().sheet_border;
        let page_width = f64::from(ui.shaped.layout.page_width.get());
        let widths: Vec<f64> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokeRect { rect, color, .. }
                    if *color == border && rect.width() < page_width =>
                {
                    Some(rect.width())
                }
                _ => None,
            })
            .collect();
        assert_eq!(widths.len(), 3, "one region and two cells");
        let region = widths.iter().copied().fold(0.0, f64::max);
        let cell = widths.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            (region - 2.0 * cell).abs() < 0.5,
            "the region spans two columns"
        );
        assert_eq!(
            surface.count(glyphs).get(),
            3,
            "the covered text is not drawn"
        );
    }

    /// A styled table: two cells, the first one typed, both laid out and
    /// painted with the shipped faces.
    fn styled_table() -> (ProseState, loro::TreeID, loro::TreeID) {
        use crate::logic::tables::tests::{cell_at, document_with_table, type_in};
        let (mut state, table) = document_with_table(ambre_core::PeerId::new(1), 1, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let first = cell_at(&state, table, 0, 0);
        let second = cell_at(&state, table, 0, 1);
        type_in(&mut state, first, "a");
        type_in(&mut state, second, "b");
        (state, first, second)
    }

    fn style(state: &mut ProseState, cell: loro::TreeID, edit: impl FnOnce(&mut ProseCommand)) {
        let mut command = ProseCommand::SetCellStyle {
            cell_id: block_id(cell),
            bg: None,
            valign: None,
            border: None,
            padding: None,
        };
        edit(&mut command);
        assert!(
            state.handle_command(command).update.is_some(),
            "the style is written"
        );
    }

    /// Where the leftmost glyph run is drawn: the first cell's text.
    fn first_text_at(surface: &RecordingSurface) -> (f64, f64) {
        surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::Glyphs { transform, .. } => {
                    let at = transform.translation();
                    Some((at.x, at.y))
                }
                _ => None,
            })
            .fold(
                (f64::MAX, 0.0),
                |best, at| if at.0 < best.0 { at } else { best },
            )
    }

    /// `PROSE-UI-179`, `PROSE-UI-181`: a styled cell draws its fill and its
    /// four edges in its colour; the cell beside it keeps the hairline.
    #[test]
    fn a_cell_draws_its_fill_and_its_border_and_a_plain_one_the_hairline() {
        use crate::message::CellBorder;
        let (mut state, first, _) = styled_table();
        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { bg, border, .. } = command {
                *bg = Some(ambre_core::Rgb::from_bits(0x00ff_8800));
                *border = Some(CellBorder {
                    width: PrintLength::<Pt>::new(3.0),
                    color: ambre_core::Rgb::from_bits(0x0012_3456),
                });
            }
        });
        let mut ui = view_of(&mut state);
        let surface = paint(&mut ui);
        let fill = Color::from_rgba8(0xff, 0x88, 0x00, 0xff);
        let edge = Color::from_rgba8(0x12, 0x34, 0x56, 0xff);
        let fills = |colour: Color| {
            surface
                .ops()
                .iter()
                .filter(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == colour))
                .count()
        };
        assert_eq!(fills(fill), 1, "one fill");
        assert_eq!(fills(edge), 4, "four edges");
        let page_width = f64::from(ui.shaped.layout.page_width.get());
        let hairlines = surface
            .ops()
            .iter()
            .filter(|op| {
                matches!(op, DrawOp::StrokeRect { rect, color, .. }
                    if *color == palette().sheet_border && rect.width() < page_width)
            })
            .count();
        assert_eq!(hairlines, 1, "the plain cell alone keeps the hairline");
    }

    /// `PROSE-UI-180`: the padding moves the text in and makes the row taller
    /// by twice its width. The bottom alignment drops the text in a row a
    /// taller neighbour sized.
    #[test]
    fn the_padding_insets_the_text_and_the_alignment_places_it() {
        let (mut state, first, second) = styled_table();
        let mut ui = view_of(&mut state);
        let (x0, y0) = first_text_at(&paint(&mut ui));
        let table = |ui: &ProseUiState| {
            ui.shaped
                .tables
                .values()
                .next()
                .map_or(Px::<Doc>::ZERO, |t| {
                    t.row_tops.iter().last().copied().unwrap_or(Px::<Doc>::ZERO)
                })
        };
        let height0 = table(&ui);

        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { padding, .. } = command {
                *padding = Some(PrintLength::<Pt>::new(12.0));
            }
        });
        let mut ui = view_of(&mut state);
        let (x1, y1) = first_text_at(&paint(&mut ui));
        let pad = f64::from(PrintLength::<Pt>::new(12.0).to_doc_px().get());
        assert!(
            (x1 - x0 - pad).abs() < 0.5,
            "the text moves in by the padding"
        );
        assert!((y1 - y0 - pad).abs() < 0.5, "and down by it");
        assert!(
            (f64::from((table(&ui) - height0).get()) - 2.0 * pad).abs() < 0.5,
            "the row grows by both sides"
        );

        // The neighbour pads more, so the first cell has room below its text.
        style(&mut state, second, |command| {
            if let ProseCommand::SetCellStyle { padding, .. } = command {
                *padding = Some(PrintLength::<Pt>::new(48.0));
            }
        });
        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { valign, .. } = command {
                *valign = Some(crate::message::VAlign::Bottom);
            }
        });
        let mut ui = view_of(&mut state);
        let (_, y2) = first_text_at(&paint(&mut ui));
        let room = 2.0 * f64::from(PrintLength::<Pt>::new(36.0).to_doc_px().get());
        assert!((y2 - y1 - room).abs() < 0.5, "the text sits at the bottom");
    }

    /// Shape the paragraphs the draft changes, as the host does each frame.
    fn shape(ui: &mut ProseUiState) {
        let mut font_cx = parley::FontContext::new();
        for bytes in crate::layout::fonts::shipped_faces() {
            crate::layout::fonts::register_faces(&mut font_cx, bytes);
        }
        let mut layout_cx = parley::LayoutContext::new();
        ui.shape_draft(&mut font_cx, &mut layout_cx);
    }

    /// The text of the paragraph drawn for `block` in place of its run.
    fn provisional(ui: &ProseUiState, block: BlockId) -> String {
        ui.provisional_run(block)
            .expect("the paragraph is shaped again")
            .data()
            .text()
            .to_owned()
    }

    fn drawn_glyphs(surface: &RecordingSurface, n: u32) -> bool {
        surface
            .ops()
            .iter()
            .any(|op| matches!(op, DrawOp::Glyphs { glyph_count, .. } if glyph_count.get() == n))
    }

    /// `PROSE-UI-142`: a composition opened away from a draft still in
    /// flight is drawn at once, in glyphs of its own at the caret.
    #[test]
    fn a_composition_away_from_the_draft_is_drawn_at_once() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "abc",
            &id,
            Pos::<DocChar>::new(14),
        );
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: Pos::<DocChar>::ZERO,
            anchor: None,
        });
        ui.composing = "ni".to_owned();
        shape(&mut ui);
        let surface = paint(&mut ui);
        assert!(
            drawn_glyphs(&surface, 17),
            "the draft is drawn in its paragraph"
        );
        assert!(drawn_glyphs(&surface, 2), "the composition is drawn");
    }

    /// `PROSE-UI-134`: the draft is drawn at its anchor before the document
    /// has answered, in the paragraph shaped again around it.
    #[test]
    fn a_draft_is_drawn_before_the_document_answers() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        assert!(drawn_glyphs(&paint(&mut ui), 14));
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "abc",
            &id,
            Pos::<DocChar>::new(4),
        );
        shape(&mut ui);
        assert_eq!(provisional(&ui, id), "wordabc word word");
        assert!(drawn_glyphs(&paint(&mut ui), 17));
    }

    /// `PROSE-UI-134`: a draft typed into a header is drawn there before the
    /// document answers, as body text is.
    #[test]
    fn a_draft_in_a_header_is_drawn_before_the_document_answers() {
        let mut state =
            ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        append(&mut state, BlockInit::Paragraph, "Body");
        state.handle_command(ProseCommand::SetHeaderEnabled {
            shown: crate::events::BandState::On,
        });
        let header = state.bands().header.expect("the header block");
        let mut ui = view_of(&mut state);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "Head",
            &header,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        assert_eq!(provisional(&ui, header), "Head");
        assert!(drawn_glyphs(&paint(&mut ui), 4));
    }

    /// `PROSE-UI-213`: a draft typed into a cell is drawn there before the
    /// document answers, wrapped at its column's width. Its row grows until
    /// the answer, and the cell beside it with it.
    #[test]
    fn a_draft_in_a_cell_is_drawn_before_the_document_answers() {
        use crate::logic::tables::tests::{cell_at, document_with_table, first_block_of};
        let (mut state, table) = document_with_table(ambre_core::PeerId::new(1), 1, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let cell = cell_at(&state, table, 0, 0);
        let block = block_id(first_block_of(&state, cell));
        let mut ui = view_of(&mut state);
        let laid = ui.shaped.tables[&table].clone();
        let row = laid.row_tops.as_slice()[1] - laid.row_tops.as_slice()[0];

        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "Cell",
            &block,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        assert_eq!(provisional(&ui, block), "Cell");
        assert!(drawn_glyphs(&paint(&mut ui), 4));

        // A column is half the text wide: this wraps over many lines.
        let long = "word ".repeat(60);
        ui.draft.clear();
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            &long,
            &block,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        let run = ui.provisional_run(block).expect("the cell is shaped again");
        let first = &laid.cells[Pos::<crate::layout::run::ShapeCell>::ZERO];
        let text_width = first.width - first.style.padding * 2.0;
        assert!(
            run.data().line_count().get() > 1,
            "the draft wraps in its column"
        );
        let widest = (0..=run.data().len().get())
            .map(|at| run.data().caret_box(Pos::<RunChar>::new(at)).x0)
            .fold(0.0, f64::max);
        assert!(
            widest <= f64::from(text_width.get()) + 0.5,
            "at the column's width"
        );
        let (pieces, _) = ui.pieces(Pos::<crate::events::Page>::new(0));
        let grown = pieces
            .iter()
            .find_map(|piece| piece.table.clone())
            .expect("the table is drawn");
        let tall = grown.row_tops.as_slice()[1] - grown.row_tops.as_slice()[0];
        assert!(tall > row + run.data().height() / 2.0, "the row grows");
    }

    /// Where offset `at` of the paragraph drawn for `block` sits, from its
    /// left edge.
    fn x_of(ui: &ProseUiState, block: BlockId, at: u32) -> f64 {
        ui.run_of(block)
            .expect("a run")
            .caret_box(Pos::<RunChar>::new(at))
            .x0
    }

    /// `PROSE-UI-199`: the text after the draft is drawn after it, a draft's
    /// width to the right of where it was, and never under it.
    #[test]
    fn the_text_after_a_draft_moves_right_by_its_width() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let after = x_of(&ui, id, 4);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "abc",
            &id,
            Pos::<DocChar>::new(4),
        );
        shape(&mut ui);
        let width = x_of(&ui, id, 7) - x_of(&ui, id, 4);
        assert!(width > 0.0, "the draft takes room");
        assert!((x_of(&ui, id, 7) - (after + width)).abs() < 0.5);
    }

    /// The width of `text` typed at the start of `block`, as drawn.
    fn draft_width(ui: &mut ProseUiState, block: BlockId, text: &str) -> f64 {
        ui.draft.clear();
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            text,
            &block,
            Pos::<DocChar>::ZERO,
        );
        shape(ui);
        x_of(ui, block, ambre_core::char_len(text).get())
    }

    /// `PROSE-UI-200`: the draft takes the style its block is laid out in,
    /// even when the same text was shaped for another block just before.
    #[test]
    fn a_draft_in_a_heading_is_drawn_as_the_heading() {
        let (mut state, _) = laid_out(1, 3);
        let heading = append(
            &mut state,
            BlockInit::Heading {
                level: HeadingLevel::H1,
            },
            "Title",
        );
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        let paragraph = first_block(&ui);
        let laid = ui.shaped.blocks[&tree_id(heading)].data().style().cloned();
        let plain = draft_width(&mut ui, paragraph, "abc");
        let titled = draft_width(&mut ui, heading, "abc");
        let run = ui
            .provisional_run(heading)
            .expect("the heading is shaped again");
        assert_eq!(
            run.data().style(),
            laid.as_ref(),
            "the draft is in the heading's style"
        );
        assert!(titled > plain, "the heading's bold reaches the draft");
    }

    /// `PROSE-UI-208`: a draft typed right after a bold word is shaped
    /// bold, and one typed after plain text is not.
    #[test]
    fn a_draft_after_a_bold_word_is_drawn_bold() {
        let (mut state, _) = laid_out(1, 2);
        let block = *state.root_blocks().last().expect("a block");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..4),
            key: "b".into(),
            value: None,
        });
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let bold_at = |ui: &mut ProseUiState, at: u32| {
            ui.draft.clear();
            ui.draft.push_str(
                ambre_core::Seq::<crate::message::TextAck>::nth(1),
                "abc",
                &id,
                Pos::<DocChar>::new(at),
            );
            shape(ui);
            let run = ui
                .provisional_run(id)
                .expect("the paragraph is shaped again");
            let (start, end) = (at as usize, at as usize + 3);
            run.data().marks().iter().any(|span| {
                span.bold == ambre_core::Weight::Bold
                    && span.start <= Pos::<Byte>::from_usize(start)
                    && span.end >= Pos::<Byte>::from_usize(end)
            })
        };
        assert!(bold_at(&mut ui, 4), "after the bold word");
        assert!(!bold_at(&mut ui, 9), "after the plain word");
        assert!(!bold_at(&mut ui, 0), "before any text");
    }

    /// `PROSE-UI-209`: an underline and a strike-through are each drawn as
    /// one line across the text they cover, the strike above the underline.
    #[test]
    fn an_underline_and_a_strike_through_are_drawn_across_their_text() {
        let (mut state, _) = laid_out(1, 2);
        let block = *state.root_blocks().last().expect("a block");
        for key in ["u", "s"] {
            state.handle_command(ProseCommand::ApplyMark {
                block_id: block,
                range: Span::<DocChar>::from_usize_range(0..4),
                key: key.into(),
                value: None,
            });
        }
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        let text = palette().text;
        let surface = paint(&mut ui);
        let lines: Vec<(f64, f64, f64)> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokeLine { line, color, .. } if *color == text => {
                    Some((line.p0.x, line.p1.x, line.p0.y))
                }
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 2, "one underline and one strike-through");
        let (under, strike) = (lines[0], lines[1]);
        assert!(under.1 > under.0, "the underline spans the word");
        assert!((under.0 - strike.0).abs() < 0.01 && (under.1 - strike.1).abs() < 0.01);
        assert!(strike.2 < under.2, "the strike sits above the underline");
    }

    /// `PROSE-UI-208`: a draft typed right after an underlined word is
    /// drawn underlined, one line running from the word through the draft.
    #[test]
    fn a_draft_after_an_underlined_word_is_drawn_underlined() {
        let (mut state, _) = laid_out(1, 2);
        let block = *state.root_blocks().last().expect("a block");
        state.handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..4),
            key: "u".into(),
            value: None,
        });
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let text = palette().text;
        let lines = |ui: &mut ProseUiState| -> Vec<(f64, f64)> {
            paint(ui)
                .ops()
                .iter()
                .filter_map(|op| match op {
                    DrawOp::StrokeLine { line, color, .. } if *color == text => {
                        Some((line.p0.x, line.p1.x))
                    }
                    _ => None,
                })
                .collect()
        };
        let word = lines(&mut ui);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "abc",
            &id,
            Pos::<DocChar>::new(4),
        );
        shape(&mut ui);
        let drawn = lines(&mut ui);
        let (Some(word), Some(line)) = (word.first(), drawn.first()) else {
            panic!("the word and the draft are underlined");
        };
        assert_eq!(drawn.len(), 1, "one line under the word and the draft");
        assert!((line.0 - word.0).abs() < 0.5, "it starts with the word");
        assert!(line.1 > word.1 + 1.0, "it runs on under the draft");
    }

    /// `PROSE-UI-201`: a draft longer than its line wraps where the
    /// document will wrap the paragraph, before the document answers.
    #[test]
    fn a_long_draft_wraps_as_the_paragraph_will() {
        let (mut state, _) = laid_out(1, 3);
        let block = *state.root_blocks().last().expect("a block");
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let typed = " more".repeat(40);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            &typed,
            &id,
            Pos::<DocChar>::new(14),
        );
        shape(&mut ui);
        let drawn = ui
            .provisional_run(id)
            .expect("the paragraph is shaped again");
        state.handle_command(ProseCommand::InsertText {
            block_id: id,
            pos: Pos::<DocChar>::new(14),
            text: typed.clone(),
        });
        let answered = state.shaped_layout();
        let laid = answered.blocks[&tree_id(block)].data();
        assert!(drawn.data().line_count().get() > 1, "the draft wraps");
        assert_eq!(drawn.data().text(), laid.text());
        assert_eq!(drawn.data().line_tops(), laid.line_tops());
        let starts = |run: &crate::layout::run::ParleyRunData| {
            (0..run.line_count().get())
                .map(|line| {
                    run.at_line(
                        Pos::<crate::layout::run::RunLine>::new(line),
                        Px::<Doc>::new(0.0),
                    )
                    .0
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(starts(drawn.data()), starts(laid), "the lines break alike");
    }

    /// `PROSE-UI-202`: the text a deletion takes out is gone from the screen
    /// before the document answers.
    #[test]
    fn deleted_text_leaves_the_screen_before_the_answer() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.draft.push_delete(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            &id,
            Pos::<DocChar>::ZERO,
            Pos::<DocChar>::new(5),
        );
        shape(&mut ui);
        assert_eq!(provisional(&ui, id), "word word");
        let surface = paint(&mut ui);
        assert!(drawn_glyphs(&surface, 9));
        assert!(!drawn_glyphs(&surface, 14), "the old line is not drawn");
    }

    /// `PROSE-UI-203`: the paragraph a split opens is drawn below the part
    /// it leaves, with the text typed into it, before the document answers.
    /// The blocks after it move down to make room.
    #[test]
    fn the_paragraph_a_split_opens_is_drawn_before_the_answer() {
        let (_, shaped) = laid_out(2, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let first = first_block(&ui);
        let top_of_second = |ui: &ProseUiState| {
            let pieces = ui.pieces(Pos::<crate::events::Page>::new(0)).0;
            let at = pieces
                .iter()
                .position(|piece| {
                    ui.shaped.block(piece.index).map(|b| b.block_id) == Some(tree_id(first))
                })
                .expect("the split block is drawn");
            pieces[at + 1].fragment.top
        };
        let before = top_of_second(&ui);
        ui.splitting = Some(crate::view::provisional::Splitting {
            block: first,
            at: Pos::<DocChar>::new(10),
            new_block: None,
        });
        // Typed while the split is unanswered: held, and drawn.
        ui.queued
            .push_back(crate::input::ProseInput::Text("xy".to_owned()));
        shape(&mut ui);
        assert_eq!(provisional(&ui, first), "word word ");
        let ghost = ui.ghost().expect("the new paragraph is shaped").run.clone();
        assert_eq!(ghost.data().text(), "xyword");
        let surface = paint(&mut ui);
        assert!(drawn_glyphs(&surface, 6), "the new paragraph is drawn");
        let moved = top_of_second(&ui) - before;
        assert!(
            (moved - ghost.data().height()).abs() < Px::<Doc>::new(0.5),
            "the next block moves down by the new paragraph"
        );
        // The document answers with the block it minted: the text typed
        // into it is drawn there, and the caret with it.
        let minted = BlockId::new(ambre_core::PeerId::new(9), ambre_core::OpCounter::new(9));
        ui.apply_event(&crate::events::ProseEvent::BlockSplit {
            source: first,
            new_block: minted,
        });
        ui.queued.clear();
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "xy",
            &minted,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        assert_eq!(provisional(&ui, minted), "xyword");
        assert_eq!(ui.block_len(minted), Len::<DocChar>::new(6));
    }

    /// `PROSE-UI-214`: a press in the paragraph a split opens lands in the
    /// block the document minted, before a layout carries it. It still does
    /// when an input replayed since has made the drawn paragraph stale.
    #[test]
    fn a_press_in_the_paragraph_a_split_opens_lands_in_it() {
        let (_, shaped) = laid_out(2, 3);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let first = first_block(&ui);
        ui.splitting = Some(crate::view::provisional::Splitting {
            block: first,
            at: Pos::<DocChar>::new(10),
            new_block: None,
        });
        let minted = BlockId::new(ambre_core::PeerId::new(9), ambre_core::OpCounter::new(9));
        ui.apply_event(&crate::events::ProseEvent::BlockSplit {
            source: first,
            new_block: minted,
        });
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "xy",
            &minted,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        let at = ui
            .pieces(Pos::<crate::events::Page>::new(0))
            .1
            .expect("the new paragraph is drawn");
        let run = ui.ghost().expect("shaped").run.clone();
        let caret = run.data().caret_box(Pos::<RunChar>::new(3));
        let layout = &ui.shaped.layout;
        let x = layout.margin_left
            + run.data().indent()
            + Px::<Doc>::new(caret.x0 as f32)
            + Px::<Doc>::new(0.5);
        let y = ui.column.page_top_y(Pos::<crate::events::Page>::new(0))
            + layout.body_top()
            + at.top
            + Px::<Doc>::new((caret.y0 + caret.y1) as f32 / 2.0);
        let scale = ui.viewport.scale();
        let point = (
            ui.column.left(&ui.viewport) + Px::<View>::new(x.get() * scale),
            (y - ui.viewport.scroll_y).get() * scale,
        );
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0.get(), point.1)),
            Some((minted, Pos::<DocChar>::new(3)))
        );
        // A keystroke replayed before the next frame shapes again.
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(2),
            "z",
            &minted,
            Pos::<DocChar>::new(2),
        );
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0.get(), point.1)),
            Some((minted, Pos::<DocChar>::new(3)))
        );
    }

    /// The screen point of offset `at` in the text of `block`, a cell of
    /// the table drawn on page 0.
    fn cell_point(ui: &ProseUiState, block: BlockId, at: u32) -> (f32, f32) {
        let (pieces, _) = ui.pieces(Pos::<crate::events::Page>::new(0));
        let piece = pieces
            .iter()
            .find(|piece| piece.table.is_some())
            .expect("the table is drawn");
        let table = piece.table.as_ref().expect("a table");
        let (frame, top, run) = table
            .cells
            .iter()
            .find_map(|cell| {
                let (_, top, run) = cell
                    .blocks
                    .iter()
                    .find(|(held, _, _)| block_id(*held) == block)?;
                Some((
                    crate::view::hit::cell_frame(table, cell)?,
                    *top,
                    run.clone(),
                ))
            })
            .expect("the block is in a cell");
        let run = ui.provisional_run(block).cloned().unwrap_or(run);
        let caret = run.data().caret_box(Pos::<RunChar>::new(at));
        let layout = &ui.shaped.layout;
        let x = layout.margin_left
            + frame.text_x
            + run.data().indent()
            + Px::<Doc>::new(caret.x0 as f32)
            + Px::<Doc>::new(0.5);
        let y = ui.column.page_top_y(Pos::<crate::events::Page>::new(0))
            + layout.body_top()
            + piece.fragment.top
            + frame.text_y
            + top
            + Px::<Doc>::new((caret.y0 + caret.y1) as f32 / 2.0);
        let scale = ui.viewport.scale();
        (
            (ui.column.left(&ui.viewport) + Px::<View>::new(x.get() * scale)).get(),
            (y - ui.viewport.scroll_y).get() * scale,
        )
    }

    /// `PROSE-UI-216`: a press in a cell lands in its text, at the offset
    /// under the pointer. It does so in a laid run and in a draft.
    #[test]
    fn a_press_in_a_cell_lands_in_its_text() {
        use crate::logic::tables::tests::{cell_at, document_with_table, first_block_of, type_in};
        let (mut state, table) = document_with_table(ambre_core::PeerId::new(1), 1, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let (left, right) = (cell_at(&state, table, 0, 0), cell_at(&state, table, 0, 1));
        type_in(&mut state, right, "abcdef");
        let (left, right) = (
            block_id(first_block_of(&state, left)),
            block_id(first_block_of(&state, right)),
        );
        let mut ui = view_of(&mut state);
        let point = cell_point(&ui, right, 3);
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0, point.1)),
            Some((right, Pos::<DocChar>::new(3)))
        );

        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            &"word ".repeat(60),
            &left,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        let point = cell_point(&ui, left, 200);
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0, point.1)),
            Some((left, Pos::<DocChar>::new(200)))
        );
        let point = cell_point(&ui, right, 3);
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0, point.1)),
            Some((right, Pos::<DocChar>::new(3)))
        );
    }

    /// `PROSE-UI-216`: a press in a cell reads the provisional run drawn
    /// there as a document offset. A stale run still counts so.
    #[test]
    fn a_press_in_a_stale_cell_draft_lands_where_it_was_drawn() {
        use crate::logic::tables::tests::{cell_at, document_with_table, first_block_of, type_in};
        let (mut state, table) = document_with_table(ambre_core::PeerId::new(1), 1, 1);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let cell = cell_at(&state, table, 0, 0);
        type_in(&mut state, cell, "abcdef");
        let cell = block_id(first_block_of(&state, cell));
        let mut ui = view_of(&mut state);
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(1),
            "xy",
            &cell,
            Pos::<DocChar>::ZERO,
        );
        shape(&mut ui);
        let point = cell_point(&ui, cell, 1);
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0, point.1)),
            Some((cell, Pos::<DocChar>::new(1)))
        );
        // A keystroke replayed before the next frame, after the point.
        ui.draft.push_str(
            ambre_core::Seq::<crate::message::TextAck>::nth(2),
            "z",
            &cell,
            Pos::<DocChar>::new(2),
        );
        assert_eq!(
            ui.place_at(crate::view::tests::view_at(point.0, point.1)),
            Some((cell, Pos::<DocChar>::new(1)))
        );
    }

    fn thread(block: BlockId, start: u32, end: u32) -> crate::events::CommentThread {
        crate::events::CommentThread {
            comment_id: Id::mint(),
            anchor_block: Some(block),
            anchor_start: Pos::<DocChar>::new(start),
            anchor_end: Pos::<DocChar>::new(end),
            orphaned: crate::events::AnchorState::Anchored,
            resolved: ambre_core::ThreadState::Open,
            replies: Vec::new(),
        }
    }

    /// The tinted rectangles of `color`, left to right.
    fn tints(surface: &RecordingSurface, tint: Color) -> Vec<(f64, f64)> {
        surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::FillRect { rect, color } if *color == tint => Some((rect.x0, rect.x1)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_thread_the_logic_thread_sends_is_painted_over_its_range_and_no_other() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let comment = palette().highlight(HighlightKind::Comment);
        assert!(tints(&paint(&mut ui), comment).is_empty());
        ui.apply_event(&crate::events::ProseEvent::CommentsUpdated {
            threads: vec![thread(id, 4, 9)],
        });
        let surface = paint(&mut ui);
        let data = ui.shaped.blocks[&crate::events::tree_id(id)].data();
        let (start, end) = (
            data.caret_box(Pos::<RunChar>::new(4)).x0,
            data.caret_box(Pos::<RunChar>::new(9)).x0,
        );
        let painted = tints(&surface, comment);
        assert_eq!(painted.len(), 1);
        let (x0, x1) = painted[0];
        assert!((x1 - x0 - (end - start)).abs() < 0.5, "{painted:?}");
    }

    #[test]
    fn two_overlapping_threads_are_both_painted() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.apply_event(&crate::events::ProseEvent::CommentsUpdated {
            threads: vec![thread(id, 2, 10), thread(id, 6, 14)],
        });
        let surface = paint(&mut ui);
        let painted = tints(&surface, palette().highlight(HighlightKind::Comment));
        assert_eq!(painted.len(), 2);
        // The second overlaps the first: each is drawn whole, one over the other.
        assert!(painted[1].0 < painted[0].1);
    }

    #[test]
    fn an_insertion_is_underlined_and_a_deletion_struck_through() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let mark = |kind| Highlight {
            kind,
            span: crate::view::answered_span(Pos::<DocChar>::new(2), Pos::<DocChar>::new(6)),
        };
        ui.highlights.insert(
            id,
            vec![
                mark(HighlightKind::Insertion),
                mark(HighlightKind::Deletion),
            ],
        );
        let surface = paint(&mut ui);
        let pal = palette();
        let tint = |color| {
            surface
                .ops()
                .iter()
                .find_map(|op| match op {
                    DrawOp::FillRect { rect, color: c } if *c == color => Some(*rect),
                    _ => None,
                })
                .expect("the tint")
        };
        let line = |color| {
            surface
                .ops()
                .iter()
                .find_map(|op| match op {
                    DrawOp::StrokeLine { line, color: c, .. } if *c == color => Some(line.p0.y),
                    _ => None,
                })
                .expect("the line")
        };
        let inserted = tint(pal.highlight(HighlightKind::Insertion));
        let deleted = tint(pal.highlight(HighlightKind::Deletion));
        // Under the text for an insertion, through its middle for a deletion.
        assert!(line(pal.success) > inserted.y0 + inserted.height() * 0.75);
        let middle = (deleted.y0 + deleted.y1) / 2.0;
        assert!((line(pal.error) - middle).abs() < 0.5);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(32))]

        /// The screen draws the pages' walk unchanged among its own layers,
        /// whatever the zoom, scroll and canvas (`CANVAS-074`).
        #[test]
        fn the_screen_draws_the_walk_at_any_camera(
            zoom in 0.25f32..4.0,
            scroll in 0.0f32..1.0,
            width in 320.0f32..2400.0,
            height in 240.0f32..1600.0,
        ) {
            let (_, shaped) = laid_out(1, 20);
            let mut ui = ProseUiState::default();
            ui.resize(Px::<View>::new(width), Px::<View>::new(height));
            ui.apply_layout(shaped);
            ui.viewport.zoom = ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(f64::from(zoom));
            let bottom = ui.column.page_top_y(Pos::<crate::events::Page>::new(1)).get();
            ui.viewport.scroll_y = ui.column.clamp_scroll(&ui.viewport, Px::<Doc>::new(scroll * bottom));
            let id = first_block(&ui);
            ui.cursor = Some(CursorPosition {
                block_id: id,
                offset: Pos::<DocChar>::new(12),
                anchor: Some((id, Pos::<DocChar>::new(2))),
            });
            ui.highlights.insert(
                id,
                vec![Highlight {
                    kind: HighlightKind::Comment,
                    span: crate::view::answered_span(Pos::<DocChar>::new(5), Pos::<DocChar>::new(15)),
                }],
            );
            let screen = paint(&mut ui);

            let mut frame = super::Frame::default();
            for page in ui.column.built_pages(&ui.viewport).into_iter().flatten() {
                let top = Px::<Surface>::new(f64::from(ui.column.page_top_y(page).get()));
                ui.resolve_page(&mut frame, page, top);
            }
            let region = ui.walk_region(ui.camera(), &frame);
            let palette = palette();
            let view = PageView {
                palette: &palette,
                width: Px::<Surface>::new(f64::from(ui.shaped.layout.page_width.get())),
                height: Px::<Surface>::new(f64::from(ui.shaped.layout.page_height.get())),
                pages: &frame.pages,
            };
            let mut runs = RunCache::default();
            let mut walked = RecordingSurface::new();
            PageWalk {
                runs: &mut runs,
                highlights_walked: ambre_core::Count::<crate::view::walk::Highlights>::ZERO,
            }
            .walk(&view, region, &mut walked);
            proptest::prop_assert!(walked.count(glyphs) > ambre_core::Count::<ambre_draw::testing::Ops>::ZERO);
            proptest::prop_assert!(screen.holds_walk(&walked));
        }
    }
}
