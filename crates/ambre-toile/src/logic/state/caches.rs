//! Derived-cache maintenance for [`ToileState`](super::ToileState): the
//! `bounds` map, the `rstar` spatial index and the materialized `layers` order.
//! Split out of `state/mod.rs`. The incremental helpers keep the caches in step
//! with local edits. [`refresh_caches`](ToileState::refresh_caches) re-reads
//! what a remote import or a history step wrote.
//! [`rebuild_caches`](ToileState::rebuild_caches) reseeds them whole, and the
//! tests compare the refresh with it.

use ambre_core::hash::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use kurbo::Rect;
use loro::event::{ContainerDiff, Diff, DiffEvent};
use loro::{ContainerID, Index, LoroDoc, LoroValue, Subscription};

use crate::logic::index::{ElementBounds, SpatialIndex};
use ambre_core::id::{Connector, Element, Id};
use ambre_core::{ContentHash, Validated};

use crate::logic::model::NodeId;

use super::ToileState;
use super::codec::{expanded_aabb, map_placement, rotation_of, stored_rect};
use super::codec::{has_type, value_has_type};
use super::names::{ElementKey, FieldValues, Fields, Root, entry};
use crate::logic::model::ElementKind;
use ambre_core::crdt::CrdtName as _;

impl ToileState {
    /// Recompute one element's AABB from its map and mirror it into `bounds` and
    /// the R-tree. Returns the AABB it replaced, `None` for an element the
    /// caches did not hold yet.
    pub(super) fn update_bounds(
        &mut self,
        id: &Id<Element>,
        em: &Fields<ElementKey>,
    ) -> Option<Rect> {
        let rect = expanded_aabb(stored_rect(em), rotation_of(em));
        self.index
            .update(ElementBounds::from_rect(id.clone(), rect));
        self.bounds.insert(id.clone(), rect)
    }

    /// Note the stored file `id` draws, or forget `id` when it draws none.
    /// The export lists its pictures off this map alone, so a request never
    /// reads the other elements (`TOILE-IO-099`).
    pub(super) fn note_picture(&mut self, id: &Id<Element>, em: &Fields<ElementKey>) {
        match picture_of(em) {
            Some(hash) => self.pictures.insert(id.clone(), hash),
            None => self.pictures.remove(id),
        };
    }

    /// Full rebuild of every derived cache from `doc`. Used after a remote
    /// import; the incremental path (local commands) keeps them in step directly.
    pub(super) fn rebuild_caches(&mut self) {
        self.bounds.clear();
        self.pictures.clear();
        self.parents.clear();
        self.members.clear();
        let mut leaves = Vec::new();
        if let LoroValue::Map(m) = self.walk_elements() {
            for (key, v) in m.iter() {
                if let LoroValue::Map(fields) = v {
                    let fields = FieldValues::<ElementKey>::new(fields);
                    // Groups and frames name their members; the same pass
                    // indexes them, so the parent lookup costs no walk of its own.
                    if let Some(LoroValue::List(kids)) = fields.get(ElementKey::Children) {
                        let parent = Id::<Element>::from_text(key.clone());
                        for kid in kids.iter() {
                            if let LoroValue::String(s) = kid {
                                self.link_parent(&Id::<Element>::from_text(s.to_string()), &parent);
                            }
                        }
                    }
                    // Groups carry no stored geometry; their bounds are derived
                    // from members by `recompute_groups` after this leaf pass.
                    if value_has_type(fields, ElementKind::Group) {
                        continue;
                    }
                    let (stored, rot) = map_placement(fields);
                    let id = Id::<Element>::from_text(key.clone());
                    if let Some(hash) = value_picture(fields) {
                        self.pictures.insert(id.clone(), hash);
                    }
                    let rect = expanded_aabb(stored, rot);
                    self.bounds.insert(id.clone(), rect);
                    leaves.push(ElementBounds::from_rect(id, rect));
                }
            }
        }
        self.index = SpatialIndex::build(leaves.into_iter());
        self.rematerialize_layers();
        self.recompute_groups();
    }

    /// Drop what the subscription noted of the board so far: the next refresh
    /// reads only what lands after this call. The arrows it noted stay noted,
    /// for [`wires`](Self::wires) to read.
    pub(super) fn forget_touched(&self) {
        self.touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .board = BoardTouched::default();
    }

    /// Bring every derived cache in step with what a remote change or a
    /// history step wrote, and with nothing else (`TOILE-092`, `TOILE-292`). The document's own events name the
    /// elements it touched and whether it reordered. A change the events cannot
    /// place falls back to the full rebuild.
    pub(super) fn refresh_caches(&mut self) {
        let touched = std::mem::take(
            &mut self
                .touched
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .board,
        );
        if touched.reach == Reach::Whole {
            self.rebuild_caches();
            return;
        }
        for id in &touched.elements {
            self.refresh_element(id);
        }
        if touched.layers == PaintOrder::Changed {
            self.rematerialize_layers();
        }
        self.refresh_groups(&touched.elements);
    }

    /// Re-read one element: its bounds, its leaf and what its `children` list
    /// names. A group's bounds are left to [`refresh_groups`](Self::refresh_groups).
    fn refresh_element(&mut self, id: &Id<Element>) {
        let Some(em) = self.element_map(id) else {
            self.bounds.remove(id);
            self.pictures.remove(id);
            self.index.remove(id);
            self.relink_members(id, Vec::new());
            return;
        };
        let kids = self.children_of(id);
        self.relink_members(id, kids);
        // Only a birth writes the address of a picture, so an element the
        // caches held already keeps the entry it had. A move of many
        // elements then reads no address.
        if !has_type(&em, ElementKind::Group) && self.update_bounds(id, &em).is_none() {
            self.note_picture(id, &em);
        }
    }

    /// Recompute the bounds of every group a touched element sits in, at any
    /// depth, and of every touched group, to a fixpoint. A group missing a
    /// member's bounds holds none, as after a full rebuild.
    fn refresh_groups(&mut self, touched: &HashSet<Id<Element>>) {
        let mut groups: Vec<Id<Element>> = Vec::new();
        let mut seen: HashSet<Id<Element>> = HashSet::default();
        let mut stack: Vec<Id<Element>> = touched.iter().cloned().collect();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if self
                .element_map(&id)
                .is_some_and(|em| has_type(&em, ElementKind::Group))
            {
                groups.push(id.clone());
            }
            stack.extend(self.parents_of(&id).iter().cloned());
        }
        // Children before parents, so one pass settles a nesting of any
        // depth, and the next one finds nothing changed. A loop of groups,
        // which a peer can write, gets a third. The cap ends the loop then,
        // and when a box never compares equal to itself, as a NaN box does
        // (`TOILE-275`, `TOILE-306`).
        let groups = super::groups::bottom_up(&groups, |g| {
            self.members.get(g).map_or(&[][..], Vec::as_slice)
        });
        let mut changed = true;
        let mut passes = 0..3;
        while changed && passes.next().is_some() {
            changed = false;
            for g in &groups {
                let kids = self.members.get(g).map_or(&[][..], Vec::as_slice);
                let mut union: Option<Rect> = None;
                let mut all = !kids.is_empty();
                for k in kids {
                    match self.bounds.get(k) {
                        Some(r) => union = Some(union.map_or(*r, |u| u.union(*r))),
                        None => all = false,
                    }
                }
                let next = if all { union } else { None };
                if self.bounds.get(g).copied() == next {
                    continue;
                }
                match next {
                    Some(rect) => {
                        self.bounds.insert(g.clone(), rect);
                        self.index.update(ElementBounds::from_rect(g.clone(), rect));
                    }
                    None => {
                        self.bounds.remove(g);
                        self.index.remove(g);
                    }
                }
                changed = true;
            }
        }
    }

    /// The whole element map, resolved. Every deep read of the board goes
    /// through here, so `board_walks` counts what a command costs (`TOILE-056`).
    pub(super) fn walk_elements(&self) -> LoroValue {
        self.board_walks.set(self.board_walks.get().incremented());
        self.doc.get_map(Root::Elements.as_str()).get_deep_value()
    }

    /// Record that `parent`'s `children` list names `child` once more.
    pub(super) fn link_parent(&mut self, child: &Id<Element>, parent: &Id<Element>) {
        self.parents
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
        self.members
            .entry(parent.clone())
            .or_default()
            .push(child.clone());
    }

    /// Record that `parent`'s `children` list names `child` once less.
    pub(super) fn unlink_parent(&mut self, child: &Id<Element>, parent: &Id<Element>) {
        drop_one(&mut self.parents, child, parent);
        drop_one(&mut self.members, parent, child);
    }

    /// Drop every entry naming `child`, once the board no longer holds it.
    pub(super) fn forget_parents(&mut self, child: &Id<Element>) {
        for parent in self.parents.remove(child).unwrap_or_default() {
            drop_one(&mut self.members, &parent, child);
        }
    }

    /// Drop every entry naming `id`, as a member and as a parent, once the
    /// board no longer holds it and no list names it. A peer refreshing the
    /// removed map drops the links its list held, so the two caches agree.
    pub(super) fn forget_links(&mut self, id: &Id<Element>) {
        self.forget_parents(id);
        self.relink_members(id, Vec::new());
    }

    /// Replace what `parent`'s `children` list names with `kids`, read back
    /// from the document after a change touched it.
    fn relink_members(&mut self, parent: &Id<Element>, kids: Vec<Id<Element>>) {
        for old in self.members.get(parent).cloned().unwrap_or_default() {
            self.unlink_parent(&old, parent);
        }
        for kid in &kids {
            self.link_parent(kid, parent);
        }
    }

    /// The groups and frames whose `children` list names `id`, stale ones
    /// included. A reader filters by kind, which also skips a parent since
    /// deleted.
    pub(super) fn parents_of(&self, id: &Id<Element>) -> &[Id<Element>] {
        self.parents.get(id).map_or(&[], Vec::as_slice)
    }

    /// Re-read the `layers` list into `self.layers`, skipping malformed tags and
    /// de-duplicating (keep first) per `toile-z-order`.
    pub(super) fn rematerialize_layers(&mut self) {
        let mut seen = HashSet::default();
        let mut out = Vec::new();
        if let LoroValue::List(list) = self
            .doc
            .get_movable_list(Root::Layers.as_str())
            .get_deep_value()
        {
            for v in list.iter() {
                if let LoroValue::String(s) = v
                    && let Some(node) = NodeId::decode(s)
                    && seen.insert(node.clone())
                {
                    out.push(node);
                }
            }
        }
        self.layers = out;
    }

    /// Delete from `layers` every entry naming a node for which `keep` fails.
    /// Used by deletion cleanup. Each dropped entry is one delete at its own
    /// index, so the rest of the order is never rewritten (`TOILE-061`). An
    /// entry whose tag this build cannot decode is never named by a deletion.
    /// It is preserved verbatim (`TOILE-129`), not destroyed by an unrelated
    /// delete.
    pub(super) fn strip_layers(&self, keep: impl Fn(&NodeId) -> bool) {
        let list = self.doc.get_movable_list(Root::Layers.as_str());
        let mut dropped: Vec<usize> = Vec::new();
        if let LoroValue::List(cur) = list.get_deep_value() {
            for (i, v) in cur.iter().enumerate() {
                if let LoroValue::String(s) = v
                    && let Some(node) = NodeId::decode(s)
                    && !keep(&node)
                {
                    dropped.push(i);
                }
            }
        }
        // From the back, so each index still names the entry it was read at.
        for i in dropped.into_iter().rev() {
            let _ = list.delete(i, 1);
        }
    }

    /// The map of an element, or `None` if absent.
    pub(super) fn element_map(&self, id: &Id<Element>) -> Option<Fields<ElementKey>> {
        entry::<ElementKey>(&self.doc, id)
    }
}

/// Remove one `value` from the list `map` keeps under `key`, and the key once
/// the list is empty.
/// The stored file an image element draws, `None` for any other element or
/// for an address that does not parse.
fn picture_of(em: &Fields<ElementKey>) -> Option<Validated<ContentHash>> {
    super::derive::image_address(em).filter(|_| has_type(em, ElementKind::Image))
}

/// [`picture_of`], read off the resolved fields of a full rebuild.
fn value_picture(fields: FieldValues<'_, ElementKey>) -> Option<Validated<ContentHash>> {
    match fields.get(ElementKey::Hash) {
        Some(LoroValue::String(hash)) if value_has_type(fields, ElementKind::Image) => {
            Validated::<ContentHash>::parse(hash.as_str()).ok()
        }
        _ => None,
    }
}

fn drop_one(
    map: &mut HashMap<Id<Element>, Vec<Id<Element>>>,
    key: &Id<Element>,
    value: &Id<Element>,
) {
    if let Some(list) = map.get_mut(key) {
        if let Some(idx) = list.iter().position(|v| v == value) {
            list.remove(idx);
        }
        if list.is_empty() {
            map.remove(key);
        }
    }
}

/// What the document was written since the caches last read it, as its own
/// events report it. The board half feeds the remote refresh (`TOILE-092`).
/// The arrow half feeds the arrow cache the cull reads (`TOILE-045`).
#[derive(Debug, Default)]
pub(super) struct Touched {
    board: BoardTouched,
    pub(super) wires: WiresTouched,
}

/// The elements and the paint order a change wrote.
#[derive(Debug, Default)]
struct BoardTouched {
    /// Every element a change wrote to: its entry, or a container under it.
    elements: HashSet<Id<Element>>,
    /// Whether the paint order changed.
    layers: PaintOrder,
    /// `Whole` once a change under the element map the events do not place
    /// under one element arrives. The caches then rebuild whole.
    reach: Reach,
}

/// The arrows a change wrote.
#[derive(Debug, Default)]
pub(super) struct WiresTouched {
    pub(super) ids: HashSet<Id<Connector>>,
    /// `Whole` once a change under the arrow map the events do not place under
    /// one arrow arrives. The arrow cache then reads the whole map again.
    pub(super) reach: Reach,
}

ambre_core::define_flag! {
    /// Whether a commit changed the paint order.
    enum PaintOrder {
        Kept,
        Changed,
    }

    /// What a cache refresh reads: the entries the events named, or the whole
    /// map, when a change landed under it that no event places.
    pub(super) enum Reach {
        Entries,
        Whole,
    }
}

/// The shared sink a document subscription writes into.
pub(super) type TouchedSink = Arc<Mutex<Touched>>;

/// Subscribe to `doc`, noting into the returned sink what every commit and
/// every import writes. The subscription lives as long as the handle.
pub(super) fn watch(doc: &LoroDoc) -> (TouchedSink, Subscription) {
    let sink: TouchedSink = Arc::default();
    let into = Arc::clone(&sink);
    let subscription = doc.subscribe_root(Arc::new(move |event: DiffEvent<'_>| {
        let mut touched = into.lock().unwrap_or_else(PoisonError::into_inner);
        for change in &event.events {
            note(change, &mut touched);
        }
    }));
    (sink, subscription)
}

/// Place one container change: the paint order, or the entries of the element
/// map or the arrow map it lands under.
fn note(change: &ContainerDiff<'_>, touched: &mut Touched) {
    if matches!(change.target, ContainerID::Root { name, .. } if name.as_str() == Root::Layers.as_str())
    {
        touched.board.layers = PaintOrder::Changed;
        return;
    }
    let board = &mut touched.board;
    if let Some(keys) = keys_under(change, Root::Elements) {
        match keys {
            Some(keys) => board
                .elements
                .extend(keys.into_iter().map(Id::<Element>::from_text)),
            None => board.reach = Reach::Whole,
        }
        return;
    }
    let wires = &mut touched.wires;
    if let Some(keys) = keys_under(change, Root::Connectors) {
        match keys {
            Some(keys) => wires
                .ids
                .extend(keys.into_iter().map(Id::<Connector>::from_text)),
            None => wires.reach = Reach::Whole,
        }
    }
}

/// The entries of the root map `root` that one change wrote. They are the keys
/// it updated on the map itself, or the one entry its container sits under.
/// `None` when the change is not under `root`, and `Some(None)` when
/// it is but names no entry.
fn keys_under(change: &ContainerDiff<'_>, root: Root) -> Option<Option<Vec<String>>> {
    let is_root = |c: &ContainerID| matches!(c, ContainerID::Root { name, .. } if name.as_str() == root.as_str());
    if is_root(change.target) {
        return Some(match &change.diff {
            Diff::Map(map) => Some(map.updated.keys().map(ToString::to_string).collect()),
            _ => None,
        });
    }
    let at = change.path.iter().position(|(c, _)| is_root(c))?;
    Some(match change.path.get(at + 1) {
        Some((_, Index::Key(key))) => Some(vec![key.to_string()]),
        _ => None,
    })
}

/// Geometry a hostile or broken peer can store, which no command of ours
/// writes.
#[cfg(test)]
mod peer_geometry_tests {
    use kurbo::{Point, Rect, Size};
    use loro::ExportMode;

    use crate::logic::command::{InitContent, ToileCommand, ToileEvent};
    use crate::logic::model::{ElementKind, ShapeKind};
    use ambre_core::id::{Element, Id};

    use super::super::ToileState;
    use super::super::names::ElementKey;

    fn create(st: &mut ToileState) -> Id<Element> {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        });
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("a create names its element")
    }

    /// Write `value` under `key` of `id`'s map on `author`, behind the engine,
    /// and return the delta a peer receives.
    fn forge(
        author: &mut ToileState,
        id: &Id<Element>,
        key: ElementKey,
        value: f64,
    ) -> ambre_core::Payload<ambre_core::DeltaBytes> {
        let before = author.doc.oplog_vv();
        let em = author.element_map(id).expect("the board holds it");
        em.insert(key, value).expect("a number field");
        author.doc.commit();
        ambre_core::Payload::<ambre_core::DeltaBytes>::new(
            author
                .doc
                .export(ExportMode::updates(&before))
                .expect("a delta"),
        )
    }

    fn board_of_eight() -> (ToileState, Vec<Id<Element>>) {
        let mut st = ToileState::new();
        let ids = (0..8).map(|_| create(&mut st)).collect();
        (st, ids)
    }

    /// `TOILE-274`: a peer stores NaN, an infinity and a huge number in the
    /// box of one element of eight. The delta, a reopen and an undo panic
    /// nothing, and every box reads finite inside the world bound.
    #[test]
    fn a_peer_nan_coordinate_panics_no_index() {
        let (mut author, ids) = board_of_eight();
        let mut victim = ToileState::new();
        victim.apply_remote_snapshot(&author.export_snapshot());
        let hit = ids.get(3).expect("eight elements");
        for (key, value) in [
            (ElementKey::X, f64::NAN),
            (ElementKey::Y, f64::INFINITY),
            (ElementKey::W, f64::MAX),
            (ElementKey::Rot, f64::NAN),
        ] {
            victim.apply_remote(&forge(&mut author, hit, key, value));
        }
        let mut reopened = ToileState::new();
        reopened
            .import_snapshot(&author.export_snapshot())
            .expect("a bad value never refuses the board");
        for st in [&victim, &reopened] {
            let rect = st
                .bounds
                .get(hit)
                .copied()
                .expect("the element keeps a box");
            assert!(rect.is_finite(), "{rect:?}");
            assert_eq!((rect.x0, rect.y0), (0.0, 0.0), "NaN and inf read as zero");
            assert_eq!(rect.x1, 1_000_000.0, "a huge width is held at the bound");
            assert_eq!(
                st.index.len(),
                ambre_core::Count::<ambre_core::Elements>::new(8)
            );
        }
        victim.handle_command(ToileCommand::MoveElements {
            ids: vec![hit.clone()],
            delta: kurbo::Vec2::new(5.0, 5.0),
        });
        assert!(victim.bounds.get(hit).is_some_and(|r| r.is_finite()));
    }

    /// `TOILE-274`: a drag past the world bound stores the bound. The box
    /// read back is the box written, and a nudge after it loses nothing.
    #[test]
    fn a_drag_past_the_world_bound_holds_at_the_bound() {
        let mut st = ToileState::new();
        let id = create(&mut st);
        let nudge = |st: &mut ToileState, dx: f64| {
            st.handle_command(ToileCommand::MoveElements {
                ids: vec![id.clone()],
                delta: kurbo::Vec2::new(dx, 0.0),
            });
            let em = st.element_map(&id).expect("the board holds it");
            let stored = super::super::codec::f64_of(&em, ElementKey::X);
            let read = st.bounds.get(&id).map(|r| r.x0);
            assert_eq!(stored, read, "the box read is the box written");
            read
        };
        assert_eq!(nudge(&mut st, 1_200_000.0), Some(1_000_000.0));
        assert_eq!(nudge(&mut st, 5.0), Some(1_000_000.0), "held at the bound");
        assert_eq!(nudge(&mut st, -5.0), Some(999_995.0), "nothing lost");
    }

    /// `TOILE-275`: a group whose member a peer moved to NaN settles, on the
    /// delta and on a rebuild. A NaN box forged straight into the caches, which
    /// never compares equal to itself, still ends the fixpoint.
    #[test]
    fn a_group_over_a_nan_member_settles() {
        let (mut author, ids) = board_of_eight();
        author.handle_command(ToileCommand::GroupElements {
            ids: ids.iter().take(2).cloned().collect(),
        });
        let mut victim = ToileState::new();
        victim.apply_remote_snapshot(&author.export_snapshot());
        let first = ids.first().expect("eight elements");
        victim.apply_remote(&forge(&mut author, first, ElementKey::X, f64::NAN));
        victim.rebuild_caches();
        assert!(victim.bounds.values().all(Rect::is_finite));

        let nan = Rect::new(f64::NAN, 0.0, 1.0, 1.0);
        for id in ids.iter().take(2) {
            victim.bounds.insert(id.clone(), nan);
        }
        victim.recompute_groups();
        let touched = ids.iter().take(2).cloned().collect();
        victim.refresh_groups(&touched);
    }
}
