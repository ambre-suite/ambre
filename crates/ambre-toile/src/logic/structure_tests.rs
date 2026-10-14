//! The shape of a board, driven through the engine's command surface. It
//! covers the birth of an element, the lock, groups and frame membership. It
//! also covers the paint order, the culling camera and the ids.

use std::collections::HashSet;

use kurbo::{Point, Rect, Size, Vec2};
use loro::LoroValue;

use crate::logic::command::{
    DisplayNode, InitContent, ToileCommand, ToileError, ToileEvent, ToileOutcome, ZTarget,
};
use crate::logic::index::Viewport;
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType,
    ShapeKind, StrokePoint,
};
use crate::logic::state::{CONNECTORS, ELEMENTS, ToileState};

fn create(st: &mut ToileState, kind: ElementKind, at: Point, size: Size) -> ToileOutcome {
    let content = match kind {
        ElementKind::Sticky | ElementKind::Text => InitContent::Text(String::new()),
        _ => InitContent::None,
    };
    st.handle_command(ToileCommand::CreateElement {
        kind,
        at,
        size,
        content,
        fill: None,
        correlate: 0,
    })
}

fn created(out: &ToileOutcome) -> ElementId {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("expected an ElementCreated event")
}

fn rect(st: &mut ToileState, x: f64, y: f64) -> ElementId {
    let out = create(
        st,
        ElementKind::Shape(ShapeKind::Rect),
        Point::new(x, y),
        Size::new(10.0, 10.0),
    );
    created(&out)
}

fn frame(st: &mut ToileState, x: f64, y: f64) -> ElementId {
    let out = create(
        st,
        ElementKind::Frame,
        Point::new(x, y),
        Size::new(300.0, 300.0),
    );
    created(&out)
}

fn group(st: &mut ToileState, ids: &[&ElementId]) -> ElementId {
    created(&st.handle_command(ToileCommand::GroupElements {
        ids: ids.iter().map(|id| (*id).clone()).collect(),
    }))
}

fn set_frame(
    st: &mut ToileState,
    frame: &ElementId,
    add: &[&ElementId],
    remove: &[&ElementId],
) -> ToileOutcome {
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: add.iter().map(|id| (*id).clone()).collect(),
        remove: remove.iter().map(|id| (*id).clone()).collect(),
    })
}

fn reorder(st: &mut ToileState, id: &ElementId, to: ZTarget) -> ToileOutcome {
    st.handle_command(ToileCommand::Reorder {
        node: NodeId::Element(id.clone()),
        to,
    })
}

fn order(st: &ToileState) -> Vec<NodeId> {
    st.layer_order().to_vec()
}

fn nodes(ids: &[&ElementId]) -> Vec<NodeId> {
    ids.iter()
        .map(|id| NodeId::Element((*id).clone()))
        .collect()
}

/// The element ids of a derived frame, in paint order.
fn painted(st: &ToileState) -> Vec<ElementId> {
    st.debug_viewport()
        .nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e.id.clone()),
            DisplayNode::Connector(_) => None,
        })
        .collect()
}

fn errors(out: &ToileOutcome) -> Vec<&ToileError> {
    out.display
        .iter()
        .filter_map(|e| match e {
            ToileEvent::Error(err) => Some(err),
            _ => None,
        })
        .collect()
}

fn viewports(events: &[ToileEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, ToileEvent::ViewportData(_)))
        .count()
}

// -----------------------------------------------------------------------------
// Birth
// -----------------------------------------------------------------------------

/// `TOILE-188`: a birth box that is empty, negative or not finite is refused
/// and mints nothing. A flat box with one side is still taken, as a level pen
/// stroke has no height.
#[test]
fn a_birth_box_of_no_usable_size_is_refused() {
    let kind = ElementKind::Shape(ShapeKind::Rect);
    for size in [
        Size::new(0.0, 0.0),
        Size::new(-10.0, 20.0),
        Size::new(20.0, -10.0),
        Size::new(f64::NAN, 20.0),
        Size::new(20.0, f64::INFINITY),
    ] {
        let mut st = ToileState::new();
        let out = create(&mut st, kind, Point::ZERO, size);
        assert!(out.update.is_none(), "{size:?} wrote a change");
        assert_eq!(errors(&out), [&ToileError::UnusableSize], "{size:?}");
        assert_eq!(st.element_count(), 0, "{size:?}");
    }
    let mut st = ToileState::new();
    let out = create(
        &mut st,
        kind,
        Point::new(f64::NAN, 0.0),
        Size::new(9.0, 9.0),
    );
    assert_eq!(errors(&out), [&ToileError::UnusableSize]);
    let out = create(&mut st, kind, Point::ZERO, Size::new(40.0, 0.0));
    assert!(out.update.is_some(), "a flat box is drawn");
}

/// `TOILE-035`: each kind is born with the style its kind seeds. A note takes
/// the note card, and every other kind the plain default.
#[test]
fn each_kind_is_born_with_its_own_style() {
    let mut st = ToileState::new();
    let note = created(&create(
        &mut st,
        ElementKind::Sticky,
        Point::ZERO,
        Size::new(80.0, 80.0),
    ));
    let shape = rect(&mut st, 100.0, 0.0);
    let sticky = ElementStyle::for_kind(ElementKind::Sticky);
    assert_eq!(st.style_of(&note), sticky);
    assert_ne!(
        sticky,
        ElementStyle::default(),
        "a note has a card of its own"
    );
    assert_eq!(st.style_of(&shape), ElementStyle::default());
}

/// `TOILE-191`: a seed that does not match the kind is refused, with no write
/// and no element.
#[test]
fn a_seed_of_another_kind_is_refused() {
    let mut st = ToileState::new();
    for (kind, content) in [
        (
            ElementKind::Shape(ShapeKind::Rect),
            InitContent::Text("a body".to_owned()),
        ),
        (
            ElementKind::Sticky,
            InitContent::Embed("https://x.test".to_owned()),
        ),
    ] {
        let out = st.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::ZERO,
            size: Size::new(10.0, 10.0),
            content,
            fill: None,
            correlate: 0,
        });
        assert!(out.update.is_none());
        assert_eq!(errors(&out), [&ToileError::ContentKindMismatch]);
    }
    assert_eq!(st.element_count(), 0);
    assert!(st.layer_order().is_empty());
}

/// `TOILE-036`: a resize, a rotate or a body overwrite naming an id the board
/// does not hold answers its error and writes nothing.
#[test]
fn a_write_to_a_missing_element_is_refused() {
    let mut st = ToileState::new();
    let kept = rect(&mut st, 0.0, 0.0);
    let ghost = ElementId("0000000000000009_00000099".to_owned());
    for cmd in [
        ToileCommand::ResizeElement {
            id: ghost.clone(),
            new: Rect::new(0.0, 0.0, 50.0, 50.0),
        },
        ToileCommand::RotateElement {
            id: ghost.clone(),
            radians: 1.0,
        },
        ToileCommand::SetText {
            id: ghost.clone(),
            base: String::new(),
            text: "x".to_owned(),
        },
    ] {
        let out = st.handle_command(cmd);
        assert!(out.update.is_none());
        assert_eq!(errors(&out), [&ToileError::NoSuchElement(ghost.0.clone())]);
    }
    assert_eq!(st.element_count(), 1);
    assert_eq!(st.bounds_of(&kept), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

// -----------------------------------------------------------------------------
// The lock
// -----------------------------------------------------------------------------

/// `TOILE-033`: a locked element refuses every command but the lock. A
/// resize, a rotate, a restyle, a body overwrite, a delete and a reorder all
/// write nothing. A group holding it is kept whole by a delete. The unlock
/// then lets the delete through.
#[test]
fn a_locked_element_refuses_every_command_but_the_lock() {
    let mut st = ToileState::new();
    let note = created(&create(
        &mut st,
        ElementKind::Sticky,
        Point::ZERO,
        Size::new(80.0, 80.0),
    ));
    let other = rect(&mut st, 200.0, 0.0);
    let bystander = rect(&mut st, 400.0, 0.0);
    st.handle_command(ToileCommand::LockElements {
        ids: vec![note.clone()],
        locked: true,
    });
    let before = (order(&st), st.bounds_of(&note), st.style_of(&note));
    let red = ElementStyle {
        fill: Some(0xffff_0000),
        ..ElementStyle::default()
    };
    for cmd in [
        ToileCommand::ResizeElement {
            id: note.clone(),
            new: Rect::new(0.0, 0.0, 300.0, 300.0),
        },
        ToileCommand::RotateElement {
            id: note.clone(),
            radians: 1.0,
        },
        ToileCommand::RestyleElements {
            ids: vec![note.clone()],
            style: red,
        },
        ToileCommand::SetText {
            id: note.clone(),
            base: String::new(),
            text: "overwritten".to_owned(),
        },
        ToileCommand::DeleteElements {
            ids: vec![note.clone()],
        },
        ToileCommand::Reorder {
            node: NodeId::Element(note.clone()),
            to: ZTarget::Front,
        },
    ] {
        let label = format!("{cmd:?}");
        let out = st.handle_command(cmd);
        assert!(out.update.is_none(), "{label} wrote a change");
    }
    assert_eq!(
        (order(&st), st.bounds_of(&note), st.style_of(&note)),
        before
    );

    // A group with a locked member survives a delete; an unlocked element
    // named beside it still goes.
    let g = group(&mut st, &[&note, &other]);
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![g.clone(), bystander.clone()],
    });
    assert!(st.bounds_of(&g).is_some(), "the group was deleted");
    assert!(st.bounds_of(&bystander).is_none(), "the free element stays");

    st.handle_command(ToileCommand::LockElements {
        ids: vec![note.clone()],
        locked: false,
    });
    st.handle_command(ToileCommand::DeleteElements { ids: vec![g] });
    assert_eq!(st.element_count(), 0, "the unlock frees the delete");
}

// -----------------------------------------------------------------------------
// Groups
// -----------------------------------------------------------------------------

/// `TOILE-149`: an element belongs to one container at most. Framed out of
/// its group, it leaves the group; grouped out of its frame, it leaves the
/// frame.
#[test]
fn an_element_sits_in_one_container_at_most() {
    let mut st = ToileState::new();
    let f = frame(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let b = rect(&mut st, 30.0, 10.0);
    let c = rect(&mut st, 50.0, 10.0);
    let g = group(&mut st, &[&a, &b, &c]);
    set_frame(&mut st, &f, &[&a], &[]);
    assert_eq!(st.membership_of(&f), std::slice::from_ref(&a));
    assert_eq!(st.membership_of(&g), [b.clone(), c.clone()]);

    let g2 = group(&mut st, &[&a, &b]);
    assert_eq!(st.membership_of(&g2), [a.clone(), b.clone()]);
    assert!(st.membership_of(&f).is_empty(), "a still sits in the frame");
    assert!(!st.membership_of(&g).contains(&b), "b still sits in g");

    // Two members of one frame grouped: the group takes their place in it.
    let d = rect(&mut st, 70.0, 10.0);
    let e = rect(&mut st, 90.0, 10.0);
    set_frame(&mut st, &f, &[&d, &e], &[]);
    let g3 = group(&mut st, &[&d, &e]);
    assert_eq!(st.membership_of(&f), std::slice::from_ref(&g3));
    assert_eq!(st.membership_of(&g3), [d, e]);
    let mut rebuilt = ToileState::new();
    rebuilt.apply_remote(&st.export_snapshot());
    assert_eq!(st.debug_caches(), rebuilt.debug_caches());
}

/// `TOILE-054`: a group whose members all leave is removed, and the caches
/// equal a rebuild's.
#[test]
fn a_group_left_with_no_member_is_removed() {
    let mut st = ToileState::new();
    let f = frame(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let b = rect(&mut st, 30.0, 10.0);
    let g = group(&mut st, &[&a, &b]);
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone()],
    });
    assert_eq!(st.membership_of(&g), std::slice::from_ref(&b));
    set_frame(&mut st, &f, &[&b], &[]);
    assert!(st.bounds_of(&g).is_none(), "the empty group is still there");
    assert!(!order(&st).contains(&NodeId::Element(g)));
    let mut rebuilt = ToileState::new();
    rebuilt.apply_remote(&st.export_snapshot());
    assert_eq!(st.debug_caches(), rebuilt.debug_caches());
}

/// `TOILE-151`, `TOILE-152`, `TOILE-246`, `TOILE-153`: a binding drops an id
/// the board lost and a group selected with its own member. It binds the
/// enclosed member it kept, and it moves nobody.
#[test]
fn a_binding_keeps_what_it_can_hold_and_moves_nobody() {
    let mut a = ToileState::with_peer(1);
    let x = rect(&mut a, 0.0, 0.0);
    let y = rect(&mut a, 30.0, 0.0);
    let z = rect(&mut a, 60.0, 0.0);
    let gone = rect(&mut a, 90.0, 0.0);
    let inner = group(&mut a, &[&x, &y]);
    // A peer deletes `gone` while this user still has it selected.
    let mut b = ToileState::with_peer(2);
    b.apply_remote(&a.export_snapshot());
    let deleted = b.handle_command(ToileCommand::DeleteElements {
        ids: vec![gone.clone()],
    });
    a.apply_remote(&deleted.update.expect("the delete is a change"));

    let boxes = [x.clone(), y.clone(), z.clone()].map(|id| a.bounds_of(&id));
    let g = group(&mut a, &[&inner, &x, &z, &gone]);
    assert_eq!(
        a.membership_of(&g),
        [x.clone(), z.clone()],
        "the enclosing group and the lost id are dropped, the member kept"
    );
    assert_eq!(
        [x, y, z].map(|id| a.bounds_of(&id)),
        boxes,
        "a member moved"
    );
}

/// `TOILE-053`: a move of a group reaches every leaf of a nested subtree.
#[test]
fn a_move_reaches_every_leaf_of_a_nested_group() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 20.0, 0.0);
    let c = rect(&mut st, 40.0, 0.0);
    let d = rect(&mut st, 60.0, 0.0);
    let inner = group(&mut st, &[&a, &b]);
    let mid = group(&mut st, &[&inner, &c]);
    let outer = group(&mut st, &[&mid, &d]);
    st.handle_command(ToileCommand::MoveElements {
        ids: vec![outer],
        delta: Vec2::new(5.0, 7.0),
    });
    for (id, x) in [(&a, 0.0), (&b, 20.0), (&c, 40.0), (&d, 60.0)] {
        assert_eq!(
            st.bounds_of(id),
            Some(Rect::new(x + 5.0, 7.0, x + 15.0, 17.0)),
            "{id:?}"
        );
    }
}

/// `TOILE-157`, `TOILE-158`: ungrouping a nested group hands its children to
/// the enclosing group, and moves no element.
#[test]
fn a_nested_ungroup_hands_its_children_up_and_moves_nothing() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 20.0, 0.0);
    let c = rect(&mut st, 40.0, 0.0);
    let inner = group(&mut st, &[&a, &b]);
    let outer = group(&mut st, &[&inner, &c]);
    let boxes = [a.clone(), b.clone(), c.clone()].map(|id| st.bounds_of(&id));
    st.handle_command(ToileCommand::UngroupElements {
        groups: vec![inner],
    });
    let mut members = st.membership_of(&outer);
    members.sort();
    let mut expected = vec![a.clone(), b.clone(), c.clone()];
    expected.sort();
    assert_eq!(members, expected);
    assert_eq!([a, b, c].map(|id| st.bounds_of(&id)), boxes);
}

/// `TOILE-160`: ungrouping an id the board does not hold answers its error.
#[test]
fn ungrouping_a_missing_id_says_so() {
    let mut st = ToileState::new();
    let ghost = ElementId("0000000000000009_00000099".to_owned());
    let out = st.handle_command(ToileCommand::UngroupElements {
        groups: vec![ghost.clone()],
    });
    assert!(out.update.is_none());
    assert_eq!(errors(&out), [&ToileError::NoSuchElement(ghost.0)]);
}

/// `TOILE-162`: deleting a group removes its own subtree and no element
/// outside it.
#[test]
fn deleting_a_group_spares_a_bystander() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 20.0, 0.0);
    let by = rect(&mut st, 5.0, 5.0);
    let g = group(&mut st, &[&a, &b]);
    st.handle_command(ToileCommand::DeleteElements { ids: vec![g] });
    assert_eq!(st.element_count(), 1);
    assert_eq!(order(&st), nodes(&[&by]));
}

// -----------------------------------------------------------------------------
// Frame membership
// -----------------------------------------------------------------------------

/// `TOILE-100`: a member removed and added again in one gesture ends last.
#[test]
fn a_member_removed_and_added_in_one_gesture_ends_last() {
    let mut st = ToileState::new();
    let f = frame(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let b = rect(&mut st, 30.0, 10.0);
    set_frame(&mut st, &f, &[&a, &b], &[]);
    set_frame(&mut st, &f, &[&a], &[&a]);
    assert_eq!(st.membership_of(&f), [b, a]);
}

/// `TOILE-101`: the frame itself, an unknown id and a current member are
/// skipped, and the rest of the gesture still joins.
#[test]
fn an_ineligible_id_does_not_stop_the_rest() {
    let mut st = ToileState::new();
    let f = frame(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let b = rect(&mut st, 30.0, 10.0);
    let ghost = ElementId("0000000000000009_00000099".to_owned());
    set_frame(&mut st, &f, &[&a], &[]);
    let out = set_frame(&mut st, &f, &[&f, &ghost, &a, &b], &[]);
    assert!(errors(&out).is_empty());
    assert_eq!(st.membership_of(&f), [a, b]);
}

/// `TOILE-102`: a gesture that changes nothing writes nothing.
#[test]
fn a_membership_change_with_no_effect_writes_nothing() {
    let mut st = ToileState::new();
    let f = frame(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let b = rect(&mut st, 30.0, 10.0);
    set_frame(&mut st, &f, &[&a], &[]);
    let out = set_frame(&mut st, &f, &[&a], &[&b]);
    assert!(out.update.is_none());
    assert!(out.display.is_empty());
}

/// `TOILE-050`: a membership change aimed at an element that is not a frame
/// is refused aloud, and writes nothing.
#[test]
fn a_membership_change_on_a_non_frame_is_refused() {
    let mut st = ToileState::new();
    let not_a_frame = rect(&mut st, 0.0, 0.0);
    let a = rect(&mut st, 10.0, 10.0);
    let out = set_frame(&mut st, &not_a_frame, &[&a], &[]);
    assert!(out.update.is_none());
    assert_eq!(errors(&out), [&ToileError::ContentKindMismatch]);
    assert!(st.membership_of(&not_a_frame).is_empty());
}

// -----------------------------------------------------------------------------
// The paint order
// -----------------------------------------------------------------------------

/// `TOILE-119`: the order paints back to front. The derived frame hands the
/// walk the last entry last, so it paints over the first.
#[test]
fn the_last_entry_paints_last() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 5.0, 5.0);
    assert_eq!(painted(&st), [a.clone(), b.clone()]);
    reorder(&mut st, &a, ZTarget::Front);
    assert_eq!(painted(&st), [b, a]);
}

/// `TOILE-120`: an entry naming an element the document no longer holds is
/// skipped, and the rest still paints. The entry is forged, as only a
/// concurrent change leaves one.
#[test]
fn an_entry_for_a_lost_element_is_skipped() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let ghost = ElementId("0000000000000009_00000099".to_owned());
    st.debug_push_layer_raw(&NodeId::Element(ghost).encode());
    let b = rect(&mut st, 20.0, 0.0);
    assert_eq!(painted(&st), [a, b]);
}

/// `TOILE-122`: a node the order lists twice paints once, at its first place.
#[test]
fn a_node_listed_twice_paints_once_at_its_first_place() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 5.0, 5.0);
    st.debug_push_layer_raw(&NodeId::Element(a.clone()).encode());
    assert_eq!(
        st.debug_raw_layers().len(),
        3,
        "the duplicate is in the list"
    );
    assert_eq!(painted(&st), [a, b]);
}

/// `TOILE-125`, `TOILE-126`: front, forward, backward and an index past the
/// end resolve against the list. The other nodes keep their relative order.
#[test]
fn each_target_resolves_and_the_others_keep_their_order() {
    let mut st = ToileState::new();
    let [a, b, c, d] = [0.0, 20.0, 40.0, 60.0].map(|x| rect(&mut st, x, 0.0));
    reorder(&mut st, &b, ZTarget::Front);
    assert_eq!(order(&st), nodes(&[&a, &c, &d, &b]));
    reorder(&mut st, &a, ZTarget::Forward);
    assert_eq!(order(&st), nodes(&[&c, &a, &d, &b]));
    reorder(&mut st, &d, ZTarget::Backward);
    assert_eq!(order(&st), nodes(&[&c, &d, &a, &b]));
    reorder(&mut st, &c, ZTarget::Index(99));
    assert_eq!(order(&st), nodes(&[&d, &a, &b, &c]));
    reorder(&mut st, &b, ZTarget::Index(1));
    assert_eq!(order(&st), nodes(&[&d, &b, &a, &c]));
}

/// `TOILE-127`: a reorder to the place the node holds writes nothing.
#[test]
fn a_reorder_to_its_own_place_writes_nothing() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 20.0, 0.0);
    for (id, to) in [
        (&b, ZTarget::Front),
        (&b, ZTarget::Forward),
        (&a, ZTarget::Back),
        (&a, ZTarget::Backward),
        (&a, ZTarget::Index(0)),
    ] {
        let out = reorder(&mut st, id, to);
        assert!(out.update.is_none(), "{to:?}");
        assert!(out.display.is_empty(), "{to:?}");
    }
}

/// `TOILE-128`: a reorder naming a node the order does not hold does nothing,
/// and answers no error.
#[test]
fn a_reorder_of_an_unknown_node_does_nothing_quietly() {
    let mut st = ToileState::new();
    let a = rect(&mut st, 0.0, 0.0);
    let ghost = ElementId("0000000000000009_00000099".to_owned());
    let out = reorder(&mut st, &ghost, ZTarget::Front);
    assert!(out.update.is_none());
    assert!(out.display.is_empty());
    assert_eq!(order(&st), nodes(&[&a]));
}

// -----------------------------------------------------------------------------
// The engine
// -----------------------------------------------------------------------------

/// `TOILE-209`: a command culls against the default camera until the UI
/// reports one, then against the last camera it reported.
#[test]
fn a_command_culls_against_the_last_reported_camera() {
    let mut st = ToileState::new();
    let near = rect(&mut st, 0.0, 0.0);
    let far = rect(&mut st, 5000.0, 5000.0);
    let shown = |out: &ToileOutcome| -> Vec<ElementId> {
        out.display
            .iter()
            .find_map(|e| match e {
                ToileEvent::ViewportData(vp) => Some(
                    vp.nodes
                        .iter()
                        .filter_map(|n| match n {
                            DisplayNode::Element(e) => Some(e.id.clone()),
                            DisplayNode::Connector(_) => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .expect("a command answers a frame")
    };
    let out = st.handle_command(ToileCommand::MoveElements {
        ids: vec![near.clone()],
        delta: Vec2::new(1.0, 0.0),
    });
    assert_eq!(
        shown(&out),
        std::slice::from_ref(&near),
        "the default camera culls far"
    );
    st.handle_command(ToileCommand::SetViewport(Viewport {
        center: Point::new(5005.0, 5005.0),
        zoom: 1.0,
        size: Size::new(400.0, 400.0),
    }));
    let out = st.handle_command(ToileCommand::MoveElements {
        ids: vec![near],
        delta: Vec2::new(1.0, 0.0),
    });
    assert_eq!(shown(&out), [far], "the reported camera culls near");
}

/// `TOILE-213`: a batch of remote deltas repaints once, where the same deltas
/// one by one repaint once each.
#[test]
fn a_remote_batch_repaints_once() {
    let mut a = ToileState::with_peer(1);
    let deltas: Vec<Vec<u8>> = (0..5)
        .map(|i| {
            create(
                &mut a,
                ElementKind::Shape(ShapeKind::Rect),
                Point::new(f64::from(i) * 20.0, 0.0),
                Size::new(10.0, 10.0),
            )
            .update
            .expect("a create is a change")
        })
        .collect();
    let mut batched = ToileState::with_peer(2);
    let events = batched.apply_remote_batch(&deltas);
    assert_eq!(viewports(&events), 1);
    assert_eq!(batched.element_count(), 5);
    let mut single = ToileState::with_peer(3);
    let each: usize = deltas
        .iter()
        .map(|d| viewports(&single.apply_remote(d)))
        .sum();
    assert_eq!(each, 5);
    assert_eq!(batched.debug_caches(), single.debug_caches());
}

// -----------------------------------------------------------------------------
// The schema
// -----------------------------------------------------------------------------

/// `TOILE-231`: every id this build writes renders at the minted width: 25
/// characters for an element or an arrow, 27 for an order entry. A busy board
/// holds each kind of id, and the extremes of the mint hold the width too.
#[test]
fn every_id_the_board_writes_has_the_minted_width() {
    for (peer, counter) in [(0, 0), (u64::MAX, u32::MAX)] {
        assert_eq!(ElementId::mint(peer, counter).as_str().len(), 25);
        assert_eq!(ConnectorId::mint(peer, counter).as_str().len(), 25);
    }
    // Loro reserves the top peer id, so the widest a document takes is one less.
    let mut st = ToileState::with_peer(u64::MAX - 1);
    let a = rect(&mut st, 0.0, 0.0);
    let b = rect(&mut st, 40.0, 0.0);
    let f = frame(&mut st, 0.0, 100.0);
    group(&mut st, &[&a, &b]);
    set_frame(&mut st, &f, &[&a], &[]);
    st.handle_command(ToileCommand::CreateConnector {
        from: a,
        from_anch: Anchor::Right,
        to: b,
        to_anch: Anchor::Left,
        path_type: PathType::Straight,
        arrows: EndCaps {
            start: false,
            end: true,
        },
    });
    st.handle_command(ToileCommand::CommitStroke {
        points: [0.0f32, 10.0]
            .map(|x| StrokePoint {
                x,
                y: x,
                pressure: 1.0,
            })
            .to_vec(),
        style: ElementStyle::default(),
    });
    let keys = |name: &str| -> Vec<String> {
        match st.crdt().get_map(name).get_deep_value() {
            LoroValue::Map(m) => m.keys().cloned().collect(),
            _ => Vec::new(),
        }
    };
    let ids: HashSet<String> = keys(ELEMENTS).into_iter().chain(keys(CONNECTORS)).collect();
    assert_eq!(
        ids.len(),
        6,
        "two shapes, a frame, a group, a stroke, an arrow"
    );
    for id in &ids {
        assert_eq!(id.len(), 25, "{id}");
    }
    let raw = st.debug_raw_layers();
    assert_eq!(raw.len(), 6);
    for entry in &raw {
        assert_eq!(entry.len(), 27, "{entry}");
    }
}
