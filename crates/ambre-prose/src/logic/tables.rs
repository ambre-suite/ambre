//! Tables: the grid, the merged regions, and the commands that change them
//! (`features/ambre-spec-prose-tables.md`).
//!
//! A table, a row and a cell are three block types of the tree. So a cell's
//! identity survives every insert, delete and move around it. A merged region
//! is stored as its two corner cells and a stamp, never as a span
//! (`PROSE-009`). The rectangle it covers is worked out here, from the grid the
//! table has now, every time it is read.
//!
//! **Who decides which region survives.** [`surviving_regions`] reads the
//! region records and the grid of live cells, and nothing else
//! (`PROSE-FEAT-229`). Every peer holds both once the documents converge, so
//! every peer keeps the same regions without exchanging anything more.

use ambre_core::{Pt, Rgb, StepDirection};
use loro::{LoroDoc, LoroTree, LoroValue, TreeID, TreeParentId};

use crate::doc::{self, Region};
use crate::events::grid::{CellPos, CellRect, Col, ColCount, Row, RowCount};
use crate::events::{
    BlockType, CellBorder, ProseEvent, ProseOutcome, ProseRefusal, TableValue, VAlign, block_id,
};

use super::ProseState;

/// A table as it stands: its rows in order, and each row's cells in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub table: TreeID,
    pub rows: Vec<TreeID>,
    pub cells: Vec<Vec<TreeID>>,
}

impl Grid {
    /// The grid of a live table, or `None` when the id names no table.
    pub(crate) fn read(tree: &LoroTree, table: &TreeID) -> Option<Self> {
        if doc::block_type_of(tree, table) != Some(BlockType::Table) {
            return None;
        }
        let rows = doc::children_of(tree, Some(*table));
        let cells = rows
            .iter()
            .map(|row| doc::children_of(tree, Some(*row)))
            .collect();
        Some(Self {
            table: *table,
            rows,
            cells,
        })
    }

    /// The row and column a cell sits at.
    pub(crate) fn position_of(&self, cell: &TreeID) -> Option<CellPos> {
        self.cells.iter().enumerate().find_map(|(row, cells)| {
            cells
                .iter()
                .position(|candidate| candidate == cell)
                .map(|col| CellPos::new(Row::from_usize(row), Col::from_usize(col)))
        })
    }

    /// The cell at a grid position, when its row holds one there.
    pub(crate) fn cell(&self, at: CellPos) -> Option<TreeID> {
        self.cells
            .get(at.row.to_usize())?
            .get(at.col.to_usize())
            .copied()
    }

    /// How many rows the grid holds.
    pub(crate) fn row_count(&self) -> RowCount {
        RowCount::from_len(self.rows.len())
    }

    /// The grid position of a cell, then the cell itself, in reading order.
    fn reading_order(&self) -> impl Iterator<Item = (CellPos, TreeID)> + '_ {
        self.cells.iter().enumerate().flat_map(|(row, cells)| {
            cells.iter().enumerate().map(move |(col, cell)| {
                (
                    CellPos::new(Row::from_usize(row), Col::from_usize(col)),
                    *cell,
                )
            })
        })
    }
}

/// The rectangle a region covers in the grid the table has now.
///
/// `None` when either corner no longer resolves to a live cell of this table
/// (`PROSE-009`, `PROSE-FEAT-253`).
pub(crate) fn rect_of(grid: &Grid, region: &Region) -> Option<CellRect> {
    let anchor = grid.position_of(&region.anchor)?;
    let extent = grid.position_of(&region.extent)?;
    Some(CellRect::spanning(anchor, extent))
}

/// The regions that stand, each with its rectangle, and the ones to drop.
///
/// A region whose corners no longer resolve is dropped (`PROSE-FEAT-253`). Of
/// two regions covering one cell the later stands (`PROSE-FEAT-254`). The
/// later is the greater stamp, then the greater anchor, so every peer breaks a
/// tie alike. The answer reads the records and the grid alone
/// (`PROSE-FEAT-229`).
pub(crate) fn surviving_regions(
    grid: &Grid,
    regions: &[Region],
) -> (Vec<(Region, CellRect)>, Vec<Region>) {
    let mut ordered: Vec<&Region> = regions.iter().collect();
    ordered.sort_by_key(|region| std::cmp::Reverse((region.stamp, region.anchor)));
    let mut kept: Vec<(Region, CellRect)> = Vec::new();
    let mut dropped = Vec::new();
    for region in ordered {
        match rect_of(grid, region) {
            Some(rect) if !kept.iter().any(|(_, other)| other.overlaps(rect)) => {
                kept.push((*region, rect));
            }
            _ => dropped.push(*region),
        }
    }
    kept.sort_by_key(|(region, _)| region.anchor);
    (kept, dropped)
}

/// The Lamport time the next operation of this document will carry.
///
/// A region written after another one was seen carries a greater stamp than
/// it, which is what "the later of the two" means (`PROSE-FEAT-254`). Two
/// concurrent writes may share a stamp, and the anchor breaks the tie.
fn next_stamp(document: &LoroDoc) -> ambre_core::LamportStamp {
    let mut next = 0_i64;
    for id in document.oplog_frontiers().iter() {
        let Some(change) = document.get_change(id) else {
            continue;
        };
        let offset = i64::from(id.counter) - i64::from(change.id.counter);
        next = next.max(i64::from(change.lamport) + offset + 1);
    }
    ambre_core::LamportStamp::from_i64(next)
}

/// Whether any text block under `block` holds a character.
fn holds_text(document: &LoroDoc, tree: &LoroTree, block: &TreeID) -> bool {
    doc::children_of(tree, Some(*block)).iter().any(|child| {
        let has_body = doc::block_type_of(tree, child).is_some_and(|t| t.has_body());
        (has_body && !doc::get_block_text(document, child).is_empty())
            || holds_text(document, tree, child)
    })
}

/// The first text block of a cell, where a caret entering it lands.
fn first_text_block(tree: &LoroTree, block: &TreeID) -> Option<TreeID> {
    doc::children_of(tree, Some(*block))
        .into_iter()
        .find_map(|child| {
            if doc::block_type_of(tree, &child).is_some_and(|t| t.has_body()) {
                Some(child)
            } else {
                first_text_block(tree, &child)
            }
        })
}

/// A width or a padding the format holds: finite, and not negative. A column
/// width must also be above zero.
fn usable(value: Pt, floor: Floor) -> bool {
    value.is_finite() && (value > Pt::ZERO || (floor == Floor::ZeroAllowed && value == Pt::ZERO))
}

/// The least a width or a padding may be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Floor {
    /// A column width: above zero.
    AboveZero,
    /// A border or a padding: zero or more.
    ZeroAllowed,
}

/// The reads a test takes of a table. The layout reads the grid and the
/// regions itself, through [`Grid`] and [`surviving_regions`].
#[cfg(test)]
impl ProseState {
    /// The grid of a live table.
    #[must_use]
    pub(crate) fn grid_of(&self, table: &TreeID) -> Option<Grid> {
        Grid::read(&doc::blocks(&self.doc), table)
    }

    /// The column widths a table carries, in points.
    #[must_use]
    pub(crate) fn column_widths(&self, table: &TreeID) -> Vec<Pt> {
        doc::col_widths(&doc::blocks(&self.doc), table)
    }

    /// The regions a table holds, as the document stores them.
    #[must_use]
    pub(crate) fn regions_of(&self, table: &TreeID) -> Vec<Region> {
        doc::regions(&doc::blocks(&self.doc), table)
    }

    /// The regions that stand, with the rectangle each covers today.
    #[must_use]
    pub(crate) fn merged_rects(&self, table: &TreeID) -> Vec<(Region, CellRect)> {
        let tree = doc::blocks(&self.doc);
        let Some(grid) = Grid::read(&tree, table) else {
            return Vec::new();
        };
        surviving_regions(&grid, &doc::regions(&tree, table)).0
    }
}

impl ProseState {
    /// The table a row or a cell sits in.
    fn table_of(tree: &LoroTree, block: &TreeID) -> Option<TreeID> {
        let mut current = *block;
        loop {
            if doc::block_type_of(tree, &current) == Some(BlockType::Table) {
                return Some(current);
            }
            match tree.parent(current) {
                Some(TreeParentId::Node(parent)) => current = parent,
                _ => return None,
            }
        }
    }

    /// Mint one cell at the end of a row, holding one empty paragraph.
    fn mint_cell(&self, row: &TreeID) -> Option<TreeID> {
        let cell = doc::create_block(&self.doc, Some(*row), BlockType::TableCell)?;
        doc::create_block(&self.doc, Some(cell), BlockType::Paragraph)?;
        Some(cell)
    }

    /// Mint one row at the end of a table, holding `cols` cells.
    fn mint_row(&self, table: &TreeID, cols: ColCount) -> Option<TreeID> {
        let row = doc::create_block(&self.doc, Some(*table), BlockType::TableRow)?;
        for _ in 0..cols.get() {
            self.mint_cell(&row)?;
        }
        Some(row)
    }

    /// The attribute map of a live table, minted when it has none.
    fn table_attrs(&self, table: &TreeID) -> Option<doc::Fields<doc::AttrKey>> {
        let tree = doc::blocks(&self.doc);
        doc::attrs_at(&doc::node_of(&tree, table)?)
    }

    /// Drop one region record.
    fn drop_region(&self, table: &TreeID, anchor: &TreeID) -> Result<(), doc::WriteError> {
        let Some(map) = self
            .table_attrs(table)
            .and_then(|attrs| doc::map_at(&attrs, doc::AttrKey::Merged))
        else {
            return Err(doc::WriteError::Refused(
                doc::RefusalCause::RegionMapRefused,
            ));
        };
        map.delete(&anchor.to_string())
            .map_err(doc::WriteError::from)
    }

    /// Drop every region with a corner in the given positions
    /// (`PROSE-FEAT-049`). A region the positions merely cross keeps both
    /// corners (`PROSE-FEAT-237`).
    fn drop_regions_cornered_in(
        &self,
        grid: &Grid,
        doomed: impl Fn(CellPos) -> bool,
    ) -> Result<(), doc::WriteError> {
        let tree = doc::blocks(&self.doc);
        for region in doc::regions(&tree, &grid.table) {
            let cornered = [region.anchor, region.extent]
                .iter()
                .any(|corner| grid.position_of(corner).is_some_and(&doomed));
            if cornered {
                self.drop_region(&grid.table, &region.anchor)?;
            }
        }
        Ok(())
    }

    // -- the commands --------------------------------------------------------

    /// A table of `rows` by `cols`, every cell holding one empty paragraph and
    /// every column at the default width.
    pub(super) fn insert_table(
        &mut self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        rows: RowCount,
        cols: ColCount,
    ) -> ProseOutcome {
        if rows.is_zero() || cols.is_zero() {
            // A table of no column paints nothing and accepts no text
            // (`PROSE-FEAT-228`).
            return Self::err_table(TableValue::Size { rows, cols });
        }
        match self.mint_table(parent_id, after_id, rows, cols) {
            Ok(_) => self.commit_with(Vec::new()),
            Err(refused) => refused,
        }
    }

    /// Mint a table of `rows` by `cols` without committing it, for a caller
    /// that writes more in the same commit.
    pub(super) fn mint_table(
        &self,
        parent_id: Option<TreeID>,
        after_id: Option<TreeID>,
        rows: RowCount,
        cols: ColCount,
    ) -> Result<TreeID, ProseOutcome> {
        let table = self.mint_placed(parent_id, after_id, BlockType::Table)?;
        let Some(attrs) = self.table_attrs(&table) else {
            return Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::AttrMapRefused,
            )));
        };
        let Some(widths) = doc::movable_list_at(&attrs, doc::AttrKey::ColWidths) else {
            return Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::WidthListRefused,
            )));
        };
        for _ in 0..cols.get() {
            if let Err(e) = widths.push(f64::from(doc::DEFAULT_COLUMN_WIDTH.get())) {
                return Err(Self::err_write(e));
            }
        }
        if doc::map_at(&attrs, doc::AttrKey::Merged).is_none() {
            return Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::RegionMapRefused,
            )));
        }
        for _ in 0..rows.get() {
            if self.mint_row(&table, cols).is_none() {
                return Err(Self::err_write(doc::WriteError::Refused(
                    doc::RefusalCause::TreeRefusedRow,
                )));
            }
        }
        Ok(table)
    }

    /// A row of one cell per column, after `after_row` or first
    /// (`PROSE-FEAT-227`).
    pub(super) fn insert_table_row(
        &mut self,
        table: &TreeID,
        after_row: Option<TreeID>,
    ) -> ProseOutcome {
        match self.add_row(table, after_row) {
            Ok(_) => self.commit_with(Vec::new()),
            Err(refused) => refused,
        }
    }

    /// The write half of a row insert, answering the new row.
    fn add_row(&self, table: &TreeID, after_row: Option<TreeID>) -> Result<TreeID, ProseOutcome> {
        let tree = doc::blocks(&self.doc);
        let Some(grid) = Grid::read(&tree, table) else {
            return Err(Self::err_no_block(table));
        };
        if let Some(after) = after_row
            && !grid.rows.contains(&after)
        {
            return Err(Self::err_no_block(&after));
        }
        let cols = ColCount::from_len(doc::col_widths(&tree, table).len());
        let Some(row) = self.mint_row(table, cols) else {
            return Err(Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::TreeRefusedRow,
            )));
        };
        let moved = match after_row {
            Some(after) => tree.mov_after(row, after),
            None => tree.mov_to(row, *table, 0),
        };
        moved.map_err(Self::err_write)?;
        Ok(row)
    }

    /// Remove a row, after dropping every region cornered in it
    /// (`PROSE-FEAT-049`). The last row takes the table with it
    /// (`PROSE-FEAT-293`).
    pub(super) fn delete_table_row(&mut self, row: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, row) != Some(BlockType::TableRow) {
            return Self::err_no_block(row);
        }
        let Some(grid) = Self::table_of(&tree, row).and_then(|t| Grid::read(&tree, &t)) else {
            return Self::err_no_block(row);
        };
        let Some(index) = grid.rows.iter().position(|r| r == row).map(Row::from_usize) else {
            return Self::err_no_block(row);
        };
        let doomed = if grid.rows.len() == 1 {
            grid.table
        } else {
            *row
        };
        if let Err(e) = self.drop_regions_cornered_in(&grid, |at| at.row == index) {
            return Self::err_write(e);
        }
        if let Err(e) = doc::hide_block(&self.doc, &tree, &doomed) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// A cell in every row at `at_col`, and one width entry beside them, in
    /// the same operation (`PROSE-FEAT-231`).
    pub(super) fn insert_table_col(
        &mut self,
        table: &TreeID,
        at_col: Col,
        width: Option<Pt>,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(grid) = Grid::read(&tree, table) else {
            return Self::err_no_block(table);
        };
        let width = width.unwrap_or(doc::DEFAULT_COLUMN_WIDTH);
        if !usable(width, Floor::AboveZero) {
            return Self::err_table(TableValue::Width(width));
        }
        let Some(widths) = doc::col_widths_read(&tree, table) else {
            return Self::err_write(doc::WriteError::Refused(doc::RefusalCause::NoWidthList));
        };
        let at = at_col.to_usize();
        if at > widths.len() {
            return ProseOutcome::refused(ProseRefusal::OutsideTheGrid);
        }
        for row in &grid.rows {
            let Some(cell) = self.mint_cell(row) else {
                return Self::err_write(doc::WriteError::Refused(
                    doc::RefusalCause::TreeRefusedCell,
                ));
            };
            // A row a concurrent edit left short takes the cell at its end.
            let index = at.min(doc::children_of(&tree, Some(*row)).len().saturating_sub(1));
            if let Err(e) = tree.mov_to(cell, *row, index) {
                return Self::err_write(e);
            }
        }
        if let Err(e) = widths.insert(at, f64::from(width.get())) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Remove the cell at `col_index` in every row and the matching width
    /// entry (`PROSE-FEAT-235`). The last column takes the table with it
    /// (`PROSE-FEAT-045`).
    pub(super) fn delete_table_col(&mut self, table: &TreeID, col: Col) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(grid) = Grid::read(&tree, table) else {
            return Self::err_no_block(table);
        };
        let Some(widths) = doc::col_widths_read(&tree, table) else {
            return Self::err_write(doc::WriteError::Refused(doc::RefusalCause::NoWidthList));
        };
        if col.to_usize() >= widths.len() {
            return ProseOutcome::refused(ProseRefusal::OutsideTheGrid);
        }
        if widths.len() == 1 {
            if let Err(e) = doc::hide_block(&self.doc, &tree, table) {
                return Self::err_write(e);
            }
            return self.commit_with(Vec::new());
        }
        if let Err(e) = self.drop_regions_cornered_in(&grid, |at| at.col == col) {
            return Self::err_write(e);
        }
        for row in (0..grid.rows.len()).map(Row::from_usize) {
            if let Some(cell) = grid.cell(CellPos::new(row, col))
                && let Err(e) = doc::hide_block(&self.doc, &tree, &cell)
            {
                return Self::err_write(e);
            }
        }
        if let Err(e) = widths.delete(col.to_usize(), 1) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Rewrite one width entry, and never the count (`PROSE-FEAT-239`).
    pub(super) fn resize_table_col(&mut self, table: &TreeID, col: Col, width: Pt) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if Grid::read(&tree, table).is_none() {
            return Self::err_no_block(table);
        }
        if !usable(width, Floor::AboveZero) {
            return Self::err_table(TableValue::Width(width));
        }
        let Some(widths) = doc::col_widths_read(&tree, table) else {
            return Self::err_write(doc::WriteError::Refused(doc::RefusalCause::NoWidthList));
        };
        if col.to_usize() >= widths.len() {
            return ProseOutcome::refused(ProseRefusal::OutsideTheGrid);
        }
        if let Err(e) = widths.set(col.to_usize(), f64::from(width.get())) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Merge the rectangle between two grid positions into one region.
    ///
    /// The positions are resolved to two block ids before anything is written
    /// (`PROSE-FEAT-241`). A shape that is not a solid rectangle is refused
    /// (`PROSE-FEAT-047`), and so is a covered cell that holds text
    /// (`PROSE-FEAT-046`). Every region the rectangle meets is removed, then
    /// one entry is written (`PROSE-FEAT-242`, `PROSE-FEAT-243`).
    pub(super) fn merge_table_cells(
        &mut self,
        table: &TreeID,
        start: CellPos,
        end: CellPos,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(grid) = Grid::read(&tree, table) else {
            return Self::err_no_block(table);
        };
        let rect = CellRect::spanning(start, end);
        let cols = ColCount::from_len(doc::col_widths(&tree, table).len());
        if rect.bottom_right.row >= grid.row_count().as_end()
            || rect.bottom_right.col >= cols.as_end()
        {
            return ProseOutcome::refused(ProseRefusal::OutsideTheGrid);
        }
        if rect.is_single_cell() {
            // One cell is already one cell (`PROSE-FEAT-244`).
            return ProseOutcome::nothing();
        }
        let mut covered = Vec::new();
        for position in rect.cells() {
            let Some(cell) = grid.cell(position) else {
                return ProseOutcome::refused(ProseRefusal::MergeNotRectangular);
            };
            covered.push(cell);
        }
        let (Some(anchor), Some(extent)) = (grid.cell(rect.top_left), grid.cell(rect.bottom_right))
        else {
            return ProseOutcome::refused(ProseRefusal::MergeNotRectangular);
        };
        if covered
            .iter()
            .filter(|cell| **cell != anchor)
            .any(|cell| holds_text(&self.doc, &tree, cell))
        {
            return ProseOutcome::refused(ProseRefusal::MergeOverText);
        }
        let stamp = next_stamp(&self.doc);
        let (standing, _) = surviving_regions(&grid, &doc::regions(&tree, table));
        for (region, other) in standing {
            if other.overlaps(rect)
                && let Err(e) = self.drop_region(table, &region.anchor)
            {
                return Self::err_write(e);
            }
        }
        let Some(map) = self
            .table_attrs(table)
            .and_then(|attrs| doc::map_at(&attrs, doc::AttrKey::Merged))
        else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::RegionMapRefused,
            ));
        };
        let record = LoroValue::Map(
            [
                (
                    doc::RegionKey::Extent.as_str().to_owned(),
                    LoroValue::from(extent.to_string()),
                ),
                (
                    doc::RegionKey::Stamp.as_str().to_owned(),
                    LoroValue::I64(stamp.to_i64_saturating()),
                ),
            ]
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>()
            .into(),
        );
        if let Err(e) = map.insert(&anchor.to_string(), record) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Remove the region `cell` anchors, or the one covering it
    /// (`PROSE-FEAT-245`). A cell in no region writes nothing
    /// (`PROSE-FEAT-247`).
    pub(super) fn unmerge_table_cell(&mut self, cell: &TreeID) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, cell) != Some(BlockType::TableCell) {
            return Self::err_no_block(cell);
        }
        let Some(grid) = Self::table_of(&tree, cell).and_then(|t| Grid::read(&tree, &t)) else {
            return Self::err_no_block(cell);
        };
        let Some(position) = grid.position_of(cell) else {
            return Self::err_no_block(cell);
        };
        let (standing, _) = surviving_regions(&grid, &doc::regions(&tree, &grid.table));
        let Some((region, _)) = standing.iter().find(|(_, rect)| rect.contains(position)) else {
            return ProseOutcome::nothing();
        };
        if let Err(e) = self.drop_region(&grid.table, &region.anchor) {
            return Self::err_write(e);
        }
        self.commit_with(Vec::new())
    }

    /// Write the cell properties the command named on that cell alone
    /// (`PROSE-FEAT-248`).
    pub(super) fn set_cell_style(
        &mut self,
        cell: &TreeID,
        bg: Option<Rgb>,
        valign: Option<VAlign>,
        border: Option<CellBorder>,
        padding: Option<Pt>,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        if doc::block_type_of(&tree, cell) != Some(BlockType::TableCell) {
            return Self::err_no_block(cell);
        }
        let mut writes: Vec<(doc::AttrKey, LoroValue)> = Vec::new();
        if let Some(bg) = bg {
            writes.push((doc::AttrKey::CellBg, bg.to_string().into()));
        }
        if let Some(valign) = valign {
            writes.push((doc::AttrKey::VAlign, i64::from(valign.code()).into()));
        }
        if let Some(border) = border {
            let width = border.width;
            if !usable(width, Floor::ZeroAllowed) {
                return Self::err_table(TableValue::Border(width));
            }
            for key in [
                doc::AttrKey::EdgeTop,
                doc::AttrKey::EdgeBottom,
                doc::AttrKey::EdgeLeft,
                doc::AttrKey::EdgeRight,
            ] {
                writes.push((key, f64::from(width.get()).into()));
            }
            writes.push((doc::AttrKey::EdgeColor, border.color.to_string().into()));
        }
        if let Some(padding) = padding {
            if !usable(padding, Floor::ZeroAllowed) {
                return Self::err_table(TableValue::Padding(padding));
            }
            writes.push((doc::AttrKey::Padding, f64::from(padding.get()).into()));
        }
        if writes.is_empty() {
            return ProseOutcome::nothing();
        }
        let Some(attrs) = doc::node_of(&tree, cell).and_then(|n| doc::attrs_at(&n)) else {
            return Self::err_write(doc::WriteError::Refused(doc::RefusalCause::AttrMapRefused));
        };
        for (key, value) in writes {
            if let Err(e) = attrs.insert(key, value) {
                return Self::err_write(e);
            }
        }
        self.commit_with(Vec::new())
    }

    /// Step to the next or the previous stop in reading order.
    ///
    /// A region is one stop, on its anchor (`PROSE-FEAT-250`). Stepping writes
    /// nothing (`PROSE-FEAT-051`), except past the last cell, where a row is
    /// appended and the caret lands in its first cell (`PROSE-FEAT-251`).
    pub(super) fn step_table_cell(
        &mut self,
        cell: &TreeID,
        direction: StepDirection,
    ) -> ProseOutcome {
        let tree = doc::blocks(&self.doc);
        let Some(table) = Self::table_of(&tree, cell) else {
            return Self::err_no_block(cell);
        };
        let Some(grid) = Grid::read(&tree, &table) else {
            return Self::err_no_block(cell);
        };
        let Some(position) = grid.position_of(cell) else {
            return Self::err_no_block(cell);
        };
        let (standing, _) = surviving_regions(&grid, &doc::regions(&tree, &table));
        let stop_of = |position: CellPos, cell: TreeID| match standing
            .iter()
            .find(|(_, rect)| rect.contains(position))
        {
            Some((region, _)) => region.anchor,
            None => cell,
        };
        let mut stops: Vec<TreeID> = Vec::new();
        for (position, cell) in grid.reading_order() {
            let stop = stop_of(position, cell);
            if !stops.contains(&stop) {
                stops.push(stop);
            }
        }
        let here = stop_of(position, *cell);
        let Some(index) = stops.iter().position(|stop| *stop == here) else {
            return Self::err_no_block(cell);
        };
        let next = match direction {
            StepDirection::Backward => stops.get(index.saturating_sub(1)).copied(),
            StepDirection::Forward => stops.get(index + 1).copied(),
        };
        if let Some(next) = next {
            return ProseOutcome::display_only(vec![Self::entered(&tree, next)]);
        }
        let last_row = grid.rows.last().copied();
        let row = match self.add_row(&table, last_row) {
            Ok(row) => row,
            Err(refused) => return refused,
        };
        let tree = doc::blocks(&self.doc);
        let Some(first) = doc::children_of(&tree, Some(row)).first().copied() else {
            return Self::err_write(doc::WriteError::Refused(
                doc::RefusalCause::AppendedRowEmpty,
            ));
        };
        let entered = Self::entered(&tree, first);
        self.commit_with(vec![entered])
    }

    /// The event that puts the caret at the start of a cell.
    fn entered(tree: &LoroTree, cell: TreeID) -> ProseEvent {
        let block = first_text_block(tree, &cell).unwrap_or(cell);
        ProseEvent::CellEntered {
            cell: block_id(cell),
            block: block_id(block),
        }
    }

    /// Settle every table after a remote change.
    ///
    /// Each region whose corners no longer resolve is dropped, and of two
    /// regions covering one cell the later stands (`PROSE-FEAT-253`,
    /// `PROSE-FEAT-254`). A covered cell that still holds text is reported and
    /// kept (`PROSE-FEAT-255`). The pass writes region records alone, and the
    /// change that arrived is applied whatever it finds (`PROSE-FEAT-256`).
    pub(super) fn validate_tables(&mut self) -> Vec<ProseEvent> {
        let tree = doc::blocks(&self.doc);
        let mut events = Vec::new();
        for table in doc::roots(&tree) {
            let Some(grid) = Grid::read(&tree, &table) else {
                continue;
            };
            let (standing, dropped) = surviving_regions(&grid, &doc::regions(&tree, &table));
            for region in dropped {
                if let Err(e) = self.drop_region(&table, &region.anchor) {
                    tracing::warn!("Prose: a stale region could not be dropped: {e}");
                }
            }
            for (region, rect) in standing {
                for position in rect.cells() {
                    let Some(cell) = grid.cell(position) else {
                        continue;
                    };
                    if cell != region.anchor && holds_text(&self.doc, &tree, &cell) {
                        events.push(ProseEvent::MergeConflictDetected {
                            cell_id: block_id(cell),
                        });
                    }
                }
            }
        }
        events
    }

    /// The refusal of a dimension, a width or a colour the format cannot hold.
    fn err_table(what: TableValue) -> ProseOutcome {
        ProseOutcome::refused(ProseRefusal::BadTableValue(what))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::events::{ProseCommand, tree_id};
    use ambre_bus::app::prose::DocOffset;

    /// The first table at the root of the document.
    pub(crate) fn the_table(state: &ProseState) -> TreeID {
        let tree = doc::blocks(state.document());
        doc::roots(&tree)
            .into_iter()
            .find(|id| doc::block_type_of(&tree, id) == Some(BlockType::Table))
            .expect("the document holds a table")
    }

    /// The cell at a row and a column.
    pub(crate) fn cell_at(state: &ProseState, table: TreeID, row: usize, col: usize) -> TreeID {
        state
            .grid_of(&table)
            .and_then(|grid| grid.cell(CellPos::new(Row::from_usize(row), Col::from_usize(col))))
            .expect("the grid holds that cell")
    }

    /// The first text block of a cell.
    pub(crate) fn first_block_of(state: &ProseState, cell: TreeID) -> TreeID {
        first_text_block(&doc::blocks(state.document()), &cell).expect("a cell holds a paragraph")
    }

    /// A document holding one table of `rows` by `cols` after its paragraph.
    pub(crate) fn document_with_table(
        peer: ambre_core::PeerId,
        rows: u32,
        cols: u32,
    ) -> (ProseState, TreeID) {
        let mut state = ProseState::with_peer(peer).expect("a document is built");
        let root = state.root_blocks()[0];
        state.handle_command(ProseCommand::InsertTable {
            parent_id: None,
            after_id: Some(root),
            rows: RowCount::new(rows),
            cols: ColCount::new(cols),
        });
        let table = the_table(&state);
        (state, table)
    }

    /// Type into the paragraph of one cell.
    pub(crate) fn type_in(
        state: &mut ProseState,
        cell: TreeID,
        text: &str,
    ) -> Option<ambre_core::DeltaBytes> {
        let block = first_block_of(state, cell);
        state
            .handle_command(ProseCommand::InsertText {
                block_id: block_id(block),
                pos: DocOffset::ZERO,
                text: text.to_owned(),
            })
            .update
    }

    /// The text of every cell, row by row.
    pub(crate) fn texts(state: &ProseState, table: TreeID) -> Vec<Vec<String>> {
        let grid = state.grid_of(&table).expect("a table");
        grid.cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| {
                        let block = first_block_of(state, *cell);
                        state.text_of(&block_id(block)).unwrap_or_default()
                    })
                    .collect()
            })
            .collect()
    }

    fn row_lengths(state: &ProseState, table: TreeID) -> Vec<usize> {
        state
            .grid_of(&table)
            .expect("a table")
            .cells
            .iter()
            .map(Vec::len)
            .collect()
    }

    fn refusal(outcome: &ProseOutcome) -> Option<ProseRefusal> {
        outcome.display.iter().find_map(|event| match event {
            ProseEvent::Error(error) => Some(error.reason.clone()),
            _ => None,
        })
    }

    fn merge(
        state: &mut ProseState,
        table: TreeID,
        start: (u32, u32),
        end: (u32, u32),
    ) -> ProseOutcome {
        state.handle_command(ProseCommand::MergeTableCells {
            table_id: block_id(table),
            start: pos(start),
            end: pos(end),
        })
    }

    /// The grid position at a row and a column.
    pub(crate) fn pos((row, col): (u32, u32)) -> CellPos {
        CellPos::new(Row::new(row), Col::new(col))
    }

    // -- the state -------------------------------------------------------

    #[test]
    fn a_new_table_holds_one_cell_per_column_in_every_row() {
        let (state, table) = document_with_table(ambre_core::PeerId(1), 3, 4);
        assert_eq!(row_lengths(&state, table), vec![4, 4, 4]);
        assert_eq!(state.column_widths(&table).len(), 4);
    }

    #[test]
    fn every_table_command_leaves_every_row_one_cell_per_column() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let first_row = state.grid_of(&table).expect("a table").rows[0];
        for command in [
            ProseCommand::InsertTableRow {
                table_id: block_id(table),
                after_row: Some(block_id(first_row)),
            },
            ProseCommand::InsertTableCol {
                table_id: block_id(table),
                at_col: Col::new(1),
                width: None,
            },
            ProseCommand::DeleteTableCol {
                table_id: block_id(table),
                col_index: Col::new(0),
            },
            ProseCommand::DeleteTableRow {
                row_id: block_id(first_row),
            },
        ] {
            state.handle_command(command);
            let widths = state.column_widths(&table).len();
            assert!(row_lengths(&state, table).iter().all(|len| *len == widths));
        }
    }

    #[test]
    fn a_table_of_no_row_or_no_column_is_refused() {
        let mut state = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        for (rows, cols) in [(0, 3), (3, 0)] {
            let outcome = state.handle_command(ProseCommand::InsertTable {
                parent_id: None,
                after_id: None,
                rows: RowCount::new(rows),
                cols: ColCount::new(cols),
            });
            assert_eq!(
                refusal(&outcome),
                Some(ProseRefusal::BadTableValue(TableValue::Size {
                    rows: RowCount::new(rows),
                    cols: ColCount::new(cols),
                }))
            );
            assert!(outcome.update.is_none());
        }
    }

    #[test]
    fn a_region_is_stored_as_two_corners_and_a_stamp_alone() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        merge(&mut state, table, (0, 0), (1, 1));
        let tree = doc::blocks(state.document());
        let map = doc::merged_read(&tree, &table).expect("a region map");
        let LoroValue::Map(entries) = map.get_value() else {
            panic!("the region map reads as a map");
        };
        assert_eq!(entries.len(), 1);
        let (anchor, record) = entries.iter().next().expect("one region");
        assert_eq!(anchor, &cell_at(&state, table, 0, 0).to_string());
        let LoroValue::Map(record) = record else {
            panic!("a region is a record");
        };
        let mut keys: Vec<&str> = record.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                doc::RegionKey::Extent.as_str(),
                doc::RegionKey::Stamp.as_str()
            ]
        );
    }

    #[test]
    fn the_regions_that_stand_are_decided_by_the_records_and_the_grid() {
        // Two records, the same grid: the answer is a function of the two, and
        // no text of any cell moves it.
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        merge(&mut state, table, (0, 0), (1, 1));
        let before = state.merged_rects(&table);
        let cell = cell_at(&state, table, 1, 1);
        let tree = doc::blocks(state.document());
        let paragraph = first_text_block(&tree, &cell).expect("a paragraph");
        doc::get_block_text(state.document(), &paragraph)
            .insert(0, "masked")
            .expect("a write");
        state.document().commit();
        assert_eq!(state.merged_rects(&table), before);
    }

    // -- insert a row or a column -----------------------------------------

    #[test]
    fn a_column_insert_splices_one_width_beside_the_cells() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
            width: ambre_core::Pt::new(100.0),
        });
        let first = cell_at(&state, table, 0, 0);
        type_in(&mut state, first, "kept");
        state.handle_command(ProseCommand::InsertTableCol {
            table_id: block_id(table),
            at_col: Col::new(1),
            width: Some(ambre_core::Pt::new(50.0)),
        });
        assert_eq!(
            state.column_widths(&table),
            vec![
                doc::DEFAULT_COLUMN_WIDTH,
                ambre_core::Pt::new(50.0),
                ambre_core::Pt::new(100.0)
            ]
        );
        assert_eq!(row_lengths(&state, table), vec![3, 3]);
        assert_eq!(texts(&state, table)[0], vec!["kept", "", ""]);
    }

    #[test]
    fn a_column_insert_carrying_no_width_takes_the_default() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 1, 1);
        state.handle_command(ProseCommand::InsertTableCol {
            table_id: block_id(table),
            at_col: Col::new(0),
            width: None,
        });
        assert_eq!(
            state.column_widths(&table),
            vec![doc::DEFAULT_COLUMN_WIDTH; 2]
        );
    }

    #[test]
    fn a_column_insert_past_the_grid_is_refused_and_reported() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let outcome = state.handle_command(ProseCommand::InsertTableCol {
            table_id: block_id(table),
            at_col: Col::new(3),
            width: None,
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::OutsideTheGrid));
        assert!(outcome.update.is_none());
        assert_eq!(row_lengths(&state, table), vec![2, 2]);
    }

    #[test]
    fn an_insert_leaves_widths_texts_and_corners_alone() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        merge(&mut state, table, (0, 0), (2, 1));
        let corner = cell_at(&state, table, 2, 2);
        type_in(&mut state, corner, "text");
        let regions = state.regions_of(&table);
        let widths = state.column_widths(&table);
        let second_row = state.grid_of(&table).expect("a table").rows[1];
        state.handle_command(ProseCommand::InsertTableRow {
            table_id: block_id(table),
            after_row: Some(block_id(second_row)),
        });
        assert_eq!(state.regions_of(&table), regions);
        assert_eq!(state.column_widths(&table), widths);
        assert_eq!(texts(&state, table)[3][2], "text");
        state.handle_command(ProseCommand::InsertTableCol {
            table_id: block_id(table),
            at_col: Col::new(1),
            width: Some(ambre_core::Pt::new(10.0)),
        });
        assert_eq!(state.regions_of(&table), regions);
        assert_eq!(texts(&state, table)[3][3], "text");
        // The region grew by what fell between its corners (`PROSE-FEAT-230`
        // on one peer).
        let (_, rect) = state.merged_rects(&table)[0];
        assert_eq!(rect.bottom_right, pos((3, 2)));
    }

    // -- delete a row or a column -----------------------------------------

    #[test]
    fn a_column_delete_removes_one_width_beside_the_cells() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 3);
        state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(2),
            width: ambre_core::Pt::new(30.0),
        });
        state.handle_command(ProseCommand::DeleteTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
        });
        assert_eq!(
            state.column_widths(&table),
            vec![doc::DEFAULT_COLUMN_WIDTH, ambre_core::Pt::new(30.0)]
        );
        assert_eq!(row_lengths(&state, table), vec![2, 2]);
    }

    #[test]
    fn deleting_a_corner_drops_the_region_and_frees_its_cells() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        merge(&mut state, table, (0, 1), (1, 2));
        let first_row = state.grid_of(&table).expect("a table").rows[0];
        state.handle_command(ProseCommand::DeleteTableRow {
            row_id: block_id(first_row),
        });
        assert!(state.regions_of(&table).is_empty());
        assert!(state.merged_rects(&table).is_empty());
        // Every cell is a stop of its own again.
        let cell = cell_at(&state, table, 0, 1);
        let outcome = state.handle_command(ProseCommand::StepTableCell {
            cell_id: block_id(cell),
            direction: StepDirection::Forward,
        });
        assert!(matches!(
            outcome.display.first(),
            Some(ProseEvent::CellEntered { cell: next, .. }) if *next == block_id(cell_at(&state, table, 0, 2))
        ));
    }

    #[test]
    fn a_column_delete_drops_a_region_cornered_in_it() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 3);
        merge(&mut state, table, (0, 0), (1, 1));
        state.handle_command(ProseCommand::DeleteTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
        });
        assert!(state.regions_of(&table).is_empty());
    }

    #[test]
    fn a_region_a_delete_crosses_keeps_its_corners_and_narrows() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        merge(&mut state, table, (0, 0), (2, 2));
        let regions = state.regions_of(&table);
        let middle = state.grid_of(&table).expect("a table").rows[1];
        state.handle_command(ProseCommand::DeleteTableRow {
            row_id: block_id(middle),
        });
        state.handle_command(ProseCommand::DeleteTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
        });
        assert_eq!(state.regions_of(&table), regions);
        let (_, rect) = state.merged_rects(&table)[0];
        assert_eq!(rect.bottom_right, pos((1, 1)));
    }

    #[test]
    fn deleting_the_last_column_deletes_the_table() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 1);
        state.handle_command(ProseCommand::DeleteTableCol {
            table_id: block_id(table),
            col_index: Col::new(0),
        });
        assert!(state.grid_of(&table).is_none());
        assert_eq!(state.root_blocks().len(), 1);
    }

    #[test]
    fn deleting_the_last_row_deletes_the_table() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 1, 2);
        let row = state.grid_of(&table).expect("a table").rows[0];
        state.handle_command(ProseCommand::DeleteTableRow {
            row_id: block_id(row),
        });
        assert!(state.grid_of(&table).is_none());
    }

    #[test]
    fn a_delete_leaves_surviving_widths_and_texts_alone() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 3);
        state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(0),
            width: ambre_core::Pt::new(20.0),
        });
        let kept = cell_at(&state, table, 1, 2);
        type_in(&mut state, kept, "kept");
        let first_row = state.grid_of(&table).expect("a table").rows[0];
        state.handle_command(ProseCommand::DeleteTableRow {
            row_id: block_id(first_row),
        });
        state.handle_command(ProseCommand::DeleteTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
        });
        assert_eq!(
            state.column_widths(&table),
            vec![ambre_core::Pt::new(20.0), doc::DEFAULT_COLUMN_WIDTH]
        );
        assert_eq!(
            texts(&state, table),
            vec![vec!["".to_owned(), "kept".to_owned()]]
        );
    }

    // -- resize a column ----------------------------------------------------

    #[test]
    fn a_resize_rewrites_one_width_and_nothing_else() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 3);
        let cell = cell_at(&state, table, 0, 1);
        type_in(&mut state, cell, "x");
        state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(1),
            width: ambre_core::Pt::new(144.0),
        });
        let d = doc::DEFAULT_COLUMN_WIDTH;
        assert_eq!(
            state.column_widths(&table),
            vec![d, ambre_core::Pt::new(144.0), d]
        );
        assert_eq!(texts(&state, table)[0][1], "x");
        let outcome = state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(3),
            width: ambre_core::Pt::new(10.0),
        });
        assert_eq!(refusal(&outcome), Some(ProseRefusal::OutsideTheGrid));
        assert_eq!(state.column_widths(&table).len(), 3);
    }

    // -- merge cells --------------------------------------------------------

    #[test]
    fn a_merge_over_a_ragged_grid_is_refused() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let cell = cell_at(&state, table, 1, 1);
        state.handle_command(ProseCommand::DeleteBlock {
            block_id: block_id(cell),
        });
        let outcome = merge(&mut state, table, (0, 0), (1, 1));
        assert_eq!(refusal(&outcome), Some(ProseRefusal::MergeNotRectangular));
        assert!(state.regions_of(&table).is_empty());
    }

    #[test]
    fn a_merge_over_a_cell_holding_text_is_refused_and_writes_nothing() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let covered = cell_at(&state, table, 1, 0);
        type_in(&mut state, covered, "in the way");
        let outcome = merge(&mut state, table, (0, 0), (1, 1));
        assert_eq!(refusal(&outcome), Some(ProseRefusal::MergeOverText));
        assert!(outcome.update.is_none());
        assert!(state.regions_of(&table).is_empty());
    }

    #[test]
    fn a_merge_resolves_its_positions_to_two_corner_ids() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        // The corners arrive in either order.
        merge(&mut state, table, (2, 2), (1, 1));
        let region = state.regions_of(&table)[0];
        assert_eq!(region.anchor, cell_at(&state, table, 1, 1));
        assert_eq!(region.extent, cell_at(&state, table, 2, 2));
    }

    #[test]
    fn a_merge_supersedes_the_regions_it_overlaps() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 3, 3);
        merge(&mut state, table, (0, 0), (1, 1));
        merge(&mut state, table, (2, 0), (2, 2));
        merge(&mut state, table, (1, 1), (2, 2));
        let regions = state.regions_of(&table);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].anchor, cell_at(&state, table, 1, 1));
    }

    #[test]
    fn a_merge_writes_one_entry_keyed_on_the_anchor() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        merge(&mut state, table, (0, 0), (0, 1));
        let regions = state.regions_of(&table);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].anchor, cell_at(&state, table, 0, 0));
        assert_eq!(regions[0].extent, cell_at(&state, table, 0, 1));
    }

    #[test]
    fn a_merge_of_one_cell_writes_nothing() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let outcome = merge(&mut state, table, (1, 1), (1, 1));
        assert!(outcome.update.is_none());
        assert!(outcome.display.is_empty());
    }

    // -- unmerge ------------------------------------------------------------

    #[test]
    fn an_unmerge_from_any_cell_of_a_region_removes_it() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        for cell in [(0, 0), (1, 1)] {
            merge(&mut state, table, (0, 0), (1, 1));
            let named = cell_at(&state, table, cell.0, cell.1);
            state.handle_command(ProseCommand::UnmergeTableCell {
                cell_id: block_id(named),
            });
            assert!(state.regions_of(&table).is_empty());
        }
    }

    #[test]
    fn an_unmerge_of_a_cell_in_no_region_writes_nothing() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let cell = cell_at(&state, table, 0, 0);
        let outcome = state.handle_command(ProseCommand::UnmergeTableCell {
            cell_id: block_id(cell),
        });
        assert!(outcome.update.is_none());
        assert!(outcome.display.is_empty());
    }

    // -- style a cell -------------------------------------------------------

    #[test]
    fn a_cell_style_lands_on_the_named_cell_alone() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 1, 2);
        let styled = cell_at(&state, table, 0, 0);
        let other = cell_at(&state, table, 0, 1);
        type_in(&mut state, styled, "text");
        let widths = state.column_widths(&table);
        let regions = state.regions_of(&table);
        state.handle_command(ProseCommand::SetCellStyle {
            cell_id: block_id(styled),
            bg: Some(Rgb::from_bits(0x00ff_8800)),
            valign: Some(VAlign::Middle),
            border: Some(CellBorder {
                width: ambre_core::Pt::new(1.5),
                color: Rgb::from_bits(0x0012_3456),
            }),
            padding: Some(ambre_core::Pt::new(4.0)),
        });
        let tree = doc::blocks(state.document());
        let attrs = doc::attrs_read(&tree, &styled).expect("the cell carries its style");
        assert_eq!(
            attrs
                .get(doc::AttrKey::CellBg)
                .and_then(|v| v.into_value().ok()),
            Some(LoroValue::from("#ff8800"))
        );
        assert_eq!(
            attrs
                .get(doc::AttrKey::EdgeColor)
                .and_then(|v| v.into_value().ok()),
            Some(LoroValue::from("#123456"))
        );
        assert!(doc::attrs_read(&tree, &other).is_none());
        assert_eq!(texts(&state, table)[0][0], "text");
        assert_eq!(state.column_widths(&table), widths);
        assert_eq!(state.regions_of(&table), regions);
    }

    #[test]
    fn a_bad_width_or_padding_is_refused() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 1, 1);
        let cell = cell_at(&state, table, 0, 0);
        let style = |padding| ProseCommand::SetCellStyle {
            cell_id: block_id(cell),
            bg: None,
            valign: None,
            border: None,
            padding,
        };
        let commands = [
            style(Some(ambre_core::Pt::new(-1.0))),
            style(Some(ambre_core::Pt::new(f32::NAN))),
            ProseCommand::ResizeTableCol {
                table_id: block_id(table),
                col_index: Col::new(0),
                width: ambre_core::Pt::new(f32::INFINITY),
            },
            ProseCommand::InsertTableCol {
                table_id: block_id(table),
                at_col: Col::new(0),
                width: Some(ambre_core::Pt::new(0.0)),
            },
        ];
        for command in commands {
            let outcome = state.handle_command(command);
            assert!(matches!(
                refusal(&outcome),
                Some(ProseRefusal::BadTableValue(_))
            ));
            assert!(outcome.update.is_none());
        }
    }

    // -- move the caret -------------------------------------------------------

    fn step(state: &mut ProseState, cell: TreeID, direction: StepDirection) -> ProseOutcome {
        state.handle_command(ProseCommand::StepTableCell {
            cell_id: block_id(cell),
            direction,
        })
    }

    fn entered(outcome: &ProseOutcome) -> TreeID {
        outcome
            .display
            .iter()
            .find_map(|event| match event {
                ProseEvent::CellEntered { cell, .. } => Some(tree_id(*cell)),
                _ => None,
            })
            .expect("the caret entered a cell")
    }

    #[test]
    fn traversal_lands_on_an_anchor_and_skips_what_it_covers() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 3);
        merge(&mut state, table, (0, 1), (1, 2));
        let first = cell_at(&state, table, 0, 0);
        let outcome = step(&mut state, first, StepDirection::Forward);
        let anchor = cell_at(&state, table, 0, 1);
        assert_eq!(entered(&outcome), anchor);
        // From the anchor, the next stop is the next row's first cell.
        let outcome = step(&mut state, anchor, StepDirection::Forward);
        assert_eq!(entered(&outcome), cell_at(&state, table, 1, 0));
        // And back again lands on the anchor, not on a covered cell.
        let from = cell_at(&state, table, 1, 0);
        let outcome = step(&mut state, from, StepDirection::Backward);
        assert_eq!(entered(&outcome), anchor);
    }

    #[test]
    fn stepping_between_cells_writes_nothing() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let first = cell_at(&state, table, 0, 0);
        let outcome = step(&mut state, first, StepDirection::Forward);
        assert!(outcome.update.is_none());
        let outcome = step(&mut state, first, StepDirection::Backward);
        assert!(outcome.update.is_none());
    }

    #[test]
    fn stepping_past_the_last_cell_appends_a_row_and_enters_it() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        let last = cell_at(&state, table, 1, 1);
        type_in(&mut state, last, "end");
        state.handle_command(ProseCommand::ResizeTableCol {
            table_id: block_id(table),
            col_index: Col::new(0),
            width: ambre_core::Pt::new(12.0),
        });
        merge(&mut state, table, (0, 0), (0, 1));
        let widths = state.column_widths(&table);
        let regions = state.regions_of(&table);
        let outcome = step(&mut state, last, StepDirection::Forward);
        assert!(outcome.update.is_some());
        assert_eq!(row_lengths(&state, table), vec![2, 2, 2]);
        let first_of_new = cell_at(&state, table, 2, 0);
        assert_eq!(entered(&outcome), first_of_new);
        let texts = texts(&state, table);
        assert_eq!(texts[1][1], "end");
        assert_eq!(state.column_widths(&table), widths);
        assert_eq!(state.regions_of(&table), regions);
    }

    // -- validate after a remote change ---------------------------------------

    #[test]
    fn a_region_whose_corner_is_gone_is_dropped_by_validation() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        merge(&mut state, table, (0, 0), (1, 1));
        // A delete the table command did not make: the corner goes, and the
        // record stays until a pass settles it.
        let corner = cell_at(&state, table, 1, 1);
        state.handle_command(ProseCommand::DeleteBlock {
            block_id: block_id(corner),
        });
        assert_eq!(state.regions_of(&table).len(), 1);
        state.validate_tables();
        assert!(state.regions_of(&table).is_empty());
    }

    #[test]
    fn of_two_regions_over_one_cell_the_later_stands() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        merge(&mut state, table, (0, 0), (0, 1));
        let earlier = state.regions_of(&table);
        // A second record over the same cell, written as a peer's concurrent
        // merge lands: straight into the map, with a later stamp.
        let (anchor, extent) = (cell_at(&state, table, 0, 1), cell_at(&state, table, 1, 1));
        let later = earlier
            .first()
            .map_or(0, |region| region.stamp.to_i64_saturating())
            + 10;
        let map = state
            .table_attrs(&table)
            .and_then(|attrs| doc::map_at(&attrs, doc::AttrKey::Merged))
            .expect("the region map");
        let record = LoroValue::Map(
            [
                (
                    doc::RegionKey::Extent.as_str().to_owned(),
                    LoroValue::from(extent.to_string()),
                ),
                (
                    doc::RegionKey::Stamp.as_str().to_owned(),
                    LoroValue::I64(later),
                ),
            ]
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>()
            .into(),
        );
        map.insert(&anchor.to_string(), record)
            .expect("the record lands");
        state.validate_tables();
        let standing = state.regions_of(&table);
        assert_eq!(standing.len(), 1);
        assert_eq!(standing[0].anchor, anchor);
        assert_eq!(standing[0].stamp, ambre_core::LamportStamp::from_i64(later));
    }

    /// `PROSE-FEAT-254`: the later region is the one with the greater Lamport
    /// stamp, whatever order the records arrive in. A record stamped before
    /// the standing one is the one dropped.
    #[test]
    fn region_resolution_orders_by_the_lamport_stamp() {
        let (mut state, table) = document_with_table(ambre_core::PeerId(1), 2, 2);
        merge(&mut state, table, (0, 0), (0, 1));
        let first = state.regions_of(&table)[0];
        assert!(first.stamp > ambre_core::LamportStamp(0), "{}", first.stamp);
        let (anchor, extent) = (cell_at(&state, table, 0, 1), cell_at(&state, table, 1, 1));
        let map = state
            .table_attrs(&table)
            .and_then(|attrs| doc::map_at(&attrs, doc::AttrKey::Merged))
            .expect("the region map");
        let earlier = first.stamp.to_i64_saturating() - 1;
        let record = LoroValue::Map(
            [
                (
                    doc::RegionKey::Extent.as_str().to_owned(),
                    LoroValue::from(extent.to_string()),
                ),
                (
                    doc::RegionKey::Stamp.as_str().to_owned(),
                    LoroValue::I64(earlier),
                ),
            ]
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>()
            .into(),
        );
        map.insert(&anchor.to_string(), record)
            .expect("the record lands");
        state.validate_tables();
        assert_eq!(state.regions_of(&table), [first], "the earlier stamp drops");
    }
}
