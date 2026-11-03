// ambre-trame/src/logic/sort.rs.rs
// Single-column range/sheet sort
//
// Part of the `TrameState` impl, split out of the original logic.rs by concern.
// Submodule of `logic`, so it reaches the private fields/helpers of the parent
// module through the `use super::*` glob.

use super::*;
use crate::formula::identity::{AxisEdit, LineEdit};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Len, Pos, Span};

impl TrameState {
    /// Single-column sort of a row range (`trame-sort`).
    ///
    /// Reorders whole rows of the active sheet by the typed value in the column
    /// `sort_col` columns into `rect`. Cells of a row outside the range's columns
    /// travel with it (Google Sheets "sort range").
    ///
    /// **Realized as a pure row-order permutation of `rows_{sheet}`**. Each sortable row
    /// is keyed by its own move-invariant `Uid`, the list element *value*, not a Loro slot
    /// id. The desired order is replayed with `LoroMovableList::mov`. Cells keyed by
    /// `(row_uid, col_uid)` travel with their row for free -- no cell content is read or
    /// rewritten (`trame-rows-and-columns`, `trame-formula-engine`).
    ///
    /// References then **follow the data**. The formula engine's bytecode is uid-anchored,
    /// so the sort preserves it (no reseed/re-parse) and merely recomputes. Every `A1`
    /// then resolves to its row/column's new position, the `trame-formula-engine`
    /// guarantee. The `mov` ops produce a CRDT delta like any edit; LWW on the row order
    /// resolves concurrent sorts deterministically.
    ///
    /// Sorting only ever targets the **active sheet**: the UI sends an
    /// [`ActivateSheet`](TrameCommand::ActivateSheet) before any range op. The formula
    /// cache the sort keys read from is pinned to index 0. A command for any other sheet,
    /// or an empty/virtual range, is a no-op.
    pub(super) fn sort_range(
        &mut self,
        sheet_id: &Id<Sheet>,
        rect: CellRect,
        sort_col: Len<Col>,
        order: SortOrder,
        header: HeaderRow,
    ) -> TrameOutcome {
        // Only the active sheet has its values positioned in the engine cache.
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        let sort_col_abs = rect.top_left.col + sort_col;
        if sort_col_abs > rect.bottom_right.col {
            return TrameOutcome::display_only(vec![]);
        }
        let Some(rows) = self.sortable_rows(rect, header_rows(header)) else {
            return TrameOutcome::display_only(vec![]);
        };

        // Key each sortable row by its uid and the evaluated value of its sort
        // column. `evaluated_value` returns the formula result or parsed literal;
        // an empty cell sorts as `Empty` (last). Whole rows travel by uid, so no
        // cell contents are read or rewritten -- the sort is a pure permutation.
        let mut keyed: Vec<(Uid, CellValue)> = rows
            .into_iter()
            .filter_map(|pos| {
                let row_uid = self.index.row_id(pos)?;
                let value = self
                    .engine
                    .evaluated_value(self.active_cell_id(CellPos::new(pos, sort_col_abs)))
                    .unwrap_or(CellValue::Empty);
                Some((row_uid, value))
            })
            .collect();

        // Current uid order of the sortable region, to detect an already-sorted
        // no-op after the stable sort below.
        let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
        keyed.sort_by(|(_, a), (_, b)| {
            let ord = compare_cell_values(a, b);
            directed(ord, order)
        });
        let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
        self.realize_sort(rows.start, &before, target)
    }

    /// The rows of `rect` a sort reorders: clamped to the materialized rows,
    /// past its `header` rows. `None` with fewer than two rows to order.
    fn sortable_rows(&self, rect: CellRect, header: Len<Row>) -> Option<Span<Row>> {
        // Virtual rows carry no cell data to reorder and no value to sort on.
        let row_end = rect.bottom_right.row.min(Row::len(&self.index).last()?);
        if rect.top_left.row > row_end {
            return None;
        }
        // The header rows stay pinned in place.
        let sort_start = rect.top_left.row + header;
        // Zero or one sortable row: nothing to reorder.
        (sort_start < row_end).then(|| sort_start.through(row_end))
    }

    /// Multi-column sort of a row range (`trame-sort`).
    ///
    /// Same pure row-order permutation as [`sort_range`](Self::sort_range), only
    /// the comparator changes. Rows are ordered lexicographically by `sort_keys` in
    /// decreasing priority (`[primary, secondary, ...]`), each key applying its own
    /// ascending/descending direction. Each `SortKey::col` is an offset within the
    /// range (`0` = its first column). Keys whose column falls outside the range are
    /// skipped. Cells follow their move-invariant row `Uid`, references follow the
    /// data (`trame-formula-engine`); LWW resolves concurrent sorts.
    pub(super) fn sort_range_multi(
        &mut self,
        sheet_id: &Id<Sheet>,
        rect: CellRect,
        sort_keys: &[SortKey],
        header: HeaderRow,
    ) -> TrameOutcome {
        // Only the active sheet has its values positioned in the engine cache.
        if *sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        // Resolve the keys to absolute columns inside the range; drop out-of-range
        // ones. With no usable key there is nothing to order by.
        let keys: Vec<(Pos<Col>, SortOrder)> = sort_keys
            .iter()
            .map(|k| (rect.top_left.col + k.col, k.order))
            .filter(|(col, _)| *col <= rect.bottom_right.col)
            .collect();
        if keys.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        let Some(rows) = self.sortable_rows(rect, header_rows(header)) else {
            return TrameOutcome::display_only(vec![]);
        };

        // Key each sortable row by its uid and the evaluated value in every sort
        // column. Whole rows travel by uid, so no cell contents are rewritten.
        let mut keyed: Vec<(Uid, Vec<CellValue>)> = rows
            .into_iter()
            .filter_map(|pos| {
                let row_uid = self.index.row_id(pos)?;
                let values = keys
                    .iter()
                    .map(|&(col, _)| {
                        self.engine
                            .evaluated_value(self.active_cell_id(CellPos::new(pos, col)))
                            .unwrap_or(CellValue::Empty)
                    })
                    .collect();
                Some((row_uid, values))
            })
            .collect();

        let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
        keyed.sort_by(|(_, a), (_, b)| {
            // Lexicographic over the keys: first non-equal column decides, each
            // key applying its own direction.
            for ((_, order), (va, vb)) in keys.iter().zip(a.iter().zip(b)) {
                let ord = directed(compare_cell_values(va, vb), *order);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
        let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
        self.realize_sort(rows.start, &before, target)
    }

    /// Realize a computed row permutation as `mov` ops on `rows_{sheet}` and
    /// refresh the derived caches. Shared by the single- and multi-column sorts.
    /// `before` is the current uid order of the sortable region, `target` the
    /// desired one (same multiset), and `sort_start` the first sortable position.
    /// Returns [`TrameEvent::SortComplete`] alone (no CRDT delta) when the order
    /// is already correct.
    fn realize_sort(
        &mut self,
        sort_start: Pos<Row>,
        before: &[Uid],
        target: Vec<Uid>,
    ) -> TrameOutcome {
        if before == target {
            // Stable sort left the order untouched: acknowledge, no CRDT delta.
            return TrameOutcome::display_only(vec![TrameEvent::SortComplete]);
        }

        // Realize the permutation as `mov` ops on `rows_{sheet}`: cells keyed by
        // the move-invariant uid follow for free (`trame-rows-and-columns`).
        // Move each desired
        // uid into place left-to-right over the sortable region; `cur` mirrors the
        // list's evolving order so the `mov` indices stay correct.
        let list = self
            .doc
            .get_movable_list(SheetContainer::of(SheetRoot::Rows, &self.active_sheet).as_str());
        let mut cur: Vec<Uid> = self.index.row_ids().as_slice().to_vec();
        let Some(base) = sort_start.to_index() else {
            return TrameOutcome::display_only(vec![TrameEvent::SortComplete]);
        };
        for (offset, want) in target.into_iter().enumerate() {
            let dst = base + offset;
            if cur.get(dst) == Some(&want) {
                continue;
            }
            // The wanted uid is always at or after `dst` (earlier slots are fixed).
            let src = cur
                .iter()
                .skip(dst)
                .position(|u| *u == want)
                .map(|p| p + dst);
            let Some(src) = src else { continue };
            let _ = list.mov(src, dst);
            let u = cur.remove(src);
            cur.insert(dst, u);
        }
        let rewritten = self.reanchor_formulas(AxisEdit::Rows(LineEdit::Reorder));
        self.doc.commit();

        let mut outcome = self.after_reorder(rewritten);
        outcome.display.push(TrameEvent::SortComplete);
        outcome
    }

    /// Shared tail of any whole-line reorder (sort, drag move): refresh the position<->uid
    /// cache and the position-keyed derived caches (merges, formatting, filter) for the new
    /// order. The compiled bytecode is uid-anchored, so a cell reference already follows
    /// its moved data, and recomputing is enough. A formula whose range was re-anchored
    /// (`rewritten`, `TRAME-227`) changed its text, so the engine is reseeded from it.
    pub(super) fn after_reorder(&mut self, rewritten: bool) -> TrameOutcome {
        self.rebuild_index();
        let mut display = self.rebuild_merges();
        self.reseed_styles();
        // The hidden set is keyed by row uid, so it follows the rows across the
        // permutation -- remap positions, do not recompute.
        self.rebuild_filter();
        // So are the sizes: each follows its own line (`TRAME-170`).
        self.reseed_metrics();
        if rewritten {
            self.reseed_engine();
        } else {
            let indices = self.engine_indices();
            self.engine.mark_all_dirty();
            self.engine.compute_all_dirty(&indices);
        }
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome {
            display,
            update,
            ephemeral: None,
        }
    }
}

/// The rows a sort leaves in place at the top of its range.
fn header_rows(header: HeaderRow) -> Len<Row> {
    match header {
        HeaderRow::Absent => Len::<Row>::ZERO,
        HeaderRow::Present => Len::<Row>::new(1),
    }
}

/// `ord`, turned around for a descending key.
fn directed(ord: std::cmp::Ordering, order: SortOrder) -> std::cmp::Ordering {
    match order {
        SortOrder::Ascending => ord,
        SortOrder::Descending => ord.reverse(),
    }
}
