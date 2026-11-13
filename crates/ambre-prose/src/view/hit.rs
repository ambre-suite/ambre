//! From a point on the canvas to a place in the text
//! (`features/ambre-spec-prose-selection.md section 2`).
//!
//! The answer is read off the runs the layout carries, never off a fresh
//! shaping pass (`PROSE-UI-114`). So a block the view culled and never painted
//! answers like any other (`PROSE-UI-172`).

use ambre_core::index::Pos;
use ambre_core::{Doc, Point, Px, View};
use ambre_draw::HitTest;
use loro::TreeID;

use crate::events::{BlockId, Page, block_id, tree_id};
use crate::layout::place::BlockLayout;
use crate::layout::run::{CellShape, RunChar, TableShape};
use crate::message::{DocChar, VAlign};
use crate::ui::ProseUiState;
use crate::view::column::DocPos;
use crate::view::provisional::PieceRuns;

/// The part of a block one page shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fragment {
    /// The top of the fragment, from the top of the body.
    pub top: Px<Doc>,
    pub height: Px<Doc>,
    /// The top of the fragment's first line, from the top of the run.
    pub run_top: Px<Doc>,
}

/// Where a cell and its text sit, from the top-left corner of its table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellFrame {
    pub x0: Px<Doc>,
    pub y0: Px<Doc>,
    pub x1: Px<Doc>,
    pub y1: Px<Doc>,
    /// The top-left corner of the text: inside the padding, at the cell's
    /// vertical alignment (`PROSE-UI-180`).
    pub text_x: Px<Doc>,
    pub text_y: Px<Doc>,
}

/// The frame of `cell` in `table`. The view draws and hits it alike.
pub(crate) fn cell_frame(table: &TableShape, cell: &CellShape) -> Option<CellFrame> {
    let y0 = table.row_top(cell.rows.start)?;
    let y1 = table.row_top(cell.rows.end)?;
    let pad = cell.style.padding;
    let room = (y1 - y0 - pad * 2.0 - cell.content_height).max(Px::<Doc>::ZERO);
    let shift = match cell.style.valign {
        VAlign::Top => Px::<Doc>::ZERO,
        VAlign::Middle => room / 2.0,
        VAlign::Bottom => room,
    };
    Some(CellFrame {
        x0: cell.x,
        y0,
        x1: cell.x + cell.width,
        y1,
        text_x: cell.x + pad,
        text_y: y0 + pad + shift,
    })
}

/// The offset a hit reads off the run drawn under the point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitOffset {
    /// Read off a provisional run, which holds the draft: a document offset.
    Drawn(Pos<DocChar>),
    /// Read off the run of the layout in force, which lacks the draft.
    Laid(Pos<RunChar>),
}

impl HitOffset {
    /// The offset read off the run drawn for `block`, in the space that run
    /// counts.
    fn of(ui: &ProseUiState, block: BlockId, offset: Pos<RunChar>) -> Self {
        if ui.provisional_run(block).is_some() {
            Self::Drawn(offset.as_axis::<DocChar>())
        } else {
            Self::Laid(offset)
        }
    }
}

/// The block and the offset under `(x, y)` in `table`, both from its
/// top-left corner, the offset counted in the run held there.
fn cell_hit(table: &TableShape, x: Px<Doc>, y: Px<Doc>) -> Option<(BlockId, Pos<RunChar>)> {
    let (cell, frame) = table.cells.iter().find_map(|cell| {
        let frame = cell_frame(table, cell)?;
        (x >= frame.x0 && x < frame.x1 && y >= frame.y0 && y < frame.y1).then_some((cell, frame))
    })?;
    let y = y - frame.text_y;
    let mut chosen = cell.blocks.first()?;
    for held in &cell.blocks {
        if y >= held.1 {
            chosen = held;
        }
    }
    let (held, top, run) = chosen;
    let id = block_id(*held);
    let data = run.data();
    let x = x - frame.text_x - data.indent();
    let y = (y - *top).clamp(Px::<Doc>::ZERO, data.height().max(Px::<Doc>::ZERO));
    Some((id, data.offset_at(Point::new(x, y))))
}

/// The fragment of `block` on `page`, or `None` when the block shows nothing
/// there.
pub(crate) fn fragment(
    block: &BlockLayout,
    page: Pos<Page>,
    line_tops: &[Px<Doc>],
) -> Option<Fragment> {
    if page == block.page_start {
        return Some(Fragment {
            top: block.y_in_page,
            height: block.height,
            run_top: Px::<Doc>::ZERO,
        });
    }
    let k = page.len_from(block.page_start).to_usize().checked_sub(1)?;
    let height = *block.continuation_heights.get(k)?;
    let first_line = block.split_points.get(k)?.to_usize();
    Some(Fragment {
        top: Px::<Doc>::ZERO,
        height,
        run_top: line_tops
            .get(first_line)
            .copied()
            .unwrap_or(Px::<Doc>::ZERO),
    })
}

/// A band drawn on a page: its block, and its top and bottom from the top of
/// the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BandSlot {
    pub block: TreeID,
    pub top: Px<Doc>,
    pub bottom: Px<Doc>,
}

/// A footnote drawn at the foot of a page. It carries its identity, the block
/// holding its text, and its top and bottom from the top of the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NoteSlot {
    pub footnote_id: ambre_core::Id<ambre_core::id::Footnote>,
    pub block: TreeID,
    pub top: Px<Doc>,
    pub bottom: Px<Doc>,
}

/// The block and the offset under a canvas point (`UI-THREAD-001`).
///
/// A point in a header or a footer lands in its text. Elsewhere, a point
/// above the first block of a page lands on that block, and one below the
/// last on the last. `None` for a layout with no page, or a page that shows
/// no block.
impl HitTest for ProseUiState {
    type Target = (BlockId, Pos<DocChar>);

    fn hit_test(&self, at: Point<Px<View>>) -> Option<(BlockId, Pos<DocChar>)> {
        let (block, offset) = self.hit(at)?;
        Some((block, self.doc_of_hit(block, offset)))
    }
}

impl ProseUiState {
    /// The notes `page` draws at its foot, in the order of their ordinals,
    /// stacked up from the foot of the body. The one geometry the painter
    /// and the hit-test both read.
    pub(crate) fn note_slots(&self, page: Pos<Page>) -> Vec<NoteSlot> {
        let layout = &self.shaped.layout;
        let mut notes: Vec<&crate::layout::place::FootnoteLayout> = self
            .blocks_on(page)
            .iter()
            .filter_map(|index| self.shaped.block(*index))
            .flat_map(|block| block.notes.iter())
            .filter(|note| note.page == page)
            .collect();
        notes.sort_by_key(|note| note.ordinal);
        let total: Px<Doc> = notes.iter().map(|note| note.height).sum();
        let mut y = layout.body_top() + layout.body_height() - total;
        notes
            .into_iter()
            .map(|note| {
                let top = y;
                y += note.height;
                NoteSlot {
                    footnote_id: note.footnote_id,
                    block: note.block_id,
                    top,
                    bottom: y,
                }
            })
            .collect()
    }

    /// The header and the footer `page` draws, where it draws them. The one
    /// geometry the painter and the hit-test both read.
    pub(crate) fn band_slots(&self, page: Pos<Page>) -> Vec<BandSlot> {
        let layout = &self.shaped.layout;
        let first = page == Pos::<Page>::ZERO && layout.bands.different_first_page.has_own();
        let (header_height, footer_height) = if first {
            (layout.header_first_height, layout.footer_first_height)
        } else {
            (layout.header_height, layout.footer_height)
        };
        let top = layout.margin_top;
        let bottom = top + layout.content_height;
        let (header, footer) = layout.bands_on_page(page);
        [
            (header, top, top + header_height),
            (footer, bottom - footer_height, bottom),
        ]
        .into_iter()
        .filter_map(|(band, top, bottom)| {
            Some(BandSlot {
                block: band?.block?,
                top,
                bottom,
            })
        })
        .collect()
    }

    /// Whether `block` is the text of a header, a footer or a note: one
    /// block drawn apart from the flow, which holds its own selection.
    pub(crate) fn is_band(&self, block: BlockId) -> bool {
        let id = tree_id(block);
        self.shaped.layout.bands.register_of(&id).is_some() || self.note_page(block).is_some()
    }

    /// The page a note's text is drawn on, when `block` holds a note's text.
    pub(crate) fn note_page(&self, block: BlockId) -> Option<Pos<Page>> {
        let id = tree_id(block);
        self.shaped
            .layout
            .blocks
            .iter()
            .flat_map(|placed| placed.notes.iter())
            .find(|note| note.block_id == id)
            .map(|note| note.page)
    }

    /// The text of a band or a note under a point of `page`, when the point
    /// is in one.
    fn band_hit(
        &self,
        page: Pos<Page>,
        x_in_page: Px<Doc>,
        y_in_page: Px<Doc>,
    ) -> Option<(BlockId, Pos<RunChar>)> {
        let bands = self
            .band_slots(page)
            .into_iter()
            .map(|slot| (slot.block, slot.top, slot.bottom));
        let notes = self
            .note_slots(page)
            .into_iter()
            .map(|slot| (slot.block, slot.top, slot.bottom));
        let (block, top, bottom) = bands
            .chain(notes)
            .find(|(_, top, bottom)| y_in_page >= *top && y_in_page < *bottom)?;
        let slot = BandSlot { block, top, bottom };
        let run = self.drawn_of(slot.block)?.data();
        let x = x_in_page - self.shaped.layout.margin_left - run.indent();
        let y = (y_in_page - slot.top).clamp(
            Px::<Doc>::ZERO,
            (slot.bottom - slot.top).max(Px::<Doc>::ZERO),
        );
        Some((block_id(slot.block), run.offset_at(Point::new(x, y))))
    }

    /// The indices of the blocks a page shows, in flow order. Read from the
    /// index built when the layout was taken, never by walking every block.
    pub(crate) fn blocks_on(&self, page: Pos<Page>) -> &[Pos<crate::layout::run::Flow>] {
        self.shaped.blocks_on(page)
    }

    /// The document offset a hit on `block` names.
    pub(crate) fn doc_of_hit(&self, block: BlockId, offset: HitOffset) -> Pos<DocChar> {
        match offset {
            HitOffset::Drawn(doc) => doc,
            HitOffset::Laid(laid) => self.to_doc(block, laid),
        }
    }

    /// The picture block under a canvas point whose picture failed for good,
    /// so a press there asks for it again (`PROSE-UI-145`).
    pub(crate) fn failed_picture_at(&self, at: Point<Px<View>>) -> Option<BlockId> {
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = self.column.screen_to_doc(&self.viewport, at)
        else {
            return None;
        };
        let layout = &self.shaped.layout;
        let x = x_in_page - layout.margin_left;
        let y = y_in_page - layout.body_top();
        let (pieces, _) = self.pieces(page);
        pieces.iter().find_map(|piece| {
            let picture = piece.picture.as_ref()?;
            let inside = x >= picture.x
                && x < picture.x + picture.width
                && y >= piece.fragment.top
                && y < piece.fragment.top + picture.height;
            let failed = matches!(
                picture
                    .address
                    .as_ref()
                    .and_then(|address| self.pictures.get(address)),
                Some(crate::message::PictureShown::Failed(_))
            );
            let block = self.shaped.block(piece.index)?;
            (inside && failed).then(|| crate::events::block_id(block.block_id))
        })
    }

    /// The block and the offset under a canvas point, tagged with the run it
    /// was read off. [`HitTest`] gives it as a document offset.
    ///
    /// A point in the paragraph a split opens lands in it once the document
    /// has named it, before a layout carries it (`PROSE-UI-214`).
    pub(crate) fn hit(&self, at: Point<Px<View>>) -> Option<(BlockId, HitOffset)> {
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = self.column.screen_to_doc(&self.viewport, at)
        else {
            return None;
        };
        // A header, a footer or a note answers like body text (`PROSE-UI-033`,
        // `PROSE-UI-187`).
        if let Some((block, offset)) = self.band_hit(page, x_in_page, y_in_page) {
            return Some((block, HitOffset::of(self, block, offset)));
        }
        let layout = &self.shaped.layout;
        let y_body = y_in_page - layout.body_top();
        let (pieces, ghost_at) = self.pieces(page);
        if let (Some(at), Some(new_block), Some(ghost)) = (
            ghost_at,
            self.splitting.and_then(|split| split.new_block),
            self.provisional.ghost(),
        ) && y_body >= at.top
            && y_body < at.top + at.height
        {
            let run = ghost.run.data();
            let x = x_in_page - layout.margin_left - run.indent();
            return Some((
                new_block,
                HitOffset::Drawn(
                    run.offset_at(Point::new(x, y_body - at.top))
                        .as_axis::<DocChar>(),
                ),
            ));
        }
        let mut chosen: Option<&crate::view::provisional::Piece> = None;
        for piece in &pieces {
            let first = chosen.is_none();
            if first || y_body >= piece.fragment.top {
                chosen = Some(piece);
            }
            if y_body < piece.fragment.top + piece.fragment.height {
                break;
            }
        }
        let chosen = chosen?;
        let block = self.shaped.block(chosen.index)?;
        let piece = chosen.fragment;
        // A press in a table lands in the cell under it, in the text drawn
        // there (`PROSE-UI-216`).
        if let Some(table) = &chosen.table {
            let x = x_in_page - layout.margin_left;
            let y = (y_body - piece.top).clamp(Px::<Doc>::ZERO, piece.height.max(Px::<Doc>::ZERO))
                + piece.run_top;
            if let Some((id, offset)) = cell_hit(table, x, y) {
                // A table drawn with its provisional runs counts the document
                // in them, even once an input replayed since made them stale.
                let drawn = chosen.provisional == PieceRuns::Provisional
                    && self.drawn_provisional_run(id).is_some();
                let offset = if drawn {
                    HitOffset::Drawn(offset.as_axis::<DocChar>())
                } else {
                    HitOffset::Laid(offset)
                };
                return Some((id, offset));
            }
        }
        let offset = match &chosen.run {
            Some(run) => {
                let run = run.data();
                let x = x_in_page - layout.margin_left - run.indent();
                let y = (y_body - piece.top)
                    .clamp(Px::<Doc>::ZERO, piece.height.max(Px::<Doc>::ZERO))
                    + piece.run_top;
                run.offset_at(Point::new(x, y))
            }
            None => Pos::<RunChar>::ZERO,
        };
        let offset = if chosen.provisional == PieceRuns::Provisional {
            HitOffset::Drawn(offset.as_axis::<DocChar>())
        } else {
            HitOffset::Laid(offset)
        };
        Some((block_id(block.block_id), offset))
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::layout::run::RunChar;
    use crate::message::DocChar;
    use ambre_core::index::Pos;
    use ambre_core::{Doc, Point, Px, View};
    use ambre_draw::HitTest;

    use crate::events::block_id;
    use crate::ui::ProseUiState;
    use crate::view::tests::laid_out;

    /// Where a block's code-point offset is drawn on the canvas, the way the
    /// painter places it.
    fn drawn_at(ui: &ProseUiState, index: usize, offset: u32) -> (f32, f32) {
        let layout = &ui.shaped.layout;
        let block = &layout.blocks[index];
        let run = ui.shaped.blocks[&block.block_id].data();
        let caret = run.caret_box(Pos::<RunChar>::new(offset));
        let x = layout.margin_left
            + run.indent()
            + Px::<Doc>::new(caret.x0 as f32)
            + Px::<Doc>::new(0.5);
        let y_doc = ui.column.page_top_y(block.page_start)
            + layout.body_top()
            + block.y_in_page
            + Px::<Doc>::new((caret.y0 + caret.y1) as f32 / 2.0);
        let scale = ui.viewport.scale();
        (
            (ui.column.left(&ui.viewport) + Px::<View>::new(x.get() * scale)).get(),
            (y_doc - ui.viewport.scroll_y).get() * scale,
        )
    }

    #[test]
    fn a_block_never_painted_resolves_a_point_off_its_shaped_runs() {
        let (_, shaped) = laid_out(80, 50);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        ui.apply_layout(shaped);
        // Scroll to a far block no paint ever reached.
        let far = ui.shaped.layout.blocks.len() - 2;
        let page = ui.shaped.layout.blocks[far].page_start;
        ui.viewport.scroll_y = ui.column.page_top_y(page);
        assert_eq!(
            ui.runs.built(),
            ambre_core::Count::<crate::view::cache::Builds>::ZERO
        );
        for offset in [0, 7, 42] {
            let (x, y) = drawn_at(&ui, far, offset);
            let hit = ui.hit_test(Point::new(Px::<View>::new(x), Px::<View>::new(y)));
            let expected = block_id(ui.shaped.layout.blocks[far].block_id);
            assert_eq!(hit, Some((expected, Pos::<DocChar>::new(offset))));
        }
        // Resolving built nothing: the drawing state is a paint's alone.
        assert_eq!(
            ui.runs.built(),
            ambre_core::Count::<crate::view::cache::Builds>::ZERO
        );
    }

    /// `PROSE-UI-236`: a block whose run the Logic thread dropped, then
    /// shaped again, resolves a point as it did before the drop.
    #[test]
    fn a_block_whose_run_was_dropped_resolves_a_point_as_before() {
        let (mut state, _) = laid_out(400, 60);
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(1200.0), Px::<View>::new(900.0));
        let show = |state: &mut crate::logic::ProseState, first: usize, last: usize| {
            let _ = state.handle_command(crate::events::ProseCommand::ShowPages {
                first: Pos::<crate::events::Page>::from_usize(first),
                last: Pos::<crate::events::Page>::from_usize(last),
            });
            state.layout_event().expect("a new window is handed over")
        };
        ui.apply_event(&show(&mut state, 0, 7));
        let pages = ui.shaped.layout.page_count.to_usize();
        let index = 1;
        let expected = block_id(ui.shaped.layout.blocks[index].block_id);
        let points: Vec<(f32, f32)> = [0, 7, 42]
            .into_iter()
            .map(|offset| drawn_at(&ui, index, offset))
            .collect();
        let held = state.runs_held();
        // The view goes to the end, far past the margin of the first page, and
        // the runs of the start are dropped.
        assert!(pages > 30);
        let _ = show(&mut state, pages - 8, pages - 1);
        assert!(state.runs_held() < held);
        ui.apply_event(&show(&mut state, 0, 7));
        for (point, offset) in points.into_iter().zip([0, 7, 42]) {
            let hit = ui.hit_test(Point::new(
                Px::<View>::new(point.0),
                Px::<View>::new(point.1),
            ));
            assert_eq!(hit, Some((expected, Pos::<DocChar>::new(offset))));
        }
    }

    #[test]
    fn a_layout_with_no_page_resolves_no_position() {
        let ui = ProseUiState::default();
        assert_eq!(
            ui.hit_test(Point::new(Px::<View>::new(10.0), Px::<View>::new(10.0))),
            None
        );
    }
}
