//! The axis module: the one place that reads or writes the order of a sheet's
//! lines (`TRAME-275`).
//!
//! The rows and the columns of a sheet each live in a `LoroMovableList` whose
//! element value names a line. Two peers that derive one line push two
//! elements of one uid, so the merged list can hold a uid twice. The element
//! with the smallest element id is the line, and every other element of that
//! uid is hidden (`TRAME-270`). No write removes a hidden element: a formula
//! reference may be bound to it.
//!
//! Every other module of the crate sees the order through an [`AxisView`] alone.
//! A view is built here and nowhere else, from a private constructor. So code
//! outside this module cannot read the raw list and miscount a repeated uid:
//!
//! ```compile_fail,E0624
//! use ambre_trame::model::AxisView;
//! use ambre_bus::Row;
//! let _view: AxisView<Row> = AxisView::read(&loro::LoroMovableList::new());
//! ```
//!
//! The struct literal is refused for the same reason:
//!
//! ```compile_fail
//! use ambre_trame::model::AxisView;
//! use ambre_bus::Row;
//! let _view: AxisView<Row> = AxisView { elements: Default::default() };
//! ```
//!
//! A string names a container anywhere, so `scripts/crate_boundaries.py`
//! refuses an axis container name outside this file.

use std::collections::HashMap;

use ambre_bus::{CellPos, CellRect, Col, Row};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Axis, IdxVec, Len, Pos};
use loro::{LoroDoc, LoroMovableList, LoroValue};

use super::line::Line;
use super::sheet_index::{LineRead, Uid};
use crate::logic::{SheetContainer, SheetRoot};

/// The identity of one element of an axis list (`TRAME-270`).
///
/// A minted line's element is its uid. A derived line's element is a uid
/// minted from the session counter, so no two pushes share one. The order of
/// two element ids picks the visible element of a repeated uid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(Uid);

impl ElementId {
    /// The element id a minted line carries: its uid.
    #[must_use]
    pub fn of_minted(uid: Uid) -> Self {
        Self(uid)
    }

    /// The element id `id`, minted from the session counter for a derived
    /// line.
    #[must_use]
    pub fn minted(id: Uid) -> Self {
        Self(id)
    }

    /// The uid form the element id is spelled in.
    #[must_use]
    pub fn spelled(self) -> Uid {
        self.0
    }
}

/// One element of an axis list: the line it names and its own identity.
///
/// A formula reference to a line stores the same pair, so it reads `#REF!`
/// once its element is gone, whatever uid comes back (`TRAME-278`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Element {
    /// The line.
    pub uid: Uid,
    /// The element that carries it.
    pub id: ElementId,
}

impl Element {
    /// The element of a minted line, whose id is its uid.
    #[must_use]
    pub fn minted(uid: Uid) -> Self {
        Self {
            uid,
            id: ElementId::of_minted(uid),
        }
    }

    /// The element `id` of the derived line `uid`.
    #[must_use]
    pub fn derived(uid: Uid, id: ElementId) -> Self {
        Self { uid, id }
    }

    /// Whether the element names its line by its uid alone, as a minted line
    /// does.
    #[must_use]
    pub fn is_minted(&self) -> bool {
        self.id.0 == self.uid
    }

    /// The list value: the bare uid of a minted line, `"{uid}#{element}"` for a
    /// derived one, each half in the canonical spelling (`TRAME-248`).
    fn spell(&self) -> String {
        if self.is_minted() {
            self.uid.to_key_part()
        } else {
            format!("{}#{}", self.uid.to_key_part(), self.id.0.to_key_part())
        }
    }

    /// Read a list value back. `None` for any other spelling, a bare uid or a
    /// derived pair each read from its canonical halves alone (`TRAME-248`).
    fn parse(value: &str) -> Option<Self> {
        match value.split_once('#') {
            None => Uid::parse(value).map(Self::minted),
            Some((uid, id)) => {
                let element = Self::derived(Uid::parse(uid)?, ElementId(Uid::parse(id)?));
                // A pair spelling a minted element has one spelling: the bare uid.
                (!element.is_minted()).then_some(element)
            }
        }
    }
}

/// The index space of the raw list: one slot per element, hidden ones too.
#[derive(Debug, PartialEq, Eq)]
pub enum Slot {}

impl Axis for Slot {
    const NAME: &'static str = "Slot";
}

/// The deduplicated order of one axis (`TRAME-270`, `TRAME-275`).
///
/// Each uid reads as one line, at the slot of its element with the smallest
/// element id. Every other element of the uid is hidden: it holds no position,
/// and a reference bound to it reads the line all the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisView<L: Axis> {
    elements: IdxVec<Slot, Element>,
    lines: IdxVec<L, Uid>,
    shown: IdxVec<L, Pos<Slot>>,
    at: HashMap<Uid, Pos<L>>,
    owner: HashMap<ElementId, Uid>,
    read: LineRead,
}

/// The visible element of each line, and its slot, keyed by uid.
type Winners = HashMap<Uid, (ElementId, Pos<Slot>)>;

impl<L: Axis> AxisView<L> {
    /// An axis with no line.
    fn empty() -> Self {
        Self::from_elements(IdxVec::default(), LineRead::Clean)
    }

    /// The view of `list`, read in order. An element no reader can parse is
    /// logged and skipped, so the view never holds a gap (`SCHEMA-003`).
    fn read(list: &LoroMovableList) -> Self {
        let LoroValue::List(items) = list.get_value() else {
            return Self::empty();
        };
        let mut read = LineRead::Clean;
        let elements = items
            .iter()
            .enumerate()
            .filter_map(|(slot, item)| {
                let element = match item {
                    LoroValue::String(value) => Element::parse(value),
                    _ => None,
                };
                if element.is_none() {
                    tracing::error!("[AxisView] unreadable line element at slot {slot}");
                    read = LineRead::Skipped;
                }
                element
            })
            .collect();
        Self::from_elements(elements, read)
    }

    /// The view the elements `elements` make, in list order.
    fn from_elements(elements: IdxVec<Slot, Element>, read: LineRead) -> Self {
        let mut winners: Winners = HashMap::new();
        let mut owner = HashMap::with_capacity(elements.len().to_usize());
        for (slot, element) in elements.iter_enumerated() {
            owner.insert(element.id, element.uid);
            winners
                .entry(element.uid)
                .and_modify(|(id, at)| {
                    if element.id < *id {
                        *id = element.id;
                        *at = slot;
                    }
                })
                .or_insert((element.id, slot));
        }
        let mut lines = IdxVec::default();
        let mut shown = IdxVec::default();
        let mut at = HashMap::with_capacity(winners.len());
        for (slot, element) in elements.iter_enumerated() {
            if winners.get(&element.uid).map(|(_, s)| *s) == Some(slot) {
                at.insert(element.uid, lines.push(element.uid));
                shown.push(slot);
            }
        }
        Self {
            elements,
            lines,
            shown,
            at,
            owner,
            read,
        }
    }

    /// How many lines the axis shows.
    #[must_use]
    pub fn len(&self) -> Len<L> {
        self.lines.len()
    }

    /// Whether the axis shows no line.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line at `pos`, or `None` past the used range.
    #[must_use]
    pub fn uid(&self, pos: Pos<L>) -> Option<Uid> {
        self.lines.get(pos).copied()
    }

    /// Where the line `uid` sits, or `None` once it is gone.
    #[must_use]
    pub fn pos(&self, uid: &Uid) -> Option<Pos<L>> {
        self.at.get(uid).copied()
    }

    /// The visible element of the line at `pos`: the one a reference binds to.
    #[must_use]
    pub fn element(&self, pos: Pos<L>) -> Option<Element> {
        let slot = *self.shown.get(pos)?;
        self.elements.get(slot).copied()
    }

    /// Where the line of `element` sits. A live element, visible or hidden,
    /// reads the visible line of its uid. A deleted element reads `None`, even
    /// when another element now carries the uid (`TRAME-278`).
    #[must_use]
    pub fn pos_of_element(&self, element: &Element) -> Option<Pos<L>> {
        if self.owner.get(&element.id) != Some(&element.uid) {
            return None;
        }
        self.pos(&element.uid)
    }

    /// The lines in grid order.
    #[must_use]
    pub fn lines(&self) -> &IdxVec<L, Uid> {
        &self.lines
    }

    /// How many elements the list holds that show no line.
    #[must_use]
    pub fn hidden(&self) -> Len<Slot> {
        let shown = |pos: Pos<L>| self.shown.get(pos).copied();
        Len::of(
            self.elements
                .iter_enumerated()
                .filter(|(slot, e)| self.pos(&e.uid).and_then(shown) != Some(*slot))
                .count(),
        )
    }

    /// Whether the read skipped an element it could not parse.
    #[must_use]
    pub fn read_state(&self) -> LineRead {
        self.read
    }

    /// The view after `element` lands at the end of the list.
    fn pushed(&mut self, element: Element) {
        let fresh = !self.at.contains_key(&element.uid);
        let slot = self.elements.push(element);
        self.owner.insert(element.id, element.uid);
        if fresh {
            self.at.insert(element.uid, self.lines.push(element.uid));
            self.shown.push(slot);
        } else {
            // A repeat of a live uid: the smallest element id decides again.
            let elements = std::mem::take(&mut self.elements);
            *self = Self::from_elements(elements, self.read);
        }
    }

    /// The slot each element of `uid` sits at, last first, for a delete that
    /// removes every one of them (`TRAME-269`).
    fn slots_of(&self, uid: &Uid) -> Vec<Pos<Slot>> {
        let mut slots: Vec<Pos<Slot>> = self
            .elements
            .iter_enumerated()
            .filter(|(_, e)| e.uid == *uid)
            .map(|(slot, _)| slot)
            .collect();
        slots.reverse();
        slots
    }

    /// The slot an insert at `at` writes to: before the line now at `at`, or at
    /// the end of the list.
    fn insert_slot(&self, at: Pos<L>) -> Option<Pos<Slot>> {
        match self.shown.get(at) {
            Some(slot) => Some(*slot),
            None if at == self.len().as_end() => Some(self.elements.len().as_end()),
            None => None,
        }
    }
}

/// The rows and the columns of one sheet, each deduplicated, and the
/// position <-> uid cache the engine reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetIndex {
    rows: AxisView<Row>,
    cols: AxisView<Col>,
}

impl Default for SheetIndex {
    fn default() -> Self {
        Self {
            rows: AxisView::empty(),
            cols: AxisView::empty(),
        }
    }
}

impl SheetIndex {
    /// The row view.
    #[must_use]
    pub fn rows(&self) -> &AxisView<Row> {
        &self.rows
    }

    /// The column view.
    #[must_use]
    pub fn cols(&self) -> &AxisView<Col> {
        &self.cols
    }

    /// Whether the build skipped a line element it could not read
    /// (`SCHEMA-003`).
    #[must_use]
    pub fn read(&self) -> LineRead {
        self.rows.read_state().max(self.cols.read_state())
    }

    /// Number of materialized rows (the used-range high-water mark).
    #[must_use]
    pub fn row_len(&self) -> Len<Row> {
        self.rows.len()
    }

    /// Number of materialized columns.
    #[must_use]
    pub fn col_len(&self) -> Len<Col> {
        self.cols.len()
    }

    /// The rectangle of materialized cells, or `None` while an axis is empty.
    #[must_use]
    pub fn used_range(&self) -> Option<CellRect> {
        let last_row = self.row_len().last()?;
        let last_col = self.col_len().last()?;
        Some(CellRect::spanning(
            CellPos::ORIGIN,
            CellPos::new(last_row, last_col),
        ))
    }

    /// Resolve a grid position to its `(row, col)` uid pair. `None` past the
    /// used range on either axis: a virtual, never-written cell.
    #[must_use]
    pub fn resolve(&self, at: CellPos) -> Option<(Uid, Uid)> {
        Some((self.row_id(at.row)?, self.col_id(at.col)?))
    }

    /// The row at grid position `pos`, if materialized. It needs one axis
    /// alone, so it answers while the other axis is empty.
    #[must_use]
    pub fn row_id(&self, pos: Pos<Row>) -> Option<Uid> {
        self.rows.uid(pos)
    }

    /// The column at grid position `pos`, if materialized.
    #[must_use]
    pub fn col_id(&self, pos: Pos<Col>) -> Option<Uid> {
        self.cols.uid(pos)
    }

    /// The full row order, position 0..hwm.
    #[must_use]
    pub fn row_ids(&self) -> &IdxVec<Row, Uid> {
        self.rows.lines()
    }

    /// Grid position of a row, if it is materialized.
    #[must_use]
    pub fn row_pos(&self, id: &Uid) -> Option<Pos<Row>> {
        self.rows.pos(id)
    }

    /// Grid position of a column, if it is materialized.
    #[must_use]
    pub fn col_pos(&self, id: &Uid) -> Option<Pos<Col>> {
        self.cols.pos(id)
    }

    /// Grid position of a cell keyed by its `(row, col)` uid pair, if both of
    /// its lines are materialized.
    #[must_use]
    pub fn pos_of(&self, row: &Uid, col: &Uid) -> Option<CellPos> {
        Some(CellPos::new(self.row_pos(row)?, self.col_pos(col)?))
    }

    /// Whether the row `id` still exists in this sheet (`#REF!` detection).
    #[must_use]
    pub fn contains_row(&self, id: &Uid) -> bool {
        self.rows.pos(id).is_some()
    }

    /// Whether the column `id` still exists in this sheet.
    #[must_use]
    pub fn contains_col(&self, id: &Uid) -> bool {
        self.cols.pos(id).is_some()
    }

    /// Record a minted row appended at the end, with no document. Unit tests
    /// of the engine build a sheet with no `LoroDoc` through it.
    #[cfg(test)]
    pub(crate) fn push_row(&mut self, id: Uid) {
        self.rows.pushed(Element::minted(id));
    }

    /// Record a minted column appended at the end, with no document.
    #[cfg(test)]
    pub(crate) fn push_col(&mut self, id: Uid) {
        self.cols.pushed(Element::minted(id));
    }

    /// Mirror an insert of the minted row `id` at `pos`, with no document. A
    /// position past the end inserts nothing, as the list refuses it too.
    #[cfg(test)]
    pub(crate) fn apply_insert_row(&mut self, pos: Pos<Row>, id: Uid) {
        apply_insert(&mut self.rows, pos, id);
    }

    /// Mirror an insert of the minted column `id` at `pos`.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn apply_insert_col(&mut self, pos: Pos<Col>, id: Uid) {
        apply_insert(&mut self.cols, pos, id);
    }

    /// Mirror a delete of the row at `pos`, with no document. Returns the
    /// removed uid, `None` past the end.
    #[cfg(test)]
    pub(crate) fn apply_delete_row(&mut self, pos: Pos<Row>) -> Option<Uid> {
        apply_delete(&mut self.rows, pos)
    }

    /// Mirror a delete of the column at `pos`.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn apply_delete_col(&mut self, pos: Pos<Col>) -> Option<Uid> {
        apply_delete(&mut self.cols, pos)
    }

    /// Mirror a move of the row at `from` to `to`, with no document. A no-op
    /// when either position lies past the end.
    #[cfg(test)]
    pub(crate) fn apply_move_row(&mut self, from: Pos<Row>, to: Pos<Row>) {
        let view = &mut self.rows;
        let (Some(src), Some(dst)) = (view.shown.get(from).copied(), view.shown.get(to).copied())
        else {
            return;
        };
        let mut elements: Vec<Element> = view.elements.as_slice().to_vec();
        let (Some(src), Some(dst)) = (src.to_index(), dst.to_index()) else {
            return;
        };
        let moved = elements.remove(src);
        elements.insert(dst, moved);
        *view = AxisView::from_elements(elements.into(), view.read);
    }
}

/// [`SheetIndex::apply_insert_row`] on one view.
#[cfg(test)]
fn apply_insert<L: Axis>(view: &mut AxisView<L>, pos: Pos<L>, id: Uid) {
    let Some(slot) = view.insert_slot(pos).and_then(Pos::to_index) else {
        return;
    };
    let mut elements: Vec<Element> = view.elements.as_slice().to_vec();
    elements.insert(slot, Element::minted(id));
    *view = AxisView::from_elements(elements.into(), view.read);
}

/// [`SheetIndex::apply_delete_row`] on one view.
#[cfg(test)]
fn apply_delete<L: Axis>(view: &mut AxisView<L>, pos: Pos<L>) -> Option<Uid> {
    let uid = view.uid(pos)?;
    let elements: Vec<Element> = view
        .elements
        .iter()
        .filter(|e| e.uid != uid)
        .copied()
        .collect();
    *view = AxisView::from_elements(elements.into(), view.read);
    Some(uid)
}

/// The list that orders one axis, named by its line marker.
pub(crate) trait AxisOrder: Line {
    /// The container family of the list.
    #[doc(hidden)]
    fn order_root() -> SheetRoot;
    /// The same view, to update in place.
    #[doc(hidden)]
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Self>;
}

impl AxisOrder for Row {
    fn order_root() -> SheetRoot {
        SheetRoot::Rows
    }
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Row> {
        &mut index.rows
    }
}

impl AxisOrder for Col {
    fn order_root() -> SheetRoot {
        SheetRoot::Cols
    }
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Col> {
        &mut index.cols
    }
}

/// The axes of one sheet of one document: every read and write of a line
/// order goes through it.
pub(crate) struct Axes<'a> {
    doc: &'a LoroDoc,
    sheet: &'a Id<Sheet>,
}

impl<'a> Axes<'a> {
    /// The axes of `sheet` in `doc`.
    pub(crate) fn of(doc: &'a LoroDoc, sheet: &'a Id<Sheet>) -> Self {
        Self { doc, sheet }
    }

    /// The raw list of axis `L`. Private: no caller sees a repeated uid.
    fn list<L: AxisOrder>(&self) -> LoroMovableList {
        self.doc
            .get_movable_list(SheetContainer::of(L::order_root(), self.sheet).as_str())
    }

    /// The index of the sheet, both axes read from the document.
    pub(crate) fn index(&self) -> SheetIndex {
        SheetIndex {
            rows: AxisView::read(&self.list::<Row>()),
            cols: AxisView::read(&self.list::<Col>()),
        }
    }

    /// Insert the minted line `uid` before the line at `at`, or after the last
    /// line when `at` is the end. `None` when `at` lies further, or the list
    /// refuses the write.
    pub(crate) fn insert<L: AxisOrder>(&self, at: Pos<L>, uid: Uid) -> Option<()> {
        let list = self.list::<L>();
        let slot = AxisView::<L>::read(&list).insert_slot(at)?;
        list.insert(slot.to_index()?, Element::minted(uid).spell().as_str())
            .ok()
    }

    /// Delete the line at `at`: every element of its uid, the hidden ones too
    /// (`TRAME-269`). Returns the uid removed, `None` past the end.
    pub(crate) fn delete<L: AxisOrder>(&self, at: Pos<L>) -> Option<Uid> {
        let list = self.list::<L>();
        let view = AxisView::<L>::read(&list);
        let uid = view.uid(at)?;
        for slot in view.slots_of(&uid) {
            list.delete(slot.to_index()?, 1).ok()?;
        }
        Some(uid)
    }

    /// Move the line at `from` so it shows at `to`. Only the visible element
    /// moves: a hidden one shows nothing wherever it sits (`TRAME-268`).
    pub(crate) fn move_line<L: AxisOrder>(&self, from: Pos<L>, to: Pos<L>) -> Option<()> {
        let list = self.list::<L>();
        let view = AxisView::<L>::read(&list);
        let src = view.shown.get(from)?.to_index()?;
        // The moved element lands on the slot of the line now at `to`: before
        // it when moving up, after it when moving down.
        let dst = view.shown.get(to)?.to_index()?;
        list.mov(src, dst).ok()
    }

    /// Reorder the rows from `start` so they show `target`, a permutation of
    /// the lines there. Each step moves one visible element, so cells keyed by
    /// the uid follow for free. The list stays mirrored in memory, one `mov` a
    /// step, rather than read again.
    pub(crate) fn reorder_rows(&self, start: Pos<Row>, target: &[Uid]) -> Option<()> {
        let list = self.list::<Row>();
        let view = AxisView::<Row>::read(&list);
        let mut elements: Vec<Element> = view.elements.as_slice().to_vec();
        // Visibility depends on the element ids alone, never on the order.
        let visible = |e: &Element| {
            view.pos(&e.uid)
                .and_then(|pos| view.element(pos))
                .is_some_and(|shown| shown.id == e.id)
        };
        let base = start.to_index()?;
        for (offset, want) in target.iter().enumerate() {
            let shown: Vec<usize> = elements
                .iter()
                .enumerate()
                .filter(|(_, e)| visible(e))
                .map(|(slot, _)| slot)
                .collect();
            let Some(dst) = shown.get(base + offset).copied() else {
                break;
            };
            if elements.get(dst).map(|e| e.uid) == Some(*want) {
                continue;
            }
            // Earlier positions are fixed, so the wanted line sits further on.
            let Some(src) = shown
                .iter()
                .skip(base + offset)
                .copied()
                .find(|slot| elements.get(*slot).map(|e| e.uid) == Some(*want))
            else {
                continue;
            };
            list.mov(src, dst).ok()?;
            let moved = elements.remove(src);
            elements.insert(dst, moved);
        }
        Some(())
    }

    /// Append `element` to axis `L`, and record it in `index` so the caller
    /// needs no rebuild. `None` when the list refuses the write.
    pub(crate) fn push<L: AxisOrder>(
        &self,
        index: &mut SheetIndex,
        element: Element,
    ) -> Option<()> {
        self.list::<L>().push(element.spell().as_str()).ok()?;
        L::view_mut(index).pushed(element);
        Some(())
    }

    /// Empty both axes, for a sheet the workbook drops.
    pub(crate) fn clear(&self) {
        let _ = self.list::<Row>().clear();
        let _ = self.list::<Col>().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::default_sheet;

    fn uid(peer: u64, counter: u32) -> Uid {
        Uid {
            peer: ambre_core::PeerId::new(peer),
            counter: ambre_core::IdCounter::new(counter),
        }
    }

    /// A document whose row list holds `values`, raw.
    fn rows_of(values: &[String]) -> (LoroDoc, SheetIndex) {
        let doc = LoroDoc::new();
        let sheet = default_sheet();
        let list = Axes::of(&doc, &sheet).list::<Row>();
        for value in values {
            list.push(value.as_str()).unwrap();
        }
        doc.commit();
        let index = Axes::of(&doc, &sheet).index();
        (doc, index)
    }

    /// A uid the list holds twice reads as one line, at the slot of its
    /// element with the smallest element id.
    #[test]
    fn a_repeated_uid_reads_as_one_line_at_its_smallest_element() {
        let line = uid(7, 1);
        let late = Element::derived(line, ElementId::minted(uid(9, 4)));
        let early = Element::derived(line, ElementId::minted(uid(3, 8)));
        let other = Element::minted(uid(1, 0));
        let (_doc, index) = rows_of(&[late.spell(), other.spell(), early.spell()]);
        let view = index.rows();
        assert_eq!(view.len(), Len::<Row>::new(2));
        assert_eq!(view.lines().as_slice(), &[uid(1, 0), line]);
        assert_eq!(view.element(Pos::<Row>::new(1)), Some(early));
        assert_eq!(view.hidden(), Len::<Slot>::new(1));
        assert!(early.id < late.id);
        // Both elements are live, so a reference bound to either reads the line.
        assert_eq!(view.pos_of_element(&late), Some(Pos::<Row>::new(1)));
        assert_eq!(view.pos_of_element(&early), Some(Pos::<Row>::new(1)));
        // An element the list never held reads no line, whatever uid it names.
        let gone = Element::derived(line, ElementId::minted(uid(5, 5)));
        assert_eq!(view.pos_of_element(&gone), None);
    }

    /// A delete removes every element of the uid, and a move moves the
    /// visible one alone.
    #[test]
    fn a_delete_drops_every_element_and_a_move_moves_the_visible_one() {
        let line = uid(7, 1);
        let late = Element::derived(line, ElementId::minted(uid(9, 4)));
        let early = Element::derived(line, ElementId::minted(uid(3, 8)));
        let a = Element::minted(uid(1, 0));
        let b = Element::minted(uid(1, 1));
        let (doc, _) = rows_of(&[a.spell(), late.spell(), b.spell(), early.spell()]);
        let sheet = default_sheet();
        let axes = Axes::of(&doc, &sheet);
        // Shown: a, b, line. Move the line to the top.
        axes.move_line::<Row>(Pos::new(2), Pos::new(0)).unwrap();
        let index = axes.index();
        assert_eq!(index.row_ids().as_slice(), &[line, uid(1, 0), uid(1, 1)]);
        assert_eq!(index.rows().hidden(), Len::<Slot>::new(1));
        axes.delete::<Row>(Pos::new(0)).unwrap();
        let index = axes.index();
        assert_eq!(index.row_ids().as_slice(), &[uid(1, 0), uid(1, 1)]);
        assert_eq!(index.rows().hidden(), Len::<Slot>::ZERO);
    }

    /// A derived element reads back from its canonical spelling alone.
    #[test]
    fn a_derived_element_reads_back_from_its_canonical_spelling_alone() {
        let element = Element::derived(uid(0xab, 2), ElementId::minted(uid(1, 3)));
        assert_eq!(Element::parse(&element.spell()), Some(element));
        assert_eq!(element.spell().len(), 51);
        for form in [
            "00000000000000ab_00000002#1_3",
            "00000000000000ab_00000002#",
            "00000000000000ab_00000002#00000000000000ab_00000002",
        ] {
            assert_eq!(Element::parse(form), None, "{form}");
        }
    }
}
