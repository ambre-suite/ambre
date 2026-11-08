// ambre-trame/src/logic/remote.rs
// The remote apply, carried by what the import changed (`TRAME-282`)
//
// Part of the `TrameState` impl. Submodule of `logic`, so it reaches the
// private fields and helpers of the parent module through `use super::*`.
//
// A subscription notes what every import writes, container by container. The
// tail of a remote batch reads that note and carries the engine. The cells the
// batch wrote compile again. A structural edit of the sheet on screen goes
// through the carried structural step. Whatever the note cannot place falls
// back to the full reseed (`ambre-spec-trame-engine.md section 3.F`).

use crate::hash::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use ambre_core::crdt::FoundationRoot;
use ambre_core::id::{Id, Sheet};
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, EventTriggerKind, LoroDoc, Subscription};

use super::repaint::{Repaint, whole_sheet};
use super::sheets::is_formula;
use super::*;
use crate::formula::ACTIVE_SHEET;
use crate::formula::text::{CellText, StoredForm};
use crate::model::{
    AxisChange, CellKey, Element, Kept, LayoutChange, LineRead, ListAxis, SlotEdit, axis_list,
    slot_edits,
};
use ambre_core::index::{Axis, Len};

ambre_core::define_flag! {
    /// Whether the imports since the last take wrote only where the carried
    /// path can follow.
    pub(super) enum Reach {
        /// Every change landed in a container the note places.
        Placed,
        /// A change landed where the note cannot follow it. That is the sheet
        /// list, the tab order, a defined name or the schema register. A
        /// container no family names, or a nested one, counts too.
        Whole,
    }
}

/// What the imports since the last take wrote, by container.
#[derive(Debug)]
pub(super) struct Imported {
    /// The cell keys each sheet's cell map gained, changed or lost.
    cells: HashMap<Id<Sheet>, HashSet<CellKey>>,
    /// The sheets whose row or column order changed.
    axes: HashSet<Id<Sheet>>,
    /// The diffs each sheet's row list reported, one per import, in order.
    rows: HashMap<Id<Sheet>, Vec<Vec<SlotEdit>>>,
    /// The diffs each sheet's column list reported, one per import, in order.
    cols: HashMap<Id<Sheet>, Vec<Vec<SlotEdit>>>,
    /// Whether a list diff held a value no build writes, or came in another
    /// shape. The layout is then read from the lists.
    lines: LineRead,
    /// The sheets whose merge map changed.
    merges: HashSet<Id<Sheet>>,
    /// The sheets whose style map changed.
    styles: HashSet<Id<Sheet>>,
    /// The cell keys each sheet's style map gained, changed or lost.
    styled: HashMap<Id<Sheet>, HashSet<CellKey>>,
    /// The sheets whose width or height map changed.
    sizes: HashSet<Id<Sheet>>,
    /// The sheets whose filter or hidden-row map changed.
    filters: HashSet<Id<Sheet>>,
    /// Whether a cell key arrived that no build writes (`TRAME-248`).
    keys: LineRead,
    reach: Reach,
}

impl Default for Imported {
    fn default() -> Self {
        Self {
            cells: HashMap::default(),
            axes: HashSet::default(),
            rows: HashMap::default(),
            cols: HashMap::default(),
            lines: LineRead::Clean,
            merges: HashSet::default(),
            styles: HashSet::default(),
            styled: HashMap::default(),
            sizes: HashSet::default(),
            filters: HashSet::default(),
            keys: LineRead::Clean,
            reach: Reach::Placed,
        }
    }
}

ambre_core::define_flag! {
    /// Whether the subscription notes a local commit as it notes an import.
    pub(super) enum LocalNote {
        /// A local commit keeps the caches on its own path.
        Ignored,
        /// An undo or a redo runs: its commit is noted, so the tail carries
        /// the engine by what it wrote, as after a remote batch.
        Noted,
    }
}

/// What the subscription writes, and whether it notes a local commit.
#[derive(Debug, Default)]
pub(super) struct Note {
    imported: Imported,
    local: LocalNote,
}

/// The shared note a document subscription writes into.
pub(super) type ImportSink = Arc<Mutex<Note>>;

/// Subscribe to `doc`, noting into the returned sink what every import
/// writes. A local commit is noted only while an undo or a redo runs
/// ([`LocalNote`]). A checkout is never noted. The subscription lives as
/// long as the handle.
pub(super) fn watch(doc: &LoroDoc) -> (ImportSink, Subscription) {
    let sink: ImportSink = Arc::default();
    let into = Arc::clone(&sink);
    let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
        let mut held = into.lock().unwrap_or_else(PoisonError::into_inner);
        let noted = match event.triggered_by {
            EventTriggerKind::Import => true,
            EventTriggerKind::Local => held.local == LocalNote::Noted,
            EventTriggerKind::Checkout => false,
        };
        if !noted {
            return;
        }
        for change in &event.events {
            note(change, &mut held.imported);
        }
    }));
    (sink, subscription)
}

/// Place one container change in the note.
fn note(change: &ContainerDiff<'_>, imported: &mut Imported) {
    let ContainerID::Root { name, .. } = change.target else {
        // The workbook holds no nested container: a peer wrote one.
        imported.reach = Reach::Whole;
        return;
    };
    let name = name.as_str();
    // The registry entry of a peer is no content (`AUTHOR-009`).
    if name == FoundationRoot::Peers.as_str() {
        return;
    }
    let Some((root, sheet)) = SheetContainer::parse(name) else {
        imported.reach = Reach::Whole;
        return;
    };
    match root {
        SheetRoot::Cells => {
            let Diff::Map(map) = &change.diff else {
                imported.reach = Reach::Whole;
                return;
            };
            let keys = imported.cells.entry(sheet).or_default();
            for key in map.updated.keys() {
                match parse_cell_key(key) {
                    Some((row, col)) => {
                        keys.insert(CellKey::of(row, col));
                    }
                    None => imported.keys = LineRead::Skipped,
                }
            }
        }
        SheetRoot::Merges => {
            imported.merges.insert(sheet);
        }
        SheetRoot::CellStyle => {
            if let Diff::Map(map) = &change.diff {
                let keys = imported.styled.entry(sheet.clone()).or_default();
                keys.extend(
                    map.updated
                        .keys()
                        .filter_map(|key| parse_cell_key(key))
                        .map(|(row, col)| CellKey::of(row, col)),
                );
            }
            imported.styles.insert(sheet);
        }
        SheetRoot::ColWidths | SheetRoot::RowHeights => {
            imported.sizes.insert(sheet);
        }
        SheetRoot::Filters | SheetRoot::FilterHidden => {
            imported.filters.insert(sheet);
        }
        // The axis module alone names a line list (`TRAME-275`).
        other => match axis_list(other) {
            Some(axis) => note_lines(change, axis, sheet, imported),
            None => imported.reach = Reach::Whole,
        },
    }
}

/// Note one diff of a line list of `sheet`.
fn note_lines(
    change: &ContainerDiff<'_>,
    axis: ListAxis,
    sheet: Id<Sheet>,
    imported: &mut Imported,
) {
    if axis == ListAxis::Generations {
        return;
    }
    let edits = match &change.diff {
        Diff::List(diff) => slot_edits(diff),
        _ => None,
    };
    match edits {
        Some(edits) => {
            let lists = match axis {
                ListAxis::Rows => &mut imported.rows,
                ListAxis::Cols | ListAxis::Generations => &mut imported.cols,
            };
            lists.entry(sheet.clone()).or_default().push(edits);
        }
        None => imported.lines = LineRead::Skipped,
    }
    imported.axes.insert(sheet);
}

/// The path a remote batch takes, for the test that holds the carried engine
/// against a full rebuild (`TRAME-282`).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemotePath {
    /// The engine carried by what the batch wrote.
    Carry,
    /// The reference model: a full reseed after every batch.
    Reseed,
}

ambre_core::define_flag! {
    /// When the carried step compiles again the formulas that read positions.
    pub(super) enum Readers {
        /// When the batch moved a line of the sheet on screen.
        OnMove,
        /// Always: an undo or a redo. A local write that adds a line leaves
        /// a formula reading a position the line now holds, which a reseed
        /// would bind to the line. The history step binds it too, so the
        /// engine equals a reseed after it (`TRAME-285`).
        Always,
    }
}

/// What the engine does once a remote batch merged, and the caches are
/// rebuilt.
struct RemoteStep {
    /// The engine cells of the lines the batch removed.
    removed: HashSet<CellId>,
    /// The formulas a structural edit of the batch moves, each with its text.
    reread: Vec<(CellId, CellText<StoredForm>)>,
    /// The cells the batch wrote, each with the text it now shows.
    written: Vec<(CellId, CellText<StoredForm>)>,
    /// The layout after the batch of every sheet but the active one.
    others: HashMap<Pos<EngineSheet>, SheetIndex>,
    /// The stored keys whose formula now places on no cell, by sheet.
    orphaned: Vec<(Id<Sheet>, CellKey)>,
    /// The stored keys that no longer hold a formula placing on no cell.
    placed: Vec<(Id<Sheet>, CellKey)>,
}

impl TrameState {
    /// Take what the imports since the last call wrote, the note cleared.
    pub(super) fn take_imported(&self) -> Imported {
        std::mem::take(
            &mut self
                .imported
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .imported,
        )
    }

    /// Note, or stop noting, the local commits, for an undo or a redo.
    pub(super) fn note_local(&self, local: LocalNote) {
        self.imported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .local = local;
    }

    /// Whether the engine may carry a batch that wrote `imported`, judged on
    /// the state before any cache is rebuilt. The full reseed answers every
    /// other batch:
    ///
    /// * a change the note cannot place ([`Reach::Whole`]);
    /// * an engine a structural edit cannot carry either: a formula refused
    ///   for a cycle, or a cell the last reseed left out (`TRAME-280`);
    /// * a line order or a merge map of a sheet off the screen. Only the
    ///   active sheet keeps its layout before the batch, which the carried
    ///   step reads.
    pub(super) fn carries(&self, imported: &Imported) -> bool {
        #[cfg(test)]
        if self.remote_path == RemotePath::Reseed {
            return false;
        }
        imported.reach == Reach::Placed
            && !self.engine_holds_a_skip()
            && imported
                .axes
                .iter()
                .chain(&imported.merges)
                .all(|sheet| *sheet == self.active_sheet)
    }

    /// The tail of a remote batch the engine carries. It rebuilds the caches
    /// the batch reached, runs the engine step, and answers with the events
    /// of a full reseed. The repaint names the cells the batch changed, or the
    /// whole window when the engine reseeded after all (`TRAME-167`). `imported` is what the batch wrote, and
    /// [`carries`](Self::carries) accepted it.
    pub(super) fn carry_after_merge(&mut self, imported: &Imported) -> Vec<TrameEvent> {
        self.adopt_imported_cursor();
        let (conflicts, repaint) = self.carry_imported(imported, Readers::OnMove);
        self.merge_events(conflicts, repaint)
    }

    /// Rebuild the caches `imported` reached and run the engine step, with
    /// no event built. Returns the merge conflicts and the repaint. A remote
    /// batch and an undo or a redo share it (`TRAME-285`).
    pub(super) fn carry_imported(
        &mut self,
        imported: &Imported,
        readers: Readers,
    ) -> (Vec<TrameEvent>, Repaint) {
        let active = self.active_sheet.clone();
        let moved = imported.axes.contains(&active);
        let layout = if moved {
            self.adopt_batch_layout(imported)
        } else {
            BatchLayout::Kept
        };
        if imported.keys == LineRead::Skipped {
            self.note_unreadable_key();
        }
        // A covered cell a peer wrote is masked: the merges always rebuild,
        // and a masked cell leaves the engine, which only a reseed does.
        let merges_before = self.merges.clone();
        let conflicts = self.rebuild_merges();
        let change = match &layout {
            BatchLayout::Kept => Some(None),
            BatchLayout::Moved(change) => Some(Some(change)),
            BatchLayout::Read => None,
        };
        let step = change
            .filter(|_| self.masked_cells.is_empty())
            .and_then(|change| self.remote_step(imported, change, readers));
        let mut repaint = match step {
            Some(step) => self.carry_remote(step),
            None => {
                self.reseed_engine();
                Repaint::Whole
            }
        };
        repaint = self.take_engine_changes(repaint);
        // A moved line moves every cell after it, and a size, a filter or a
        // merge moves the layout of the whole sheet.
        if let BatchLayout::Moved(change) = &layout {
            repaint = moved_cells(change, &self.index)
                .into_iter()
                .fold(repaint, Repaint::with_rect);
        }
        if [&imported.sizes, &imported.filters, &imported.merges]
            .iter()
            .any(|sheets| sheets.contains(&active))
        {
            repaint = repaint.with_rect(whole_sheet());
        }
        // A merge whose corner line moved or went changes its span, though
        // the merge map is untouched: each block that differs repaints.
        repaint = merges_before
            .iter()
            .filter(|(master, end)| self.merges.get(master) != Some(end))
            .chain(
                self.merges
                    .iter()
                    .filter(|(master, end)| merges_before.get(master) != Some(end)),
            )
            .map(|(master, end)| CellRect::spanning(*master, *end))
            .fold(repaint, Repaint::with_rect);
        if let Some(keys) = imported.styled.get(&active) {
            repaint = keys
                .iter()
                .filter_map(|key| key.place_in(&self.index))
                .map(|(at, _)| at)
                .fold(repaint, Repaint::with);
        }
        if moved || imported.styles.contains(&active) {
            self.reseed_styles();
        }
        if moved || imported.sizes.contains(&active) {
            self.reseed_metrics();
        }
        if moved || imported.filters.contains(&active) {
            self.rebuild_filter();
        }
        (conflicts, repaint)
    }

    /// Take the active sheet's layout after the batch as the position cache.
    /// It derives from the list diffs the batch reported, with no read of the
    /// lists ([`SheetIndex::apply`]). It is read from the lists when only a
    /// read can tell. A line the batch removed and added again may sit on
    /// another element: only a reseed then says which keys read where.
    fn adopt_batch_layout(&mut self, imported: &Imported) -> BatchLayout {
        let none: Vec<Vec<SlotEdit>> = Vec::new();
        let rows = imported.rows.get(&self.active_sheet).unwrap_or(&none);
        let cols = imported.cols.get(&self.active_sheet).unwrap_or(&none);
        if imported.lines == LineRead::Clean
            && let Some(change) = self.index.apply(rows, cols)
        {
            // Each test checks the derived layout against a read of the
            // lists, as the structural path does.
            #[cfg(test)]
            assert_eq!(
                self.index,
                self.axes().index(),
                "the derived layout is the lists'"
            );
            if change.kept() == Kept::Same {
                return BatchLayout::Moved(change);
            }
            return BatchLayout::Read;
        }
        self.rebuild_index();
        BatchLayout::Read
    }

    /// What the engine does after the batch, or `None` when only a reseed can
    /// say. `change` is what the batch did to the active sheet's lines, when
    /// it moved them. Each line it removed and added again kept its element.
    ///
    /// A cell shown under a second key is one no dependency names, as a
    /// reseed would leave it out (`TRAME-279`). So is a masked cell of a sheet
    /// off the screen. Both answer `None`.
    fn remote_step(
        &self,
        imported: &Imported,
        change: Option<&LayoutChange>,
        readers: Readers,
    ) -> Option<RemoteStep> {
        let others = self.other_engine_indices();
        let index_of = |sheet: Pos<EngineSheet>| {
            if sheet == ACTIVE_SHEET {
                Some(&self.index)
            } else {
                others.get(&sheet)
            }
        };
        let removed =
            change.map_or_else(HashSet::default, |change| gone_cells(change, &self.index));
        let mut written: HashMap<CellId, CellText<StoredForm>> = HashMap::default();
        let mut orphaned = Vec::new();
        let mut placed = Vec::new();
        for (sheet, keys) in &imported.cells {
            // A sheet the strip does not carry is fed by no reseed either.
            let Some(&at_sheet) = self.engine_sheet_by_id.get(sheet) else {
                continue;
            };
            let index = index_of(at_sheet)?;
            if *sheet != self.active_sheet && !self.masked_cells_for(sheet, index).is_empty() {
                return None;
            }
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
            for key in keys {
                // A key whose line is gone reads nowhere, and a structural
                // edit still walks its formula (`TRAME-280`).
                let Some((at, _)) = key.place_in(index) else {
                    let entry = (sheet.clone(), key.clone());
                    if map
                        .get(key.as_str())
                        .is_some_and(|value| is_formula(&value.get_deep_value()))
                    {
                        orphaned.push(entry);
                    } else {
                        placed.push(entry);
                    }
                    continue;
                };
                placed.push((sheet.clone(), key.clone()));
                let (row, col) = index.resolve(at)?;
                let text = shown_text(&map, index, at)?;
                written.insert(CellId::bound(at_sheet, row, col), text);
            }
        }
        if let Some(change) = change {
            let active = &self.active_sheet;
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, active).as_str());
            // A line back on its element places its keys again.
            if let Some(keys) = self.unplaced.get(active) {
                placed.extend(
                    keys.iter()
                        .filter(|key| key.place_in(&self.index).is_some())
                        .map(|key| (active.clone(), key.clone())),
                );
            }
            // A key under a removed element that the batch names nowhere,
            // as one a peer wrote while the line went, places on no cell.
            orphaned.extend(
                left_under(change, &self.index)
                    .into_iter()
                    .filter(|key| {
                        map.get(key.as_str())
                            .is_some_and(|value| is_formula(&value.get_deep_value()))
                    })
                    .map(|key| (active.clone(), key)),
            );
            // An undo of a delete brings a line back on its own element. A key
            // written under it while it was gone reads again, and the batch
            // names no such key. So each cell an added line crosses is read.
            let map = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &self.active_sheet).as_str());
            for cell in added_cells(change, &self.index) {
                if written.contains_key(&cell) {
                    continue;
                }
                let (Some(row), Some(col)) = (cell.row_uid(), cell.col_uid()) else {
                    continue;
                };
                let Some(at) = self.index.pos_of(&row, &col) else {
                    continue;
                };
                let text = shown_text(&map, &self.index, at)?;
                if !text.is_empty() {
                    written.insert(cell, text);
                }
            }
        }
        let mut reread = Vec::new();
        if change.is_some() || readers == Readers::Always {
            for cell in self.engine.structural_reach(&removed) {
                // A cell the batch also wrote may hold the text it compiled
                // from, over lines the batch moved or removed. It compiles
                // again all the same, from the text it now shows.
                if let Some(text) = written.remove(&cell) {
                    reread.push((cell, text));
                    continue;
                }
                let Some(index) = index_of(cell.sheet) else {
                    continue;
                };
                let (Some(row), Some(col)) = (cell.row_uid(), cell.col_uid()) else {
                    continue;
                };
                let Some(at) = index.pos_of(&row, &col) else {
                    continue;
                };
                let Some(sheet) = self.sheets_by_idx.get(cell.sheet.to_usize()) else {
                    continue;
                };
                let map = self
                    .doc
                    .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str());
                reread.push((cell, shown_text(&map, index, at)?));
            }
        }
        Some(RemoteStep {
            removed,
            reread,
            written: written.into_iter().collect(),
            others,
            orphaned,
            placed,
        })
    }

    /// The layout of every sheet but the active one, keyed by engine index,
    /// as [`engine_indices`](Self::engine_indices) builds them.
    pub(super) fn other_engine_indices(&self) -> HashMap<Pos<EngineSheet>, SheetIndex> {
        self.sheets_by_idx
            .iter()
            .enumerate()
            .skip(1)
            .map(|(idx, id)| {
                (
                    Pos::<EngineSheet>::from_usize(idx),
                    self.build_sheet_index(id),
                )
            })
            .collect()
    }

    /// Run the engine step: clear what went, compile again what the batch
    /// moved or wrote, and recompute the dirty closure. The active sheet's
    /// layout is lent to the engine for the call, not copied. A step that
    /// meets a cycle reseeds after all, so the feed order decides which cell
    /// of it is refused.
    fn carry_remote(&mut self, step: RemoteStep) -> Repaint {
        for (sheet, key) in &step.placed {
            if let Some(keys) = self.unplaced.get_mut(sheet) {
                keys.remove(key);
            }
        }
        for (sheet, key) in step.orphaned {
            self.unplaced.entry(sheet).or_default().insert(key);
        }
        self.unplaced.retain(|_, keys| !keys.is_empty());
        let mut indices = step.others;
        indices.insert(ACTIVE_SHEET, std::mem::take(&mut self.index));
        let ctx = TrameCtx {
            engine_sheet_by_name: &self.engine_sheet_by_name,
            engine_sheet_by_id: &self.engine_sheet_by_id,
            named: &self.named,
        };
        let carried =
            self.engine
                .apply_remote(&step.removed, &step.reread, &step.written, &indices, &ctx);
        if let Some(index) = indices.remove(&ACTIVE_SHEET) {
            self.index = index;
        }
        if carried == crate::formula::engine::Carry::CycleMet {
            self.reseed_engine();
            return Repaint::Whole;
        }
        Repaint::Cells(None)
    }
}

/// The cells a layout change moved. A row edit moves the rows from the first
/// one it reached to the last row of the longer layout. It moves them across
/// the columns either layout used. A column edit moves the columns the same
/// way. A cell past both layouts is empty before and after.
fn moved_cells(change: &LayoutChange, index: &SheetIndex) -> Vec<CellRect> {
    // The longer of the two layouts of one axis.
    fn longest<L: Axis>(now: Len<L>, axis: &AxisChange<L>) -> Len<L> {
        now.max(now.saturating_sub(Len::<L>::of(axis.added.len())) + Len::<L>::of(axis.gone.len()))
    }
    let (Some(last_row), Some(last_col)) = (
        longest(index.row_len(), &change.rows).last(),
        longest(index.col_len(), &change.cols).last(),
    ) else {
        return Vec::new();
    };
    let corner = CellPos::new(last_row, last_col);
    let from_row = change
        .rows
        .first
        .filter(|row| *row <= last_row)
        .map(|row| CellRect::spanning(CellPos::new(row, Pos::<Col>::ZERO), corner));
    let from_col = change
        .cols
        .first
        .filter(|col| *col <= last_col)
        .map(|col| CellRect::spanning(CellPos::new(Pos::<Row>::ZERO, col), corner));
    from_row.into_iter().chain(from_col).collect()
}

/// What a batch did to the active sheet's lines.
enum BatchLayout {
    /// The batch left them as they were.
    Kept,
    /// The batch moved them, and each line it removed and added again kept
    /// its element.
    Moved(LayoutChange),
    /// The layout was read from the lists, and only a reseed carries the
    /// engine across it.
    Read,
}

/// The engine cells of the active sheet on the lines `change` removed. Each
/// gone line is crossed with every line of the other axis before the batch.
/// That axis held what `index` shows, less what the batch added, plus what it
/// removed.
fn gone_cells(change: &LayoutChange, index: &SheetIndex) -> HashSet<CellId> {
    fn before<L: Axis>(now: &[Uid], axis: &AxisChange<L>) -> Vec<Uid> {
        let added: HashSet<&Uid> = axis.added.iter().collect();
        now.iter()
            .filter(|uid| !added.contains(uid))
            .chain(&axis.gone)
            .copied()
            .collect()
    }
    let mut cells = HashSet::default();
    if !change.rows.gone.is_empty() {
        let cols = before(index.cols().lines().as_slice(), &change.cols);
        for &row in &change.rows.gone {
            cells.extend(
                cols.iter()
                    .map(|&col| CellId::bound(ACTIVE_SHEET, row, col)),
            );
        }
    }
    if !change.cols.gone.is_empty() {
        let rows = before(index.rows().lines().as_slice(), &change.rows);
        for &col in &change.cols.gone {
            cells.extend(
                rows.iter()
                    .map(|&row| CellId::bound(ACTIVE_SHEET, row, col)),
            );
        }
    }
    cells
}

/// The engine cells of the active sheet that a line `change` added crosses.
/// Each added row meets every column `index` shows, and each added column
/// every other row.
fn added_cells(change: &LayoutChange, index: &SheetIndex) -> Vec<CellId> {
    let rows = index.rows().lines();
    let cols = index.cols().lines();
    let mut cells = Vec::new();
    for &row in &change.rows.added {
        cells.extend(
            cols.iter()
                .map(|&col| CellId::bound(ACTIVE_SHEET, row, col)),
        );
    }
    let added_rows: HashSet<&Uid> = change.rows.added.iter().collect();
    for &col in &change.cols.added {
        cells.extend(
            rows.iter()
                .filter(|row| !added_rows.contains(row))
                .map(|&row| CellId::bound(ACTIVE_SHEET, row, col)),
        );
    }
    cells
}

/// The keys a cell under an element `change` removed may sit under. Each
/// removed element of one axis meets each element of the other, live or
/// removed by the same batch.
fn left_under(change: &LayoutChange, index: &SheetIndex) -> Vec<CellKey> {
    let mut keys = Vec::new();
    // Each side is listed only when the other axis lost an element. A list
    // of every row costs one read per row of the sheet.
    if !change.rows.removed.is_empty() {
        let cols: Vec<Element> = index
            .cols()
            .lines()
            .iter_enumerated()
            .flat_map(|(pos, _)| index.cols().elements_at(pos))
            .chain(change.cols.removed.iter().copied())
            .collect();
        for &row in &change.rows.removed {
            keys.extend(cols.iter().map(|&col| CellKey::of(row, col)));
        }
    }
    if !change.cols.removed.is_empty() {
        let rows = index
            .rows()
            .lines()
            .iter_enumerated()
            .flat_map(|(pos, _)| index.rows().elements_at(pos));
        for row in rows {
            keys.extend(change.cols.removed.iter().map(|&col| CellKey::of(row, col)));
        }
    }
    keys
}

/// The text the cell at `at` shows: what its one stored key holds, or the
/// empty text when no key holds a string. `None` when two keys sit on the
/// cell, which a reseed leaves out of the engine (`TRAME-279`).
fn shown_text(
    map: &loro::LoroMap,
    index: &SheetIndex,
    at: CellPos,
) -> Option<CellText<StoredForm>> {
    let mut held = index
        .keys_at(at)
        .into_iter()
        .filter_map(|key| map.get(key.as_str()));
    let first = held.next();
    if held.next().is_some() {
        return None;
    }
    let text = match first.map(|value| value.get_deep_value()) {
        Some(LoroValue::String(text)) => text.to_string(),
        _ => String::new(),
    };
    Some(CellText::<StoredForm>::new(text))
}
