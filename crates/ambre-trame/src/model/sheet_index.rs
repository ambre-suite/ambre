// ambre-trame/src/model/sheet_index.rs
// ============================================================================
// The identity of a line, and the key of a cell.
//
// Rows and columns of a sheet live in two `LoroMovableList`s. Each element
// carries a move-invariant `Uid` as its *value* (a string), never a Loro cursor
// id. Cells are keyed in `cells_{sheet}` by the *pair* of uids
// `"{row_uid}:{col_uid}"`. A row insert is then one Loro op and never rewrites
// a cell key. A reorder (`mov`) leaves every key intact, because the uid
// travels with the value (`trame-rows-and-columns`).
//
// The lists are dense over the used range: one line per row or column up to
// the high-water mark, and virtual lines past it. The axis module reads them
// into a `SheetIndex`, the one translation layer between grid positions and
// uids (`TRAME-275`).
// ============================================================================

use super::Element;

/// Our own, move-invariant row/column identity (`trame-rows-and-columns`).
/// Minted as
/// `{peer = document PeerID, counter = per-session monotonic}` and stored as the
/// value of an element in a `rows_/cols_` `LoroMovableList`. `Copy + Eq + Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uid {
    pub peer: ambre_core::PeerId,
    pub counter: ambre_core::IdCounter,
}

impl Uid {
    /// Render the 25-char half used in cell keys and as the list element value:
    /// `"{peer:016x}_{counter:08x}"`.
    pub fn to_key_part(&self) -> String {
        let mut out = String::with_capacity(KEY_PART_LEN);
        self.push_key_part(&mut out);
        out
    }

    /// Append the key part to `out`, with no format machinery: every cell key
    /// a purge or a replay writes spells two or four of them.
    pub(crate) fn push_key_part(&self, out: &mut String) {
        // Each half zero padded and lower case, as `{:016x}` and `{:08x}`
        // write them.
        let push = |out: &mut String, value: u64, digits: u32| {
            for shift in (0..digits).rev() {
                let nibble = u32::try_from((value >> (shift * 4)) & 0xf).unwrap_or(0);
                out.push(char::from_digit(nibble, 16).unwrap_or('0'));
            }
        };
        push(out, self.peer.get(), 16);
        out.push('_');
        push(out, u64::from(self.counter.get()), 8);
    }

    /// Parse one `"{peer:016x}_{counter:08x}"` half. `None` on any malformed
    /// input (defensive against a corrupt or foreign entry).
    ///
    /// Only the canonical spelling [`to_key_part`](Self::to_key_part) writes
    /// reads back. A short, signed or upper-case spelling would name the same
    /// uid under a second key. A crafted peer could shadow a cell with it, so
    /// it is refused.
    pub fn parse(s: &str) -> Option<Self> {
        // The canonical spelling is 16 lowercase hex digits, `_`, then 8. One
        // walk of the bytes at their fixed places checks and reads them. It
        // allocates nothing: an index build and every replayed cell key read
        // each line element through it.
        let bytes = s.as_bytes();
        if bytes.len() != KEY_PART_LEN || bytes.get(16) != Some(&b'_') {
            return None;
        }
        let value = |digits: &[u8]| {
            digits.iter().try_fold(0u64, |value, digit| {
                let nibble = match digit {
                    b'0'..=b'9' => digit - b'0',
                    b'a'..=b'f' => digit - b'a' + 10,
                    _ => return None,
                };
                Some((value << 4) | u64::from(nibble))
            })
        };
        let peer = value(bytes.get(..16)?)?;
        let counter = u32::try_from(value(bytes.get(17..)?)?).ok()?;
        Some(Uid {
            peer: ambre_core::PeerId::new(peer),
            counter: ambre_core::IdCounter::new(counter),
        })
    }

    /// Parse the compact `"{peer:x}_{counter:x}"` a stored formula writes,
    /// with no padding. The formula parser recognises the token first, and a
    /// stored formula keeps the bytes it was written with.
    pub fn parse_hex(s: &str) -> Option<Self> {
        let (peer, ctr) = s.split_once('_')?;
        Some(Uid {
            peer: ambre_core::PeerId::new(u64::from_str_radix(peer, 16).ok()?),
            counter: ambre_core::IdCounter::new(u32::from_str_radix(ctr, 16).ok()?),
        })
    }
}

/// The length of a key part: 16 hex digits, `_`, then 8.
const KEY_PART_LEN: usize = 25;

/// A uid serialises as its key part, the string the document stores. A record
/// that carries one thus keeps the bytes of the string it held before.
/// The read goes through [`Uid::parse`], and a malformed key part refuses the
/// whole record.
impl serde::Serialize for Uid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_key_part())
    }
}

impl<'de> serde::Deserialize<'de> for Uid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let part = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Uid::parse(&part).ok_or_else(|| serde::de::Error::custom("not a uid key part"))
    }
}

/// The key a cell is stored under in the per-sheet cell, style and merge
/// maps: `"{row_element}:{col_element}"`, per `trame-schema`. Each half is the
/// spelling of the element that carries its line: the bare uid of a minted
/// line, `"{uid}#{element}"` of a derived one. Two minted lines make the
/// 51-char key of version 1. The one place the pair is joined.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey(String);

impl CellKey {
    /// The key of the cell at the `(row, col)` element pair.
    #[must_use]
    pub fn of(row: Element, col: Element) -> Self {
        let mut key = String::with_capacity(4 * KEY_PART_LEN + 3);
        row.push_spelling(&mut key);
        key.push(':');
        col.push_spelling(&mut key);
        Self(key)
    }

    /// The key, for the Loro call into the map.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where the key reads in `index`, and its rank against another key at
    /// the same place. `None` for a key whose element is gone.
    #[must_use]
    pub fn place_in(
        &self,
        index: &super::SheetIndex,
    ) -> Option<(ambre_bus::CellPos, super::CellRank)> {
        index.place(&self.0)
    }
}

/// Inverse of [`CellKey::of`]: parse a stored key back into its `(row, col)`
/// element pair. Returns `None` on any malformed key.
pub fn parse_cell_key(key: &str) -> Option<(Element, Element)> {
    let (row, col) = key.split_once(':')?;
    Some((Element::parse(row)?, Element::parse(col)?))
}

ambre_core::define_flag! {
    /// Whether building an index skipped a line element it could not read.
    pub enum LineRead {
        /// Every element held a uid this build reads.
        Clean,
        /// At least one element held no readable uid, and was skipped.
        Skipped,
    }
}

#[cfg(test)]
mod tests {

    /// The key part spelled by hand is the one `format!` wrote before, on
    /// the edges of both halves and between them.
    #[test]
    fn a_key_part_spells_as_the_format_did() {
        for (peer, counter) in [
            (0, 0),
            (1, 1),
            (u64::MAX, u32::MAX),
            (0x6f22_48d2_c4d5_d214, 0x4e0f_3172),
        ] {
            let uid = Uid {
                peer: ambre_core::PeerId::new(peer),
                counter: ambre_core::IdCounter::new(counter),
            };
            assert_eq!(uid.to_key_part(), format!("{peer:016x}_{counter:08x}"));
            assert_eq!(Uid::parse(&uid.to_key_part()), Some(uid));
        }
        for refused in [
            "",
            "0000000000000001_0000000",
            "0000000000000001-00000001",
            "000000000000000A_00000001",
            "000000000000001_000000001",
            "+000000000000001_00000001",
        ] {
            assert_eq!(Uid::parse(refused), None, "{refused:?} is no key part");
        }
    }

    use super::*;
    use crate::logic::{SheetContainer, SheetRoot, default_sheet};
    use crate::model::SheetIndex;
    use ambre_bus::{CellPos, Col, Row};
    use ambre_core::index::{Len, Pos};
    use loro::LoroDoc;

    /// The order list of one axis of the default sheet, as a workbook names it.
    fn axis(root: SheetRoot) -> SheetContainer {
        SheetContainer::of(root, &default_sheet())
    }

    /// Build a `SheetIndex` for a fresh doc with `n_rows` rows and `n_cols`
    /// columns materialized. Returns the doc to keep the lists attached.
    fn index_with(n_rows: usize, n_cols: usize) -> (LoroDoc, SheetIndex) {
        let doc = LoroDoc::new();
        let rows = doc.get_movable_list(axis(SheetRoot::Rows).as_str());
        let cols = doc.get_movable_list(axis(SheetRoot::Cols).as_str());
        for i in 0..n_rows {
            rows.push(
                Uid {
                    peer: ambre_core::PeerId::new(1),
                    counter: ambre_core::IdCounter::new(i as u32),
                }
                .to_key_part()
                .as_str(),
            )
            .unwrap();
        }
        for i in 0..n_cols {
            cols.push(
                Uid {
                    peer: ambre_core::PeerId::new(2),
                    counter: ambre_core::IdCounter::new(i as u32),
                }
                .to_key_part()
                .as_str(),
            )
            .unwrap();
        }
        doc.commit();
        let idx = crate::model::axis::Axes::of(&doc, &default_sheet()).index();
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
            let col = Pos::<Col>::new(idx);
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
        assert_eq!(idx.row_len(), Len::<Row>::new(5));
        assert_eq!(idx.col_len(), Len::<Col>::new(3));

        // Every resolvable position round-trips through contains_*.
        let (r2, c1) = idx
            .resolve(CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(1)))
            .expect("in range");
        assert!(idx.contains_row(&r2));
        assert!(idx.contains_col(&c1));
        // Distinct positions have distinct ids.
        assert_ne!(
            idx.resolve(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
            idx.resolve(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)))
        );
    }

    #[test]
    fn resolve_returns_none_in_virtual_space() {
        let (_doc, idx) = index_with(4, 2);
        assert!(
            idx.resolve(CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(1)))
                .is_some()
        );
        assert!(
            idx.resolve(CellPos::new(Pos::<Row>::new(4), Pos::<Col>::new(1)))
                .is_none(),
            "row beyond used range is virtual"
        );
        assert!(
            idx.resolve(CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(2)))
                .is_none(),
            "col beyond used range is virtual"
        );
    }

    #[test]
    fn apply_insert_and_delete_row_shift_positions() {
        let (_doc, mut idx) = index_with(3, 1);
        let original_first = idx
            .resolve(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
            .unwrap()
            .0;

        // Insert a synthetic id at position 0: the old row 0 becomes row 1.
        let new_id = Uid {
            peer: ambre_core::PeerId::new(999),
            counter: ambre_core::IdCounter::new(7),
        };
        idx.apply_insert_row(Pos::<Row>::new(0), new_id);
        assert_eq!(idx.row_len(), Len::<Row>::new(4));
        assert_eq!(
            idx.resolve(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
                .unwrap()
                .0,
            new_id
        );
        assert_eq!(
            idx.resolve(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(0)))
                .unwrap()
                .0,
            original_first
        );

        // Delete it again: positions restore.
        let removed = idx.apply_delete_row(Pos::<Row>::new(0));
        assert_eq!(removed, Some(new_id));
        assert!(!idx.contains_row(&new_id));
        assert_eq!(
            idx.resolve(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
                .unwrap()
                .0,
            original_first
        );
    }

    /// A uid reads back only from the spelling `to_key_part` writes. Each other
    /// spelling of the same uid is refused: a second key for one uid could
    /// shadow a cell.
    #[test]
    fn a_uid_reads_back_from_its_canonical_spelling_alone() {
        let uid = Uid {
            peer: ambre_core::PeerId::new(0xab),
            counter: ambre_core::IdCounter::new(2),
        };
        assert_eq!(Uid::parse("00000000000000ab_00000002"), Some(uid));
        for form in [
            "ab_2",
            "+00000000000000ab_00000002",
            "00000000000000AB_00000002",
            "00000000000000ab_+0000002",
            "000000000000000ab_00000002",
        ] {
            assert_eq!(Uid::parse(form), None, "{form}");
        }
        assert_eq!(
            Uid::parse_hex("ab_2"),
            Some(uid),
            "a stored formula's spelling"
        );
    }

    #[test]
    fn apply_move_row_permutes_order_and_reindexes() {
        let (_doc, mut idx) = index_with(4, 1);
        let ids: Vec<Uid> = (0..4)
            .map(|p| idx.row_id(Pos::<Row>::new(p)).unwrap())
            .collect();

        // Move row 0 down to position 2: order becomes [1, 2, 0, 3].
        idx.apply_move_row(Pos::<Row>::new(0), Pos::<Row>::new(2));
        assert_eq!(idx.row_ids().as_slice(), &[ids[1], ids[2], ids[0], ids[3]]);
        // Position cache is consistent both ways for every moved id.
        assert_eq!(idx.row_pos(&ids[0]), Some(Pos::<Row>::new(2)));
        assert_eq!(idx.row_pos(&ids[1]), Some(Pos::<Row>::new(0)));
        assert_eq!(idx.row_pos(&ids[2]), Some(Pos::<Row>::new(1)));
        assert_eq!(idx.row_pos(&ids[3]), Some(Pos::<Row>::new(3)));
        // No id is lost; the cell-key identity of each row is preserved.
        for id in &ids {
            assert!(idx.contains_row(id));
        }

        // Out-of-range indices are a no-op (defends the sort replay loop).
        let snapshot = idx.row_ids().as_slice().to_vec();
        idx.apply_move_row(Pos::<Row>::new(0), Pos::<Row>::new(9));
        idx.apply_move_row(Pos::<Row>::new(9), Pos::<Row>::new(0));
        assert_eq!(idx.row_ids().as_slice(), snapshot.as_slice());
    }

    #[test]
    fn cell_key_is_fixed_length_and_stable() {
        let r = Uid {
            peer: ambre_core::PeerId::new(0x1234_5678_9abc_def0),
            counter: ambre_core::IdCounter::new(0xffff_ffff),
        };
        let c = Uid {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::IdCounter::new(2),
        };
        let (r, c) = (Element::minted(r), Element::minted(c));
        let key = CellKey::of(r, c);
        assert_eq!(
            key.as_str().len(),
            51,
            "two minted lines key as in version 1"
        );
        assert_eq!(key, CellKey::of(r, c), "deterministic");
        assert_ne!(key, CellKey::of(c, r), "order matters");
        assert_eq!(
            parse_cell_key(key.as_str()),
            Some((r, c)),
            "round-trips, full u32 counter"
        );
        let derived = Element::derived(c.uid, crate::model::ElementId::minted(r.uid));
        let key = CellKey::of(derived, r);
        assert_eq!(key.as_str().len(), 77, "a derived half spells its element");
        assert_eq!(parse_cell_key(key.as_str()), Some((derived, r)));
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
                let rows = doc.get_movable_list(axis(SheetRoot::Rows).as_str());
                let cols = doc.get_movable_list(axis(SheetRoot::Cols).as_str());
                for (n, op) in ops.into_iter().enumerate() {
                    let id = Uid { peer: ambre_core::PeerId::new(9), counter: ambre_core::IdCounter::new(n as u32) };
                    let key = id.to_key_part();
                    let (nr, nc) = (idx.row_len().get() as usize, idx.col_len().get() as usize);
                    let row = |p: usize| Pos::<Row>::from_index(p).expect("a row in range");
                    let col = |p: usize| Pos::<Col>::from_index(p).expect("a column in range");
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
                    prop_assert_eq!(&idx, &crate::model::axis::Axes::of(&doc, &default_sheet()).index());
                }
            }
        }
    }
}
