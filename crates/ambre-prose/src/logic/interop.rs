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

use crate::message::{DocChar, FootnoteCall, HeadingLevel};
use ambre_core::id::{Comment, Footnote, Id, Reply, Suggestion};
use ambre_core::index::{Axis, Len, Pos};
use ambre_core::{Ordinal, StoredAsI64};
use loro::{LoroValue, TreeID};
use std::collections::HashMap;

use crate::message::{
    BlockInit, BlockType, DroppedElement, ExportFormat, ImportFormat, LiftedBlock, ListStyle,
    OrderedStyle,
};

use super::ProseState;
use super::tables::Grid;
use super::text::{MarkedRun, body_span, marked_runs, write_runs};
use crate::doc::{self, StoredLevel};
use crate::events::grid::{CellPos, Col, Row};
use crate::events::{AnchorState, SuggestionKind};
use crate::events::{ProseEvent, ProseOutcome};
use crate::interop::{
    Banding, IrBlock, IrChange, IrComment, IrFootnote, IrListStyle, IrMeta, IrOrderedStyle,
    IrReply, IrRun, IrSuggestion, IrTableCell, IrTableRow, IrThread, Merging, OwnStyle, Paging,
    ProseIr, Referencing, Warnings,
};
use crate::mark::MarkRoot;
use crate::message::{ConversionReport, InteropError};
use crate::{comment, suggest};
use ambre_bus::interop::{Direction, InteropAnswer};

/// The place of one operation in the list an import builds. A later
/// operation names its parent by it, before any block exists.
#[derive(PartialEq, Eq)]
pub enum Op {}

impl Axis for Op {
    const NAME: &'static str = "Op";
}

/// Where an operation's block goes. It sits at the root, under a block an
/// earlier operation minted, or in a cell of a table one minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Parent {
    Root,
    Op(Pos<Op>),
    Cell { table: Pos<Op>, at: CellPos },
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
        rows: Len<Row>,
        cols: Len<Col>,
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
fn op_type(ops: &[ImportOp], index: Pos<Op>) -> Option<BlockType> {
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

/// The document's id of each thread and each suggestion of a form, minted
/// before the build, so the builder marks the runs and mints nothing.
#[derive(Debug, Default)]
pub(super) struct ImportIds {
    pub(super) threads: Vec<Id<Comment>>,
    pub(super) suggestions: Vec<Id<Suggestion>>,
}

impl ImportIds {
    fn thread(&self, at: Pos<IrThread>) -> Option<&Id<Comment>> {
        self.threads.get(at.to_usize())
    }

    fn suggestion(&self, at: Option<Pos<IrChange>>) -> Option<&Id<Suggestion>> {
        self.suggestions.get(at?.to_usize())
    }
}

/// Build the operations of a form, in tree order (`PROSE-IO-050`). The
/// builder reads no document (`PROSE-IO-055`). Without `ids`, the session
/// has no identity to stamp an annotation with: each comment and suggestion
/// is dropped and reported, and its text kept.
pub(super) fn build(
    ir: &ProseIr,
    ids: Option<&ImportIds>,
    warnings: &mut Warnings,
) -> Vec<ImportOp> {
    let mut ops = Vec::new();
    let mut builder = Builder { ids, warnings };
    for block in &ir.blocks {
        builder.place(&mut ops, block, &[Parent::Root]);
    }
    ops
}

/// What the builder reads beside the form: the minted ids, and where it
/// reports.
struct Builder<'a, 'w> {
    ids: Option<&'a ImportIds>,
    warnings: &'w mut Warnings,
}

impl Builder<'_, '_> {
    /// Place `block` under the last parent of `chain`, or the nearest one up the
    /// chain that takes it. A lift is reported (`PROSE-IO-052`).
    fn place(&mut self, ops: &mut Vec<ImportOp>, block: &IrBlock, chain: &[Parent]) {
        let warnings = &mut *self.warnings;
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
            IrBlock::Paragraph { styled, runs } => {
                // The form carries the flag and none of the values, so the
                // look is lost and named (`PROSE-IO-087`).
                if *styled == OwnStyle::Styled {
                    self.warnings
                        .unsupported(crate::message::UnsupportedFeature::ParagraphFormatting);
                }
                self.push(ops, parent, BlockInit::Paragraph, runs);
            }
            IrBlock::Heading { level, runs } => {
                let level = level.clamped();
                self.push(ops, parent, BlockInit::Heading { level }, runs);
            }
            IrBlock::CodeBlock { lang, text } => {
                let init = BlockInit::CodeBlock { lang: lang.clone() };
                self.push(ops, parent, init, &[IrRun::plain(text.clone())]);
            }
            IrBlock::HRule => {
                self.push(ops, parent, BlockInit::HRule, &[]);
            }
            IrBlock::PageBreak => {
                self.push(ops, parent, BlockInit::PageBreak, &[]);
            }
            IrBlock::ListItem {
                style,
                runs,
                children,
            } => {
                let init = BlockInit::ListItem {
                    style: list_style(*style),
                };
                let index = self.push(ops, parent, init, runs);
                let mut inner = chain.to_vec();
                inner.push(Parent::Op(index));
                for child in children {
                    self.place(ops, child, &inner);
                }
            }
            IrBlock::Table { rows, .. } => {
                let cols = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
                if rows.is_empty() || cols == 0 {
                    return;
                }
                let table = Pos::<Op>::from_usize(ops.len());
                ops.push(ImportOp::Table {
                    parent,
                    rows: Len::<Row>::of(rows.len()),
                    cols: Len::<Col>::of(cols),
                });
                for (row, cells) in rows.iter().enumerate() {
                    for (col, cell) in cells.cells.iter().enumerate() {
                        if cell.styled == OwnStyle::Styled {
                            self.warnings
                                .unsupported(crate::message::UnsupportedFeature::CellFormatting);
                        }
                        let at =
                            CellPos::new(Pos::<Row>::from_usize(row), Pos::<Col>::from_usize(col));
                        let mut inner = chain.to_vec();
                        inner.push(Parent::Cell { table, at });
                        for child in &cell.blocks {
                            self.place(ops, child, &inner);
                        }
                    }
                }
            }
            // No reader lands these yet: an image needs its bytes, and a table of
            // contents is a block of the document's own.
            IrBlock::Image { .. } => self.warnings.dropped(DroppedElement::Image),
            IrBlock::Toc => self.warnings.dropped(DroppedElement::TableOfContents),
        }
    }

    fn push(
        &mut self,
        ops: &mut Vec<ImportOp>,
        parent: Parent,
        init: BlockInit,
        runs: &[IrRun],
    ) -> Pos<Op> {
        let index = Pos::<Op>::from_usize(ops.len());
        let runs = runs.iter().map(|run| self.marked_of(run)).collect();
        ops.push(ImportOp::Block { parent, init, runs });
        index
    }

    /// The marks a run of the form writes: the formatting it carries, the link,
    /// and the threads and the suggestions it lies under (`PROSE-IO-060`). A
    /// footnote call is not built on import, so it is reported and its text
    /// kept.
    fn marked_of(&mut self, run: &IrRun) -> MarkedRun {
        let mut marked = marked_of(run);
        if run.styled == OwnStyle::Styled {
            self.warnings
                .unsupported(crate::message::UnsupportedFeature::Styling);
        }
        if run.footnote_ref.is_some() {
            self.warnings.dropped(DroppedElement::Footnote);
        }
        let Some(ids) = self.ids else {
            if !run.comments.is_empty() {
                self.warnings
                    .unsupported(crate::message::UnsupportedFeature::Comment);
            }
            if run.insertion.is_some() || run.deletion.is_some() {
                self.warnings
                    .unsupported(crate::message::UnsupportedFeature::Suggestion);
            }
            return marked;
        };
        for thread in run.comments.iter().filter_map(|at| ids.thread(*at)) {
            marked.marks.push((
                comment::mark_key(thread),
                LoroValue::from(thread.to_hyphenated()),
            ));
        }
        let changes = [
            (SuggestionKind::Insertion, ids.suggestion(run.insertion)),
            (SuggestionKind::Deletion, ids.suggestion(run.deletion)),
        ];
        for (kind, id) in changes {
            if let Some(id) = id {
                marked.marks.push((
                    suggest::mark_key(kind, id),
                    LoroValue::from(id.to_hyphenated()),
                ));
            }
        }
        marked
    }
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

/// What the walk reads beside a run: the ordinal of each footnote, and the
/// place of each thread and each suggestion in the form.
#[derive(Default)]
pub(super) struct WalkMaps {
    notes: HashMap<Id<Footnote>, Ordinal<FootnoteCall>>,
    threads: HashMap<Id<Comment>, Pos<IrThread>>,
    suggestions: HashMap<Id<Suggestion>, Pos<IrChange>>,
}

/// A run of the document as the form holds it.
fn ir_run(run: MarkedRun, maps: &WalkMaps) -> IrRun {
    let mut out = IrRun::plain(run.text);
    for (key, value) in run.marks {
        if let Some((kind, id)) = suggest::of_key(key.as_str()) {
            let at = maps.suggestions.get(&id).copied();
            match kind {
                SuggestionKind::Insertion => out.insertion = out.insertion.or(at),
                SuggestionKind::Deletion => out.deletion = out.deletion.or(at),
            }
            continue;
        }
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
            "cm" => {
                if let LoroValue::String(id) = value
                    && let Ok(id) = Id::<Comment>::parse_str(&id)
                    && let Some(at) = maps.threads.get(&id)
                    && !out.comments.contains(at)
                {
                    out.comments.push(*at);
                }
            }
            "ref" => out.cross_ref = Referencing::Reference,
            "fn" => {
                if let LoroValue::String(id) = value
                    && let Ok(id) = Id::<Footnote>::parse_str(&id)
                {
                    out.footnote_ref = maps.notes.get(&id).copied();
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
        let (comments, suggestions, mut maps) = self.walk_annotations();
        maps.notes = crate::note::ordinals(&self.doc).into_iter().collect();
        let bands = doc::read_bands(&self.doc);
        let mut footnotes = Vec::new();
        for (id, ordinal) in crate::note::ordinals(&self.doc) {
            if let Some(body) = crate::note::body(&self.doc, &id) {
                let runs = marked_runs(&body, body_span(&body))
                    .into_iter()
                    .map(|run| ir_run(run, &maps))
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
            blocks: self.walk_children(&tree, None, &maps),
            footnotes,
            comments,
            suggestions,
        }
    }

    /// The threads a mark still anchors, and the pending suggestions, as the
    /// form lists them, and the place of each by its id. An orphaned thread
    /// anchors on no text, so no format could place it.
    fn walk_annotations(&self) -> (Vec<IrComment>, Vec<IrSuggestion>, WalkMaps) {
        let mut maps = WalkMaps::default();
        let comments = comment::threads(&self.doc)
            .into_iter()
            .filter(|thread| thread.orphaned == AnchorState::Anchored)
            .enumerate()
            .map(|(at, thread)| {
                maps.threads
                    .insert(thread.comment_id, Pos::<IrThread>::from_usize(at));
                IrComment {
                    resolved: thread.resolved,
                    replies: thread
                        .replies
                        .into_iter()
                        .map(|reply| IrReply {
                            author: reply.author,
                            created_at: Some(reply.created_at),
                            text: reply.text,
                        })
                        .collect(),
                }
            })
            .collect();
        let suggestions = suggest::pending(&self.doc)
            .into_iter()
            .enumerate()
            .map(|(at, suggestion)| {
                maps.suggestions
                    .insert(suggestion.suggestion_id, Pos::<IrChange>::from_usize(at));
                IrSuggestion {
                    kind: suggestion.kind,
                    author: suggestion.author,
                    date: None,
                }
            })
            .collect();
        (comments, suggestions, maps)
    }

    fn walk_children(
        &self,
        tree: &loro::LoroTree,
        parent: Option<TreeID>,
        maps: &WalkMaps,
    ) -> Vec<IrBlock> {
        let ids = match parent {
            None => doc::roots(tree),
            Some(parent) => doc::children_of(tree, Some(parent)),
        };
        ids.into_iter()
            .filter_map(|id| self.walk_block(tree, id, maps))
            .collect()
    }

    fn walk_block(&self, tree: &loro::LoroTree, id: TreeID, maps: &WalkMaps) -> Option<IrBlock> {
        let runs = || -> Vec<IrRun> {
            self.body_of(&id)
                .map(|body| {
                    marked_runs(&body, body_span(&body))
                        .into_iter()
                        .map(|run| ir_run(run, maps))
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
                children: self.walk_children(tree, Some(id), maps),
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
                                    blocks: self.walk_children(tree, Some(*cell), maps),
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

    /// An export that reaches the view command path. Each format a module
    /// writes runs in [`Self::handle_conversion`] once the Logic dispatcher
    /// loaded its module, so one that arrives here had none.
    pub(super) fn export_document(&self, format: ExportFormat) -> ProseOutcome {
        match format {
            // The native file is the Logic dispatcher's (`PROSE-IO-028`).
            ExportFormat::Ambre => {
                Self::err_write(doc::WriteError::Refused(doc::RefusalCause::FileByLogic))
            }
            ExportFormat::Markdown(_) | ExportFormat::Word(_) | ExportFormat::OpenDocument(_) => {
                ProseOutcome::display_only(vec![ProseEvent::Interop(InteropAnswer::failed(
                    Direction::Export,
                    InteropError::Stopped("the converter is not loaded".to_owned()),
                ))])
            }
        }
    }

    /// Bring a file in after the last block, in one commit
    /// (`PROSE-IO-027`, `PROSE-IO-051`). A file that does not read changes
    /// nothing (`PROSE-IO-011`).
    pub(super) fn import_document(
        &mut self,
        format: ImportFormat,
        read: Result<(ProseIr, ConversionReport), InteropError>,
        now: ambre_core::Instant<ambre_core::Authored>,
    ) -> ProseOutcome {
        let _ = format;
        let (ir, report) = match read {
            Ok(read) => read,
            Err(e) => {
                return ProseOutcome::display_only(vec![ProseEvent::Interop(
                    InteropAnswer::failed(Direction::Import, e),
                )]);
            }
        };
        let mut warnings = Warnings::from_report(report);
        // An annotation is stamped with the importing session's identity, as
        // one written by hand is (`PROSE-FEAT-262`, `PROSE-FEAT-077`).
        let ids = self.my_user_id.map(|_| ImportIds {
            threads: ir
                .comments
                .iter()
                .map(|_| self.ids.mint::<Id<Comment>>())
                .collect(),
            suggestions: ir
                .suggestions
                .iter()
                .map(|_| self.ids.mint::<Id<Suggestion>>())
                .collect(),
        });
        let ops = build(&ir, ids.as_ref(), &mut warnings);
        let annotations = ids.as_ref().map(|ids| Annotations {
            ids,
            comments: &ir.comments,
            suggestions: &ir.suggestions,
            now,
        });
        self.apply_import(&ops, warnings, annotations.as_ref())
    }

    /// Write the operations in one commit. The document's lone empty
    /// paragraph, the one a new document holds, gives way to the file.
    ///
    /// The engine takes no write back once made, and a refusal midway would
    /// leave the earlier writes for the next commit (`PROSE-IO-011`). So the
    /// list is first written into a scratch document and dropped. A refusal
    /// there costs nothing. Past it, the only step that reads this document
    /// is where the first operation lands, and it refuses before writing.
    pub(super) fn apply_import(
        &mut self,
        ops: &[ImportOp],
        warnings: Warnings,
        annotations: Option<&Annotations<'_>>,
    ) -> ProseOutcome {
        let report = warnings.report();
        if ops.is_empty() {
            return ProseOutcome::display_only(vec![ProseEvent::Interop(
                InteropAnswer::ImportCompleted { report },
            )]);
        }
        let Some(mut rehearsal) = Self::new() else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::NoScratchDocument,
            ));
        };
        if let Err(refused) = rehearsal.write_import(ops) {
            return refused;
        }
        let minted = match self.write_import(ops) {
            Ok(minted) => minted,
            Err(refused) => return refused,
        };
        let mut display = vec![ProseEvent::Interop(InteropAnswer::ImportCompleted {
            report,
        })];
        if let Some(annotations) = annotations
            && !(annotations.comments.is_empty() && annotations.suggestions.is_empty())
        {
            if let Err(e) = self.write_annotations(annotations, ops, &minted) {
                return Self::err_write(e);
            }
            display.push(ProseEvent::CommentsUpdated {
                threads: comment::threads(&self.doc),
            });
            display.push(self.suggestions_event());
        }
        self.commit_with(display)
    }

    /// The entry of each thread and each suggestion the import marked. It
    /// lands in the commit of the blocks, so one undo takes the file back
    /// whole (`PROSE-IO-051`). A thread anchors on the first block its mark
    /// covers, as a hand-made one does. A thread no run names anchors
    /// nowhere, so it is not written.
    fn write_annotations(
        &self,
        annotations: &Annotations<'_>,
        ops: &[ImportOp],
        minted: &[Option<TreeID>],
    ) -> Result<(), doc::WriteError> {
        let user = self.my_user_id.ok_or(doc::WriteError::NoIdentity)?;
        let anchors = anchors(ops, minted);
        for (id, thread) in annotations.ids.threads.iter().zip(annotations.comments) {
            let Some(span) = anchors.get(id).copied() else {
                continue;
            };
            let entry = self.write_thread(id, span)?;
            entry.insert(
                doc::AnnotKey::Resolved,
                thread.resolved == ambre_core::ThreadState::Resolved,
            )?;
            let replies = comment::replies_of(&entry)
                .ok_or(doc::WriteError::Refused(doc::RefusalCause::NoReplyList))?;
            for reply in &thread.replies {
                let map = replies
                    .push_container(loro::LoroMap::new())
                    .map(doc::Fields::<doc::ReplyKey>::new)?;
                map.insert(
                    doc::ReplyKey::Id,
                    self.ids.mint::<Id<Reply>>().to_hyphenated(),
                )?;
                map.insert(doc::ReplyKey::Author, author_text(reply.author.as_ref()))?;
                map.insert(doc::ReplyKey::User, user.to_string())?;
                let created = reply.created_at.unwrap_or(annotations.now);
                map.insert(doc::ReplyKey::Created, created.to_i64_saturating())?;
                let body = map.insert_container(doc::ReplyKey::Text, loro::LoroText::new())?;
                body.insert(0, &reply.text)?;
            }
        }
        let annots = doc::annots(&self.doc);
        for (id, suggestion) in annotations
            .ids
            .suggestions
            .iter()
            .zip(annotations.suggestions)
        {
            let entry = annots
                .insert_container(&id.to_hyphenated(), loro::LoroMap::new())
                .map(doc::Fields::<doc::AnnotKey>::new)?;
            entry.insert(doc::AnnotKey::Kind, doc::AnnotKind::Suggestion.as_str())?;
            entry.insert(
                doc::AnnotKey::Author,
                author_text(suggestion.author.as_ref()),
            )?;
            entry.insert(doc::AnnotKey::User, user.to_string())?;
        }
        Ok(())
    }

    /// Write the operations, uncommitted, after the last block.
    ///
    /// # Errors
    /// The refusal to answer with. Writes made before it stay pending.
    fn write_import(&mut self, ops: &[ImportOp]) -> Result<Vec<Option<TreeID>>, ProseOutcome> {
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
        let mut grids: HashMap<Pos<Op>, Grid> = HashMap::new();
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
                    Pos::<DocChar>::ZERO,
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
                        && let Err(e) = write_runs(
                            &doc::get_block_text(&self.doc, &id),
                            Pos::<DocChar>::ZERO,
                            runs,
                        )
                    {
                        return Err(Self::err_write(e));
                    }
                    id
                }
                ImportOp::Table { rows, cols, .. } => {
                    let id = self.mint_table(parent_id, after, *rows, *cols)?;
                    if let Some(grid) = Grid::read(&doc::blocks(&self.doc), &id) {
                        grids.insert(Pos::<Op>::from_usize(minted.len()), grid);
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
        Ok(minted)
    }
}

/// The threads and the suggestions an import writes beside its blocks, each
/// with the id the builder marked its runs with.
pub(super) struct Annotations<'a> {
    ids: &'a ImportIds,
    comments: &'a [IrComment],
    suggestions: &'a [IrSuggestion],
    /// When the user chose the file: the date of a reply the file leaves
    /// undated.
    now: ambre_core::Instant<ambre_core::Authored>,
}

/// The name an imported annotation carries, or the empty text a stamp with
/// no name holds.
fn author_text(name: Option<&ambre_core::Validated<ambre_core::DisplayName>>) -> String {
    name.map(|name| name.as_str().to_owned())
        .unwrap_or_default()
}

/// Where each thread's mark lands: the first block an operation minted
/// that carries it, over the runs of that block that carry it.
fn anchors(ops: &[ImportOp], minted: &[Option<TreeID>]) -> HashMap<Id<Comment>, comment::Span> {
    let mut out: HashMap<Id<Comment>, comment::Span> = HashMap::new();
    for (op, block) in ops.iter().zip(minted) {
        let (ImportOp::Block { runs, .. }, Some(block)) = (op, block) else {
            continue;
        };
        let mut at = Pos::<DocChar>::ZERO;
        for run in runs {
            let end = at + Len::<DocChar>::of(run.text.chars().count());
            for (key, value) in &run.marks {
                if key.root() != crate::mark::MarkRoot::Comment.as_str() {
                    continue;
                }
                let LoroValue::String(id) = value else {
                    continue;
                };
                let Ok(id) = Id::<Comment>::parse_str(id) else {
                    continue;
                };
                out.entry(id)
                    .and_modify(|span| {
                        if span.block == *block {
                            span.end = end;
                        }
                    })
                    .or_insert(comment::Span {
                        block: *block,
                        start: at,
                        end,
                    });
            }
            at = end;
        }
    }
    out
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::events::{BlockId, ProseCommand, tree_id};
    use crate::message::{
        ConversionReport, InteropError, MarkdownFlavor, ProseConstruct, ProseImport,
        UnsupportedFeature,
    };
    use crate::view::tests::append;
    use ambre_bus::interop::{InteropFailure, InteropRequest, Outcome};

    fn document() -> ProseState {
        ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built")
    }

    /// When a test chooses its file: a whole second, as a Word date is.
    const IMPORTED_AT: ambre_core::Instant<ambre_core::Authored> =
        ambre_core::Instant::<ambre_core::Authored>::new(1_800_000_000_000);

    fn import(state: &mut ProseState, markdown: &str) -> ProseOutcome {
        state.handle_converted(ProseCommand::Interop(InteropRequest::Import(ProseImport {
            format: ImportFormat::Markdown,
            data: ambre_core::Payload::<ambre_core::FileBytes>::new(markdown.as_bytes().to_vec()),
            now: IMPORTED_AT,
            zone: crate::message::TimeZone::UTC,
        })))
    }

    fn export(state: &mut ProseState, flavor: MarkdownFlavor) -> (String, ConversionReport) {
        let outcome = state.handle_converted(ProseCommand::Interop(InteropRequest::Export(
            ExportFormat::Markdown(flavor),
        )));
        let Some(ProseEvent::Interop(InteropAnswer::ExportReady { file, report })) =
            outcome.display.into_iter().next()
        else {
            panic!("an export answers with a file");
        };
        assert_eq!(file.media_type.as_str(), "text/markdown; charset=utf-8");
        (
            String::from_utf8(file.data.into_bytes()).expect("UTF-8"),
            report,
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
            outcome.display.iter().any(|e| matches!(
                e,
                ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
            )),
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
        let (_, report) = export(&mut state, MarkdownFlavor::Gfm);
        assert_eq!(
            report
                .count(
                    ProseConstruct::Feature(UnsupportedFeature::CellFormatting),
                    Outcome::Dropped
                )
                .get(),
            1,
            "{report:?}"
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
        let outcome =
            state.handle_converted(ProseCommand::Interop(InteropRequest::Import(ProseImport {
                format: ImportFormat::Markdown,
                data: ambre_core::Payload::<ambre_core::FileBytes>::new(vec![0xff, 0xfe, 0xfd]),
                now: IMPORTED_AT,
                zone: crate::message::TimeZone::UTC,
            })));
        assert!(outcome.update.is_none());
        assert!(matches!(
            outcome.display.as_slice(),
            [ProseEvent::Interop(InteropAnswer::Failed(InteropFailure {
                direction: Direction::Import,
                error: InteropError::ParseFailed(_),
            }))]
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
            paragraph(Parent::Op(Pos::<Op>::new(1)), "refused"),
        ];
        let warnings = Warnings::new(Direction::Import, ambre_bus::interop::FileFormat::Markdown);
        let outcome = state.apply_import(&ops, warnings, None);
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
        let Some(ProseEvent::Interop(InteropAnswer::ImportCompleted { report })) =
            outcome.display.iter().find(|e| {
                matches!(
                    e,
                    ProseEvent::Interop(InteropAnswer::ImportCompleted { .. })
                )
            })
        else {
            panic!("no report");
        };
        assert_eq!(
            report
                .count(
                    ProseConstruct::Block(LiftedBlock::CodeBlock),
                    Outcome::Moved
                )
                .get(),
            1
        );
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
        let (_, report) = export(&mut state, MarkdownFlavor::CommonMark);
        assert!(
            !report
                .count(
                    ProseConstruct::Feature(UnsupportedFeature::PageFormat),
                    Outcome::Dropped
                )
                .is_zero()
        );
    }

    mod word {
        use super::*;
        use ambre_core::id::User;
        use ambre_docx::model::{
            Block, Comment as WordComment, CommentState, Inline, Paragraph, Revision, RevisionKind,
            Run, WordId,
        };
        use ambre_docx::{WordConstruct, WordDocument};

        const ALICE: Id<User> = Id::from_bytes([1; 16]);

        fn signed_in() -> ProseState {
            let mut state = document();
            state.set_user_id(ALICE);
            state
        }

        fn run(text: &str, revision: Option<(RevisionKind, u32, &str)>) -> Inline {
            Inline::Run(Run {
                revision: revision.map(|(kind, id, author)| Revision {
                    kind,
                    id: WordId::new(id),
                    author: author.to_owned(),
                    date: None,
                }),
                ..Run::plain(text)
            })
        }

        fn body_comment(id: u32, author: &str, text: &str, parent: Option<u32>) -> WordComment {
            WordComment {
                id: WordId::new(id),
                author: author.to_owned(),
                date: Some(ambre_docx::model::WordDate::Zoned {
                    utc: ambre_core::Instant::<ambre_core::Authored>::new(1_700_000_000_000),
                    wall: ambre_core::Instant::<ambre_core::Authored>::new(1_700_000_000_000),
                }),
                paragraphs: vec![Paragraph {
                    inlines: vec![Inline::Run(Run::plain(text))],
                    ..Paragraph::default()
                }],
                parent: parent.map(WordId::new),
                state: CommentState::Open,
            }
        }

        /// A Word file of one paragraph: a comment with a reply over
        /// `world`, a tracked insertion by Bob, and a tracked deletion by Eve.
        fn reviewed() -> Vec<u8> {
            let document = WordDocument {
                body: vec![Block::Paragraph(Paragraph {
                    inlines: vec![
                        run("Hello ", None),
                        Inline::CommentStart(WordId::new(0)),
                        run("world", None),
                        Inline::CommentEnd(WordId::new(0)),
                        run(" new", Some((RevisionKind::Insertion, 2, "Bob"))),
                        run(" old", Some((RevisionKind::Deletion, 3, "Eve"))),
                    ],
                    ..Paragraph::default()
                })],
                comments: vec![
                    body_comment(0, "Ada", "Check this.", None),
                    body_comment(1, "Bob", "Done.", Some(0)),
                ],
                ..WordDocument::default()
            };
            ambre_docx::write(&document)
                .expect("the model writes")
                .into_bytes()
        }

        fn import(state: &mut ProseState, bytes: Vec<u8>) -> ConversionReport {
            let outcome = state.handle_converted_in_family(ProseCommand::Interop(
                InteropRequest::Import(ProseImport {
                    format: ImportFormat::Word,
                    data: ambre_core::Payload::<ambre_core::FileBytes>::new(bytes),
                    now: IMPORTED_AT,
                    zone: crate::message::TimeZone::UTC,
                }),
            ));
            outcome
                .display
                .into_iter()
                .find_map(|event| match event {
                    ProseEvent::Interop(InteropAnswer::ImportCompleted { report }) => Some(report),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the import completes"))
        }

        /// `PROSE-IO-060`: a Word comment lands as a thread with its reply,
        /// each with its author, and the tracked changes land as suggestions
        /// with theirs. No body of a comment reaches the text.
        #[test]
        fn a_word_comment_and_tracked_change_land_as_a_thread_and_a_suggestion() {
            let mut state = signed_in();
            let report = import(&mut state, reviewed());
            let threads = comment::threads(&state.doc);
            let [thread] = threads.as_slice() else {
                panic!("one thread, got {}", threads.len());
            };
            let replies: Vec<(Option<&str>, &str)> = thread
                .replies
                .iter()
                .map(|reply| {
                    (
                        reply.author.as_ref().map(|a| a.as_str()),
                        reply.text.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                replies,
                [(Some("Ada"), "Check this."), (Some("Bob"), "Done.")]
            );
            assert_eq!(thread.orphaned, AnchorState::Anchored);
            assert_eq!(
                thread.anchor_end.get() - thread.anchor_start.get(),
                5,
                "the thread anchors on `world`"
            );
            let suggestions = suggest::pending(&state.doc);
            let kinds: Vec<(SuggestionKind, Option<&str>)> = suggestions
                .iter()
                .map(|s| (s.kind, s.author.as_ref().map(|a| a.as_str())))
                .collect();
            assert_eq!(
                kinds,
                [
                    (SuggestionKind::Insertion, Some("Bob")),
                    (SuggestionKind::Deletion, Some("Eve")),
                ]
            );
            assert!(suggestions.iter().all(|s| s.user_id == ALICE));
            let text = texts(&state).concat();
            assert_eq!(text, "Hello world new old");
            assert!(!text.contains("Check this") && !text.contains("Done"));
            assert_eq!(
                report
                    .count(
                        ProseConstruct::Word(WordConstruct::Comment),
                        Outcome::Carried
                    )
                    .get(),
                2
            );
        }

        /// `PROSE-IO-083`: a Word comment with no date lands at the time the
        /// user chose the file. The form the module answers marks its date as
        /// unknown.
        #[test]
        fn an_undated_word_comment_lands_at_the_import_time() {
            let document = WordDocument {
                body: vec![Block::Paragraph(Paragraph {
                    inlines: vec![
                        Inline::CommentStart(WordId::new(0)),
                        run("dated by nobody", None),
                        Inline::CommentEnd(WordId::new(0)),
                    ],
                    ..Paragraph::default()
                })],
                comments: vec![WordComment {
                    date: None,
                    ..body_comment(0, "Ada", "When?", None)
                }],
                ..WordDocument::default()
            };
            let decoded = ambre_docx::read(
                ambre_docx::write(&document)
                    .expect("the model writes")
                    .as_bytes(),
            )
            .expect("the package reads");
            let (ir, _) = ambre_prose_form::word_read::read_document(
                &decoded.document,
                &decoded.report,
                &ambre_prose_form::zone::Zone::utc(),
            );
            assert_eq!(ir.comments[0].replies[0].created_at, None);
            let mut state = signed_in();
            import(
                &mut state,
                ambre_docx::write(&document)
                    .expect("the model writes")
                    .into_bytes(),
            );
            let threads = comment::threads(&state.doc);
            assert_eq!(threads[0].replies[0].created_at, IMPORTED_AT);
        }

        /// `PROSE-IO-087`: a look the form flags and carries no value of is
        /// named on import: a styled run, a styled paragraph, a styled cell.
        #[test]
        fn a_look_the_form_flags_is_named_on_import() {
            let ir = ProseIr {
                blocks: vec![
                    IrBlock::Paragraph {
                        styled: OwnStyle::Styled,
                        runs: vec![IrRun {
                            styled: OwnStyle::Styled,
                            ..IrRun::plain("red")
                        }],
                    },
                    IrBlock::Table {
                        merged: Merging::Unmerged,
                        rows: vec![IrTableRow {
                            cells: vec![IrTableCell {
                                styled: OwnStyle::Styled,
                                blocks: Vec::new(),
                            }],
                        }],
                    },
                ],
                ..ProseIr::default()
            };
            let mut warnings =
                Warnings::new(Direction::Import, ambre_bus::interop::FileFormat::Docx);
            build(&ir, None, &mut warnings);
            let report = warnings.report();
            for feature in [
                UnsupportedFeature::Styling,
                UnsupportedFeature::ParagraphFormatting,
                UnsupportedFeature::CellFormatting,
            ] {
                assert_eq!(
                    report
                        .count(ProseConstruct::Feature(feature), Outcome::Dropped)
                        .get(),
                    1,
                    "{feature:?}"
                );
            }
        }

        /// `PROSE-IO-060`: the comments of a file Word's own test set holds
        /// land as threads.
        #[test]
        fn a_real_word_file_lands_its_comments_as_threads() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../ambre-docx/tests/fixtures/pandoc/comments.docx"
            );
            let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let mut state = signed_in();
            import(&mut state, bytes);
            let threads = comment::threads(&state.doc);
            assert!(!threads.is_empty());
            assert!(
                threads
                    .iter()
                    .all(|t| t.replies.iter().all(|r| r.author.is_some()))
            );
        }

        /// A session with no identity has no one to stamp an annotation
        /// with: each comment and each suggestion is reported, its text kept.
        #[test]
        fn without_an_identity_annotations_are_reported_and_their_text_kept() {
            let mut state = document();
            let report = import(&mut state, reviewed());
            assert!(comment::threads(&state.doc).is_empty());
            assert!(suggest::pending(&state.doc).is_empty());
            assert_eq!(texts(&state).concat(), "Hello world new old");
            assert!(
                !report
                    .count(
                        ProseConstruct::Feature(UnsupportedFeature::Comment),
                        Outcome::Dropped
                    )
                    .is_zero()
            );
            assert!(
                !report
                    .count(
                        ProseConstruct::Feature(UnsupportedFeature::Suggestion),
                        Outcome::Dropped
                    )
                    .is_zero()
            );
        }

        /// `PROSE-IO-024`: a macro project lands as nothing but its name in
        /// the report. Its bytes inflate past the bound of a part, so a
        /// reader that read them would fail the import.
        #[test]
        fn a_word_macro_is_reported_and_never_run() {
            use ambre_docx::opc::{PackageWriter, PartName, RelKind, RelTarget};
            let part = |name: &str| PartName::new(name).expect("a part name");
            let main = part("word/document.xml");
            let project = part("word/vbaProject.bin");
            let mut writer = PackageWriter::new();
            writer.relate(
                &ambre_docx::opc::root(),
                RelKind::OFFICE_DOCUMENT,
                RelTarget::Part(main.clone()),
            );
            writer.relate(
                &main,
                ambre_docx::rel::VBA_PROJECT,
                RelTarget::Part(project.clone()),
            );
            writer.part(
                main,
                Some("application/vnd.ms-word.document.macroEnabled.main+xml"),
                ambre_core::Payload::new(
                    b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>hi</w:t></w:r></w:p></w:body></w:document>".to_vec(),
                ),
            );
            let size = usize::try_from(ambre_docx::opc::MAX_PART_SIZE.get()).expect("fits") + 1;
            writer.part(
                project,
                Some("application/vnd.ms-office.vbaProject"),
                ambre_core::Payload::new(vec![0; size]),
            );
            let bytes = writer.finish().expect("writes").into_bytes();
            let mut state = signed_in();
            let report = import(&mut state, bytes);
            assert_eq!(texts(&state), ["hi"]);
            assert_eq!(
                report
                    .count(ProseConstruct::Word(WordConstruct::Macro), Outcome::Dropped)
                    .get(),
                1
            );
        }

        /// `PROSE-IO-014`: a Word export of a reviewed document reads back
        /// with its text, its thread and its tracked changes. It writes no
        /// delta.
        #[test]
        fn a_word_export_reads_back_with_its_threads_and_changes() {
            let mut state = signed_in();
            import(&mut state, reviewed());
            let outcome = state.handle_converted_in_family(ProseCommand::Interop(
                InteropRequest::Export(ExportFormat::Word(crate::message::WordExport {
                    review: crate::message::ReviewExport::Included,
                    zone: crate::message::TimeZone::UTC,
                })),
            ));
            assert!(outcome.update.is_none(), "an export writes nothing");
            let Some(ProseEvent::Interop(InteropAnswer::ExportReady { file, report })) =
                outcome.display.into_iter().next()
            else {
                panic!("an export answers with a file");
            };
            assert_eq!(
                file.media_type.as_str(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            );
            assert!(file.filename.as_str().ends_with(".docx"));
            let decoded = ambre_docx::read(file.data.as_bytes()).expect("the package reads");
            let (ir, _) = ambre_prose_form::word_read::read_document(
                &decoded.document,
                &decoded.report,
                &ambre_prose_form::zone::Zone::utc(),
            );
            assert_eq!(ir.comments.len(), 1);
            assert_eq!(ir.comments[0].replies.len(), 2);
            assert_eq!(ir.suggestions.len(), 2);
            assert!(
                report
                    .entries()
                    .iter()
                    .all(|e| e.outcome != Outcome::Dropped)
            );
        }

        /// `PROSE-IO-081`: the plain Word export carries no thread and no
        /// suggestion. Its text is the text before the review: the suggested
        /// insertion is left out, and the suggested deletion stays.
        #[test]
        fn a_word_export_without_the_review_carries_none_of_it() {
            let mut state = signed_in();
            import(&mut state, reviewed());
            let outcome = state.handle_converted_in_family(ProseCommand::Interop(
                InteropRequest::Export(ExportFormat::Word(crate::message::WordExport {
                    review: crate::message::ReviewExport::Omitted,
                    zone: crate::message::TimeZone::UTC,
                })),
            ));
            let Some(ProseEvent::Interop(InteropAnswer::ExportReady { file, .. })) =
                outcome.display.into_iter().next()
            else {
                panic!("an export answers with a file");
            };
            let decoded = ambre_docx::read(file.data.as_bytes()).expect("the package reads");
            assert!(decoded.document.comments.is_empty());
            let (ir, _) = ambre_prose_form::word_read::read_document(
                &decoded.document,
                &decoded.report,
                &ambre_prose_form::zone::Zone::utc(),
            );
            assert!(ir.suggestions.is_empty());
            let text: String = ir
                .blocks
                .iter()
                .flat_map(|block| match block {
                    IrBlock::Paragraph { runs, .. } => runs.clone(),
                    _ => Vec::new(),
                })
                .map(|run| run.text)
                .collect();
            assert_eq!(text, "Hello world old");
        }

        /// The fixture pairs of the OpenDocument codec: one source written
        /// as an `.odt` and as a `.docx`.
        const ODT_PAIRS: [&str; 3] = ["libreoffice/features", "pandoc/inline", "pandoc/structure"];

        fn import_as(
            state: &mut ProseState,
            bytes: Vec<u8>,
            format: ImportFormat,
        ) -> ConversionReport {
            let outcome = state.handle_converted_in_family(ProseCommand::Interop(
                InteropRequest::Import(ProseImport {
                    format,
                    data: ambre_core::Payload::<ambre_core::FileBytes>::new(bytes),
                    now: IMPORTED_AT,
                    zone: crate::message::TimeZone::UTC,
                }),
            ));
            outcome
                .display
                .into_iter()
                .find_map(|event| match event {
                    ProseEvent::Interop(InteropAnswer::ImportCompleted { report }) => Some(report),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the import completes"))
        }

        /// The runs of a walk, two neighbours of the same marks joined.
        /// Where a file cuts its runs is no data the user reads.
        fn joined(blocks: &mut [IrBlock]) {
            for block in blocks {
                match block {
                    IrBlock::Paragraph { runs, .. } | IrBlock::Heading { runs, .. } => {
                        join(runs);
                    }
                    IrBlock::ListItem { runs, children, .. } => {
                        join(runs);
                        joined(children);
                    }
                    IrBlock::Table { rows, .. } => {
                        for cell in rows.iter_mut().flat_map(|row| row.cells.iter_mut()) {
                            joined(&mut cell.blocks);
                        }
                    }
                    _ => {}
                }
            }
        }

        fn join(runs: &mut Vec<IrRun>) {
            let mut out: Vec<IrRun> = Vec::new();
            for run in runs.drain(..) {
                if let Some(last) = out.last_mut() {
                    let (mut a, mut b) = (last.clone(), run.clone());
                    a.text.clear();
                    b.text.clear();
                    if a == b {
                        last.text.push_str(&run.text);
                        continue;
                    }
                }
                out.push(run);
            }
            *runs = out;
        }

        /// `PROSE-IO-088`: an OpenDocument text lands as the document its
        /// Word twin lands as. The blocks, the threads, the suggestions and
        /// the report are the same.
        #[test]
        fn an_odt_file_lands_as_the_document_its_word_twin_lands_as() {
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-odt/tests/fixtures");
            for pair in ODT_PAIRS {
                let file = |extension: &str| {
                    let path = format!("{dir}/{pair}.{extension}");
                    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
                };
                let mut from_odt = signed_in();
                let odt_report = import_as(&mut from_odt, file("odt"), ImportFormat::OpenDocument);
                let mut from_docx = signed_in();
                let docx_report = import_as(&mut from_docx, file("docx"), ImportFormat::Word);
                let (mut odt, mut docx) = (from_odt.walk(), from_docx.walk());
                joined(&mut odt.blocks);
                joined(&mut docx.blocks);
                assert!(!odt.blocks.is_empty(), "{pair}");
                assert_eq!(odt, docx, "{pair}");
                assert_eq!(odt_report.entries(), docx_report.entries(), "{pair}");
                assert_eq!(odt_report.format(), ambre_bus::interop::FileFormat::Odt);
            }
        }

        /// `PROSE-IO-089`: an OpenDocument export, imported again, lands as
        /// the document the Word export lands as. The threads, the replies,
        /// the suggestions and the report are the same both ways.
        #[test]
        fn an_odt_export_reimports_as_the_word_export_does() {
            use crate::message::{ReviewExport, WordExport};
            for review in [ReviewExport::Omitted, ReviewExport::Included] {
                let mut state = signed_in();
                import(&mut state, reviewed());
                let mut export = |format| {
                    let outcome = state.handle_converted_in_family(ProseCommand::Interop(
                        InteropRequest::Export(format),
                    ));
                    assert!(outcome.update.is_none(), "an export writes nothing");
                    match outcome.display.into_iter().next() {
                        Some(ProseEvent::Interop(InteropAnswer::ExportReady { file, report })) => {
                            (file, report)
                        }
                        _ => panic!("an export answers with a file"),
                    }
                };
                let options = WordExport {
                    review,
                    zone: crate::message::TimeZone::UTC,
                };
                let (odt, odt_report) = export(ExportFormat::OpenDocument(options.clone()));
                let (docx, docx_report) = export(ExportFormat::Word(options));
                assert_eq!(
                    odt.media_type.as_str(),
                    "application/vnd.oasis.opendocument.text"
                );
                assert!(odt.filename.as_str().ends_with(".odt"));
                assert_eq!(odt_report.entries(), docx_report.entries());
                let mut from_odt = signed_in();
                let back_odt = import_as(
                    &mut from_odt,
                    odt.data.into_bytes(),
                    ImportFormat::OpenDocument,
                );
                let mut from_docx = signed_in();
                let back_docx =
                    import_as(&mut from_docx, docx.data.into_bytes(), ImportFormat::Word);
                let (mut a, mut b) = (from_odt.walk(), from_docx.walk());
                joined(&mut a.blocks);
                joined(&mut b.blocks);
                assert_eq!(a, b, "{review:?}");
                assert_eq!(back_odt.entries(), back_docx.entries(), "{review:?}");
                if review == ReviewExport::Included {
                    assert_eq!(a.comments.len(), 1);
                    assert_eq!(a.comments[0].replies.len(), 2);
                    assert_eq!(a.suggestions.len(), 2);
                }
            }
        }
    }
}
