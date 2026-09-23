//! From a point on the canvas to a place in the text
//! (`features/ambre-spec-prose-selection.md section 2`).
//!
//! The answer is read off the runs the layout carries, never off a fresh
//! shaping pass (`PROSE-UI-114`). So a block the view culled and never painted
//! answers like any other (`PROSE-UI-172`).

use crate::events::{BlockId, block_id};
use crate::layout::place::BlockLayout;
use crate::ui::ProseUiState;
use crate::view::column::DocPos;

/// The part of a block one page shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fragment {
    /// The top of the fragment, from the top of the body.
    pub top: f32,
    pub height: f32,
    /// The top of the fragment's first line, from the top of the run.
    pub run_top: f32,
}

/// The fragment of `block` on `page`, or `None` when the block shows nothing
/// there.
pub(crate) fn fragment(block: &BlockLayout, page: u32, line_tops: &[f32]) -> Option<Fragment> {
    if page == block.page_start {
        return Some(Fragment {
            top: block.y_in_page,
            height: block.height_px,
            run_top: 0.0,
        });
    }
    let k = page.checked_sub(block.page_start)?.checked_sub(1)? as usize;
    let height = *block.continuation_heights.get(k)?;
    let first_line = *block.split_points.get(k)? as usize;
    Some(Fragment {
        top: 0.0,
        height,
        run_top: line_tops.get(first_line).copied().unwrap_or(0.0),
    })
}

impl ProseUiState {
    /// The indices of the blocks a page shows, in flow order. Read from the
    /// index built when the layout was taken, never by walking every block.
    pub(crate) fn blocks_on(&self, page: u32) -> &[usize] {
        self.page_blocks
            .get(page as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// The block and the offset under a canvas point.
    ///
    /// A point above the first block of a page lands on that block, and one
    /// below the last on the last. `None` for a layout with no page, or a
    /// page that shows no block.
    #[must_use]
    pub fn hit_test(&self, x_px: f32, y_px: f32) -> Option<(BlockId, u32)> {
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = self.column.screen_to_doc(&self.viewport, x_px, y_px)
        else {
            return None;
        };
        let layout = &self.shaped.layout;
        let y_body = y_in_page - layout.body_top_px();
        let mut chosen: Option<(&BlockLayout, Fragment)> = None;
        for index in self.blocks_on(page) {
            let Some(block) = layout.blocks.get(*index) else {
                continue;
            };
            let tops = self
                .shaped
                .blocks
                .get(&block.block_id)
                .map(|run| run.data().line_tops())
                .unwrap_or_default();
            let Some(piece) = fragment(block, page, &tops) else {
                continue;
            };
            let first = chosen.is_none();
            if first || y_body >= piece.top {
                chosen = Some((block, piece));
            }
            if y_body < piece.top + piece.height {
                break;
            }
        }
        let (block, piece) = chosen?;
        let offset = match self.shaped.blocks.get(&block.block_id) {
            Some(run) => {
                let run = run.data();
                let x = x_in_page - layout.margin_left_px - run.indent_px();
                let y = (y_body - piece.top).clamp(0.0, piece.height.max(0.0)) + piece.run_top;
                run.offset_at(x, y)
            }
            None => 0,
        };
        Some((block_id(block.block_id), offset))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::events::block_id;
    use crate::ui::ProseUiState;
    use crate::view::tests::laid_out;

    /// Where a block's code-point offset is drawn on the canvas, the way the
    /// painter places it.
    fn drawn_at(ui: &ProseUiState, index: usize, offset: u32) -> (f32, f32) {
        let layout = &ui.shaped.layout;
        let block = &layout.blocks[index];
        let run = ui.shaped.blocks[&block.block_id].data();
        let caret = run.caret_box(offset);
        let x = layout.margin_left_px + run.indent_px() + caret.x0 as f32 + 0.5;
        let y_doc = ui.column.page_top_y(block.page_start)
            + layout.body_top_px()
            + block.y_in_page
            + (caret.y0 + caret.y1) as f32 / 2.0;
        let scale = ui.viewport.scale();
        (
            ui.column.left_px(&ui.viewport) + x * scale,
            (y_doc - ui.viewport.scroll_y) * scale,
        )
    }

    #[test]
    fn a_block_never_painted_resolves_a_point_off_its_shaped_runs() {
        let (_, shaped) = laid_out(80, 50);
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui.apply_layout(shaped);
        // Scroll to a far block no paint ever reached.
        let far = ui.shaped.layout.blocks.len() - 2;
        let page = ui.shaped.layout.blocks[far].page_start;
        ui.viewport.scroll_y = ui.column.page_top_y(page);
        assert_eq!(ui.runs.built(), 0);
        for offset in [0, 7, 42] {
            let (x, y) = drawn_at(&ui, far, offset);
            let hit = ui.hit_test(x, y);
            let expected = block_id(ui.shaped.layout.blocks[far].block_id);
            assert_eq!(hit, Some((expected, offset)));
        }
        // Resolving built nothing: the drawing state is a paint's alone.
        assert_eq!(ui.runs.built(), 0);
    }

    #[test]
    fn a_layout_with_no_page_resolves_no_position() {
        let ui = ProseUiState::default();
        assert_eq!(ui.hit_test(10.0, 10.0), None);
    }
}
