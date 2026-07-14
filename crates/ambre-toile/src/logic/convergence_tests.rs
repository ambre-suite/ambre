//! Engine lifecycle tests and the 2-peer CRDT convergence property test (§10).
//!
//! The convergence test drives two independent [`ToileState`]s, relaying every
//! local `update` delta to the other peer via `apply_remote`, then asserts they
//! reach an identical `LoroDoc` **and** identical derived caches
//! (`bounds` / `layers`) -- the whiteboard has no computed values to compare,
//! only state and derived structures (§6).

use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use ambre_bus::UserId;

use crate::logic::autolayout::{Align, AutoLayout, Direction, Sizing};
use crate::logic::command::{
    DisplayNode, End, InitContent, PresencePayload, Reaction, ReactionKind, TextDelta,
    ToileCommand, ToileError, ToileEvent, ZTarget,
};
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType,
    ShapeKind, StrokePoint,
};
use crate::logic::state::ToileState;

// -----------------------------------------------------------------------------
// Lifecycle unit tests
// -----------------------------------------------------------------------------

fn created_id(out: &crate::logic::command::ToileOutcome) -> ElementId {
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

/// Read an element's plaintext body from the derived viewport (Phase 0 stores
/// it as `DisplayGeom::Text`).
fn body_of(st: &ToileState, id: &ElementId) -> Option<String> {
    st.debug_viewport().nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => match &e.geom {
            crate::logic::command::DisplayGeom::Text(t) => Some(t.clone()),
            _ => None,
        },
        _ => None,
    })
}

#[test]
fn set_text_overwrites_sticky_body() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Sticky,
        at: Point::new(0.0, 0.0),
        size: Size::new(120.0, 80.0),
        content: InitContent::Text("hello".to_owned()),
    });
    let id = created_id(&out);
    assert_eq!(body_of(&st, &id).as_deref(), Some("hello"));

    let out = st.handle_command(ToileCommand::SetText {
        id: id.clone(),
        text: "bonjour monde".to_owned(),
    });
    assert!(
        out.update.is_some(),
        "a text edit must produce a CRDT delta"
    );
    assert_eq!(body_of(&st, &id).as_deref(), Some("bonjour monde"));
}

#[test]
fn set_text_rejects_a_non_text_element() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::SetText {
        id,
        text: "nope".to_owned(),
    });
    assert!(
        out.update.is_none(),
        "rejected edit must not mutate the doc"
    );
    assert!(matches!(
        out.display.first(),
        Some(ToileEvent::Error(
            crate::logic::command::ToileError::ContentKindMismatch
        ))
    ));
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
fn selection_stats_empty_ids_span_whole_board() {
    // Empty ids = whole-board extent, for View -> Fit (`toile-ui §5quinquies.E`):
    // the union of every element's bounds, regardless of the (UI-only) selection.
    let mut st = ToileState::new();
    let _a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let _b = create_rect(&mut st, Point::new(500.0, 300.0), Size::new(40.0, 20.0));
    let out = st.handle_command(ToileCommand::RequestSelectionStats { ids: vec![] });
    match &out.display[0] {
        ToileEvent::SelectionStats(s) => {
            assert_eq!(s.count, 2);
            assert_eq!(s.bounds, Some(Rect::new(0.0, 0.0, 540.0, 320.0)));
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
}

#[test]
fn selection_stats_empty_ids_on_empty_board_is_none() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::RequestSelectionStats { ids: vec![] });
    match &out.display[0] {
        ToileEvent::SelectionStats(s) => {
            assert_eq!(s.count, 0);
            assert_eq!(s.bounds, None);
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
                crate::logic::command::ToileError::NotCreatable { .. }
            ))
        ));
        assert_eq!(st.element_count(), 0);
    }
}

// -----------------------------------------------------------------------------
// Find (board text search, §8 quater)
// -----------------------------------------------------------------------------

fn create_sticky(st: &mut ToileState, at: Point, body: &str) -> ElementId {
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Sticky,
        at,
        size: Size::new(80.0, 80.0),
        content: InitContent::Text(body.to_owned()),
    });
    created_id(&out)
}

fn find(
    st: &mut ToileState,
    query: &str,
    options: crate::logic::command::FindOptions,
    from: Option<ElementId>,
) -> crate::logic::command::FindResults {
    find_dir(st, query, options, from, false)
}

fn find_dir(
    st: &mut ToileState,
    query: &str,
    options: crate::logic::command::FindOptions,
    from: Option<ElementId>,
    backward: bool,
) -> crate::logic::command::FindResults {
    let out = st.handle_command(ToileCommand::Find {
        query: query.to_owned(),
        options,
        from,
        backward,
    });
    assert!(out.update.is_none(), "Find must not mutate the CRDT");
    out.display
        .into_iter()
        .find_map(|e| match e {
            ToileEvent::FindResults(r) => Some(r),
            _ => None,
        })
        .expect("expected a FindResults event")
}

#[test]
fn find_matches_in_z_order_and_is_case_insensitive_by_default() {
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "Hello world");
    let _b = create_sticky(&mut st, Point::new(200.0, 0.0), "nothing here");
    let c = create_sticky(&mut st, Point::new(400.0, 0.0), "another HELLO");

    let opts = crate::logic::command::FindOptions::default();
    let r = find(&mut st, "hello", opts, None);
    // Both stickies match (case folded); hits follow the `layers` paint order.
    assert_eq!(r.hits, vec![a.clone(), c.clone()]);
    // A fresh search focuses the first hit.
    assert_eq!(r.focus, Some(a));
}

#[test]
fn find_focus_wraps_after_from() {
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "match");
    let b = create_sticky(&mut st, Point::new(200.0, 0.0), "match");

    let opts = crate::logic::command::FindOptions::default();
    // Resuming from the first hit focuses the second.
    let r = find(&mut st, "match", opts, Some(a.clone()));
    assert_eq!(r.focus, Some(b.clone()));
    // Resuming from the last hit wraps to the first.
    let r = find(&mut st, "match", opts, Some(b.clone()));
    assert_eq!(r.focus, Some(a.clone()));
}

#[test]
fn find_backward_steps_to_previous_hit_wrapping() {
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "match");
    let b = create_sticky(&mut st, Point::new(200.0, 0.0), "match");

    let opts = crate::logic::command::FindOptions::default();
    // Backward from the second hit focuses the first.
    let r = find_dir(&mut st, "match", opts, Some(b.clone()), true);
    assert_eq!(r.focus, Some(a.clone()));
    // Backward from the first hit wraps to the last.
    let r = find_dir(&mut st, "match", opts, Some(a), true);
    assert_eq!(r.focus, Some(b));
}

#[test]
fn find_honors_match_case_and_whole_word() {
    let mut st = ToileState::new();
    let exact = create_sticky(&mut st, Point::ZERO, "Cat");
    let _lower = create_sticky(&mut st, Point::new(200.0, 0.0), "cat");
    let _substr = create_sticky(&mut st, Point::new(400.0, 0.0), "Cathedral");

    let case = crate::logic::command::FindOptions {
        match_case: true,
        whole_word: false,
    };
    let r = find(&mut st, "Cat", case, None);
    // Case-sensitive: "Cat" and "Cathedral" (substring) match, "cat" does not.
    assert_eq!(r.hits.len(), 2);

    let whole = crate::logic::command::FindOptions {
        match_case: true,
        whole_word: true,
    };
    let r = find(&mut st, "Cat", whole, None);
    // Whole-word + case: only the standalone "Cat" survives.
    assert_eq!(r.hits, vec![exact]);
}

#[test]
fn find_ignores_non_text_elements_and_empty_query() {
    let mut st = ToileState::new();
    let _rect = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    create_sticky(&mut st, Point::new(200.0, 0.0), "findme");

    let opts = crate::logic::command::FindOptions::default();
    // A shape carries no searchable text.
    assert!(find(&mut st, "rect", opts, None).hits.is_empty());
    // An empty query clears the result set (the UI's "panel closed" signal).
    let r = find(&mut st, "", opts, None);
    assert!(r.hits.is_empty());
    assert!(r.focus.is_none());
    // A real match still resolves.
    assert_eq!(find(&mut st, "findme", opts, None).hits.len(), 1);
}

// -----------------------------------------------------------------------------
// Ephemeral presence (§7)
// -----------------------------------------------------------------------------

fn peer_presence(events: Vec<ToileEvent>) -> crate::logic::command::PresenceUpdate {
    events
        .into_iter()
        .find_map(|e| match e {
            ToileEvent::PeerPresence(u) => Some(u),
            _ => None,
        })
        .expect("expected a PeerPresence event")
}

#[test]
fn identity_is_seeded_and_read_back() {
    // #1: my_user_id is `None` until SessionReady, then the dispatcher seeds it.
    let mut st = ToileState::new();
    assert!(st.user_id().is_none());
    let me = UserId::new();
    st.set_user_id(me);
    assert_eq!(st.user_id(), Some(me));
}

#[test]
fn presence_command_broadcasts_ephemeral_only() {
    // A local presence sample rides the ephemeral leg: no display event, no CRDT
    // delta (the UI draws its own cursor).
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "note");
    let payload = PresencePayload {
        cursor: Some(Point::new(12.0, 34.0)),
        selection: vec![a],
        live_stroke: None,
        laser: Some(Point::new(50.0, 60.0)),
    };
    let out = st.handle_command(ToileCommand::Presence(payload));
    assert!(out.display.is_empty(), "presence emits no display event");
    assert!(out.update.is_none(), "presence is never a CRDT delta");
    assert!(
        out.ephemeral.is_some(),
        "presence must broadcast an ephemeral frame"
    );
}

#[test]
fn apply_ephemeral_keys_presence_by_user_id_without_touching_history() {
    // #2 / §13 DoD: an inbound presence frame yields a PeerPresence keyed by the
    // resolved UserId and mutates neither the doc nor its history.
    let mut sender = ToileState::new();
    let payload = PresencePayload {
        cursor: Some(Point::new(7.0, 8.0)),
        selection: Vec::new(),
        live_stroke: Some(vec![StrokePoint {
            x: 1.0,
            y: 2.0,
            pressure: 1.0,
        }]),
        laser: None,
    };
    let frame = sender
        .handle_command(ToileCommand::Presence(payload.clone()))
        .ephemeral
        .expect("presence frame");

    let mut rx = ToileState::new();
    let before = rx.export_snapshot();
    let count_before = rx.element_count();

    let alice = UserId::new();
    let update = peer_presence(rx.apply_ephemeral(alice, &frame));
    assert_eq!(update.user_id, alice);
    assert_eq!(update.presence, payload);
    assert_eq!(rx.peer_presence(&alice), Some(&payload));

    // Non-mutating: the CRDT snapshot and element set are untouched (§7).
    assert_eq!(rx.export_snapshot(), before);
    assert_eq!(rx.element_count(), count_before);

    // A second peer is tracked independently, keyed by its own UserId.
    let bob = UserId::new();
    let bob_frame = sender
        .handle_command(ToileCommand::Presence(PresencePayload::default()))
        .ephemeral
        .expect("presence frame");
    let _ = rx.apply_ephemeral(bob, &bob_frame);
    assert!(rx.peer_presence(&alice).is_some());
    assert_eq!(rx.peer_presence(&bob), Some(&PresencePayload::default()));
}

#[test]
fn apply_ephemeral_drops_a_malformed_frame() {
    let mut rx = ToileState::new();
    let events = rx.apply_ephemeral(UserId::new(), &[0xFF, 0x00, 0x13, 0x37]);
    assert!(
        events.is_empty(),
        "a malformed presence frame yields no event"
    );
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
fn derive_populates_the_group_edge_on_members() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(90.0, 90.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    let vp = st.debug_viewport();
    let group_of = |id: &ElementId| {
        vp.nodes.iter().find_map(|n| match n {
            DisplayNode::Element(e) if &e.id == id => Some(e.group.clone()),
            _ => None,
        })
    };
    // Both members carry the enclosing group edge; the group node itself is
    // top-level (no parent group).
    assert_eq!(group_of(&a), Some(Some(g.clone())));
    assert_eq!(group_of(&b), Some(Some(g.clone())));
    assert_eq!(group_of(&g), Some(None));
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
// Smart Animate snapshot derivation (`toile-features` §1.C)
// -----------------------------------------------------------------------------

#[test]
fn frame_snapshot_captures_declared_children_props() {
    use crate::logic::animate::smart_animate;

    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(10.0, 20.0), Size::new(30.0, 40.0));
    let b = create_rect(&mut st, Point::new(60.0, 10.0), Size::new(10.0, 10.0));

    // Give `a` a distinctive style + rotation so the snapshot picks them up.
    let style = ElementStyle {
        fill: Some(0xFF12_3456),
        opacity: 0.5,
        ..ElementStyle::default()
    };
    st.handle_command(ToileCommand::RestyleElements {
        ids: vec![a.clone()],
        style,
    });
    st.handle_command(ToileCommand::RotateElement {
        id: a.clone(),
        radians: 1.0,
    });

    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone(), b.clone()],
        remove: vec![],
    });

    let snap = st.frame_snapshot(&f);
    // Membership is the declared `children`, in order.
    assert_eq!(
        snap.elements
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![a.clone(), b.clone()]
    );
    let sa = snap.by_id(&a).expect("a captured");
    assert_eq!(sa.bounds, Rect::new(10.0, 20.0, 40.0, 60.0));
    assert_eq!(sa.rotation, 1.0);
    assert_eq!(sa.opacity, 0.5);
    assert_eq!(sa.fill, Some(0xFF12_3456));

    // A frame with no children yields an empty snapshot; a transition between an
    // empty and the populated frame fades every child in (endpoint sanity).
    let empty = st.frame_snapshot(&b); // b is a plain rect, no children
    assert!(empty.elements.is_empty());
    let anim = smart_animate(&empty, &snap, 1.0);
    assert_eq!(anim.len(), 2);
    assert!(
        anim.iter()
            .any(|e| e.id == a && (e.opacity - 0.5).abs() < 1e-6)
    );
}

// -----------------------------------------------------------------------------
// Clipboard: copy / paste / duplicate (§8 ter)
// -----------------------------------------------------------------------------

fn clip_of(out: &crate::logic::command::ToileOutcome) -> crate::logic::command::ClipboardPayload {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::Clipboard(p) => Some(p.clone()),
            _ => None,
        })
        .expect("expected a Clipboard event")
}

fn created_ids(out: &crate::logic::command::ToileOutcome) -> Vec<ElementId> {
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
    let back: crate::logic::command::ClipboardPayload =
        postcard::from_bytes(&bytes).expect("deserialize");
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
// Presentation: slide sequence CRDT + playback (`toile-features` §1)
// -----------------------------------------------------------------------------

fn slide_changed(out: &crate::logic::command::ToileOutcome) -> Option<usize> {
    out.display.iter().find_map(|e| match e {
        ToileEvent::SlideChanged(c) => Some(c.index),
        _ => None,
    })
}

#[test]
fn set_slide_order_produces_delta_and_is_undoable() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));

    let out = st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![b.clone(), a.clone()],
    });
    assert!(
        out.update.is_some(),
        "the slide sequence is CRDT and must broadcast a delta"
    );
    assert_eq!(st.slide_order(), vec![b.clone(), a.clone()]);

    // The whole reorder collapses under a single undo (§8 bis).
    st.handle_command(ToileCommand::Undo);
    assert!(
        st.slide_order().is_empty(),
        "one undo reverts the whole slide order"
    );
}

#[test]
fn slide_order_dedups_repeated_frames() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone(), a.clone()],
    });
    // The repeated first frame is dropped, keeping first occurrence (§1.A).
    assert_eq!(st.slide_order(), vec![a, b]);
}

#[test]
fn playback_bounds() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    let c = create_frame(&mut st, Point::new(600.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a, b, c],
    });

    // Next saturates at the last slide.
    assert_eq!(
        slide_changed(&st.handle_command(ToileCommand::NextSlide)),
        Some(1)
    );
    assert_eq!(
        slide_changed(&st.handle_command(ToileCommand::NextSlide)),
        Some(2)
    );
    assert_eq!(
        slide_changed(&st.handle_command(ToileCommand::NextSlide)),
        Some(2),
        "Next past the last slide stays put"
    );
    // Prev saturates at the first slide.
    st.handle_command(ToileCommand::PrevSlide);
    st.handle_command(ToileCommand::PrevSlide);
    assert_eq!(
        slide_changed(&st.handle_command(ToileCommand::PrevSlide)),
        Some(0),
        "Prev before the first slide stays put"
    );
    // Goto clamps to the deck length.
    assert_eq!(
        slide_changed(&st.handle_command(ToileCommand::GotoSlide(99))),
        Some(2)
    );
}

fn slide_change(out: &crate::logic::command::ToileOutcome) -> crate::logic::command::SlideChange {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::SlideChanged(c) => Some(c.clone()),
            _ => None,
        })
        .expect("a playback command emits a SlideChanged")
}

#[test]
fn slide_change_carries_snapshots_and_fit_rect() {
    // The camera pass needs both frame snapshots + the target rect in one shot,
    // since the UI holds no board state (toile-features §1.C / §1.D).
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone()],
    });

    let change = slide_change(&st.handle_command(ToileCommand::NextSlide));
    assert_eq!(change.index, 1);
    // `from` is slide 0, `to` is slide 1, each the engine's own frame snapshot.
    assert_eq!(change.from, st.frame_snapshot(&a));
    assert_eq!(change.to, st.frame_snapshot(&b));
    // The fit target is the new active frame's world rect.
    assert_eq!(change.to_rect, st.element_rect(&b));
    assert_eq!(
        change.to_rect,
        Some(kurbo::Rect::new(300.0, 0.0, 500.0, 200.0))
    );
}

#[test]
fn enter_present_starts_from_empty_snapshot() {
    // Entering present mode has no source frame: `from` is empty so slide 0 fades
    // in rather than tweening from a stale frame (§1.C).
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone()],
    });

    let change = slide_change(&st.handle_command(ToileCommand::EnterPresent { focus: true }));
    assert_eq!(change.index, 0);
    assert!(change.from.elements.is_empty(), "no source frame on entry");
    assert_eq!(change.to, st.frame_snapshot(&a));
    assert_eq!(change.to_rect, st.element_rect(&a));
}

#[test]
fn present_mode_toggle_is_local() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder { order: vec![a] });
    st.handle_command(ToileCommand::NextSlide); // cursor is not at 0

    let out = st.handle_command(ToileCommand::EnterPresent { focus: true });
    assert!(
        out.update.is_none(),
        "present mode is local, never a CRDT delta"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentModeChanged(true))),
        "EnterPresent announces present mode"
    );
    assert_eq!(
        slide_changed(&out),
        Some(0),
        "EnterPresent resets to slide 0"
    );
    assert_eq!(st.playback(), (true, 0, true));

    let out = st.handle_command(ToileCommand::ExitPresent);
    assert!(out.update.is_none());
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentModeChanged(false)))
    );
    assert!(!st.playback().0, "ExitPresent leaves present mode");
}

#[test]
fn playback_is_not_synced() {
    // Two peers sharing a deck: A drives its own playback; B never sees it, since
    // playback carries no CRDT delta (§1.B).
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let f0 = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let f1 = create_frame(&mut a, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    a.handle_command(ToileCommand::SetSlideOrder {
        order: vec![f0, f1],
    });
    // Bring B fully up to date (frames + sequence) so it shares the deck.
    b.apply_remote(&a.export_snapshot());
    assert_eq!(a.slide_order(), b.slide_order(), "the sequence is shared");

    // A enters present mode and steps forward; neither produces a delta to relay.
    assert!(
        a.handle_command(ToileCommand::EnterPresent { focus: false })
            .update
            .is_none()
    );
    assert!(a.handle_command(ToileCommand::NextSlide).update.is_none());
    assert_eq!(a.playback(), (true, 1, false));
    assert_eq!(
        b.playback(),
        (false, 0, false),
        "B's playback is untouched by A's presentation"
    );
}

// -----------------------------------------------------------------------------
// Speaker notes (toile-features §1.D / §5)
// -----------------------------------------------------------------------------

/// A single-splice insertion at `at` (no deletion), the common editor edit.
fn insert_notes(at: usize, s: &str) -> TextDelta {
    TextDelta {
        at,
        delete: 0,
        insert: s.to_owned(),
    }
}

#[test]
fn edit_notes_produces_delta_and_is_undoable() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));

    let out = st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: insert_notes(0, "Intro slide"),
    });
    assert!(
        out.update.is_some(),
        "speaker notes are CRDT and must broadcast a delta"
    );
    assert_eq!(st.notes(&f), "Intro slide");

    // A second splice appends " -- talk track" after the existing text.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: insert_notes(11, " -- talk track"),
    });
    assert_eq!(st.notes(&f), "Intro slide -- talk track");

    // Each EditNotes is one undo item (§8 bis): undo drops the last splice only.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.notes(&f), "Intro slide", "one undo reverts one splice");
    st.handle_command(ToileCommand::Redo);
    assert_eq!(
        st.notes(&f),
        "Intro slide -- talk track",
        "redo re-applies it"
    );
}

#[test]
fn edit_notes_clamps_out_of_range_splice() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: insert_notes(0, "abc"),
    });
    // An `at` past the end is clamped to the length: the insert lands at the tail
    // rather than being rejected.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: insert_notes(999, "Z"),
    });
    assert_eq!(st.notes(&f), "abcZ");
    // An oversized deletion span is clamped to what remains.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: TextDelta {
            at: 0,
            delete: 999,
            insert: String::new(),
        },
    });
    assert_eq!(st.notes(&f), "");
}

#[test]
fn edit_notes_rejects_non_frame() {
    let mut st = ToileState::new();
    let sticky = create_sticky(&mut st, Point::ZERO, "body");

    // A sticky carries no `children` field, so it is not a frame: rejected no-op.
    let out = st.handle_command(ToileCommand::EditNotes {
        frame: sticky.clone(),
        delta: insert_notes(0, "nope"),
    });
    assert!(out.update.is_none(), "a rejected EditNotes emits no delta");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::ContentKindMismatch))),
        "notes on a non-frame report a ContentKindMismatch"
    );
    assert_eq!(st.notes(&sticky), "", "no notes were written to the sticky");

    // A missing element reports NoSuchElement.
    let out = st.handle_command(ToileCommand::EditNotes {
        frame: ElementId("ghost".to_owned()),
        delta: insert_notes(0, "nope"),
    });
    assert!(out.update.is_none());
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::NoSuchElement(_)))),
    );
}

#[test]
fn concurrent_notes_edits_converge() {
    // Two peers share a frame (and thus its seeded notes LoroText, §1.D), then
    // edit the notes concurrently at different positions. The char-CRDT must
    // converge: both peers reach the same notes string after a sync round.
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let f = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    // Bring B up to date so both share the same frame + notes container.
    b.apply_remote(&a.export_snapshot());
    assert_eq!(a.notes(&f), "");
    assert_eq!(b.notes(&f), "");

    // Concurrent, non-overlapping edits, each unaware of the other.
    let da = a
        .handle_command(ToileCommand::EditNotes {
            frame: f.clone(),
            delta: insert_notes(0, "AAA"),
        })
        .update
        .expect("A's notes edit broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::EditNotes {
            frame: f.clone(),
            delta: insert_notes(0, "BBB"),
        })
        .update
        .expect("B's notes edit broadcasts a delta");

    // Relay each peer's delta to the other.
    b.apply_remote(&da);
    a.apply_remote(&db);

    // Char-CRDT convergence: identical final notes on both peers, and both edits
    // survived (no LWW loss of a rival container).
    assert_eq!(a.notes(&f), b.notes(&f), "concurrent notes edits diverged");
    let merged = a.notes(&f);
    assert!(
        merged.contains("AAA") && merged.contains("BBB"),
        "both concurrent insertions must survive, got {merged:?}"
    );
}

// -----------------------------------------------------------------------------
// Auto-layout spec register (toile-features §2 / §5)
// -----------------------------------------------------------------------------

fn a_layout(direction: Direction, gap: f64) -> AutoLayout {
    AutoLayout {
        direction,
        gap,
        padding: [4.0, 4.0, 4.0, 4.0],
        align: Align::Start,
        sizing: Sizing::Hug,
    }
}

#[test]
fn set_auto_layout_add_change_clear() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    assert_eq!(st.auto_layout(&f), None, "a fresh frame has no layout spec");

    // Add a spec.
    let out = st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: Some(a_layout(Direction::Horizontal, 8.0)),
    });
    assert!(
        out.update.is_some(),
        "an auto-layout spec is CRDT and must broadcast"
    );
    assert_eq!(
        st.auto_layout(&f),
        Some(a_layout(Direction::Horizontal, 8.0))
    );

    // Change it (LWW register).
    st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: Some(a_layout(Direction::Vertical, 12.0)),
    });
    assert_eq!(
        st.auto_layout(&f),
        Some(a_layout(Direction::Vertical, 12.0))
    );

    // One SetAutoLayout is one undo item (§8 bis): undo reverts to the prior spec.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.auto_layout(&f),
        Some(a_layout(Direction::Horizontal, 8.0))
    );

    // Clear it.
    st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: None,
    });
    assert_eq!(st.auto_layout(&f), None, "clearing removes the spec");

    // A no-op clear (already absent) broadcasts nothing.
    let out = st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: None,
    });
    assert!(out.update.is_none(), "a no-op clear broadcasts nothing");
}

#[test]
fn set_auto_layout_rejects_non_frame() {
    let mut st = ToileState::new();
    let sticky = create_sticky(&mut st, Point::ZERO, "note");
    let out = st.handle_command(ToileCommand::SetAutoLayout {
        frame: sticky.clone(),
        layout: Some(a_layout(Direction::Horizontal, 8.0)),
    });
    assert!(
        out.update.is_none(),
        "a rejected SetAutoLayout emits no delta"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::ContentKindMismatch))),
        "auto-layout on a non-frame reports a ContentKindMismatch"
    );
    assert_eq!(st.auto_layout(&sticky), None);

    // A missing element is likewise a rejected no-op.
    let out = st.handle_command(ToileCommand::SetAutoLayout {
        frame: ElementId("ghost".to_owned()),
        layout: Some(a_layout(Direction::Horizontal, 8.0)),
    });
    assert!(out.update.is_none());
}

#[test]
fn concurrent_auto_layout_edits_converge() {
    // Two peers share a frame, then set different auto-layout specs concurrently.
    // The LWW register must converge: both peers agree on one surviving spec.
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let f = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::SetAutoLayout {
            frame: f.clone(),
            layout: Some(a_layout(Direction::Horizontal, 8.0)),
        })
        .update
        .expect("A's layout edit broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::SetAutoLayout {
            frame: f.clone(),
            layout: Some(a_layout(Direction::Wrap, 16.0)),
        })
        .update
        .expect("B's layout edit broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    // LWW convergence: identical surviving spec on both peers.
    assert_eq!(
        a.auto_layout(&f),
        b.auto_layout(&f),
        "concurrent auto-layout specs diverged"
    );
    assert!(
        a.auto_layout(&f).is_some(),
        "one spec must survive the merge"
    );
}

// -----------------------------------------------------------------------------
// Reactions / votes (toile-features §4.A / §5)
// -----------------------------------------------------------------------------

fn emoji(s: &str) -> Reaction {
    Reaction {
        kind: ReactionKind::Emoji(s.to_owned()),
        ts: 0,
    }
}

fn vote(w: i32) -> Reaction {
    Reaction {
        kind: ReactionKind::Vote(w),
        ts: 0,
    }
}

/// Seed a state with a canonical `UserId` (as the dispatcher does on
/// `SessionReady`) and a target element to react on. Returns the target id.
fn state_with_user(user: UserId) -> (ToileState, ElementId) {
    let mut st = ToileState::new();
    st.set_user_id(user);
    let target = create_sticky(&mut st, Point::ZERO, "target");
    (st, target)
}

#[test]
fn react_add_change_remove_updates_tally() {
    let (mut st, t) = state_with_user(UserId::new());

    // Add an emoji reaction.
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("thumbsup")),
    });
    let tally = st.reaction_tally(&t);
    assert_eq!(tally.emoji, vec![("thumbsup".to_owned(), 1)]);
    assert_eq!(tally.vote_count, 0);

    // Change the same user's reaction: one entry per user, so it replaces (LWW on
    // the own key), never accumulates.
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(vote(1)),
    });
    let tally = st.reaction_tally(&t);
    assert!(
        tally.emoji.is_empty(),
        "the emoji entry was replaced, not kept"
    );
    assert_eq!((tally.vote_total, tally.vote_count), (1, 1));
    assert_eq!(st.my_reaction(&t), Some(vote(1)));

    // Remove the user's own entry.
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: None,
    });
    assert_eq!(st.reaction_tally(&t), Default::default());
    assert_eq!(st.my_reaction(&t), None);
}

#[test]
fn react_broadcasts_delta_and_is_undoable() {
    let (mut st, t) = state_with_user(UserId::new());

    let out = st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("heart")),
    });
    assert!(
        out.update.is_some(),
        "a reaction is CRDT and must broadcast a delta"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::ReactionsUpdated(id) if id == &t)),
        "a reaction emits ReactionsUpdated for its target"
    );

    // One React is one undo item (§8 bis): undo drops the reaction.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.reaction_tally(&t),
        Default::default(),
        "undo reverts the reaction"
    );

    // A no-op remove (nothing to remove) emits no delta and no commit.
    let out = st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: None,
    });
    assert!(out.update.is_none(), "a no-op remove broadcasts nothing");
}

#[test]
fn react_before_session_ready_is_a_noop() {
    // Without a canonical UserId there is no per-user key to write under.
    let mut st = ToileState::new();
    let t = create_sticky(&mut st, Point::ZERO, "target");
    assert!(st.user_id().is_none());

    let out = st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("thumbsup")),
    });
    assert!(out.update.is_none(), "no identity means nothing is written");
    assert_eq!(st.reaction_tally(&t), Default::default());
}

#[test]
fn tally_skips_a_deleted_target() {
    let (mut st, t) = state_with_user(UserId::new());
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("thumbsup")),
    });
    assert_eq!(st.reaction_tally(&t).emoji.len(), 1);

    // Deleting the target skips its reaction sub-map at derivation (§4.A): the
    // orphan bytes are never destroyed, only not rendered.
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![t.clone()],
    });
    assert_eq!(
        st.reaction_tally(&t),
        Default::default(),
        "a deleted target's reactions are skipped at derivation"
    );
}

#[test]
fn concurrent_reactions_distinct_users_converge() {
    // Two different users react on the same target concurrently. Per-user keys
    // mean both entries survive; the tally counts both, with no double-count.
    let alice = UserId::new();
    let bob = UserId::new();
    let mut a = ToileState::new();
    a.set_user_id(alice);
    let t = create_sticky(&mut a, Point::ZERO, "target");

    let mut b = ToileState::new();
    b.set_user_id(bob);
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::React {
            target: t.clone(),
            reaction: Some(emoji("thumbsup")),
        })
        .update
        .expect("A's reaction broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::React {
            target: t.clone(),
            reaction: Some(emoji("thumbsup")),
        })
        .update
        .expect("B's reaction broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let ta = a.reaction_tally(&t);
    assert_eq!(ta, b.reaction_tally(&t), "concurrent reactions diverged");
    assert_eq!(
        ta.emoji,
        vec![("thumbsup".to_owned(), 2)],
        "two distinct users must both count, no double-count"
    );
}

#[test]
fn same_user_two_devices_is_one_entry() {
    // The same account reacting from two devices (two sessions / PeerIDs) must
    // collapse to a single entry keyed by the canonical UserId (§4.A) -- never a
    // double-count.
    let me = UserId::new();
    let mut a = ToileState::new();
    a.set_user_id(me);
    let t = create_sticky(&mut a, Point::ZERO, "target");

    let mut b = ToileState::new();
    b.set_user_id(me);
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::React {
            target: t.clone(),
            reaction: Some(vote(1)),
        })
        .update
        .expect("device A's reaction broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::React {
            target: t.clone(),
            reaction: Some(vote(1)),
        })
        .update
        .expect("device B's reaction broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let ta = a.reaction_tally(&t);
    assert_eq!(
        ta,
        b.reaction_tally(&t),
        "same-user two-device tally diverged"
    );
    assert_eq!(
        ta.vote_count, 1,
        "two devices of one user is a single entry, not two votes"
    );
}

// -----------------------------------------------------------------------------
// 2-peer CRDT convergence property test (DoD)
// -----------------------------------------------------------------------------

/// A local command issued by a peer, parameterized over a small shared id pool
/// resolved at replay time (so both peers can name each other's creations after
/// a sync round).
#[derive(Clone, Debug)]
enum PeerOp {
    Create {
        x: f64,
        y: f64,
    },
    MoveNth {
        nth: usize,
        dx: f64,
        dy: f64,
    },
    ResizeNth {
        nth: usize,
        w: f64,
        h: f64,
    },
    RotateNth {
        nth: usize,
        radians: f64,
    },
    RestyleNth {
        nth: usize,
        fill: u32,
    },
    DeleteNth {
        nth: usize,
    },
    ReorderNthToBack {
        nth: usize,
    },
    // Layers / children-heavy paths -- where concurrent `LoroMovableList`
    // ordering and derived-cache rebuilds are most likely to diverge.
    GroupNth {
        i: usize,
        j: usize,
    },
    UngroupNth {
        nth: usize,
    },
    ConnectNth {
        i: usize,
        j: usize,
    },
    Stroke {
        x: f64,
        y: f64,
    },
    // Presentation sequence -- concurrent `LoroMovableList<FrameId>` reorders.
    CreateFrame {
        x: f64,
        y: f64,
    },
    SetSlideOrderRot {
        by: usize,
    },
    // Auto-layout spec -- concurrent LWW register edits on a shared frame (§2).
    SetAutoLayoutNth {
        nth: usize,
        dir: u8,
        gap: f64,
    },
    // Reactions -- concurrent per-user entries on a shared target (§4.A). Each
    // peer holds a distinct `UserId`, so two reactions on one target must both
    // survive and the derived tally must converge.
    ReactNth {
        nth: usize,
        emoji: bool,
        weight: i32,
    },
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
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::CreateFrame { x, y }),
        (0usize..6).prop_map(|by| PeerOp::SetSlideOrderRot { by }),
        (0usize..6, 0u8..3, 0.0f64..24.0).prop_map(|(nth, dir, gap)| PeerOp::SetAutoLayoutNth {
            nth,
            dir,
            gap
        }),
        (0usize..6, any::<bool>(), -2i32..3).prop_map(|(nth, emoji, weight)| PeerOp::ReactNth {
            nth,
            emoji,
            weight
        }),
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
        PeerOp::CreateFrame { x, y } => st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Frame,
            at: Point::new(*x, *y),
            size: Size::new(200.0, 200.0),
            content: InitContent::None,
        }),
        PeerOp::SetSlideOrderRot { by } => {
            // Rotate the current live-id set to build a permutation, so concurrent
            // peers set overlapping-but-different slide orders (§1.A convergence).
            if ids.is_empty() {
                return None;
            }
            let k = by % ids.len();
            let mut order = ids.clone();
            order.rotate_left(k);
            st.handle_command(ToileCommand::SetSlideOrder { order })
        }
        PeerOp::SetAutoLayoutNth { nth: n, dir, gap } => match nth(*n) {
            // A non-frame target is a rejected no-op (filtered by `out.update`),
            // so this only exercises the register on real frames.
            Some(id) => {
                let direction = [Direction::Horizontal, Direction::Vertical, Direction::Wrap]
                    [(*dir % 3) as usize];
                st.handle_command(ToileCommand::SetAutoLayout {
                    frame: id,
                    layout: Some(a_layout(direction, *gap)),
                })
            }
            None => return None,
        },
        PeerOp::ReactNth {
            nth: n,
            emoji: is_emoji,
            weight,
        } => match nth(*n) {
            Some(id) => {
                let reaction = if *is_emoji {
                    // A tiny emoji alphabet so distinct peers sometimes agree
                    // (merged count) and sometimes differ (distinct rows).
                    let set = ["up", "down", "heart"];
                    emoji(set[(*weight).rem_euclid(3) as usize])
                } else {
                    vote(*weight)
                };
                st.handle_command(ToileCommand::React {
                    target: id,
                    reaction: Some(reaction),
                })
            }
            None => return None,
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
fn wide_viewport() -> crate::logic::index::Viewport {
    crate::logic::index::Viewport {
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
        // Distinct canonical identities (as `SessionReady` would seed), so a
        // `ReactNth` on each peer writes under its own key: concurrent reactions
        // on one target must both survive (§4.A).
        a.set_user_id(UserId::new());
        b.set_user_id(UserId::new());

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

        // Identical slide sequence: concurrent `SetSlideOrder` reorders must
        // converge on the shared `LoroMovableList<FrameId>` (§1.A).
        prop_assert_eq!(a.slide_order(), b.slide_order(), "slide order diverged");

        // Identical auto-layout spec on every surviving element: concurrent
        // `SetAutoLayout` edits converge on the shared LWW register (§2 / §6).
        for id in &la {
            prop_assert_eq!(
                a.auto_layout(id), b.auto_layout(id),
                "auto-layout spec diverged for {:?}", id
            );
        }

        // Identical derived reaction tally on every surviving target: concurrent
        // per-user reactions converge and no vote is double-counted (§4.A / §6).
        for id in &la {
            prop_assert_eq!(
                a.reaction_tally(id), b.reaction_tally(id),
                "reaction tally diverged for {:?}", id
            );
        }

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
        // Seed an identity so `ReactNth` ops actually write (else a no-op), so the
        // rebuilt peer `b` exercises the reactions map on the snapshot path too.
        a.set_user_id(UserId::new());
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
