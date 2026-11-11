// ambre-trame/src/logic/line_cells.rs
// The cells of each line, kept by a subscription to the document (`TRAME-081`)
//
// A delete purges every cell and style keyed by an element of the removed
// line. A walk of the whole cell map finds them at a cost that grows with the
// sheet. This index answers the same question from the line alone. A
// subscription keeps it from every event of the document: a local commit, an
// import, an undo and a checkout alike. No write path updates it by hand, so
// none can forget to.

use crate::hash::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::id::{Id, Sheet};
use loro::ContainerID;
#[cfg(all(test, not(target_arch = "wasm32")))]
use loro::LoroDoc;
use loro::event::{ContainerDiff, Diff};

use super::{Axis, SheetContainer, SheetRoot};
use crate::model::{CellKey, Element, parse_cell_key};

/// The two maps a line purge reads: the cells and their styles.
pub(super) const PURGED: [SheetRoot; 2] = [SheetRoot::Cells, SheetRoot::CellStyle];

/// The keys of one map, by the element of each axis they sit under.
#[derive(Debug, Default, PartialEq, Eq)]
struct ByLine {
    /// For each row element, the column elements of its keys.
    rows: HashMap<Element, HashSet<Element>>,
    /// For each column element, the row elements of its keys.
    cols: HashMap<Element, HashSet<Element>>,
}

impl ByLine {
    /// Note the key at `row`, `col`.
    fn insert(&mut self, row: Element, col: Element) {
        self.rows.entry(row).or_default().insert(col);
        self.cols.entry(col).or_default().insert(row);
    }

    /// Forget the key at `row`, `col`. A line left with no key leaves the
    /// index, so the index equals one built from the map alone.
    fn remove(&mut self, row: Element, col: Element) {
        fn drop_one(lines: &mut HashMap<Element, HashSet<Element>>, line: Element, other: Element) {
            if let Some(others) = lines.get_mut(&line) {
                others.remove(&other);
                if others.is_empty() {
                    lines.remove(&line);
                }
            }
        }
        drop_one(&mut self.rows, row, col);
        drop_one(&mut self.cols, col, row);
    }
}

/// The keys each purged map holds, by line. A key no reader parses is left
/// out, as the purge leaves it in the map (`TRAME-248`).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct LineCells {
    maps: HashMap<SheetContainer, ByLine>,
}

/// The shared index a document subscription writes into.
pub(super) type LineSink = Arc<Mutex<LineCells>>;

/// One key a purged map gained or lost: its text, the pair it reads as, and
/// whether the key now holds a value.
pub(super) struct KeyChange<'a> {
    pub(super) key: &'a str,
    pub(super) pair: Option<(Element, Element)>,
    pub(super) held: KeyHeld,
}

ambre_core::define_flag! {
    /// Whether a key a change named holds a value after it.
    pub(super) enum KeyHeld {
        /// The change deleted the key.
        Gone,
        /// The change wrote a value under the key.
        Held,
    }
}

/// The keys `change` wrote in the purged map `root` of a sheet, each parsed
/// once. `None` for a change of any other container, or of another kind.
pub(super) fn purged_keys<'a>(
    change: &'a ContainerDiff<'_>,
) -> Option<(SheetRoot, Id<Sheet>, Vec<KeyChange<'a>>)> {
    let ContainerID::Root { name, .. } = change.target else {
        return None;
    };
    let (root, sheet) = SheetContainer::parse(name.as_str())?;
    if !PURGED.contains(&root) {
        return None;
    }
    let Diff::Map(map) = &change.diff else {
        return None;
    };
    let keys = map
        .updated
        .iter()
        .map(|(key, value)| KeyChange {
            key: key.as_ref(),
            pair: parse_cell_key(key),
            held: match value {
                Some(_) => KeyHeld::Held,
                None => KeyHeld::Gone,
            },
        })
        .collect();
    Some((root, sheet, keys))
}

impl LineCells {
    /// Apply the keys one change wrote in the purged map `root` of `sheet`,
    /// as [`purged_keys`] read them. A key no reader parses is left out.
    pub(super) fn note(&mut self, root: SheetRoot, sheet: &Id<Sheet>, keys: &[KeyChange<'_>]) {
        let by = self
            .maps
            .entry(SheetContainer::of(root, sheet))
            .or_default();
        for change in keys {
            let Some((row, col)) = change.pair else {
                continue;
            };
            match change.held {
                KeyHeld::Held => by.insert(row, col),
                KeyHeld::Gone => by.remove(row, col),
            }
        }
        if by.rows.is_empty() {
            self.maps.remove(&SheetContainer::of(root, sheet));
        }
    }

    /// The keys the map `root` of `sheet` holds under each of `elements` of
    /// `axis`. The map is found once for every element.
    pub(super) fn keys_on<'e>(
        &self,
        root: SheetRoot,
        sheet: &Id<Sheet>,
        axis: Axis,
        elements: impl IntoIterator<Item = &'e Element>,
    ) -> Vec<CellKey> {
        let Some(by) = self.maps.get(&SheetContainer::of(root, sheet)) else {
            return Vec::new();
        };
        let lines = match axis {
            Axis::Row => &by.rows,
            Axis::Col => &by.cols,
        };
        let mut keys = Vec::new();
        for element in elements {
            let Some(others) = lines.get(element) else {
                continue;
            };
            keys.extend(others.iter().map(|other| match axis {
                Axis::Row => CellKey::of(*element, *other),
                Axis::Col => CellKey::of(*other, *element),
            }));
        }
        keys
    }

    /// The index a walk of every purged map of `doc` builds: the reference
    /// model the kept index must equal. A map of a deleted sheet counts too.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn walk(doc: &LoroDoc) -> Self {
        use loro::LoroValue;

        let mut lines = Self::default();
        let LoroValue::Map(roots) = doc.get_deep_value() else {
            return lines;
        };
        for (name, value) in roots.iter() {
            let Some((root, sheet)) = SheetContainer::parse(name) else {
                continue;
            };
            let LoroValue::Map(entries) = value else {
                continue;
            };
            if !PURGED.contains(&root) {
                continue;
            }
            let mut by = ByLine::default();
            for (row, col) in entries.keys().filter_map(|key| parse_cell_key(key)) {
                by.insert(row, col);
            }
            if !by.rows.is_empty() {
                lines.maps.insert(SheetContainer::of(root, &sheet), by);
            }
        }
        lines
    }
}

/// Read the index kept in `sink`.
pub(super) fn read(sink: &LineSink) -> std::sync::MutexGuard<'_, LineCells> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}
