//! Spatial index: an `rstar` R-tree over element AABBs for O(log N + k)
//! viewport culling and point hit pre-filtering.
//!
//! The tree is a **derived cache**, never authoritative: it is rebuilt from
//! `LoroDoc` on load and maintained incrementally on every mutation. A full
//! rebuild must yield the same visible set as the incremental path -- the
//! index-equivalence invariant, proven by the property test at the bottom.

use std::collections::HashMap;

use kurbo::{Point, Rect};
use rstar::{AABB, RTree, RTreeObject};

use crate::logic::model::ElementId;

/// The camera / viewport type lives in the `ambre-bus` IPC contract, carried by
/// `ToileCommand::SetViewport`. It is re-exported here so the index keeps its
/// `crate::logic::index::Viewport` path and its culling helpers.
pub use ambre_bus::app::toile::Viewport;

/// R-tree leaf: an element's world-space, rotation-expanded AABB plus its id.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementBounds {
    pub id: ElementId,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl ElementBounds {
    /// Build from a `kurbo::Rect` (already rotation-expanded by the caller).
    pub fn from_rect(id: ElementId, r: Rect) -> Self {
        Self {
            id,
            min: [r.x0, r.y0],
            max: [r.x1, r.y1],
        }
    }
}

impl RTreeObject for ElementBounds {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(self.min, self.max)
    }
}

/// R-tree wrapper. `remove` / `update` keep the tree in step with local and
/// remote mutations; `query_visible` / `query_point` read it.
///
/// `rstar` removes a leaf by geometric match, so the wrapper keeps each id's
/// current leaf beside the tree. A removal then descends only the nodes whose
/// envelope holds that leaf, and never walks the index (`TOILE-091`).
#[derive(Default)]
pub struct SpatialIndex {
    tree: RTree<ElementBounds>,
    leaves: HashMap<ElementId, ElementBounds>,
    /// How many times a call walked every leaf. A test probe: a drag step
    /// or a deletion must leave it at zero.
    #[cfg(test)]
    walks: std::cell::Cell<usize>,
}

impl SpatialIndex {
    /// O(N) bulk build at load; asserted identical to the incremental path.
    /// A repeated id keeps its last leaf.
    pub fn build(elems: impl Iterator<Item = ElementBounds>) -> Self {
        let leaves: HashMap<ElementId, ElementBounds> = elems.map(|b| (b.id.clone(), b)).collect();
        Self {
            tree: RTree::bulk_load(leaves.values().cloned().collect()),
            leaves,
            #[cfg(test)]
            walks: std::cell::Cell::new(0),
        }
    }

    /// O(log N + k). Ids are returned unordered; the caller paint-orders them
    /// through `layers`.
    pub fn query_visible<'a>(&'a self, vp: &Viewport) -> impl Iterator<Item = ElementId> + 'a {
        let r = vp.to_world_rect();
        self.tree
            .locate_in_envelope_intersecting(&AABB::from_corners([r.x0, r.y0], [r.x1, r.y1]))
            .map(|e| e.id.clone())
    }

    /// Point hit pre-filter (screen->world done by caller). Exact hit-test
    /// (rotation, path fill, stroke width) is refined in `toile-selection`.
    pub fn query_point<'a>(&'a self, p: Point) -> impl Iterator<Item = ElementId> + 'a {
        // Our leaves are rectangles, not points, so `locate_all_at_point` (which
        // needs a `Point` object) does not apply. A degenerate point-envelope
        // intersection query gives every leaf whose AABB contains `p`.
        self.tree
            .locate_in_envelope_intersecting(&AABB::from_corners([p.x, p.y], [p.x, p.y]))
            .map(|e| e.id.clone())
    }

    /// Add the leaf for `b.id`, replacing the one it already had.
    pub fn insert(&mut self, b: ElementBounds) {
        self.update(b);
    }

    /// Remove the leaf carrying `id`, if present. O(log N) for leaves that do
    /// not overlap much: the descent follows the leaf's own envelope.
    pub fn remove(&mut self, id: &ElementId) {
        let Some(leaf) = self.leaves.remove(id) else {
            return;
        };
        if self.tree.remove(&leaf).is_none() {
            // A leaf whose corners are not numbers fails every envelope test,
            // so the descent cannot find it. Drop it by walking the tree: the
            // one path that does, and only for a leaf that was never drawable.
            self.walked();
            let rest: Vec<ElementBounds> = self.tree.drain().filter(|e| &e.id != id).collect();
            self.tree = RTree::bulk_load(rest);
        }
    }

    /// Replace the leaf for `b.id` (remove then insert) -- the geometry changed.
    pub fn update(&mut self, b: ElementBounds) {
        self.remove(&b.id);
        self.leaves.insert(b.id.clone(), b.clone());
        self.tree.insert(b);
    }

    /// Number of leaves. Test helper.
    pub fn len(&self) -> usize {
        self.tree.size()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }

    /// Every id in the tree, unordered. Test / rebuild helper.
    pub fn ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.walked();
        self.tree.iter().map(|e| e.id.clone())
    }

    #[cfg(test)]
    fn walked(&self) {
        self.walks.set(self.walks.get() + 1);
    }

    #[cfg(not(test))]
    fn walked(&self) {}

    /// How many calls walked every leaf so far. Test probe.
    #[cfg(test)]
    pub(crate) fn walks(&self) -> usize {
        self.walks.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Size` only feeds the `Viewport` fixtures below; the production code paths
    // in this module need just `Point` / `Rect`.
    use kurbo::Size;

    fn bounds(id: &str, x: f64, y: f64, w: f64, h: f64) -> ElementBounds {
        ElementBounds::from_rect(ElementId(id.to_owned()), Rect::new(x, y, x + w, y + h))
    }

    #[test]
    fn query_visible_culls_offscreen() {
        let idx = SpatialIndex::build(
            [
                bounds("a", 0.0, 0.0, 10.0, 10.0),
                bounds("b", 10_000.0, 10_000.0, 10.0, 10.0),
            ]
            .into_iter(),
        );
        let vp = Viewport {
            center: Point::new(5.0, 5.0),
            zoom: 1.0,
            size: Size::new(100.0, 100.0),
        };
        let visible: Vec<_> = idx.query_visible(&vp).collect();
        assert_eq!(visible, vec![ElementId("a".to_owned())]);
    }

    #[test]
    fn remove_then_query_is_empty() {
        let mut idx = SpatialIndex::default();
        idx.insert(bounds("a", 0.0, 0.0, 10.0, 10.0));
        idx.remove(&ElementId("a".to_owned()));
        assert!(idx.is_empty());
    }

    /// `TOILE-091`: a drag step and a deletion find their leaf by its own
    /// envelope. Over a board of 5 000 leaves, neither walks the index, and the
    /// tree still holds exactly what the moves left.
    #[test]
    fn a_drag_and_a_delete_walk_no_index() {
        let grid = |i: usize, dx: f64| {
            bounds(
                &format!("e{i}"),
                (i % 100) as f64 * 20.0 + dx,
                (i / 100) as f64 * 20.0,
                10.0,
                10.0,
            )
        };
        let mut idx = SpatialIndex::build((0..5_000).map(|i| grid(i, 0.0)));
        for i in (0..5_000).step_by(50) {
            idx.update(grid(i, 3.0));
        }
        for i in (25..5_000).step_by(50) {
            idx.remove(&ElementId(format!("e{i}")));
        }
        assert_eq!(idx.walks(), 0, "no call walked the index");
        assert_eq!(idx.len(), 4_900);
        let moved = Viewport {
            center: Point::new(8.0, 5.0),
            zoom: 1.0,
            size: Size::new(2.0, 2.0),
        };
        let hit: Vec<_> = idx.query_visible(&moved).collect();
        assert_eq!(hit, vec![ElementId("e0".to_owned())], "the leaf moved");
    }

    /// A leaf with corners that are not numbers still leaves the index on its
    /// removal, through the one walk kept for it.
    #[test]
    fn a_leaf_that_is_not_a_number_still_leaves() {
        let mut idx = SpatialIndex::default();
        idx.insert(bounds("a", 0.0, 0.0, 10.0, 10.0));
        idx.insert(bounds("nan", f64::NAN, 0.0, 10.0, 10.0));
        idx.remove(&ElementId("nan".to_owned()));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.walks(), 1);
    }
}

/// Index-equivalence property test (mandatory): for a random op sequence,
/// the incrementally maintained tree must return the same visible set as a
/// from-scratch bulk rebuild. Host-only (proptest does not build on wasm32).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod prop {
    use super::*;
    use kurbo::Size;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    /// A single mutation over a small element set keyed 0..8.
    #[derive(Clone, Debug)]
    enum Op {
        Upsert {
            key: u8,
            x: f64,
            y: f64,
            w: f64,
            h: f64,
        },
        Remove {
            key: u8,
        },
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            (
                0u8..8,
                -500.0f64..500.0,
                -500.0f64..500.0,
                1.0f64..200.0,
                1.0f64..200.0
            )
                .prop_map(|(key, x, y, w, h)| Op::Upsert { key, x, y, w, h }),
            (0u8..8).prop_map(|key| Op::Remove { key }),
        ]
    }

    fn eid(key: u8) -> ElementId {
        ElementId(format!("k{key}"))
    }

    proptest! {
        #[test]
        fn incremental_matches_rebuild(ops in proptest::collection::vec(op_strategy(), 0..64)) {
            let mut incremental = SpatialIndex::default();
            // The authoritative model the tree is supposed to mirror.
            let mut model: BTreeMap<u8, ElementBounds> = BTreeMap::new();

            for op in ops {
                match op {
                    Op::Upsert { key, x, y, w, h } => {
                        let b = ElementBounds::from_rect(eid(key), Rect::new(x, y, x + w, y + h));
                        model.insert(key, b.clone());
                        incremental.update(b);
                    }
                    Op::Remove { key } => {
                        model.remove(&key);
                        incremental.remove(&eid(key));
                    }
                }
            }

            let rebuilt = SpatialIndex::build(model.values().cloned());

            // Same population.
            let mut a: Vec<_> = incremental.ids().collect();
            let mut b: Vec<_> = rebuilt.ids().collect();
            a.sort();
            b.sort();
            prop_assert_eq!(&a, &b);

            // Same visible set on a probe viewport that covers the populated area.
            let vp = Viewport {
                center: Point::ZERO,
                zoom: 1.0,
                size: Size::new(2000.0, 2000.0),
            };
            let mut va: Vec<_> = incremental.query_visible(&vp).collect();
            let mut vb: Vec<_> = rebuilt.query_visible(&vp).collect();
            va.sort();
            vb.sort();
            prop_assert_eq!(va, vb);
        }
    }
}
