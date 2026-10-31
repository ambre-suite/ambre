//! The two halves of a conversion that see the document
//! (`features/ambre-spec-prose-import-export.md section 3`).
//!
//! The walk reads the document into the intermediate form, once for every
//! writer (`PROSE-IO-046`). The builder turns a form into a list of block
//! operations in tree order, parent before child (`PROSE-IO-050`). It lifts a
//! block the tree refuses to the nearest parent that takes it
//! (`PROSE-IO-026`).
//!
//! The builder reads no document and writes none
//! (`PROSE-IO-055`). Only [`ProseState::apply_import`] writes. It first
//! rehearses the list on a scratch document, so a refusal writes nothing
//! (`PROSE-IO-011`). Then it writes the list in one commit, so one undo takes
//! the file back (`PROSE-IO-051`). Everything runs on the Logic thread, which holds the
//! document (`PROSE-IO-001`), and nothing reaches the relay but the one
//! encrypted delta the commit produces (`PROSE-IO-043`).

use std::collections::HashMap;

use ambre_bus::app::prose::{DocOffset, FootnoteOrdinal, HeadingLevel};
use ambre_core::id::{Footnote, Id};
use loro::{LoroValue, TreeID};

use ambre_bus::app::prose::{
    BlockInit, BlockType, DroppedElement, ExportFormat, ImportFormat, LiftedBlock, ListStyle,
    OrderedStyle,
};

use super::ProseState;
use super::tables::Grid;
use super::text::{MarkedRun, body_span, marked_runs, write_runs};
use crate::doc::{self, StoredLevel};
use crate::events::grid::{CellPos, Col, ColCount, Row, RowCount};
use crate::events::{ProseEvent, ProseOutcome, ProseRefusal};
use crate::interop::{
    Banding, Commenting, IrBlock, IrFootnote, IrListStyle, IrMeta, IrOrderedStyle, IrRun,
    IrTableCell, IrTableRow, Merging, OwnStyle, Paging, ProseIr, Referencing, Suggesting, Warnings,
    markdown_read, markdown_write,
};
use crate::mark::MarkRoot;

ambre_core::define_index! {
    /// The place of one operation in the list an import builds. A later
    /// operation names its parent by it, before any block exists.
    pub struct OpIndex(u32);
    /// A number of operations.
    pub struct OpCount;
    /// The signed gap between two operations.
    pub struct OpDelta(i64);
    /// A run of operations.
    pub struct OpSpan;
    /// A list indexed by operation.
    pub struct OpVec;
}

/// Where an operation's block goes. It sits at the root, under a block an
/// earlier operation minted, or in a cell of a table one minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Parent {
    Root,
    Op(OpIndex),
    Cell { table: OpIndex, at: CellPos },
}

/// One write of an import. An operation names its parent by the index of the
/// operation that minted it, so the list is built before any block exists.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ImportOp {
    Block {
        parent: Parent,
        init: BlockInit,
        runs: Vec<MarkedRun>,
    },
    Table {
        parent: Parent,
        rows: RowCount,
        cols: ColCount,
    },
}

impl ImportOp {
    fn parent(&self) -> Parent {
        match self {
            Self::Block { parent, .. } | Self::Table { parent, .. } => *parent,
        }
    }
}

/// The type an operation mints, for the nesting check.
fn op_type(ops: &[ImportOp], index: OpIndex) -> Option<BlockType> {
    match ops.get(index.to_usize())? {
        ImportOp::Block { init, .. } => Some(init.block_type()),
        ImportOp::Table { .. } => Some(BlockType::Table),
    }
}

/// The type a parent is, for the nesting check. The root is `None`.
fn parent_type(ops: &[ImportOp], parent: Parent) -> Option<BlockType> {
    match parent {
        Parent::Root => None,
        Parent::Op(index) => op_type(ops, index),
        Parent::Cell { .. } => Some(BlockType::TableCell),
    }
}

fn accepts(ops: &[ImportOp], parent: Parent, child: BlockType) -> bool {
    match parent_type(ops, parent) {
        None => child.accepted_at_root(),
        Some(parent) => parent.accepts(child),
    }
}

/// Build the operations of a form, in tree order (`PROSE-IO-050`). The
/// builder reads no document (`PROSE-IO-055`).
pub(super) fn build(ir: &ProseIr, warnings: &mut Warnings) -> Vec<ImportOp> {
    let mut ops = Vec::new();
    for block in &ir.blocks {
        place(&mut ops, block, &[Parent::Root], warnings);
    }
    ops
}

/// Place `block` under the last parent of `chain`, or the nearest one up the
/// chain that takes it. A lift is reported (`PROSE-IO-052`).
fn place(ops: &mut Vec<ImportOp>, block: &IrBlock, chain: &[Parent], warnings: &mut Warnings) {
    let kind = ir_type(block);
    let mut depth = chain.len();
    while depth > 1
        && chain
            .get(depth - 1)
            .is_some_and(|&p| !accepts(ops, p, kind))
    {
        depth -= 1;
    }
    if depth < chain.len() {
        warnings.lifted(lifted_block(kind));
    }
    // The walk always seeds `chain` with the root, so it is never empty.
    let chain = chain.get(..depth).unwrap_or_default();
    let Some(&parent) = chain.last() else {
        return;
    };
    match block {
        IrBlock::Paragraph { runs, .. } => {
            push(ops, parent, BlockInit::Paragraph, runs);
        }
        IrBlock::Heading { level, runs } => {
            let level = level.clamped();
            push(ops, parent, BlockInit::Heading { level }, runs);
        }
        IrBlock::CodeBlock { lang, text } => {
            let init = BlockInit::CodeBlock { lang: lang.clone() };
            push(ops, parent, init, &[IrRun::plain(text.clone())]);
        }
        IrBlock::HRule => {
            push(ops, parent, BlockInit::HRule, &[]);
        }
        IrBlock::PageBreak => {
            push(ops, parent, BlockInit::PageBreak, &[]);
        }
        IrBlock::ListItem {
            style,
            runs,
            children,
        } => {
            let init = BlockInit::ListItem {
                style: list_style(*style),
            };
            let index = push(ops, parent, init, runs);
            let mut inner = chain.to_vec();
            inner.push(Parent::Op(index));
            for child in children {
                place(ops, child, &inner, warnings);
            }
        }
        IrBlock::Table { rows, .. } => {
            let cols = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
            if rows.is_empty() || cols == 0 {
                return;
            }
            let table = OpIndex::from_usize(ops.len());
            ops.push(ImportOp::Table {
                parent,
                rows: RowCount::from_len(rows.len()),
                cols: ColCount::from_len(cols),
            });
            for (row, cells) in rows.iter().enumerate() {
                for (col, cell) in cells.cells.iter().enumerate() {
                    let at = CellPos::new(Row::from_usize(row), Col::from_usize(col));
                    let mut inner = chain.to_vec();
                    inner.push(Parent::Cell { table, at });
                    for child in &cell.blocks {
                        place(ops, child, &inner, warnings);
                    }
                }
            }
        }
        // No reader lands these yet: an image needs its bytes, and a table of
        // contents is a block of the document's own.
        IrBlock::Image { .. } => warnings.dropped(DroppedElement::Image),
        IrBlock::Toc => warnings.dropped(DroppedElement::TableOfContents),
    }
}

fn push(ops: &mut Vec<ImportOp>, parent: Parent, init: BlockInit, runs: &[IrRun]) -> OpIndex {
    let index = OpIndex::from_usize(ops.len());
    ops.push(ImportOp::Block {
        parent,
        init,
        runs: runs.iter().map(marked_of).collect(),
    });
    index
}

/// A cell stores a look of its own: a fill, a border, a padding or a
/// vertical alignment (`PROSE-IO-004`).
fn cell_styled(tree: &loro::LoroTree, cell: &TreeID) -> bool {
    doc::attrs_read(tree, cell).is_some_and(|attrs| {
        [
            doc::AttrKey::CellBg,
            doc::AttrKey::EdgeTop,
            doc::AttrKey::EdgeBottom,
            doc::AttrKey::EdgeLeft,
            doc::AttrKey::EdgeRight,
            doc::AttrKey::Padding,
            doc::AttrKey::VAlign,
        ]
        .iter()
        .any(|key| attrs.contains(*key))
    })
}

fn ir_type(block: &IrBlock) -> BlockType {
    match block {
        IrBlock::Paragraph { .. } => BlockType::Paragraph,
        IrBlock::Heading { .. } => BlockType::Heading,
        IrBlock::ListItem { .. } => BlockType::ListItem,
        IrBlock::Table { .. } => BlockType::Table,
        IrBlock::CodeBlock { .. } => BlockType::CodeBlock,
        IrBlock::Image { .. } => BlockType::Image,
        IrBlock::PageBreak => BlockType::PageBreak,
        IrBlock::HRule => BlockType::HRule,
        IrBlock::Toc => BlockType::Toc,
    }
}

fn lifted_block(kind: BlockType) -> LiftedBlock {
    match kind {
        BlockType::Paragraph => LiftedBlock::Paragraph,
        BlockType::Heading => LiftedBlock::Heading,
        BlockType::ListItem => LiftedBlock::ListItem,
        BlockType::Table | BlockType::TableRow | BlockType::TableCell => LiftedBlock::Table,
        BlockType::CodeBlock => LiftedBlock::CodeBlock,
        BlockType::Image => LiftedBlock::Image,
        BlockType::PageBreak => LiftedBlock::PageBreak,
        BlockType::HRule => LiftedBlock::Rule,
        BlockType::Toc => LiftedBlock::TableOfContents,
    }
}

fn list_style(style: IrListStyle) -> ListStyle {
    match style {
        IrListStyle::Bullet => ListStyle::Bullet,
        IrListStyle::Todo(done) => ListStyle::Todo(done),
        IrListStyle::Ordered(ordered) => ListStyle::Ordered(match ordered {
            IrOrderedStyle::Decimal => OrderedStyle::Decimal,
            IrOrderedStyle::LowerAlpha => OrderedStyle::LowerAlpha,
            IrOrderedStyle::UpperAlpha => OrderedStyle::UpperAlpha,
            IrOrderedStyle::LowerRoman => OrderedStyle::LowerRoman,
            IrOrderedStyle::UpperRoman => OrderedStyle::UpperRoman,
        }),
    }
}

fn ir_list_style(style: ListStyle) -> IrListStyle {
    match style {
        ListStyle::Bullet => IrListStyle::Bullet,
        ListStyle::Todo(done) => IrListStyle::Todo(done),
        ListStyle::Ordered(ordered) => IrListStyle::Ordered(match ordered {
            OrderedStyle::Decimal => IrOrderedStyle::Decimal,
            OrderedStyle::LowerAlpha => IrOrderedStyle::LowerAlpha,
            OrderedStyle::UpperAlpha => IrOrderedStyle::UpperAlpha,
            OrderedStyle::LowerRoman => IrOrderedStyle::LowerRoman,
            OrderedStyle::UpperRoman => IrOrderedStyle::UpperRoman,
        }),
    }
}

/// The marks a run of the form writes: the formatting it carries, and the
/// link. An anchor a reader cannot carry is never written.
fn marked_of(run: &IrRun) -> MarkedRun {
    let mut marks = Vec::new();
    let flags = [
        (run.bold.as_bool(), MarkRoot::Bold),
        (run.italic.as_bool(), MarkRoot::Italic),
        (run.underline.as_bool(), MarkRoot::Underline),
        (run.strikethrough.as_bool(), MarkRoot::Strike),
        (run.superscript.as_bool(), MarkRoot::Superscript),
        (run.subscript.as_bool(), MarkRoot::Subscript),
        (run.code.as_bool(), MarkRoot::Code),
    ];
    for (on, key) in flags {
        if on {
            marks.push((key.key(), LoroValue::from(true)));
        }
    }
    if let Some(url) = &run.link {
        marks.push((MarkRoot::Link.key(), LoroValue::from(url.as_str())));
    }
    MarkedRun {
        text: run.text.chars().filter(|c| !c.is_control()).collect(),
        marks,
    }
}

/// A run of the document as the form holds it.
fn ir_run(run: MarkedRun, notes: &HashMap<Id<Footnote>, FootnoteOrdinal>) -> IrRun {
    let mut out = IrRun::plain(run.text);
    for (key, value) in run.marks {
        let on = !matches!(value, LoroValue::Null | LoroValue::Bool(false));
        if !on {
            continue;
        }
        match key.root() {
            "b" => out.bold = ambre_core::Weight::Bold,
            "i" => out.italic = ambre_core::Slant::Italic,
            "u" => out.underline = ambre_core::Underline::Underlined,
            "s" => out.strikethrough = ambre_core::Strikethrough::Struck,
            "sup" => out.superscript = crate::mark::Superscript::Raised,
            "sub" => out.subscript = crate::mark::Subscript::Lowered,
            "c" => out.code = crate::mark::InlineCode::Code,
            "a" => {
                if let LoroValue::String(url) = value {
                    out.link = Some(ambre_core::LinkUrl::new(url.to_string()));
                }
            }
            "fg" | "bg" | "fs" | "ff" => out.styled = OwnStyle::Styled,
            "cm" => out.comment = Commenting::Commented,
            "si" | "sd" => out.suggestion = Suggesting::Suggested,
            "ref" => out.cross_ref = Referencing::Reference,
            "fn" => {
                if let LoroValue::String(id) = value
                    && let Ok(id) = Id::<Footnote>::parse_str(&id)
                {
                    out.footnote_ref = notes.get(&id).copied();
                }
            }
            _ => {}
        }
    }
    out
}

impl ProseState {
    /// The document as the intermediate form, the one walk every writer
    /// reads (`PROSE-IO-046`). It writes nothing (`PROSE-IO-017`).
    pub(crate) fn walk(&self) -> ProseIr {
        let tree = doc::blocks(&self.doc);
        let notes: HashMap<Id<Footnote>, FootnoteOrdinal> =
            crate::note::ordinals(&self.doc).into_iter().collect();
        let bands = doc::read_bands(&self.doc);
        let mut footnotes = Vec::new();
        for (id, ordinal) in crate::note::ordinals(&self.doc) {
            if let Some(body) = crate::note::body(&self.doc, &id) {
                let runs = marked_runs(&body, body_span(&body))
                    .into_iter()
                    .map(|run| ir_run(run, &notes))
                    .collect();
                footnotes.push(IrFootnote { ordinal, runs });
            }
        }
        let title = doc::read_title(&self.doc);
        ProseIr {
            meta: IrMeta {
                title: (!title.is_empty()).then_some(title),
                paged: Paging::Paged,
                bands: Banding::from_bool(
                    bands.header_enabled.is_on() || bands.footer_enabled.is_on(),
                ),
            },
            blocks: self.walk_children(&tree, None, &notes),
            footnotes,
        }
    }

    fn walk_children(
        &self,
        tree: &loro::LoroTree,
        parent: Option<TreeID>,
        notes: &HashMap<Id<Footnote>, FootnoteOrdinal>,
    ) -> Vec<IrBlock> {
        let ids = match parent {
            None => doc::roots(tree),
            Some(parent) => doc::children_of(tree, Some(parent)),
        };
        ids.into_iter()
            .filter_map(|id| self.walk_block(tree, id, notes))
            .collect()
    }

    fn walk_block(
        &self,
        tree: &loro::LoroTree,
        id: TreeID,
        notes: &HashMap<Id<Footnote>, FootnoteOrdinal>,
    ) -> Option<IrBlock> {
        let runs = || -> Vec<IrRun> {
            self.body_of(&id)
                .map(|body| {
                    marked_runs(&body, body_span(&body))
                        .into_iter()
                        .map(|run| ir_run(run, notes))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(match doc::block_type_of(tree, &id)? {
            BlockType::Paragraph => IrBlock::Paragraph {
                styled: OwnStyle::from_bool(
                    !doc::read_paragraph_style(tree, &id).is_empty()
                        || doc::style_of(tree, &id).is_some_and(|style| {
                            doc::BuiltInStyle::of(&style) != Some(doc::BuiltInStyle::Body)
                        }),
                ),
                runs: runs(),
            },
            // The stored level, unclamped (`PROSE-FEAT-091`).
            BlockType::Heading => IrBlock::Heading {
                level: doc::heading_level_of(tree, &id)
                    .unwrap_or(StoredLevel::of(HeadingLevel::H1)),
                runs: runs(),
            },
            BlockType::ListItem => IrBlock::ListItem {
                style: ir_list_style(doc::list_style_of(tree, &id).unwrap_or(ListStyle::Bullet)),
                runs: runs(),
                children: self.walk_children(tree, Some(id), notes),
            },
            BlockType::CodeBlock => IrBlock::CodeBlock {
                lang: self.code_lang_of(&crate::events::block_id(id)),
                text: self.body_of(&id).map(|b| b.to_string()).unwrap_or_default(),
            },
            BlockType::Table => {
                let grid = Grid::read(tree, &id)?;
                IrBlock::Table {
                    merged: Merging::from_bool(!doc::regions(tree, &id).is_empty()),
                    rows: grid
                        .cells
                        .iter()
                        .map(|cells| IrTableRow {
                            cells: cells
                                .iter()
                                .map(|cell| IrTableCell {
                                    styled: OwnStyle::from_bool(cell_styled(tree, cell)),
                                    blocks: self.walk_children(tree, Some(*cell), notes),
                                })
                                .collect(),
                        })
                        .collect(),
                }
            }
            BlockType::TableRow | BlockType::TableCell => return None,
            BlockType::Image => IrBlock::Image { alt: String::new() },
            BlockType::PageBreak => IrBlock::PageBreak,
            BlockType::HRule => IrBlock::HRule,
            BlockType::Toc => IrBlock::Toc,
        })
    }

    /// Write the document out in a text format, and the report of what the
    /// format lost (`PROSE-IO-018`). Read-only: no delta (`PROSE-IO-017`).
    pub(super) fn export_document(&self, format: ExportFormat) -> ProseOutcome {
        let ExportFormat::Markdown(flavor) = format else {
            // The native file is the Logic dispatcher's (`PROSE-IO-028`).
            return Self::err_write(doc::WriteError::Refused(doc::RefusalCause::FileByLogic));
        };
        let ir = self.walk();
        // The title is user text, so the name is cleaned of what a host
        // refuses and bounded (`DOCFILE-019`, `DOCFILE-020`).
        let filename = ambre_core::ExportFilename::from_title(
            ir.meta.title.as_deref().unwrap_or_default(),
            "document",
            ambre_core::ExportExtension::Markdown,
        );
        let (text, report) = markdown_write::write(&ir, flavor);
        ProseOutcome::display_only(vec![ProseEvent::ExportReady {
            filename,
            media_type: format.media_type(),
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(text.into_bytes()),
            report,
        }])
    }

    /// Bring a file in after the last block, in one commit
    /// (`PROSE-IO-027`, `PROSE-IO-051`). A file that does not read changes
    /// nothing (`PROSE-IO-011`).
    pub(super) fn import_document(
        &mut self,
        format: ImportFormat,
        data: &ambre_core::Payload<ambre_core::FileBytes>,
    ) -> ProseOutcome {
        let ImportFormat::Markdown = format;
        let (ir, report) = match markdown_read::read(data) {
            Ok(read) => read,
            Err(e) => return ProseOutcome::refused(ProseRefusal::Interop(e)),
        };
        let mut warnings = Warnings::from_report(report);
        let ops = build(&ir, &mut warnings);
        self.apply_import(&ops, warnings)
    }

    /// Write the operations in one commit. The document's lone empty
    /// paragraph, the one a new document holds, gives way to the file.
    ///
    /// The engine takes no write back once made, and a refusal midway would
    /// leave the earlier writes for the next commit (`PROSE-IO-011`). So the
    /// list is first written into a scratch document and dropped. A refusal
    /// there costs nothing. Past it, the only step that reads this document
    /// is where the first operation lands, and it refuses before writing.
    pub(super) fn apply_import(&mut self, ops: &[ImportOp], warnings: Warnings) -> ProseOutcome {
        let report = warnings.report();
        if ops.is_empty() {
            return ProseOutcome::display_only(vec![ProseEvent::ImportCompleted { report }]);
        }
        let Some(mut rehearsal) = Self::new() else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::NoScratchDocument,
            ));
        };
        if let Err(refused) = rehearsal.write_import(ops) {
            return refused;
        }
        if let Err(refused) = self.write_import(ops) {
            return refused;
        }
        self.commit_with(vec![ProseEvent::ImportCompleted { report }])
    }

    /// Write the operations, uncommitted, after the last block.
    ///
    /// # Errors
    /// The refusal to answer with. Writes made before it stay pending.
    fn write_import(&mut self, ops: &[ImportOp]) -> Result<(), ProseOutcome> {
        let tree = doc::blocks(&self.doc);
        let roots = doc::roots(&tree);
        let blank = match roots.as_slice() {
            [only]
                if doc::block_type_of(&tree, only) == Some(BlockType::Paragraph)
                    && doc::get_block_text(&self.doc, only).is_empty()
                    && doc::children_of(&tree, Some(*only)).is_empty() =>
            {
                Some(*only)
            }
            _ => None,
        };
        let mut minted: Vec<Option<TreeID>> = Vec::with_capacity(ops.len());
        let mut grids: HashMap<OpIndex, Grid> = HashMap::new();
        // The last block each parent holds, so the next lands after it. A
        // cell starts at the empty paragraph the table minted in it.
        let mut last: HashMap<Parent, TreeID> = HashMap::new();
        if let Some(root) = roots.last() {
            last.insert(Parent::Root, *root);
        }
        for op in ops {
            let parent = op.parent();
            let parent_id = match parent {
                Parent::Root => None,
                Parent::Op(index) => minted.get(index.to_usize()).copied().flatten(),
                Parent::Cell { table, at } => {
                    let cell = grids.get(&table).and_then(|grid| grid.cell(at));
                    if cell.is_none() {
                        minted.push(None);
                        continue;
                    }
                    cell
                }
            };
            if !matches!(parent, Parent::Root) && parent_id.is_none() {
                minted.push(None);
                continue;
            }
            // The first paragraph of a cell fills the one the table minted.
            if let Parent::Cell { .. } = parent
                && let ImportOp::Block {
                    init: BlockInit::Paragraph,
                    runs,
                    ..
                } = op
                && !last.contains_key(&parent)
                && let Some(cell) = parent_id
                && let Some(first) = doc::children_of(&doc::blocks(&self.doc), Some(cell))
                    .first()
                    .copied()
            {
                if let Err(e) = write_runs(
                    &doc::get_block_text(&self.doc, &first),
                    DocOffset::ZERO,
                    runs,
                ) {
                    return Err(Self::err_write(e));
                }
                last.insert(parent, first);
                minted.push(Some(first));
                continue;
            }
            let after = last.get(&parent).copied();
            let id = match op {
                ImportOp::Block { init, runs, .. } => {
                    let id = self.mint_placed(parent_id, after, init.block_type())?;
                    if let Err(e) = self.write_init(&id, init) {
                        return Err(Self::err_write(e));
                    }
                    if init.block_type().has_body()
                        && let Err(e) =
                            write_runs(&doc::get_block_text(&self.doc, &id), DocOffset::ZERO, runs)
                    {
                        return Err(Self::err_write(e));
                    }
                    id
                }
                ImportOp::Table { rows, cols, .. } => {
                    let id = self.mint_table(parent_id, after, *rows, *cols)?;
                    if let Some(grid) = Grid::read(&doc::blocks(&self.doc), &id) {
                        grids.insert(OpIndex::from_usize(minted.len()), grid);
                    }
                    id
                }
            };
            last.insert(parent, id);
            minted.push(Some(id));
        }
        if let Some(blank) = blank
            && let Err(e) = doc::hide_block(&self.doc, &doc::blocks(&self.doc), &blank)
        {
            return Err(Self::err_write(e));
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::{BlockId, ProseCommand, tree_id};
    use crate::view::tests::append;
    use ambre_bus::app::prose::{
        ConversionWarning, InteropError, MarkdownFlavor, UnsupportedFeature,
    };

    fn document() -> ProseState {
        ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built")
    }

    fn import(state: &mut ProseState, markdown: &str) -> ProseOutcome {
        state.handle_command(ProseCommand::ImportDocument {
            format: ImportFormat::Markdown,
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(markdown.as_bytes().to_vec()),
        })
    }

    fn export(state: &mut ProseState, flavor: MarkdownFlavor) -> (String, Vec<ConversionWarning>) {
        let outcome = state.handle_command(ProseCommand::ExportDocument {
            format: ExportFormat::Markdown(flavor),
        });
        let Some(ProseEvent::ExportReady {
            data,
            report,
            media_type,
            ..
        }) = outcome.display.into_iter().next()
        else {
            panic!("an export answers with a file");
        };
        assert_eq!(media_type.as_str(), "text/markdown; charset=utf-8");
        (
            String::from_utf8(data.into_bytes()).expect("UTF-8"),
            report.warnings,
        )
    }

    fn texts(state: &ProseState) -> Vec<String> {
        let tree = doc::blocks(&state.doc);
        doc::flow(&tree)
            .into_iter()
            .filter_map(|id| state.body_of(&id).map(|b| b.to_string()))
            .collect()
    }

    const SAMPLE: &str = "# Plan\n\nSome **bold** text.\n\n- one\n  - two\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n";

    /// `PROSE-IO-027`, `PROSE-IO-002`, `PROSE-IO-050`: a file lands as the
    /// blocks it holds, nested as it nests, and a new document's empty
    /// paragraph gives way to it.
    #[test]
    fn a_markdown_file_lands_as_blocks() {
        let mut state = document();
        let outcome = import(&mut state, SAMPLE);
        assert!(
            outcome
                .display
                .iter()
                .any(|e| matches!(e, ProseEvent::ImportCompleted { .. })),
            "{:?}",
            outcome.display
        );
        assert_eq!(
            texts(&state),
            [
                "Plan",
                "Some bold text.",
                "one",
                "two",
                "a",
                "b",
                "1",
                "2",
                "let x = 1;"
            ]
        );
        let (written, _) = export(&mut state, MarkdownFlavor::Gfm);
        assert_eq!(written, SAMPLE);
    }

    /// `PROSE-IO-004`: a cell's fill does not survive a pipe table, and the
    /// report names it.
    #[test]
    fn a_styled_cell_is_reported_on_export() {
        let mut state = document();
        import(&mut state, SAMPLE);
        let tree = doc::blocks(&state.doc);
        let cell = doc::flow(&tree)
            .into_iter()
            .find(|id| doc::block_type_of(&tree, id) == Some(BlockType::TableCell))
            .expect("the sample holds a table");
        state.handle_command(ProseCommand::SetCellStyle {
            cell_id: crate::events::block_id(cell),
            bg: Some(ambre_core::Rgb::from_bits(0x00ff_8800)),
            valign: None,
            border: None,
            padding: None,
        });
        let (_, warnings) = export(&mut state, MarkdownFlavor::Gfm);
        assert!(
            warnings.iter().any(|w| matches!(
                w,
                ConversionWarning::FeatureNotSupported {
                    feature: UnsupportedFeature::CellFormatting,
                    count,
                    ..
                } if count.get() == 1
            )),
            "{warnings:?}"
        );
    }

    /// `PROSE-IO-051`: one undo takes the whole file back.
    #[test]
    fn one_undo_takes_the_import_back() {
        let mut state = document();
        append(&mut state, BlockInit::Paragraph, "kept");
        let before = texts(&state);
        import(&mut state, SAMPLE);
        assert_ne!(texts(&state), before);
        state.handle_command(ProseCommand::Undo);
        assert_eq!(texts(&state), before);
    }

    /// `PROSE-IO-011`, `PROSE-IO-047`: a file that does not read is a typed
    /// refusal, and the document is untouched.
    #[test]
    fn a_file_that_does_not_read_changes_nothing() {
        let mut state = document();
        let before = state.doc.oplog_vv();
        let outcome = state.handle_command(ProseCommand::ImportDocument {
            format: ImportFormat::Markdown,
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(vec![0xff, 0xfe, 0xfd]),
        });
        assert!(outcome.update.is_none());
        assert!(matches!(
            outcome.display.as_slice(),
            [ProseEvent::Error(error)]
                if matches!(error.reason, ProseRefusal::Interop(InteropError::ParseFailed(_)))
        ));
        assert_eq!(state.doc.oplog_vv(), before);
    }

    /// `PROSE-IO-011`: an operation the tree refuses after others have
    /// written leaves nothing behind. The builder never emits a paragraph
    /// under a paragraph, so the list is written by hand.
    #[test]
    fn a_refusal_midway_leaves_nothing_for_the_next_commit() {
        let mut state = document();
        let before = texts(&state);
        let paragraph = |parent, text: &str| ImportOp::Block {
            parent,
            init: BlockInit::Paragraph,
            runs: vec![MarkedRun {
                text: text.to_owned(),
                marks: Vec::new(),
            }],
        };
        let ops = [
            paragraph(Parent::Root, "first"),
            paragraph(Parent::Root, "second"),
            paragraph(Parent::Op(OpIndex::new(1)), "refused"),
        ];
        let warnings = Warnings::new(ambre_bus::app::prose::ConversionFormat::Markdown);
        let outcome = state.apply_import(&ops, warnings);
        assert!(outcome.update.is_none());
        assert!(matches!(outcome.display.as_slice(), [ProseEvent::Error(_)]));
        let vv = state.doc.oplog_vv();
        state.doc.commit();
        assert_eq!(state.doc.oplog_vv(), vv, "no write was left pending");
        assert_eq!(texts(&state), before);
    }

    /// `PROSE-IO-026`, `PROSE-IO-052`: a code block inside a list item, which
    /// the tree refuses, lands after the item, and the report says so.
    #[test]
    fn a_block_the_item_refuses_is_lifted_and_reported() {
        let mut state = document();
        let outcome = import(&mut state, "- item\n\n  ```\n  code\n  ```\n");
        let Some(ProseEvent::ImportCompleted { report }) = outcome
            .display
            .iter()
            .find(|e| matches!(e, ProseEvent::ImportCompleted { .. }))
        else {
            panic!("no report");
        };
        assert!(report.warnings.iter().any(|w| matches!(
            w,
            ConversionWarning::BlockLifted {
                block: LiftedBlock::CodeBlock,
                ..
            }
        )));
        let tree = doc::blocks(&state.doc);
        let roots = doc::roots(&tree);
        assert_eq!(roots.len(), 2);
        assert_eq!(
            doc::block_type_of(&tree, &roots[1]),
            Some(BlockType::CodeBlock)
        );
    }

    /// `PROSE-IO-003`, `PROSE-IO-017`: the same document writes the same
    /// bytes twice, and writing them emits no delta.
    #[test]
    fn an_export_is_idempotent_and_writes_nothing() {
        let mut state = document();
        import(&mut state, SAMPLE);
        let before = state.doc.oplog_vv();
        let first = export(&mut state, MarkdownFlavor::Gfm);
        let second = export(&mut state, MarkdownFlavor::Gfm);
        assert_eq!(first, second);
        assert_eq!(state.doc.oplog_vv(), before);
    }

    /// `PROSE-FEAT-091`: the walk carries the stored level
    /// as it is, even one no heading style has.
    #[test]
    fn the_walk_keeps_a_stored_level_outside_the_range() {
        let mut state = document();
        let heading: BlockId = append(
            &mut state,
            BlockInit::Heading {
                level: HeadingLevel::H2,
            },
            "deep",
        );
        let tree = doc::blocks(&state.doc);
        let attrs =
            doc::attrs_at(&doc::node_of(&tree, &tree_id(heading)).expect("node")).expect("attrs");
        attrs.insert(doc::AttrKey::Level, 9_i64).expect("written");
        state.doc.commit();
        let ir = state.walk();
        assert!(ir.blocks.contains(&IrBlock::Heading {
            level: StoredLevel::from_stored(9),
            runs: vec![IrRun::plain("deep")],
        }));
    }

    /// `PROSE-IO-018`: the page model is reported on every Markdown export.
    #[test]
    fn the_page_model_is_reported() {
        let mut state = document();
        let (_, warnings) = export(&mut state, MarkdownFlavor::CommonMark);
        assert!(warnings.iter().any(|w| matches!(
            w,
            ConversionWarning::FeatureNotSupported {
                feature: UnsupportedFeature::PageFormat,
                ..
            }
        )));
    }
}
