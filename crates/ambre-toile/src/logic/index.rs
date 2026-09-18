//! Spatial index: an `rstar` R-tree over element AABBs for O(log N + k)
//! viewport culling and point hit pre-filtering.
//!
//! The tree is a **derived cache**, never authoritative: it is rebuilt from
//! `LoroDoc` on load and maintained incrementally on every mutation. A full
//! rebuild must yield the same visible set as the incremental path -- the
//! index-equivalence invariant, proven by the property test at the bottom.

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
#[derive(Default)]
pub struct SpatialIndex {
    tree: RTree<ElementBounds>,
}

impl SpatialIndex {
    /// O(N) bulk build at load; asserted identical to the incremental path.
    pub fn build(elems: impl Iterator<Item = ElementBounds>) -> Self {
        Self {
            tree: RTree::bulk_load(elems.collect()),
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

    pub fn insert(&mut self, b: ElementBounds) {
        self.tree.insert(b);
    }

    /// Remove the leaf carrying `id`, if present.
    pub fn remove(&mut self, id: &ElementId) {
        // rstar removes by geometric match, and we cannot locate by id alone. So
        // we find the leaf whose id matches. A linear scan is acceptable: remove
        // is rare relative to culling, and the tree stores only current elements.
        let found = self.tree.iter().find(|e| &e.id == id).cloned();
        if let Some(found) = found {
            self.tree.remove(&found);
        }
    }

    /// Replace the leaf for `b.id` (remove then insert) -- the geometry changed.
    pub fn update(&mut self, b: ElementBounds) {
        self.remove(&b.id);
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
        self.tree.iter().map(|e| e.id.clone())
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
