// ambre-trame/src/logic/line_cells.rs
// The cells of each line, kept by a subscription to the document (`TRAME-081`)
//
// A delete purges every cell and style keyed by an element of the removed
// line. A walk of the whole cell map finds them at a cost that grows with the
// sheet. This index answers the same question from the line alone. A
// subscription keeps it from every event of the document: a local commit, an
// import, an undo and a checkout alike. No write path updates it by hand, so
// none can forget to.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::id::{Id, Sheet};
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, LoroDoc, Subscription};

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

/// Subscribe to `doc`, keeping in the returned sink the keys of each purged
/// map by line. The subscription lives as long as the handle.
///
/// Loro emits the events of a commit, an import or a checkout before the call
/// returns. A write not yet committed is therefore absent from the index. A
/// purge runs at the start of its command, after the commit of every earlier
/// one, so that write is never one it must find.
pub(super) fn watch(doc: &LoroDoc) -> (LineSink, Subscription) {
    let sink: LineSink = Arc::default();
    let into = Arc::clone(&sink);
    let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
        let mut lines = into.lock().unwrap_or_else(PoisonError::into_inner);
        for change in &event.events {
            lines.note(change);
        }
    }));
    (sink, subscription)
}

impl LineCells {
    /// Apply one container change of the document.
    fn note(&mut self, change: &ContainerDiff<'_>) {
        let ContainerID::Root { name, .. } = change.target else {
            return;
        };
        let Some((root, sheet)) = SheetContainer::parse(name.as_str()) else {
            return;
        };
        if !PURGED.contains(&root) {
            return;
        }
        let Diff::Map(map) = &change.diff else {
            return;
        };
        let by = self
            .maps
            .entry(SheetContainer::of(root, &sheet))
            .or_default();
        for (key, value) in &map.updated {
            let Some((row, col)) = parse_cell_key(key) else {
                continue;
            };
            match value {
                Some(_) => by.insert(row, col),
                None => by.remove(row, col),
            }
        }
        if by.rows.is_empty() {
            self.maps.remove(&SheetContainer::of(root, &sheet));
        }
    }

    /// The keys the map `root` of `sheet` holds under `element` of `axis`.
    pub(super) fn keys_on(
        &self,
        root: SheetRoot,
        sheet: &Id<Sheet>,
        axis: Axis,
        element: &Element,
    ) -> Vec<CellKey> {
        let Some(by) = self.maps.get(&SheetContainer::of(root, sheet)) else {
            return Vec::new();
        };
        let lines = match axis {
            Axis::Row => &by.rows,
            Axis::Col => &by.cols,
        };
        lines
            .get(element)
            .into_iter()
            .flatten()
            .map(|other| match axis {
                Axis::Row => CellKey::of(*element, *other),
                Axis::Col => CellKey::of(*other, *element),
            })
            .collect()
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
