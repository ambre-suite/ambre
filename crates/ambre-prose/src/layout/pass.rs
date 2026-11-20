//! Pass 1: what each block costs
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! The walk is a depth-first pre-order traversal of the block tree, and that
//! walk is the document flow (`PROSE-077`). Each block is measured against
//! what its type is, and the result is the flat sequence placement consumes.
//! Nothing here writes to the document it reads (`PROSE-079`).

use ambre_core::Ordinal;
use ambre_core::index::{IdxVec, Len, Pos, Span};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::logic::Roots;
use crate::message::{
    BlockType, DocChar, FootnoteCall, KeepTogether, KeepWithNext, Page, ParagraphStyle,
};
use ambre_core::id::{Footnote, Id};
use ambre_core::{Doc, Flags, PrintLength, Pt, Px};
use ambre_draw::LazyScripts;
use loro::{LoroDoc, LoroTree, LoroValue, TreeID};

use crate::doc;
use crate::events::grid::{CellPos, Col, Row};
use crate::layout::geometry::PageBox;
use crate::layout::measure::{BlockLine, EffectiveStyle, Measurer, TextMetrics};
use crate::layout::place::{self, BlockLayout, FootnoteLayout, MeasuredBlock, NoteCall, Placement};
use crate::layout::run::{CellBorderPx, CellShape, CellStyle, ShapedRun, TableShape};
use crate::logic::tables::{Grid, surviving_regions};
use crate::note;

/// The height of a horizontal rule: one pixel of the document, per the
/// measurement table of the pagination spec.
const RULE_HEIGHT: Px<Doc> = Px::<Doc>::new(1.0);

/// The whole document laid out: the page box, and every block on a page.
///
/// The single source of truth for both the screen and PDF export
/// (`PROSE-064`). Derived on every recomputation and stored nowhere
/// (`PROSE-033`, `PROSE-065`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub page_width: Px<Doc>,
    pub page_height: Px<Doc>,
    pub content_width: Px<Doc>,
    pub content_height: Px<Doc>,
    /// Where the content box starts on the sheet.
    pub margin_left: Px<Doc>,
    pub margin_top: Px<Doc>,
    pub page_count: Len<Page>,
    pub blocks: Vec<BlockLayout>,
    pub header_height: Px<Doc>,
    pub header_first_height: Px<Doc>,
    pub footer_height: Px<Doc>,
    pub footer_first_height: Px<Doc>,
    /// The first page this layout differs from the one it replaces on
    /// (`PROSE-067`). `None` when nothing moved, and the first page for a layout
    /// that replaces nothing.
    pub first_dirty_page: Option<Pos<Page>>,
    /// The scripts the document uses that no loaded face covered when this
    /// layout was measured (`PROSE-110`).
    ///
    /// Empty is the settled case. Every run shaped on a face that has the
    /// glyphs, so a peer with the same shipped faces measures the same
    /// heights. A non-empty set means some text shaped into tofu, so a page
    /// number taken from here is not one to store (`PROSE-112`).
    pub missing_scripts: Flags<LazyScripts>,
    /// The bands as the document held them when this layout was taken.
    pub bands: doc::Bands,
}

impl PageLayout {
    /// The header and the footer drawn on the zero-based `page`, each with
    /// its block and its alignment (`PROSE-FEAT-169`, `PROSE-UI-133`,
    /// `PROSE-FEAT-285`). `None` for a hidden side.
    #[must_use]
    pub fn bands_on_page(&self, page: Pos<Page>) -> (Option<doc::Band>, Option<doc::Band>) {
        self.bands.on_page(page)
    }

    /// Every fragment visible on a page, continuations of split blocks
    /// included (`PROSE-066`). Empty for a page that carries nothing, and for
    /// one out of range (`PROSE-011`).
    pub fn blocks_on_page(&self, page: Pos<Page>) -> impl Iterator<Item = &BlockLayout> {
        self.blocks.iter().filter(move |block| block.shows_on(page))
    }

    /// The footnotes drawn at the foot of `page`, in the order of their
    /// ordinals (`PROSE-015`). Empty for a page that carries none.
    #[must_use]
    pub fn footnotes_on_page(&self, page: Pos<Page>) -> Vec<&FootnoteLayout> {
        let mut notes: Vec<&FootnoteLayout> = self
            .blocks
            .iter()
            .flat_map(|block| block.notes.iter())
            .filter(|note| note.page == page)
            .collect();
        notes.sort_by_key(|note| note.ordinal);
        notes
    }

    /// The same geometry, holding `blocks` in place of its own.
    #[must_use]
    pub fn with_blocks(&self, blocks: Vec<BlockLayout>) -> Self {
        Self { blocks, ..*self }
    }

    /// Where the body starts on a sheet, below the taller header. Every page
    /// gets the same body, so every page starts it at the same height.
    #[must_use]
    pub fn body_top(&self) -> Px<Doc> {
        self.margin_top + self.header_height.max(self.header_first_height)
    }

    /// The height left for body content on a page, both bands deducted.
    #[must_use]
    pub fn body_height(&self) -> Px<Doc> {
        let header = self.header_height.max(self.header_first_height);
        let footer = self.footer_height.max(self.footer_first_height);
        (self.content_height - header - footer).max(Px::<Doc>::ZERO)
    }
}

/// Lay the whole document out.
///
/// The one entry point of a full recomputation: read the page box, measure
/// every block, then place them. The bands are measured before the first
/// block is placed (`PROSE-076`).
pub fn lay_out(measurer: &mut Measurer, document: &LoroDoc, roots: &Roots) -> PageLayout {
    let page_box = PageBox::read(document, roots);
    let bands = BandHeights::measure(measurer, document, roots, page_box.content_width);
    let notes = NotePlan::read(measurer, document, roots, page_box.content_width);
    let blocks = measure_document_with(
        measurer,
        (document, roots),
        (page_box.content_width, &doc::read_defaults(document, roots)),
        (&HashMap::new(), &HashMap::new()),
        &HashMap::new(),
        &notes,
        None,
    );
    let placement = place::place(&blocks, bands.body_height(&page_box));
    let mut layout = layout_of(page_box, &placement);
    bands.stamp(&mut layout);
    layout
}

/// The room between a note's number and its text, in pixels.
pub const NOTE_GAP: Px<Doc> = Px::<Doc>::new(4.0);

/// One reachable note: its id, its ordinal, its text and the marked spans
/// of it (`PROSE-UI-207`).
///
/// The text is document content, so `Debug` prints its length alone.
#[derive(Clone, PartialEq)]
pub(crate) struct NoteKey {
    id: Id<Footnote>,
    ordinal: Ordinal<FootnoteCall>,
    text: String,
    marks: Vec<crate::layout::measure::MarkedSpan>,
}

impl std::fmt::Debug for NoteKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoteKey")
            .field("id", &self.id)
            .field("ordinal", &self.ordinal)
            .field("text", &ambre_core::Redacted::new(&self.text))
            .field("marks", &self.marks)
            .finish()
    }
}

/// Every footnote numbered and measured, before any block is placed
/// (`PROSE-117`).
///
/// The ordinals are counted from the calls in document order (`PROSE-068`),
/// so where placement later puts a note never changes its number
/// (`PROSE-099`). A note's text is shaped alone, as the run of its block, with
/// its number beside it. An offset in the run is an offset in the block, so
/// the caret reads a note like any other text (`PROSE-UI-187`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NotePlan {
    /// What the plan was read from: each reachable note, its ordinal, its
    /// text and its marks. A reflow compares it to decide whether the plan
    /// still holds.
    pub(crate) key: Vec<NoteKey>,
    /// The calls of each block: the code-point offset and the note.
    #[expect(
        clippy::type_complexity,
        reason = "the axis argument of the index family lengthens a tuple shape that predates it"
    )]
    calls: HashMap<TreeID, Vec<(Pos<DocChar>, Id<Footnote>)>>,
    /// Each note's ordinal, shaped height and block.
    notes: HashMap<Id<Footnote>, (Ordinal<FootnoteCall>, Px<Doc>, TreeID)>,
    /// Each note's number as it was shaped.
    pub(crate) numbers: HashMap<Id<Footnote>, ShapedRun>,
    /// Each note's text as it was shaped, under the note's block, indented
    /// past its number.
    pub(crate) texts: Vec<(TreeID, ShapedRun)>,
}

impl NotePlan {
    /// What the plan of `document` would be read from, without shaping a
    /// note. The cheap half a reflow runs on every edit (`PROSE-035`).
    pub(crate) fn key_of(document: &LoroDoc, roots: &Roots) -> Vec<NoteKey> {
        // A document with no tree holds no call, and reading one would mint
        // the tree (`PROSE-079`).
        if doc::blocks_read(document, roots).is_none() {
            return Vec::new();
        }
        note::ordinals(document, roots)
            .into_iter()
            .map(|(id, ordinal)| {
                let (text, marks) = note::body(document, roots, &id)
                    .map(|body| crate::layout::measure::marked_text(&body))
                    .unwrap_or_default();
                NoteKey {
                    id,
                    ordinal,
                    text,
                    marks,
                }
            })
            .collect()
    }

    /// Number every note, then shape each one against the content width:
    /// the number first, then the text in the room left beside it.
    pub(crate) fn read(
        measurer: &mut Measurer,
        document: &LoroDoc,
        roots: &Roots,
        content_width: Px<Doc>,
    ) -> Self {
        let key = Self::key_of(document, roots);
        if key.is_empty() {
            return Self::default();
        }
        let defaults = doc::read_defaults(document, roots);
        let style = EffectiveStyle::from_parts(
            &defaults,
            &ParagraphStyle::default(),
            ambre_core::Weight::Regular,
        );
        let width = style.text_width(content_width);
        let mut numbers = HashMap::with_capacity(key.len());
        let mut texts = Vec::with_capacity(key.len());
        let mut notes = HashMap::with_capacity(key.len());
        for NoteKey {
            id,
            ordinal,
            text,
            marks,
        } in &key
        {
            let Some(block) = note::block(document, roots, id) else {
                continue;
            };
            let (number_metrics, number) =
                measurer.measure_run(&ordinal.to_string(), &style, width);
            let mut hung = style.clone();
            hung.indent_left += number.data().full_width() + NOTE_GAP;
            let (metrics, _, run) = measurer.measure_with_lines(
                (text, marks),
                &hung,
                hung.text_width(content_width),
                &[],
                None,
            );
            let height = metrics.text_height.max(number_metrics.text_height);
            numbers.insert(*id, number);
            texts.push((block, run));
            notes.insert(*id, (*ordinal, height, block));
        }
        #[expect(
            clippy::type_complexity,
            reason = "the axis argument of the index family lengthens a tuple shape that predates it"
        )]
        let mut calls: HashMap<TreeID, Vec<(Pos<DocChar>, Id<Footnote>)>> = HashMap::new();
        for anchor in note::footnote_calls(document) {
            let id = anchor.value;
            if notes.contains_key(&id) {
                calls
                    .entry(anchor.block)
                    .or_default()
                    .push((anchor.offset, id));
            }
        }
        Self {
            key,
            calls,
            notes,
            numbers,
            texts,
        }
    }

    /// Whether an edit to `block` may change the plan: the block is a note,
    /// or holds a call.
    pub(crate) fn touches(&self, block: &TreeID) -> bool {
        self.calls.contains_key(block) || self.notes.values().any(|(_, _, id)| id == block)
    }

    /// The calls of `block`, the line of each still to find.
    fn calls_of(&self, block: &TreeID) -> &[(Pos<DocChar>, Id<Footnote>)] {
        self.calls.get(block).map_or(&[], Vec::as_slice)
    }

    /// One call, placed on `line`.
    fn call(&self, id: &Id<Footnote>, line: Pos<BlockLine>) -> Option<NoteCall> {
        let (ordinal, height, block_id) = *self.notes.get(id)?;
        Some(NoteCall {
            footnote_id: *id,
            block_id,
            ordinal,
            line,
            height,
        })
    }
}

/// The four bands measured, and the settings they were measured under.
#[derive(Clone, Debug, Default)]
pub(crate) struct BandHeights {
    pub(crate) bands: doc::Bands,
    header_height: Px<Doc>,
    header_first_height: Px<Doc>,
    footer_height: Px<Doc>,
    footer_first_height: Px<Doc>,
    /// The text of each band shown, as it was shaped, for the view to draw.
    pub(crate) runs: Vec<(TreeID, ShapedRun)>,
}

/// Two measurements are equal when they leave every page the same body. The
/// runs are left out: a band edited to a text of the same height moves no
/// block.
impl PartialEq for BandHeights {
    fn eq(&self, other: &Self) -> bool {
        self.bands == other.bands
            && self.header_height == other.header_height
            && self.header_first_height == other.header_first_height
            && self.footer_height == other.footer_height
            && self.footer_first_height == other.footer_first_height
    }
}

impl BandHeights {
    /// Measure every band a page may draw. A hidden side measures zero.
    pub(crate) fn measure(
        measurer: &mut Measurer,
        document: &LoroDoc,
        roots: &Roots,
        content_width: Px<Doc>,
    ) -> Self {
        let bands = doc::read_bands(document, roots);
        let mut runs = Vec::new();
        let mut height = |band: doc::Band, shown: bool| match band.block {
            Some(id) if shown => {
                let measured = measure_band(measurer, document, roots, &id, content_width);
                if let Some(run) = measured.run {
                    runs.push((id, run));
                }
                measured.height
            }
            _ => Px::<Doc>::ZERO,
        };
        let first = bands.different_first_page.has_own();
        let (header_shown, footer_shown) =
            (bands.header_enabled.is_on(), bands.footer_enabled.is_on());
        let header_height = height(bands.header, header_shown);
        let header_first_height = height(bands.header_first, header_shown && first);
        let footer_height = height(bands.footer, footer_shown);
        let footer_first_height = height(bands.footer_first, footer_shown && first);
        Self {
            bands,
            header_height,
            header_first_height,
            footer_height,
            footer_first_height,
            runs,
        }
    }

    /// The body height every page gets, each band capped at a quarter of the
    /// content height (`PROSE-016`).
    ///
    /// One height for every page: the taller of the regular band and the
    /// first page's own. A page never overflows its bands. The price is
    /// some room lost after the first page when its own bands are the taller.
    pub(crate) fn body_height(&self, page_box: &PageBox) -> Px<Doc> {
        page_box.body_height(
            self.header_height.max(self.header_first_height),
            self.footer_height.max(self.footer_first_height),
        )
    }

    /// Write the heights, capped, and the settings into a layout.
    pub(crate) fn stamp(&self, layout: &mut PageLayout) {
        let cap =
            (layout.content_height * crate::layout::geometry::BAND_SHARE).max(Px::<Doc>::ZERO);
        layout.header_height = self.header_height.min(cap);
        layout.header_first_height = self.header_first_height.min(cap);
        layout.footer_height = self.footer_height.min(cap);
        layout.footer_first_height = self.footer_first_height.min(cap);
        layout.bands = self.bands;
    }
}

/// Assemble the result from the page box and what placement returned.
#[must_use]
pub(crate) fn layout_of(page_box: PageBox, placement: &Placement) -> PageLayout {
    PageLayout {
        page_width: page_box.page_width,
        page_height: page_box.page_height,
        content_width: page_box.content_width,
        content_height: page_box.content_height,
        margin_left: page_box.margin_left,
        margin_top: page_box.margin_top,
        page_count: placement.page_count,
        blocks: placement.blocks.clone(),
        header_height: Px::<Doc>::ZERO,
        header_first_height: Px::<Doc>::ZERO,
        footer_height: Px::<Doc>::ZERO,
        footer_first_height: Px::<Doc>::ZERO,
        first_dirty_page: Some(Pos::<Page>::ZERO),
        // The caller knows what its measurer was missing; this one assembles
        // geometry and placement alone.
        missing_scripts: Flags::<LazyScripts>::EMPTY,
        bands: doc::Bands::default(),
    }
}

/// One band block measured against the content width, before the cap
/// (`PROSE-016`).
#[must_use]
pub(crate) fn measure_band(
    measurer: &mut Measurer,
    document: &LoroDoc,
    roots: &Roots,
    id: &TreeID,
    content_width: Px<Doc>,
) -> MeasuredBlock {
    let Some(tree) = doc::blocks_read(document, roots) else {
        return MeasuredBlock {
            block_id: *id,
            block_type: BlockType::Paragraph,
            height: Px::<Doc>::ZERO,
            space_before: Px::<Doc>::ZERO,
            space_after: Px::<Doc>::ZERO,
            keep_together: KeepTogether::Splittable,
            keep_with_next: KeepWithNext::Free,
            metrics: None,
            notes: Vec::new(),
            run: None,
            text_len: None,
            table: None,
            picture: None,
        };
    };
    let defaults = doc::read_defaults(document, roots);
    let (cached, toc_pages) = (HashMap::new(), HashMap::new());
    let scope = Walk {
        document,
        tree: &tree,
        defaults: &defaults,
        content_width,
        cached: &cached,
        previous: &cached,
        toc_pages: &toc_pages,
        notes: &NotePlan::default(),
        keep: None,
        consumed: std::cell::Cell::new(Px::<Doc>::ZERO),
        place: Place::Flow,
    };
    measure_one(measurer, &scope, id, BlockType::Paragraph)
}

/// Measure every block of the document, in flow order (`PROSE-077`).
#[must_use]
pub fn measure_document(
    measurer: &mut Measurer,
    document: &LoroDoc,
    roots: &Roots,
    content_width: Px<Doc>,
) -> Vec<MeasuredBlock> {
    let notes = NotePlan::read(measurer, document, roots, content_width);
    measure_document_with(
        measurer,
        (document, roots),
        (content_width, &doc::read_defaults(document, roots)),
        (&HashMap::new(), &HashMap::new()),
        &HashMap::new(),
        &notes,
        None,
    )
}

/// Measure every block, reusing a measurement the caller already holds.
///
/// A block found in `cached` is not shaped again (`PROSE-083`). The caller
/// decides what belongs there, which is every block its edit did not touch.
/// `toc_pages` holds the page each heading started on in the layout before,
/// which is what a table of contents is measured with (`PROSE-019`). `notes`
/// is the plan pass 0 read, and every call is measured against it.
/// `previous` holds what an edited block measured before, whose lines above
/// the edit its new run may keep (`PROSE-128`). `defaults` are the
/// document's, which the caller has read or already holds.
#[must_use]
pub(crate) fn measure_document_with(
    measurer: &mut Measurer,
    (document, roots): (&LoroDoc, &Roots),
    (content_width, defaults): (Px<Doc>, &doc::DocDefaults),
    (cached, previous): (
        &HashMap<TreeID, &MeasuredBlock>,
        &HashMap<TreeID, &MeasuredBlock>,
    ),
    toc_pages: &HashMap<TreeID, Ordinal<Page>>,
    notes: &NotePlan,
    keep: Option<&Keep<'_>>,
) -> Vec<MeasuredBlock> {
    let Some(tree) = doc::blocks_read(document, roots) else {
        return Vec::new();
    };
    let mut measured = Vec::new();
    let scope = Walk {
        document,
        tree: &tree,
        defaults,
        content_width,
        cached,
        previous,
        toc_pages,
        notes,
        keep,
        consumed: std::cell::Cell::new(Px::<Doc>::ZERO),
        place: Place::Flow,
    };
    for root in doc::roots(&tree) {
        walk(measurer, &scope, &root, &mut measured);
    }
    measured
}

/// Shape again the runs of the blocks `ids`, which were measured before and
/// whose runs were dropped (`PROSE-137`).
///
/// Each block is measured as the walk measures it, so its run breaks, draws
/// and places a caret as the one it replaces. A table keeps its cells and is
/// left out, as is a block the tree no longer holds. The caller passes what
/// the last layout was measured with, so no height moves.
#[must_use]
pub(crate) fn reshape(
    measurer: &mut Measurer,
    document: &LoroDoc,
    (content_width, defaults): (Px<Doc>, &doc::DocDefaults),
    toc_pages: &HashMap<TreeID, Ordinal<Page>>,
    notes: &NotePlan,
    ids: &[TreeID],
) -> Vec<(TreeID, ShapedRun)> {
    // The blocks were measured, so the tree exists and taking it attaches
    // nothing (`PROSE-079`).
    if ids.is_empty() {
        return Vec::new();
    }
    let tree = doc::blocks(document);
    let none = HashMap::new();
    let scope = Walk {
        document,
        tree: &tree,
        defaults,
        content_width,
        cached: &none,
        previous: &none,
        toc_pages,
        notes,
        keep: None,
        consumed: std::cell::Cell::new(Px::<Doc>::ZERO),
        place: Place::Flow,
    };
    ids.iter()
        .filter_map(|id| {
            let block_type = doc::block_type_of(&tree, id)?;
            if block_type == BlockType::Table {
                return None;
            }
            Some((*id, measure_one(measurer, &scope, id, block_type).run?))
        })
        .collect()
}

/// Which runs a walk keeps as it measures (`PROSE-139`).
///
/// A full recomputation shapes every block to measure it. Holding each run
/// until placement ends would hold the runs of the whole document at once.
/// So the walk drops a run at once when its block cannot reach the pages
/// held. The pages held are decided after placement (`PROSE-136`), and a run
/// dropped here that a page held needs is shaped again then (`PROSE-137`).
pub(crate) struct Keep<'a> {
    /// The pages whose blocks keep their run, the margin included.
    pub pages: Span<Page>,
    /// The content past which a block starts after the last page held.
    ///
    /// A page holds no more content than its body, and a block taller than
    /// a body takes at least one page. So a block preceded by more than two
    /// bodies past that page starts beyond it.
    pub past: Px<Doc>,
    /// The height of the body of a page.
    pub body_height: Px<Doc>,
    /// Every block of the layout before, by its place in the flow.
    pub order: &'a HashMap<TreeID, Pos<crate::layout::run::Flow>>,
    /// The blocks of the layout before. A block it placed far from the pages
    /// held most likely lands far from them again.
    pub before: &'a [BlockLayout],
}

impl Keep<'_> {
    /// Whether the run of `id` is kept, with `consumed` the content measured
    /// before it.
    fn keeps(&self, id: &TreeID, consumed: Px<Doc>) -> bool {
        if consumed > self.past {
            return false;
        }
        let placed = self
            .order
            .get(id)
            .and_then(|index| self.before.get(index.to_usize()));
        match (placed, self.pages.last()) {
            (Some(placed), Some(last)) => {
                placed.last_page() >= self.pages.start && placed.page_start <= last
            }
            _ => true,
        }
    }
}

/// What the walk carries from one block to the next.
struct Walk<'a> {
    document: &'a LoroDoc,
    tree: &'a LoroTree,
    defaults: &'a doc::DocDefaults,
    content_width: Px<Doc>,
    cached: &'a HashMap<TreeID, &'a MeasuredBlock>,
    previous: &'a HashMap<TreeID, &'a MeasuredBlock>,
    toc_pages: &'a HashMap<TreeID, Ordinal<Page>>,
    notes: &'a NotePlan,
    keep: Option<&'a Keep<'a>>,
    /// The content measured so far, each block counted at most one body:
    /// a floor on the pages the blocks before take.
    consumed: std::cell::Cell<Px<Doc>>,
    /// Where the blocks of this walk sit.
    place: Place,
}

/// Where a walk measures its blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// The flow of the document.
    Flow,
    /// A cell of a table. A table there breaks the nesting rules, so it
    /// measures as nothing. A peer may still write one, and measuring it
    /// would recurse once per table nested in it (`PROSE-140`).
    Cell,
}

/// Measure one block and then its children, in that order: the depth-first
/// pre-order of the tree, which is the document flow.
///
/// A table is the one subtree the walk does not descend into. Its rows are
/// the boundaries it breaks at (`PROSE-071`), so it is measured whole and
/// placed as one block.
///
/// A peer writes the tree, and nothing bounds its depth. So the walk keeps
/// the blocks still to measure on a stack of its own, never on the call
/// stack (`PROSE-140`).
fn walk(measurer: &mut Measurer, scope: &Walk<'_>, id: &TreeID, out: &mut Vec<MeasuredBlock>) {
    let mut pending = vec![*id];
    while let Some(id) = pending.pop() {
        // A block measured before keeps its type, so only an edited one is
        // looked up in the tree.
        let block_type = match scope.cached.get(&id) {
            Some(block) => {
                out.push((*block).clone());
                block.block_type
            }
            None => {
                let Some(block_type) = doc::block_type_of(scope.tree, &id) else {
                    continue;
                };
                let mut block = measure_one(measurer, scope, &id, block_type);
                if let Some(keep) = scope.keep {
                    let consumed = scope.consumed.get();
                    if !keep.keeps(&id, consumed) {
                        block.run = None;
                    }
                    scope
                        .consumed
                        .set(consumed + block.height.min(keep.body_height));
                }
                out.push(block);
                block_type
            }
        };
        if block_type == BlockType::Table {
            continue;
        }
        // Reversed, so the first child is the next block measured.
        pending.extend(doc::children_of(scope.tree, Some(id)).into_iter().rev());
    }
}

/// What one block costs, measured against what its type is.
fn measure_one(
    measurer: &mut Measurer,
    scope: &Walk<'_>,
    id: &TreeID,
    block_type: BlockType,
) -> MeasuredBlock {
    let (document, tree, defaults) = (scope.document, scope.tree, scope.defaults);
    let content_width = scope.content_width;
    let weight = if block_type == BlockType::Heading {
        ambre_core::Weight::Bold
    } else {
        ambre_core::Weight::Regular
    };
    let style = EffectiveStyle::resolve(defaults, tree, id, weight);
    let text_width = style.text_width(content_width);
    let mut notes = Vec::new();
    let mut run = None;
    let mut table = None;
    let mut picture = None;
    let (height, metrics) = match block_type {
        // A signal, not a sized block.
        BlockType::PageBreak => (Px::<Doc>::ZERO, None),
        BlockType::HRule => (RULE_HEIGHT, None),
        BlockType::Image => {
            let shape = picture_shape(measurer, tree, id, &style, text_width);
            let height = shape.height
                + shape
                    .caption
                    .as_ref()
                    .map_or(Px::<Doc>::ZERO, |caption| caption.data().height());
            picture = Some(Arc::new(shape));
            (height, None)
        }
        BlockType::Table if scope.place == Place::Cell => (Px::<Doc>::ZERO, None),
        BlockType::Table => {
            let (rows, shape) =
                table_metrics(measurer, document, tree, defaults, id, content_width);
            notes = table_calls(scope, id);
            table = Some(Arc::new(shape));
            (rows.text_height, Some(rows))
        }
        // One line per entry, measured on the pages the layout before gave
        // the headings. A table of contents that lists nothing measures as
        // nothing.
        BlockType::Toc => {
            let text = crate::toc::build(document, tree, id, scope.toc_pages)
                .map(|toc| crate::toc::lines(&toc))
                .unwrap_or_default();
            if text.is_empty() {
                (Px::<Doc>::ZERO, None)
            } else {
                let (metrics, shaped) = measurer.measure_run(&text, &style, text_width);
                run = Some(shaped);
                (metrics.text_height, Some(metrics))
            }
        }
        _ => {
            let (text, marks) =
                crate::layout::measure::marked_text(&doc::get_block_text(document, id));
            let calls = scope.notes.calls_of(id);
            let offsets: Vec<Pos<DocChar>> = calls.iter().map(|(offset, _)| *offset).collect();
            let previous = scope.previous.get(id).and_then(|block| block.run.as_ref());
            let (metrics, lines, shaped) = measurer.measure_with_lines(
                (&text, &marks),
                &style,
                text_width,
                &offsets,
                previous,
            );
            run = Some(shaped);
            notes = calls
                .iter()
                .zip(lines)
                .filter_map(|((_, note), line)| scope.notes.call(note, line))
                .collect();
            (metrics.text_height, Some(metrics))
        }
    };
    MeasuredBlock {
        block_id: *id,
        block_type,
        height,
        space_before: style.space_before,
        space_after: style.space_after,
        keep_together: style.keep_together,
        keep_with_next: style.keep_with_next,
        metrics,
        notes,
        text_len: run.as_ref().map(|run| run.data().len()),
        run,
        table,
        picture,
    }
}

/// The calls the blocks of a table's cells hold, each on its cell's row. A
/// row is the line a table breaks at (`PROSE-071`).
fn table_calls(scope: &Walk<'_>, table: &TreeID) -> Vec<NoteCall> {
    let mut out = Vec::new();
    for (row_index, row) in doc::children_of(scope.tree, Some(*table))
        .iter()
        .enumerate()
    {
        let line = Pos::<BlockLine>::from_usize(row_index);
        for cell in doc::children_of(scope.tree, Some(*row)) {
            for block in doc::children_of(scope.tree, Some(cell)) {
                out.extend(
                    scope
                        .notes
                        .calls_of(&block)
                        .iter()
                        .filter_map(|(_, note)| scope.notes.call(note, line)),
                );
            }
        }
    }
    out
}

/// A picture's box, its alternative text and its caption.
///
/// The stored size is a display size in pixels, never the native one
/// (`ambre-spec-prose-schema.md section 3`). A box wider than the text is
/// scaled down to it, its ratio kept, and the document keeps the stored size
/// (`PROSE-FEAT-317`). A centred picture sits in the middle of the text;
/// every other alignment draws it at the left edge (`PROSE-FEAT-209`). A
/// picture that declares no size is measured on its caption alone.
fn picture_shape(
    measurer: &mut Measurer,
    tree: &LoroTree,
    id: &TreeID,
    style: &EffectiveStyle,
    text_width: Px<Doc>,
) -> crate::layout::run::PictureShape {
    let record = crate::picture::read_image(tree, id);
    let (address, alt, caption, stored, align) = match record {
        Some(record) => (
            record.address,
            record.alt,
            record.caption,
            record.size,
            record.align,
        ),
        None => (
            None,
            ambre_core::Validated::default(),
            ambre_core::Validated::default(),
            ambre_core::Size::new(Px::ZERO, Px::ZERO),
            crate::message::ImageAlign::Block,
        ),
    };
    let laid = crate::picture::laid_size(stored, text_width);
    let x = style.indent_left
        + if align.is_centred() {
            ((text_width - laid.width) / 2.0).max(Px::ZERO)
        } else {
            Px::ZERO
        };
    let shaped = |measurer: &mut Measurer, text: &str, width: Px<Doc>| {
        (!text.is_empty() && width > Px::ZERO).then(|| measurer.measure_run(text, style, width).1)
    };
    crate::layout::run::PictureShape {
        address,
        x,
        width: laid.width,
        height: laid.height,
        alt: shaped(measurer, alt.as_str(), laid.width),
        caption: shaped(measurer, caption.as_str(), text_width),
    }
}

/// A table, measured per row: the tallest cell of each row decides its height.
///
/// The line tops the result carries are the row boundaries, which is what
/// makes a table break between two rows and never inside one (`PROSE-071`).
///
/// A column is as wide as the table's width list says. When the list does not
/// match the grid, each column takes an equal share of the text width.
/// A merged region is one cell as wide as its columns. When its text is
/// taller than the rows it spans, the last of them grows. A covered cell is
/// neither measured nor placed (`PROSE-FEAT-048`). The padding narrows the
/// text and adds to the cell's height on both sides (`PROSE-UI-180`).
fn table_metrics(
    measurer: &mut Measurer,
    document: &LoroDoc,
    tree: &LoroTree,
    defaults: &doc::DocDefaults,
    table: &TreeID,
    content_width: Px<Doc>,
) -> (TextMetrics, TableShape) {
    let Some(grid) = Grid::read(tree, table) else {
        let empty = TextMetrics {
            text_height: Px::<Doc>::ZERO,
            line_count: Len::<BlockLine>::ZERO,
            line_tops: vec![Px::<Doc>::ZERO],
        };
        return (empty, TableShape::default());
    };
    let cols = grid.cells.iter().map(Vec::len).max().unwrap_or(0);
    let widths = column_widths(
        &doc::col_widths(tree, table),
        Len::<Col>::of(cols),
        content_width,
    );
    let mut col_x = Vec::with_capacity(cols + 1);
    col_x.push(Px::<Doc>::ZERO);
    for width in &widths {
        col_x.push(col_x.last().copied().unwrap_or(Px::<Doc>::ZERO) + *width);
    }
    let (regions, _) = surviving_regions(&grid, &doc::regions(tree, table));
    let covered: HashSet<CellPos> = regions
        .iter()
        .flat_map(|(_, rect)| rect.cells().filter(move |at| *at != rect.top_left))
        .collect();

    let mut cells = Vec::new();
    let mut heights = Vec::new();
    for (row, row_cells) in grid.cells.iter().enumerate() {
        for (col, cell) in row_cells.iter().enumerate() {
            let at = CellPos::new(Pos::<Row>::from_usize(row), Pos::<Col>::from_usize(col));
            if covered.contains(&at) {
                continue;
            }
            let last = regions
                .iter()
                .find(|(_, rect)| rect.top_left == at)
                .map_or(at, |(_, rect)| rect.bottom_right);
            let x = col_x.get(col).copied().unwrap_or(Px::<Doc>::ZERO);
            let right_x = col_x.get(last.col.next().to_usize()).copied().unwrap_or(x);
            let width = right_x - x;
            let style = cell_style(tree, cell);
            let text_width = (width - style.padding * 2.0).max(Px::<Doc>::ZERO);
            let stacked = cell_blocks(measurer, document, tree, defaults, cell, text_width);
            let mut top = Px::<Doc>::ZERO;
            let mut blocks = Vec::new();
            for block in stacked {
                top += block.space_before;
                if let Some(run) = block.run {
                    blocks.push((block.block_id, top, run));
                }
                top += block.height + block.space_after;
            }
            heights.push(top + style.padding * 2.0);
            cells.push(CellShape {
                cell: *cell,
                x,
                width,
                rows: Span::<Row>::new(at.row, last.row.next()),
                blocks: blocks.into(),
                content_height: top,
                style,
            });
        }
    }

    // Single-row cells first, then each region takes what it lacks from the
    // last row it spans.
    let mut row_heights: IdxVec<Row, Px<Doc>> = vec![Px::<Doc>::ZERO; grid.rows.len()].into();
    for (cell, height) in cells.iter().zip(&heights) {
        if cell.rows.len() == Len::<Row>::ONE
            && let Some(slot) = row_heights.get_mut(cell.rows.start)
        {
            *slot = slot.max(*height);
        }
    }
    for (cell, height) in cells.iter().zip(&heights) {
        if cell.rows.len() > Len::<Row>::ONE {
            // A span the grid does not hold whole sums to nothing, as the
            // slice read gave before the rows were typed.
            let spanned: Px<Doc> = row_heights
                .as_slice()
                .get(cell.rows.to_usize_range())
                .map_or(Px::<Doc>::ZERO, |rows| rows.iter().copied().sum());
            if let Some(last) = cell.rows.last().and_then(|row| row_heights.get_mut(row)) {
                *last += (*height - spanned).max(Px::<Doc>::ZERO);
            }
        }
    }
    let mut row_tops = IdxVec::<Row, _>::from(vec![Px::<Doc>::ZERO]);
    let mut height = Px::<Doc>::ZERO;
    for row_height in &row_heights {
        height += *row_height;
        row_tops.push(height);
    }
    let metrics = TextMetrics {
        text_height: height,
        line_count: row_heights.len().as_axis::<BlockLine>(),
        line_tops: row_tops.as_slice().to_vec(),
    };
    (
        metrics,
        TableShape {
            row_tops,
            cells: cells.into(),
        },
    )
}

/// Each column's width in pixels. The stored widths are points, one per
/// column. A list that does not match the grid, or holds an unusable width,
/// gives every column an equal share.
pub(crate) fn column_widths(
    stored: &[PrintLength<Pt>],
    cols: Len<Col>,
    content_width: Px<Doc>,
) -> Vec<Px<Doc>> {
    let usable = Len::<Col>::of(stored.len()) == cols
        && stored
            .iter()
            .all(|width| width.is_finite() && *width > PrintLength::<Pt>::ZERO);
    if usable {
        stored.iter().map(|width| width.to_doc_px()).collect()
    } else {
        let share = if cols.is_zero() {
            content_width
        } else {
            content_width / cols.get() as f32
        };
        vec![share; cols.to_usize()]
    }
}

/// The look a cell stores. A value that does not parse, or a width that is
/// negative or not finite, reads as absent.
pub(crate) fn cell_style(tree: &LoroTree, cell: &TreeID) -> CellStyle {
    let Some(attrs) = doc::attrs_read(tree, cell) else {
        return CellStyle::default();
    };
    let length = |key| {
        number_at(&attrs, key)
            .filter(|pt| pt.is_finite() && *pt >= 0.0)
            .map(|pt| PrintLength::<Pt>::new(pt).to_doc_px())
    };
    let colour = |key| string_at(&attrs, key).and_then(|raw| crate::mark::parse_color(&raw).ok());
    let edges = [
        length(doc::AttrKey::EdgeTop),
        length(doc::AttrKey::EdgeBottom),
        length(doc::AttrKey::EdgeLeft),
        length(doc::AttrKey::EdgeRight),
    ];
    let [top, bottom, left, right] = edges;
    let border = edges.iter().any(Option::is_some).then(|| CellBorderPx {
        top: top.unwrap_or(Px::<Doc>::ZERO),
        bottom: bottom.unwrap_or(Px::<Doc>::ZERO),
        left: left.unwrap_or(Px::<Doc>::ZERO),
        right: right.unwrap_or(Px::<Doc>::ZERO),
        color: colour(doc::AttrKey::EdgeColor),
    });
    let valign = number_at(&attrs, doc::AttrKey::VAlign)
        .and_then(|code| u8::try_from(code as i64).ok())
        .and_then(crate::message::VAlign::from_code)
        .unwrap_or(crate::message::VAlign::Top);
    CellStyle {
        fill: colour(doc::AttrKey::CellBg),
        border,
        padding: length(doc::AttrKey::Padding).unwrap_or(Px::<Doc>::ZERO),

        valign,
    }
}

/// One cell: the blocks it holds, measured at its width.
fn cell_blocks(
    measurer: &mut Measurer,
    document: &LoroDoc,
    tree: &LoroTree,
    defaults: &doc::DocDefaults,
    cell: &TreeID,
    width: Px<Doc>,
) -> Vec<MeasuredBlock> {
    let mut stacked = Vec::new();
    let empty = HashMap::new();
    let no_pages = HashMap::new();
    let scope = Walk {
        document,
        tree,
        defaults,
        content_width: width,
        cached: &empty,
        previous: &empty,
        toc_pages: &no_pages,
        notes: &NotePlan::default(),
        keep: None,
        consumed: std::cell::Cell::new(Px::<Doc>::ZERO),
        place: Place::Cell,
    };
    for child in doc::children_of(tree, Some(*cell)) {
        walk(measurer, &scope, &child, &mut stacked);
    }
    stacked
}

/// A number an attribute holds, whichever number shape it was written in.
fn number_at(attrs: &doc::Fields<doc::AttrKey>, key: doc::AttrKey) -> Option<f32> {
    match attrs.get(key).and_then(|value| value.into_value().ok())? {
        LoroValue::Double(number) => Some(number as f32),
        LoroValue::I64(number) => Some(number as f32),
        _ => None,
    }
}

/// A string an attribute holds.
fn string_at(attrs: &doc::Fields<doc::AttrKey>, key: doc::AttrKey) -> Option<String> {
    match attrs.get(key).and_then(|value| value.into_value().ok())? {
        LoroValue::String(text) => Some(text.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_key_prints_its_text_by_length_alone() {
        let key = NoteKey {
            id: Id::<Footnote>::mint(),
            ordinal: Ordinal::<FootnoteCall>::FIRST,
            text: "private note".into(),
            marks: Vec::new(),
        };
        assert!(crate::redact::tests::hides(&key, "private note"));
    }

    /// A document with one block per type the caller names, all at the root.
    fn document_of(types: &[BlockType]) -> (LoroDoc, Vec<TreeID>) {
        let document = LoroDoc::new();
        let ids = types
            .iter()
            .filter_map(|kind| doc::create_block(&document, None, *kind))
            .collect();
        (document, ids)
    }

    fn write_text(document: &LoroDoc, id: &TreeID, text: &str) {
        let body = doc::get_block_text(document, id);
        let _ = body.insert(0, text);
    }

    fn ids_of(blocks: &[MeasuredBlock]) -> Vec<TreeID> {
        blocks.iter().map(|block| block.block_id).collect()
    }

    #[test]
    fn the_walk_visits_the_tree_depth_first() {
        let document = LoroDoc::new();
        let Some(first) = doc::create_block(&document, None, BlockType::Paragraph) else {
            return;
        };
        let nested = doc::create_block(&document, Some(first), BlockType::ListItem);
        let deeper =
            nested.and_then(|id| doc::create_block(&document, Some(id), BlockType::ListItem));
        let Some(second) = doc::create_block(&document, None, BlockType::Paragraph) else {
            return;
        };
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let measured = measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0));
        let mut expected = vec![first];
        expected.extend(nested);
        expected.extend(deeper);
        expected.push(second);
        assert_eq!(ids_of(&measured), expected);
    }

    #[test]
    fn a_page_break_costs_nothing_and_a_rule_costs_one_pixel() {
        let (document, _) = document_of(&[BlockType::PageBreak, BlockType::HRule]);
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let measured = measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0));
        assert_eq!(measured.len(), 2);
        assert_eq!(measured[0].height, Px::<Doc>::new(0.0));
        assert_eq!(measured[1].height, Px::<Doc>::new(RULE_HEIGHT.get()));
    }

    #[test]
    fn an_image_that_declares_no_size_is_measured_on_what_it_carries() {
        let (document, _) = document_of(&[BlockType::Image]);
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let measured = measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0));
        assert_eq!(measured.len(), 1);
        // Nothing here decodes the file, so an image with no display height
        // and no caption takes no room at all.
        assert_eq!(measured[0].height, Px::<Doc>::new(0.0));
        assert!(!measured[0].splittable());
    }

    #[test]
    fn a_table_is_one_block_and_each_of_its_rows_is_a_break() {
        let document = LoroDoc::new();
        let Some(table) = doc::create_block(&document, None, BlockType::Table) else {
            return;
        };
        for _ in 0..3 {
            let Some(row) = doc::create_block(&document, Some(table), BlockType::TableRow) else {
                return;
            };
            let Some(cell) = doc::create_block(&document, Some(row), BlockType::TableCell) else {
                return;
            };
            let Some(paragraph) = doc::create_block(&document, Some(cell), BlockType::Paragraph)
            else {
                return;
            };
            write_text(&document, &paragraph, "a cell of one short line");
        }
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let measured = measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0));
        // The rows and the cells are inside the table's own height, so the
        // walk emits the table and stops there.
        assert_eq!(ids_of(&measured), vec![table]);
        let Some(metrics) = measured[0].metrics.as_ref() else {
            return;
        };
        assert_eq!(metrics.line_count.get(), 3);
        assert_eq!(metrics.line_tops.len(), 4);
        assert!(metrics.text_height > Px::<Doc>::new(0.0));
    }

    #[test]
    fn measuring_the_document_writes_nothing_to_it() {
        let (document, ids) = document_of(&[BlockType::Paragraph, BlockType::Heading]);
        for id in &ids {
            write_text(&document, id, "a line of text long enough to be shaped");
        }
        // Every document past its first operation carries its version in `meta`
        // (`PROSE-028`), and the reader of `meta` takes the root it holds.
        crate::schema::stamp(&document);
        let before = format!("{:?}", document.get_deep_value());
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let _ = lay_out(&mut measurer, &document, &roots);
        // `PROSE-079`: laying out reads, and reading never mints a container.
        assert_eq!(format!("{:?}", document.get_deep_value()), before);
    }

    #[test]
    fn an_empty_document_lays_out_on_one_blank_page() {
        let document = LoroDoc::new();
        let before = format!("{:?}", document.get_deep_value());
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let layout = lay_out(&mut measurer, &document, &roots);
        // A document with no register at all is the case a reader that mints
        // one would be caught by (`PROSE-079`).
        assert_eq!(format!("{:?}", document.get_deep_value()), before);
        assert_eq!(layout.page_count, Len::<crate::events::Page>::new(1));
        assert_eq!(
            layout
                .blocks_on_page(Pos::<crate::events::Page>::new(0))
                .count(),
            0
        );
        assert!(layout.page_width > Px::<Doc>::new(0.0));
        assert!(layout.body_height() > Px::<Doc>::new(0.0));
    }

    #[test]
    fn a_manual_break_puts_what_follows_it_on_the_next_page() {
        let (document, ids) = document_of(&[
            BlockType::Paragraph,
            BlockType::PageBreak,
            BlockType::Paragraph,
        ]);
        for id in [&ids[0], &ids[2]] {
            write_text(&document, id, "a paragraph of one line");
        }
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let layout = lay_out(&mut measurer, &document, &roots);
        assert_eq!(layout.page_count, Len::<crate::events::Page>::new(2));
        let Some(last) = layout.blocks.last() else {
            return;
        };
        assert_eq!(last.page_start, Pos::<crate::events::Page>::new(1));
        assert_eq!(
            layout
                .blocks_on_page(Pos::<crate::events::Page>::new(1))
                .count(),
            1
        );
    }

    #[test]
    fn a_document_taller_than_a_page_takes_more_than_one() {
        let paragraphs = vec![BlockType::Paragraph; 60];
        let (document, ids) = document_of(&paragraphs);
        for id in &ids {
            write_text(&document, id, "a paragraph that takes a line of its own");
        }
        let mut measurer = crate::layout::measure::shipped_measurer();
        let roots = crate::logic::Roots::watch(&document);
        let layout = lay_out(&mut measurer, &document, &roots);
        assert!(layout.page_count.get() > 1);
        // Every page the layout reports is one a reader can ask for
        // (`PROSE-011`).
        for page in layout.page_count.span() {
            let _ = layout.blocks_on_page(page).count();
        }
    }

    /// Run `work` on a thread whose call stack holds 1 MiB, far less than
    /// one frame per level of the trees below needs.
    fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(work)
            .expect("a thread")
            .join()
            .expect("no stack overflow")
    }

    /// Ten thousand blocks, each one the child of the block before, measure
    /// and count on a small stack. The walk recursed once per level and
    /// overflowed the stack of the Logic thread (`PROSE-140`, audit `P36`).
    #[test]
    fn a_chain_of_ten_thousand_nested_blocks_measures_on_a_small_stack() {
        let (measured, paragraphs) = on_small_stack(|| {
            let document = LoroDoc::new();
            let mut parent = None;
            for _ in 0..10_000 {
                parent = doc::create_block(&document, parent, BlockType::Paragraph);
                if let Some(id) = &parent {
                    write_text(&document, id, "a");
                }
            }
            let mut measurer = crate::layout::measure::shipped_measurer();
            let roots = crate::logic::Roots::watch(&document);
            let measured =
                measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0));
            let count = crate::count::compute_word_count(&document, &roots);
            (measured.len(), count.paragraphs)
        });
        assert_eq!(measured, 10_000);
        assert_eq!(paragraphs.get(), 10_000);
    }

    /// A table inside a cell breaks the nesting rules, and a peer may write
    /// one anyway, two thousand deep. The outer table still measures on a
    /// small stack, and each nested table measures as nothing (`PROSE-140`).
    #[test]
    fn tables_nested_in_cells_measure_on_a_small_stack() {
        let measured = on_small_stack(|| {
            let document = LoroDoc::new();
            let mut parent = None;
            for _ in 0..2_000 {
                let table = doc::create_block(&document, parent, BlockType::Table);
                let row =
                    table.and_then(|t| doc::create_block(&document, Some(t), BlockType::TableRow));
                parent =
                    row.and_then(|r| doc::create_block(&document, Some(r), BlockType::TableCell));
            }
            let mut measurer = crate::layout::measure::shipped_measurer();
            let roots = crate::logic::Roots::watch(&document);
            measure_document(&mut measurer, &document, &roots, Px::<Doc>::new(400.0)).len()
        });
        assert_eq!(measured, 1, "the outer table is one block of the flow");
    }
}
