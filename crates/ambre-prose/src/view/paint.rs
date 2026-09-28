//! Painting the pages (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! One implementation, generic over the drawing surface: the screen and the
//! PDF export call the same code (`PROSE-UI-018`). Every colour comes from the
//! palette the host fills from the theme tokens, or from a peer's own colour
//! (`PROSE-UI-014`). A paint reads the layout in force and writes nothing to
//! the document (`PROSE-UI-120`).

use ambre_draw::{Affine, Brush, Color, DrawSurface, PositionedLayoutItem, Rect};

use crate::events::{BlockId, CursorPosition, block_id};
use crate::layout::measure::{EffectiveStyle, shape_draft};
use crate::layout::place::NOTE_SEPARATOR_PX;
use crate::layout::run::{CellStyle, ParleyRunData, ShapedRun, TableShape};
use crate::ui::ProseUiState;
use crate::view::HighlightKind;
use crate::view::cache::{RunCache, RunKey};
use crate::view::hit::{Fragment, cell_frame};

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

/// How far the shadow falls from the sheet.
const SHADOW_OFFSET_PX: f64 = 3.0;
/// The width of the local caret and of a remote one.
const CARET_WIDTH_PX: f64 = 1.5;
/// The tab a remote caret carries above its bar, standing for its label.
const LABEL_TAB_PX: f64 = 5.0;

impl PagePalette {
    fn highlight(&self, kind: HighlightKind) -> Color {
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
struct Placed {
    origin_x: f64,
    origin_y: f64,
    clip: Rect,
}

/// Where a run is drawn: from the left of the content box, at the top of
/// its fragment, clipped to it.
fn place(left: f64, body_top: f64, content_width: f64, run: &ShapedRun, piece: Fragment) -> Placed {
    let clip_top = body_top + f64::from(piece.top);
    Placed {
        origin_x: left + f64::from(run.data().indent_px()),
        origin_y: clip_top - f64::from(piece.run_top),
        clip: Rect::new(
            left,
            clip_top,
            left + content_width,
            clip_top + f64::from(piece.height),
        ),
    }
}

/// The code-point range of `block` a selection from `start` to `end` covers,
/// the two ends already in flow order.
fn covered(
    ui: &ProseUiState,
    block: BlockId,
    len: u32,
    ends: ((BlockId, u32), (BlockId, u32)),
) -> Option<(u32, u32)> {
    let ((first, from), (last, to)) = ends;
    // Within one block, the flow is not asked: a header or a footer holds
    // its own selection and has no place in it.
    if first == last {
        return (first == block && from < to).then_some((from, to.min(len)));
    }
    let here = ui.flow_index(&crate::events::tree_id(block))?;
    let first_index = ui.flow_index(&crate::events::tree_id(first))?;
    let last_index = ui.flow_index(&crate::events::tree_id(last))?;
    if here < first_index || here > last_index {
        return None;
    }
    let start = if here == first_index { from } else { 0 };
    let end = if here == last_index { to } else { len };
    (start < end).then_some((start, end))
}

/// The two ends of a selection in flow order, or `None` for a caret alone.
fn ordered(ui: &ProseUiState, cursor: &CursorPosition) -> Option<((BlockId, u32), (BlockId, u32))> {
    let anchor = cursor.anchor?;
    let focus = (cursor.block_id, cursor.offset);
    let key = |(block, offset): (BlockId, u32)| {
        (
            ui.flow_index(&crate::events::tree_id(block))
                .unwrap_or(usize::MAX),
            offset,
        )
    };
    if key(anchor) <= key(focus) {
        Some((anchor, focus))
    } else {
        Some((focus, anchor))
    }
}

fn to_rect(placed: &Placed, rect: &ambre_draw::BoundingBox) -> Rect {
    Rect::new(
        placed.origin_x + rect.x0,
        placed.origin_y + rect.y0,
        placed.origin_x + rect.x1,
        placed.origin_y + rect.y1,
    )
}

/// A text the view draws on a line ahead of the layout: the draft or the
/// composition. The glyphs after its point move right by its width.
#[derive(Clone, Copy)]
struct Insertion {
    line: usize,
    x: f64,
    width: f64,
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
fn draw_shape<S: DrawSurface>(surface: &mut S, shape: &ShapedRun, x: f64, y: f64, color: Color) {
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
        wanted: Option<(BlockId, u32, String)>,
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
                    &ambre_bus::app::prose::ParagraphStyle::default(),
                    false,
                )
            });
        // Typed after a marked span, the text lands in it (`PROSE-UI-208`).
        let marks = [crate::layout::measure::MarkedSpan {
            start: 0,
            end: text.len(),
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
    /// Per page, the sheet comes first. Each block then draws its selections,
    /// its glyphs and its inline highlights, in that order (`PROSE-UI-117`).
    /// The bands and the footnotes follow (`PROSE-UI-115`), and the carets of
    /// the other peers come last (`PROSE-UI-118`).
    pub fn paint<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette) {
        let Some(pages) = self.column.built_pages(&self.viewport) else {
            return;
        };
        let scale = f64::from(self.viewport.scale());
        let camera = Affine::translate((f64::from(self.column.left_px(&self.viewport)), 0.0))
            * Affine::scale(scale)
            * Affine::translate((0.0, -f64::from(self.viewport.scroll_y)));
        surface.push_transform(camera);
        for page in pages {
            let top = f64::from(self.column.page_top_y(page));
            surface.push_transform(Affine::translate((0.0, top)));
            self.paint_page(surface, palette, page);
            surface.pop_transform();
        }
        surface.pop_transform();
    }

    fn paint_page<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette, page: u32) {
        let layout = std::sync::Arc::clone(&self.shaped.layout);
        let width = f64::from(layout.page_width_px);
        let height = f64::from(layout.page_height_px);
        let sheet = Rect::new(0.0, 0.0, width, height);
        let shadow = Rect::new(
            SHADOW_OFFSET_PX,
            SHADOW_OFFSET_PX,
            width + SHADOW_OFFSET_PX,
            height + SHADOW_OFFSET_PX,
        );
        surface.fill_rect(shadow, palette.shadow);
        surface.fill_rect(sheet, palette.sheet);
        surface.stroke_rect(sheet, palette.sheet_border, 1.0);

        let left = f64::from(layout.margin_left_px);
        let body_top = f64::from(layout.body_top_px());
        let content_width = f64::from(layout.content_width_px);
        let (pieces, ghost_at) = self.pieces(page);
        let mut drawn: Vec<(BlockId, ShapedRun, Placed)> = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let Some(block) = self.shaped.block(piece.index) else {
                continue;
            };
            let fragment = piece.fragment;
            if let Some(table) = piece.table {
                let clip_top = body_top + f64::from(fragment.top);
                // A table may be wider than the text: it grows to the right
                // rather than squeezing its columns.
                let right = table
                    .cells
                    .iter()
                    .map(|cell| f64::from(cell.x_px + cell.width_px))
                    .fold(content_width, f64::max);
                let clip = Rect::new(
                    left,
                    clip_top,
                    left + right + 1.0,
                    clip_top + f64::from(fragment.height) + 1.0,
                );
                surface.push_clip(clip);
                self.paint_table(
                    surface,
                    palette,
                    &table,
                    left,
                    clip_top - f64::from(fragment.run_top),
                );
                surface.pop_clip();
                continue;
            }
            let Some(run) = piece.run else {
                continue;
            };
            let id = block_id(block.block_id);
            let placed = place(left, body_top, content_width, &run, fragment);
            surface.push_clip(placed.clip);
            self.paint_block(surface, palette, id, &run, &placed, true);
            surface.pop_clip();
            drawn.push((id, run, placed));
        }
        // The paragraph a split opens, below the block it parts
        // (`PROSE-UI-203`).
        if let (Some(at), Some(ghost)) = (ghost_at, self.ghost().cloned()) {
            let placed = place(left, body_top, content_width, &ghost.run, at);
            surface.push_clip(placed.clip);
            match self.splitting.and_then(|split| split.new_block) {
                Some(new_block) => {
                    self.paint_block(surface, palette, new_block, &ghost.run, &placed, true);
                }
                None => {
                    let data = ghost.run.data();
                    draw_shape(
                        surface,
                        &ghost.run,
                        placed.origin_x,
                        placed.origin_y,
                        palette.text,
                    );
                    if let Some(caret) = ghost.caret {
                        let rect = to_rect(&placed, &data.caret_box(caret));
                        surface.fill_rect(
                            Rect::new(rect.x0, rect.y0, rect.x0 + CARET_WIDTH_PX, rect.y1),
                            palette.accent,
                        );
                    }
                }
            }
            surface.pop_clip();
        }
        self.paint_bands(surface, palette, page);
        self.paint_notes(surface, palette, page);
        // Last, so a collaborator's caret never hides the text under it.
        for (id, run, placed) in &drawn {
            self.paint_carets(surface, palette, *id, run.data(), placed);
        }
    }

    fn paint_block<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        palette: &PagePalette,
        id: BlockId,
        run: &ShapedRun,
        placed: &Placed,
        local: bool,
    ) {
        let data = run.data();
        // Under the glyphs: the local selection, then each peer's.
        if local
            && let Some(ends) = self
                .cursor
                .as_ref()
                .and_then(|cursor| ordered(self, cursor))
            && let Some((start, end)) = covered(self, id, data.len(), ends)
        {
            for rect in data.range_boxes(start, end) {
                surface.fill_rect(to_rect(placed, &rect), palette.accent.multiply_alpha(0.2));
            }
        }
        for peer in self.remote_at.get(&id).map_or(&[][..], Vec::as_slice) {
            if let Some(ends) = ordered(self, &peer.position)
                && let Some((start, end)) = covered(self, id, data.len(), ends)
            {
                let colour = ambre_draw::peer_color(&peer.user_id).multiply_alpha(1.0 / 6.0);
                for rect in data.range_boxes(start, end) {
                    surface.fill_rect(to_rect(placed, &rect), colour);
                }
            }
        }
        // The composition, placed before the run is drawn: the text after it
        // moves right by its width (`PROSE-UI-199`).
        let caret = self
            .cursor
            .as_ref()
            .filter(|cursor| local && cursor.block_id == id && cursor.anchor.is_none())
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
                let caret = data.caret_box_at(laid, self.caret_upstream);
                (
                    data.line_of(laid, self.caret_upstream),
                    caret,
                    cursor.offset,
                )
            });
        let compose = caret.and_then(|(line, caret, _)| {
            let (shaped_text, shape) = self.compose_shape.as_ref()?;
            (*shaped_text == self.composing).then(|| {
                let width = f64::from(shape.data().full_width());
                (line, caret.x0, width, shape)
            })
        });
        let insertions: Vec<Insertion> = compose
            .map(|(line, x, width, _)| Insertion { line, x, width })
            .into_iter()
            .collect();
        draw_run(
            surface,
            &mut self.runs,
            RunKey::Block(crate::events::tree_id(id)),
            run,
            placed,
            palette.text,
            &insertions,
        );
        // Over the glyphs: the inline highlights of this block and no other.
        for mark in self.highlights.get(&id).map_or(&[][..], Vec::as_slice) {
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
        if let Some((_, caret, _)) = caret {
            let mut rect = to_rect(placed, &caret);
            // The composition in progress, at the caret, and the caret after
            // it. It never waits on the document (`PROSE-UI-142`).
            if let Some((_, _, width, shape)) = compose {
                draw_shape(surface, shape, rect.x0, rect.y0, palette.text);
                let base = rect.y1 - 1.0;
                surface.stroke_line(rect.x0, base, rect.x0 + width, base, palette.text, 1.0);
                rect.x0 += width;
            }
            surface.fill_rect(
                Rect::new(rect.x0, rect.y0, rect.x0 + CARET_WIDTH_PX, rect.y1),
                palette.accent,
            );
        }
    }

    /// A table: each cell's fill and border, then the text it holds. A merged
    /// region is one cell and is drawn once; a covered cell is not in the
    /// shape, so nothing is drawn for it (`PROSE-UI-147`). The text sits
    /// inside the padding, at the cell's vertical alignment (`PROSE-UI-180`).
    fn paint_table<S: DrawSurface>(
        &mut self,
        surface: &mut S,
        palette: &PagePalette,
        table: &TableShape,
        left: f64,
        top: f64,
    ) {
        for cell in &table.cells {
            let Some(frame) = cell_frame(table, cell) else {
                continue;
            };
            let bounds = Rect::new(
                left + f64::from(frame.x0),
                top + f64::from(frame.y0),
                left + f64::from(frame.x1),
                top + f64::from(frame.y1),
            );
            paint_cell_box(surface, palette, &cell.style, bounds);
            surface.push_clip(bounds);
            for (block, y, run) in &cell.blocks {
                let placed = Placed {
                    origin_x: left + f64::from(frame.text_x + run.data().indent_px()),
                    origin_y: top + f64::from(frame.text_y + *y),
                    clip: bounds,
                };
                self.paint_block(surface, palette, block_id(*block), run, &placed, true);
            }
            surface.pop_clip();
        }
    }

    /// The header and the footer this page carries, from their own runs.
    /// They draw like body text: the local caret and selection show on the
    /// page the caret was placed on (`PROSE-UI-033`).
    fn paint_bands<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette, page: u32) {
        let left = f64::from(self.shaped.layout.margin_left_px);
        let right = left + f64::from(self.shaped.layout.content_width_px);
        let local = self.band_page == Some(page);
        for slot in self.band_slots(page) {
            let Some(run) = self.drawn_of(slot.block).cloned() else {
                continue;
            };
            let (from, to) = (f64::from(slot.top), f64::from(slot.bottom));
            let placed = Placed {
                origin_x: left + f64::from(run.data().indent_px()),
                origin_y: from,
                clip: Rect::new(left, from, right, to),
            };
            surface.push_clip(placed.clip);
            self.paint_block(surface, palette, block_id(slot.block), &run, &placed, local);
            surface.pop_clip();
        }
    }

    /// The footnote band at the foot of the body: a separator, then each note
    /// this page carries, in the order of their ordinals. Each note draws its
    /// number, then its text as any block draws, caret and selection with it
    /// (`PROSE-UI-187`).
    fn paint_notes<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette, page: u32) {
        let slots = self.note_slots(page);
        let Some(first) = slots.first() else {
            return;
        };
        let left = f64::from(self.shaped.layout.margin_left_px);
        let right = left + f64::from(self.shaped.layout.content_width_px);
        let rule = f64::from(first.top - NOTE_SEPARATOR_PX / 2.0);
        surface.stroke_line(
            left,
            rule,
            left + (right - left) / 3.0,
            rule,
            palette.sheet_border,
            1.0,
        );
        for slot in slots {
            let (top, bottom) = (f64::from(slot.top), f64::from(slot.bottom));
            let clip = Rect::new(left, top, right, bottom);
            surface.push_clip(clip);
            if let Some(number) = self.shaped.notes.get(&slot.footnote_id).cloned() {
                let placed = Placed {
                    origin_x: left,
                    origin_y: top,
                    clip,
                };
                draw_run(
                    surface,
                    &mut self.runs,
                    RunKey::Note(slot.footnote_id),
                    &number,
                    &placed,
                    palette.text,
                    &[],
                );
            }
            if let Some(text) = self.drawn_of(slot.block).cloned() {
                let placed = Placed {
                    origin_x: left + f64::from(text.data().indent_px()),
                    origin_y: top,
                    clip,
                };
                self.paint_block(surface, palette, block_id(slot.block), &text, &placed, true);
            }
            surface.pop_clip();
        }
    }

    /// The carets of the other peers in one block, each with the tab that
    /// stands for its label.
    fn paint_carets<S: DrawSurface>(
        &self,
        surface: &mut S,
        _palette: &PagePalette,
        id: BlockId,
        data: &ParleyRunData,
        placed: &Placed,
    ) {
        for peer in self.remote_at.get(&id).map_or(&[][..], Vec::as_slice) {
            let colour = ambre_draw::peer_color(&peer.user_id);
            let caret = to_rect(placed, &data.caret_box(peer.position.offset));
            surface.fill_rect(
                Rect::new(caret.x0, caret.y0, caret.x0 + CARET_WIDTH_PX, caret.y1),
                colour.multiply_alpha(0.8),
            );
            surface.fill_rect(
                Rect::new(
                    caret.x0,
                    caret.y0 - LABEL_TAB_PX,
                    caret.x0 + LABEL_TAB_PX,
                    caret.y0,
                ),
                colour,
            );
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use ambre_core::UserId;
    use ambre_draw::Color;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    use super::PagePalette;
    use crate::events::{
        BlockId, BlockInit, CursorPosition, ProseCommand, RemoteCursor, block_id, tree_id,
    };
    use crate::logic::ProseState;
    use crate::ui::ProseUiState;
    use crate::view::cache::RunKey;
    use crate::view::tests::{append, laid_out};
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        ui
    }

    fn paint(ui: &mut ProseUiState) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        ui.paint(&mut surface, &palette());
        surface
    }

    fn sheets(surface: &RecordingSurface) -> usize {
        let sheet = palette().sheet;
        surface.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == sheet))
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let pages = ui.column.page_count;
        assert!(pages > 20, "the document spans {pages} pages");
        let surface = paint(&mut ui);
        let built = ui.column.built_pages(&ui.viewport).expect("a page");
        assert_eq!(sheets(&surface), built.clone().count());
        assert!(sheets(&surface) < 5);
        // No block past the built pages holds drawing state.
        for block in &ui.shaped.layout.blocks {
            let run = &ui.shaped.blocks[&block.block_id];
            if block.page_start > *built.end() + 1 {
                assert!(!ui.runs.holds(RunKey::Block(block.block_id), run));
            }
        }
    }

    #[test]
    fn a_run_is_built_on_its_first_paint_and_reused_after() {
        let (_, shaped) = laid_out(6, 30);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        assert_eq!(ui.runs.built(), 0);
        let first_paint = paint(&mut ui);
        let built = ui.runs.built();
        assert!(built >= 6);
        let second_paint = paint(&mut ui);
        assert_eq!(ui.runs.built(), built);
        // A paint reads the layout and changes nothing: two paints agree.
        assert_eq!(first_paint.ops(), second_paint.ops());
    }

    #[test]
    fn the_run_cache_stays_bounded_over_a_long_scroll() {
        let (mut state, shaped) = laid_out(1200, 8);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let bottom = ui.column.max_scroll(&ui.viewport);
        while ui.viewport.scroll_y < bottom {
            super::super::tests::follow(&mut ui, &mut state);
            paint(&mut ui);
            ui.scroll_by(800.0);
        }
        paint(&mut ui);
        assert!(ui.runs.built() > super::super::cache::RUN_CACHE_RUNS as u64);
        assert!(ui.runs.len() <= super::super::cache::RUN_CACHE_RUNS);
    }

    #[test]
    fn selections_go_under_the_glyphs_and_highlights_over_them() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: 12,
            anchor: Some((id, 2)),
        });
        ui.highlights.insert(
            id,
            vec![Highlight {
                kind: HighlightKind::Comment,
                start: 5,
                end: 15,
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let user = UserId::from_bytes([7; 16]);
        ui.remote_at.insert(
            id,
            vec![RemoteCursor {
                user_id: user,
                display_name: "Bob".to_owned(),
                position: CursorPosition {
                    block_id: id,
                    offset: 3,
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let user = UserId::from_bytes([9; 16]);
        ui.remote_at.insert(
            id,
            vec![RemoteCursor {
                user_id: user,
                display_name: "Eve".to_owned(),
                position: CursorPosition {
                    block_id: id,
                    offset: 8,
                    anchor: Some((id, 1)),
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
                start: 0,
                end: 4,
            })
            .collect();
        ui.highlights.insert(id, marks);
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: 2,
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
        let mut state = ProseState::with_peer(1).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let body = append(&mut state, BlockInit::Paragraph, "Body text with a note.");
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        state.handle_command(ProseCommand::SetFooterEnabled { enabled: true });
        let header = state.bands().header.expect("the header block");
        state.handle_command(ProseCommand::InsertText {
            block_id: header,
            pos: 0,
            text: "Header".to_owned(),
        });
        let footer = state.bands().footer.expect("the footer block");
        state.handle_command(ProseCommand::InsertText {
            block_id: footer,
            pos: 0,
            text: "Footer".to_owned(),
        });
        state.handle_command(ProseCommand::InsertFootnote {
            block_id: body,
            pos: 4,
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
        assert_eq!(separator, 1);
    }

    #[test]
    fn a_block_split_across_two_pages_is_drawn_on_both() {
        let (_, shaped) = laid_out(3, 900);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 4000.0);
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
        assert!(clips >= ui.shaped.layout.blocks.len() + pages.1);
    }

    #[test]
    fn highlights_are_walked_on_the_blocks_drawn_and_no_other() {
        let (_, shaped) = laid_out(300, 60);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let mark = Highlight {
            kind: HighlightKind::Comment,
            start: 0,
            end: 3,
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
        let drawn: usize = built.map(|page| ui.blocks_on(page).len()).sum();
        assert_eq!(ui.highlights_walked, drawn as u64);
        assert!(drawn < ids.len());
    }

    /// `PROSE-UI-147`: a merged region is drawn once, as wide as the columns
    /// it covers. A covered cell draws nothing, not even the text it kept
    /// (`PROSE-FEAT-048`).
    #[test]
    fn a_merged_region_is_drawn_as_one_cell_and_a_covered_cell_not_at_all() {
        use crate::logic::tables::tests::{cell_at, document_with_table, type_in};
        let (mut state, table) = document_with_table(1, 2, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let outcome = state.handle_command(ProseCommand::MergeTableCells {
            table_id: block_id(table),
            start: (0, 0),
            end: (0, 1),
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
        let page_width = f64::from(ui.shaped.layout.page_width_px);
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
        assert_eq!(surface.count(glyphs), 3, "the covered text is not drawn");
    }

    /// A styled table: two cells, the first one typed, both laid out and
    /// painted with the shipped faces.
    fn styled_table() -> (ProseState, loro::TreeID, loro::TreeID) {
        use crate::logic::tables::tests::{cell_at, document_with_table, type_in};
        let (mut state, table) = document_with_table(1, 1, 2);
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
            padding_pt: None,
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
        use ambre_bus::app::prose::CellBorder;
        let (mut state, first, _) = styled_table();
        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { bg, border, .. } = command {
                *bg = Some(0x00ff_8800);
                *border = Some(CellBorder {
                    width_pt: 3.0,
                    color: 0x0012_3456,
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
        let page_width = f64::from(ui.shaped.layout.page_width_px);
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
                .map_or(0.0, |t| t.row_tops.last().copied().unwrap_or(0.0))
        };
        let height0 = table(&ui);

        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { padding_pt, .. } = command {
                *padding_pt = Some(12.0);
            }
        });
        let mut ui = view_of(&mut state);
        let (x1, y1) = first_text_at(&paint(&mut ui));
        let pad = f64::from(12.0 * crate::layout::measure::PX_PER_PT);
        assert!(
            (x1 - x0 - pad).abs() < 0.5,
            "the text moves in by the padding"
        );
        assert!((y1 - y0 - pad).abs() < 0.5, "and down by it");
        assert!(
            (f64::from(table(&ui) - height0) - 2.0 * pad).abs() < 0.5,
            "the row grows by both sides"
        );

        // The neighbour pads more, so the first cell has room below its text.
        style(&mut state, second, |command| {
            if let ProseCommand::SetCellStyle { padding_pt, .. } = command {
                *padding_pt = Some(48.0);
            }
        });
        style(&mut state, first, |command| {
            if let ProseCommand::SetCellStyle { valign, .. } = command {
                *valign = Some(ambre_bus::app::prose::VAlign::Bottom);
            }
        });
        let mut ui = view_of(&mut state);
        let (_, y2) = first_text_at(&paint(&mut ui));
        let room = 2.0 * f64::from(36.0 * crate::layout::measure::PX_PER_PT);
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

    fn drawn_glyphs(surface: &RecordingSurface, n: usize) -> bool {
        surface
            .ops()
            .iter()
            .any(|op| matches!(op, DrawOp::Glyphs { glyph_count, .. } if *glyph_count == n))
    }

    /// `PROSE-UI-142`: a composition opened away from a draft still in
    /// flight is drawn at once, in glyphs of its own at the caret.
    #[test]
    fn a_composition_away_from_the_draft_is_drawn_at_once() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.draft.push_str(1, "abc", &id, 14);
        ui.cursor = Some(CursorPosition {
            block_id: id,
            offset: 0,
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        assert!(drawn_glyphs(&paint(&mut ui), 14));
        ui.draft.push_str(1, "abc", &id, 4);
        shape(&mut ui);
        assert_eq!(provisional(&ui, id), "wordabc word word");
        assert!(drawn_glyphs(&paint(&mut ui), 17));
    }

    /// `PROSE-UI-134`: a draft typed into a header is drawn there before the
    /// document answers, as body text is.
    #[test]
    fn a_draft_in_a_header_is_drawn_before_the_document_answers() {
        let mut state = ProseState::with_peer(1).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        append(&mut state, BlockInit::Paragraph, "Body");
        state.handle_command(ProseCommand::SetHeaderEnabled { enabled: true });
        let header = state.bands().header.expect("the header block");
        let mut ui = view_of(&mut state);
        ui.draft.push_str(1, "Head", &header, 0);
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
        let (mut state, table) = document_with_table(1, 1, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let cell = cell_at(&state, table, 0, 0);
        let block = block_id(first_block_of(&state, cell));
        let mut ui = view_of(&mut state);
        let laid = ui.shaped.tables[&table].clone();
        let row = laid.row_tops[1] - laid.row_tops[0];

        ui.draft.push_str(1, "Cell", &block, 0);
        shape(&mut ui);
        assert_eq!(provisional(&ui, block), "Cell");
        assert!(drawn_glyphs(&paint(&mut ui), 4));

        // A column is half the text wide: this wraps over many lines.
        let long = "word ".repeat(60);
        ui.draft.clear();
        ui.draft.push_str(1, &long, &block, 0);
        shape(&mut ui);
        let run = ui.provisional_run(block).expect("the cell is shaped again");
        let text_px = laid.cells[0].width_px - 2.0 * laid.cells[0].style.padding_px;
        assert!(run.data().line_count() > 1, "the draft wraps in its column");
        let widest = (0..=run.data().len())
            .map(|at| run.data().caret_box(at).x0)
            .fold(0.0, f64::max);
        assert!(widest <= f64::from(text_px) + 0.5, "at the column's width");
        let (pieces, _) = ui.pieces(0);
        let grown = pieces
            .iter()
            .find_map(|piece| piece.table.clone())
            .expect("the table is drawn");
        let tall = grown.row_tops[1] - grown.row_tops[0];
        assert!(tall > row + run.data().height() / 2.0, "the row grows");
    }

    /// Where offset `at` of the paragraph drawn for `block` sits, from its
    /// left edge.
    fn x_of(ui: &ProseUiState, block: BlockId, at: u32) -> f64 {
        ui.run_of(block).expect("a run").caret_box(at).x0
    }

    /// `PROSE-UI-199`: the text after the draft is drawn after it, a draft's
    /// width to the right of where it was, and never under it.
    #[test]
    fn the_text_after_a_draft_moves_right_by_its_width() {
        let (_, shaped) = laid_out(1, 3);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let after = x_of(&ui, id, 4);
        ui.draft.push_str(1, "abc", &id, 4);
        shape(&mut ui);
        let width = x_of(&ui, id, 7) - x_of(&ui, id, 4);
        assert!(width > 0.0, "the draft takes room");
        assert!((x_of(&ui, id, 7) - (after + width)).abs() < 0.5);
    }

    /// The width of `text` typed at the start of `block`, as drawn.
    fn draft_width(ui: &mut ProseUiState, block: BlockId, text: &str) -> f64 {
        ui.draft.clear();
        ui.draft.push_str(1, text, &block, 0);
        shape(ui);
        x_of(ui, block, crate::ui::chars(text))
    }

    /// `PROSE-UI-200`: the draft takes the style its block is laid out in,
    /// even when the same text was shaped for another block just before.
    #[test]
    fn a_draft_in_a_heading_is_drawn_as_the_heading() {
        let (mut state, _) = laid_out(1, 3);
        let heading = append(&mut state, BlockInit::Heading { level: 1 }, "Title");
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
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
            range: (0, 4),
            key: "b".to_owned(),
            value: None,
        });
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let bold_at = |ui: &mut ProseUiState, at: u32| {
            ui.draft.clear();
            ui.draft.push_str(1, "abc", &id, at);
            shape(ui);
            let run = ui
                .provisional_run(id)
                .expect("the paragraph is shaped again");
            let (start, end) = (at as usize, at as usize + 3);
            run.data()
                .marks()
                .iter()
                .any(|span| span.bold && span.start <= start && span.end >= end)
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
                range: (0, 4),
                key: key.to_owned(),
                value: None,
            });
        }
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        let text = palette().text;
        let surface = paint(&mut ui);
        let lines: Vec<(f64, f64, f64)> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokeLine {
                    x0, y0, x1, color, ..
                } if *color == text => Some((*x0, *x1, *y0)),
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
            range: (0, 4),
            key: "u".to_owned(),
            value: None,
        });
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let text = palette().text;
        let lines = |ui: &mut ProseUiState| -> Vec<(f64, f64)> {
            paint(ui)
                .ops()
                .iter()
                .filter_map(|op| match op {
                    DrawOp::StrokeLine { x0, x1, color, .. } if *color == text => Some((*x0, *x1)),
                    _ => None,
                })
                .collect()
        };
        let word = lines(&mut ui);
        ui.draft.push_str(1, "abc", &id, 4);
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(state.shaped_layout());
        let id = block;
        let typed = " more".repeat(40);
        ui.draft.push_str(1, &typed, &id, 14);
        shape(&mut ui);
        let drawn = ui
            .provisional_run(id)
            .expect("the paragraph is shaped again");
        state.handle_command(ProseCommand::InsertText {
            block_id: id,
            pos: 14,
            text: typed.clone(),
        });
        let answered = state.shaped_layout();
        let laid = answered.blocks[&tree_id(block)].data();
        assert!(drawn.data().line_count() > 1, "the draft wraps");
        assert_eq!(drawn.data().text(), laid.text());
        assert_eq!(drawn.data().line_tops(), laid.line_tops());
        let starts = |run: &crate::layout::run::ParleyRunData| {
            (0..run.line_count())
                .map(|line| run.at_line(line, 0.0).0)
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        ui.draft.push_delete(1, &id, 0, 5);
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let first = first_block(&ui);
        let top_of_second = |ui: &ProseUiState| {
            let pieces = ui.pieces(0).0;
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
            at: 10,
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
            (moved - ghost.data().height()).abs() < 0.5,
            "the next block moves down by the new paragraph"
        );
        // The document answers with the block it minted: the text typed
        // into it is drawn there, and the caret with it.
        let minted = BlockId::new(9, 9);
        ui.apply_event(&crate::events::ProseEvent::BlockSplit {
            source: first,
            new_block: minted,
        });
        ui.queued.clear();
        ui.draft.push_str(1, "xy", &minted, 0);
        shape(&mut ui);
        assert_eq!(provisional(&ui, minted), "xyword");
        assert_eq!(ui.block_len(minted), 6);
    }

    /// `PROSE-UI-214`: a press in the paragraph a split opens lands in the
    /// block the document minted, before a layout carries it. It still does
    /// when an input replayed since has made the drawn paragraph stale.
    #[test]
    fn a_press_in_the_paragraph_a_split_opens_lands_in_it() {
        let (_, shaped) = laid_out(2, 3);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let first = first_block(&ui);
        ui.splitting = Some(crate::view::provisional::Splitting {
            block: first,
            at: 10,
            new_block: None,
        });
        let minted = BlockId::new(9, 9);
        ui.apply_event(&crate::events::ProseEvent::BlockSplit {
            source: first,
            new_block: minted,
        });
        ui.draft.push_str(1, "xy", &minted, 0);
        shape(&mut ui);
        let at = ui.pieces(0).1.expect("the new paragraph is drawn");
        let run = ui.ghost().expect("shaped").run.clone();
        let caret = run.data().caret_box(3);
        let layout = &ui.shaped.layout;
        let x = layout.margin_left_px + run.data().indent_px() + caret.x0 as f32 + 0.5;
        let y = ui.column.page_top_y(0)
            + layout.body_top_px()
            + at.top
            + (caret.y0 + caret.y1) as f32 / 2.0;
        let scale = ui.viewport.scale();
        let point = (
            ui.column.left_px(&ui.viewport) + x * scale,
            (y - ui.viewport.scroll_y) * scale,
        );
        assert_eq!(ui.place_at(point.0, point.1), Some((minted, 3)));
        // A keystroke replayed before the next frame shapes again.
        ui.draft.push_str(2, "z", &minted, 2);
        assert_eq!(ui.place_at(point.0, point.1), Some((minted, 3)));
    }

    /// The screen point of offset `at` in the text of `block`, a cell of
    /// the table drawn on page 0.
    fn cell_point(ui: &ProseUiState, block: BlockId, at: u32) -> (f32, f32) {
        let (pieces, _) = ui.pieces(0);
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
        let caret = run.data().caret_box(at);
        let layout = &ui.shaped.layout;
        let x =
            layout.margin_left_px + frame.text_x + run.data().indent_px() + caret.x0 as f32 + 0.5;
        let y = ui.column.page_top_y(0)
            + layout.body_top_px()
            + piece.fragment.top
            + frame.text_y
            + top
            + (caret.y0 + caret.y1) as f32 / 2.0;
        let scale = ui.viewport.scale();
        (
            ui.column.left_px(&ui.viewport) + x * scale,
            (y - ui.viewport.scroll_y) * scale,
        )
    }

    /// `PROSE-UI-216`: a press in a cell lands in its text, at the offset
    /// under the pointer. It does so in a laid run and in a draft.
    #[test]
    fn a_press_in_a_cell_lands_in_its_text() {
        use crate::logic::tables::tests::{cell_at, document_with_table, first_block_of, type_in};
        let (mut state, table) = document_with_table(1, 1, 2);
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let (left, right) = (cell_at(&state, table, 0, 0), cell_at(&state, table, 0, 1));
        type_in(&mut state, right, "abcdef");
        let (left, right) = (
            block_id(first_block_of(&state, left)),
            block_id(first_block_of(&state, right)),
        );
        let mut ui = view_of(&mut state);
        let point = cell_point(&ui, right, 3);
        assert_eq!(ui.place_at(point.0, point.1), Some((right, 3)));

        ui.draft.push_str(1, &"word ".repeat(60), &left, 0);
        shape(&mut ui);
        let point = cell_point(&ui, left, 200);
        assert_eq!(ui.place_at(point.0, point.1), Some((left, 200)));
        let point = cell_point(&ui, right, 3);
        assert_eq!(ui.place_at(point.0, point.1), Some((right, 3)));
    }

    fn thread(block: BlockId, start: u32, end: u32) -> crate::events::CommentThread {
        crate::events::CommentThread {
            comment_id: ambre_core::Uuid::new(),
            anchor_block: Some(block),
            anchor_start: start,
            anchor_end: end,
            orphaned: false,
            resolved: false,
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let comment = palette().highlight(HighlightKind::Comment);
        assert!(tints(&paint(&mut ui), comment).is_empty());
        ui.apply_event(&crate::events::ProseEvent::CommentsUpdated {
            threads: vec![thread(id, 4, 9)],
        });
        let surface = paint(&mut ui);
        let data = ui.shaped.blocks[&crate::events::tree_id(id)].data();
        let (start, end) = (data.caret_box(4).x0, data.caret_box(9).x0);
        let painted = tints(&surface, comment);
        assert_eq!(painted.len(), 1);
        let (x0, x1) = painted[0];
        assert!((x1 - x0 - (end - start)).abs() < 0.5, "{painted:?}");
    }

    #[test]
    fn two_overlapping_threads_are_both_painted() {
        let (_, shaped) = laid_out(1, 20);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
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
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        let id = first_block(&ui);
        let mark = |kind| Highlight {
            kind,
            start: 2,
            end: 6,
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
                    DrawOp::StrokeLine { y0, color: c, .. } if *c == color => Some(*y0),
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
}
