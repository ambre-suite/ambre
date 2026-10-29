//! The paginator: a layout kept between edits, and the reflow that updates it
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! A reflow reuses the shaping of every block whose text and style are
//! unchanged (`PROSE-083`) and resumes placement at the block that moved
//! (`PROSE-081`). It stops at the first later block that lands where it
//! already was (`PROSE-100`). What it produces is what a full recomputation
//! would have produced (`PROSE-080`).
//!
//! A table of contents lists the pages its headings start on, and its own
//! height moves them. A reflow measures it on the pages the layout before
//! gave. Then [`Paginator::settle_toc`] is the bounded pass that brings the two
//! into step, off the keystroke path (`PROSE-019`, `PROSE-084`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ambre_bus::app::prose::{BlockType, PageCount, PageIndex, PageNumber, PageSpan};
use ambre_core::{DocPx, FootnoteId};
use loro::{LoroDoc, TreeID};

use crate::doc;
use crate::layout::geometry::PageBox;
use crate::layout::measure::{BlockIndex, Measurer, ShapedCount};
use crate::layout::pass::{self, PageLayout};
use crate::layout::place::{self, BlockLayout, MeasuredBlock, Placement};
use crate::layout::run::LayoutVersion;
use crate::layout::run::{FlowIndex, ShapedLayout, TextEnds, Window, text_ends};
use crate::toc;

/// The most layouts one settling of the tables of contents runs
/// (`PROSE-019`).
///
/// A document whose entries move the pages they list, which move the entries,
/// could otherwise lay itself out without end. The bound makes it cheap rather
/// than fatal: the last layout stands, one page number possibly off.
pub const TOC_SETTLE_PASSES: PassCount = PassCount::new(3);

ambre_core::define_tally! {
    /// How many layouts one settling of the tables of contents ran.
    pub struct PassCount;
}

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
    /// The page each heading started on when the tables of contents were
    /// last measured. Only [`Paginator::settle_toc`] moves it.
    toc_pages: HashMap<TreeID, PageNumber>,
    /// The bands the last layout was placed under (`PROSE-076`).
    bands: pass::BandHeights,
    /// The footnotes the last layout was placed with (`PROSE-117`).
    notes: pass::NotePlan,
    /// Stamped on each layout this paginator hands over, so the view can
    /// tell a late one from a new one (`PROSE-UI-016`).
    version: LayoutVersion,
    /// Every block of the layout, by its place in the flow. Shared by every
    /// window handed over, and built again only when the blocks change.
    order: Arc<HashMap<TreeID, FlowIndex>>,
    /// The first and the last block holding text.
    ends: Option<TextEnds>,
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
            toc_pages: HashMap::new(),
            bands: pass::BandHeights::default(),
            notes: pass::NotePlan::default(),
            version: LayoutVersion::FIRST,
            order: Arc::default(),
            ends: None,
        }
    }

    /// How many runs have been shaped, over the life of this paginator.
    #[must_use]
    pub fn shaped(&self) -> ShapedCount {
        self.measurer.shaped()
    }

    /// Load a face into the measurer this paginator holds.
    ///
    /// The next reflow recomputes in full when the face covers a script an
    /// earlier run was missing (`PROSE-111`).
    pub fn register_faces(&mut self, bytes: Vec<u8>) {
        self.measurer.register_faces(bytes);
    }

    /// The layout as it stands.
    #[must_use]
    pub fn layout(&self) -> &PageLayout {
        &self.layout
    }

    /// The layout as it stands, with the runs the view draws on it, over the
    /// whole document.
    ///
    /// Nothing is shaped here: every run is the one the last pass kept
    /// (`PROSE-UI-006`).
    #[must_use]
    pub fn shaped_layout(&self) -> ShapedLayout {
        self.window(PageIndex::ZERO.through(PageIndex::MAX))
    }

    /// The layout of the pages `pages`, with the runs the view draws on them
    /// (`PROSE-125`). An empty span asks for its start page.
    ///
    /// The page geometry and the page count stay the document's. The blocks,
    /// runs, tables and notes are the window's. A layout handed over costs
    /// what the pages shown hold, not the document. Nothing is shaped here
    /// (`PROSE-UI-006`).
    #[must_use]
    pub fn window(&self, pages: PageSpan) -> ShapedLayout {
        let last = pages
            .last()
            .unwrap_or(pages.start)
            .min(self.layout.page_count.last().unwrap_or(PageIndex::ZERO));
        let first = pages.start.min(last);
        let all = &self.layout.blocks;
        let start = all.partition_point(|block| block.last_page() < first);
        let end = all.partition_point(|block| block.page_start <= last);
        let range = start..end.max(start);
        let blocks = all.get(range.clone()).unwrap_or_default();
        let measured = self.measured.get(range).unwrap_or_default();
        let called: HashSet<FootnoteId> = blocks
            .iter()
            .flat_map(|block| block.notes.iter().map(|note| note.footnote_id))
            .collect();
        let note_blocks: HashSet<TreeID> = blocks
            .iter()
            .flat_map(|block| block.notes.iter().map(|note| note.block_id))
            .collect();
        let runs = measured
            .iter()
            .filter_map(|block| Some((block.block_id, block.run.clone()?)))
            .chain(self.bands.runs.iter().cloned())
            .chain(
                self.notes
                    .texts
                    .iter()
                    .filter(|(block, _)| note_blocks.contains(block))
                    .cloned(),
            )
            .collect();
        let numbers = self
            .notes
            .numbers
            .iter()
            .filter(|(id, _)| called.contains(id))
            .map(|(id, run)| (*id, run.clone()))
            .collect();
        let tables = measured
            .iter()
            .filter_map(|block| Some((block.block_id, Arc::clone(block.table.as_ref()?))))
            .collect();
        ShapedLayout::new(
            self.version,
            Arc::new(self.layout.with_blocks(blocks.to_vec())),
            runs,
            numbers,
            tables,
        )
        .with_window(Window {
            first_index: FlowIndex::from_usize(start),
            pages: first.through(last),
            order: Arc::clone(&self.order),
            ends: self.ends,
        })
    }

    /// The page a block starts on, when the layout holds it.
    #[must_use]
    pub fn page_of(&self, block: &TreeID) -> Option<PageIndex> {
        let index = *self.order.get(block)?;
        Some(self.layout.blocks.get(index.to_usize())?.page_start)
    }

    /// Lay the whole document out again, shaping every block.
    ///
    /// The path a page-box change, a band change, a default change, a snapshot
    /// import and a footnote edit all take (`PROSE-082`, `PROSE-035`).
    pub fn full(&mut self, document: &LoroDoc) -> &PageLayout {
        let page_box = PageBox::read(document);
        self.defaults = doc::read_defaults(document);
        // Every block is about to be shaped again, so what an earlier run was
        // missing says nothing about this one (`PROSE-110`).
        self.measurer.forget_missing();
        // The bands first: they decide the height every page leaves the body
        // (`PROSE-076`).
        self.bands =
            pass::BandHeights::measure(&mut self.measurer, document, page_box.content_width);
        // Every note numbered before the first block is placed (`PROSE-117`).
        self.notes = pass::NotePlan::read(&mut self.measurer, document, page_box.content_width);
        let measured = pass::measure_document_with(
            &mut self.measurer,
            document,
            (page_box.content_width, &self.defaults),
            (&HashMap::new(), &HashMap::new()),
            &self.toc_pages,
            &self.notes,
        );
        let placement = place::place(&measured, self.bands.body_height(&page_box));
        self.commit(page_box, measured, placement, PageIndex::ZERO);
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
        // A band is out of the flow, so no dirty set names it. Measuring the
        // four again is a few short paragraphs, and a band that moved changes
        // every page's body height: that is a full recomputation.
        let bands =
            pass::BandHeights::measure(&mut self.measurer, document, page_box.content_width);
        if page_box != self.page_box
            || bands != self.bands
            || doc::read_defaults(document) != self.defaults
            || self.measured.is_empty()
            || self.measurer.coverage_grew(self.layout.missing_scripts)
            // A call added or removed renumbers every later note, and a note
            // edited changes its reservation: no cached block answers for
            // either (`PROSE-035`).
            || pass::NotePlan::key_of(document) != self.notes.key
        {
            // A face that arrived since the last run re-shapes every block of
            // its script, wherever they sit, so no tail survives
            // (`PROSE-111`).
            return self.full(document);
        }
        // Same heights, and perhaps new text: the view draws the runs just
        // shaped.
        self.bands = bands;
        let measured = self.remeasure(document, page_box, dirty);
        let from = first_difference(&self.measured, &measured);
        let Some(from) = from else {
            // Nothing moved: the layout already on hand is the answer. A
            // run may have new text on the same lines, so the view still
            // gets a new version to draw.
            self.measured = measured;
            self.version = self.version.next();
            self.layout.first_dirty_page = None;
            return &self.layout;
        };
        let placement = self.replace_tail(&measured, page_box, from);
        let first_dirty = first_dirty_page(&self.layout.blocks, &placement.blocks);
        self.commit(page_box, measured, placement, first_dirty);
        &self.layout
    }

    /// Lay the document out again after text alone was typed or erased in
    /// the blocks named in `dirty`.
    ///
    /// No block moved, no setting or mark changed, no footnote call came or
    /// went. So the page box, the defaults, the note plan and the bands hold
    /// as they were. A dirty band, note or call holder is the exception: it
    /// takes [`Paginator::reflow`], which reads them all on every edit.
    pub fn reflow_text(&mut self, document: &LoroDoc, dirty: &[TreeID]) -> &PageLayout {
        let still = !self.measured.is_empty()
            && !self.measurer.coverage_grew(self.layout.missing_scripts)
            && !dirty
                .iter()
                .any(|block| self.notes.touches(block) || self.bands.bands.holds(block));
        if !still {
            return self.reflow(document, dirty);
        }
        let page_box = self.page_box;
        let measured = self.remeasure(document, page_box, dirty);
        let Some(from) = first_difference(&self.measured, &measured) else {
            self.measured = measured;
            self.version = self.version.next();
            self.layout.first_dirty_page = None;
            return &self.layout;
        };
        let placement = self.replace_tail(&measured, page_box, from);
        let first_dirty = first_dirty_page(&self.layout.blocks, &placement.blocks);
        self.commit(page_box, measured, placement, first_dirty);
        &self.layout
    }

    /// Bring every table of contents into step with the pages it lists
    /// (`PROSE-019`).
    ///
    /// Each pass reads the page every heading starts on and, when one moved,
    /// lays the document out again with the entries measured on it. It stops
    /// once no page moved, or after [`TOC_SETTLE_PASSES`] layouts, and answers
    /// how many it ran. No reflow calls it: a keystroke never pays for it
    /// (`PROSE-084`).
    pub fn settle_toc(&mut self, document: &LoroDoc) -> PassCount {
        let Some(tree) = doc::blocks_read(document) else {
            return PassCount::ZERO;
        };
        if self.measured.is_empty() {
            let _ = self.full(document);
        }
        let mut passes = PassCount::ZERO;
        while passes < TOC_SETTLE_PASSES {
            let pages = toc::heading_pages(&tree, &self.layout.blocks);
            if pages == self.toc_pages {
                break;
            }
            self.toc_pages = pages;
            let _ = self.reflow(document, &[]);
            passes = passes.incremented();
        }
        passes
    }

    /// The heading pages the tables of contents were last measured on.
    #[cfg(test)]
    pub(crate) fn toc_pages(&self) -> &HashMap<TreeID, PageNumber> {
        &self.toc_pages
    }

    /// Shape the dirty blocks, and keep what the others already measured.
    ///
    /// A table of contents is always shaped again: it reads every heading, so
    /// any edit may have changed it.
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
            .filter(|block| block.block_type != BlockType::Toc)
            .map(|block| (block.block_id, block))
            .collect();
        let previous: HashMap<TreeID, &MeasuredBlock> = self
            .measured
            .iter()
            .filter(|block| dirty.contains(&block.block_id))
            .map(|block| (block.block_id, block))
            .collect();
        pass::measure_document_with(
            &mut self.measurer,
            document,
            (page_box.content_width, &self.defaults),
            (&cached, &previous),
            &self.toc_pages,
            &self.notes,
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
        from: BlockIndex,
    ) -> Placement {
        let body_height = self.bands.body_height(&page_box);
        let Some(resume) = self.cursor_before(from, body_height) else {
            return place::place(measured, body_height);
        };
        let mut cursor = resume;
        let mut placed: Vec<BlockLayout> = match self.layout.blocks.get(..from.to_usize()) {
            Some(head) => head.to_vec(),
            None => return place::place(measured, body_height),
        };
        // The tail can only be reused when the sequence still has the same
        // shape. An inserted or deleted block shifts every index after it, so
        // an old entry would then answer for another block.
        let same_shape = measured.len() == self.measured.len();
        let stopped =
            place::place_tail_until(measured, from, &mut cursor, &mut placed, |index, live| {
                same_shape && self.cursor_before(index, body_height) == Some(*live)
            });
        let Some(stop) = stopped else {
            return Placement {
                blocks: placed,
                page_count: PageCount::through(cursor.page),
            };
        };
        // From here the old layout is the answer, entry for entry
        // (`PROSE-100`).
        if let Some(tail) = self.layout.blocks.get(stop.to_usize()..) {
            placed.extend_from_slice(tail);
        }
        Placement {
            blocks: placed,
            page_count: self.layout.page_count,
        }
    }

    /// The cursor the block before `from` left, or `None` when `from` is the
    /// first block or the old layout is too short to answer.
    fn cursor_before(&self, from: BlockIndex, body_height: DocPx) -> Option<place::Cursor> {
        let index = from.prev()?.to_usize();
        let entry = self.layout.blocks.get(index)?;
        let block = self.measured.get(index)?;
        Some(place::cursor_after(entry, block, body_height))
    }

    fn commit(
        &mut self,
        page_box: PageBox,
        measured: Vec<MeasuredBlock>,
        placement: Placement,
        first_dirty: PageIndex,
    ) {
        self.page_box = page_box;
        let same_blocks = self.order.len() == measured.len()
            && measured.iter().enumerate().all(|(index, block)| {
                self.order.get(&block.block_id) == Some(&FlowIndex::from_usize(index))
            });
        if !same_blocks {
            self.order = Arc::new(
                measured
                    .iter()
                    .enumerate()
                    .map(|(index, block)| (block.block_id, FlowIndex::from_usize(index)))
                    .collect(),
            );
        }
        self.ends = text_ends(measured.iter().map(|block| {
            (
                block.block_id,
                block.run.as_ref().map(|run| run.data().len()),
            )
        }));
        self.measured = measured;
        self.version = self.version.next();
        self.layout = pass::layout_of(page_box, &placement);
        self.bands.stamp(&mut self.layout);
        self.layout.first_dirty_page = Some(first_dirty);
        self.layout.missing_scripts = self.measurer.missing();
    }
}

/// The index of the first block whose measurement moved, or `None` when none
/// did.
///
/// A block added or removed moves the sequence from where it sits, so a
/// length change answers at the first index that differs.
#[must_use]
fn first_difference(before: &[MeasuredBlock], after: &[MeasuredBlock]) -> Option<BlockIndex> {
    (0..before.len().max(after.len()))
        .find(|&index| before.get(index) != after.get(index))
        .map(BlockIndex::from_usize)
}

/// The first page the new layout differs from the old one on (`PROSE-067`).
///
/// The UI thread repaints from there rather than everywhere.
#[must_use]
fn first_dirty_page(before: &[BlockLayout], after: &[BlockLayout]) -> PageIndex {
    for index in 0..before.len().max(after.len()) {
        match (before.get(index), after.get(index)) {
            (Some(old), Some(new)) if old == new => {}
            (None, None) => {}
            (old, new) => {
                let pages = [
                    old.map(|entry| entry.page_start),
                    new.map(|entry| entry.page_start),
                ];
                return pages.into_iter().flatten().min().unwrap_or(PageIndex::ZERO);
            }
        }
    }
    PageIndex::ZERO
}

/// A paginator holding the faces the application ships.
///
/// Tests only: it stands in for the `LoadFonts` delivery a running peer gets.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn shipped_paginator() -> Paginator {
    let mut paginator = Paginator::new();
    for bytes in crate::layout::fonts::shipped_faces() {
        paginator.register_faces(bytes.into_bytes());
    }
    paginator
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
        let mut fresh = super::shipped_paginator();
        fresh.full(document).clone()
    }

    #[test]
    fn a_reflow_of_an_untouched_document_moves_nothing() {
        let (document, _) = document_of(8);
        let mut paginator = super::shipped_paginator();
        let before = paginator.full(&document).clone();
        let after = paginator.reflow(&document, &[]);
        assert_eq!(after.blocks, before.blocks);
        // Nothing moved, so there is no page to repaint (`PROSE-067`).
        assert_eq!(after.first_dirty_page, None);
    }

    #[test]
    fn a_reflow_produces_what_a_full_recomputation_would_have() {
        let (document, ids) = document_of(40);
        let mut paginator = super::shipped_paginator();
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
        let mut paginator = super::shipped_paginator();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let Some(edited) = ids.get(5) else {
            return;
        };
        grow(&document, edited, 2);
        let _ = paginator.reflow(&document, std::slice::from_ref(edited));
        // `PROSE-083`: one block was named, so one run was shaped.
        assert_eq!(paginator.shaped(), after_full.incremented());
    }

    #[test]
    fn a_document_default_change_forces_a_full_recomputation() {
        let (document, ids) = document_of(10);
        let mut paginator = super::shipped_paginator();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let meta = crate::doc::meta(&document);
        let Some(defaults) = crate::doc::map_at(&meta, crate::doc::MetaKey::Defaults) else {
            return;
        };
        let _ = crate::doc::Fields::of(defaults).insert(crate::doc::DefaultsKey::FontSizePt, 24.0);
        let _ = paginator.reflow(&document, &[]);
        // `PROSE-082`: the inherited size moved, so every block is re-styled
        // and shaped again, even though no block was named dirty.
        assert_eq!(
            paginator.shaped(),
            after_full.plus(ShapedCount::of_len(ids.len()))
        );
    }

    #[test]
    fn a_page_box_change_forces_a_full_recomputation() {
        let (document, ids) = document_of(10);
        let mut paginator = super::shipped_paginator();
        let _ = paginator.full(&document);
        let after_full = paginator.shaped();
        let meta = crate::doc::meta(&document);
        let Some(page) = crate::doc::map_at(&meta, crate::doc::MetaKey::Page) else {
            return;
        };
        let _ = crate::doc::Fields::of(page).insert(crate::doc::PageKey::WidthMm, 100.0);
        let _ = paginator.reflow(&document, &[]);
        // `PROSE-082`: the width moved, so every block is shaped again.
        assert_eq!(
            paginator.shaped(),
            after_full.plus(ShapedCount::of_len(ids.len()))
        );
    }

    #[test]
    fn the_layout_names_the_first_page_it_changed() {
        let (document, ids) = document_of(60);
        let mut paginator = super::shipped_paginator();
        let before = paginator.full(&document).clone();
        assert!(before.page_count.get() > 1);
        let Some(edited) = ids.last() else {
            return;
        };
        grow(&document, edited, 40);
        let after = paginator.reflow(&document, std::slice::from_ref(edited));
        let Some(dirty) = after.first_dirty_page else {
            panic!("a layout that replaced another names a page");
        };
        // `PROSE-067`: the edit is at the end, so the early pages are clean.
        assert!(dirty > PageIndex::ZERO);
    }

    #[test]
    fn a_block_inserted_in_the_middle_reflows_the_whole_tail() {
        let (document, ids) = document_of(12);
        let mut paginator = super::shipped_paginator();
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

    /// A document opening on a table of contents, then `headings` headings,
    /// each followed by a paragraph long enough to fill part of a page.
    fn outline(headings: usize) -> (LoroDoc, Vec<TreeID>) {
        let document = LoroDoc::new();
        let mut ids = Vec::new();
        let Some(toc) = crate::doc::create_block(&document, None, BlockType::Toc) else {
            return (document, ids);
        };
        let mut last = toc;
        for index in 0..headings {
            let Some(heading) = crate::doc::create_block(&document, None, BlockType::Heading)
            else {
                break;
            };
            let tree = crate::doc::blocks(&document);
            let _ = tree.mov_after(heading, last);
            let attrs =
                crate::doc::node_of(&tree, &heading).and_then(|node| crate::doc::attrs_at(&node));
            if let Some(attrs) = attrs {
                let _ = attrs.insert(crate::doc::AttrKey::Level, 1_i64);
            }
            let body = crate::doc::get_block_text(&document, &heading);
            let _ = body.insert(0, &format!("Heading {index}"));
            let Some(paragraph) = crate::doc::create_block(&document, None, BlockType::Paragraph)
            else {
                break;
            };
            let _ = crate::doc::blocks(&document).mov_after(paragraph, heading);
            grow(&document, &paragraph, 12);
            ids.push(heading);
            last = paragraph;
        }
        (document, ids)
    }

    #[test]
    fn settling_stops_once_the_pages_hold_and_never_runs_past_its_bound() {
        let (document, headings) = outline(40);
        let mut paginator = super::shipped_paginator();
        let _ = paginator.full(&document);
        let passes = paginator.settle_toc(&document);
        // `PROSE-019`: at least one pass, since the first layout measured the
        // entries on no page at all, and never more than the bound.
        assert!(
            passes > PassCount::ZERO && passes <= TOC_SETTLE_PASSES,
            "{passes}"
        );
        let tree = crate::doc::blocks(&document);
        let pages = crate::toc::heading_pages(&tree, &paginator.layout().blocks);
        assert_eq!(&pages, paginator.toc_pages());
        assert_eq!(pages.len(), headings.len());
        // Settled, a second call finds nothing to move.
        assert_eq!(paginator.settle_toc(&document), PassCount::ZERO);
    }

    #[test]
    fn a_reflow_does_not_settle_the_table_of_contents() {
        let (document, headings) = outline(20);
        let mut paginator = super::shipped_paginator();
        let _ = paginator.full(&document);
        let _ = paginator.settle_toc(&document);
        let settled = paginator.toc_pages().clone();
        let Some(first) = headings.first() else {
            return;
        };
        // A page break ahead of the first heading moves every heading a page
        // down.
        let Some(page_break) = crate::doc::create_block(&document, None, BlockType::PageBreak)
        else {
            return;
        };
        let _ = crate::doc::blocks(&document).mov_before(page_break, *first);
        let _ = paginator.reflow(&document, &[page_break]);
        // `PROSE-084`: the keystroke path laid the document out and left the
        // entries on the pages they had.
        assert_eq!(paginator.toc_pages(), &settled);
        let tree = crate::doc::blocks(&document);
        let moved = crate::toc::heading_pages(&tree, &paginator.layout().blocks);
        assert_ne!(&moved, paginator.toc_pages());
    }
}

/// What two peers on one document have to agree on (`PROSE-109`, `PROSE-110`,
/// `PROSE-111`).
///
/// The faces are the only input a document does not carry. Ambre ships its
/// own, so the base set is the same bytes everywhere and only script coverage
/// can differ. These hold the three consequences. A context holding extra
/// faces measures no differently. A measurement short of a face says so, and
/// a face that arrives throws the layout away.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod convergence {
    use ambre_bus::app::prose::BlockType;
    use ambre_draw::LazyFont;
    use loro::LoroDoc;
    use proptest::prelude::*;

    use super::{Paginator, ShapedCount};

    /// Where the faces the application ships sit in the workspace.
    const FONT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");

    /// A document of one paragraph per entry.
    fn document_of(paragraphs: &[String]) -> LoroDoc {
        let document = LoroDoc::new();
        for text in paragraphs {
            let Some(id) = crate::doc::create_block(&document, None, BlockType::Paragraph) else {
                break;
            };
            let body = crate::doc::get_block_text(&document, &id);
            let _ = body.insert(0, text);
        }
        document
    }

    /// A paginator whose measurer holds the base faces plus the named files.
    fn paginator_with(files: &[&str]) -> Paginator {
        let mut paginator = super::shipped_paginator();
        for file in files {
            let Ok(bytes) = std::fs::read(format!("{FONT_DIR}{file}")) else {
                continue;
            };
            paginator.register_faces(bytes);
        }
        paginator
    }

    /// A latin paragraph, of a length the generator picks.
    fn any_paragraph() -> impl Strategy<Value = String> {
        (1usize..30).prop_map(|words| {
            let mut text = String::new();
            for word in 0..words {
                text.push_str(&format!("word{word} "));
            }
            text
        })
    }

    proptest! {
        /// Two peers covering the same scripts break one document alike
        /// (`PROSE-109`).
        ///
        /// The second peer holds two faces the first does not, and the text
        /// asks for neither. A resolution that read the context would answer
        /// differently on the two, and the layouts would part.
        #[test]
        fn two_peers_covering_the_same_scripts_paginate_alike(
            paragraphs in proptest::collection::vec(any_paragraph(), 0..20)
        ) {
            let document = document_of(&paragraphs);
            let mut one = paginator_with(&[]);
            let mut other = paginator_with(&[
                "NotoSansArabic-Regular.ttf",
                "NotoSansDevanagari-Regular.ttf",
            ]);
            let here = one.full(&document).clone();
            let there = other.full(&document).clone();
            prop_assert_eq!(here.page_count, there.page_count);
            prop_assert_eq!(here.blocks, there.blocks);
        }
    }

    /// A document the base faces cover names no missing script (`PROSE-110`).
    #[test]
    fn a_latin_document_names_no_missing_script() {
        let document = document_of(&["a plain latin paragraph".to_owned()]);
        let mut paginator = paginator_with(&[]);
        assert!(paginator.full(&document).missing_scripts.is_empty());
    }

    /// A measurement short of a face says which script it was short of
    /// (`PROSE-110`).
    #[test]
    fn a_script_with_no_face_is_named_in_the_layout() {
        let document = document_of(&["\u{4e00}\u{4e8c}\u{4e09}".repeat(40)]);
        let mut paginator = paginator_with(&[]);
        let layout = paginator.full(&document);
        assert!(layout.missing_scripts.contains(LazyFont::Cjk));
    }

    /// A face that arrives after a layout was measured throws it away
    /// (`PROSE-111`).
    ///
    /// Every block of that script re-shapes, wherever it sits, so the reflow
    /// takes the full path however little the edit named.
    #[test]
    fn a_face_that_arrives_forces_a_full_recomputation() {
        let paragraphs: Vec<String> = (0..6)
            .map(|_| "\u{4e00}\u{4e8c}\u{4e09}".repeat(40))
            .collect();
        let document = document_of(&paragraphs);
        let mut paginator = paginator_with(&[]);
        let after_full = {
            let layout = paginator.full(&document);
            assert!(layout.missing_scripts.contains(LazyFont::Cjk));
            paginator.shaped()
        };
        let Ok(bytes) = std::fs::read(format!("{FONT_DIR}NotoSansSC-Regular.otf")) else {
            return;
        };
        paginator.register_faces(bytes);
        let layout = paginator.reflow(&document, &[]).clone();
        assert_eq!(
            paginator.shaped(),
            after_full.plus(ShapedCount::of_len(paragraphs.len()))
        );
        assert!(layout.missing_scripts.is_empty());
    }
}
