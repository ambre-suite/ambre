//! The paginator: a layout kept between edits, and the reflow that updates it
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! A reflow reuses the shaping of every block whose text and style are
//! unchanged (`PROSE-083`) and resumes placement at the block that moved
//! (`PROSE-081`). It stops at the first later block that lands where it
//! already was (`PROSE-100`). What it produces is what a full recomputation
//! would have produced (`PROSE-080`).

use std::collections::HashMap;

use loro::{LoroDoc, TreeID};

use crate::doc;
use crate::layout::geometry::PageBox;
use crate::layout::measure::Measurer;
use crate::layout::pass::{self, PageLayout};
use crate::layout::place::{self, BlockLayout, MeasuredBlock, Placement};

/// The layout of one document, and what it takes to update it in place.
///
/// One per open document, held by the Logic thread (`PROSE-010`). Nothing it
/// holds is written back to the document (`PROSE-079`).
pub struct Paginator {
    measurer: Measurer,
    page_box: PageBox,
    /// What the last layout inherited from the document. A change to any of
    /// it re-styles every block, so no tail can be reused (`PROSE-082`).
    defaults: doc::DocDefaults,
    measured: Vec<MeasuredBlock>,
    layout: PageLayout,
}

impl Default for Paginator {
    fn default() -> Self {
        Self::new()
    }
}

impl Paginator {
    /// A paginator over the system faces, holding no layout yet.
    #[must_use]
    pub fn new() -> Self {
        Self::with_measurer(Measurer::new())
    }

    /// A paginator over a measurer the caller built.
    #[must_use]
    pub fn with_measurer(measurer: Measurer) -> Self {
        Self {
            measurer,
            page_box: PageBox::default(),
            defaults: doc::DocDefaults::default(),
            measured: Vec::new(),
            layout: PageLayout::default(),
        }
    }

    /// How many runs have been shaped, over the life of this paginator.
    #[must_use]
    pub fn shaped(&self) -> u64 {
        self.measurer.shaped()
    }

    /// The layout as it stands.
    #[must_use]
    pub fn layout(&self) -> &PageLayout {
        &self.layout
    }

    /// Lay the whole document out again, shaping every block.
    ///
    /// The path a page-box change, a band change, a default change, a snapshot
    /// import and a footnote edit all take (`PROSE-082`, `PROSE-035`).
    pub fn full(&mut self, document: &LoroDoc) -> &PageLayout {
        let page_box = PageBox::read(document);
        self.defaults = doc::read_defaults(document);
        let measured =
            pass::measure_document(&mut self.measurer, document, page_box.content_width_px);
        let placement = place::place(&measured, page_box.body_height_px(0.0, 0.0));
        self.commit(page_box, measured, placement, 0);
        &self.layout
    }

    /// Lay the document out again after an edit to the blocks named in
    /// `dirty`.
    ///
    /// A block not named keeps the measurement it already had, so it is never
    /// shaped twice (`PROSE-083`). A page-box change is not an edit this path
    /// takes: it falls back to a full recomputation (`PROSE-082`).
    pub fn reflow(&mut self, document: &LoroDoc, dirty: &[TreeID]) -> &PageLayout {
        let page_box = PageBox::read(document);
        if page_box != self.page_box
            || doc::read_defaults(document) != self.defaults
            || self.measured.is_empty()
        {
            return self.full(document);
        }
        let measured = self.remeasure(document, page_box, dirty);
        let from = first_difference(&self.measured, &measured);
        let Some(from) = from else {
            // Nothing moved: the layout already on hand is the answer.
            self.measured = measured;
            self.layout.first_dirty_page = None;
            return &self.layout;
        };
        let placement = self.replace_tail(&measured, page_box, from);
        let first_dirty = first_dirty_page(&self.layout.blocks, &placement.blocks);
        self.commit(page_box, measured, placement, first_dirty);
        &self.layout
    }

    /// Shape the dirty blocks, and keep what the others already measured.
    fn remeasure(
        &mut self,
        document: &LoroDoc,
        page_box: PageBox,
        dirty: &[TreeID],
    ) -> Vec<MeasuredBlock> {
        let cached: HashMap<TreeID, &MeasuredBlock> = self
            .measured
            .iter()
            .filter(|block| !dirty.contains(&block.block_id))
            .map(|block| (block.block_id, block))
            .collect();
        pass::measure_document_with(
            &mut self.measurer,
            document,
            page_box.content_width_px,
            &cached,
        )
    }

    /// Place the tail from `from`, keeping the head of the layout as it is.
    ///
    /// Placement resumes on the cursor the block before `from` left behind,
    /// and stops as soon as the cursor rejoins what it was (`PROSE-100`).
    fn replace_tail(
        &self,
        measured: &[MeasuredBlock],
        page_box: PageBox,
        from: usize,
    ) -> Placement {
        let body_height_px = page_box.body_height_px(0.0, 0.0);
        let Some(resume) = self.cursor_before(from, body_height_px) else {
            return place::place(measured, body_height_px);
        };
        let mut cursor = resume;
        let mut placed: Vec<BlockLayout> = match self.layout.blocks.get(..from) {
            Some(head) => head.to_vec(),
            None => return place::place(measured, body_height_px),
        };
        // The tail can only be reused when the sequence still has the same
        // shape. An inserted or deleted block shifts every index after it, so
        // an old entry would then answer for another block.
        let same_shape = measured.len() == self.measured.len();
        let stopped =
            place::place_tail_until(measured, from, &mut cursor, &mut placed, |index, live| {
                same_shape && self.cursor_before(index, body_height_px) == Some(*live)
            });
        let Some(stop) = stopped else {
            return Placement {
                blocks: placed,
                page_count: cursor.page.saturating_add(1),
            };
        };
        // From here the old layout is the answer, entry for entry
        // (`PROSE-100`).
        if let Some(tail) = self.layout.blocks.get(stop..) {
            placed.extend_from_slice(tail);
        }
        Placement {
            blocks: placed,
            page_count: self.layout.page_count,
        }
    }

    /// The cursor the block before `from` left, or `None` when `from` is the
    /// first block or the old layout is too short to answer.
    fn cursor_before(&self, from: usize, body_height_px: f32) -> Option<place::Cursor> {
        let index = from.checked_sub(1)?;
        let entry = self.layout.blocks.get(index)?;
        let block = self.measured.get(index)?;
        Some(place::cursor_after(entry, block, body_height_px))
    }

    fn commit(
        &mut self,
        page_box: PageBox,
        measured: Vec<MeasuredBlock>,
        placement: Placement,
        first_dirty: u32,
    ) {
        self.page_box = page_box;
        self.measured = measured;
        self.layout = pass::layout_of(page_box, &placement);
        self.layout.first_dirty_page = Some(first_dirty);
    }
}

/// The index of the first block whose measurement moved, or `None` when none
/// did.
///
/// A block added or removed moves the sequence from where it sits, so a
/// length change answers at the first index that differs.
#[must_use]
fn first_difference(before: &[MeasuredBlock], after: &[MeasuredBlock]) -> Option<usize> {
    (0..before.len().max(after.len())).find(|&index| before.get(index) != after.get(index))
}

/// The first page the new layout differs from the old one on (`PROSE-067`).
///
/// The UI thread repaints from there rather than everywhere.
#[must_use]
fn first_dirty_page(before: &[BlockLayout], after: &[BlockLayout]) -> u32 {
    for index in 0..before.len().max(after.len()) {
        match (before.get(index), after.get(index)) {
            (Some(old), Some(new)) if old == new => {}
            (None, None) => {}
            (old, new) => {
                let pages = [
                    old.map(|entry| entry.page_start),
                    new.map(|entry| entry.page_start),
                ];
                return pages.into_iter().flatten().min().unwrap_or(0);
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use ambre_bus::app::prose::BlockType;

    use super::*;

    /// A document of `count` paragraphs, each one short line.
    fn document_of(count: usize) -> (LoroDoc, Vec<TreeID>) {
        let document = LoroDoc::new();
        let mut ids = Vec::new();
        for index in 0..count {
            let Some(id) = crate::doc::create_block(&document, None, BlockType::Paragraph) else {
                break;
            };
            let body = crate::doc::get_block_text(&document, &id);
            let _ = body.insert(0, &format!("paragraph number {index}"));
            ids.push(id);
        }
        (document, ids)
    }

    fn grow(document: &LoroDoc, id: &TreeID, times: usize) {
        let body = crate::doc::get_block_text(document, id);
        for _ in 0..times {
            let at = body.len_unicode();
            let _ = body.insert(at, " and a good deal more text than it carried before");
        }
    }

    fn full_layout(document: &LoroDoc) -> PageLayout {
        let mut fresh = Paginator::new();
        fresh.full(document).clone()
    }

    #[test]
    fn a_reflow_of_an_untouched_document_moves_nothing() {
        let (document, _) = document_of(8);
        let mut paginator = Paginator::new();
        let before = paginator.full(&document).clone();
        let after = paginator.reflow(&document, &[]);
        assert_eq!(after.blocks, before.blocks);
        // Nothing moved, so there is no page to repaint (`PROSE-067`).
        assert_eq!(after.first_dirty_page, None);
    }

    #[test]
    fn a_reflow_produces_what_a_full_recomputation_would_have() {
        let (document, ids) = document_of(40);
        let mut paginator = Paginator::new();
        let _ = paginator.full(&document);
        let Some(edited) = ids.get(12) else {
            return;
        };
        grow(&document, edited, 30);
        let reflowed = paginator
            .reflow(&document, std::slice::from_ref(edited))
            .clone();
        let full = full_layout(&document);
        // `PROSE-080`: the incremental path and the full one agree, entry for
        // entry and page for page.
        assert_eq!(reflowed.blocks, full.blocks);
        assert_eq!(reflowed.page_count, full.page_count);
    }

    #[test]
    fn a_reflow_does_not_shape_a_block_it_was_not_told_about() {
        let (document, ids) = document_of(20);
        let mut paginator = Paginator::new();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let Some(edited) = ids.get(5) else {
            return;
        };
        grow(&document, edited, 2);
        let _ = paginator.reflow(&document, std::slice::from_ref(edited));
        // `PROSE-083`: one block was named, so one run was shaped.
        assert_eq!(paginator.shaped(), after_full + 1);
    }

    #[test]
    fn a_document_default_change_forces_a_full_recomputation() {
        let (document, ids) = document_of(10);
        let mut paginator = Paginator::new();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let meta = document.get_map(crate::doc::META);
        let Some(defaults) = crate::doc::map_at(&meta, "defaults") else {
            return;
        };
        let _ = defaults.insert("font_size_pt", 24.0);
        let _ = paginator.reflow(&document, &[]);
        // `PROSE-082`: the inherited size moved, so every block is re-styled
        // and shaped again, even though no block was named dirty.
        assert_eq!(paginator.shaped(), after_full + ids.len() as u64);
    }

    #[test]
    fn a_page_box_change_forces_a_full_recomputation() {
        let (document, ids) = document_of(10);
        let mut paginator = Paginator::new();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let meta = document.get_map(crate::doc::META);
        let Some(page) = crate::doc::map_at(&meta, "page") else {
            return;
        };
        let _ = page.insert("width_mm", 100.0);
        let _ = paginator.reflow(&document, &[]);
        // `PROSE-082`: the width moved, so every block is shaped again.
        assert_eq!(paginator.shaped(), after_full + ids.len() as u64);
    }

    #[test]
    fn the_layout_names_the_first_page_it_changed() {
        let (document, ids) = document_of(60);
        let mut paginator = Paginator::new();
        let before = paginator.full(&document).clone();
        assert!(before.page_count > 1);
        let Some(edited) = ids.last() else {
            return;
        };
        grow(&document, edited, 40);
        let after = paginator.reflow(&document, std::slice::from_ref(edited));
        let Some(dirty) = after.first_dirty_page else {
            panic!("a layout that replaced another names a page");
        };
        // `PROSE-067`: the edit is at the end, so the early pages are clean.
        assert!(dirty > 0);
    }

    #[test]
    fn a_block_inserted_in_the_middle_reflows_the_whole_tail() {
        let (document, ids) = document_of(12);
        let mut paginator = Paginator::new();
        let _ = paginator.full(&document);
        let Some(anchor) = ids.get(4) else {
            return;
        };
        let Some(added) = crate::doc::create_block(&document, Some(*anchor), BlockType::Paragraph)
        else {
            return;
        };
        let body = crate::doc::get_block_text(&document, &added);
        let _ = body.insert(0, "a paragraph that was not there before");
        let reflowed = paginator.reflow(&document, &[added]).clone();
        assert_eq!(reflowed.blocks, full_layout(&document).blocks);
    }
}
