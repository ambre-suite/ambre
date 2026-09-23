//! Pass 1: what each block costs
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! The walk is a depth-first pre-order traversal of the block tree, and that
//! walk is the document flow (`PROSE-077`). Each block is measured against
//! what its type is, and the result is the flat sequence placement consumes.
//! Nothing here writes to the document it reads (`PROSE-079`).

use std::collections::HashMap;

use ambre_bus::app::prose::BlockType;
use ambre_draw::ScriptSet;
use loro::{LoroDoc, LoroMap, LoroTree, LoroValue, TreeID};

use crate::doc;
use crate::layout::geometry::PageBox;
use crate::layout::measure::{EffectiveStyle, Measurer, TextMetrics};
use crate::layout::place::{self, BlockLayout, MeasuredBlock, Placement};

/// The height of a horizontal rule, in pixels. One device pixel, per the
/// measurement table of section 2.
const RULE_HEIGHT_PX: f32 = 1.0;

/// The two image attributes a height is read from.
///
/// The schema declares them (`ambre-spec-prose-schema.md section 3`), and no
/// command writes them yet. So they carry no constant in `doc.rs`: a key
/// declared there must appear in the frozen shape (`PROSE-029`). They move
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
}

impl PageLayout {
    /// Every fragment visible on a page, continuations of split blocks
    /// included (`PROSE-066`). Empty for a page that carries nothing, and for
    /// one out of range (`PROSE-011`).
    pub fn blocks_on_page(&self, page: u32) -> impl Iterator<Item = &BlockLayout> {
        self.blocks.iter().filter(move |block| block.shows_on(page))
    }

    /// The height left for body content on a page, both bands deducted.
    #[must_use]
    pub fn body_height_px(&self) -> f32 {
        (self.content_height_px - self.header_height_px - self.footer_height_px).max(0.0)
    }
}

/// Lay the whole document out.
///
/// The one entry point of a full recomputation: read the page box, measure
/// every block, then place them. The two bands measure zero until the headers
/// Story lands (`ambre-roadmap.md` E5), and section 5 of the spec carries that
/// gap.
pub fn lay_out(measurer: &mut Measurer, document: &LoroDoc) -> PageLayout {
    let page_box = PageBox::read(document);
    let blocks = measure_document(measurer, document, page_box.content_width_px);
    let body_height_px = page_box.body_height_px(0.0, 0.0);
    let placement = place::place(&blocks, body_height_px);
    layout_of(page_box, &placement)
}

/// Assemble the result from the page box and what placement returned.
#[must_use]
pub(crate) fn layout_of(page_box: PageBox, placement: &Placement) -> PageLayout {
    PageLayout {
        page_width_px: page_box.page_width_px,
        page_height_px: page_box.page_height_px,
        content_width_px: page_box.content_width_px,
        content_height_px: page_box.content_height_px,
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
    }
}

/// Measure every block of the document, in flow order (`PROSE-077`).
#[must_use]
pub fn measure_document(
    measurer: &mut Measurer,
    document: &LoroDoc,
    content_width_px: f32,
) -> Vec<MeasuredBlock> {
    measure_document_with(measurer, document, content_width_px, &HashMap::new())
}

/// Measure every block, reusing a measurement the caller already holds.
///
/// A block found in `cached` is not shaped again (`PROSE-083`). The caller
/// decides what belongs there, which is every block its edit did not touch.
#[must_use]
pub fn measure_document_with(
    measurer: &mut Measurer,
    document: &LoroDoc,
    content_width_px: f32,
    cached: &HashMap<TreeID, &MeasuredBlock>,
) -> Vec<MeasuredBlock> {
    let Some(tree) = doc::blocks_read(document) else {
        return Vec::new();
    };
    let defaults = doc::read_defaults(document);
    let mut measured = Vec::new();
    let scope = Walk {
        document,
        tree: &tree,
        defaults: &defaults,
        content_width_px,
        cached,
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
}

/// Measure one block and then its children, in that order.
///
/// A table is the one subtree the walk does not descend into. Its rows are
/// the boundaries it breaks at (`PROSE-071`), so it is measured whole and
/// placed as one block.
fn walk(measurer: &mut Measurer, scope: &Walk<'_>, id: &TreeID, out: &mut Vec<MeasuredBlock>) {
    let Some(block_type) = doc::block_type_of(scope.tree, id) else {
        return;
    };
    match scope.cached.get(id) {
        Some(block) => out.push((*block).clone()),
        None => out.push(measure_one(
            measurer,
            scope.document,
            scope.tree,
            scope.defaults,
            id,
            block_type,
            scope.content_width_px,
        )),
    }
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
    document: &LoroDoc,
    tree: &LoroTree,
    defaults: &doc::DocDefaults,
    id: &TreeID,
    block_type: BlockType,
    content_width_px: f32,
) -> MeasuredBlock {
    let bold = block_type == BlockType::Heading;
    let style = EffectiveStyle::resolve(defaults, tree, id, bold);
    let text_width_px = style.text_width_px(content_width_px);
    let (height_px, metrics) = match block_type {
        // A signal, not a sized block.
        BlockType::PageBreak => (0.0, None),
        BlockType::HRule => (RULE_HEIGHT_PX, None),
        BlockType::Image => (
            image_height(measurer, tree, id, &style, text_width_px),
            None,
        ),
        BlockType::Table => {
            let rows = table_metrics(measurer, document, tree, defaults, id, content_width_px);
            (rows.text_height_px, Some(rows))
        }
        // The entries are the qualified headings, and the generated block that
        // holds them lands with the table-of-contents Story
        // (`ambre-roadmap.md` E5). An empty one measures as nothing.
        BlockType::Toc => (0.0, None),
        _ => {
            let text = doc::get_block_text(document, id).to_string();
            let metrics = measurer.measure(&text, &style, text_width_px);
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
    }
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
/// Cell padding and borders are not in the schema yet and count as nothing;
/// they land with the tables Story (`ambre-roadmap.md` E5).
fn table_metrics(
    measurer: &mut Measurer,
    document: &LoroDoc,
    tree: &LoroTree,
    defaults: &doc::DocDefaults,
    table: &TreeID,
    content_width_px: f32,
) -> TextMetrics {
    let rows = doc::children_of(tree, Some(*table));
    let cells_wide = rows
        .iter()
        .map(|row| doc::children_of(tree, Some(*row)).len())
        .max()
        .unwrap_or(0);
    let cell_width_px = if cells_wide == 0 {
        content_width_px
    } else {
        content_width_px / cells_wide as f32
    };
    let mut line_tops = vec![0.0];
    let mut height_px = 0.0;
    let mut line_count = 0;
    for row in &rows {
        let mut tallest = 0.0_f32;
        for cell in doc::children_of(tree, Some(*row)) {
            tallest = tallest.max(cell_height(
                measurer,
                document,
                tree,
                defaults,
                &cell,
                cell_width_px,
            ));
        }
        height_px += tallest;
        line_count += 1;
        line_tops.push(height_px);
    }
    TextMetrics {
        text_height_px: height_px,
        line_count,
        line_tops,
    }
}

/// One cell: the blocks it holds, stacked with their margins.
fn cell_height(
    measurer: &mut Measurer,
    document: &LoroDoc,
    tree: &LoroTree,
    defaults: &doc::DocDefaults,
    cell: &TreeID,
    width_px: f32,
) -> f32 {
    let mut stacked = Vec::new();
    let empty = HashMap::new();
    let scope = Walk {
        document,
        tree,
        defaults,
        content_width_px: width_px,
        cached: &empty,
    };
    for child in doc::children_of(tree, Some(*cell)) {
        walk(measurer, &scope, &child, &mut stacked);
    }
    stacked
        .iter()
        .map(|block| block.height_px + block.space_before_px + block.space_after_px)
        .sum()
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
        let mut measurer = Measurer::new();
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
        let mut measurer = Measurer::new();
        let measured = measure_document(&mut measurer, &document, 400.0);
        assert_eq!(measured.len(), 2);
        assert_eq!(measured[0].height_px, 0.0);
        assert_eq!(measured[1].height_px, RULE_HEIGHT_PX);
    }

    #[test]
    fn an_image_that_declares_no_size_is_measured_on_what_it_carries() {
        let (document, _) = document_of(&[BlockType::Image]);
        let mut measurer = Measurer::new();
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
        let mut measurer = Measurer::new();
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
        let mut measurer = Measurer::new();
        let _ = lay_out(&mut measurer, &document);
        // `PROSE-079`: laying out reads, and reading never mints a container.
        assert_eq!(format!("{:?}", document.get_deep_value()), before);
    }

    #[test]
    fn an_empty_document_lays_out_on_one_blank_page() {
        let document = LoroDoc::new();
        let before = format!("{:?}", document.get_deep_value());
        let mut measurer = Measurer::new();
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
        let mut measurer = Measurer::new();
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
        let mut measurer = Measurer::new();
        let layout = lay_out(&mut measurer, &document);
        assert!(layout.page_count > 1);
        // Every page the layout reports is one a reader can ask for
        // (`PROSE-011`).
        for page in 0..layout.page_count {
            let _ = layout.blocks_on_page(page).count();
        }
    }
}
