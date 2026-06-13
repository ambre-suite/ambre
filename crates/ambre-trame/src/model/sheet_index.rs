// ambre-trame/src/model/sheet_index.rs
// ============================================================================
// SheetIndex -- bidirectional grid-position <-> stable-id cache for one sheet.
//
// Rows and columns of a sheet live in two `LoroMovableList`s. Each list element
// carries a stable `ListItemId = (PeerID, Counter)` (here `Lid`, an alias of
// `loro::ID`), obtained from a Loro cursor (see below). Cells are keyed in
// `cells_{sheet}` by the *pair* of ids `"{row_lid}:{col_lid}"`, so a row insert
// is one Loro op and never rewrites a cell key (`ambre-spec-trame-logic.md` §2).
//
// The lists are **dense over the used range** `[0..hwm]`: list index == grid
// position. There is exactly one element per row/column up to the high-water
// mark; rows/columns beyond it are virtual (no element). This keeps `resolve`
// an O(1) index lookup and keeps concurrent inserts convergent and
// intent-preserving (each insert is an independent element, no shared counter).
// See §2.A bis and the rejected "sparse run-length" entry in §13.
//
// The formula engine stays positional; `SheetIndex` is the *only* translation
// layer between grid positions and CRDT ids.
// ============================================================================

use std::collections::HashMap;

use loro::cursor::Side;
use loro::{ID, LoroMovableList};

/// Stable identity of a row/column list element. Alias of Loro's op id, which
/// is `Copy + Eq + Hash` and replicates with the document (stable across
/// sessions and peers).
pub type Lid = ID;

/// Build the 51-char fixed-length cell key from a `(row, col)` id pair, per
/// `ambre-spec-trame-logic.md` §2.A: `"{peer:016x}_{ctr:08x}:{peer:016x}_{ctr:08x}"`.
/// `counter` is an `i32`; it is rendered as its unsigned bit pattern so the
/// width is always 8 hex digits.
pub fn cell_key(row: Lid, col: Lid) -> String {
    format!(
        "{:016x}_{:08x}:{:016x}_{:08x}",
        row.peer, row.counter as u32, col.peer, col.counter as u32,
    )
}

/// Inverse of [`cell_key`]: parse a stored key back into its `(row, col)` id
/// pair. Returns `None` on any malformed key (defensive against a corrupt or
/// foreign entry).
pub fn parse_cell_key(key: &str) -> Option<(Lid, Lid)> {
    let (row, col) = key.split_once(':')?;
    Some((parse_lid(row)?, parse_lid(col)?))
}

/// Parse one `"{peer:016x}_{ctr:08x}"` half into a [`Lid`].
fn parse_lid(s: &str) -> Option<Lid> {
    let (peer, ctr) = s.split_once('_')?;
    Some(Lid {
        peer: u64::from_str_radix(peer, 16).ok()?,
        // `ctr` was written as the unsigned bit pattern of the `i32` counter.
        counter: u32::from_str_radix(ctr, 16).ok()? as i32,
    })
}

/// Bidirectional position <-> id cache for one sheet's rows and columns.
#[derive(Debug, Default, Clone)]
pub struct SheetIndex {
    row_by_pos: Vec<Lid>,
    col_by_pos: Vec<Lid>,
    row_to_pos: HashMap<Lid, usize>,
    col_to_pos: HashMap<Lid, usize>,
}

impl SheetIndex {
    /// O(N_used) reconstruction from the two `LoroMovableList`s at load time
    /// (N_used = materialized rows + columns, never the 100k virtual grid).
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
    pub fn resolve(&self, row: usize, col: usize) -> Option<(Lid, Lid)> {
        Some((*self.row_by_pos.get(row)?, *self.col_by_pos.get(col)?))
    }

    /// Record a newly materialized row id appended at the end of the list
    /// (used by the write path when extending the used range). The caller must
    /// have just pushed the matching element onto the Loro `rows_{sheet}` list.
    pub fn push_row(&mut self, id: Lid) {
        self.row_to_pos.insert(id, self.row_by_pos.len());
        self.row_by_pos.push(id);
    }
    pub fn push_col(&mut self, id: Lid) {
        self.col_to_pos.insert(id, self.col_by_pos.len());
        self.col_by_pos.push(id);
    }

    /// Grid position of a row/column id, if it is materialized. Inverse of
    /// [`resolve`](Self::resolve), used to map a stored cell key back to its
    /// `(row, col)` grid position when reseeding the formula engine.
    pub fn row_pos(&self, id: &Lid) -> Option<usize> {
        self.row_to_pos.get(id).copied()
    }
    pub fn col_pos(&self, id: &Lid) -> Option<usize> {
        self.col_to_pos.get(id).copied()
    }

    /// Whether `id` still exists in this sheet (used for `#REF!` detection: a
    /// formula referencing a deleted row/column resolves to a missing id).
    pub fn contains_row(&self, id: &Lid) -> bool {
        self.row_to_pos.contains_key(id)
    }
    pub fn contains_col(&self, id: &Lid) -> bool {
        self.col_to_pos.contains_key(id)
    }

    /// Incremental update mirroring a `rows_{sheet}.insert(pos, _)` structural
    /// op: shift every row at position `>= pos` down by one. O(N_used) in RAM;
    /// the CRDT side stays a single Loro op. (Wired by the insert-row command in
    /// a later step; provided now so that step only adds the command plumbing.)
    pub fn apply_insert_row(&mut self, pos: usize, id: Lid) {
        self.row_by_pos.insert(pos, id);
        self.reindex_rows_from(pos);
    }
    pub fn apply_insert_col(&mut self, pos: usize, id: Lid) {
        self.col_by_pos.insert(pos, id);
        self.reindex_cols_from(pos);
    }

    /// Incremental update mirroring a `rows_{sheet}.delete(pos, 1)` structural
    /// op. Returns the removed id so the caller can drop its cells / detect
    /// `#REF!`.
    pub fn apply_delete_row(&mut self, pos: usize) -> Lid {
        let removed = self.row_by_pos.remove(pos);
        self.row_to_pos.remove(&removed);
        self.reindex_rows_from(pos);
        removed
    }
    pub fn apply_delete_col(&mut self, pos: usize) -> Lid {
        let removed = self.col_by_pos.remove(pos);
        self.col_to_pos.remove(&removed);
        self.reindex_cols_from(pos);
        removed
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

/// Read a movable list in order and capture each element's stable id via its
/// Loro cursor. The cursor at `Side::Middle` anchors to the element itself; its
/// `id` is the op that created it -- stable and unique. An attached, in-range
/// element always yields an id; a `None` would be a Loro inconsistency, logged
/// and skipped (it must not desync the dense `Vec`, so we never push a gap).
fn collect_ids(list: &LoroMovableList) -> (Vec<Lid>, HashMap<Lid, usize>) {
    let len = list.len();
    let mut by_pos = Vec::with_capacity(len);
    let mut to_pos = HashMap::with_capacity(len);
    for pos in 0..len {
        match list.get_cursor(pos, Side::Middle).and_then(|c| c.id) {
            Some(id) => {
                to_pos.insert(id, by_pos.len());
                by_pos.push(id);
            }
            None => {
                tracing::error!(
                    "[SheetIndex] no cursor id for list element at pos {pos} (len {len})"
                );
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
        for _ in 0..n_rows {
            rows.push(loro::LoroValue::Null).unwrap();
        }
        for _ in 0..n_cols {
            cols.push(loro::LoroValue::Null).unwrap();
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
        let new_id = Lid { peer: 999, counter: 7 };
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
    fn cell_key_is_fixed_length_and_stable() {
        let r = Lid { peer: 0x1234_5678_9abc_def0, counter: -1 };
        let c = Lid { peer: 1, counter: 2 };
        let key = cell_key(r, c);
        assert_eq!(key.len(), 51);
        assert_eq!(key, cell_key(r, c), "deterministic");
        assert_ne!(key, cell_key(c, r), "order matters");
        assert_eq!(parse_cell_key(&key), Some((r, c)), "round-trips, counter sign preserved");
        assert!(parse_cell_key("garbage").is_none());
    }
}
