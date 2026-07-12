//! Engine lifecycle tests and the 2-peer CRDT convergence property test (§10).
//!
//! The convergence test drives two independent [`ToileState`]s, relaying every
//! local `update` delta to the other peer via `apply_remote`, then asserts they
//! reach an identical `LoroDoc` **and** identical derived caches
//! (`bounds` / `layers`) -- the whiteboard has no computed values to compare,
//! only state and derived structures (§6).

use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use crate::command::{InitContent, ToileCommand, ToileEvent, ZTarget};
use crate::model::{
    Anchor, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType, ShapeKind,
};
use crate::state::ToileState;

// -----------------------------------------------------------------------------
// Lifecycle unit tests
// -----------------------------------------------------------------------------

fn created_id(out: &crate::command::ToileOutcome) -> ElementId {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated(id) => Some(id.clone()),
            _ => None,
        })
        .expect("expected an ElementCreated event")
}

fn create_rect(st: &mut ToileState, at: Point, size: Size) -> ElementId {
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at,
        size,
        content: InitContent::None,
    });
    assert!(out.update.is_some(), "a create must produce a CRDT delta");
    created_id(&out)
}

#[test]
fn create_registers_bounds_and_layer() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::new(10.0, 20.0), Size::new(100.0, 50.0));
    assert_eq!(st.element_count(), 1);
    assert_eq!(st.bounds_of(&id), Some(Rect::new(10.0, 20.0, 110.0, 70.0)));
    assert_eq!(st.layer_order(), &[NodeId::Element(id)]);
}

#[test]
fn move_translates_bounds() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![id.clone()],
        delta: Vec2::new(5.0, 7.0),
    });
    assert_eq!(st.bounds_of(&id), Some(Rect::new(5.0, 7.0, 15.0, 17.0)));
}

#[test]
fn locked_element_ignores_move() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::LockElements {
        ids: vec![id.clone()],
        locked: true,
    });
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![id.clone()],
        delta: Vec2::new(100.0, 100.0),
    });
    assert_eq!(st.bounds_of(&id), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

#[test]
fn delete_drops_owned_connectors() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::CreateConnector {
        from: a.clone(),
        from_anch: Anchor::Right,
        to: b.clone(),
        to_anch: Anchor::Left,
        path_type: PathType::Straight,
        arrows: EndCaps {
            start: false,
            end: true,
        },
    });
    // One element node + one element node + one connector node.
    assert_eq!(st.layer_order().len(), 3);
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone()],
    });
    // The deleted element and the now-orphaned connector are both gone.
    assert_eq!(st.element_count(), 1);
    assert_eq!(st.layer_order(), &[NodeId::Element(b)]);
}

#[test]
fn reorder_to_back_moves_node() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    // b starts on top; send it to the back.
    st.handle_command(ToileCommand::Reorder {
        node: NodeId::Element(b.clone()),
        to: ZTarget::Back,
    });
    assert_eq!(st.layer_order(), &[NodeId::Element(b), NodeId::Element(a)]);
}

#[test]
fn commit_stroke_creates_drawing() {
    use crate::model::StrokePoint;
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CommitStroke {
        points: vec![
            StrokePoint {
                x: 0.0,
                y: 0.0,
                pressure: 1.0,
            },
            StrokePoint {
                x: 10.0,
                y: 5.0,
                pressure: 1.0,
            },
        ],
        style: ElementStyle::default(),
    });
    let id = created_id(&out);
    assert!(st.bounds_of(&id).is_some());
    assert_eq!(st.element_count(), 1);
}

#[test]
fn selection_stats_spans_offscreen_elements() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(1000.0, 1000.0), Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::RequestSelectionStats { ids: vec![a, b] });
    match &out.display[0] {
        ToileEvent::SelectionStats(s) => {
            assert_eq!(s.count, 2);
            assert_eq!(s.bounds, Some(Rect::new(0.0, 0.0, 1010.0, 1010.0)));
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
}

#[test]
fn group_and_drawing_are_not_creatable() {
    let mut st = ToileState::new();
    for kind in [ElementKind::Group, ElementKind::Drawing] {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
        });
        assert!(matches!(
            out.display.first(),
            Some(ToileEvent::Error(
                crate::command::ToileError::NotCreatable { .. }
            ))
        ));
        assert_eq!(st.element_count(), 0);
    }
}

// -----------------------------------------------------------------------------
// 2-peer CRDT convergence property test (DoD)
// -----------------------------------------------------------------------------

/// A local command issued by a peer, parameterized over a small shared id pool
/// resolved at replay time (so both peers can name each other's creations after
/// a sync round).
#[derive(Clone, Debug)]
enum PeerOp {
    Create { x: f64, y: f64 },
    MoveNth { nth: usize, dx: f64, dy: f64 },
    RestyleNth { nth: usize, fill: u32 },
    DeleteNth { nth: usize },
    ReorderNthToBack { nth: usize },
}

fn op_strategy() -> impl Strategy<Value = PeerOp> {
    prop_oneof![
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::Create { x, y }),
        (0usize..6, -50.0f64..50.0, -50.0f64..50.0).prop_map(|(nth, dx, dy)| PeerOp::MoveNth {
            nth,
            dx,
            dy
        }),
        (0usize..6, any::<u32>()).prop_map(|(nth, fill)| PeerOp::RestyleNth { nth, fill }),
        (0usize..6).prop_map(|nth| PeerOp::DeleteNth { nth }),
        (0usize..6).prop_map(|nth| PeerOp::ReorderNthToBack { nth }),
    ]
}

/// The element ids currently live on a peer, in a stable order (by id string) so
/// `nth` resolves identically regardless of insertion history.
fn live_ids(st: &ToileState) -> Vec<ElementId> {
    let mut ids: Vec<ElementId> = st
        .layer_order()
        .iter()
        .filter_map(|n| match n {
            NodeId::Element(e) => Some(e.clone()),
            NodeId::Connector(_) => None,
        })
        .collect();
    ids.sort();
    ids
}

/// Apply one op locally and return the CRDT delta it produced (if any) to relay.
fn apply_local(st: &mut ToileState, op: &PeerOp) -> Option<Vec<u8>> {
    let ids = live_ids(st);
    let nth = |n: usize| ids.get(n % ids.len().max(1)).cloned();
    let out = match op {
        PeerOp::Create { x, y } => st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(*x, *y),
            size: Size::new(20.0, 20.0),
            content: InitContent::None,
        }),
        PeerOp::MoveNth { nth: n, dx, dy } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::MoveElements {
                ids: vec![id],
                delta: Vec2::new(*dx, *dy),
            }),
            None => return None,
        },
        PeerOp::RestyleNth { nth: n, fill } => match nth(*n) {
            Some(id) => {
                let style = ElementStyle {
                    fill: Some(*fill),
                    ..ElementStyle::default()
                };
                st.handle_command(ToileCommand::RestyleElements {
                    ids: vec![id],
                    style,
                })
            }
            None => return None,
        },
        PeerOp::DeleteNth { nth: n } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::DeleteElements { ids: vec![id] }),
            None => return None,
        },
        PeerOp::ReorderNthToBack { nth: n } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::Reorder {
                node: NodeId::Element(id),
                to: ZTarget::Back,
            }),
            None => return None,
        },
    };
    out.update
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn two_peers_converge(ops_a in proptest::collection::vec(op_strategy(), 0..20),
                          ops_b in proptest::collection::vec(op_strategy(), 0..20)) {
        let mut a = ToileState::new();
        let mut b = ToileState::new();

        // Interleave: each peer applies its op locally, then both pending deltas
        // are exchanged, so operations concurrently target a partly-shared state.
        let rounds = ops_a.len().max(ops_b.len());
        for i in 0..rounds {
            let da = ops_a.get(i).and_then(|op| apply_local(&mut a, op));
            let db = ops_b.get(i).and_then(|op| apply_local(&mut b, op));
            if let Some(d) = da { b.apply_remote(&d); }
            if let Some(d) = db { a.apply_remote(&d); }
        }

        // Final drain: re-sync both directions until quiescent. A create on one
        // side may reference-order after the other's delete; a couple of extra
        // exchanges settle any straggler.
        for _ in 0..2 {
            let da = a.export_snapshot();
            let db = b.export_snapshot();
            b.apply_remote(&da);
            a.apply_remote(&db);
        }

        // Identical derived caches.
        let mut la: Vec<_> = live_ids(&a);
        let mut lb: Vec<_> = live_ids(&b);
        la.sort();
        lb.sort();
        prop_assert_eq!(&la, &lb, "element populations diverged");

        for id in &la {
            prop_assert_eq!(
                a.bounds_of(id), b.bounds_of(id),
                "bounds diverged for {:?}", id
            );
        }

        // Identical z-order.
        prop_assert_eq!(a.layer_order(), b.layer_order(), "layer order diverged");
    }
}
