// ambre-trame/src/model/sheet_index.rs
// ============================================================================
// SheetIndex -- bidirectional grid-position <-> stable-uid cache for one sheet.
//
// Rows and columns of a sheet live in two `LoroMovableList`s. Each element
// carries an own, move-invariant `Uid` that *we* mint, stored as the element's
// *value* (a string), NOT a Loro cursor id. Cells are keyed in `cells_{sheet}`
// by the *pair* of uids `"{row_uid}:{col_uid}"`. A row insert is then one Loro
// op and never rewrites a cell key. A reorder (`mov`) leaves every key intact,
// because the uid travels with the value (`trame-rows-and-columns`).
//
// The lists are **dense over the used range** `[0..hwm]`: list index == grid
// position. There is exactly one element per row/column up to the high-water
// mark; rows/columns beyond it are virtual (no element). This keeps `resolve`
// an O(1) index lookup and keeps concurrent inserts convergent and
// intent-preserving (each insert mints an independent uid, no shared counter).
// See `trame-rows-and-columns`, and the rejected "sparse run-length" / "Loro
// slot id as key" entries in `trame-schema`.
//
// `SheetIndex` is the *only* translation layer between grid positions and uids.
// ============================================================================

use std::collections::HashMap;

use ambre_bus::{CellPos, CellRect, Col, ColCount, ColVec, Row, RowCount, RowVec};
use loro::{LoroMovableList, LoroValue};

/// Our own, move-invariant row/column identity (`trame-rows-and-columns`).
/// Minted as
/// `{peer = document PeerID, counter = per-session monotonic}` and stored as the
/// value of an element in a `rows_/cols_` `LoroMovableList`. `Copy + Eq + Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid {
    pub peer: ambre_core::PeerId,
    pub counter: u32,
}

impl Uid {
    /// Render the 25-char half used in cell keys and as the list element value:
    /// `"{peer:016x}_{counter:08x}"`.
    pub fn to_key_part(&self) -> String {
        format!("{:016x}_{:08x}", self.peer, self.counter)
    }

    /// Parse one `"{peer:016x}_{counter:08x}"` half. `None` on any malformed
    /// input (defensive against a corrupt or foreign entry).
    pub fn parse(s: &str) -> Option<Self> {
        let (peer, ctr) = s.split_once('_')?;
        Some(Uid {
            peer: ambre_core::PeerId(u64::from_str_radix(peer, 16).ok()?),
            counter: u32::from_str_radix(ctr, 16).ok()?,
        })
    }
}

/// Build the 51-char fixed-length cell key from a `(row, col)` uid pair, per
/// `trame-schema`: `"{row_uid}:{col_uid}"`.
pub fn cell_key(row: Uid, col: Uid) -> String {
    format!("{}:{}", row.to_key_part(), col.to_key_part())
}

/// Inverse of [`cell_key`]: parse a stored key back into its `(row, col)` uid
/// pair. Returns `None` on any malformed key.
pub fn parse_cell_key(key: &str) -> Option<(Uid, Uid)> {
    let (row, col) = key.split_once(':')?;
    Some((Uid::parse(row)?, Uid::parse(col)?))
}

/// Bidirectional position <-> uid cache for one sheet's rows and columns.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SheetIndex {
    row_by_pos: RowVec<Uid>,
    col_by_pos: ColVec<Uid>,
    row_to_pos: HashMap<Uid, Row>,
    col_to_pos: HashMap<Uid, Col>,
}

impl SheetIndex {
    /// O(N_used) reconstruction from the two `LoroMovableList`s at load time
    /// (N_used = materialized rows + columns, never the 100k virtual grid). Reads
    /// each element's uid *value* in list order (`trame-engine`).
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self {
        let row_by_pos: RowVec<Uid> = collect_ids(rows).into();
        let col_by_pos: ColVec<Uid> = collect_ids(cols).into();
        let row_to_pos = row_by_pos
            .iter_enumerated()
            .map(|(p, id)| (*id, p))
            .collect();
        let col_to_pos = col_by_pos
            .iter_enumerated()
            .map(|(p, id)| (*id, p))
            .collect();
        Self {
            row_by_pos,
            col_by_pos,
            row_to_pos,
            col_to_pos,
        }
    }

    /// Number of materialized rows / columns (the used-range high-water mark).
    pub fn row_len(&self) -> RowCount {
        self.row_by_pos.len()
    }
    pub fn col_len(&self) -> ColCount {
        self.col_by_pos.len()
    }

    /// The rectangle of materialized cells, or `None` while an axis is empty.
    pub fn used_range(&self) -> Option<CellRect> {
        let last_row = self.row_len().as_end().prev()?;
        let last_col = self.col_len().as_end().prev()?;
        Some(CellRect::spanning(
            CellPos::ORIGIN,
            CellPos::new(last_row, last_col),
        ))
    }

    /// Resolve a grid position to its `(row_lid, col_lid)` pair. Returns `None`
    /// if either coordinate is beyond the used range (a virtual, never-written
    /// cell -- the caller renders its generated label).
    pub fn resolve(&self, at: CellPos) -> Option<(Uid, Uid)> {
        Some((self.row_id(at.row)?, self.col_id(at.col)?))
    }

    /// Record a newly materialized row id appended at the end of the list
    /// (used by the write path when extending the used range). The caller must
    /// have just pushed the matching element onto the Loro `rows_{sheet}` list.
    pub fn push_row(&mut self, id: Uid) {
        let at = self.row_by_pos.push(id);
        self.row_to_pos.insert(id, at);
    }
    pub fn push_col(&mut self, id: Uid) {
        let at = self.col_by_pos.push(id);
        self.col_to_pos.insert(id, at);
    }

    /// Stable id of the row/column at grid position `pos`, if materialized.
    /// Unlike [`resolve`](Self::resolve) this needs only one axis. So it works
    /// even when the other axis has zero materialized elements, as in the
    /// structural delete path that identifies the row/column being removed.
    pub fn row_id(&self, pos: Row) -> Option<Uid> {
        self.row_by_pos.get(pos).copied()
    }
    pub fn col_id(&self, pos: Col) -> Option<Uid> {
        self.col_by_pos.get(pos).copied()
    }

    /// The full materialized row order, position 0..hwm. Used by the sort path
    /// to compute a row permutation and replay it as `LoroMovableList` moves.
    pub fn row_ids(&self) -> &RowVec<Uid> {
        &self.row_by_pos
    }

    /// Grid position of a row/column id, if it is materialized. Inverse of
    /// [`resolve`](Self::resolve), used to map a stored cell key back to its
    /// `(row, col)` grid position when reseeding the formula engine.
    pub fn row_pos(&self, id: &Uid) -> Option<Row> {
        self.row_to_pos.get(id).copied()
    }
    pub fn col_pos(&self, id: &Uid) -> Option<Col> {
        self.col_to_pos.get(id).copied()
    }

    /// Grid position of a cell keyed by its `(row, col)` uid pair, if both of
    /// its lines are materialized.
    pub fn pos_of(&self, row: &Uid, col: &Uid) -> Option<CellPos> {
        Some(CellPos::new(self.row_pos(row)?, self.col_pos(col)?))
    }

    /// Whether `id` still exists in this sheet (used for `#REF!` detection: a
    /// formula referencing a deleted row/column resolves to a missing id).
    pub fn contains_row(&self, id: &Uid) -> bool {
        self.row_to_pos.contains_key(id)
    }
    pub fn contains_col(&self, id: &Uid) -> bool {
        self.col_to_pos.contains_key(id)
    }

    /// Incremental update mirroring a `rows_{sheet}.insert(pos, _)` structural
    /// op: shift every row at position `>= pos` down by one. O(N_used) in RAM;
    /// the CRDT side stays a single Loro op. A position past the end inserts
    /// nothing, as the list refuses it too.
    pub fn apply_insert_row(&mut self, pos: Row, id: Uid) {
        if self.row_by_pos.insert(pos, id).is_ok() {
            self.reindex_rows_from(pos);
        }
    }
    pub fn apply_insert_col(&mut self, pos: Col, id: Uid) {
        if self.col_by_pos.insert(pos, id).is_ok() {
            self.reindex_cols_from(pos);
        }
    }

    /// Incremental update mirroring a `rows_{sheet}.delete(pos, 1)` structural
    /// op. Returns the removed id so the caller can drop its cells / detect
    /// `#REF!`, and `None` for a position past the end.
    pub fn apply_delete_row(&mut self, pos: Row) -> Option<Uid> {
        let removed = self.row_by_pos.remove(pos)?;
        self.row_to_pos.remove(&removed);
        self.reindex_rows_from(pos);
        Some(removed)
    }
    pub fn apply_delete_col(&mut self, pos: Col) -> Option<Uid> {
        let removed = self.col_by_pos.remove(pos)?;
        self.col_to_pos.remove(&removed);
        self.reindex_cols_from(pos);
        Some(removed)
    }

    /// Permute the row order to mirror a `rows_{sheet}.mov(from, to)` (sort,
    /// reorder). The uid is move-invariant, so cells keyed by it follow for
    /// free; only the position mapping shifts. No-op if either index is out of
    /// range. See `trame-rows-and-columns` and `trame-formula-engine`.
    pub fn apply_move_row(&mut self, from: Row, to: Row) {
        let len = self.row_by_pos.len().as_end();
        if from >= len || to >= len {
            return;
        }
        if let Some(uid) = self.row_by_pos.remove(from)
            && self.row_by_pos.insert(to, uid).is_ok()
        {
            self.reindex_rows_from(from.min(to));
        }
    }

    fn reindex_rows_from(&mut self, from: Row) {
        for (at, id) in self
            .row_by_pos
            .iter_enumerated()
            .skip_while(|(at, _)| *at < from)
        {
            self.row_to_pos.insert(*id, at);
        }
    }
    fn reindex_cols_from(&mut self, from: Col) {
        for (at, id) in self
            .col_by_pos
            .iter_enumerated()
            .skip_while(|(at, _)| *at < from)
        {
            self.col_to_pos.insert(*id, at);
        }
    }
}

/// Read a movable list in order and parse each element's uid *value*
/// (`trame-rows-and-columns`). Each element holds a
/// `"{peer:016x}_{counter:08x}"` string. A missing or malformed value
/// would be a corruption, logged and skipped. It must not desync the dense
/// list, so we never push a gap.
fn collect_ids(list: &LoroMovableList) -> Vec<Uid> {
    let LoroValue::List(items) = list.get_value() else {
        return Vec::new();
    };
    let mut ids = Vec::with_capacity(items.len());
    for (pos, item) in items.iter().enumerate() {
        match item {
            LoroValue::String(s) => match Uid::parse(s) {
                Some(uid) => ids.push(uid),
                None => tracing::error!("[SheetIndex] unparsable uid value {s:?} at pos {pos}"),
            },
            other => {
                tracing::error!("[SheetIndex] non-string list element {other:?} at pos {pos}")
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use loro::LoroDoc;

    /// Build a `SheetIndex` for a fresh doc with `n_rows` rows and `n_cols`
    /// columns materialized. Returns the doc to keep the lists attached.
    fn index_with(n_rows: usize, n_cols: usize) -> (LoroDoc, SheetIndex) {
        let doc = LoroDoc::new();
        let rows = doc.get_movable_list("rows");
        let cols = doc.get_movable_list("cols");
        for i in 0..n_rows {
            rows.push(
                Uid {
                    peer: ambre_core::PeerId(1),
                    counter: i as u32,
                }
                .to_key_part()
                .as_str(),
            )
            .unwrap();
        }
        for i in 0..n_cols {
            cols.push(
                Uid {
                    peer: ambre_core::PeerId(2),
                    counter: i as u32,
                }
                .to_key_part()
                .as_str(),
            )
            .unwrap();
        }
        doc.commit();
        let idx = SheetIndex::build(&rows, &cols);
        (doc, idx)
    }

    #[test]
    fn col_letter_round_trips() {
        for (idx, letters) in [
            (0, "A"),
            (25, "Z"),
            (26, "AA"),
            (27, "AB"),
            (701, "ZZ"),
            (702, "AAA"),
        ] {
            let col = Col::new(idx);
            assert_eq!(crate::columns::col_to_alpha(col), letters, "idx {idx}");
            assert_eq!(
                crate::columns::alpha_to_col(letters),
                Ok(col),
                "letters {letters}"
            );
        }
    }

    #[test]
    fn build_maps_positions_to_stable_ids() {
        let (_doc, idx) = index_with(5, 3);
        assert_eq!(idx.row_len(), RowCount::new(5));
        assert_eq!(idx.col_len(), ColCount::new(3));

        // Every resolvable position round-trips through contains_*.
        let (r2, c1) = idx
            .resolve(CellPos::new(Row::new(2), Col::new(1)))
            .expect("in range");
        assert!(idx.contains_row(&r2));
        assert!(idx.contains_col(&c1));
        // Distinct positions have distinct ids.
        assert_ne!(
            idx.resolve(CellPos::new(Row::new(0), Col::new(0))),
            idx.resolve(CellPos::new(Row::new(1), Col::new(0)))
        );
    }

    #[test]
    fn resolve_returns_none_in_virtual_space() {
        let (_doc, idx) = index_with(4, 2);
        assert!(
            idx.resolve(CellPos::new(Row::new(3), Col::new(1)))
                .is_some()
        );
        assert!(
            idx.resolve(CellPos::new(Row::new(4), Col::new(1)))
                .is_none(),
            "row beyond used range is virtual"
        );
        assert!(
            idx.resolve(CellPos::new(Row::new(3), Col::new(2)))
                .is_none(),
            "col beyond used range is virtual"
        );
    }

    #[test]
    fn apply_insert_and_delete_row_shift_positions() {
        let (_doc, mut idx) = index_with(3, 1);
        let original_first = idx
            .resolve(CellPos::new(Row::new(0), Col::new(0)))
            .unwrap()
            .0;

        // Insert a synthetic id at position 0: the old row 0 becomes row 1.
        let new_id = Uid {
            peer: ambre_core::PeerId(999),
            counter: 7,
        };
        idx.apply_insert_row(Row::new(0), new_id);
        assert_eq!(idx.row_len(), RowCount::new(4));
        assert_eq!(
            idx.resolve(CellPos::new(Row::new(0), Col::new(0)))
                .unwrap()
                .0,
            new_id
        );
        assert_eq!(
            idx.resolve(CellPos::new(Row::new(1), Col::new(0)))
                .unwrap()
                .0,
            original_first
        );

        // Delete it again: positions restore.
        let removed = idx.apply_delete_row(Row::new(0));
        assert_eq!(removed, Some(new_id));
        assert!(!idx.contains_row(&new_id));
        assert_eq!(
            idx.resolve(CellPos::new(Row::new(0), Col::new(0)))
                .unwrap()
                .0,
            original_first
        );
    }

    #[test]
    fn apply_move_row_permutes_order_and_reindexes() {
        let (_doc, mut idx) = index_with(4, 1);
        let ids: Vec<Uid> = (0..4).map(|p| idx.row_id(Row::new(p)).unwrap()).collect();

        // Move row 0 down to position 2: order becomes [1, 2, 0, 3].
        idx.apply_move_row(Row::new(0), Row::new(2));
        assert_eq!(idx.row_ids().as_slice(), &[ids[1], ids[2], ids[0], ids[3]]);
        // Position cache is consistent both ways for every moved id.
        assert_eq!(idx.row_pos(&ids[0]), Some(Row::new(2)));
        assert_eq!(idx.row_pos(&ids[1]), Some(Row::new(0)));
        assert_eq!(idx.row_pos(&ids[2]), Some(Row::new(1)));
        assert_eq!(idx.row_pos(&ids[3]), Some(Row::new(3)));
        // No id is lost; the cell-key identity of each row is preserved.
        for id in &ids {
            assert!(idx.contains_row(id));
        }

        // Out-of-range indices are a no-op (defends the sort replay loop).
        let snapshot = idx.row_ids().as_slice().to_vec();
        idx.apply_move_row(Row::new(0), Row::new(9));
        idx.apply_move_row(Row::new(9), Row::new(0));
        assert_eq!(idx.row_ids().as_slice(), snapshot.as_slice());
    }

    #[test]
    fn cell_key_is_fixed_length_and_stable() {
        let r = Uid {
            peer: ambre_core::PeerId(0x1234_5678_9abc_def0),
            counter: 0xffff_ffff,
        };
        let c = Uid {
            peer: ambre_core::PeerId(1),
            counter: 2,
        };
        let key = cell_key(r, c);
        assert_eq!(key.len(), 51);
        assert_eq!(key, cell_key(r, c), "deterministic");
        assert_ne!(key, cell_key(c, r), "order matters");
        assert_eq!(
            parse_cell_key(&key),
            Some((r, c)),
            "round-trips, full u32 counter"
        );
        assert!(parse_cell_key("garbage").is_none());
    }

    /// The incremental index equals a rebuild from the lists after any run of
    /// structural operations (`TRAME-023`). Host-only: `proptest` does not
    /// build on wasm32.
    #[cfg(not(target_arch = "wasm32"))]
    mod incremental_equals_rebuild {
        use super::*;
        use proptest::prelude::*;

        #[derive(Debug, Clone)]
        enum Op {
            PushRow,
            PushCol,
            InsertRow(usize),
            InsertCol(usize),
            DeleteRow(usize),
            DeleteCol(usize),
            MoveRow(usize, usize),
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                Just(Op::PushRow),
                Just(Op::PushCol),
                any::<usize>().prop_map(Op::InsertRow),
                any::<usize>().prop_map(Op::InsertCol),
                any::<usize>().prop_map(Op::DeleteRow),
                any::<usize>().prop_map(Op::DeleteCol),
                (any::<usize>(), any::<usize>()).prop_map(|(f, t)| Op::MoveRow(f, t)),
            ]
        }

        proptest! {
            #[test]
            fn the_incremental_index_equals_a_rebuild(ops in prop::collection::vec(op(), 0..60)) {
                let (doc, mut idx) = index_with(3, 2);
                let rows = doc.get_movable_list("rows");
                let cols = doc.get_movable_list("cols");
                for (n, op) in ops.into_iter().enumerate() {
                    let id = Uid { peer: ambre_core::PeerId(9), counter: n as u32 };
                    let key = id.to_key_part();
                    let (nr, nc) = (idx.row_len().get() as usize, idx.col_len().get() as usize);
                    let row = |p: usize| Row::from_index(p).expect("a row in range");
                    let col = |p: usize| Col::from_index(p).expect("a column in range");
                    match op {
                        Op::PushRow => {
                            rows.push(key.as_str()).unwrap();
                            idx.push_row(id);
                        }
                        Op::PushCol => {
                            cols.push(key.as_str()).unwrap();
                            idx.push_col(id);
                        }
                        Op::InsertRow(p) => {
                            let p = p % (nr + 1);
                            rows.insert(p, key.as_str()).unwrap();
                            idx.apply_insert_row(row(p), id);
                        }
                        Op::InsertCol(p) => {
                            let p = p % (nc + 1);
                            cols.insert(p, key.as_str()).unwrap();
                            idx.apply_insert_col(col(p), id);
                        }
                        Op::DeleteRow(p) if nr > 0 => {
                            rows.delete(p % nr, 1).unwrap();
                            idx.apply_delete_row(row(p % nr));
                        }
                        Op::DeleteCol(p) if nc > 0 => {
                            cols.delete(p % nc, 1).unwrap();
                            idx.apply_delete_col(col(p % nc));
                        }
                        Op::MoveRow(f, t) if nr > 0 => {
                            rows.mov(f % nr, t % nr).unwrap();
                            idx.apply_move_row(row(f % nr), row(t % nr));
                        }
                        Op::DeleteRow(_) | Op::DeleteCol(_) | Op::MoveRow(..) => {}
                    }
                    doc.commit();
                    prop_assert_eq!(&idx, &SheetIndex::build(&rows, &cols));
                }
            }
        }
    }
}
