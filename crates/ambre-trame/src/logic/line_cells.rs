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
#[cfg(all(test, not(target_arch = "wasm32")))]
use loro::LoroDoc;
use loro::event::{ContainerDiff, Diff};
use loro::{ContainerID, LoroValue, ValueOrContainer};

use super::{Axis, SheetContainer, SheetRoot};
use crate::model::{CellKey, Element, parse_cell_key};

/// The two maps a line purge reads: the cells and their styles.
pub(super) const PURGED: [SheetRoot; 2] = [SheetRoot::Cells, SheetRoot::CellStyle];

/// The keys of one map, by the element of each axis they sit under, and the
/// value each key holds.
///
/// The value is the map's own, shared and not copied. An undo of a block
/// delete reads here the cells of the lines it brings back. It spells no key
/// and looks nothing up in the document (`TRAME-299`).
#[derive(Debug, Default, PartialEq)]
pub(super) struct ByLine {
    /// For each row element, the value under each column element.
    rows: HashMap<Element, HashMap<Element, LoroValue>>,
    /// For each column element, the row elements of its keys.
    cols: HashMap<Element, HashSet<Element>>,
}

impl ByLine {
    /// Note the key at `row`, `col`, which now holds `value`.
    fn insert(&mut self, row: Element, col: Element, value: LoroValue) {
        self.rows.entry(row).or_default().insert(col, value);
        self.cols.entry(col).or_default().insert(row);
    }

    /// Each column element and its value under the row element `row`.
    pub(super) fn row(&self, row: &Element) -> impl Iterator<Item = (&Element, &LoroValue)> {
        self.rows.get(row).into_iter().flatten()
    }

    /// The value under `row`, `col`, if the map holds the key.
    pub(super) fn get(&self, row: &Element, col: &Element) -> Option<&LoroValue> {
        self.rows.get(row)?.get(col)
    }

    /// Forget the key at `row`, `col`. A line left with no key leaves the
    /// index, so the index equals one built from the map alone.
    fn remove(&mut self, row: Element, col: Element) {
        if let Some(others) = self.rows.get_mut(&row) {
            others.remove(&col);
            if others.is_empty() {
                self.rows.remove(&row);
            }
        }
        if let Some(others) = self.cols.get_mut(&col) {
            others.remove(&row);
            if others.is_empty() {
                self.cols.remove(&col);
            }
        }
    }
}

/// The keys each purged map holds, by line. A key no reader parses is left
/// out, as the purge leaves it in the map (`TRAME-248`).
#[derive(Debug, Default, PartialEq)]
pub(super) struct LineCells {
    maps: HashMap<SheetContainer, ByLine>,
}

/// The shared index a document subscription writes into.
pub(super) type LineSink = Arc<Mutex<LineCells>>;

/// One key a purged map gained or lost: its text, the pair it reads as, and
/// the value it now holds, `None` once deleted.
pub(super) struct KeyChange<'a> {
    pub(super) key: &'a str,
    pub(super) pair: Option<(Element, Element)>,
    pub(super) value: Option<LoroValue>,
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
            // A container under a cell key is no value this build writes:
            // it reads as an empty value, and the key stays indexed.
            value: value.as_ref().map(|value| match value {
                ValueOrContainer::Value(value) => value.clone(),
                ValueOrContainer::Container(_) => LoroValue::Null,
            }),
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
            match &change.value {
                Some(value) => by.insert(row, col, value.clone()),
                None => by.remove(row, col),
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
        let mut keys = Vec::new();
        for element in elements {
            match axis {
                Axis::Row => {
                    keys.extend(by.row(element).map(|(col, _)| CellKey::of(*element, *col)));
                }
                Axis::Col => keys.extend(
                    by.cols
                        .get(element)
                        .into_iter()
                        .flatten()
                        .map(|row| CellKey::of(*row, *element)),
                ),
            }
        }
        keys
    }

    /// The keys of the map `root` of `sheet` by line, with their values.
    pub(super) fn map(&self, root: SheetRoot, sheet: &Id<Sheet>) -> Option<&ByLine> {
        self.maps.get(&SheetContainer::of(root, sheet))
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
            for (key, value) in entries.iter() {
                if let Some((row, col)) = parse_cell_key(key) {
                    by.insert(row, col, value.clone());
                }
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
