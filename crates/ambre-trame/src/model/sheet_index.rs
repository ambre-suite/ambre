// ambre-trame/src/model/sheet_index.rs
// ============================================================================
// SheetIndex -- bidirectional grid-position <-> stable-uid cache for one sheet.
//
// Rows and columns of a sheet live in two `LoroMovableList`s. Each element
// carries an own, move-invariant `Uid` that *we* mint, stored as the element's
// *value* (a string), NOT a Loro cursor id. Cells are keyed in `cells_{sheet}`
// by the *pair* of uids `"{row_uid}:{col_uid}"`, so a row insert is one Loro op
// and never rewrites a cell key, and a reorder (`mov`) leaves every key intact
// because the uid travels with the value (`ambre-spec-trame-logic.md` §2.A).
//
// The lists are **dense over the used range** `[0..hwm]`: list index == grid
// position. There is exactly one element per row/column up to the high-water
// mark; rows/columns beyond it are virtual (no element). This keeps `resolve`
// an O(1) index lookup and keeps concurrent inserts convergent and
// intent-preserving (each insert mints an independent uid, no shared counter).
// See §2.A bis and the rejected "sparse run-length" / "Loro slot id as key"
// entries in §13.
//
// `SheetIndex` is the *only* translation layer between grid positions and uids.
// ============================================================================

use std::collections::HashMap;

use loro::{LoroMovableList, LoroValue};

/// Our own, move-invariant row/column identity (§2.A). Minted as
/// `{peer = document PeerID, counter = per-session monotonic}` and stored as the
/// value of an element in a `rows_/cols_` `LoroMovableList`. `Copy + Eq + Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid {
    pub peer: u64,
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
            peer: u64::from_str_radix(peer, 16).ok()?,
            counter: u32::from_str_radix(ctr, 16).ok()?,
        })
    }
}

/// Build the 51-char fixed-length cell key from a `(row, col)` uid pair, per
/// `ambre-spec-trame-logic.md` §2.A: `"{row_uid}:{col_uid}"`.
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
#[derive(Debug, Default, Clone)]
pub struct SheetIndex {
    row_by_pos: Vec<Uid>,
    col_by_pos: Vec<Uid>,
    row_to_pos: HashMap<Uid, usize>,
    col_to_pos: HashMap<Uid, usize>,
}

impl SheetIndex {
    /// O(N_used) reconstruction from the two `LoroMovableList`s at load time
    /// (N_used = materialized rows + columns, never the 100k virtual grid). Reads
    /// each element's uid *value* in list order (§2.C).
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self {
        let (row_by_pos, row_to_pos) = collect_ids(rows);
        let (col_by_pos, col_to_pos) = collect_ids(cols);
        Self { row_by_pos, col_by_pos, row_to_pos, col_to_pos }
    }

    /// Number of materialized rows / columns (the used-range high-water mark).
    pub fn row_len(&self) -> usize {
        self.row_by_pos.len()
    }
    pub fn col_len(&self) -> usize {
        self.col_by_pos.len()
    }

    /// Resolve a grid position to its `(row_lid, col_lid)` pair. Returns `None`
    /// if either coordinate is beyond the used range (a virtual, never-written
    /// cell -- the caller renders its generated label).
    pub fn resolve(&self, row: usize, col: usize) -> Option<(Uid, Uid)> {
        Some((*self.row_by_pos.get(row)?, *self.col_by_pos.get(col)?))
    }

    /// Record a newly materialized row id appended at the end of the list
    /// (used by the write path when extending the used range). The caller must
    /// have just pushed the matching element onto the Loro `rows_{sheet}` list.
    pub fn push_row(&mut self, id: Uid) {
        self.row_to_pos.insert(id, self.row_by_pos.len());
        self.row_by_pos.push(id);
    }
    pub fn push_col(&mut self, id: Uid) {
        self.col_to_pos.insert(id, self.col_by_pos.len());
        self.col_by_pos.push(id);
    }

    /// Stable id of the row/column at grid position `pos`, if materialized.
    /// Unlike [`resolve`](Self::resolve) this needs only one axis, so it works
    /// even when the other axis has zero materialized elements (used by the
    /// structural delete path to identify the row/column being removed).
    pub fn row_id(&self, pos: usize) -> Option<Uid> {
        self.row_by_pos.get(pos).copied()
    }
    pub fn col_id(&self, pos: usize) -> Option<Uid> {
        self.col_by_pos.get(pos).copied()
    }

    /// The full materialized row order, position 0..hwm. Used by the sort path
    /// to compute a row permutation and replay it as `LoroMovableList` moves.
    pub fn row_ids(&self) -> &[Uid] {
        &self.row_by_pos
    }

    /// Grid position of a row/column id, if it is materialized. Inverse of
    /// [`resolve`](Self::resolve), used to map a stored cell key back to its
    /// `(row, col)` grid position when reseeding the formula engine.
    pub fn row_pos(&self, id: &Uid) -> Option<usize> {
        self.row_to_pos.get(id).copied()
    }
    pub fn col_pos(&self, id: &Uid) -> Option<usize> {
        self.col_to_pos.get(id).copied()
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
    /// the CRDT side stays a single Loro op. (Wired by the insert-row command in
    /// a later step; provided now so that step only adds the command plumbing.)
    pub fn apply_insert_row(&mut self, pos: usize, id: Uid) {
        self.row_by_pos.insert(pos, id);
        self.reindex_rows_from(pos);
    }
    pub fn apply_insert_col(&mut self, pos: usize, id: Uid) {
        self.col_by_pos.insert(pos, id);
        self.reindex_cols_from(pos);
    }

    /// Incremental update mirroring a `rows_{sheet}.delete(pos, 1)` structural
    /// op. Returns the removed id so the caller can drop its cells / detect
    /// `#REF!`.
    pub fn apply_delete_row(&mut self, pos: usize) -> Uid {
        let removed = self.row_by_pos.remove(pos);
        self.row_to_pos.remove(&removed);
        self.reindex_rows_from(pos);
        removed
    }
    pub fn apply_delete_col(&mut self, pos: usize) -> Uid {
        let removed = self.col_by_pos.remove(pos);
        self.col_to_pos.remove(&removed);
        self.reindex_cols_from(pos);
        removed
    }

    /// Permute the row order to mirror a `rows_{sheet}.mov(from, to)` (sort,
    /// reorder). The uid is move-invariant, so cells keyed by it follow for
    /// free; only the position mapping shifts. No-op if either index is out of
    /// range. See §2.A / §12.
    pub fn apply_move_row(&mut self, from: usize, to: usize) {
        if from >= self.row_by_pos.len() || to >= self.row_by_pos.len() {
            return;
        }
        let uid = self.row_by_pos.remove(from);
        self.row_by_pos.insert(to, uid);
        self.reindex_rows_from(from.min(to));
    }

    fn reindex_rows_from(&mut self, from: usize) {
        for (i, id) in self.row_by_pos.iter().enumerate().skip(from) {
            self.row_to_pos.insert(*id, i);
        }
    }
    fn reindex_cols_from(&mut self, from: usize) {
        for (i, id) in self.col_by_pos.iter().enumerate().skip(from) {
            self.col_to_pos.insert(*id, i);
        }
    }

    /// Zero-based column index -> spreadsheet letters (0 -> "A", 25 -> "Z",
    /// 26 -> "AA"). From `ambre-spec-trame-logic.md` §2.C.
    pub fn col_letter(index: usize) -> String {
        let mut chars = Vec::new();
        let mut n = index;
        loop {
            chars.push((b'A' + (n % 26) as u8) as char);
            if n < 26 {
                break;
            }
            n = n / 26 - 1;
        }
        chars.into_iter().rev().collect()
    }

    /// Inverse of [`col_letter`](Self::col_letter): "A" -> 0, "AA" -> 26.
    /// Expects uppercase ASCII letters.
    pub fn letter_to_index(s: &str) -> usize {
        s.bytes()
            .fold(0usize, |acc, b| acc * 26 + (b - b'A') as usize + 1)
            - 1
    }
}

/// Read a movable list in order and parse each element's uid *value* (§2.A).
/// Each element holds a `"{peer:016x}_{counter:08x}"` string; a missing or
/// malformed value would be a corruption, logged and skipped (it must not
/// desync the dense `Vec`, so we never push a gap).
fn collect_ids(list: &LoroMovableList) -> (Vec<Uid>, HashMap<Uid, usize>) {
    let LoroValue::List(items) = list.get_value() else {
        return (Vec::new(), HashMap::new());
    };
    let mut by_pos = Vec::with_capacity(items.len());
    let mut to_pos = HashMap::with_capacity(items.len());
    for (pos, item) in items.iter().enumerate() {
        match item {
            LoroValue::String(s) => match Uid::parse(s) {
                Some(uid) => {
                    to_pos.insert(uid, by_pos.len());
                    by_pos.push(uid);
                }
                None => tracing::error!("[SheetIndex] unparsable uid value {s:?} at pos {pos}"),
            },
            other => {
                tracing::error!("[SheetIndex] non-string list element {other:?} at pos {pos}")
            }
        }
    }
    (by_pos, to_pos)
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
            rows.push(Uid { peer: 1, counter: i as u32 }.to_key_part().as_str()).unwrap();
        }
        for i in 0..n_cols {
            cols.push(Uid { peer: 2, counter: i as u32 }.to_key_part().as_str()).unwrap();
        }
        doc.commit();
        let idx = SheetIndex::build(&rows, &cols);
        (doc, idx)
    }

    #[test]
    fn col_letter_round_trips() {
        for (idx, letters) in [(0, "A"), (25, "Z"), (26, "AA"), (27, "AB"), (701, "ZZ"), (702, "AAA")] {
            assert_eq!(SheetIndex::col_letter(idx), letters, "idx {idx}");
            assert_eq!(SheetIndex::letter_to_index(letters), idx, "letters {letters}");
        }
    }

    #[test]
    fn build_maps_positions_to_stable_ids() {
        let (_doc, idx) = index_with(5, 3);
        assert_eq!(idx.row_len(), 5);
        assert_eq!(idx.col_len(), 3);

        // Every resolvable position round-trips through contains_*.
        let (r2, c1) = idx.resolve(2, 1).expect("in range");
        assert!(idx.contains_row(&r2));
        assert!(idx.contains_col(&c1));
        // Distinct positions have distinct ids.
        assert_ne!(idx.resolve(0, 0), idx.resolve(1, 0));
    }

    #[test]
    fn resolve_returns_none_in_virtual_space() {
        let (_doc, idx) = index_with(4, 2);
        assert!(idx.resolve(3, 1).is_some());
        assert!(idx.resolve(4, 1).is_none(), "row beyond used range is virtual");
        assert!(idx.resolve(3, 2).is_none(), "col beyond used range is virtual");
    }

    #[test]
    fn apply_insert_and_delete_row_shift_positions() {
        let (_doc, mut idx) = index_with(3, 1);
        let original_first = idx.resolve(0, 0).unwrap().0;

        // Insert a synthetic id at position 0: the old row 0 becomes row 1.
        let new_id = Uid { peer: 999, counter: 7 };
        idx.apply_insert_row(0, new_id);
        assert_eq!(idx.row_len(), 4);
        assert_eq!(idx.resolve(0, 0).unwrap().0, new_id);
        assert_eq!(idx.resolve(1, 0).unwrap().0, original_first);

        // Delete it again: positions restore.
        let removed = idx.apply_delete_row(0);
        assert_eq!(removed, new_id);
        assert!(!idx.contains_row(&new_id));
        assert_eq!(idx.resolve(0, 0).unwrap().0, original_first);
    }

    #[test]
    fn apply_move_row_permutes_order_and_reindexes() {
        let (_doc, mut idx) = index_with(4, 1);
        let ids: Vec<Uid> = (0..4).map(|p| idx.row_id(p).unwrap()).collect();

        // Move row 0 down to position 2: order becomes [1, 2, 0, 3].
        idx.apply_move_row(0, 2);
        assert_eq!(idx.row_ids(), &[ids[1], ids[2], ids[0], ids[3]]);
        // Position cache is consistent both ways for every moved id.
        assert_eq!(idx.row_pos(&ids[0]), Some(2));
        assert_eq!(idx.row_pos(&ids[1]), Some(0));
        assert_eq!(idx.row_pos(&ids[2]), Some(1));
        assert_eq!(idx.row_pos(&ids[3]), Some(3));
        // No id is lost; the cell-key identity of each row is preserved.
        for id in &ids {
            assert!(idx.contains_row(id));
        }

        // Out-of-range indices are a no-op (defends the sort replay loop).
        let snapshot = idx.row_ids().to_vec();
        idx.apply_move_row(0, 9);
        idx.apply_move_row(9, 0);
        assert_eq!(idx.row_ids(), snapshot.as_slice());
    }

    #[test]
    fn cell_key_is_fixed_length_and_stable() {
        let r = Uid { peer: 0x1234_5678_9abc_def0, counter: 0xffff_ffff };
        let c = Uid { peer: 1, counter: 2 };
        let key = cell_key(r, c);
        assert_eq!(key.len(), 51);
        assert_eq!(key, cell_key(r, c), "deterministic");
        assert_ne!(key, cell_key(c, r), "order matters");
        assert_eq!(parse_cell_key(&key), Some((r, c)), "round-trips, full u32 counter");
        assert!(parse_cell_key("garbage").is_none());
    }
}
