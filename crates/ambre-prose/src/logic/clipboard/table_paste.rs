//! A paste of HTML that holds a table (`PROSE-UI-225`).
//!
//! The markup module lays each table out on its grid. A merged cell is one
//! cell, and the slots it covers point back at it. The paste mints a table
//! of that grid and fills each cell with its blocks. It writes one region per
//! merged cell, the record a merge command writes. The whole paste is one
//! commit, so one undo takes it back.

use ambre_core::index::{Len, Pos};
use loro::TreeID;

use ambre_prose_form::clip::{ClipBlock, ClipCell, ClipCover, ClipItem, ClipKind, ClipTable};

use super::{init, len_of, marked};
use crate::doc;
use crate::events::grid::{CellPos, Col, Row};
use crate::events::{BlockInit, BlockType, ProseEvent, ProseOutcome, block_id};
use crate::logic::ProseState;
use crate::logic::tables::{Grid, next_stamp};
use crate::logic::text::{marked_runs, write_runs};
use crate::message::DocChar;
use ambre_core::index::Span;

/// Where the pasted items land around the paragraph of the caret.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// The paragraph splits at the caret, and the items land between its
    /// two halves.
    Split,
    /// The items land before the paragraph, which keeps its text: a table
    /// pasted at the start of a paragraph leaves no empty one above it.
    Before,
}

/// The block the last pasted item ended in, and the offset after it.
type Caret = (TreeID, Pos<DocChar>);

impl ProseState {
    /// Write `items` at `at`, in one commit. A first block joins the
    /// paragraph at `at`, and a last block takes the text that followed
    /// `at`, as a paste of blocks does. A last table is followed by a
    /// paragraph that takes that text, so the caret has a place after it.
    pub(super) fn paste_items(
        &mut self,
        at: (TreeID, Pos<DocChar>),
        items: Vec<ClipItem>,
    ) -> ProseOutcome {
        let Some(body) = self.body_of(&at.0) else {
            return Self::err_no_block(&at.0);
        };
        let pos = at.1;
        let len = Len::<DocChar>::of(body.len_unicode()).as_end();
        if pos > len {
            return Self::err_bad_range();
        }
        let landing = match items.first() {
            Some(ClipItem::Table(_)) if pos == Pos::<DocChar>::ZERO => Landing::Before,
            _ => Landing::Split,
        };
        let tail = match landing {
            Landing::Split => marked_runs(&body, Span::<DocChar>::new(pos, len)),
            Landing::Before => Vec::new(),
        };
        if landing == Landing::Split
            && pos < len
            && let Err(e) = body.delete(pos.to_usize(), len.len_from(pos).to_usize())
        {
            return Self::err_write(e);
        }
        // The block the next item lands after; `None` lands it before `at`.
        let mut previous = match landing {
            Landing::Split => Some(at.0),
            Landing::Before => None,
        };
        let mut caret: Option<Caret> = None;
        for (index, item) in items.into_iter().enumerate() {
            let placed = match item {
                ClipItem::Block(block) if index == 0 => self.join_first(at, len, &block),
                ClipItem::Block(block) => self.place_block(at.0, previous, &block),
                ClipItem::Table(table) => self.place_table(at.0, previous, &table),
            };
            match placed {
                Ok((id, end)) => {
                    previous = Some(id);
                    caret = end.map(|end| (id, end));
                }
                Err(refused) => return refused,
            }
        }
        let caret = match (landing, caret) {
            (Landing::Split, Some((id, offset))) => {
                let text = doc::get_block_text(&self.doc, &id);
                if let Err(e) = write_runs(&text, offset, &tail) {
                    return Self::err_write(e);
                }
                (id, offset)
            }
            (Landing::Split, None) => {
                let empty = ClipBlock {
                    kind: ClipKind::Paragraph,
                    runs: Vec::new(),
                };
                match self.place_block(at.0, previous, &empty) {
                    Ok((id, _)) => {
                        let text = doc::get_block_text(&self.doc, &id);
                        if let Err(e) = write_runs(&text, Pos::<DocChar>::ZERO, &tail) {
                            return Self::err_write(e);
                        }
                        (id, Pos::<DocChar>::ZERO)
                    }
                    Err(refused) => return refused,
                }
            }
            (Landing::Before, Some(caret)) => caret,
            (Landing::Before, None) => (at.0, Pos::<DocChar>::ZERO),
        };
        self.commit_with(vec![ProseEvent::CaretPlaced {
            block: block_id(caret.0),
            offset: caret.1,
        }])
    }

    /// Write the first pasted block into the paragraph at `at`, which takes
    /// its type when it is empty.
    fn join_first(
        &mut self,
        at: (TreeID, Pos<DocChar>),
        len: Pos<DocChar>,
        block: &ClipBlock,
    ) -> Result<(TreeID, Option<Pos<DocChar>>), ProseOutcome> {
        if len == Pos::<DocChar>::ZERO {
            self.adopt(&at.0, block)?;
        }
        let text = doc::get_block_text(&self.doc, &at.0);
        write_runs(&text, at.1, &marked(block)).map_err(Self::err_write)?;
        Ok((at.0, Some(at.1 + len_of(block))))
    }

    /// Mint a block after `previous`, or before `target` when none, and
    /// write `block` into it.
    fn place_block(
        &self,
        target: TreeID,
        previous: Option<TreeID>,
        block: &ClipBlock,
    ) -> Result<(TreeID, Option<Pos<DocChar>>), ProseOutcome> {
        let init = init(&block.kind);
        let id = self.mint_beside(target, previous, init.block_type())?;
        self.write_init(&id, &init).map_err(Self::err_write)?;
        let text = doc::get_block_text(&self.doc, &id);
        write_runs(&text, Pos::<DocChar>::ZERO, &marked(block)).map_err(Self::err_write)?;
        Ok((id, Some(len_of(block).as_end())))
    }

    /// Mint a block of `kind` after `previous`, or before `target` when
    /// none.
    fn mint_beside(
        &self,
        target: TreeID,
        previous: Option<TreeID>,
        kind: BlockType,
    ) -> Result<TreeID, ProseOutcome> {
        let id = self.mint_placed(None, Some(previous.unwrap_or(target)), kind)?;
        if previous.is_none() {
            doc::blocks(&self.doc)
                .mov_before(id, target)
                .map_err(Self::err_write)?;
        }
        Ok(id)
    }

    /// Mint a table of `table`'s grid after `previous`, or before `target`
    /// when none, fill its cells and write its merged cells as regions.
    /// A table ends no text, so it places no caret.
    fn place_table(
        &self,
        target: TreeID,
        previous: Option<TreeID>,
        table: &ClipTable,
    ) -> Result<(TreeID, Option<Pos<DocChar>>), ProseOutcome> {
        let rows = Len::<Row>::of(table.rows.len());
        let cols = Len::<Col>::of(
            table
                .rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        );
        let id = self.mint_table(None, Some(previous.unwrap_or(target)), rows, cols)?;
        if previous.is_none() {
            doc::blocks(&self.doc)
                .mov_before(id, target)
                .map_err(Self::err_write)?;
        }
        let Some(grid) = Grid::read(&doc::blocks(&self.doc), &id) else {
            return Err(Self::err_no_block(&id));
        };
        for (row, cells) in table.rows.iter().enumerate() {
            for (col, cell) in cells.cells.iter().enumerate() {
                let at = CellPos::new(Pos::<Row>::from_usize(row), Pos::<Col>::from_usize(col));
                let (ClipCell::Own(blocks), Some(cell_id)) = (cell, grid.cell(at)) else {
                    continue;
                };
                self.fill_cell(cell_id, blocks)?;
                if let Some(extent) = grid.cell(extent_of(table, at))
                    && extent != cell_id
                {
                    self.write_region(&id, &cell_id, &extent, next_stamp(&self.doc))
                        .map_err(Self::err_write)?;
                }
            }
        }
        Ok((id, None))
    }

    /// Write `blocks` into a fresh cell, whose one empty paragraph takes the
    /// first. A kind a cell does not hold, a code block, lands as a
    /// paragraph.
    fn fill_cell(&self, cell: TreeID, blocks: &[ClipBlock]) -> Result<(), ProseOutcome> {
        let tree = doc::blocks(&self.doc);
        let Some(mut previous) = doc::children_of(&tree, Some(cell)).first().copied() else {
            return Err(Self::err_no_block(&cell));
        };
        for (index, block) in blocks.iter().enumerate() {
            let init = match init(&block.kind) {
                init if BlockType::TableCell.accepts(init.block_type()) => init,
                _ => BlockInit::Paragraph,
            };
            let id = if index == 0 {
                self.retype(&previous, &init).map_err(Self::err_write)?;
                previous
            } else {
                self.mint_placed(None, Some(previous), init.block_type())?
            };
            if index > 0 {
                self.write_init(&id, &init).map_err(Self::err_write)?;
            }
            let text = doc::get_block_text(&self.doc, &id);
            write_runs(&text, Pos::<DocChar>::ZERO, &marked(block)).map_err(Self::err_write)?;
            previous = id;
        }
        Ok(())
    }

    /// Give a fresh paragraph the type `init` writes.
    fn retype(&self, id: &TreeID, init: &BlockInit) -> Result<(), doc::WriteError> {
        let kind = init.block_type();
        if kind == BlockType::Paragraph {
            return Ok(());
        }
        let tree = doc::blocks(&self.doc);
        let Some(node) = doc::node_of(&tree, id) else {
            return Err(doc::WriteError::Refused(
                doc::RefusalCause::TreeRefusedBlock,
            ));
        };
        node.insert(doc::NodeKey::Type, i64::from(kind.code()))?;
        self.write_init(id, init)
    }
}

/// The far corner of the merged cell anchored at `at`. Its column is the
/// last slot of the row that points left at it. Its row is the last slot of
/// the column that points up at it.
fn extent_of(table: &ClipTable, at: CellPos) -> CellPos {
    let slot = |row: usize, col: usize| table.rows.get(row).and_then(|cells| cells.cells.get(col));
    let (row, col) = (at.row.to_usize(), at.col.to_usize());
    let wide = (col + 1..)
        .take_while(|c| slot(row, *c) == Some(&ClipCell::Covered(ClipCover::Left)))
        .count();
    let deep = (row + 1..)
        .take_while(|r| slot(*r, col) == Some(&ClipCell::Covered(ClipCover::Above)))
        .count();
    CellPos::new(
        Pos::<Row>::from_usize(row + deep),
        Pos::<Col>::from_usize(col + wide),
    )
}
