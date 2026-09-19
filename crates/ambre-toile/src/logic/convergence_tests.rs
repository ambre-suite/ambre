//! Engine lifecycle tests and the 2-peer CRDT convergence property test.
//!
//! The convergence test drives two independent [`ToileState`]s. It relays every
//! local `update` delta to the other peer through `apply_remote`. It then
//! asserts that the two peers reach an identical `LoroDoc`, and identical
//! derived caches (`bounds` / `layers`). The whiteboard has no computed values
//! to compare, only state and derived structures.

use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use ambre_bus::UserId;

use crate::logic::align::AlignOp;
use crate::logic::autolayout::{Align, AutoLayout, Direction, Sizing};
use crate::logic::command::{
    CommentAnchor, DisplayGeom, DisplayNode, End, InitContent, PresencePayload, Reaction,
    ReactionKind, StatsRequestKind, TextDelta, ToileCommand, ToileError, ToileEvent, ZTarget,
};
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, LinkTarget, NodeId,
    PathType, ShapeKind, StrokePoint, ThreadId,
};
use crate::logic::state::{PRESENCE_TTL_MS, ToileState};

// -----------------------------------------------------------------------------
// Lifecycle unit tests
// -----------------------------------------------------------------------------

fn created_id(out: &crate::logic::command::ToileOutcome) -> ElementId {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
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
        fill: None,
        correlate: 0,
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

/// Read an element's plaintext body from the derived viewport (Phase 0 stores it
/// as `DisplayGeom::Sticky` for a sticky, `DisplayGeom::Text` for a text element).
fn body_of(st: &ToileState, id: &ElementId) -> Option<String> {
    use crate::logic::command::DisplayGeom;
    st.debug_viewport().nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => match &e.geom {
            DisplayGeom::Sticky(t) | DisplayGeom::Text(t) => Some(t.clone()),
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
        fill: None,
        correlate: 0,
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

// -----------------------------------------------------------------------------
// Connector captions (toile-connectors)
// -----------------------------------------------------------------------------

/// Join two fresh rects with a straight arrow and answer its minted id.
fn create_connector(st: &mut ToileState) -> ConnectorId {
    let a = create_rect(st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
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
    // The newest connector is the last node in the paint order, which is where
    // a create puts it. Taking the first would answer an earlier arrow.
    st.layer_order()
        .iter()
        .rev()
        .find_map(|n| match n {
            NodeId::Connector(c) => Some(c.clone()),
            NodeId::Element(_) => None,
        })
        .expect("a create leaves the connector in the paint order")
}

/// Read a connector's caption back out of the derived viewport -- the same
/// string the UI paints.
fn caption(st: &ToileState, id: &ConnectorId) -> String {
    st.debug_viewport()
        .nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Connector(c) if &c.id == id => Some(c.label.clone()),
            _ => None,
        })
        .expect("the connector is in the viewport")
}

/// Read a connector's style back out of the derived viewport.
fn conn_style(st: &ToileState, id: &ConnectorId) -> ElementStyle {
    st.debug_viewport()
        .nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Connector(c) if &c.id == id => Some(c.style.clone()),
            _ => None,
        })
        .expect("the connector is in the viewport")
}

/// `TOILE-046`: an arrow carries its own style, so a user can colour and
/// thicken it without touching any other arrow.
#[test]
fn an_arrow_carries_its_own_style() {
    let mut st = ToileState::new();
    let one = create_connector(&mut st);
    let two = create_connector(&mut st);
    let red = ElementStyle {
        stroke: Some(0xFFCC_2222),
        stroke_w: 6.0,
        ..Default::default()
    };

    let out = st.handle_command(ToileCommand::RestyleConnector {
        id: one.clone(),
        style: red.clone(),
    });
    assert!(out.update.is_some(), "a restyle broadcasts a delta");
    let got = conn_style(&st, &one);
    assert_eq!(got.stroke, red.stroke);
    assert_eq!(got.stroke_w, red.stroke_w);
    // The second arrow keeps its own style: the register is per connector.
    assert_eq!(conn_style(&st, &two).stroke, ElementStyle::default().stroke);
}

/// The board refuses a restyle whole when it does not hold the arrow.
#[test]
fn restyling_a_missing_arrow_is_refused() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::RestyleConnector {
        id: ConnectorId("nope".into()),
        style: ElementStyle::default(),
    });
    assert!(out.update.is_none(), "a refused restyle writes nothing");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::NoSuchConnector(_)))),
    );
}

/// A caption written on an arrow comes back on the arrow's display node.
#[test]
fn a_caption_reaches_the_viewport() {
    let mut st = ToileState::new();
    let c = create_connector(&mut st);
    assert_eq!(caption(&st, &c), "", "an arrow is born without a caption");
    let out = st.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        text: "owns".into(),
    });
    assert!(out.update.is_some(), "a caption broadcasts a delta");
    assert_eq!(caption(&st, &c), "owns");
}

/// The board refuses a caption on an arrow nobody holds, and writes nothing.
#[test]
fn captioning_a_missing_arrow_is_refused() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::SetConnectorLabel {
        id: ConnectorId("nope".into()),
        text: "hi".into(),
    });
    assert!(out.update.is_none(), "a refused caption writes nothing");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::NoSuchConnector(_)))),
    );
}

/// `TOILE-047`: two peers captioning one arrow converge character by character.
/// The whole caption travels on the wire, but the handler diffs it into the
/// `label` char-CRDT, so the handler drops neither side's typing whole.
#[test]
fn concurrent_captions_converge() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let c = create_connector(&mut a);
    // B takes the board, and with it the caption container seeded at birth.
    b.apply_remote(&a.export_snapshot());

    // Both start from "cost" and edit a different end of it, unaware of each
    // other: A prepends, B appends.
    a.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        text: "cost".into(),
    });
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            text: "unit cost".into(),
        })
        .update
        .expect("A's caption broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            text: "cost in EUR".into(),
        })
        .update
        .expect("B's caption broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let merged = caption(&a, &c);
    assert_eq!(merged, caption(&b, &c), "concurrent captions diverged");
    assert!(
        merged.contains("unit") && merged.contains("EUR"),
        "both edits must survive a merge, got {merged:?}"
    );
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
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids: vec![a, b],
        kind: StatsRequestKind::Band,
    });
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
    // Empty ids = whole-board extent, for View -> Fit (`toile-viewport`):
    // the union of every element's bounds, regardless of the (UI-only) selection.
    let mut st = ToileState::new();
    let _a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let _b = create_rect(&mut st, Point::new(500.0, 300.0), Size::new(40.0, 20.0));
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids: vec![],
        kind: StatsRequestKind::Fit,
    });
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
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids: vec![],
        kind: StatsRequestKind::Fit,
    });
    match &out.display[0] {
        ToileEvent::SelectionStats(s) => {
            assert_eq!(s.count, 0);
            assert_eq!(s.bounds, None);
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
}

/// The reply names the request it answers. The Logic thread echoes the kind
/// token back, so a surface spends only its own answer (`TOILE-UI-108`).
#[test]
fn selection_stats_echoes_the_request_kind() {
    let mut st = ToileState::new();
    create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    for kind in [
        StatsRequestKind::Band,
        StatsRequestKind::Fit,
        StatsRequestKind::FindFocus,
        StatsRequestKind::CommentFocus,
    ] {
        let out = st.handle_command(ToileCommand::RequestSelectionStats { ids: vec![], kind });
        match &out.display[0] {
            ToileEvent::SelectionStats(s) => assert_eq!(s.kind, kind),
            other => panic!("expected SelectionStats, got {other:?}"),
        }
    }
}

/// `RequestSelectionData` answers for the whole selection, off-screen included
/// (`TOILE-UI-070`, `TOILE-UI-134`).
#[test]
fn selection_data_includes_offscreen_elements() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(1000.0, 1000.0), Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::RequestSelectionData {
        ids: vec![a.clone(), b.clone()],
    });
    match &out.display[0] {
        ToileEvent::SelectionData(elements) => {
            let ids: Vec<_> = elements.iter().map(|e| e.id.clone()).collect();
            assert!(
                ids.contains(&a) && ids.contains(&b),
                "off-screen b included"
            );
        }
        other => panic!("expected SelectionData, got {other:?}"),
    }
}

/// `RequestBoardIds` returns every element id, off-screen included, which is the
/// whole board select-all must cover (`TOILE-UI-033`).
#[test]
fn board_ids_returns_every_element() {
    let mut st = ToileState::new();
    create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    create_rect(&mut st, Point::new(1000.0, 1000.0), Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::RequestBoardIds);
    match &out.display[0] {
        ToileEvent::BoardIds(ids) => assert_eq!(ids.len(), 2),
        other => panic!("expected BoardIds, got {other:?}"),
    }
}

/// `TOILE-UI-241`: select-all obeys the exclusions a pick obeys -- a locked
/// element and a group node never join the board's id list.
#[test]
fn board_ids_excludes_locked_and_group_nodes() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(40.0, 0.0), Size::new(10.0, 10.0));
    let c = create_rect(&mut st, Point::new(80.0, 0.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::LockElements {
        ids: vec![a.clone()],
        locked: true,
    });
    st.handle_command(ToileCommand::GroupElements {
        ids: vec![b.clone(), c.clone()],
    });

    let out = st.handle_command(ToileCommand::RequestBoardIds);
    match &out.display[0] {
        ToileEvent::BoardIds(ids) => {
            // The locked rect and the group node are gone; the members stay.
            assert!(!ids.contains(&a));
            assert!(ids.contains(&b));
            assert!(ids.contains(&c));
            assert_eq!(ids.len(), 2, "locked + group nodes are excluded");
        }
        other => panic!("expected BoardIds, got {other:?}"),
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
            fill: None,
            correlate: 0,
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
// Find (board text search,)
// -----------------------------------------------------------------------------

fn create_sticky(st: &mut ToileState, at: Point, body: &str) -> ElementId {
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Sticky,
        at,
        size: Size::new(80.0, 80.0),
        content: InitContent::Text(body.to_owned()),
        fill: None,
        correlate: 0,
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
// Ephemeral presence
// -----------------------------------------------------------------------------

/// The live peer set the events carry (`PRESENCE-009`): one event, the whole
/// set, which the view adopts as it stands.
fn live_set(events: Vec<ToileEvent>) -> Vec<crate::logic::command::PresenceUpdate> {
    events
        .into_iter()
        .find_map(|e| match e {
            ToileEvent::PeerPresence(peers) => Some(peers),
            _ => None,
        })
        .expect("expected a PeerPresence event")
}

/// A presence sample with a cursor and nothing else.
fn sample_at(x: f64, y: f64) -> PresencePayload {
    PresencePayload {
        cursor: Some(Point::new(x, y)),
        ..PresencePayload::default()
    }
}

/// Broadcast `payload` from `st` at `now_ms` and return the ephemeral frame.
fn presence_frame(st: &mut ToileState, payload: PresencePayload, now_ms: i64) -> Vec<u8> {
    st.handle_command(ToileCommand::Presence { payload, now_ms })
        .ephemeral
        .expect("presence broadcasts an ephemeral frame")
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
    // A local presence sample rides the ephemeral leg. The board emits no
    // display event while the live set holds steady, and no CRDT delta. The UI
    // draws its own cursor.
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "note");
    let payload = PresencePayload {
        name: None,
        cursor: Some(Point::new(12.0, 34.0)),
        selection: vec![a],
        live_stroke: None,
        laser: Some(Point::new(50.0, 60.0)),
        present_slide: None,
    };
    let out = st.handle_command(ToileCommand::Presence {
        payload,
        now_ms: 1_000,
    });
    assert!(out.display.is_empty(), "presence emits no display event");
    assert!(out.update.is_none(), "presence is never a CRDT delta");
    assert!(
        out.ephemeral.is_some(),
        "presence must broadcast an ephemeral frame"
    );
}

#[test]
fn the_outbound_sample_carries_the_session_display_name() {
    // TOILE-UI-076: the name comes from the session this thread already holds,
    // so a collaborator reads a name and not a hex prefix. It is cosmetic
    // (`PRESENCE-004`): it travels beside the sample and decides nothing.
    let mut sender = ToileState::new();
    sender.set_display_name("Ada".to_owned());
    let frame = presence_frame(&mut sender, sample_at(1.0, 2.0), 0);

    let mut rx = ToileState::new();
    let peers = live_set(rx.apply_ephemeral(UserId::new(), &frame));
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].presence.name.as_deref(), Some("Ada"));

    // A session with no name yet sends none, and the receiver falls back to the
    // short handle rather than paint an empty chip.
    let mut anonymous = ToileState::new();
    let frame = presence_frame(&mut anonymous, sample_at(3.0, 4.0), 0);
    let peers = live_set(rx.apply_ephemeral(UserId::new(), &frame));
    assert!(peers.iter().any(|p| p.presence.name.is_none()));
}

#[test]
fn a_peer_who_stops_sending_leaves_the_live_set() {
    // TOILE-185 / TOILE-UI-075 / PRESENCE-009: a departure is the absence of an
    // entry. No departure signal exists anywhere. The peer ages out against the
    // clock the local samples carry, and the event that says so is the whole
    // set.
    let mut sender = ToileState::new();
    let frame = presence_frame(&mut sender, sample_at(5.0, 5.0), 0);

    let mut rx = ToileState::new();
    // The receiver's clock starts at the first local sample.
    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now_ms: 1_000,
    });
    let alice = UserId::new();
    assert_eq!(live_set(rx.apply_ephemeral(alice, &frame)).len(), 1);

    // Still inside the TTL: Alice is live, and the local sample says nothing new.
    let out = rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now_ms: 1_000 + PRESENCE_TTL_MS - 1,
    });
    assert!(out.display.is_empty(), "nothing left the set yet");
    assert!(rx.peer_presence(&alice).is_some());

    // Past it: the set changes, and the event that announces it is empty.
    let out = rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now_ms: 1_000 + PRESENCE_TTL_MS + 1,
    });
    assert!(
        live_set(out.display).is_empty(),
        "a peer past its TTL is an absent entry"
    );
    assert!(rx.peer_presence(&alice).is_none());
}

#[test]
fn a_departure_message_drops_the_peer_at_once() {
    // PRESENCE-014: the relay says a peer stopped watching, so the live set
    // loses them now rather than at the end of the TTL. The timeout stays the
    // fallback for a message that never arrives.
    let mut sender = ToileState::new();
    let frame = presence_frame(&mut sender, sample_at(3.0, 3.0), 0);

    let mut rx = ToileState::new();
    let alice = UserId::new();
    assert_eq!(live_set(rx.apply_ephemeral(alice, &frame)).len(), 1);

    let events = rx.peer_left(alice);
    assert!(
        live_set(events).is_empty(),
        "the departure hands back the set that remains"
    );
    assert!(rx.peer_presence(&alice).is_none());

    // A peer nobody was painting is not a change: no event, no repaint.
    assert!(
        rx.peer_left(UserId::new()).is_empty(),
        "a departure for an unknown peer says nothing"
    );
}

#[test]
fn a_clock_that_steps_backwards_ages_nobody() {
    // The local clock is monotonic: an NTP correction or a suspended tab must
    // not empty the board of every peer at once.
    let mut sender = ToileState::new();
    let frame = presence_frame(&mut sender, sample_at(1.0, 1.0), 0);

    let mut rx = ToileState::new();
    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now_ms: 100_000,
    });
    let alice = UserId::new();
    rx.apply_ephemeral(alice, &frame);

    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now_ms: 0,
    });
    assert!(
        rx.peer_presence(&alice).is_some(),
        "a clock stepping back keeps the peer"
    );
}

#[test]
fn a_sample_replaces_the_peers_presence_whole() {
    // TOILE-178: a field a sender leaves out is a field the receiver clears. A
    // merge of the two samples would keep the stale selection.
    let mut sender = ToileState::new();
    let a = create_sticky(&mut sender, Point::ZERO, "note");
    let with_selection = PresencePayload {
        cursor: Some(Point::new(1.0, 1.0)),
        selection: vec![a],
        laser: Some(Point::new(2.0, 2.0)),
        ..PresencePayload::default()
    };
    let first = presence_frame(&mut sender, with_selection, 0);
    let second = presence_frame(&mut sender, sample_at(9.0, 9.0), 0);

    let mut rx = ToileState::new();
    let alice = UserId::new();
    rx.apply_ephemeral(alice, &first);
    rx.apply_ephemeral(alice, &second);

    let now = rx.peer_presence(&alice).expect("Alice is live");
    assert_eq!(now.cursor, Some(Point::new(9.0, 9.0)));
    assert!(now.selection.is_empty(), "the selection was not kept");
    assert!(now.laser.is_none(), "the laser was not kept");
}

#[test]
fn apply_ephemeral_keys_presence_by_user_id_without_touching_history() {
    // #2 / DoD: an inbound presence frame yields a PeerPresence keyed by the
    // resolved UserId and mutates neither the doc nor its history.
    let mut sender = ToileState::new();
    let payload = PresencePayload {
        name: None,
        cursor: Some(Point::new(7.0, 8.0)),
        selection: Vec::new(),
        live_stroke: Some(vec![StrokePoint {
            x: 1.0,
            y: 2.0,
            pressure: 1.0,
        }]),
        laser: None,
        present_slide: None,
    };
    let frame = presence_frame(&mut sender, payload.clone(), 0);

    let mut rx = ToileState::new();
    let before = rx.export_snapshot();
    let count_before = rx.element_count();

    let alice = UserId::new();
    let peers = live_set(rx.apply_ephemeral(alice, &frame));
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].user_id, alice);
    assert_eq!(peers[0].presence, payload);
    assert_eq!(rx.peer_presence(&alice), Some(&payload));

    // Non-mutating: neither the CRDT snapshot nor the element set moves.
    assert_eq!(rx.export_snapshot(), before);
    assert_eq!(rx.element_count(), count_before);

    // The board tracks a second peer independently, keyed by its own UserId,
    // and the set carries both.
    let bob = UserId::new();
    let bob_frame = presence_frame(&mut sender, PresencePayload::default(), 0);
    let peers = live_set(rx.apply_ephemeral(bob, &bob_frame));
    assert_eq!(peers.len(), 2, "the event carries the whole live set");
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
// The export cursor (`toile-engine section 1.4`)
// -----------------------------------------------------------------------------

/// `TOILE-217`: an import takes the cursor past the operations it brought in.
/// The next local edit therefore does not send a peer its own work back.
#[test]
fn an_import_is_not_broadcast_back() {
    let mut author = ToileState::new();
    create_rect(&mut author, Point::ZERO, Size::new(10.0, 10.0));
    let snapshot = author.export_snapshot();

    let mut reader = ToileState::new();
    reader.apply_remote(&snapshot);
    let out = reader.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(50.0, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: 0,
    });
    let delta = out.update.expect("a create must produce a delta");

    // A peer holding nothing sees nothing in this delta: the author's element
    // is not in it. Its own create is, but it depends on the reader's genesis
    // operations, which this peer does not hold, so Loro keeps it pending.
    let mut naive = ToileState::new();
    naive.apply_remote(&delta);
    assert_eq!(
        naive.element_count(),
        0,
        "the delta must carry no operation the reader merely imported"
    );

    // The author, who holds everything the delta depends on, gets the new one.
    let mut third = ToileState::new();
    third.apply_remote(&snapshot);
    third.apply_remote(&delta);
    assert_eq!(third.element_count(), 2);
}

/// The board opens by committing its own scaffolding, and the first import
/// lands before any flush. Those genesis operations are ours, and no flush has
/// sent them out. The cursor must therefore keep our own counter across an
/// import. Otherwise the next local delta depends on operations no peer holds,
/// and a peer that cannot resolve a dependency never shows the edit.
#[test]
fn an_import_does_not_swallow_our_own_unbroadcast_genesis() {
    let mut author = ToileState::new();
    create_rect(&mut author, Point::ZERO, Size::new(10.0, 10.0));
    let snapshot = author.export_snapshot();

    // A brand-new board: it commits its own scaffolding and never sends it.
    let mut reader = ToileState::new();
    reader.apply_remote(&snapshot);
    let out = reader.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(50.0, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: 0,
    });
    let delta = out.update.expect("a create must produce a delta");

    // The author receives it. Without the genesis operations the create depends
    // on, Loro would hold it pending and the element would never appear.
    author.apply_remote(&delta);
    assert_eq!(
        author.element_count(),
        2,
        "the delta must carry everything it depends on"
    );
}

/// `TOILE-236`: a local restore leaves the cursor alone. What this client wrote
/// before it closed may have reached no peer, and a reopen mints a fresh peer
/// id. Absorbing the restored history would therefore drop an offline session
/// for good.
#[test]
fn a_restore_still_offers_what_this_client_wrote_offline() {
    // A session that wrote something and closed without ever flushing to a peer.
    let mut offline = ToileState::new();
    create_rect(&mut offline, Point::ZERO, Size::new(10.0, 10.0));
    let disk = offline.export_snapshot();

    // The reopen: a fresh board, a fresh peer id, restoring from disk.
    let mut reopened = ToileState::new();
    reopened
        .import_snapshot(&disk)
        .expect("a board this build wrote reads back at the current schema");
    assert_eq!(reopened.element_count(), 1);

    let out = reopened.handle_command(ToileCommand::MoveElements {
        ids: vec![
            reopened
                .debug_viewport()
                .nodes
                .iter()
                .find_map(|n| match n {
                    crate::logic::command::DisplayNode::Element(e) => Some(e.id.clone()),
                    crate::logic::command::DisplayNode::Connector(_) => None,
                })
                .expect("the restored element"),
        ],
        delta: Vec2::new(5.0, 0.0),
    });
    let delta = out.update.expect("a move must produce a delta");

    // A peer that has seen nothing gets the whole restored session out of that
    // first flush, the element included -- not just the move.
    let mut peer = ToileState::new();
    peer.apply_remote(&delta);
    assert_eq!(
        peer.element_count(),
        1,
        "the restored history must still be offered to a peer"
    );
}

// -----------------------------------------------------------------------------
// Groups
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
// Frames: membership + z-order invariant
// -----------------------------------------------------------------------------

fn create_frame(st: &mut ToileState, at: Point, size: Size) -> ElementId {
    created_id(&st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Frame,
        at,
        size,
        content: InitContent::None,
        fill: None,
        correlate: 0,
    }))
}

// -----------------------------------------------------------------------------
// Links on an element (toile-links)
// -----------------------------------------------------------------------------

/// Read an element's link back out of the derived viewport.
fn link_of(st: &ToileState, id: &ElementId) -> Option<LinkTarget> {
    st.debug_viewport().nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if &e.id == id => Some(e.link.clone()),
        _ => None,
    })?
}

/// A command sets a link, the painted element carries it back, and absence
/// clears it rather than an empty value (`TOILE-062`, `TOILE-230`).
#[test]
fn a_link_is_set_and_cleared() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    assert_eq!(link_of(&st, &id), None, "an element is born unlinked");

    let target = LinkTarget::Url("https://example.org/".into());
    let out = st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: Some(target.clone()),
    });
    assert!(out.update.is_some(), "a link broadcasts a delta");
    assert_eq!(link_of(&st, &id), Some(target));

    st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: None,
    });
    assert_eq!(link_of(&st, &id), None, "clearing removes the register");
}

/// `TOILE-111`: setting a link writes the link register alone -- the style, the
/// geometry and the lock are what they were.
#[test]
fn setting_a_link_touches_nothing_else() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::new(4.0, 8.0), Size::new(30.0, 20.0));
    let before = st
        .debug_viewport()
        .nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Element(e) if e.id == id => Some(e.clone()),
            _ => None,
        })
        .expect("the element is painted");

    st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: Some(LinkTarget::Frame(ElementId("f".into()))),
    });

    let after = st
        .debug_viewport()
        .nodes
        .iter()
        .find_map(|n| match n {
            DisplayNode::Element(e) if e.id == id => Some(e.clone()),
            _ => None,
        })
        .expect("the element is still painted");
    assert_eq!(after.bounds, before.bounds, "geometry moved");
    assert_eq!(after.style.fill, before.style.fill, "fill changed");
    assert_eq!(after.style.stroke, before.style.stroke, "stroke changed");
    assert_eq!(after.locked, before.locked, "lock changed");
    assert_eq!(st.bounds_of(&id), st.bounds_of(&id));
    assert!(
        after.link.is_some(),
        "the link is the one thing that changed"
    );
}

/// `TOILE-109`: a link outlives its target. Deleting the frame a link points at
/// leaves the register alone. The frame may come back through a late change or
/// an undo, and a read must never destroy data.
#[test]
fn deleting_the_target_frame_leaves_the_link() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let id = create_rect(&mut st, Point::new(400.0, 0.0), Size::new(10.0, 10.0));
    let target = LinkTarget::Frame(frame.clone());
    st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: Some(target.clone()),
    });

    st.handle_command(ToileCommand::DeleteElements { ids: vec![frame] });
    assert_eq!(
        link_of(&st, &id),
        Some(target),
        "the link must survive its target"
    );
}

/// The board refuses a link whole when it does not hold the target element.
#[test]
fn linking_a_missing_element_is_refused() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::SetLink {
        id: ElementId("nope".into()),
        link: Some(LinkTarget::Url("https://example.org/".into())),
    });
    assert!(out.update.is_none(), "a refused link writes nothing");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::NoSuchElement(_)))),
    );
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
// Smart Animate snapshot derivation (`toile-presentation`)
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
// Clipboard: copy / paste / duplicate
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
            ToileEvent::ElementCreated { id, .. } => Some(id.clone()),
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
    // Cross-board portability (DoD): copy from one board, paste into a fresh
    // one, and the paste reproduces the selection.
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
// Undo / redo
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
// Transforms, connectors, viewport
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
    // rotation-expanded AABB is (25,-25)..(75,75) -- the invariant.
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
    // A dangling connector routes to None, and the paint set drops it.
    assert_eq!(connector_node_count(&st), 0);
}

#[test]
fn deleting_an_endpoint_makes_the_connector_dangle() {
    // an endpoint deletion leaves the connector unroutable. Derivation skips it
    // (route -> None) and never corrupts state.
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
    // derivation only emits the visible set. Default viewport spans
    // roughly (-640,-400)..(640,400), so derivation culls a node at (5000,5000).
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
// Presentation: slide sequence CRDT + playback (`toile-presentation`)
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

    // The whole reorder collapses under a single undo.
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
    // The handler drops the repeat and keeps the first occurrence.
    assert_eq!(st.slide_order(), vec![a, b]);
}

#[test]
fn set_slide_order_unchanged_writes_nothing() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone()],
    });
    let out = st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone()],
    });
    assert!(
        out.update.is_none(),
        "writing the order already held must produce neither a commit nor a delta"
    );
    assert_eq!(st.slide_order(), vec![a, b]);
}

#[test]
fn shorter_deck_clamps_the_active_slide() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    let c = create_frame(&mut st, Point::new(600.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b, c],
    });
    st.handle_command(ToileCommand::GotoSlide(2));
    assert_eq!(st.playback().1, 2);

    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone()],
    });
    assert_eq!(
        st.playback().1,
        0,
        "a deck shorter than the viewer's cursor clamps it to the new last slide"
    );
}

#[test]
fn deleting_a_frame_drops_its_slide() {
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    let c = create_frame(&mut st, Point::new(600.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone(), c.clone()],
    });
    st.handle_command(ToileCommand::GotoSlide(2));

    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![b.clone()],
    });
    assert_eq!(
        st.slide_order(),
        vec![a.clone(), c.clone()],
        "the deck must hold only slides whose frame still exists"
    );
    assert_eq!(
        st.playback().1,
        1,
        "the shortened deck clamps the viewer's cursor like any other write"
    );

    // The frame and the slide left in one write, so one undo brings both back.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.slide_order(), vec![a, b, c]);
}

#[test]
fn deleting_an_element_seeds_no_deck() {
    let mut st = ToileState::new();
    let r = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::DeleteElements { ids: vec![r] });
    assert!(
        st.slide_order().is_empty(),
        "deleting an element on a board with no deck must not seed a presentation"
    );
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
    // since the UI holds no board state (toile-presentation).
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
fn slide_change_carries_count_frame_and_notes() {
    // The presenter overlay panel (toile-presentation) reads the deck length,
    // the active frame id and its speaker notes straight off the enriched
    // SlideChanged. The chrome holds no board state of its own.
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone()],
    });
    st.handle_command(ToileCommand::EditNotes {
        frame: b.clone(),
        delta: insert_notes(0, "Talk track for slide two"),
    });

    let change = slide_change(&st.handle_command(ToileCommand::NextSlide));
    assert_eq!(
        change.count, 2,
        "the whole deck length drives the k / n counter"
    );
    assert_eq!(change.frame, Some(b.clone()), "the active slide's frame id");
    assert_eq!(
        change.notes, "Talk track for slide two",
        "the active slide's speaker notes seed the editor"
    );

    // Stepping back to a note-less slide reports empty notes, not the stale ones.
    let change = slide_change(&st.handle_command(ToileCommand::PrevSlide));
    assert_eq!(change.frame, Some(a));
    assert!(
        change.notes.is_empty(),
        "a note-less slide carries empty notes"
    );
}

#[test]
fn enter_present_starts_from_empty_snapshot() {
    // Entering present mode has no source frame: `from` is empty so slide 0 fades
    // in rather than tweening from a stale frame.
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
    // playback carries no CRDT delta.
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
// Follow presenter: ephemeral slide broadcast (`toile-presentation`)
// -----------------------------------------------------------------------------

/// Decode a broadcast slide index out of an outcome's ephemeral frame, if any.
fn broadcast_slide(out: &crate::logic::command::ToileOutcome) -> Option<usize> {
    let bytes = out.ephemeral.as_ref()?;
    let p: PresencePayload = postcard::from_bytes(bytes).expect("decodable presence frame");
    p.present_slide
}

/// A three-frame deck shared by the follow-presenter tests.
fn deck_of_three(st: &mut ToileState) {
    let a = create_frame(st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    let c = create_frame(st, Point::new(600.0, 0.0), Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a, b, c],
    });
}

#[test]
fn broadcast_frame_carries_active_slide() {
    // While broadcasting, a navigation additionally emits an ephemeral frame
    // carrying the new active index -- and never a CRDT delta.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    st.handle_command(ToileCommand::SetPresentBroadcast(true));

    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(slide_changed(&out), Some(1), "the local view still glides");
    assert_eq!(
        broadcast_slide(&out),
        Some(1),
        "the ephemeral frame carries the new active index"
    );
    assert!(out.update.is_none(), "playback is never a CRDT delta");
}

#[test]
fn no_broadcast_frame_when_not_broadcasting() {
    // With broadcasting off, navigation stays purely local: no ephemeral frame.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(slide_changed(&out), Some(1));
    assert!(
        out.ephemeral.is_none(),
        "no broadcast frame while broadcasting is off"
    );
}

#[test]
fn follower_adopts_presenter_slide() {
    // A follower adopts the presenter's broadcast slide (gliding via SlideChanged)
    // without touching the CRDT or its history.
    let mut presenter = ToileState::new();
    deck_of_three(&mut presenter);
    presenter.handle_command(ToileCommand::SetPresentBroadcast(true));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(2))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote(&presenter.export_snapshot());
    let alice = UserId::new();
    rx.handle_command(ToileCommand::FollowPresenter(Some(alice)));

    let before = rx.export_snapshot();
    let events = rx.apply_ephemeral(alice, &frame);
    assert_eq!(rx.playback().1, 2, "the follower adopted slide 2");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ToileEvent::SlideChanged(c) if c.index == 2)),
        "adoption emits a SlideChanged so the camera glides"
    );
    assert_eq!(
        rx.export_snapshot(),
        before,
        "adoption never mutates history"
    );
}

#[test]
fn non_follower_ignores_broadcast() {
    // Not following anyone: a broadcast frame updates presence but never moves us.
    let mut presenter = ToileState::new();
    deck_of_three(&mut presenter);
    presenter.handle_command(ToileCommand::SetPresentBroadcast(true));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(2))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote(&presenter.export_snapshot());
    let events = rx.apply_ephemeral(UserId::new(), &frame);
    assert_eq!(rx.playback().1, 0, "a non-follower does not move");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ToileEvent::SlideChanged(_))),
        "no glide when not following"
    );
}

#[test]
fn following_other_peer_ignores_broadcast() {
    // Following B, the board ignores a frame from A (identity is the canonical
    // UserId).
    let mut presenter = ToileState::new();
    deck_of_three(&mut presenter);
    presenter.handle_command(ToileCommand::SetPresentBroadcast(true));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(2))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote(&presenter.export_snapshot());
    let bob = UserId::new();
    rx.handle_command(ToileCommand::FollowPresenter(Some(bob)));
    let alice = UserId::new();
    rx.apply_ephemeral(alice, &frame);
    assert_eq!(
        rx.playback().1,
        0,
        "a frame from a non-followed peer never moves us"
    );
}

#[test]
fn adopted_slide_is_clamped_to_deck() {
    // A broadcast index past the deck end clamps to the last slide.
    let mut rx = ToileState::new();
    deck_of_three(&mut rx); // three slides -> last index 2
    let alice = UserId::new();
    rx.handle_command(ToileCommand::FollowPresenter(Some(alice)));
    let frame = postcard::to_allocvec(&PresencePayload {
        present_slide: Some(99),
        ..Default::default()
    })
    .unwrap();
    rx.apply_ephemeral(alice, &frame);
    assert_eq!(
        rx.playback().1,
        2,
        "an out-of-range index clamps to len - 1"
    );
}

#[test]
fn manual_nav_breaks_follow() {
    // A manual navigation takes control: it clears the follow and says so.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    let alice = UserId::new();
    st.handle_command(ToileCommand::FollowPresenter(Some(alice)));
    assert_eq!(st.following(), Some(alice));

    let out = st.handle_command(ToileCommand::NextSlide);
    assert!(st.following().is_none(), "manual nav takes control");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::FollowingChanged(None))),
        "clearing the follow is announced to the UI"
    );
    assert_eq!(slide_changed(&out), Some(1), "the navigation still happens");
}

#[test]
fn follow_snaps_to_last_known_slide() {
    // Following a peer whose broadcast slide is already known snaps at once.
    let mut rx = ToileState::new();
    deck_of_three(&mut rx);
    let alice = UserId::new();
    // Store a broadcast frame from alice before following her.
    let frame = postcard::to_allocvec(&PresencePayload {
        present_slide: Some(2),
        ..Default::default()
    })
    .unwrap();
    rx.apply_ephemeral(alice, &frame);
    assert_eq!(rx.playback().1, 0, "no adoption before following");

    let out = rx.handle_command(ToileCommand::FollowPresenter(Some(alice)));
    assert_eq!(
        rx.playback().1,
        2,
        "following snaps to the last known slide"
    );
    assert_eq!(
        slide_changed(&out),
        Some(2),
        "the snap glides via SlideChanged"
    );
}

#[test]
fn present_broadcast_toggle_is_local() {
    // Toggling broadcast is a local role: a CRDT delta is never produced.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    let out = st.handle_command(ToileCommand::SetPresentBroadcast(true));
    assert!(out.update.is_none(), "the broadcast toggle is never CRDT");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentBroadcastChanged(true))),
        "the toggle is announced to the UI"
    );
    assert!(st.is_broadcasting());
}

#[test]
fn follow_state_is_not_synced() {
    // Follow / broadcast toggles never produce a CRDT delta, and one peer's
    // toggles never touch another peer's playback (mirrors playback_is_not_synced).
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    deck_of_three(&mut a);
    b.apply_remote(&a.export_snapshot());

    let alice = UserId::new();
    assert!(
        a.handle_command(ToileCommand::FollowPresenter(Some(alice)))
            .update
            .is_none()
    );
    assert!(
        a.handle_command(ToileCommand::SetPresentBroadcast(true))
            .update
            .is_none()
    );
    assert_eq!(
        b.playback(),
        (false, 0, false),
        "B's playback is untouched by A's follow / broadcast toggles"
    );
}

// -----------------------------------------------------------------------------
// Speaker notes (toile-presentation)
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

    // Each EditNotes is one undo item: undo drops the last splice only.
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
    // The handler clamps an `at` past the end to the length, so the insert lands
    // at the tail instead of earning a refusal.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        delta: insert_notes(999, "Z"),
    });
    assert_eq!(st.notes(&f), "abcZ");
    // The handler clamps an oversized deletion span to what remains.
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
    // Two peers share a frame (and thus its seeded notes LoroText,), then
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
// Auto-layout spec register (toile-auto-layout)
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

    // One SetAutoLayout is one undo item: undo reverts to the prior spec.
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
// Reactions / votes (toile-reactions)
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
fn derive_viewport_carries_tally_and_skips_deleted_target() {
    let (mut st, t) = state_with_user(UserId::new());
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("thumbsup")),
    });

    // The derived viewport attaches the per-target tally, keyed by target.
    let dv = st.debug_viewport();
    let tally = dv
        .reactions
        .get(&t)
        .expect("tally present for reacted target");
    assert_eq!(tally.emoji, vec![("thumbsup".to_owned(), 1)]);

    // Deleting the target drops it from the derived map. Its orphan reaction
    // entries survive in the CRDT, and derivation skips them.
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![t.clone()],
    });
    let dv = st.debug_viewport();
    assert!(
        !dv.reactions.contains_key(&t),
        "a deleted target must not appear in the derived reaction map"
    );
}

// -----------------------------------------------------------------------------
// doc_embed: the spatial reference to a nested document
// -----------------------------------------------------------------------------

/// Pull the single `DocEmbed` geometry out of the derived viewport.
fn only_doc_embed(st: &ToileState) -> Option<DisplayGeom> {
    st.debug_viewport().nodes.into_iter().find_map(|n| match n {
        DisplayNode::Element(e) if matches!(e.geom, DisplayGeom::DocEmbed { .. }) => Some(e.geom),
        _ => None,
    })
}

#[test]
fn doc_embed_reference_survives_the_crdt_round_trip() {
    // The embed's whole purpose is to name another document. That reference goes
    // into the CRDT as a string and comes back out through `derive`. This test
    // asserts the pair survives verbatim. A `document_id` that does not
    // round-trip points the child session at the wrong document, or nowhere.
    let mut st = ToileState::new();
    let child = ambre_core::DocumentId::new();

    st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::DocEmbed,
        at: Point::new(10.0, 10.0),
        size: Size::new(400.0, 300.0),
        content: InitContent::DocEmbed {
            document_id: child,
            doc_type: ambre_core::DocType::Trame,
        },
        fill: None,
        correlate: 0,
    });

    let geom = only_doc_embed(&st).expect("the embed must derive as DocEmbed");
    let DisplayGeom::DocEmbed {
        document_id,
        doc_type,
        focused,
    } = geom
    else {
        unreachable!("filtered above")
    };
    assert_eq!(document_id, child, "the child reference did not round-trip");
    assert_eq!(doc_type, ambre_core::DocType::Trame);
    // Focus is view-local state, never CRDT: derive always says false.
    assert!(!focused, "focus must not be derived from the document");
}

#[test]
fn doc_embed_with_an_unreadable_reference_falls_back_to_a_frame() {
    // A document written by a newer/broken peer must still paint something. The
    // element degrades to a plain frame rather than vanishing from the board.
    let mut st = ToileState::new();
    st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::DocEmbed,
        at: Point::ZERO,
        size: Size::new(100.0, 100.0),
        fill: None,
        content: InitContent::DocEmbed {
            document_id: ambre_core::DocumentId::new(),
            doc_type: ambre_core::DocType::Toile,
        },
        correlate: 0,
    });
    let id = st
        .debug_viewport()
        .nodes
        .into_iter()
        .find_map(|n| match n {
            DisplayNode::Element(e) => Some(e.id),
            _ => None,
        })
        .expect("element present");

    // Corrupt the stored reference behind the engine's back.
    st.debug_set_element_field(&id, "doc_id", "not-a-uuid");

    assert!(
        only_doc_embed(&st).is_none(),
        "an unparsable reference must not derive as a live embed"
    );
    let geom = st.debug_viewport().nodes.into_iter().find_map(|n| match n {
        DisplayNode::Element(e) => Some(e.geom),
        _ => None,
    });
    assert!(
        matches!(geom, Some(DisplayGeom::Frame)),
        "expected a Frame placeholder, got {geom:?}"
    );
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

    // One React is one undo item: undo drops the reaction.
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

    // Deleting the target skips its reaction sub-map at derivation: the
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
    // collapse to a single entry, keyed by the canonical UserId. A double-count
    // is the failure this guards.
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
// Comment threads (toile-comments)
// -----------------------------------------------------------------------------

/// The thread the last command opened. Only `CreateComment` emits
/// `CommentThreadOpened`, so the event names a freshly minted thread.
fn opened_thread(out: &crate::logic::command::ToileOutcome) -> ThreadId {
    out.display
        .iter()
        .find_map(|e| match e {
            ToileEvent::CommentThreadOpened(t) => Some(t.clone()),
            _ => None,
        })
        .expect("expected a CommentThreadOpened event")
}

/// Open a thread on `anchor` and return its minted id.
fn comment_on(st: &mut ToileState, anchor: CommentAnchor, body: &str, ts: i64) -> ThreadId {
    let out = st.handle_command(ToileCommand::CreateComment {
        anchor,
        body: body.to_owned(),
        ts,
    });
    assert!(out.update.is_some(), "a comment must produce a CRDT delta");
    opened_thread(&out)
}

/// The bodies of a thread's messages, in list order.
fn bodies(st: &ToileState, thread: &ThreadId) -> Vec<String> {
    st.comment_thread(thread)
        .map(|t| t.messages.into_iter().map(|m| m.body).collect())
        .unwrap_or_default()
}

#[test]
fn a_thread_anchors_to_an_element_or_to_a_world_point() {
    // TOILE-024: both anchor shapes survive the CRDT round trip, and neither is
    // a viewport coordinate.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");

    let on_element = comment_on(&mut st, CommentAnchor::Element(e.clone()), "on the note", 0);
    let on_board = comment_on(
        &mut st,
        CommentAnchor::Point { x: 40.0, y: -12.5 },
        "on the board",
        0,
    );

    assert_eq!(
        st.comment_thread(&on_element).map(|t| t.anchor),
        Some(CommentAnchor::Element(e)),
    );
    assert_eq!(
        st.comment_thread(&on_board).map(|t| t.anchor),
        Some(CommentAnchor::Point { x: 40.0, y: -12.5 }),
    );
}

#[test]
fn a_message_author_is_the_user_id_of_the_session() {
    // TOILE-FEAT-014: the message carries the person, not the session. A board
    // with no authenticated session writes nothing at all.
    let me = UserId::new();
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Point { x: 0.0, y: 0.0 },
        body: "too early".to_owned(),
        ts: 0,
    });
    assert!(
        out.update.is_none(),
        "a comment before SessionReady writes nothing"
    );
    assert!(st.comment_thread_ids().is_empty());

    st.set_user_id(me);
    let t = comment_on(&mut st, CommentAnchor::Point { x: 0.0, y: 0.0 }, "hello", 7);
    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].author, me);
    assert_eq!(thread.messages[0].ts, 7);
    assert!(!thread.resolved);
}

#[test]
fn two_peers_commenting_on_one_spot_mint_distinct_threads() {
    // TOILE-FEAT-011: a minted id per thread makes the collision
    // unrepresentable. Keyed by the anchor element, one of the two would lose.
    let mut a = ToileState::new();
    a.set_user_id(UserId::new());
    let e = create_sticky(&mut a, Point::ZERO, "note");

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote(&a.export_snapshot());

    let out_a = a.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Element(e.clone()),
        body: "from A".to_owned(),
        ts: 0,
    });
    let out_b = b.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Element(e),
        body: "from B".to_owned(),
        ts: 0,
    });
    let (ta, tb) = (opened_thread(&out_a), opened_thread(&out_b));
    assert_ne!(ta, tb, "two peers must mint two thread ids");

    b.apply_remote(&out_a.update.expect("A's comment broadcasts a delta"));
    a.apply_remote(&out_b.update.expect("B's comment broadcasts a delta"));

    assert_eq!(a.comment_thread_ids(), b.comment_thread_ids());
    assert_eq!(
        a.comment_thread_ids().len(),
        2,
        "neither thread overwrote the other"
    );
    assert_eq!(bodies(&a, &ta), vec!["from A".to_owned()]);
    assert_eq!(bodies(&a, &tb), vec!["from B".to_owned()]);
}

#[test]
fn concurrent_replies_converge_in_list_order_not_clock_order() {
    // TOILE-FEAT-012: the order both readers see is the list order. The clocks
    // here disagree with it on purpose: A's reply carries the later timestamp.
    let mut a = ToileState::new();
    a.set_user_id(UserId::new());
    let t = comment_on(&mut a, CommentAnchor::Point { x: 0.0, y: 0.0 }, "first", 50);

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: "from A".to_owned(),
            ts: 900,
        })
        .update
        .expect("A's reply broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: "from B".to_owned(),
            ts: 1,
        })
        .update
        .expect("B's reply broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let seen = bodies(&a, &t);
    assert_eq!(seen, bodies(&b, &t), "concurrent replies diverged");
    assert_eq!(seen.len(), 3, "both replies survived");
    assert_eq!(seen[0], "first", "the seed message stays first");
}

#[test]
fn concurrent_edits_of_one_message_converge_character_wise() {
    // TOILE-FEAT-013: one author on two devices edits their own message. A
    // whole-string register would drop one edit in silence.
    let me = UserId::new();
    let mut a = ToileState::new();
    a.set_user_id(me);
    let t = comment_on(&mut a, CommentAnchor::Point { x: 0.0, y: 0.0 }, "typo", 0);

    let mut b = ToileState::new();
    b.set_user_id(me);
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::EditComment {
            thread: t.clone(),
            message: 0,
            delta: insert_notes(0, "AAA"),
        })
        .update
        .expect("device A's edit broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::EditComment {
            thread: t.clone(),
            message: 0,
            delta: insert_notes(0, "BBB"),
        })
        .update
        .expect("device B's edit broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let merged = bodies(&a, &t);
    assert_eq!(merged, bodies(&b, &t), "concurrent body edits diverged");
    let body = merged.first().cloned().unwrap_or_default();
    assert!(
        body.contains("AAA") && body.contains("BBB") && body.contains("typo"),
        "both edits and the original text must survive, got {body:?}"
    );
}

#[test]
fn editing_another_authors_message_is_refused() {
    // No rule says who may edit whose message
    // (`features/ambre-spec-toile-comments.md section 6`), so the engine allows
    // the one case that needs no rule: the author's own message.
    let mut a = ToileState::new();
    a.set_user_id(UserId::new());
    let t = comment_on(&mut a, CommentAnchor::Point { x: 0.0, y: 0.0 }, "mine", 0);

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote(&a.export_snapshot());

    let out = b.handle_command(ToileCommand::EditComment {
        thread: t.clone(),
        message: 0,
        delta: insert_notes(0, "hijacked"),
    });
    assert!(
        out.update.is_none(),
        "a refused edit must not mutate the doc"
    );
    assert_eq!(bodies(&b, &t), vec!["mine".to_owned()]);
}

#[test]
fn concurrent_resolves_converge_and_keep_the_messages() {
    // TOILE-FEAT-015: resolving is idempotent and convergent, and it destroys
    // neither the thread nor its messages.
    let mut a = ToileState::new();
    a.set_user_id(UserId::new());
    let t = comment_on(
        &mut a,
        CommentAnchor::Point { x: 0.0, y: 0.0 },
        "settled?",
        0,
    );

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::ResolveThread {
            thread: t.clone(),
            resolved: true,
        })
        .update
        .expect("A's resolve broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::ResolveThread {
            thread: t.clone(),
            resolved: true,
        })
        .update
        .expect("B's resolve broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let ra = a.comment_thread(&t).expect("A still holds the thread");
    let rb = b.comment_thread(&t).expect("B still holds the thread");
    assert_eq!(ra, rb, "concurrent resolves diverged");
    assert!(ra.resolved, "the thread reads resolved on both peers");
    assert_eq!(ra.messages.len(), 1, "resolving keeps the messages");

    // Idempotent: resolving an already-resolved thread writes nothing.
    let again = a.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        resolved: true,
    });
    assert!(again.update.is_none(), "a redundant resolve is not a write");

    // Reopening restores the thread whole.
    a.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        resolved: false,
    });
    let reopened = a.comment_thread(&t).expect("the thread survives a reopen");
    assert!(!reopened.resolved);
    assert_eq!(reopened.messages.len(), 1);
}

#[test]
fn a_thread_survives_the_deletion_of_its_anchor_element() {
    // TOILE-FEAT-016 / TOILE-025: the element may come back through an undo.
    // Deleting the discussion because the drawing changed destroys the record of
    // why it changed. The board stops painting the pin, and that half belongs to
    // the derived viewport.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");
    let t = comment_on(
        &mut st,
        CommentAnchor::Element(e.clone()),
        "about the note",
        0,
    );

    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![e.clone()],
    });

    let thread = st
        .comment_thread(&t)
        .expect("the thread outlives its anchor");
    assert_eq!(thread.anchor, CommentAnchor::Element(e));
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(st.comment_thread_ids(), vec![t]);
}

#[test]
fn undo_reverts_the_drawing_and_never_the_comment() {
    // TOILE-FEAT-017: Ctrl+Z reverts what the user drew. Withdrawing a message a
    // teammate has already read is a deletion nobody asked for.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");
    let t = comment_on(&mut st, CommentAnchor::Element(e), "already read", 0);

    st.handle_command(ToileCommand::Undo);

    assert_eq!(
        st.element_count(),
        0,
        "undo reverts the last drawing action"
    );
    assert_eq!(
        bodies(&st, &t),
        vec!["already read".to_owned()],
        "the message stays on the board"
    );

    // The comment left no item of its own either: a second undo finds an empty
    // stack rather than the thread.
    let out = st.handle_command(ToileCommand::Undo);
    assert!(out.update.is_none(), "the undo stack holds no comment item");
    assert_eq!(bodies(&st, &t).len(), 1);
}

#[test]
fn replying_to_a_missing_thread_is_a_rejected_no_op() {
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let out = st.handle_command(ToileCommand::ReplyComment {
        thread: ElementId("ghost".to_owned()),
        body: "nobody".to_owned(),
        ts: 0,
    });
    assert!(
        out.update.is_none(),
        "a rejected reply must not mutate the doc"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::NoSuchElement(_)))),
    );
}

/// The pins the board would paint for the default camera.
fn pins(st: &ToileState) -> Vec<crate::logic::command::CommentPin> {
    st.debug_viewport().pins
}

/// The events one `ReadComments` answers.
fn read_comments(st: &mut ToileState, thread: Option<ThreadId>) -> Vec<ToileEvent> {
    let out = st.handle_command(ToileCommand::ReadComments { thread });
    assert!(out.update.is_none(), "a read must not mutate the doc");
    out.display
}

#[test]
fn a_fresh_thread_is_open_anchored_and_carries_its_first_message() {
    // TOILE-FEAT-070: the three clauses of a drop, read back off the document.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");
    let t = comment_on(&mut st, CommentAnchor::Element(e.clone()), "the point", 11);

    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert!(!thread.resolved, "a dropped thread is open");
    assert_eq!(thread.anchor, CommentAnchor::Element(e));
    assert_eq!(
        thread
            .messages
            .iter()
            .map(|m| m.body.clone())
            .collect::<Vec<_>>(),
        vec!["the point".to_owned()],
        "the thread carries the message it was dropped with"
    );
}

#[test]
fn a_reply_lands_at_the_end_of_the_thread() {
    // TOILE-FEAT-071: the rule is an order, not a count. A reply inserted before
    // an earlier one fails here.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let t = comment_on(&mut st, CommentAnchor::Point { x: 0.0, y: 0.0 }, "one", 0);
    for body in ["two", "three"] {
        st.handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: body.to_owned(),
            ts: 0,
        });
    }
    assert_eq!(
        bodies(&st, &t),
        vec!["one".to_owned(), "two".to_owned(), "three".to_owned()]
    );
}

#[test]
fn correcting_a_message_leaves_its_neighbours_as_they_read() {
    // TOILE-FEAT-072: the frame half of a correction, which needs an untouched
    // neighbour to read back.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let t = comment_on(&mut st, CommentAnchor::Point { x: 0.0, y: 0.0 }, "first", 0);
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "secnd".to_owned(),
        ts: 0,
    });
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "third".to_owned(),
        ts: 0,
    });

    // Correct the middle message: insert the missing letter.
    st.handle_command(ToileCommand::EditComment {
        thread: t.clone(),
        message: 1,
        delta: TextDelta {
            at: 3,
            delete: 0,
            insert: "o".to_owned(),
        },
    });

    assert_eq!(
        bodies(&st, &t),
        vec!["first".to_owned(), "second".to_owned(), "third".to_owned()],
        "only the corrected message changed"
    );
}

#[test]
fn a_reply_moves_neither_the_pin_nor_the_resolved_flag() {
    // TOILE-FEAT-073: writing in a thread touches the message list alone.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let anchor = CommentAnchor::Point { x: 12.0, y: -4.0 };
    let t = comment_on(&mut st, anchor.clone(), "open it", 0);
    st.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        resolved: true,
    });
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "one more".to_owned(),
        ts: 0,
    });

    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert_eq!(thread.anchor, anchor, "a reply does not move the pin");
    assert!(thread.resolved, "a reply does not reopen the thread");
}

#[test]
fn a_resolve_keeps_every_message_body() {
    // TOILE-FEAT-074: a settled point stays readable. A resolve that blanked the
    // message it kept fails here.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let t = comment_on(&mut st, CommentAnchor::Point { x: 0.0, y: 0.0 }, "why", 0);
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "because".to_owned(),
        ts: 0,
    });
    st.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        resolved: true,
    });
    assert_eq!(
        bodies(&st, &t),
        vec!["why".to_owned(), "because".to_owned()]
    );
}

#[test]
fn two_peers_answering_one_brand_new_thread_both_keep_their_reply() {
    // TOILE-FEAT-068: the birth seeding. A lazily created message list would let
    // each peer mint a rival container, and Loro would keep one.
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    a.set_user_id(UserId::new());
    b.set_user_id(UserId::new());

    let t = comment_on(&mut a, CommentAnchor::Point { x: 0.0, y: 0.0 }, "seed", 0);
    b.apply_remote(&a.export_snapshot());

    let ra = a.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "from A".to_owned(),
        ts: 0,
    });
    let rb = b.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "from B".to_owned(),
        ts: 0,
    });
    b.apply_remote(&ra.update.expect("A's reply broadcasts a delta"));
    a.apply_remote(&rb.update.expect("B's reply broadcasts a delta"));

    let merged = bodies(&a, &t);
    assert_eq!(merged, bodies(&b, &t), "both peers read the same thread");
    assert_eq!(merged.len(), 3, "neither reply wrote over the other");
    assert!(merged.contains(&"from A".to_owned()) && merged.contains(&"from B".to_owned()));
}

#[test]
fn a_pin_carries_its_place_its_count_and_its_state() {
    // TOILE-026: the board says where each pin sits, how many messages it holds
    // and whether anyone has closed it.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::new(10.0, 20.0), "note");
    let on_element = comment_on(&mut st, CommentAnchor::Element(e), "about it", 0);
    st.handle_command(ToileCommand::ReplyComment {
        thread: on_element.clone(),
        body: "agreed".to_owned(),
        ts: 0,
    });
    let on_board = comment_on(&mut st, CommentAnchor::Point { x: 5.0, y: 6.0 }, "here", 0);
    st.handle_command(ToileCommand::ResolveThread {
        thread: on_board.clone(),
        resolved: true,
    });

    let pins = pins(&st);
    let element_pin = pins
        .iter()
        .find(|p| p.thread == on_element)
        .expect("the anchored thread has a pin");
    assert_eq!(element_pin.messages, 2);
    assert!(!element_pin.resolved);
    // The pin sits on the element it is about: its top-right corner.
    assert_eq!(element_pin.at, Point::new(90.0, 20.0));

    let board_pin = pins
        .iter()
        .find(|p| p.thread == on_board)
        .expect("the free-point thread has a pin");
    assert_eq!(board_pin.at, Point::new(5.0, 6.0));
    assert!(board_pin.resolved);
}

#[test]
fn a_pin_leaves_the_board_with_its_anchor_element() {
    // TOILE-025: a pin whose element is gone is not painted -- and the thread
    // itself stays, which `a_thread_survives_the_deletion_of_its_anchor_element`
    // holds.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");
    let t = comment_on(&mut st, CommentAnchor::Element(e.clone()), "about it", 0);
    assert_eq!(pins(&st).len(), 1);

    st.handle_command(ToileCommand::DeleteElements { ids: vec![e] });

    assert!(pins(&st).is_empty(), "a dangling pin is not painted");
    assert_eq!(
        st.comment_thread_ids(),
        vec![t],
        "and the thread is untouched"
    );
}

#[test]
fn the_board_carries_no_body_and_a_read_carries_them_all() {
    // TOILE-205: opening a thread reads it. The pin type has no body field at
    // all, so the compiler holds half of this. The other half is that the read
    // answers with the bodies, and nothing else does.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let t = comment_on(
        &mut st,
        CommentAnchor::Point { x: 0.0, y: 0.0 },
        "the body",
        0,
    );

    // The board-wide list carries the opening line and the counts, no more.
    let list = read_comments(&mut st, None);
    let Some(ToileEvent::CommentList(rows)) = list.first() else {
        panic!("expected a CommentList, got {list:?}");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, t);
    assert_eq!(rows[0].messages, 1);
    assert_eq!(rows[0].preview, "the body");

    // The whole thread, bodies included, only on an explicit read.
    let one = read_comments(&mut st, Some(t.clone()));
    let Some(ToileEvent::CommentThreadData(thread)) = one.first() else {
        panic!("expected a CommentThreadData, got {one:?}");
    };
    assert_eq!(thread.id, t);
    assert_eq!(thread.messages[0].body, "the body");

    // An unknown thread answers nothing rather than an empty thread.
    assert!(
        read_comments(&mut st, Some(ElementId("ghost".to_owned()))).is_empty(),
        "a missing thread answers nothing"
    );
}

// -----------------------------------------------------------------------------
// 2-peer CRDT convergence property test (DoD)
// -----------------------------------------------------------------------------

/// A local command a peer issues, parameterized over a small shared id pool.
/// The replay resolves that pool, so both peers can name each other's creations
/// after a sync round.
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
    // Auto-layout spec -- concurrent LWW register edits on a shared frame.
    SetAutoLayoutNth {
        nth: usize,
        dir: u8,
        gap: f64,
    },
    // Reactions -- concurrent per-user entries on a shared target. Each
    // peer holds a distinct `UserId`, so two reactions on one target must both
    // survive and the derived tally must converge.
    ReactNth {
        nth: usize,
        emoji: bool,
        weight: i32,
    },
    // Align / distribute -- one batch rewriting the coordinates of every live
    // element at once. Two peers aligning the same board concurrently write the
    // same registers, which is exactly where a batch could tear.
    AlignAll {
        op: u8,
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
        (0u8..8).prop_map(|op| PeerOp::AlignAll { op }),
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
            fill: None,
            correlate: 0,
        }),
        PeerOp::MoveNth { nth: n, dx, dy } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::MoveElements {
                ids: vec![id],
                delta: Vec2::new(*dx, *dy),
            }),
            None => return None,
        },
        PeerOp::AlignAll { op } => {
            let op = [
                AlignOp::Left,
                AlignOp::HCenter,
                AlignOp::Right,
                AlignOp::Top,
                AlignOp::VCenter,
                AlignOp::Bottom,
                AlignOp::DistributeH,
                AlignOp::DistributeV,
            ][usize::from(*op) % 8];
            st.handle_command(ToileCommand::AlignElements {
                ids: ids.clone(),
                op,
            })
        }
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
            // A no-op on a non-group returns no delta, and `out.update` filters it.
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
            fill: None,
            correlate: 0,
        }),
        PeerOp::SetSlideOrderRot { by } => {
            // Rotate the current live-id set to build a permutation, so concurrent
            // peers set overlapping-but-different slide orders (convergence).
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

/// A viewport wide enough to cull nothing, so `debug_viewport()` returns the
/// full derived paint set for cross-peer / rebuild comparison.
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
        // Distinct canonical identities, as `SessionReady` would seed them. A
        // `ReactNth` on each peer then writes under its own key, and concurrent
        // reactions on one target must both survive.
        a.set_user_id(UserId::new());
        b.set_user_id(UserId::new());

        // Interleave: each peer applies its op locally, then the round exchanges
        // both pending deltas. Operations therefore target a partly-shared state.
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
        // converge on the shared `LoroMovableList<FrameId>`.
        prop_assert_eq!(a.slide_order(), b.slide_order(), "slide order diverged");

        // Identical auto-layout spec on every surviving element: concurrent
        // `SetAutoLayout` edits converge on the shared LWW register.
        for id in &la {
            prop_assert_eq!(
                a.auto_layout(id), b.auto_layout(id),
                "auto-layout spec diverged for {:?}", id
            );
        }

        // Identical derived reaction tally on every surviving target: concurrent
        // per-user reactions converge and no vote is double-counted.
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

    /// The invariant beyond the isolated `SpatialIndex`. Every derived cache
    /// `ToileState` holds (bounds, layers, group unions, routed connectors) must
    /// equal what a full rebuild from the CRDT snapshot produces. Only
    /// `apply_remote` feeds `b`, which is the `rebuild_caches` path, so this
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

// -----------------------------------------------------------------------------
// A command that names nothing answers an error
// -----------------------------------------------------------------------------

/// `TOILE-194`: a move naming an element the board does not hold answers an
/// error, not a silent skip.
#[test]
fn a_move_naming_a_missing_element_answers_an_error() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::MoveElements {
        ids: vec![ElementId("ghost".to_owned())],
        delta: Vec2::new(1.0, 1.0),
    });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NoSuchElement(_))]
    ));
}

/// `TOILE-194`: a restyle naming a missing element answers an error.
#[test]
fn a_restyle_naming_a_missing_element_answers_an_error() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::RestyleElements {
        ids: vec![ElementId("ghost".to_owned())],
        style: ElementStyle::default(),
    });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NoSuchElement(_))]
    ));
}

/// `TOILE-201`: a delete naming a missing element answers an error.
#[test]
fn a_delete_naming_a_missing_element_answers_an_error() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::DeleteElements {
        ids: vec![ElementId("ghost".to_owned())],
    });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NoSuchElement(_))]
    ));
}

/// `TOILE-055`: a group of one refuses loudly, not in silence.
#[test]
fn a_group_of_one_answers_an_error() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::GroupElements { ids: vec![a] });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::GroupNeedsTwo)]
    ));
}

/// `TOILE-159`: ungrouping an id that is not a group refuses loudly.
#[test]
fn an_ungroup_of_a_non_group_answers_an_error() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::UngroupElements { group: a });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NotAGroup(_))]
    ));
}

/// `TOILE-FEAT-060`: a reaction on a target the board no longer holds answers
/// an error instead of storing an entry nothing will render.
#[test]
fn a_reaction_on_a_missing_target_answers_an_error() {
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let out = st.handle_command(ToileCommand::React {
        target: ElementId("ghost".to_owned()),
        reaction: Some(emoji("thumbsup")),
    });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NoSuchElement(_))]
    ));
}
