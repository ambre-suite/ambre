//! Pass 1: what each block costs
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! The walk is a depth-first pre-order traversal of the block tree, and that
//! walk is the document flow (`PROSE-077`). Each block is measured against
//! what its type is, and the result is the flat sequence placement consumes.
//! Nothing here writes to the document it reads (`PROSE-079`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ambre_bus::app::prose::{BlockType, DocOffset, ParagraphStyle};
use ambre_core::FootnoteId;
use ambre_draw::ScriptSet;
use loro::{LoroDoc, LoroMap, LoroTree, LoroValue, TreeID};

use crate::doc;
use crate::layout::geometry::PageBox;
use crate::layout::measure::{EffectiveStyle, Measurer, PX_PER_PT, TextMetrics};
use crate::layout::place::{self, BlockLayout, FootnoteLayout, MeasuredBlock, NoteCall, Placement};
use crate::layout::run::{CellBorderPx, CellShape, CellStyle, ShapedRun, TableShape};
use crate::logic::tables::{Grid, surviving_regions};
use crate::note;

/// The height of a horizontal rule, in pixels. One device pixel, per the
/// measurement table of section 2.
const RULE_HEIGHT_PX: f32 = 1.0;

/// The two image attributes a height is read from.
///
/// The schema declares them (`ambre-spec-prose-schema.md section 3`), and no
/// command writes them yet. So they carry no constant in `doc.rs`: a key
/// declared there must appear in the frozen shape (`SCHEMA-018`). They move
/// there with the images Story (`ambre-roadmap.md` E5).
const A_IMAGE_HEIGHT: &str = "h";
const A_IMAGE_CAPTION: &str = "caption";

/// The whole document laid out: the page box, and every block on a page.
///
/// The single source of truth for both the screen and PDF export
/// (`PROSE-064`). Derived on every recomputation and stored nowhere
/// (`PROSE-033`, `PROSE-065`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub page_width_px: f32,
    pub page_height_px: f32,
    pub content_width_px: f32,
    pub content_height_px: f32,
    /// Where the content box starts on the sheet.
    pub margin_left_px: f32,
    pub margin_top_px: f32,
    pub page_count: u32,
    pub blocks: Vec<BlockLayout>,
    pub header_height_px: f32,
    pub header_first_height_px: f32,
    pub footer_height_px: f32,
    pub footer_first_height_px: f32,
    /// The first page this layout differs from the one it replaces on
    /// (`PROSE-067`). `None` when nothing moved, and `Some(0)` for a layout
    /// that replaces nothing.
    pub first_dirty_page: Option<u32>,
    /// The scripts the document uses that no loaded face covered when this
    /// layout was measured (`PROSE-110`).
    ///
    /// Empty is the settled case. Every run shaped on a face that has the
    /// glyphs, so a peer with the same shipped faces measures the same
    /// heights. A non-empty set means some text shaped into tofu, so a page
    /// number taken from here is not one to store (`PROSE-112`).
    pub missing_scripts: ScriptSet,
    /// The bands as the document held them when this layout was taken.
    pub bands: doc::Bands,
}

impl PageLayout {
    /// The header and the footer drawn on the zero-based `page`, each with
    /// its block and its alignment (`PROSE-FEAT-169`, `PROSE-UI-133`,
    /// `PROSE-FEAT-285`). `None` for a hidden side.
    #[must_use]
    pub fn bands_on_page(&self, page: u32) -> (Option<doc::Band>, Option<doc::Band>) {
        self.bands.on_page(page)
    }

    /// Every fragment visible on a page, continuations of split blocks
    /// included (`PROSE-066`). Empty for a page that carries nothing, and for
    /// one out of range (`PROSE-011`).
    pub fn blocks_on_page(&self, page: u32) -> impl Iterator<Item = &BlockLayout> {
        self.blocks.iter().filter(move |block| block.shows_on(page))
    }

    /// The footnotes drawn at the foot of `page`, in the order of their
    /// ordinals (`PROSE-015`). Empty for a page that carries none.
    #[must_use]
    pub fn footnotes_on_page(&self, page: u32) -> Vec<&FootnoteLayout> {
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
    pub fn body_top_px(&self) -> f32 {
        self.margin_top_px + self.header_height_px.max(self.header_first_height_px)
    }

    /// The height left for body content on a page, both bands deducted.
    #[must_use]
    pub fn body_height_px(&self) -> f32 {
        let header = self.header_height_px.max(self.header_first_height_px);
        let footer = self.footer_height_px.max(self.footer_first_height_px);
        (self.content_height_px - header - footer).max(0.0)
    }
}

/// Lay the whole document out.
///
/// The one entry point of a full recomputation: read the page box, measure
/// every block, then place them. The bands are measured before the first
/// block is placed (`PROSE-076`).
pub fn lay_out(measurer: &mut Measurer, document: &LoroDoc) -> PageLayout {
    let page_box = PageBox::read(document);
    let bands = BandHeights::measure(measurer, document, page_box.content_width_px);
    let notes = NotePlan::read(measurer, document, page_box.content_width_px);
    let blocks = measure_document_with(
        measurer,
        document,
        (page_box.content_width_px, &doc::read_defaults(document)),
        (&HashMap::new(), &HashMap::new()),
        &HashMap::new(),
        &notes,
    );
    let placement = place::place(&blocks, bands.body_height_px(&page_box));
    let mut layout = layout_of(page_box, &placement);
    bands.stamp(&mut layout);
    layout
}

/// The room between a note's number and its text, in pixels.
pub const NOTE_GAP_PX: f32 = 4.0;

/// Every footnote numbered and measured, before any block is placed
/// (`PROSE-117`).
///
/// The ordinals are counted from the calls in document order (`PROSE-068`),
/// so where placement later puts a note never changes its number
/// (`PROSE-099`). A note's text is shaped alone, as the run of its block, with
/// its number beside it. An offset in the run is an offset in the block, so
/// the caret reads a note like any other text (`PROSE-UI-187`).
/// One reachable note: its id, its ordinal, its text and the marked spans
/// of it (`PROSE-UI-207`).
pub(crate) type NoteKey = (
    FootnoteId,
    u32,
    String,
    Vec<crate::layout::measure::MarkedSpan>,
);

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NotePlan {
    /// What the plan was read from: each reachable note, its ordinal, its
    /// text and its marks. A reflow compares it to decide whether the plan
    /// still holds.
    pub(crate) key: Vec<NoteKey>,
    /// The calls of each block: the code-point offset and the note.
    calls: HashMap<TreeID, Vec<(DocOffset, FootnoteId)>>,
    /// Each note's ordinal, shaped height and block.
    notes: HashMap<FootnoteId, (u32, f32, TreeID)>,
    /// Each note's number as it was shaped.
    pub(crate) numbers: HashMap<FootnoteId, ShapedRun>,
    /// Each note's text as it was shaped, under the note's block, indented
    /// past its number.
    pub(crate) texts: Vec<(TreeID, ShapedRun)>,
}

impl NotePlan {
    /// What the plan of `document` would be read from, without shaping a
    /// note. The cheap half a reflow runs on every edit (`PROSE-035`).
    pub(crate) fn key_of(document: &LoroDoc) -> Vec<NoteKey> {
        // A document with no tree holds no call, and reading one would mint
        // the tree (`PROSE-079`).
        if doc::blocks_read(document).is_none() {
            return Vec::new();
        }
        note::ordinals(document)
            .into_iter()
            .map(|(id, ordinal)| {
                let (text, marks) = note::body(document, &id)
                    .map(|body| crate::layout::measure::marked_text(&body))
                    .unwrap_or_default();
                (id, ordinal, text, marks)
            })
            .collect()
    }

    /// Number every note, then shape each one against the content width:
    /// the number first, then the text in the room left beside it.
    pub(crate) fn read(measurer: &mut Measurer, document: &LoroDoc, content_width_px: f32) -> Self {
        let key = Self::key_of(document);
        if key.is_empty() {
            return Self::default();
        }
        let defaults = doc::read_defaults(document);
        let style = EffectiveStyle::from_parts(&defaults, &ParagraphStyle::default(), false);
        let width = style.text_width_px(content_width_px);
        let mut numbers = HashMap::with_capacity(key.len());
        let mut texts = Vec::with_capacity(key.len());
        let mut notes = HashMap::with_capacity(key.len());
        for (id, ordinal, text, marks) in &key {
            let Some(block) = note::block(document, id) else {
                continue;
            };
            let (number_metrics, number) =
                measurer.measure_run(&ordinal.to_string(), &style, width);
            let mut hung = style.clone();
            hung.indent_left_px += number.data().full_width() + NOTE_GAP_PX;
            let (metrics, _, run) = measurer.measure_with_lines(
                (text, marks),
                &hung,
                hung.text_width_px(content_width_px),
                &[],
                None,
            );
            let height = metrics.text_height_px.max(number_metrics.text_height_px);
            numbers.insert(*id, number);
            texts.push((block, run));
            notes.insert(*id, (*ordinal, height, block));
        }
        let mut calls: HashMap<TreeID, Vec<(DocOffset, FootnoteId)>> = HashMap::new();
        for anchor in doc::anchors(document, doc::K_FOOTNOTE) {
            let Ok(id) = FootnoteId::parse_str(&anchor.value) else {
                continue;
            };
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
    fn calls_of(&self, block: &TreeID) -> &[(DocOffset, FootnoteId)] {
        self.calls.get(block).map_or(&[], Vec::as_slice)
    }

    /// One call, placed on `line`.
    fn call(&self, id: &FootnoteId, line: u32) -> Option<NoteCall> {
        let (ordinal, height_px, block_id) = *self.notes.get(id)?;
        Some(NoteCall {
            footnote_id: *id,
            block_id,
            ordinal,
            line,
            height_px,
        })
    }
}

/// The four bands measured, and the settings they were measured under.
#[derive(Clone, Debug, Default)]
pub(crate) struct BandHeights {
    pub(crate) bands: doc::Bands,
    header_px: f32,
    header_first_px: f32,
    footer_px: f32,
    footer_first_px: f32,
    /// The text of each band shown, as it was shaped, for the view to draw.
    pub(crate) runs: Vec<(TreeID, ShapedRun)>,
}

/// Two measurements are equal when they leave every page the same body. The
/// runs are left out: a band edited to a text of the same height moves no
/// block.
impl PartialEq for BandHeights {
    fn eq(&self, other: &Self) -> bool {
        self.bands == other.bands
            && self.header_px == other.header_px
            && self.header_first_px == other.header_first_px
            && self.footer_px == other.footer_px
            && self.footer_first_px == other.footer_first_px
    }
}

impl BandHeights {
    /// Measure every band a page may draw. A hidden side measures zero.
    pub(crate) fn measure(
        measurer: &mut Measurer,
        document: &LoroDoc,
        content_width_px: f32,
    ) -> Self {
        let bands = doc::read_bands(document);
        let mut runs = Vec::new();
        let mut height = |band: doc::Band, shown: bool| match band.block {
            Some(id) if shown => {
                let measured = measure_band(measurer, document, &id, content_width_px);
                if let Some(run) = measured.run {
                    runs.push((id, run));
                }
                measured.height_px
            }
            _ => 0.0,
        };
        let first = bands.different_first_page;
        let header_px = height(bands.header, bands.header_enabled);
        let header_first_px = height(bands.header_first, bands.header_enabled && first);
        let footer_px = height(bands.footer, bands.footer_enabled);
        let footer_first_px = height(bands.footer_first, bands.footer_enabled && first);
        Self {
            bands,
            header_px,
            header_first_px,
            footer_px,
            footer_first_px,
            runs,
        }
    }

    /// The body height every page gets, each band capped at a quarter of the
    /// content height (`PROSE-016`).
    ///
    /// One height for every page: the taller of the regular band and the
    /// first page's own. A page never overflows its bands. The price is
    /// some room lost after the first page when its own bands are the taller.
    pub(crate) fn body_height_px(&self, page_box: &PageBox) -> f32 {
        page_box.body_height_px(
            self.header_px.max(self.header_first_px),
            self.footer_px.max(self.footer_first_px),
        )
    }

    /// Write the heights, capped, and the settings into a layout.
    pub(crate) fn stamp(&self, layout: &mut PageLayout) {
        let cap = (layout.content_height_px * crate::layout::geometry::BAND_SHARE).max(0.0);
        layout.header_height_px = self.header_px.min(cap);
        layout.header_first_height_px = self.header_first_px.min(cap);
        layout.footer_height_px = self.footer_px.min(cap);
        layout.footer_first_height_px = self.footer_first_px.min(cap);
        layout.bands = self.bands;
    }
}

/// Assemble the result from the page box and what placement returned.
#[must_use]
pub(crate) fn layout_of(page_box: PageBox, placement: &Placement) -> PageLayout {
    PageLayout {
        page_width_px: page_box.page_width_px,
        page_height_px: page_box.page_height_px,
        content_width_px: page_box.content_width_px,
        content_height_px: page_box.content_height_px,
        margin_left_px: page_box.margin_left_px,
        margin_top_px: page_box.margin_top_px,
        page_count: placement.page_count,
        blocks: placement.blocks.clone(),
        header_height_px: 0.0,
        header_first_height_px: 0.0,
        footer_height_px: 0.0,
        footer_first_height_px: 0.0,
        first_dirty_page: Some(0),
        // The caller knows what its measurer was missing; this one assembles
        // geometry and placement alone.
        missing_scripts: ScriptSet::new(),
        bands: doc::Bands::default(),
    }
}

/// One band block measured against the content width, before the cap
/// (`PROSE-016`).
#[must_use]
pub(crate) fn measure_band(
    measurer: &mut Measurer,
    document: &LoroDoc,
    id: &TreeID,
    content_width_px: f32,
) -> MeasuredBlock {
    let Some(tree) = doc::blocks_read(document) else {
        return MeasuredBlock {
            block_id: *id,
            block_type: BlockType::Paragraph,
            height_px: 0.0,
            space_before_px: 0.0,
            space_after_px: 0.0,
            keep_together: false,
            keep_with_next: false,
            metrics: None,
            notes: Vec::new(),
            run: None,
            table: None,
        };
    };
    let defaults = doc::read_defaults(document);
    let (cached, toc_pages) = (HashMap::new(), HashMap::new());
    let scope = Walk {
        document,
        tree: &tree,
        defaults: &defaults,
        content_width_px,
        cached: &cached,
        previous: &cached,
        toc_pages: &toc_pages,
        notes: &NotePlan::default(),
    };
    measure_one(measurer, &scope, id, BlockType::Paragraph)
}

/// Measure every block of the document, in flow order (`PROSE-077`).
#[must_use]
pub fn measure_document(
    measurer: &mut Measurer,
    document: &LoroDoc,
    content_width_px: f32,
) -> Vec<MeasuredBlock> {
    let notes = NotePlan::read(measurer, document, content_width_px);
    measure_document_with(
        measurer,
        document,
        (content_width_px, &doc::read_defaults(document)),
        (&HashMap::new(), &HashMap::new()),
        &HashMap::new(),
        &notes,
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
    document: &LoroDoc,
    (content_width_px, defaults): (f32, &doc::DocDefaults),
    (cached, previous): (
        &HashMap<TreeID, &MeasuredBlock>,
        &HashMap<TreeID, &MeasuredBlock>,
    ),
    toc_pages: &HashMap<TreeID, u32>,
    notes: &NotePlan,
) -> Vec<MeasuredBlock> {
    // A measurement already held proves the tree exists, so taking it
    // attaches nothing (`PROSE-079`). Looking it up by path instead builds
    // the value of the whole tree, once per keystroke.
    let tree = if cached.is_empty() && previous.is_empty() {
        doc::blocks_read(document)
    } else {
        Some(document.get_tree(doc::BLOCKS))
    };
    let Some(tree) = tree else {
        return Vec::new();
    };
    let mut measured = Vec::new();
    let scope = Walk {
        document,
        tree: &tree,
        defaults,
        content_width_px,
        cached,
        previous,
        toc_pages,
        notes,
    };
    for root in doc::roots(&tree) {
        walk(measurer, &scope, &root, &mut measured);
    }
    measured
}

/// What the walk carries unchanged from one block to the next.
struct Walk<'a> {
    document: &'a LoroDoc,
    tree: &'a LoroTree,
    defaults: &'a doc::DocDefaults,
    content_width_px: f32,
    cached: &'a HashMap<TreeID, &'a MeasuredBlock>,
    previous: &'a HashMap<TreeID, &'a MeasuredBlock>,
    toc_pages: &'a HashMap<TreeID, u32>,
    notes: &'a NotePlan,
}

/// Measure one block and then its children, in that order.
///
/// A table is the one subtree the walk does not descend into. Its rows are
/// the boundaries it breaks at (`PROSE-071`), so it is measured whole and
/// placed as one block.
fn walk(measurer: &mut Measurer, scope: &Walk<'_>, id: &TreeID, out: &mut Vec<MeasuredBlock>) {
    // A block measured before keeps its type, so only an edited one is
    // looked up in the tree.
    let block_type = match scope.cached.get(id) {
        Some(block) => {
            out.push((*block).clone());
            block.block_type
        }
        None => {
            let Some(block_type) = doc::block_type_of(scope.tree, id) else {
                return;
            };
            out.push(measure_one(measurer, scope, id, block_type));
            block_type
        }
    };
    if block_type == BlockType::Table {
        return;
    }
    for child in doc::children_of(scope.tree, Some(*id)) {
        walk(measurer, scope, &child, out);
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
    let content_width_px = scope.content_width_px;
    let bold = block_type == BlockType::Heading;
    let style = EffectiveStyle::resolve(defaults, tree, id, bold);
    let text_width_px = style.text_width_px(content_width_px);
    let mut notes = Vec::new();
    let mut run = None;
    let mut table = None;
    let (height_px, metrics) = match block_type {
        // A signal, not a sized block.
        BlockType::PageBreak => (0.0, None),
        BlockType::HRule => (RULE_HEIGHT_PX, None),
        BlockType::Image => (
            image_height(measurer, tree, id, &style, text_width_px),
            None,
        ),
        BlockType::Table => {
            let (rows, shape) =
                table_metrics(measurer, document, tree, defaults, id, content_width_px);
            notes = table_calls(scope, id);
            table = Some(Arc::new(shape));
            (rows.text_height_px, Some(rows))
        }
        // One line per entry, measured on the pages the layout before gave
        // the headings. A table of contents that lists nothing measures as
        // nothing.
        BlockType::Toc => {
            let text = crate::toc::build(document, tree, id, scope.toc_pages)
                .map(|toc| crate::toc::lines(&toc))
                .unwrap_or_default();
            if text.is_empty() {
                (0.0, None)
            } else {
                let (metrics, shaped) = measurer.measure_run(&text, &style, text_width_px);
                run = Some(shaped);
                (metrics.text_height_px, Some(metrics))
            }
        }
        _ => {
            let (text, marks) =
                crate::layout::measure::marked_text(&doc::get_block_text(document, id));
            let calls = scope.notes.calls_of(id);
            let offsets: Vec<DocOffset> = calls.iter().map(|(offset, _)| *offset).collect();
            let previous = scope.previous.get(id).and_then(|block| block.run.as_ref());
            let (metrics, lines, shaped) = measurer.measure_with_lines(
                (&text, &marks),
                &style,
                text_width_px,
                &offsets,
                previous,
            );
            run = Some(shaped);
            notes = calls
                .iter()
                .zip(lines)
                .filter_map(|((_, note), line)| scope.notes.call(note, line))
                .collect();
            (metrics.text_height_px, Some(metrics))
        }
    };
    MeasuredBlock {
        block_id: *id,
        block_type,
        height_px,
        space_before_px: style.space_before_px,
        space_after_px: style.space_after_px,
        keep_together: style.keep_together,
        keep_with_next: style.keep_with_next,
        metrics,
        notes,
        run,
        table,
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
        let line = u32::try_from(row_index).unwrap_or(u32::MAX);
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

/// An image box, plus its caption when it has one.
///
/// The height attribute is a display size in pixels, never the native one
/// (`ambre-spec-prose-schema.md section 3`). An image that declares none is
/// measured on its caption alone, because nothing here decodes the file.
fn image_height(
    measurer: &mut Measurer,
    tree: &LoroTree,
    id: &TreeID,
    style: &EffectiveStyle,
    text_width_px: f32,
) -> f32 {
    let attrs = doc::node_of(tree, id).and_then(|node| doc::map_read(&node, doc::K_ATTRS));
    let Some(attrs) = attrs else {
        return 0.0;
    };
    let box_px = number_at(&attrs, A_IMAGE_HEIGHT).unwrap_or(0.0).max(0.0);
    let caption_px = match string_at(&attrs, A_IMAGE_CAPTION) {
        Some(caption) if !caption.is_empty() => {
            measurer
                .measure(&caption, style, text_width_px)
                .text_height_px
        }
        _ => 0.0,
    };
    box_px + caption_px
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
    content_width_px: f32,
) -> (TextMetrics, TableShape) {
    let Some(grid) = Grid::read(tree, table) else {
        let empty = TextMetrics {
            text_height_px: 0.0,
            line_count: 0,
            line_tops: vec![0.0],
        };
        return (empty, TableShape::default());
    };
    let cols = grid.cells.iter().map(Vec::len).max().unwrap_or(0);
    let widths = column_widths_px(&doc::col_widths(tree, table), cols, content_width_px);
    let mut col_x = Vec::with_capacity(cols + 1);
    col_x.push(0.0_f32);
    for width in &widths {
        col_x.push(col_x.last().copied().unwrap_or(0.0) + width);
    }
    let (regions, _) = surviving_regions(&grid, &doc::regions(tree, table));
    let covered: HashSet<(usize, usize)> = regions
        .iter()
        .flat_map(|(_, rect)| {
            (rect.top..=rect.bottom)
                .flat_map(move |row| (rect.left..=rect.right).map(move |col| (row, col)))
                .filter(move |at| *at != (rect.top, rect.left))
        })
        .collect();

    let mut cells = Vec::new();
    let mut heights = Vec::new();
    for (row, row_cells) in grid.cells.iter().enumerate() {
        for (col, cell) in row_cells.iter().enumerate() {
            if covered.contains(&(row, col)) {
                continue;
            }
            let (bottom, right) = regions
                .iter()
                .find(|(_, rect)| (rect.top, rect.left) == (row, col))
                .map_or((row, col), |(_, rect)| (rect.bottom, rect.right));
            let x_px = col_x.get(col).copied().unwrap_or(0.0);
            let right_x = col_x.get(right + 1).copied().unwrap_or(x_px);
            let width_px = right_x - x_px;
            let style = cell_style(tree, cell);
            let text_px = (width_px - 2.0 * style.padding_px).max(0.0);
            let stacked = cell_blocks(measurer, document, tree, defaults, cell, text_px);
            let mut top = 0.0;
            let mut blocks = Vec::new();
            for block in stacked {
                top += block.space_before_px;
                if let Some(run) = block.run {
                    blocks.push((block.block_id, top, run));
                }
                top += block.height_px + block.space_after_px;
            }
            heights.push(top + 2.0 * style.padding_px);
            cells.push(CellShape {
                cell: *cell,
                x_px,
                width_px,
                rows: row..bottom + 1,
                blocks,
                content_px: top,
                style,
            });
        }
    }

    // Single-row cells first, then each region takes what it lacks from the
    // last row it spans.
    let mut row_heights = vec![0.0_f32; grid.rows.len()];
    for (cell, height) in cells.iter().zip(&heights) {
        if cell.rows.len() == 1
            && let Some(slot) = row_heights.get_mut(cell.rows.start)
        {
            *slot = slot.max(*height);
        }
    }
    for (cell, height) in cells.iter().zip(&heights) {
        if cell.rows.len() > 1 {
            let spanned: f32 = row_heights
                .get(cell.rows.clone())
                .map_or(0.0, |r| r.iter().sum());
            if let Some(last) = row_heights.get_mut(cell.rows.end - 1) {
                *last += (height - spanned).max(0.0);
            }
        }
    }
    let mut row_tops = vec![0.0];
    let mut height_px = 0.0;
    for row_height in &row_heights {
        height_px += row_height;
        row_tops.push(height_px);
    }
    let metrics = TextMetrics {
        text_height_px: height_px,
        line_count: u32::try_from(row_heights.len()).unwrap_or(u32::MAX),
        line_tops: row_tops.clone(),
    };
    (metrics, TableShape { row_tops, cells })
}

/// Each column's width in pixels. The stored widths are points, one per
/// column. A list that does not match the grid, or holds an unusable width,
/// gives every column an equal share.
pub(crate) fn column_widths_px(stored_pt: &[f32], cols: usize, content_width_px: f32) -> Vec<f32> {
    let usable = stored_pt.len() == cols
        && stored_pt
            .iter()
            .all(|width| width.is_finite() && *width > 0.0);
    if usable {
        stored_pt.iter().map(|width| width * PX_PER_PT).collect()
    } else {
        let share = if cols == 0 {
            content_width_px
        } else {
            content_width_px / cols as f32
        };
        vec![share; cols]
    }
}

/// The look a cell stores. A value that does not parse, or a width that is
/// negative or not finite, reads as absent.
pub(crate) fn cell_style(tree: &LoroTree, cell: &TreeID) -> CellStyle {
    let Some(attrs) = doc::node_of(tree, cell).and_then(|node| doc::map_read(&node, doc::K_ATTRS))
    else {
        return CellStyle::default();
    };
    let length = |key| {
        number_at(&attrs, key)
            .filter(|pt| pt.is_finite() && *pt >= 0.0)
            .map(|pt| pt * PX_PER_PT)
    };
    let colour = |key| string_at(&attrs, key).and_then(|raw| crate::mark::parse_color(&raw).ok());
    let edges = [
        length(doc::A_EDGE_TOP),
        length(doc::A_EDGE_BOTTOM),
        length(doc::A_EDGE_LEFT),
        length(doc::A_EDGE_RIGHT),
    ];
    let [top, bottom, left, right] = edges;
    let border = edges.iter().any(Option::is_some).then(|| CellBorderPx {
        top: top.unwrap_or(0.0),
        bottom: bottom.unwrap_or(0.0),
        left: left.unwrap_or(0.0),
        right: right.unwrap_or(0.0),
        color: colour(doc::A_EDGE_COLOR),
    });
    let valign = number_at(&attrs, doc::A_VALIGN)
        .and_then(|code| u8::try_from(code as i64).ok())
        .and_then(ambre_bus::app::prose::VAlign::from_code)
        .unwrap_or(ambre_bus::app::prose::VAlign::Top);
    CellStyle {
        fill: colour(doc::A_CELL_BG),
        border,
        padding_px: length(doc::A_PADDING).unwrap_or(0.0),
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
    width_px: f32,
) -> Vec<MeasuredBlock> {
    let mut stacked = Vec::new();
    let empty = HashMap::new();
    let no_pages = HashMap::new();
    let scope = Walk {
        document,
        tree,
        defaults,
        content_width_px: width_px,
        cached: &empty,
        previous: &empty,
        toc_pages: &no_pages,
        notes: &NotePlan::default(),
    };
    for child in doc::children_of(tree, Some(*cell)) {
        walk(measurer, &scope, &child, &mut stacked);
    }
    stacked
}

/// A number an attribute holds, whichever number shape it was written in.
fn number_at(attrs: &LoroMap, key: &str) -> Option<f32> {
    match attrs.get(key).and_then(|value| value.into_value().ok())? {
        LoroValue::Double(number) => Some(number as f32),
        LoroValue::I64(number) => Some(number as f32),
        _ => None,
    }
}

/// A string an attribute holds.
fn string_at(attrs: &LoroMap, key: &str) -> Option<String> {
    match attrs.get(key).and_then(|value| value.into_value().ok())? {
        LoroValue::String(text) => Some(text.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let measured = measure_document(&mut measurer, &document, 400.0);
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
        let measured = measure_document(&mut measurer, &document, 400.0);
        assert_eq!(measured.len(), 2);
        assert_eq!(measured[0].height_px, 0.0);
        assert_eq!(measured[1].height_px, RULE_HEIGHT_PX);
    }

    #[test]
    fn an_image_that_declares_no_size_is_measured_on_what_it_carries() {
        let (document, _) = document_of(&[BlockType::Image]);
        let mut measurer = crate::layout::measure::shipped_measurer();
        let measured = measure_document(&mut measurer, &document, 400.0);
        assert_eq!(measured.len(), 1);
        // Nothing here decodes the file, so an image with no display height
        // and no caption takes no room at all.
        assert_eq!(measured[0].height_px, 0.0);
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
        let measured = measure_document(&mut measurer, &document, 400.0);
        // The rows and the cells are inside the table's own height, so the
        // walk emits the table and stops there.
        assert_eq!(ids_of(&measured), vec![table]);
        let Some(metrics) = measured[0].metrics.as_ref() else {
            return;
        };
        assert_eq!(metrics.line_count, 3);
        assert_eq!(metrics.line_tops.len(), 4);
        assert!(metrics.text_height_px > 0.0);
    }

    #[test]
    fn measuring_the_document_writes_nothing_to_it() {
        let (document, ids) = document_of(&[BlockType::Paragraph, BlockType::Heading]);
        for id in &ids {
            write_text(&document, id, "a line of text long enough to be shaped");
        }
        let before = format!("{:?}", document.get_deep_value());
        let mut measurer = crate::layout::measure::shipped_measurer();
        let _ = lay_out(&mut measurer, &document);
        // `PROSE-079`: laying out reads, and reading never mints a container.
        assert_eq!(format!("{:?}", document.get_deep_value()), before);
    }

    #[test]
    fn an_empty_document_lays_out_on_one_blank_page() {
        let document = LoroDoc::new();
        let before = format!("{:?}", document.get_deep_value());
        let mut measurer = crate::layout::measure::shipped_measurer();
        let layout = lay_out(&mut measurer, &document);
        // A document with no register at all is the case a reader that mints
        // one would be caught by (`PROSE-079`).
        assert_eq!(format!("{:?}", document.get_deep_value()), before);
        assert_eq!(layout.page_count, 1);
        assert_eq!(layout.blocks_on_page(0).count(), 0);
        assert!(layout.page_width_px > 0.0);
        assert!(layout.body_height_px() > 0.0);
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
        let layout = lay_out(&mut measurer, &document);
        assert_eq!(layout.page_count, 2);
        let Some(last) = layout.blocks.last() else {
            return;
        };
        assert_eq!(last.page_start, 1);
        assert_eq!(layout.blocks_on_page(1).count(), 1);
    }

    #[test]
    fn a_document_taller_than_a_page_takes_more_than_one() {
        let paragraphs = vec![BlockType::Paragraph; 60];
        let (document, ids) = document_of(&paragraphs);
        for id in &ids {
            write_text(&document, id, "a paragraph that takes a line of its own");
        }
        let mut measurer = crate::layout::measure::shipped_measurer();
        let layout = lay_out(&mut measurer, &document);
        assert!(layout.page_count > 1);
        // Every page the layout reports is one a reader can ask for
        // (`PROSE-011`).
        for page in 0..layout.page_count {
            let _ = layout.blocks_on_page(page).count();
        }
    }
}
