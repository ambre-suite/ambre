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
//! let _view: AxisView<Row> =
//!     AxisView::read_hiding(&loro::LoroMovableList::new(), Default::default());
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

use crate::hash::{HashMap, HashSet};

use ambre_bus::{CellPos, CellRect, Col, Row};
use ambre_core::id::{Id, Sheet};
use ambre_core::index::{Axis, IdxVec, Len, Pos, Span};
use ambre_core::{OpenRange, Version, VersionKind};
use blake2::{Blake2b512, Digest};
use loro::{LoroDoc, LoroMap, LoroMovableList, LoroValue};

use super::line::Line;
use super::sheet_index::{CellKey, KEY_PART_LEN, LineRead, Uid, parse_cell_key};
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
    pub(crate) fn spell(&self) -> String {
        let mut out = String::with_capacity(51);
        self.push_spelling(&mut out);
        out
    }

    /// Append the list value to `out`, as [`spell`](Self::spell) writes it.
    pub(crate) fn push_spelling(&self, out: &mut String) {
        self.uid.push_key_part(out);
        if !self.is_minted() {
            out.push('#');
            self.id.0.push_key_part(out);
        }
    }

    /// Read a list value back. `None` for any other spelling, a bare uid or a
    /// derived pair each read from its canonical halves alone (`TRAME-248`).
    pub(crate) fn parse(value: &str) -> Option<Self> {
        // A spelling is one half or two halves around `#`, each of a fixed
        // length: the length picks the shape, with no search for `#`.
        if value.len() == KEY_PART_LEN {
            return Uid::parse(value).map(Self::minted);
        }
        if value.as_bytes().get(KEY_PART_LEN) != Some(&b'#') {
            return None;
        }
        match value.split_at_checked(KEY_PART_LEN) {
            None => None,
            Some((uid, id)) => {
                let id = id.get(1..)?;
                let element = Self::derived(Uid::parse(uid)?, ElementId(Uid::parse(id)?));
                // A pair spelling a minted element has one spelling: the bare uid.
                (!element.is_minted()).then_some(element)
            }
        }
    }
}

/// The unit of a count of hiding registers that name one element.
#[derive(Debug, PartialEq, Eq)]
pub enum Register {}

impl Axis for Register {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("Register");
}

/// The key of one hiding register of `line_hidden_{sheet}`
/// (`ambre-spec-trame-schema.md section 3`): the axis, the session that hid
/// the lines, and the first element it hides. No two live registers of one
/// session hide one element, so no two writers spell one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HidingKey {
    axis: ListAxis,
    peer: ambre_core::PeerId,
    first: Element,
}

impl HidingKey {
    /// The key a delete of axis `L` by the session `peer` writes, whose
    /// first hidden element is `first`.
    fn of<L: AxisOrder>(peer: ambre_core::PeerId, first: Element) -> Self {
        Self {
            axis: L::LIST,
            peer,
            first,
        }
    }

    /// The session that wrote the register (`TRAME-302`).
    #[must_use]
    pub fn peer(&self) -> ambre_core::PeerId {
        self.peer
    }

    /// The map key: `"{axis}:{peer:016x}:{first element}"`.
    pub(crate) fn spell(&self) -> String {
        let root = match self.axis {
            ListAxis::Cols => <Col as AxisOrder>::ROOT_KEY,
            ListAxis::Rows | ListAxis::Generations | ListAxis::Hidden => {
                <Row as AxisOrder>::ROOT_KEY
            }
        };
        let mut out = format!("{root}:{:016x}:", self.peer.get());
        self.first.push_spelling(&mut out);
        out
    }

    /// Read a map key back. `None` for any other spelling than the one
    /// [`spell`](Self::spell) writes.
    pub(crate) fn parse(key: &str) -> Option<Self> {
        let (root, rest) = key.split_once(':')?;
        let axis = if root == <Row as AxisOrder>::ROOT_KEY {
            ListAxis::Rows
        } else if root == <Col as AxisOrder>::ROOT_KEY {
            ListAxis::Cols
        } else {
            return None;
        };
        let (peer, first) = rest.split_once(':')?;
        if peer.len() != 16 {
            return None;
        }
        let parsed = Self {
            axis,
            peer: ambre_core::PeerId::new(u64::from_str_radix(peer, 16).ok()?),
            first: Element::parse(first)?,
        };
        (parsed.spell() == key).then_some(parsed)
    }
}

/// The value of a hiding register: the spelling of each element it hides,
/// one per line, in list order.
pub(crate) fn spell_register(elements: &[Element]) -> Vec<u8> {
    let mut out = String::with_capacity(elements.len() * 52);
    for (n, element) in elements.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        element.push_spelling(&mut out);
    }
    out.into_bytes()
}

/// The elements a register value names. `None` for a value no build writes.
pub(crate) fn parse_register(value: &[u8]) -> Option<Vec<Element>> {
    std::str::from_utf8(value)
        .ok()?
        .split('\n')
        .map(Element::parse)
        .collect()
}

/// One hiding register a change wrote (`Some`) or deleted (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HiddenEdit {
    pub(crate) key: HidingKey,
    pub(crate) elements: Option<Vec<Element>>,
}

/// The index space of the raw list: one slot per element, hidden ones too.
#[derive(Debug, PartialEq, Eq)]
pub enum Slot {}

impl Axis for Slot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Slot");
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
    /// The hidden elements of each repeated uid, smallest id first.
    hidden_of: HashMap<Uid, Vec<Element>>,
    read: LineRead,
    /// How many hiding registers name each element. An element a register
    /// names shows no line, and a reference bound to it reads `#REF!`, until
    /// the register goes (`TRAME-081`, `TRAME-303`).
    concealed: HashMap<ElementId, Len<Register>>,
    /// The elements each hiding register of this axis names.
    registers: HashMap<HidingKey, Vec<Element>>,
}

/// The visible element of each line, and its slot, keyed by uid.
type Winners = HashMap<Uid, (ElementId, Pos<Slot>)>;

impl<L: Axis> AxisView<L> {
    /// An axis with no line.
    fn empty() -> Self {
        Self::build(IdxVec::default(), LineRead::Clean, HashMap::default())
    }

    /// The view of `list`, read in order, less every element one of
    /// `registers` names. An element no reader can parse is logged and
    /// skipped, so the view never holds a gap (`SCHEMA-003`).
    fn read_hiding(list: &LoroMovableList, registers: HashMap<HidingKey, Vec<Element>>) -> Self {
        let LoroValue::List(items) = list.get_value() else {
            return Self::build(IdxVec::default(), LineRead::Clean, registers);
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
        Self::build(elements, read, registers)
    }

    /// The view the elements `elements` make, in list order, less every
    /// element one of `registers` names.
    fn build(
        elements: IdxVec<Slot, Element>,
        read: LineRead,
        registers: HashMap<HidingKey, Vec<Element>>,
    ) -> Self {
        let mut concealed: HashMap<ElementId, Len<Register>> = HashMap::default();
        for element in registers.values().flatten() {
            *concealed.entry(element.id).or_default() += Len::ONE;
        }
        let mut winners: Winners = HashMap::default();
        let mut owner =
            HashMap::with_capacity_and_hasher(elements.len().to_usize(), Default::default());
        for (slot, element) in elements.iter_enumerated() {
            if concealed.contains_key(&element.id) {
                continue;
            }
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
        let mut at = HashMap::with_capacity_and_hasher(winners.len(), Default::default());
        let mut hidden_of: HashMap<Uid, Vec<Element>> = HashMap::default();
        for (slot, element) in elements.iter_enumerated() {
            if concealed.contains_key(&element.id) {
                continue;
            }
            if winners.get(&element.uid).map(|(_, s)| *s) == Some(slot) {
                at.insert(element.uid, lines.push(element.uid));
                shown.push(slot);
            } else {
                hidden_of.entry(element.uid).or_default().push(*element);
            }
        }
        for hidden in hidden_of.values_mut() {
            hidden.sort_unstable_by_key(|e| e.id);
        }
        Self {
            elements,
            lines,
            shown,
            at,
            owner,
            hidden_of,
            read,
            concealed,
            registers,
        }
    }

    /// The same view built again from its elements and its registers.
    fn rebuilt(&mut self) {
        let elements = std::mem::take(&mut self.elements);
        let registers = std::mem::take(&mut self.registers);
        *self = Self::build(elements, self.read, registers);
    }

    /// Whether a hiding register names the element `id`.
    fn is_concealed(&self, id: &ElementId) -> bool {
        !self.concealed.is_empty() && self.concealed.contains_key(id)
    }

    /// Hide the elements `elements` under the register `key`, with no read
    /// of the list (`TRAME-303`). Returns the lines that went. A line whose
    /// uid sits on another live element takes the full rebuild.
    fn conceal(&mut self, key: HidingKey, elements: Vec<Element>) -> AxisChange<L> {
        let mut fresh = Vec::new();
        for element in &elements {
            let n = self.concealed.entry(element.id).or_default();
            if *n == Len::ZERO {
                fresh.push(*element);
            }
            *n += Len::ONE;
        }
        self.registers.insert(key, elements);
        if self.read == LineRead::Skipped
            || fresh.iter().any(|e| self.hidden_of.contains_key(&e.uid))
        {
            return self.recompute();
        }
        let mut gone: Vec<(Pos<L>, Uid)> = Vec::new();
        for element in &fresh {
            if self.owner.remove(&element.id).is_some()
                && let Some(pos) = self.at.remove(&element.uid)
            {
                gone.push((pos, element.uid));
            }
        }
        let mut change = AxisChange::untouched();
        let Some(first) = gone.iter().map(|(pos, _)| *pos).min() else {
            return change;
        };
        gone.sort_unstable_by_key(|(pos, _)| *pos);
        // One merge walk of the lines from the first one that goes: the
        // positions that go are sorted, so no line costs a lookup.
        let mut skip = gone.iter().map(|(pos, _)| *pos).peekable();
        let mut lines = Vec::with_capacity(self.lines.len().to_usize());
        let mut shown = Vec::with_capacity(self.lines.len().to_usize());
        for ((pos, uid), slot) in self.lines.iter_enumerated().zip(self.shown.iter()) {
            if skip.peek() == Some(&pos) {
                skip.next();
                continue;
            }
            lines.push(*uid);
            shown.push(*slot);
        }
        self.lines = lines.into();
        self.shown = shown.into();
        self.renumber_from(first);
        change.gone = gone.into_iter().map(|(_, uid)| uid).collect();
        change.first = Some(first);
        change
    }

    /// Drop the register `key`, and show again each element no other
    /// register names, with no read of the list (`TRAME-303`). Returns the
    /// lines that came back, and `None` for a key the view does not hold.
    fn unconceal(&mut self, key: &HidingKey) -> Option<AxisChange<L>> {
        let elements = self.registers.remove(key)?;
        let mut back: HashSet<ElementId> = HashSet::default();
        for element in &elements {
            if let Some(n) = self.concealed.get_mut(&element.id) {
                *n = n.saturating_sub(Len::ONE);
                if *n == Len::ZERO {
                    self.concealed.remove(&element.id);
                    back.insert(element.id);
                }
            }
        }
        // A uid that comes back on two elements, or that another element
        // already shows, takes the rule of the smallest element id again.
        let mut uids: HashSet<Uid> = HashSet::default();
        if self.read == LineRead::Skipped
            || elements.iter().any(|e| {
                back.contains(&e.id) && (self.at.contains_key(&e.uid) || !uids.insert(e.uid))
            })
        {
            return Some(self.recompute());
        }
        let mut change = AxisChange::untouched();
        // One walk of the slots: each element no register names is a line,
        // and the first one that comes back is where the positions move.
        let mut lines = IdxVec::default();
        let mut shown = IdxVec::default();
        let mut first = None;
        for (slot, element) in self.elements.iter_enumerated() {
            if !self.concealed.is_empty() && self.concealed.contains_key(&element.id) {
                continue;
            }
            let pos = lines.push(element.uid);
            shown.push(slot);
            if back.contains(&element.id) {
                self.owner.insert(element.id, element.uid);
                change.added.push(element.uid);
                first.get_or_insert(pos);
            }
        }
        self.lines = lines;
        self.shown = shown;
        if let Some(first) = first {
            self.renumber_from(first);
        }
        change.first = first;
        Some(change)
    }

    /// The view built again from its elements and its registers, and the
    /// lines that went and came back.
    fn recompute(&mut self) -> AxisChange<L> {
        let before: HashSet<Uid> = self.lines.iter().copied().collect();
        let old = self.lines.clone();
        self.rebuilt();
        let after: HashSet<Uid> = self.lines.iter().copied().collect();
        let mut change = AxisChange::untouched();
        change.gone = old.iter().filter(|u| !after.contains(u)).copied().collect();
        change.added = self
            .lines
            .iter()
            .filter(|u| !before.contains(u))
            .copied()
            .collect();
        let differs = old
            .iter()
            .zip(self.lines.iter())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| old.len().to_usize().min(self.lines.len().to_usize()));
        change.first = Pos::<L>::from_index(differs);
        if old == self.lines {
            change.first = None;
        }
        change
    }

    /// Take one register a change wrote or deleted. A write over a key the
    /// view holds drops the old register first.
    fn hidden_edit(&mut self, edit: &HiddenEdit) -> AxisChange<L> {
        let mut change = AxisChange::untouched();
        if let Some(dropped) = self.unconceal(&edit.key) {
            change.fold(dropped);
        }
        if let Some(elements) = &edit.elements {
            let hidden = self.conceal(edit.key, elements.clone());
            change.fold(hidden);
        }
        change
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

    /// Every live element of the line at `pos`: the visible one first, then
    /// the hidden ones, smallest id first. A cell of the line may sit under
    /// any of them (`TRAME-279`).
    pub fn elements_at(&self, pos: Pos<L>) -> impl Iterator<Item = Element> + '_ {
        let visible = self.element(pos);
        let hidden = visible
            .and_then(|e| self.hidden_of.get(&e.uid))
            .into_iter()
            .flatten()
            .copied();
        visible.into_iter().chain(hidden)
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

    /// What the list diffs `edits` make of this view, one diff per import
    /// and in import order, derived with no read of the list (`TRAME-282`).
    /// Nothing changes until [`commit_diffs`](Self::commit_diffs) takes the plan.
    ///
    /// `None` when only a read can tell, in three cases:
    /// * the view skipped an element it could not read;
    /// * a uid sits on two elements before or after;
    /// * an edit runs past the end of the list.
    ///
    /// The plan costs a copy of the list and the elements the diffs touch,
    /// never a parse of the list.
    fn plan_diffs(&self, edits: &[Vec<SlotEdit>]) -> Option<AxisPlan<L>> {
        if self.read == LineRead::Skipped || !self.hidden_of.is_empty() {
            return None;
        }
        let mut elements: Vec<Element> = self.elements.as_slice().to_vec();
        let mut touched: Vec<(Element, Touch)> = Vec::new();
        let mut first = elements.len();
        for delta in edits {
            let mut next = Vec::with_capacity(elements.len());
            let mut cursor = 0usize;
            for edit in delta {
                match edit {
                    SlotEdit::Retain(n) => {
                        let end = cursor.checked_add(n.to_usize())?;
                        next.extend_from_slice(elements.get(cursor..end)?);
                        cursor = end;
                    }
                    SlotEdit::Delete(n) => {
                        let end = cursor.checked_add(n.to_usize())?;
                        let removed = elements.get(cursor..end)?;
                        touched.extend(removed.iter().map(|e| (*e, Touch::Removed)));
                        first = first.min(next.len());
                        cursor = end;
                    }
                    SlotEdit::Insert(new) => {
                        first = first.min(next.len());
                        touched.extend(new.iter().map(|e| (*e, Touch::Added)));
                        next.extend_from_slice(new);
                    }
                }
            }
            next.extend_from_slice(elements.get(cursor..)?);
            elements = next;
        }
        // An element a register names shows no line, so its removal, a
        // sweep, removes no line. A diff that adds one leaves the lines to a
        // read.
        if !self.concealed.is_empty() {
            if touched
                .iter()
                .any(|(e, t)| *t == Touch::Added && self.concealed.contains_key(&e.id))
            {
                return None;
            }
            touched.retain(|(e, _)| !self.concealed.contains_key(&e.id));
        }
        // The first edit as a position: each hidden slot before it shows no
        // line.
        let first = match elements.get(..first) {
            Some(before) if !self.concealed.is_empty() => before
                .iter()
                .filter(|e| !self.concealed.contains_key(&e.id))
                .count(),
            _ => first,
        };
        // How many elements each touched id and each touched uid ends on. A
        // view with no hidden element holds one element per uid.
        let mut ids: HashMap<ElementId, Count> = HashMap::default();
        let mut uids: HashMap<Uid, Count> = HashMap::default();
        for (element, touch) in &touched {
            let id = ids.entry(element.id).or_insert_with(|| {
                if self.owner.contains_key(&element.id) {
                    Count::held()
                } else {
                    Count::absent()
                }
            });
            let uid = uids.entry(element.uid).or_insert_with(|| {
                if self.at.contains_key(&element.uid) {
                    Count::held()
                } else {
                    Count::absent()
                }
            });
            id.step(*touch);
            uid.step(*touch);
        }
        if ids
            .values()
            .chain(uids.values())
            .any(|n| n.end > Len::<Slot>::ONE)
        {
            return None;
        }
        let removed: Vec<Element> = touched
            .iter()
            .filter(|(e, _)| ids.get(&e.id).is_some_and(|n| n.end == Len::<Slot>::ZERO))
            .map(|(e, _)| *e)
            .collect();
        let mut change = AxisChange {
            gone: Vec::new(),
            added: Vec::new(),
            removed: removed.clone(),
            first: None,
            kept: Kept::Same,
        };
        for (uid, n) in &uids {
            let (start, end) = (n.start, n.end);
            if start == Len::<Slot>::ONE && end == Len::<Slot>::ZERO {
                change.gone.push(*uid);
            } else if start == Len::<Slot>::ZERO && end == Len::<Slot>::ONE {
                change.added.push(*uid);
            }
        }
        // A line deleted, then added again by a later import, keeps its
        // element or sits on another.
        for (element, touch) in &touched {
            let was = self.pos(&element.uid).and_then(|pos| self.element(pos));
            if *touch == Touch::Added
                && was.is_some_and(|was| was.id != element.id)
                && ids
                    .get(&element.id)
                    .is_some_and(|n| n.end == Len::<Slot>::ONE)
            {
                change.kept = Kept::Changed;
            }
        }
        let first = Pos::<L>::from_index(first)?;
        change.first = Some(first);
        Some(AxisPlan {
            elements,
            removed,
            added: touched
                .iter()
                .filter(|(e, t)| {
                    *t == Touch::Added && ids.get(&e.id).is_some_and(|n| n.end == Len::<Slot>::ONE)
                })
                .map(|(e, _)| *e)
                .collect(),
            first,
            change,
            _axis: std::marker::PhantomData,
        })
    }

    /// Take a plan [`plan_diffs`](Self::plan_diffs) made of this view. The positions
    /// before its first edit keep their entry, and each one after is
    /// recorded again.
    fn commit_diffs(&mut self, plan: AxisPlan<L>) -> Option<AxisChange<L>> {
        let elements = plan.elements;
        let first = plan.first;
        for element in &plan.removed {
            self.owner.remove(&element.id);
            if self.at.get(&element.uid).is_some_and(|pos| *pos < first) {
                return None;
            }
        }
        // The plan holds each uid once, so each line after `first` is
        // recorded again over its old entry, and only a gone line is dropped.
        for uid in &plan.change.gone {
            self.at.remove(uid);
        }
        for element in &plan.added {
            self.owner.insert(element.id, element.uid);
        }
        if self.concealed.is_empty() {
            self.lines = elements.iter().map(|e| e.uid).collect();
            self.shown = (0..elements.len())
                .map(Pos::<Slot>::from_index)
                .collect::<Option<IdxVec<L, Pos<Slot>>>>()?;
        } else {
            // A hidden slot shows no line, so a line sits on the slot of its
            // element among the slots no register names.
            let mut lines = IdxVec::default();
            let mut shown = IdxVec::default();
            for (slot, element) in elements.iter().enumerate() {
                if self.concealed.contains_key(&element.id) {
                    continue;
                }
                lines.push(element.uid);
                shown.push(Pos::<Slot>::from_index(slot)?);
            }
            self.lines = lines;
            self.shown = shown;
        }
        self.renumber_from(first);
        self.elements = IdxVec::from(elements);
        Some(plan.change)
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
            self.rebuilt();
        }
    }

    /// The slot each element of `uid` sits at, last first, for a delete that
    /// removes every one of them (`TRAME-269`).
    fn slots_of(&self, uid: &Uid) -> Vec<Pos<Slot>> {
        let mut slots: Vec<Pos<Slot>> = self
            .elements
            .iter_enumerated()
            .filter(|(_, e)| e.uid == *uid && !self.is_concealed(&e.id))
            .map(|(slot, _)| slot)
            .collect();
        slots.reverse();
        slots
    }

    /// Remove every element of the line `uid` from the view, as
    /// [`Axes::delete`] removes them from the list, with no read of the list.
    /// `None` when the view skipped an element it could not read, where its
    /// slots and the list's differ, or when `uid` shows no line. The view is
    /// then left half edited, so the caller drops it.
    fn remove_line(&mut self, uid: &Uid) -> Option<()> {
        if self.read == LineRead::Skipped {
            return None;
        }
        let pos = self.pos(uid)?;
        // A hidden element stays: its register may still bring it back.
        let removed: Vec<Pos<Slot>> = self
            .elements
            .iter_enumerated()
            .filter(|(_, e)| e.uid == *uid && !self.is_concealed(&e.id))
            .map(|(slot, _)| slot)
            .collect();
        let next = self;
        for slot in removed.iter().rev() {
            let element = next.elements.remove(*slot)?;
            next.owner.remove(&element.id);
        }
        next.lines.remove(pos)?;
        next.shown.remove(pos)?;
        next.at.remove(uid);
        next.hidden_of.remove(uid);
        for shown in next.shown.iter_mut() {
            let before = *shown;
            for _ in removed.iter().filter(|r| **r < before) {
                *shown = shown.prev()?;
            }
        }
        next.renumber_from(pos);
        Some(())
    }

    /// Insert the minted line `uid` before the line at `at`, as
    /// [`Axes::insert`] writes it, with no read of the list. `None` where the
    /// view and the list may differ, as for
    /// [`remove_line`](Self::remove_line), or when `uid` already names a line.
    fn insert_minted(&mut self, at: Pos<L>, uid: Uid) -> Option<()> {
        let element = Element::minted(uid);
        if self.read == LineRead::Skipped
            || self.at.contains_key(&uid)
            || self.owner.contains_key(&element.id)
        {
            return None;
        }
        let slot = self.insert_slot(at)?;
        let next = self;
        next.elements.insert(slot, element).ok()?;
        next.owner.insert(element.id, uid);
        for shown in next.shown.iter_mut() {
            if *shown >= slot {
                *shown += Len::<Slot>::ONE;
            }
        }
        next.lines.insert(at, uid).ok()?;
        next.shown.insert(at, slot).ok()?;
        next.renumber_from(at);
        Some(())
    }

    /// Record the position of every line from `from` on, after an edit
    /// shifted them.
    fn renumber_from(&mut self, from: Pos<L>) {
        let lines = &self.lines;
        let at = &mut self.at;
        for (pos, uid) in lines.iter_enumerated().skip(from.to_usize()) {
            at.insert(*uid, pos);
        }
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

/// The space of the generation a line carries (`TRAME-271`): how many lines
/// were derived after it, less one, in the history a peer holds.
pub enum LineGeneration {}

impl VersionKind for LineGeneration {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("LineGeneration");
}

impl OpenRange for LineGeneration {}

/// The domain string the derivation hashes first, so no other BLAKE2b input of
/// the workspace can produce a line uid.
const DERIVATION_DOMAIN: &[u8] = b"ambre-trame-line-v1";

/// What a derived line follows: the line before it, or the axis root when the
/// axis is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Before {
    Root,
    Line(Uid),
}

impl Before {
    /// The half of a generation key that names what the line follows.
    fn key<L: AxisOrder>(self) -> String {
        match self {
            Before::Root => L::ROOT_KEY.to_owned(),
            Before::Line(uid) => uid.to_key_part(),
        }
    }
}

/// The generation key `"{before}:{generation:08x}"` of `line_gen_{sheet}`.
fn generation_key<L: AxisOrder>(before: Before, generation: Version<LineGeneration>) -> String {
    format!("{}:{generation:08x}", before.key::<L>())
}

/// The half of a generation key that names what the line follows. It is cut
/// at the fixed width of the generation [`generation_key`] writes, with no
/// scan of the key. `None` for a key of any other shape.
fn line_of_generation_key(key: &str) -> Option<&str> {
    let cut = key.len().checked_sub(GENERATION_DIGITS + 1)?;
    let (before, rest) = key.split_at_checked(cut)?;
    rest.starts_with(':').then_some(before)
}

/// The hex digits of the generation half of a generation key.
const GENERATION_DIGITS: usize = 8;

/// Read a generation key of axis `L` back into its two halves. `None` for a
/// key of the other axis root, and for any other spelling than the one
/// [`generation_key`] writes. A uid half names a line of either axis: the
/// derivation reads it under the line it hangs after alone.
fn parse_generation_key<L: AxisOrder>(key: &str) -> Option<(Before, Version<LineGeneration>)> {
    let (before, generation) = key.rsplit_once(':')?;
    let value = u32::from_str_radix(generation, 16).ok()?;
    let parsed = Version::<LineGeneration>::new(value);
    let before = if before == L::ROOT_KEY {
        Before::Root
    } else {
        Before::Line(Uid::parse(before)?)
    };
    (generation_key::<L>(before, parsed) == key).then_some((before, parsed))
}

/// The uid of the line derived after `before` at generation `generation`, on
/// axis `L` of `sheet`. It is BLAKE2b over the domain and the four inputs, cut
/// to 96 bits. 64 bits fill the `peer` half and 32 bits the `counter` half, so a
/// reader parses it as any uid (`TRAME-248`). The inputs are sealed document
/// content, and the uid never leaves the sealed document (`ZK-001`).
fn derive_uid<L: AxisOrder>(
    sheet: &Id<Sheet>,
    before: Before,
    generation: Version<LineGeneration>,
) -> Option<Uid> {
    let mut hasher = Blake2b512::new();
    hasher.update(DERIVATION_DOMAIN);
    let sheet = sheet.as_str().as_bytes();
    hasher.update(u64::try_from(sheet.len()).ok()?.to_le_bytes());
    hasher.update(sheet);
    hasher.update([L::TAG]);
    match before {
        Before::Root => hasher.update([0]),
        Before::Line(uid) => {
            hasher.update([1]);
            hasher.update(uid.peer.get().to_le_bytes());
            hasher.update(uid.counter.get().to_le_bytes());
        }
    }
    hasher.update(generation.get().to_le_bytes());
    let digest = hasher.finalize();
    let (peer, rest) = digest.as_slice().split_first_chunk::<8>()?;
    let counter = rest.first_chunk::<4>()?;
    Some(Uid {
        peer: ambre_core::PeerId::new(u64::from_le_bytes(*peer)),
        counter: ambre_core::IdCounter::new(u32::from_le_bytes(*counter)),
    })
}

/// The lines a write past the used range adds on one axis, planned before
/// anything is written so a refusal writes nothing.
pub(crate) struct Growth<L: AxisOrder> {
    steps: Vec<GrowthStep>,
    axis: std::marker::PhantomData<fn() -> L>,
}

/// One derived line of a [`Growth`]: the record it writes and the element it
/// pushes.
struct GrowthStep {
    before: Before,
    generation: Version<LineGeneration>,
    element: Element,
}

/// One run of a list diff, over the slots of an axis list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotEdit {
    /// Keep this many slots.
    Retain(Len<Slot>),
    /// Remove this many slots.
    Delete(Len<Slot>),
    /// Insert these elements.
    Insert(Vec<Element>),
}

/// The runs of one list diff an import reports, its inserted values read as
/// elements. `None` when a value is no element this build reads: only a read
/// of the list then says what the view holds (`SCHEMA-003`).
#[must_use]
pub fn slot_edits(diff: &[loro::event::ListDiffItem]) -> Option<Vec<SlotEdit>> {
    diff.iter()
        .map(|item| match item {
            loro::event::ListDiffItem::Retain { retain } => {
                Some(SlotEdit::Retain(Len::<Slot>::of(*retain)))
            }
            loro::event::ListDiffItem::Delete { delete } => {
                Some(SlotEdit::Delete(Len::<Slot>::of(*delete)))
            }
            loro::event::ListDiffItem::Insert { insert, .. } => insert
                .iter()
                .map(|value| match value {
                    loro::ValueOrContainer::Value(LoroValue::String(text)) => Element::parse(text),
                    _ => None,
                })
                .collect::<Option<Vec<Element>>>()
                .map(SlotEdit::Insert),
        })
        .collect()
}

/// What a list diff did to one axis. It names the lines it removed and the
/// lines it added. It says whether a line it removed and added again kept
/// its element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisChange<L: Axis> {
    pub gone: Vec<Uid>,
    pub added: Vec<Uid>,
    /// The elements the diffs removed for good. A key under one of them
    /// places on no cell any more.
    pub removed: Vec<Element>,
    /// The first line an edit reached, or `None` for an axis no diff
    /// reached.
    pub first: Option<Pos<L>>,
    pub kept: Kept,
}

impl<L: Axis> AxisChange<L> {
    /// The change of an axis no diff reached.
    fn untouched() -> Self {
        Self {
            gone: Vec::new(),
            added: Vec::new(),
            removed: Vec::new(),
            first: None,
            kept: Kept::Same,
        }
    }

    /// `next` after `self`: a line that went then came back counts as
    /// neither, and the first line reached is the earlier one.
    pub(crate) fn fold(&mut self, next: AxisChange<L>) {
        for uid in next.gone {
            match self.added.iter().position(|u| *u == uid) {
                Some(i) => {
                    self.added.swap_remove(i);
                }
                None => self.gone.push(uid),
            }
        }
        for uid in next.added {
            match self.gone.iter().position(|u| *u == uid) {
                Some(i) => {
                    self.gone.swap_remove(i);
                }
                None => self.added.push(uid),
            }
        }
        self.removed.extend(next.removed);
        if next.kept == Kept::Changed {
            self.kept = Kept::Changed;
        }
        self.first = match (self.first, next.first) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

/// What the list diffs of one batch did to a sheet's two axes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutChange {
    pub rows: AxisChange<Row>,
    pub cols: AxisChange<Col>,
}

impl LayoutChange {
    /// `next` after `self`, axis by axis ([`AxisChange::fold`]).
    pub(crate) fn fold(&mut self, next: LayoutChange) {
        self.rows.fold(next.rows);
        self.cols.fold(next.cols);
    }

    /// Whether each line the diffs removed and added again kept its element.
    #[must_use]
    pub fn kept(&self) -> Kept {
        if self.rows.kept == Kept::Same && self.cols.kept == Kept::Same {
            Kept::Same
        } else {
            Kept::Changed
        }
    }
}

/// The view a list diff leaves, before it is taken.
struct AxisPlan<L: Axis> {
    elements: Vec<Element>,
    /// The elements the diffs removed for good.
    removed: Vec<Element>,
    /// The elements the diffs added that stay.
    added: Vec<Element>,
    /// The first line an edit reached: the lines before it are untouched.
    /// A view with no hidden element holds one slot per line, in order.
    first: Pos<L>,
    change: AxisChange<L>,
    _axis: std::marker::PhantomData<L>,
}

/// Whether a diff removed an element from its slot or added it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touch {
    Removed,
    Added,
}

/// How many elements of one id, or of one uid, a view holds before a diff
/// and after it.
#[derive(Debug, Clone, Copy)]
struct Count {
    start: Len<Slot>,
    end: Len<Slot>,
}

impl Count {
    /// One element before the diff.
    fn held() -> Self {
        Self {
            start: Len::<Slot>::ONE,
            end: Len::<Slot>::ONE,
        }
    }

    /// No element before the diff.
    fn absent() -> Self {
        Self {
            start: Len::<Slot>::ZERO,
            end: Len::<Slot>::ZERO,
        }
    }

    /// The count after one more touch. A removal never takes it below zero.
    fn step(&mut self, touch: Touch) {
        self.end = match touch {
            Touch::Removed => self.end.saturating_sub(Len::<Slot>::ONE),
            Touch::Added => self.end + Len::<Slot>::ONE,
        };
    }
}

/// Which line list a per-sheet container is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ListAxis {
    /// The row order.
    Rows,
    /// The column order.
    Cols,
    /// The generation record of both axes. No cache holds it: a write that
    /// derives a line reads it.
    Generations,
    /// The hiding registers of both axes (`TRAME-302`).
    Hidden,
}

/// The line list the family `root` names, or `None` for a family that holds
/// no line. The one place outside the axis reads that a remote batch
/// asks (`TRAME-282`).
#[must_use]
pub(crate) fn axis_list(root: SheetRoot) -> Option<ListAxis> {
    match root {
        SheetRoot::Rows => Some(ListAxis::Rows),
        SheetRoot::Cols => Some(ListAxis::Cols),
        SheetRoot::LineGen => Some(ListAxis::Generations),
        SheetRoot::LineHidden => Some(ListAxis::Hidden),
        _ => None,
    }
}

ambre_core::define_flag! {
    /// Whether each line a list diff kept sits on the element it sat on.
    pub enum Kept {
        /// Every kept line sits on its element.
        Same,
        /// A kept line now sits on another element.
        Changed,
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

    /// Take the list diffs of each axis, derived with no read of the lists,
    /// and answer what they changed ([`AxisView::plan_diffs`]). An axis with no
    /// diff is kept. `None` when only a read can tell. The index is then
    /// left as it was. A plan both axes accepted may still fail to commit,
    /// and leave the index half edited: the caller reads the lists again.
    pub fn apply(
        &mut self,
        rows: &[Vec<SlotEdit>],
        cols: &[Vec<SlotEdit>],
    ) -> Option<LayoutChange> {
        let row_plan = if rows.is_empty() {
            None
        } else {
            Some(self.rows.plan_diffs(rows)?)
        };
        let col_plan = if cols.is_empty() {
            None
        } else {
            Some(self.cols.plan_diffs(cols)?)
        };
        let rows = match row_plan {
            Some(plan) => self.rows.commit_diffs(plan)?,
            None => AxisChange::untouched(),
        };
        let cols = match col_plan {
            Some(plan) => self.cols.commit_diffs(plan)?,
            None => AxisChange::untouched(),
        };
        Some(LayoutChange { rows, cols })
    }

    /// Take the hiding registers a change wrote or deleted, in order, with
    /// no read of the lists. Answer which lines went and came back
    /// (`TRAME-303`).
    pub(crate) fn apply_hidden(&mut self, edits: &[HiddenEdit]) -> LayoutChange {
        let mut change = LayoutChange {
            rows: AxisChange::untouched(),
            cols: AxisChange::untouched(),
        };
        for edit in edits {
            match edit.key.axis {
                ListAxis::Cols => change.cols.fold(self.cols.hidden_edit(edit)),
                ListAxis::Rows | ListAxis::Generations | ListAxis::Hidden => {
                    change.rows.fold(self.rows.hidden_edit(edit));
                }
            }
        }
        change
    }

    /// Whether the cell key `key` would place once every hiding register is
    /// dropped, and a register hides one of its elements. Such a cell is
    /// held, not gone: an undo shows it again. So no walk rewrites it, and no
    /// reseed counts it as a formula that places on no cell (`TRAME-304`).
    /// A key under a removed element is gone whatever a register hides.
    #[must_use]
    pub fn conceals(&self, key: &str) -> bool {
        if self.rows.concealed.is_empty() && self.cols.concealed.is_empty() {
            return false;
        }
        let Some((row, col)) = parse_cell_key(key) else {
            return false;
        };
        let (row_hidden, col_hidden) = (
            self.rows.concealed.contains_key(&row.id),
            self.cols.concealed.contains_key(&col.id),
        );
        (row_hidden || col_hidden)
            && (row_hidden || self.rows.owner.get(&row.id) == Some(&row.uid))
            && (col_hidden || self.cols.owner.get(&col.id) == Some(&col.uid))
    }

    /// Whether a hiding register names an element of the line `uid`, on
    /// either axis.
    #[must_use]
    pub fn hides_uid(&self, uid: &Uid) -> bool {
        self.rows
            .registers
            .values()
            .chain(self.cols.registers.values())
            .flatten()
            .any(|e| e.uid == *uid)
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

    /// The key a write at `at` stores its cell under: the visible element of
    /// each line. `None` past the used range.
    #[must_use]
    pub fn key_at(&self, at: CellPos) -> Option<CellKey> {
        Some(CellKey::of(
            self.rows.element(at.row)?,
            self.cols.element(at.col)?,
        ))
    }

    /// Whether a line of either axis holds a hidden element. With none, a
    /// cell sits under one key alone, the one [`key_at`](Self::key_at) spells.
    #[must_use]
    pub fn holds_hidden(&self) -> bool {
        !self.rows.hidden_of.is_empty() || !self.cols.hidden_of.is_empty()
    }

    /// Every key a cell at `at` may sit under, the one that wins first: each
    /// live element pair of its two lines, by [`CellRank`]. A read takes the
    /// first one the map holds, and a clear deletes them all.
    #[must_use]
    pub fn keys_at(&self, at: CellPos) -> Vec<CellKey> {
        let mut keys = Vec::new();
        for row in self.rows.elements_at(at.row) {
            for col in self.cols.elements_at(at.col) {
                keys.push(CellKey::of(row, col));
            }
        }
        keys
    }

    /// Where a stored cell key reads, and its rank against another key at
    /// the same place. `None` for a key whose element is gone.
    #[must_use]
    pub fn place(&self, key: &str) -> Option<(CellPos, CellRank)> {
        let (row, col) = parse_cell_key(key)?;
        let at = CellPos::new(
            self.rows.pos_of_element(&row)?,
            self.cols.pos_of_element(&col)?,
        );
        Some((
            at,
            CellRank {
                row: row.id,
                col: col.id,
            },
        ))
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

    /// The index after the line `uid` of axis `L` is deleted, derived from
    /// this one with no read of the document. `None` when only a read can tell
    /// ([`AxisView::remove_line`]).
    pub(crate) fn without_line<L: AxisOrder>(mut self, uid: &Uid) -> Option<SheetIndex> {
        L::view_mut(&mut self).remove_line(uid)?;
        Some(self)
    }

    /// The index after the minted line `uid` of axis `L` is inserted before
    /// position `at`, derived from this one with no read of the document.
    /// `None` when only a read can tell ([`AxisView::insert_minted`]).
    pub(crate) fn with_line<L: AxisOrder>(mut self, at: Pos<L>, uid: Uid) -> Option<SheetIndex> {
        L::view_mut(&mut self).insert_minted(at, uid)?;
        Some(self)
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
        view.elements = elements.into();
        view.rebuilt();
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
    view.elements = elements.into();
    view.rebuilt();
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
    view.elements = elements.into();
    view.rebuilt();
    Some(uid)
}

/// Which of two keys at one place a reader shows: the smaller rank. The
/// visible element of each line has the smallest id, so the cell written
/// under both visible elements wins (`TRAME-279`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRank {
    row: ElementId,
    col: ElementId,
}

/// The list that orders one axis, named by its line marker.
/// The line a generation key names, as the schema reading prints it: a row, a
/// column, or the root of one axis.
#[cfg(test)]
pub(crate) fn generation_owner(index: &SheetIndex, key: &str) -> Option<String> {
    let line = |before: Before| match before {
        Before::Line(uid) => match (index.row_pos(&uid), index.col_pos(&uid)) {
            (Some(row), _) => Some(format!("row {}", row.get() + 1)),
            (_, Some(col)) => Some(format!("column {}", col.get() + 1)),
            _ if index.hides_uid(&uid) => Some("a hidden line".to_owned()),
            _ => None,
        },
        Before::Root => None,
    };
    let (before, generation) = match parse_generation_key::<Row>(key) {
        Some((Before::Root, generation)) => {
            return Some(format!("the row root carries generation {generation}"));
        }
        Some(found) => found,
        None => match parse_generation_key::<Col>(key)? {
            (Before::Root, generation) => {
                return Some(format!("the column root carries generation {generation}"));
            }
            found => found,
        },
    };
    Some(format!("{} carries generation {generation}", line(before)?))
}

pub(crate) trait AxisOrder: Line {
    /// The key half that names the axis root in the generation record.
    #[doc(hidden)]
    const ROOT_KEY: &'static str;
    /// The line list of the axis.
    #[doc(hidden)]
    const LIST: ListAxis;
    /// The byte that names the axis in a derivation.
    #[doc(hidden)]
    const TAG: u8;
    /// The view of this axis inside a sheet index.
    #[doc(hidden)]
    fn view(index: &SheetIndex) -> &AxisView<Self>;
    /// The container family of the list.
    #[doc(hidden)]
    fn order_root() -> SheetRoot;
    /// The same view, to update in place.
    #[doc(hidden)]
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Self>;
}

impl AxisOrder for Row {
    const ROOT_KEY: &'static str = "rows";
    const LIST: ListAxis = ListAxis::Rows;
    const TAG: u8 = 0;
    fn view(index: &SheetIndex) -> &AxisView<Row> {
        &index.rows
    }
    fn order_root() -> SheetRoot {
        SheetRoot::Rows
    }
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Row> {
        &mut index.rows
    }
}

impl AxisOrder for Col {
    const ROOT_KEY: &'static str = "cols";
    const LIST: ListAxis = ListAxis::Cols;
    const TAG: u8 = 1;
    fn view(index: &SheetIndex) -> &AxisView<Col> {
        &index.cols
    }
    fn order_root() -> SheetRoot {
        SheetRoot::Cols
    }
    fn view_mut(index: &mut SheetIndex) -> &mut AxisView<Col> {
        &mut index.cols
    }
}

/// What a delete removed: the line, and the elements of it this peer held. A
/// purge drops the cells keyed by those elements alone. A cell another peer
/// wrote at the same moment under its own element survives (`TRAME-265`).
pub(crate) struct Removed {
    pub(crate) uid: Uid,
    pub(crate) elements: Vec<Element>,
}

/// What one slice of a sweep removed from one axis (`TRAME-305`).
pub(crate) struct Swept {
    /// Each line it removed elements of.
    pub(crate) lines: Vec<SweptLine>,
    /// Whether the axis still holds an element to sweep.
    pub(crate) more: SweepLeft,
}

impl Default for Swept {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            more: SweepLeft::Done,
        }
    }
}

/// One line a sweep removed elements of.
pub(crate) struct SweptLine {
    pub(crate) removed: Removed,
    /// Whether another element of the list still carries the uid.
    pub(crate) uid_left: UidLeft,
}

ambre_core::define_flag! {
    /// Whether a sweep left an element to remove on an axis.
    pub(crate) enum SweepLeft {
        /// Nothing an ended session hid is left.
        Done,
        /// The budget of the slice ran out first.
        More,
    }
}

ambre_core::define_flag! {
    /// Whether another element still carries the uid of a swept line.
    pub(crate) enum UidLeft {
        /// No element of the uid is left: what the uid keys goes too.
        Gone,
        /// Another element carries it: its size and its filter mark stay.
        Carried,
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

    /// The index of the sheet, both axes read from the document, less
    /// every element a hiding register names (`TRAME-303`).
    pub(crate) fn index(&self) -> SheetIndex {
        let (rows, cols) = self.registers();
        SheetIndex {
            rows: AxisView::read_hiding(&self.list::<Row>(), rows),
            cols: AxisView::read_hiding(&self.list::<Col>(), cols),
        }
    }

    /// The view of axis `L` alone, its hiding registers applied.
    fn read_view<L: AxisOrder>(&self) -> AxisView<L> {
        let (rows, cols) = self.registers();
        let registers = match L::LIST {
            ListAxis::Cols => cols,
            ListAxis::Rows | ListAxis::Generations | ListAxis::Hidden => rows,
        };
        AxisView::read_hiding(&self.list::<L>(), registers)
    }

    /// The hiding registers of the sheet, `line_hidden_{sheet}`. Private:
    /// the registers are part of the axes.
    fn hidden_map(&self) -> LoroMap {
        self.doc
            .get_map(SheetContainer::of(SheetRoot::LineHidden, self.sheet).as_str())
    }

    /// The hiding registers of each axis. A key or a value no build writes
    /// is logged and left out, so its lines show (`SCHEMA-003`).
    fn registers(
        &self,
    ) -> (
        HashMap<HidingKey, Vec<Element>>,
        HashMap<HidingKey, Vec<Element>>,
    ) {
        let mut rows = HashMap::default();
        let mut cols = HashMap::default();
        let map = self.hidden_map();
        if map.is_empty() {
            return (rows, cols);
        }
        let LoroValue::Map(entries) = map.get_value() else {
            return (rows, cols);
        };
        for (key, value) in entries.iter() {
            let read = match (HidingKey::parse(key), value) {
                (Some(key), LoroValue::Binary(bytes)) => parse_register(bytes).map(|e| (key, e)),
                _ => None,
            };
            let Some((key, elements)) = read else {
                tracing::error!("[Axes] unreadable hiding register {key}");
                continue;
            };
            match key.axis {
                ListAxis::Cols => cols.insert(key, elements),
                ListAxis::Rows | ListAxis::Generations | ListAxis::Hidden => {
                    rows.insert(key, elements)
                }
            };
        }
        (rows, cols)
    }

    /// Hide the lines at `span` under one register, where
    /// [`hide_lines`](Self::hide_lines) removes them: one map write, and
    /// no list, cell, style or generation key written (`TRAME-302`). Returns
    /// what each line hid, the last line first. A virtual line hides nothing.
    ///
    /// `layout` follows as in [`hide_lines`](Self::hide_lines).
    pub(crate) fn hide_lines<L: AxisOrder>(
        &self,
        layout: &mut Option<SheetIndex>,
        span: Span<L>,
    ) -> Vec<Removed> {
        let cached = layout
            .as_ref()
            .map(L::view)
            .filter(|view| view.read_state() == LineRead::Clean);
        #[cfg(test)]
        if let Some(view) = cached {
            assert_eq!(*view, self.read_view::<L>(), "the layout is the list's");
        }
        let read;
        let view = match cached {
            Some(view) => view,
            None => {
                read = self.read_view::<L>();
                &read
            }
        };
        let uids: Vec<Uid> = span.into_iter().filter_map(|at| view.uid(at)).collect();
        let gone: HashSet<Uid> = uids.iter().copied().collect();
        let mut elements: HashMap<Uid, Vec<Element>> = HashMap::default();
        let mut hidden = Vec::new();
        for element in view.elements.iter() {
            if gone.contains(&element.uid)
                && !view.is_concealed(&element.id)
                && !hidden.contains(element)
            {
                elements.entry(element.uid).or_default().push(*element);
                hidden.push(*element);
            }
        }
        let Some(first) = hidden.first().copied() else {
            return Vec::new();
        };
        // No live register of this session names its first element, so no
        // other write spells the key.
        let key = HidingKey::of::<L>(ambre_core::PeerId::new(self.doc.peer_id()), first);
        if self
            .hidden_map()
            .insert(
                &key.spell(),
                LoroValue::Binary(spell_register(&hidden).into()),
            )
            .is_err()
        {
            *layout = None;
            return Vec::new();
        }
        *layout = layout.take().map(|mut index| {
            L::view_mut(&mut index).conceal(key, hidden);
            index
        });
        uids.into_iter()
            .rev()
            .map(|uid| Removed {
                uid,
                elements: elements.remove(&uid).unwrap_or_default(),
            })
            .collect()
    }

    /// Insert the minted line `uid` before the line at `at`, or after the last
    /// line when `at` is the end. `None` when `at` lies further, or the list
    /// refuses the write.
    pub(crate) fn insert<L: AxisOrder>(&self, at: Pos<L>, uid: Uid) -> Option<()> {
        let list = self.list::<L>();
        let slot = self.read_view::<L>().insert_slot(at)?;
        list.insert(slot.to_index()?, Element::minted(uid).spell().as_str())
            .ok()
    }

    /// Delete the line at `at`: every element of its uid this peer holds, the
    /// hidden ones too (`TRAME-269`). Returns what it removed, `None` past the
    /// end.
    ///
    /// `layout` is the layout of the lists before the call, when the caller
    /// holds one. The slots are then read from it, not from the list. The call
    /// removes the line from `layout` too, so the next delete of one commit
    /// reads the layout after this one. `layout` turns `None` where the view
    /// cannot follow the list, and a later delete then reads the list.
    pub(crate) fn delete<L: AxisOrder>(
        &self,
        layout: &mut Option<SheetIndex>,
        at: Pos<L>,
    ) -> Option<Removed> {
        let list = self.list::<L>();
        let cached = layout
            .as_ref()
            .map(L::view)
            .filter(|view| view.read_state() == LineRead::Clean);
        #[cfg(test)]
        if let Some(view) = cached {
            assert_eq!(*view, self.read_view::<L>(), "the layout is the list's");
        }
        let read;
        let view = match cached {
            Some(view) => view,
            None => {
                read = self.read_view::<L>();
                &read
            }
        };
        let uid = view.uid(at)?;
        let slots = view.slots_of(&uid);
        // An undo that puts an element back while a peer moved it leaves the
        // list holding it twice: the purge names each element once.
        let mut elements: Vec<Element> = Vec::with_capacity(slots.len());
        for element in slots.iter().filter_map(|slot| view.elements.get(*slot)) {
            if !elements.contains(element) {
                elements.push(*element);
            }
        }
        let next = layout
            .take()
            .and_then(|index| index.without_line::<L>(&uid));
        for slot in slots {
            list.delete(slot.to_index()?, 1).ok()?;
        }
        *layout = next;
        Some(Removed { uid, elements })
    }

    /// Remove for good up to `budget` elements of axis `L` that a register of
    /// an ended session hides, in list order. Remove each register left with
    /// nothing to hide too (`TRAME-305`). An element a register of a running
    /// session also names stays. `runs` answers whether a session runs.
    /// The caller purges the cells, then commits under an origin no history
    /// records.
    pub(crate) fn sweep<L: AxisOrder>(
        &self,
        index: &SheetIndex,
        runs: &dyn Fn(ambre_core::PeerId) -> bool,
        budget: Len<Slot>,
    ) -> Swept {
        let view = L::view(index);
        let mut swept = Swept::default();
        if view.registers.is_empty() {
            return swept;
        }
        let (running, ended): (Vec<_>, Vec<_>) =
            view.registers.iter().partition(|(key, _)| runs(key.peer));
        if ended.is_empty() {
            return swept;
        }
        let kept: HashSet<ElementId> = running
            .iter()
            .flat_map(|(_, elements)| elements.iter().map(|e| e.id))
            .collect();
        let named: HashSet<ElementId> = ended
            .iter()
            .flat_map(|(_, elements)| elements.iter().map(|e| e.id))
            .filter(|id| !kept.contains(id))
            .collect();
        let mut slots = Vec::new();
        let mut left: HashSet<ElementId> = HashSet::default();
        let mut taken: HashMap<Uid, Vec<Element>> = HashMap::default();
        for (slot, element) in view.elements.iter_enumerated() {
            if !named.contains(&element.id) {
                continue;
            }
            if Len::<Slot>::of(slots.len()) < budget {
                slots.push(slot);
                taken.entry(element.uid).or_default().push(*element);
            } else {
                left.insert(element.id);
            }
        }
        let list = self.list::<L>();
        for run in slots.chunk_by(|a, b| b.prev() == Some(*a)).rev() {
            let Some(at) = run.first().and_then(|slot| slot.to_index()) else {
                continue;
            };
            if list.delete(at, run.len()).is_err() {
                return Swept::default();
            }
        }
        let map = self.hidden_map();
        for (key, elements) in &ended {
            if elements.iter().all(|e| !left.contains(&e.id)) {
                let _ = map.delete(&key.spell());
            }
        }
        // A uid another element still carries keeps what its uid keys.
        let gone: HashSet<Uid> = taken.keys().copied().collect();
        let mut carried: HashSet<Uid> = HashSet::default();
        let swept_ids: HashSet<ElementId> = taken.values().flatten().map(|e| e.id).collect();
        for element in view.elements.iter() {
            if gone.contains(&element.uid) && !swept_ids.contains(&element.id) {
                carried.insert(element.uid);
            }
        }
        swept.more = if left.is_empty() {
            SweepLeft::Done
        } else {
            SweepLeft::More
        };
        swept.lines = taken
            .into_iter()
            .map(|(uid, elements)| (Removed { uid, elements }, carried.contains(&uid)))
            .map(|(removed, carried)| SweptLine {
                removed,
                uid_left: if carried {
                    UidLeft::Carried
                } else {
                    UidLeft::Gone
                },
            })
            .collect();
        swept
    }

    /// Whether a hiding register of either axis remains.
    pub(crate) fn hides_any(&self) -> bool {
        !self.hidden_map().is_empty()
    }

    /// Move the line at `from` so it shows at `to`. Only the visible element
    /// moves: a hidden one shows nothing wherever it sits (`TRAME-268`).
    pub(crate) fn move_line<L: AxisOrder>(&self, from: Pos<L>, to: Pos<L>) -> Option<()> {
        let list = self.list::<L>();
        let view = self.read_view::<L>();
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
        let view = self.read_view::<Row>();
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

    /// Empty both axes and their record, for a sheet the workbook drops.
    pub(crate) fn clear(&self) {
        let _ = self.list::<Row>().clear();
        let _ = self.list::<Col>().clear();
        let _ = self.record().clear();
        let _ = self.hidden_map().clear();
    }

    /// The generation record of the sheet, `line_gen_{sheet}`. Private: the
    /// record is part of the axes.
    fn record(&self) -> LoroMap {
        self.doc
            .get_map(SheetContainer::of(SheetRoot::LineGen, self.sheet).as_str())
    }

    /// The highest generation the record holds under each line before.
    fn highest<L: AxisOrder>(&self) -> HashMap<Before, Version<LineGeneration>> {
        let mut highest: HashMap<Before, Version<LineGeneration>> = HashMap::default();
        for key in self.record().keys() {
            let Some((before, generation)) = parse_generation_key::<L>(&key) else {
                continue;
            };
            highest
                .entry(before)
                .and_modify(|seen| *seen = (*seen).max(generation))
                .or_insert(generation);
        }
        highest
    }

    /// Plan the lines a write past the used range adds on axis `L`, one per
    /// element id of `ids` (`TRAME-261`). Each line hangs after the last line
    /// of `index`, or after the line planned just before. Its generation is
    /// one more than the highest the record holds for that line, or 0. `None`
    /// when a generation would pass the last one: a repeat would make two
    /// lines one (`TRAME-271`).
    pub(crate) fn plan<L: AxisOrder>(
        &self,
        index: &SheetIndex,
        ids: &[ElementId],
    ) -> Option<Growth<L>> {
        let view = L::view(index);
        let mut before = view
            .len()
            .last()
            .and_then(|last| view.uid(last))
            .map_or(Before::Root, Before::Line);
        let highest = if ids.is_empty() {
            HashMap::default()
        } else {
            self.highest::<L>()
        };
        let mut steps = Vec::with_capacity(ids.len());
        for id in ids {
            let generation = match highest.get(&before) {
                Some(seen) if seen.get() == u32::MAX => return None,
                Some(seen) => seen.next(),
                None => Version::new(0),
            };
            let uid = derive_uid::<L>(self.sheet, before, generation)?;
            steps.push(GrowthStep {
                before,
                generation,
                element: Element::derived(uid, *id),
            });
            before = Before::Line(uid);
        }
        Some(Growth {
            steps,
            axis: std::marker::PhantomData,
        })
    }

    /// Write a planned growth: each derivation records its generation, drops
    /// the one below it (`TRAME-276`), and pushes its element. The caller
    /// commits, so the record rides the commit that pushes the line.
    pub(crate) fn grow<L: AxisOrder>(
        &self,
        index: &mut SheetIndex,
        growth: Growth<L>,
    ) -> Option<()> {
        let record = self.record();
        for step in growth.steps {
            record
                .insert(&generation_key::<L>(step.before, step.generation), true)
                .ok()?;
            if let Some(below) = step.generation.get().checked_sub(1) {
                let below = generation_key::<L>(step.before, Version::new(below));
                if record.get(&below).is_some() {
                    record.delete(&below).ok()?;
                }
            }
            self.push::<L>(index, step.element)?;
        }
        Some(())
    }

    /// Drop every generation the record holds under each line of `uids`, for
    /// a delete that removes them.
    pub(crate) fn purge_record(&self, uids: &[Uid]) {
        let record = self.record();
        // One walk of the record for every line, however many go. Each key
        // costs one hash: a sorted search compared the long peer prefix that
        // the uids of one peer share about nine times per key.
        let gone: HashSet<String> = uids.iter().map(Uid::to_key_part).collect();
        let keys: Vec<String> = record
            .keys()
            .filter(|key| line_of_generation_key(key).is_some_and(|before| gone.contains(before)))
            .map(|key| key.to_string())
            .collect();
        for key in keys {
            let _ = record.delete(&key);
        }
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
        axes.delete::<Row>(&mut None, Pos::new(0)).unwrap();
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
