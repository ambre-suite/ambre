//! Engine lifecycle tests and the 2-peer CRDT convergence property test (§10).
//!
//! The convergence test drives two independent [`ToileState`]s, relaying every
//! local `update` delta to the other peer via `apply_remote`, then asserts they
//! reach an identical `LoroDoc` **and** identical derived caches
//! (`bounds` / `layers`) -- the whiteboard has no computed values to compare,
//! only state and derived structures (§6).

use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use crate::command::{DisplayNode, End, InitContent, ToileCommand, ToileEvent, ZTarget};
use crate::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType,
    ShapeKind, StrokePoint,
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
// Groups (§2.F bis)
// -----------------------------------------------------------------------------

#[test]
fn group_unions_bounds_and_sits_below_members() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(90.0, 90.0), Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    });
    let g = created_id(&out);
    // Derived bounds span both members.
    assert_eq!(st.bounds_of(&g), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    // The group node sits below both members in paint order.
    assert_eq!(
        st.layer_order(),
        &[NodeId::Element(g), NodeId::Element(a), NodeId::Element(b),]
    );
}

#[test]
fn moving_a_group_translates_its_members() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![g.clone()],
        delta: Vec2::new(5.0, 5.0),
    });
    assert_eq!(st.bounds_of(&a), Some(Rect::new(5.0, 5.0, 15.0, 15.0)));
    assert_eq!(st.bounds_of(&b), Some(Rect::new(25.0, 5.0, 35.0, 15.0)));
    assert_eq!(st.bounds_of(&g), Some(Rect::new(5.0, 5.0, 35.0, 15.0)));
}

#[test]
fn ungroup_keeps_children_and_order() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    st.handle_command(ToileCommand::UngroupElements { group: g.clone() });
    // The group node is gone; both children survive in their prior order.
    assert!(st.bounds_of(&g).is_none());
    assert_eq!(st.layer_order(), &[NodeId::Element(a), NodeId::Element(b)]);
}

#[test]
fn deleting_a_group_cascades_to_members() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    st.handle_command(ToileCommand::DeleteElements { ids: vec![g] });
    assert_eq!(st.element_count(), 0);
    assert!(st.layer_order().is_empty());
}

// -----------------------------------------------------------------------------
// Frames: membership + z-order invariant (§2.F / §2.G)
// -----------------------------------------------------------------------------

fn create_frame(st: &mut ToileState, at: Point, size: Size) -> ElementId {
    created_id(&st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Frame,
        at,
        size,
        content: InitContent::None,
    }))
}

#[test]
fn set_frame_children_adds_and_removes_membership() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(30.0, 10.0), Size::new(10.0, 10.0));

    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone(), b.clone()],
        remove: vec![],
    });
    assert_eq!(st.membership_of(&f), vec![a.clone(), b.clone()]);

    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![],
        remove: vec![a.clone()],
    });
    assert_eq!(st.membership_of(&f), vec![b]);
}

#[test]
fn frame_children_paint_after_their_frame() {
    let mut st = ToileState::new();
    // Create the children first so they sit *below* the later-created frame.
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(30.0, 10.0), Size::new(10.0, 10.0));
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));

    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone(), b.clone()],
        remove: vec![],
    });

    let order = st.layer_order();
    let idx = |id: &ElementId| {
        order
            .iter()
            .position(|n| n == &NodeId::Element(id.clone()))
            .unwrap()
    };
    assert!(idx(&f) < idx(&a), "frame must paint before child a");
    assert!(idx(&f) < idx(&b), "frame must paint before child b");
}

#[test]
fn set_frame_children_detaches_from_a_group() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(30.0, 10.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));

    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });
    assert_eq!(st.membership_of(&f), vec![a.clone()]);
    // `a` left the group; only `b` remains bound to it.
    assert_eq!(st.membership_of(&g), vec![b]);
}

#[test]
fn set_frame_children_is_one_undo_item() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });
    assert_eq!(st.membership_of(&f), vec![a]);
    st.handle_command(ToileCommand::Undo);
    assert!(
        st.membership_of(&f).is_empty(),
        "one undo reverts the whole membership change"
    );
}

// -----------------------------------------------------------------------------
// Clipboard: copy / paste / duplicate (§8 ter)
// -----------------------------------------------------------------------------

fn clip_of(out: &crate::command::ToileOutcome) -> crate::command::ClipboardPayload {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::Clipboard(p) => Some(p.clone()),
            _ => None,
        })
        .expect("expected a Clipboard event")
}

fn created_ids(out: &crate::command::ToileOutcome) -> Vec<ElementId> {
    out.display
        .iter()
        .filter_map(|e| match e {
            ToileEvent::ElementCreated(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn connector_count(st: &ToileState) -> usize {
    st.layer_order()
        .iter()
        .filter(|n| matches!(n, NodeId::Connector(_)))
        .count()
}

#[test]
fn copy_does_not_mutate_and_paste_remints_offset() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(20.0, 20.0));

    // Copy reads the selection without touching the doc.
    let copy = st.handle_command(ToileCommand::CopyElements {
        ids: vec![a.clone()],
    });
    assert!(copy.update.is_none(), "copy must not produce a CRDT delta");
    assert_eq!(st.element_count(), 1);
    let payload = clip_of(&copy);
    assert_eq!(payload.origin, Rect::new(10.0, 10.0, 30.0, 30.0));

    // Paste so `origin` (top-left 10,10) lands at (110, 210): +100, +200.
    let paste = st.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(110.0, 210.0),
    });
    assert!(paste.update.is_some(), "paste must broadcast a CRDT delta");
    assert_eq!(st.element_count(), 2);
    let pasted = created_ids(&paste);
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], a, "pasted element must get a fresh id");
    assert_eq!(
        st.bounds_of(&pasted[0]),
        Some(Rect::new(110.0, 210.0, 130.0, 230.0))
    );
}

#[test]
fn paste_keeps_fully_copied_connector_drops_half_copied() {
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
    assert_eq!(connector_count(&st), 1);

    // Copying only one endpoint's element drops the connector (it would dangle).
    let half = clip_of(&st.handle_command(ToileCommand::CopyElements {
        ids: vec![a.clone()],
    }));
    assert!(half.connectors.is_empty());
    st.handle_command(ToileCommand::PasteElements {
        payload: half,
        at: Point::new(0.0, 200.0),
    });
    assert_eq!(connector_count(&st), 1, "no new connector from a half copy");

    // Copying both endpoints carries the connector, re-minted onto fresh ids.
    let both = clip_of(&st.handle_command(ToileCommand::CopyElements { ids: vec![a, b] }));
    assert_eq!(both.connectors.len(), 1);
    st.handle_command(ToileCommand::PasteElements {
        payload: both,
        at: Point::new(0.0, 400.0),
    });
    assert_eq!(
        connector_count(&st),
        2,
        "the copied connector is reproduced"
    );
}

#[test]
fn duplicate_a_group_is_one_undo_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements { ids: vec![a, b] }));
    assert_eq!(st.element_count(), 3);

    // Duplicating the group re-mints the group and both members (subtree copy).
    st.handle_command(ToileCommand::DuplicateElements {
        ids: vec![g],
        delta: Vec2::new(200.0, 0.0),
    });
    assert_eq!(st.element_count(), 6);

    // The whole duplicate collapses under a single undo.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), 3);
}

#[test]
fn clipboard_payload_postcard_roundtrips() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(90.0, 90.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    });
    let payload = clip_of(&st.handle_command(ToileCommand::CopyElements { ids: vec![a, b] }));

    let bytes = postcard::to_allocvec(&payload).expect("serialize");
    let back: crate::command::ClipboardPayload = postcard::from_bytes(&bytes).expect("deserialize");
    assert_eq!(back, payload, "clipboard payload must round-trip");
}

#[test]
fn paste_reproduces_selection_on_a_separate_board() {
    // Cross-board portability (§8 ter DoD): copy from one board, paste into a
    // fresh one and the selection is reproduced.
    let mut src = ToileState::new();
    let a = create_rect(&mut src, Point::new(5.0, 5.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut src, Point::new(25.0, 5.0), Size::new(10.0, 10.0));
    let payload = clip_of(&src.handle_command(ToileCommand::CopyElements { ids: vec![a, b] }));

    let mut dst = ToileState::new();
    assert_eq!(dst.element_count(), 0);
    let out = dst.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(5.0, 5.0), // origin top-left is (5,5): zero offset
    });
    assert_eq!(dst.element_count(), 2);
    assert_eq!(created_ids(&out).len(), 2);
}

// -----------------------------------------------------------------------------
// Undo / redo (§8 bis)
// -----------------------------------------------------------------------------

#[test]
fn undo_reverts_create_and_redo_restores() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    assert_eq!(st.element_count(), 1);

    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), 0);
    assert!(st.bounds_of(&id).is_none());

    st.handle_command(ToileCommand::Redo);
    assert_eq!(st.element_count(), 1);
    assert_eq!(st.bounds_of(&id), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

#[test]
fn undo_reverts_group_as_one_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    });
    assert_eq!(st.element_count(), 3);
    // A single undo drops the whole group, leaving the two members intact.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), 2);
    assert_eq!(st.layer_order(), &[NodeId::Element(a), NodeId::Element(b)]);
}

// -----------------------------------------------------------------------------
// Transforms, connectors, viewport (§3 / §5 / §9)
// -----------------------------------------------------------------------------

fn connect(st: &mut ToileState, from: ElementId, to: ElementId) -> ConnectorId {
    st.handle_command(ToileCommand::CreateConnector {
        from,
        from_anch: Anchor::Right,
        to,
        to_anch: Anchor::Left,
        path_type: PathType::Straight,
        arrows: EndCaps {
            start: false,
            end: true,
        },
    });
    st.connector_ids()
        .pop()
        .expect("a connector must have been created")
}

fn connector_node_count(st: &ToileState) -> usize {
    st.debug_viewport()
        .nodes
        .iter()
        .filter(|n| matches!(n, DisplayNode::Connector(_)))
        .count()
}

#[test]
fn rotate_expands_the_aabb() {
    let mut st = ToileState::new();
    // A 100x50 rect rotated 90 deg about its center (50,25) spans 50x100, so its
    // rotation-expanded AABB is (25,-25)..(75,75) -- the §3 invariant.
    let id = create_rect(&mut st, Point::ZERO, Size::new(100.0, 50.0));
    st.handle_command(ToileCommand::RotateElement {
        id: id.clone(),
        radians: std::f64::consts::FRAC_PI_2,
    });
    let b = st.bounds_of(&id).expect("bounds");
    assert!((b.x0 - 25.0).abs() < 1e-6, "x0 = {}", b.x0);
    assert!((b.y0 + 25.0).abs() < 1e-6, "y0 = {}", b.y0);
    assert!((b.x1 - 75.0).abs() < 1e-6, "x1 = {}", b.x1);
    assert!((b.y1 - 75.0).abs() < 1e-6, "y1 = {}", b.y1);
}

#[test]
fn resize_updates_bounds() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::ResizeElement {
        id: id.clone(),
        new: Rect::new(5.0, 5.0, 45.0, 25.0),
    });
    assert_eq!(st.bounds_of(&id), Some(Rect::new(5.0, 5.0, 45.0, 25.0)));
}

#[test]
fn retarget_rebinds_endpoint_without_dangling() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let c = create_rect(&mut st, Point::new(200.0, 0.0), Size::new(10.0, 10.0));
    let conn = connect(&mut st, a, b);
    let out = st.handle_command(ToileCommand::RetargetConnector {
        id: conn,
        endpoint: End::To,
        to: c,
        anch: Anchor::Left,
    });
    assert!(
        !out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::DanglingConnector(_))),
        "retarget to a live element must not dangle"
    );
    // Still routes (both endpoints present), so it paints.
    assert_eq!(connector_node_count(&st), 1);
}

#[test]
fn retarget_to_missing_element_reports_dangling() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let conn = connect(&mut st, a, b);
    let out = st.handle_command(ToileCommand::RetargetConnector {
        id: conn.clone(),
        endpoint: End::To,
        to: ElementId("ffff_0000".to_owned()), // never minted
        anch: Anchor::Left,
    });
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::DanglingConnector(c) if *c == conn)),
        "retarget to a missing element must surface DanglingConnector"
    );
    // A dangling connector routes to None and is dropped from the paint set (§5.A).
    assert_eq!(connector_node_count(&st), 0);
}

#[test]
fn deleting_an_endpoint_makes_the_connector_dangle() {
    // §5.A: an endpoint deletion leaves the connector unroutable; it is skipped
    // at derivation (route -> None), never corrupting state.
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let c = create_rect(&mut st, Point::new(200.0, 0.0), Size::new(10.0, 10.0));
    // c is not an endpoint, so deleting a does not GC this connector locally.
    let _conn = connect(&mut st, b.clone(), c);
    assert_eq!(connector_node_count(&st), 1);
    // Delete an unrelated element first to keep the connector alive, then delete
    // one of its endpoints and confirm it drops from the paint set.
    st.handle_command(ToileCommand::DeleteElements { ids: vec![a] });
    assert_eq!(connector_node_count(&st), 1);
    st.handle_command(ToileCommand::DeleteElements { ids: vec![b] });
    assert_eq!(connector_node_count(&st), 0);
}

#[test]
fn viewport_culls_offscreen_nodes() {
    // §9: derivation only emits the visible set. Default viewport spans
    // roughly (-640,-400)..(640,400), so a node at (5000,5000) is culled.
    let mut st = ToileState::new();
    let near = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let _far = create_rect(&mut st, Point::new(5000.0, 5000.0), Size::new(10.0, 10.0));
    let painted: Vec<ElementId> = st
        .debug_viewport()
        .nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e.id.clone()),
            DisplayNode::Connector(_) => None,
        })
        .collect();
    assert_eq!(painted, vec![near], "only the on-screen node is painted");
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
    ResizeNth { nth: usize, w: f64, h: f64 },
    RotateNth { nth: usize, radians: f64 },
    RestyleNth { nth: usize, fill: u32 },
    DeleteNth { nth: usize },
    ReorderNthToBack { nth: usize },
    // Layers / children-heavy paths -- where concurrent `LoroMovableList`
    // ordering and derived-cache rebuilds are most likely to diverge.
    GroupNth { i: usize, j: usize },
    UngroupNth { nth: usize },
    ConnectNth { i: usize, j: usize },
    Stroke { x: f64, y: f64 },
}

fn op_strategy() -> impl Strategy<Value = PeerOp> {
    prop_oneof![
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::Create { x, y }),
        (0usize..6, -50.0f64..50.0, -50.0f64..50.0).prop_map(|(nth, dx, dy)| PeerOp::MoveNth {
            nth,
            dx,
            dy
        }),
        (0usize..6, 5.0f64..80.0, 5.0f64..80.0).prop_map(|(nth, w, h)| PeerOp::ResizeNth {
            nth,
            w,
            h
        }),
        (0usize..6, -3.0f64..3.0).prop_map(|(nth, radians)| PeerOp::RotateNth { nth, radians }),
        (0usize..6, any::<u32>()).prop_map(|(nth, fill)| PeerOp::RestyleNth { nth, fill }),
        (0usize..6).prop_map(|nth| PeerOp::DeleteNth { nth }),
        (0usize..6).prop_map(|nth| PeerOp::ReorderNthToBack { nth }),
        (0usize..6, 0usize..6).prop_map(|(i, j)| PeerOp::GroupNth { i, j }),
        (0usize..6).prop_map(|nth| PeerOp::UngroupNth { nth }),
        (0usize..6, 0usize..6).prop_map(|(i, j)| PeerOp::ConnectNth { i, j }),
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::Stroke { x, y }),
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
        PeerOp::ResizeNth { nth: n, w, h } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::ResizeElement {
                id,
                new: Rect::new(0.0, 0.0, *w, *h),
            }),
            None => return None,
        },
        PeerOp::RotateNth { nth: n, radians } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::RotateElement {
                id,
                radians: *radians,
            }),
            None => return None,
        },
        PeerOp::GroupNth { i, j } => match (nth(*i), nth(*j)) {
            (Some(a), Some(b)) if a != b => {
                st.handle_command(ToileCommand::GroupElements { ids: vec![a, b] })
            }
            _ => return None,
        },
        PeerOp::UngroupNth { nth: n } => match nth(*n) {
            // A no-op on a non-group returns no delta and is filtered by `out.update`.
            Some(id) => st.handle_command(ToileCommand::UngroupElements { group: id }),
            None => return None,
        },
        PeerOp::ConnectNth { i, j } => match (nth(*i), nth(*j)) {
            (Some(a), Some(b)) => st.handle_command(ToileCommand::CreateConnector {
                from: a,
                from_anch: Anchor::Right,
                to: b,
                to_anch: Anchor::Left,
                path_type: PathType::Straight,
                arrows: EndCaps {
                    start: false,
                    end: true,
                },
            }),
            _ => return None,
        },
        PeerOp::Stroke { x, y } => st.handle_command(ToileCommand::CommitStroke {
            points: vec![
                StrokePoint {
                    x: *x as f32,
                    y: *y as f32,
                    pressure: 1.0,
                },
                StrokePoint {
                    x: *x as f32 + 10.0,
                    y: *y as f32 + 5.0,
                    pressure: 1.0,
                },
            ],
            style: ElementStyle::default(),
        }),
    };
    out.update
}

/// A viewport wide enough that nothing is culled, so `debug_viewport()` returns
/// the full derived paint set for cross-peer / rebuild comparison.
fn wide_viewport() -> crate::index::Viewport {
    crate::index::Viewport {
        center: Point::ZERO,
        zoom: 0.01,
        size: Size::new(2000.0, 2000.0),
    }
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

        // Comprehensive check: the full derived paint set (elements + routed
        // connectors + group union bounds) must match under an identical, wide
        // viewport. Debug equality captures every DisplayNode field.
        a.handle_command(ToileCommand::SetViewport(wide_viewport()));
        b.handle_command(ToileCommand::SetViewport(wide_viewport()));
        prop_assert_eq!(
            format!("{:?}", a.debug_viewport()),
            format!("{:?}", b.debug_viewport()),
            "derived paint set diverged"
        );
    }

    /// The §1 invariant beyond the isolated `SpatialIndex`: every derived cache
    /// held by `ToileState` (bounds, layers, group unions, routed connectors)
    /// must equal what a full rebuild from the CRDT snapshot produces. `b` is
    /// built purely through `apply_remote` (the `rebuild_caches` path), so this
    /// asserts incremental == rebuild across the whole command surface.
    #[test]
    fn caches_match_a_full_rebuild(ops in proptest::collection::vec(op_strategy(), 0..40)) {
        let mut a = ToileState::new();
        for op in &ops {
            let _ = apply_local(&mut a, op);
        }
        let mut b = ToileState::new();
        b.apply_remote(&a.export_snapshot());

        a.handle_command(ToileCommand::SetViewport(wide_viewport()));
        b.handle_command(ToileCommand::SetViewport(wide_viewport()));

        prop_assert_eq!(a.element_count(), b.element_count(), "element count diverged from rebuild");
        prop_assert_eq!(a.layer_order(), b.layer_order(), "layers diverged from rebuild");
        prop_assert_eq!(
            format!("{:?}", a.debug_viewport()),
            format!("{:?}", b.debug_viewport()),
            "derived caches diverged from a full rebuild"
        );
    }
}
