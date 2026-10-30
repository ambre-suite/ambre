//! Engine lifecycle tests and the 2-peer CRDT convergence property test.
//!
//! The convergence test drives two independent [`ToileState`]s. It relays every
//! local `update` delta to the other peer through `apply_remote`. It then
//! asserts that the two peers reach an identical `LoroDoc`, and identical
//! derived caches (`bounds` / `layers`). The whiteboard has no computed values
//! to compare, only state and derived structures.

use ambre_core::Rgba;
use ambre_core::{CharCount, CharOffset, ElementCount, StepDirection};
use kurbo::{Point, Rect, Size, Vec2};
use proptest::prelude::*;

use ambre_bus::UserId;

use crate::logic::align::AlignOp;
use crate::logic::autolayout::{Align, AutoLayout, Direction, Sizing};
use crate::logic::command::{
    BoolOp, Broadcast, CommentAnchor, DisplayGeom, DisplayNode, End, InitContent, PresencePayload,
    PresentFocus, PresentMode, Reaction, ReactionKind, SlideIndex, Sources, StatsRequestKind,
    TextDelta, ThreadMessageIndex, ToileCommand, ToileError, ToileEvent, ZTarget,
};
use crate::logic::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, LinkTarget, NodeId,
    PathType, ShapeKind, StrokePoint, ThreadId,
};
use crate::logic::state::{PRESENCE_TTL, ToileState};

// -----------------------------------------------------------------------------
// Lifecycle unit tests
// -----------------------------------------------------------------------------

/// `TOILE-265`: a reader that opened a board and wrote nothing holds no
/// history of its own. Each change it then receives follows the history it
/// holds, so the engine imports it at the cost of the change.
#[test]
fn a_reader_that_sent_nothing_holds_no_history_of_its_own() {
    let mut author = ToileState::new();
    let id = create_rect(&mut author, Point::new(0.0, 0.0), Size::new(10.0, 10.0));
    let mut reader = ToileState::new();
    reader.apply_remote_snapshot(&author.export_snapshot());
    assert_eq!(
        reader.history(),
        author.history(),
        "the scaffolding is shared"
    );
    let out = author.handle_command(ToileCommand::MoveElements {
        ids: vec![id],
        delta: Vec2::new(5.0, 0.0),
    });
    reader.apply_remote(&out.update.expect("a move writes"));
    assert_eq!(reader.history(), author.history());
}

/// Holds `SYNC-101` for Toile: a remote delta answers display events alone,
/// and the next local delta carries none of the remote peer's operations.
#[test]
fn a_remote_change_is_never_re_broadcast() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    // B's own genesis goes out with its first edit, before any import.
    create_rect(&mut b, Point::new(0.0, 0.0), Size::new(10.0, 10.0));
    let from_a = a
        .handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(50.0, 0.0),
            size: Size::new(10.0, 10.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
        })
        .update
        .expect("a create writes");
    let events = b.apply_remote(&from_a);
    assert!(events.iter().any(|e| matches!(e, ToileEvent::BoardChanged)));

    let out = b.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(90.0, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    let from_b = out.update.expect("a create writes");
    let sent = loro::LoroDoc::decode_import_blob_meta(from_a.as_bytes(), false).expect("a delta");
    let meta = loro::LoroDoc::decode_import_blob_meta(from_b.as_bytes(), false).expect("a delta");
    for (peer, _) in sent.partial_end_vv.iter() {
        assert_eq!(
            meta.partial_end_vv.get(peer),
            meta.partial_start_vv.get(peer),
            "the delta re-exports none of A's operations"
        );
    }
}

/// `TOILE-092`: a peer's move of one element on a board of 500 refreshes that
/// element alone. The receiving board resolves no element map and walks no
/// index, and the moved element lands where the peer put it.
#[test]
fn a_remote_move_walks_no_board() {
    let mut a = ToileState::new();
    let ids: Vec<ElementId> = (0..500)
        .map(|i| {
            create_rect(
                &mut a,
                Point::new(i as f64 * 20.0, 0.0),
                Size::new(10.0, 10.0),
            )
        })
        .collect();
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let walks = b.board_walks();
    let out = a.handle_command(ToileCommand::MoveElements {
        ids: vec![ids[7].clone()],
        delta: Vec2::new(0.0, 300.0),
    });
    b.apply_remote(&out.update.expect("a move writes"));
    assert_eq!(
        b.board_walks(),
        walks,
        "the refresh resolved no element map"
    );
    assert_eq!(b.index_walks(), 0, "the refresh walked no index");
    assert_eq!(b.bounds_of(&ids[7]), a.bounds_of(&ids[7]));
}

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
        correlate: None,
    });
    assert!(out.update.is_some(), "a create must produce a CRDT delta");
    created_id(&out)
}

#[test]
fn create_registers_bounds_and_layer() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::new(10.0, 20.0), Size::new(100.0, 50.0));
    assert_eq!(st.element_count(), ElementCount::new(1));
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
        correlate: None,
    });
    let id = created_id(&out);
    assert_eq!(body_of(&st, &id).as_deref(), Some("hello"));

    let out = st.handle_command(ToileCommand::SetText {
        id: id.clone(),
        base: "hello".to_owned(),
        text: "bonjour monde".to_owned(),
    });
    assert!(
        out.update.is_some(),
        "a text edit must produce a CRDT delta"
    );
    assert_eq!(body_of(&st, &id).as_deref(), Some("bonjour monde"));
}

/// `TOILE-UI-064`: the whole-body write an editor sends at its close is one
/// undo item, however far the body moved. One undo restores the body it
/// opened on.
#[test]
fn a_body_write_is_one_undo_item() {
    let mut st = ToileState::new();
    let id = create_sticky(&mut st, Point::ZERO, "hi");
    set_body(&mut st, &id, "hi", "oh, hi there!");
    st.handle_command(ToileCommand::Undo);
    assert_eq!(body_of(&st, &id).as_deref(), Some("hi"));
}

#[test]
fn set_text_rejects_a_non_text_element() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::SetText {
        id,
        base: String::new(),
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

/// Write `text` over the body `st` holds now, as an editor opened on it would.
fn set_body(
    st: &mut ToileState,
    id: &ElementId,
    base: &str,
    text: &str,
) -> crate::logic::command::ToileOutcome {
    st.handle_command(ToileCommand::SetText {
        id: id.clone(),
        base: base.to_owned(),
        text: text.to_owned(),
    })
}

/// Two authors write one body at once, each sending the whole body as the
/// editor does, then each receives the other's delta.
fn race_two_bodies(a: &mut ToileState, b: &mut ToileState, id: &ElementId, ta: &str, tb: &str) {
    let base = body_of(a, id).expect("a body");
    let da = set_body(a, id, &base, ta)
        .update
        .expect("A's edit broadcasts a delta");
    let db = set_body(b, id, &base, tb)
        .update
        .expect("B's edit broadcasts a delta");
    b.apply_remote(&da);
    a.apply_remote(&db);
}

/// `TOILE-030`: two authors typing in one note both keep their words. Each
/// sends the whole body, and the write merges it character by character, so
/// neither sentence is lost to the other.
#[test]
fn two_authors_typing_in_one_note_both_keep_their_words() {
    for kind in [ElementKind::Sticky, ElementKind::Text] {
        let mut a = ToileState::new();
        let mut b = ToileState::new();
        let id = created_id(&a.handle_command(ToileCommand::CreateElement {
            kind,
            at: Point::ZERO,
            size: Size::new(120.0, 80.0),
            content: InitContent::Text("buy milk".to_owned()),
            fill: None,
            correlate: None,
        }));
        b.apply_remote_snapshot(&a.export_snapshot());

        race_two_bodies(&mut a, &mut b, &id, "buy milk and eggs", "please buy milk");

        let merged = body_of(&a, &id).expect("a body");
        assert_eq!(
            Some(&merged),
            body_of(&b, &id).as_ref(),
            "the bodies diverged"
        );
        assert_eq!(
            merged, "please buy milk and eggs",
            "{kind:?} lost a sentence"
        );
    }
}

/// `TOILE-UI-381`: B opens an editor on "buy milk", and A's edit lands before
/// B closes it. B's close names the body it opened on, so it adds B's words
/// to A's rather than deleting them. A peer that writes no change keeps the
/// stored body whole.
#[test]
fn an_open_editor_keeps_a_peers_edit() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let id = create_sticky(&mut a, Point::ZERO, "buy milk");
    b.apply_remote_snapshot(&a.export_snapshot());

    let opened = body_of(&b, &id).expect("a body");
    let da = set_body(&mut a, &id, "buy milk", "please buy milk")
        .update
        .expect("A's edit broadcasts a delta");
    b.apply_remote(&da);
    let db = set_body(&mut b, &id, &opened, "buy milk and eggs")
        .update
        .expect("B's close broadcasts a delta");
    a.apply_remote(&db);

    assert_eq!(
        body_of(&b, &id).as_deref(),
        Some("please buy milk and eggs")
    );
    assert_eq!(body_of(&a, &id), body_of(&b, &id), "the bodies diverged");

    let unchanged = set_body(&mut b, &id, "buy milk", "buy milk");
    assert!(unchanged.update.is_none(), "an unchanged close wrote");
    assert_eq!(
        body_of(&b, &id).as_deref(),
        Some("please buy milk and eggs")
    );
}

/// `TOILE-UI-382`: the view echoes a close as `merge_text` over the body it
/// opened on, the body its viewport shows, and the typed body. On a note and
/// on a caption, that echo is the very text Logic then writes, so a peer's
/// words never leave the screen.
#[test]
fn a_close_echo_paints_what_the_document_keeps() {
    use crate::logic::text_merge::merge_text;

    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let id = create_sticky(&mut a, Point::ZERO, "buy milk");
    let c = create_connector(&mut a);
    a.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        base: String::new(),
        text: "cost".into(),
    });
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = set_body(&mut a, &id, "buy milk", "please buy milk")
        .update
        .expect("A's edit broadcasts a delta");
    b.apply_remote(&da);
    let shown = body_of(&b, &id).expect("a body");
    let echo = merge_text("buy milk", &shown, "buy milk and eggs");
    set_body(&mut b, &id, "buy milk", "buy milk and eggs");
    assert_eq!(echo, "please buy milk and eggs", "the echo lost A's words");
    assert_eq!(
        body_of(&b, &id),
        Some(echo),
        "the echo is not what Logic wrote"
    );

    let da = a
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            base: "cost".into(),
            text: "unit cost".into(),
        })
        .update
        .expect("A's caption broadcasts a delta");
    b.apply_remote(&da);
    let echo = merge_text("cost", &caption(&b, &c), "cost in EUR");
    b.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        base: "cost".into(),
        text: "cost in EUR".into(),
    });
    assert_eq!(echo, "unit cost in EUR", "the echo lost A's caption words");
    assert_eq!(caption(&b, &c), echo, "the echo is not what Logic wrote");
}

/// `TOILE-030`: a pasted note is born with a body two authors share, and a
/// pasted arrow keeps its caption. Both are texts on the board, which the
/// copy carries as their string.
#[test]
fn a_pasted_note_and_arrow_keep_their_text() {
    let mut st = ToileState::new();
    let note = create_sticky(&mut st, Point::ZERO, "draft");
    let other = create_rect(&mut st, Point::new(200.0, 0.0), Size::new(10.0, 10.0));
    let arrow = connect(&mut st, note.clone(), other.clone());
    st.handle_command(ToileCommand::SetConnectorLabel {
        id: arrow,
        base: String::new(),
        text: "next".to_owned(),
    });

    let copy = st.handle_command(ToileCommand::CopyElements {
        ids: vec![note, other],
    });
    let paste = st.handle_command(ToileCommand::PasteElements {
        payload: clip_of(&copy),
        at: Point::new(0.0, 400.0),
    });
    let pasted = created_ids(&paste)
        .into_iter()
        .find(|id| body_of(&st, id).is_some())
        .expect("the note is pasted");
    assert_eq!(body_of(&st, &pasted).as_deref(), Some("draft"));
    let pasted_arrow = st.connector_ids().pop().expect("the arrow is pasted");
    assert_eq!(
        caption(&st, &pasted_arrow),
        "next",
        "the copy lost its caption"
    );

    let mut peer = ToileState::new();
    peer.apply_remote_snapshot(&st.export_snapshot());
    race_two_bodies(&mut st, &mut peer, &pasted, "draft one", "a draft");
    assert_eq!(body_of(&st, &pasted).as_deref(), Some("a draft one"));
    assert_eq!(body_of(&st, &pasted), body_of(&peer, &pasted));
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
        lock: ambre_bus::app::toile::LockState::Locked,
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
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
        },
    });
    // One element node + one element node + one connector node.
    assert_eq!(st.layer_order().len(), 3);
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone()],
    });
    // The deleted element and the now-orphaned connector are both gone.
    assert_eq!(st.element_count(), ElementCount::new(1));
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
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
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
        stroke: Some(Rgba::from_bits(0xCC22_22FF)),
        stroke_w: ambre_bus::app::toile::StyleLength::new(6.0),
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

/// `TOILE-283`: the board refuses a restyle whole when it does not hold the arrow.
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

/// `TOILE-284`: a caption written on an arrow comes back on the arrow's display node.
#[test]
fn a_caption_reaches_the_viewport() {
    let mut st = ToileState::new();
    let c = create_connector(&mut st);
    assert_eq!(caption(&st, &c), "", "an arrow is born without a caption");
    let out = st.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        base: String::new(),
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
        base: String::new(),
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
    b.apply_remote_snapshot(&a.export_snapshot());

    // Both start from "cost" and edit a different end of it, unaware of each
    // other: A prepends, B appends.
    a.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        base: String::new(),
        text: "cost".into(),
    });
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            base: "cost".into(),
            text: "unit cost".into(),
        })
        .update
        .expect("A's caption broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            base: "cost".into(),
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

/// `TOILE-263`: a peer's caption edit that lands while this viewer's caption
/// editor is open survives the close. The close names the caption the editor
/// opened on, so only this viewer's own change is written over the peer's.
#[test]
fn an_open_caption_editor_keeps_a_peers_edit() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let c = create_connector(&mut a);
    a.handle_command(ToileCommand::SetConnectorLabel {
        id: c.clone(),
        base: String::new(),
        text: "cost".into(),
    });
    b.apply_remote_snapshot(&a.export_snapshot());

    // B opens its editor on "cost". A's edit lands on B while it is open.
    let da = a
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            base: "cost".into(),
            text: "unit cost".into(),
        })
        .update
        .expect("A's caption broadcasts a delta");
    b.apply_remote(&da);

    // B closes with its own typing, still based on "cost".
    let db = b
        .handle_command(ToileCommand::SetConnectorLabel {
            id: c.clone(),
            base: "cost".into(),
            text: "cost in EUR".into(),
        })
        .update
        .expect("B's caption broadcasts a delta");
    a.apply_remote(&db);

    assert_eq!(
        caption(&b, &c),
        "unit cost in EUR",
        "B's close lost A's edit"
    );
    assert_eq!(caption(&a, &c), caption(&b, &c), "the captions diverged");
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
            StrokePoint::full_pressure(
                ambre_core::StrokeCoord::new(0.0),
                ambre_core::StrokeCoord::new(0.0),
            ),
            StrokePoint::full_pressure(
                ambre_core::StrokeCoord::new(10.0),
                ambre_core::StrokeCoord::new(5.0),
            ),
        ],
        style: ElementStyle::default(),
    });
    let id = created_id(&out);
    assert!(st.bounds_of(&id).is_some());
    assert_eq!(st.element_count(), ElementCount::new(1));
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
            assert_eq!(s.count.get(), 2);
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
            assert_eq!(s.count.get(), 2);
            assert_eq!(s.bounds, Some(Rect::new(0.0, 0.0, 540.0, 320.0)));
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
}

/// `TOILE-117`: a tally of zero comes back with no rectangle, here for an empty
/// list on an empty board.
#[test]
fn selection_stats_empty_ids_on_empty_board_is_none() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids: vec![],
        kind: StatsRequestKind::Fit,
    });
    match &out.display[0] {
        ToileEvent::SelectionStats(s) => {
            assert_eq!(s.count.get(), 0);
            assert_eq!(s.bounds, None);
        }
        other => panic!("expected SelectionStats, got {other:?}"),
    }
}

/// The tally of one stats request, as `(count, bounds)`.
fn stats_of(st: &mut ToileState, ids: Vec<ElementId>) -> (u32, Option<Rect>) {
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids,
        kind: StatsRequestKind::Band,
    });
    match &out.display[..] {
        [ToileEvent::SelectionStats(s)] => (s.count.get(), s.bounds),
        other => panic!("expected one SelectionStats, got {other:?}"),
    }
}

/// `TOILE-117`: a selection whose every id the board has dropped tallies
/// zero down the per-id branch, and a zero tally carries no rectangle.
#[test]
fn a_selection_the_board_dropped_tallies_zero_with_no_rect() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(50.0, 0.0), Size::new(10.0, 10.0));
    let keep = create_rect(&mut st, Point::new(500.0, 0.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![a.clone(), b.clone()],
    });
    assert_eq!(stats_of(&mut st, vec![a, b]), (0, None));
    // The board is not empty, so the tally did not read it whole.
    assert_eq!(stats_of(&mut st, vec![keep]).0, 1);
}

/// `TOILE-118`: an id the board no longer holds does not stop the walk.
/// The tally counts the live ids around it and spans them alone.
#[test]
fn a_stale_id_does_not_stop_the_tally() {
    let mut st = ToileState::new();
    let gone = create_rect(&mut st, Point::new(-900.0, -900.0), Size::new(10.0, 10.0));
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 40.0), Size::new(20.0, 20.0));
    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![gone.clone()],
    });
    let never = ElementId("never-was".into());
    let ids = vec![gone.clone(), a, never, b, gone];
    assert_eq!(
        stats_of(&mut st, ids),
        (2, Some(Rect::new(0.0, 0.0, 120.0, 60.0)))
    );
}

/// `TOILE-UI-043`, the Logic side: the painted sequence is the board's one
/// order, elements and connectors interleaved, and a frame's children follow
/// their frame.
#[test]
fn the_painted_sequence_is_the_board_order() {
    let mut st = ToileState::new();
    let c = create_connector(&mut st);
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_rect(&mut st, Point::new(300.0, 10.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });
    let painted: Vec<NodeId> = st
        .debug_viewport()
        .nodes
        .iter()
        .map(|n| match n {
            DisplayNode::Element(e) => NodeId::Element(e.id.clone()),
            DisplayNode::Connector(k) => NodeId::Connector(k.id.clone()),
        })
        .collect();
    assert_eq!(painted, st.layer_order(), "the walk is the order");
    let at = |n: NodeId| painted.iter().position(|p| *p == n).expect("painted");
    let (conn, frame) = (at(NodeId::Connector(c)), at(NodeId::Element(f)));
    assert!(
        conn > 0 && conn < painted.len() - 1,
        "the arrow is interleaved"
    );
    assert_eq!(
        at(NodeId::Element(a)),
        frame + 1,
        "the child follows its frame"
    );
    assert!(at(NodeId::Element(b)) > frame);
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
        ToileEvent::SelectionData { elements, .. } => {
            let ids: Vec<_> = elements.iter().map(|e| e.id.clone()).collect();
            assert!(
                ids.contains(&a) && ids.contains(&b),
                "off-screen b included"
            );
        }
        other => panic!("expected SelectionData, got {other:?}"),
    }
}

/// `TOILE-UI-385`: the selection's data names the requested ids the board no
/// longer holds, a peer's deletion included, so the view can drop them.
#[test]
fn selection_data_names_the_ids_a_peer_deleted() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let kept = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let gone = create_rect(&mut a, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    b.apply_remote_snapshot(&a.export_snapshot());
    let delta = a
        .handle_command(ToileCommand::DeleteElements {
            ids: vec![gone.clone()],
        })
        .update
        .expect("the delete broadcasts a delta");
    b.apply_remote(&delta);

    let out = b.handle_command(ToileCommand::RequestSelectionData {
        ids: vec![kept.clone(), gone.clone()],
    });
    match out.display.as_slice() {
        [
            ToileEvent::SelectionData {
                elements, missing, ..
            },
        ] => {
            assert_eq!(missing, &vec![gone]);
            assert_eq!(elements.len(), 1);
            assert_eq!(elements.first().map(|e| &e.id), Some(&kept));
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

/// `TOILE-UI-033`: select-all covers an element the `layers` order omits. The
/// board holds it and the paint walk draws it, so a select-all
/// reading the order alone would be short of exactly that element.
#[test]
fn board_ids_covers_an_element_the_order_omits() {
    let mut st = ToileState::new();
    let listed = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let omitted = create_rect(&mut st, Point::new(40.0, 0.0), Size::new(10.0, 10.0));
    st.debug_remove_layer_entry(&NodeId::Element(omitted.clone()));

    let out = st.handle_command(ToileCommand::RequestBoardIds);
    match &out.display[0] {
        ToileEvent::BoardIds(ids) => {
            let mut want = vec![listed, omitted];
            want.sort();
            assert_eq!(ids, &want, "select-all takes the board, not the order");
        }
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
        lock: ambre_bus::app::toile::LockState::Locked,
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
            correlate: None,
        });
        assert!(matches!(
            out.display.first(),
            Some(ToileEvent::Error(
                crate::logic::command::ToileError::NotCreatable { .. }
            ))
        ));
        assert_eq!(st.element_count(), ElementCount::ZERO);
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
        correlate: None,
    });
    created_id(&out)
}

fn find(
    st: &mut ToileState,
    query: &str,
    options: crate::logic::command::FindOptions,
    from: Option<ElementId>,
) -> crate::logic::command::FindResults {
    find_dir(st, query, options, from, StepDirection::Forward)
}

fn find_dir(
    st: &mut ToileState,
    query: &str,
    options: crate::logic::command::FindOptions,
    from: Option<ElementId>,
    direction: StepDirection,
) -> crate::logic::command::FindResults {
    let out = st.handle_command(ToileCommand::Find {
        query: ambre_bus::app::find::FindText::new(query.to_owned())
            .map(|text| crate::logic::command::FindQuery::new(text, options)),
        from,
        direction,
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

/// `TOILE-247` and `TOILE-082`: a frame the user named is held under that name
/// and found by it. That is the field the search walk has always read and
/// nothing wrote.
#[test]
fn a_named_frame_is_found_by_its_title() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let opts = crate::logic::command::FindOptions::default();
    assert!(
        find(&mut st, "agenda", opts, None).hits.is_empty(),
        "an unnamed frame matches nothing"
    );

    let out = st.handle_command(ToileCommand::RenameFrame {
        frame: frame.clone(),
        name: "  Agenda  ".into(),
    });
    assert!(out.update.is_some(), "a name broadcasts a delta");
    assert_eq!(
        st.frame_name(&frame).unwrap_or_default(),
        "Agenda",
        "the name is trimmed"
    );
    assert_eq!(find(&mut st, "agenda", opts, None).hits, vec![frame]);
}

/// `TOILE-248`, `TOILE-249`, `TOILE-250` and `TOILE-251`: what a naming does
/// besides naming. One undo item, a bounded title, silence on a no-op, and a
/// refusal for anything that is not a frame.
#[test]
fn naming_a_frame_is_bounded_undoable_and_refused_off_a_frame() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::RenameFrame {
        frame: frame.clone(),
        name: "Agenda".into(),
    });

    // One undo item: the name goes, the frame stays.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.frame_name(&frame).unwrap_or_default(), "");
    assert_eq!(st.element_count(), ElementCount::new(1));
    st.handle_command(ToileCommand::Redo);
    assert_eq!(st.frame_name(&frame).unwrap_or_default(), "Agenda");

    // The same name again writes nothing at all.
    let out = st.handle_command(ToileCommand::RenameFrame {
        frame: frame.clone(),
        name: "Agenda".into(),
    });
    assert!(out.update.is_none() && out.display.is_empty());

    // A title no document should carry is cut to the bound.
    let long = "x".repeat(ambre_bus::app::toile::FRAME_NAME_MAX.to_usize() + 50);
    st.handle_command(ToileCommand::RenameFrame {
        frame: frame.clone(),
        name: long,
    });
    assert_eq!(
        st.frame_name(&frame).unwrap_or_default().chars().count(),
        ambre_bus::app::toile::FRAME_NAME_MAX.to_usize()
    );

    // Anything that is not a frame is refused, and keeps its own body.
    let sticky = create_sticky(&mut st, Point::new(400.0, 0.0), "note");
    let out = st.handle_command(ToileCommand::RenameFrame {
        frame: sticky.clone(),
        name: "Agenda".into(),
    });
    assert!(out.update.is_none(), "the refusal wrote to the document");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::ContentKindMismatch))),
        "the refusal said nothing: {:?}",
        out.display
    );
    assert_eq!(st.frame_name(&sticky), None, "a sticky carries no title");
}

/// `TOILE-252`: two users naming one frame at once converge on one writer's
/// whole name. A character-wise text would interleave them into a third title
/// nobody typed.
#[test]
fn concurrent_names_converge_on_one_whole_name() {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::RenameFrame {
            frame: frame.clone(),
            name: "Agenda".into(),
        })
        .update
        .expect("a delta");
    let db = b
        .handle_command(ToileCommand::RenameFrame {
            frame: frame.clone(),
            name: "Roadmap".into(),
        })
        .update
        .expect("a delta");
    b.apply_remote(&da);
    a.apply_remote(&db);

    let merged = a.frame_name(&frame).unwrap_or_default();
    assert_eq!(
        merged,
        b.frame_name(&frame).unwrap_or_default(),
        "the two names diverged"
    );
    assert!(
        merged == "Agenda" || merged == "Roadmap",
        "a blend of the two: {merged:?}"
    );
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

/// `FIND-015`: an edit took the resume point out of the hits, so it is no
/// longer usable. The step elects the first hit, not the one after it.
#[test]
fn a_resume_point_edited_away_restarts_at_the_first_hit() {
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "match");
    let b = create_sticky(&mut st, Point::new(200.0, 0.0), "match");
    let c = create_sticky(&mut st, Point::new(400.0, 0.0), "match");
    let opts = crate::logic::command::FindOptions::default();
    assert_eq!(
        find(&mut st, "match", opts, Some(b.clone())).focus,
        Some(c.clone())
    );

    set_body(&mut st, &b, "match", "gone");
    for direction in [StepDirection::Forward, StepDirection::Backward] {
        let r = find_dir(&mut st, "match", opts, Some(b.clone()), direction);
        assert_eq!(r.hits, vec![a.clone(), c.clone()]);
        assert_eq!(r.focus, Some(a.clone()), "direction: {direction:?}");
    }
}

/// `FIND-002`: the walk covers the whole board, not the camera. A hit far
/// outside the visible set is found and elected.
#[test]
fn a_search_finds_a_hit_far_outside_the_camera() {
    let mut st = ToileState::new();
    let near = create_sticky(&mut st, Point::ZERO, "needle");
    let far = create_sticky(&mut st, Point::new(1.0e6, 1.0e6), "needle");
    st.handle_command(ToileCommand::SetViewport(crate::logic::index::Viewport {
        center: Point::ZERO,
        zoom: ambre_core::ZoomFactor::positive(1.0),
        size: Size::new(800.0, 600.0),
    }));
    let visible: Vec<ElementId> = st
        .debug_viewport()
        .nodes
        .into_iter()
        .filter_map(|node| match node {
            DisplayNode::Element(e) => Some(e.id),
            DisplayNode::Connector(_) => None,
        })
        .collect();
    assert_eq!(visible, vec![near.clone()], "the far note is off camera");

    let opts = crate::logic::command::FindOptions::default();
    let r = find(&mut st, "needle", opts, Some(near.clone()));
    assert_eq!(r.hits, vec![near, far.clone()]);
    assert_eq!(r.focus, Some(far));
}

#[test]
fn find_backward_steps_to_previous_hit_wrapping() {
    let mut st = ToileState::new();
    let a = create_sticky(&mut st, Point::ZERO, "match");
    let b = create_sticky(&mut st, Point::new(200.0, 0.0), "match");

    let opts = crate::logic::command::FindOptions::default();
    // Backward from the second hit focuses the first.
    let r = find_dir(
        &mut st,
        "match",
        opts,
        Some(b.clone()),
        StepDirection::Backward,
    );
    assert_eq!(r.focus, Some(a.clone()));
    // Backward from the first hit wraps to the last.
    let r = find_dir(&mut st, "match", opts, Some(a), StepDirection::Backward);
    assert_eq!(r.focus, Some(b));
}

#[test]
fn find_honors_match_case_and_whole_word() {
    let mut st = ToileState::new();
    let exact = create_sticky(&mut st, Point::ZERO, "Cat");
    let _lower = create_sticky(&mut st, Point::new(200.0, 0.0), "cat");
    let substr = create_sticky(&mut st, Point::new(400.0, 0.0), "Cathedral");

    let case = crate::logic::command::FindOptions {
        case: ambre_core::CaseMatch::Sensitive,
        word: ambre_core::WordMatch::Anywhere,
    };
    let r = find(&mut st, "Cat", case, None);
    // Case-sensitive: "Cat" and "Cathedral" (substring) match, "cat" does not.
    assert_eq!(r.hits, vec![exact.clone(), substr]);

    let whole = crate::logic::command::FindOptions {
        case: ambre_core::CaseMatch::Sensitive,
        word: ambre_core::WordMatch::WholeWord,
    };
    let r = find(&mut st, "Cat", whole, None);
    // Whole-word + case: only the standalone "Cat" survives.
    assert_eq!(r.hits, vec![exact]);
}

/// `TOILE-080`: an element the paint order omits is still found. It comes
/// after the ordered hits, and the omitted ones come in identity order.
#[test]
fn find_reaches_an_element_the_order_omits() {
    let mut st = ToileState::new();
    let listed = create_sticky(&mut st, Point::ZERO, "plan");
    let mut omitted: Vec<ElementId> = (1..=3)
        .map(|i| create_sticky(&mut st, Point::new(f64::from(i) * 100.0, 0.0), "plan"))
        .collect();
    for id in &omitted {
        st.debug_remove_layer_entry(&NodeId::Element(id.clone()));
    }
    let r = find(&mut st, "plan", Default::default(), None);
    omitted.sort();
    let mut want = vec![listed];
    want.extend(omitted);
    assert_eq!(r.hits, want);
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

/// Broadcast `payload` from `st` at `now` and return the ephemeral.
fn presence_frame(
    st: &mut ToileState,
    payload: PresencePayload,
    now: ambre_core::PageTime,
) -> ambre_core::EphemeralBytes {
    st.handle_command(ToileCommand::Presence { payload, now })
        .ephemeral
        .expect("presence broadcasts an ephemeral")
}

/// `TOILE-286`: the board holds no user id until the session seeds one, and
/// that id afterwards.
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
fn a_stroke_in_flight_reaches_the_other_users() {
    // `TOILE-106`: each sample of a stroke being drawn rides presence, so the
    // other boards see the pen move instead of waiting for the commit. The
    // board it rides is never touched: an in-flight stroke is not document
    // state until release (`TOILE-105`).
    let mut drawer = ToileState::new();
    let tail = vec![
        StrokePoint {
            x: ambre_core::StrokeCoord::new(10.0),
            y: ambre_core::StrokeCoord::new(10.0),
            pressure: ambre_bus::app::toile::UnitFraction::HALF,
        },
        StrokePoint {
            x: ambre_core::StrokeCoord::new(12.0),
            y: ambre_core::StrokeCoord::new(14.0),
            pressure: ambre_bus::app::toile::UnitFraction::new(0.6)
                .expect("a pressure inside 0 ..= 1"),
        },
    ];
    let out = drawer.handle_command(ToileCommand::Presence {
        payload: PresencePayload {
            name: None,
            cursor: Some(Point::new(12.0, 14.0)),
            selection: Vec::new(),
            live_stroke: Some(tail.clone()),
            laser: None,
            present_slide: None,
        },
        now: ambre_core::PageTime(1_000.0),
    });
    assert!(out.update.is_none(), "a stroke in flight writes nothing");
    let frame = out.ephemeral.expect("the sample broadcasts");

    let mut watcher = ToileState::new();
    let drawer_id = UserId::new();
    watcher.apply_ephemeral(drawer_id, &frame);
    assert_eq!(
        watcher
            .peer_presence(&drawer_id)
            .and_then(|p| p.live_stroke.clone()),
        Some(tail),
        "the watcher sees no pen"
    );
    assert_eq!(
        watcher.element_count(),
        ElementCount::ZERO,
        "nothing landed on the board"
    );
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
        now: ambre_core::PageTime(1_000.0),
    });
    assert!(out.display.is_empty(), "presence emits no display event");
    assert!(out.update.is_none(), "presence is never a CRDT delta");
    assert!(
        out.ephemeral.is_some(),
        "presence must broadcast an ephemeral"
    );
    // `TOILE-183`: the sample pushed no undo item. The first undo takes back
    // the sticky, the last edit before it.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.element_count(),
        ElementCount::ZERO,
        "the first undo took the sticky back"
    );
}

#[test]
fn the_outbound_sample_carries_the_session_display_name() {
    // TOILE-UI-076: the name comes from the session this thread already holds,
    // so a collaborator reads a name and not a hex prefix. It is cosmetic
    // (`PRESENCE-004`): it travels beside the sample and decides nothing.
    let mut sender = ToileState::new();
    sender.set_display_name(ambre_core::DisplayName::new("Ada").unwrap());
    let frame = presence_frame(&mut sender, sample_at(1.0, 2.0), ambre_core::PageTime(0.0));

    let mut rx = ToileState::new();
    let peers = live_set(rx.apply_ephemeral(UserId::new(), &frame));
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].presence.name.as_ref().unwrap().as_str(), "Ada");

    // A session with no name yet sends none, and the receiver falls back to the
    // short handle rather than paint an empty chip.
    let mut anonymous = ToileState::new();
    let frame = presence_frame(
        &mut anonymous,
        sample_at(3.0, 4.0),
        ambre_core::PageTime(0.0),
    );
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
    let frame = presence_frame(&mut sender, sample_at(5.0, 5.0), ambre_core::PageTime(0.0));

    let mut rx = ToileState::new();
    // The receiver's clock starts at the first local sample.
    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now: ambre_core::PageTime(1_000.0),
    });
    let alice = UserId::new();
    assert_eq!(live_set(rx.apply_ephemeral(alice, &frame)).len(), 1);

    // Still inside the TTL: Alice is live, and the local sample says nothing new.
    let out = rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now: ambre_core::PageTime(1_000.0 + PRESENCE_TTL.get() - 1.0),
    });
    assert!(out.display.is_empty(), "nothing left the set yet");
    assert!(rx.peer_presence(&alice).is_some());

    // Past it: the set changes, and the event that announces it is empty.
    let out = rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now: ambre_core::PageTime(1_000.0 + PRESENCE_TTL.get() + 1.0),
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
    let frame = presence_frame(&mut sender, sample_at(3.0, 3.0), ambre_core::PageTime(0.0));

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

/// `TOILE-287`: a local clock that steps backwards ages no peer out of the
/// live set.
#[test]
fn a_clock_that_steps_backwards_ages_nobody() {
    // The local clock is monotonic: an NTP correction or a suspended tab must
    // not empty the board of every peer at once.
    let mut sender = ToileState::new();
    let frame = presence_frame(&mut sender, sample_at(1.0, 1.0), ambre_core::PageTime(0.0));

    let mut rx = ToileState::new();
    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now: ambre_core::PageTime(100_000.0),
    });
    let alice = UserId::new();
    rx.apply_ephemeral(alice, &frame);

    rx.handle_command(ToileCommand::Presence {
        payload: PresencePayload::default(),
        now: ambre_core::PageTime(0.0),
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
    let first = presence_frame(&mut sender, with_selection, ambre_core::PageTime(0.0));
    let second = presence_frame(&mut sender, sample_at(9.0, 9.0), ambre_core::PageTime(0.0));

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
    // #2 / DoD: an inbound presence sample yields a PeerPresence keyed by the
    // resolved UserId and mutates neither the doc nor its history.
    let mut sender = ToileState::new();
    let payload = PresencePayload {
        name: None,
        cursor: Some(Point::new(7.0, 8.0)),
        selection: Vec::new(),
        live_stroke: Some(vec![StrokePoint::full_pressure(
            ambre_core::StrokeCoord::new(1.0),
            ambre_core::StrokeCoord::new(2.0),
        )]),
        laser: None,
        present_slide: None,
    };
    let frame = presence_frame(&mut sender, payload.clone(), ambre_core::PageTime(0.0));

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
    let bob_frame = presence_frame(
        &mut sender,
        PresencePayload::default(),
        ambre_core::PageTime(0.0),
    );
    let peers = live_set(rx.apply_ephemeral(bob, &bob_frame));
    assert_eq!(peers.len(), 2, "the event carries the whole live set");
    assert!(rx.peer_presence(&alice).is_some());
    assert_eq!(rx.peer_presence(&bob), Some(&PresencePayload::default()));
}

#[test]
fn apply_ephemeral_drops_a_malformed_sample() {
    let mut rx = ToileState::new();
    let events = rx.apply_ephemeral(UserId::new(), &vec![0xFF, 0x00, 0x13, 0x37].into());
    assert!(
        events.is_empty(),
        "a malformed presence sample yields no event"
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
    reader.apply_remote_snapshot(&snapshot);
    let out = reader.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(50.0, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    let delta = out.update.expect("a create must produce a delta");

    // A peer holding nothing sees nothing in this delta: the author's element
    // is not in it. Its own create is, but it depends on the reader's genesis
    // operations, which this peer does not hold, so Loro keeps it pending.
    let mut naive = ToileState::new();
    naive.apply_remote(&delta);
    assert_eq!(
        naive.element_count(),
        ElementCount::ZERO,
        "the delta must carry no operation the reader merely imported"
    );

    // The author, who holds everything the delta depends on, gets the new one.
    let mut third = ToileState::new();
    third.apply_remote_snapshot(&snapshot);
    third.apply_remote(&delta);
    assert_eq!(third.element_count(), ElementCount::new(2));
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
    reader.apply_remote_snapshot(&snapshot);
    let out = reader.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(50.0, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    let delta = out.update.expect("a create must produce a delta");

    // The author receives it. Without the genesis operations the create depends
    // on, Loro would hold it pending and the element would never appear.
    author.apply_remote(&delta);
    assert_eq!(
        author.element_count(),
        ElementCount::new(2),
        "the delta must carry everything it depends on"
    );
}

/// `TOILE-245`: a local restore leaves the cursor alone. What this client wrote
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
    assert_eq!(reopened.element_count(), ElementCount::new(1));

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
        ElementCount::new(1),
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

/// Holds `TOILE-155` and `TOILE-150`: a resize aimed at a group writes nothing.
/// A rotation leaves its box derived, even with a member a peer deleted.
#[test]
fn a_group_takes_no_box_from_a_resize_or_a_rotation() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    a.set_user_id(UserId::new());
    b.set_user_id(UserId::new());
    let x = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let y = create_rect(&mut a, Point::new(90.0, 90.0), Size::new(10.0, 10.0));
    b.apply_remote_snapshot(&a.export_snapshot());
    // A binds both while B deletes one: the merged group names a dead member.
    let out = a.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), y],
    });
    let g = created_id(&out);
    let deleted = b.handle_command(ToileCommand::DeleteElements { ids: vec![x] });
    a.apply_remote(&deleted.update.expect("the delete is a change"));
    b.apply_remote(&out.update.expect("the binding is a change"));

    let resized = a.handle_command(ToileCommand::ResizeElement {
        id: g.clone(),
        new: Rect::new(0.0, 0.0, 60.0, 50.0),
    });
    assert!(resized.update.is_none(), "a group resize wrote a change");
    let rotated = a.handle_command(ToileCommand::RotateElement {
        id: g.clone(),
        radians: ambre_core::Radians::new(0.5),
    });
    b.apply_remote(&rotated.update.expect("the rotation is a change"));
    let mut rebuilt = ToileState::new();
    rebuilt.apply_remote_snapshot(&a.export_snapshot());
    assert_eq!(a.bounds_of(&g), None, "a group took a leaf box");
    assert_eq!(a.debug_caches(), b.debug_caches());
    assert_eq!(a.debug_caches(), rebuilt.debug_caches());
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

/// `TOILE-266`: two peers bind one element into two groups at once. Each
/// peer learns the two parents in its own order, and both name the same one.
#[test]
fn an_element_bound_into_two_groups_names_one_on_both_peers() {
    let mut a = ToileState::with_peer(ambre_core::PeerId(1));
    let x = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let y = create_rect(&mut a, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let z = create_rect(&mut a, Point::new(40.0, 0.0), Size::new(10.0, 10.0));
    let mut b = ToileState::with_peer(ambre_core::PeerId(2));
    b.import_snapshot(&a.export_snapshot())
        .expect("a snapshot this build wrote reads back at the current schema");

    a.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), y],
    });
    b.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), z],
    });
    let (sa, sb) = (a.export_snapshot(), b.export_snapshot());
    a.apply_remote_snapshot(&sb);
    b.apply_remote_snapshot(&sa);

    let group_of = |st: &ToileState| {
        st.debug_viewport().nodes.iter().find_map(|n| match n {
            DisplayNode::Element(e) if e.id == x => Some(e.group.clone()),
            _ => None,
        })
    };
    let on_a = group_of(&a);
    assert!(matches!(on_a, Some(Some(_))), "x sits in a group: {on_a:?}");
    assert_eq!(on_a, group_of(&b));
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
    st.handle_command(ToileCommand::UngroupElements {
        groups: vec![g.clone()],
    });
    // The group node is gone; both children survive in their prior order.
    assert!(st.bounds_of(&g).is_none());
    assert_eq!(st.layer_order(), &[NodeId::Element(a), NodeId::Element(b)]);
}

/// `TOILE-094`: ungrouping a multi-selection is one gesture, so one undo
/// restores every group it unbound.
#[test]
fn ungrouping_three_groups_is_one_undo_item() {
    let mut st = ToileState::new();
    let mut groups = Vec::new();
    for i in 0..3 {
        let x = f64::from(i) * 100.0;
        let a = create_rect(&mut st, Point::new(x, 0.0), Size::new(10.0, 10.0));
        let b = create_rect(&mut st, Point::new(x + 20.0, 0.0), Size::new(10.0, 10.0));
        groups.push(created_id(
            &st.handle_command(ToileCommand::GroupElements { ids: vec![a, b] }),
        ));
    }
    st.handle_command(ToileCommand::UngroupElements {
        groups: groups.clone(),
    });
    assert!(groups.iter().all(|g| st.bounds_of(g).is_none()));
    st.handle_command(ToileCommand::Undo);
    for g in &groups {
        assert_eq!(
            st.membership_of(g).len(),
            2,
            "one undo restores every group"
        );
    }
}

/// `TOILE-UI-175`: a restyle over a whole selection is one gesture, so one
/// undo restores the style of every element it named.
#[test]
fn a_restyle_over_a_selection_is_one_undo_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let before = (st.style_of(&a), st.style_of(&b));
    let style = ElementStyle {
        fill: Some(Rgba::from_bits(0x1122_33ff)),
        ..ElementStyle::default()
    };
    st.handle_command(ToileCommand::RestyleElements {
        ids: vec![a.clone(), b.clone()],
        style: style.clone(),
    });
    assert_eq!((st.style_of(&a), st.style_of(&b)), (style.clone(), style));
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        (st.style_of(&a), st.style_of(&b)),
        before,
        "one undo restores both"
    );
}

/// `TOILE-264`: a list holding a group unbinds it and answers nothing for the
/// ids that are not groups; a list holding none answers one error.
#[test]
fn a_mixed_ungroup_answers_nothing_for_its_non_groups() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let c = create_rect(&mut st, Point::new(40.0, 0.0), Size::new(10.0, 10.0));
    let d = create_rect(&mut st, Point::new(60.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements { ids: vec![a, b] }));
    let out = st.handle_command(ToileCommand::UngroupElements {
        groups: vec![c.clone(), g.clone(), d.clone()],
    });
    assert!(st.bounds_of(&g).is_none(), "the group unbinds");
    assert!(
        !out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(_))),
        "no error for the non-groups beside it"
    );
    let out = st.handle_command(ToileCommand::UngroupElements { groups: vec![c, d] });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NotAGroup(_))]
    ));
}

/// `TOILE-094`: a group and its own enclosing group ungrouped in one gesture
/// leave both members on the board, whichever the selection lists first.
#[test]
fn ungrouping_a_group_with_its_parent_unbinds_both() {
    for inner_first in [false, true] {
        let mut st = ToileState::new();
        let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
        let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
        let c = create_rect(&mut st, Point::new(40.0, 0.0), Size::new(10.0, 10.0));
        let inner = created_id(&st.handle_command(ToileCommand::GroupElements {
            ids: vec![a.clone(), b.clone()],
        }));
        let outer = created_id(&st.handle_command(ToileCommand::GroupElements {
            ids: vec![inner.clone(), c.clone()],
        }));
        let groups = if inner_first {
            vec![inner.clone(), outer.clone()]
        } else {
            vec![outer.clone(), inner.clone()]
        };
        st.handle_command(ToileCommand::UngroupElements { groups });
        assert!(st.bounds_of(&inner).is_none() && st.bounds_of(&outer).is_none());
        assert_eq!(
            st.layer_order(),
            &[NodeId::Element(a), NodeId::Element(b), NodeId::Element(c)]
        );
    }
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
    assert_eq!(st.element_count(), ElementCount::ZERO);
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
        correlate: None,
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

/// `TOILE-066`: setting a link and clearing it are one undo item each. One
/// undo restores the link, a second takes it away, and the element stays.
#[test]
fn each_link_write_is_one_undo_item() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let target = LinkTarget::Url("https://example.org/".into());
    st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: Some(target.clone()),
    });
    st.handle_command(ToileCommand::SetLink {
        id: id.clone(),
        link: None,
    });

    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        link_of(&st, &id),
        Some(target),
        "one undo restores the link"
    );
    st.handle_command(ToileCommand::Undo);
    assert_eq!(link_of(&st, &id), None, "a second undo takes it away");
    assert_eq!(
        st.element_count(),
        ElementCount::new(1),
        "the element is not undone"
    );
    st.handle_command(ToileCommand::Redo);
    assert!(link_of(&st, &id).is_some(), "a redo sets it again");
}

/// `TOILE-115`: the one board command a jump sends commits nothing, so the
/// undo after it reverts the edit made before it.
#[test]
fn a_jump_is_not_an_undo_item() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let out = st.handle_command(ToileCommand::RequestSelectionStats {
        ids: vec![frame],
        kind: StatsRequestKind::Fit,
    });
    assert!(out.update.is_none(), "the jump writes nothing");
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.element_count(),
        ElementCount::ZERO,
        "the undo takes back the frame"
    );
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

/// `TOILE-288`: the board refuses a link whole when it does not hold the
/// target element.
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

/// `TOILE-UI-149`: the removal takes an element out of the frame whose list
/// holds it, not out of the frame whose box contains it. Here the box sits in
/// `near`, and the member list is `far`'s.
#[test]
fn remove_from_frames_targets_the_real_parent() {
    let mut st = ToileState::new();
    let near = create_frame(&mut st, Point::ZERO, Size::new(100.0, 100.0));
    let far = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(100.0, 100.0));
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: far.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });

    let out = st.handle_command(ToileCommand::RemoveFromFrames {
        ids: vec![a.clone()],
    });
    assert!(out.update.is_some(), "the removal broadcasts a delta");
    assert!(
        st.membership_of(&far).is_empty(),
        "a stays in its own frame"
    );
    assert!(st.membership_of(&near).is_empty());

    // An element no frame holds writes nothing.
    let out = st.handle_command(ToileCommand::RemoveFromFrames { ids: vec![a] });
    assert!(out.update.is_none(), "nothing held, nothing written");
}

/// The ids a `SelectionData` reply names as framed.
fn framed_of(st: &mut ToileState, ids: Vec<ElementId>) -> Vec<ElementId> {
    let out = st.handle_command(ToileCommand::RequestSelectionData { ids });
    match out.display.as_slice() {
        [ToileEvent::SelectionData { framed, .. }] => framed.clone(),
        other => panic!("expected SelectionData, got {other:?}"),
    }
}

/// `TOILE-UI-384`: the selection's data names an element its frame holds,
/// though the element sits outside every frame's box. A write and a peer's
/// delta both announce a change, so the view asks again and sees it leave.
#[test]
fn selection_data_names_a_member_outside_its_frame() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(100.0, 100.0));
    let a = create_rect(&mut st, Point::new(500.0, 500.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(600.0, 500.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });
    assert_eq!(
        framed_of(&mut st, vec![a.clone(), b.clone()]),
        vec![a.clone()]
    );

    let mut peer = ToileState::new();
    let remote = peer.apply_remote_snapshot(&st.export_snapshot());
    assert!(
        remote.iter().any(|e| matches!(e, ToileEvent::BoardChanged)),
        "a peer's delta announces no change"
    );

    let out = st.handle_command(ToileCommand::RemoveFromFrames {
        ids: vec![a.clone()],
    });
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::BoardChanged)),
        "a write announces no change"
    );
    assert!(framed_of(&mut st, vec![a, b]).is_empty());

    let moved = st.handle_command(ToileCommand::SetViewport(
        crate::logic::index::Viewport::default(),
    ));
    assert!(
        !moved
            .display
            .iter()
            .any(|e| matches!(e, ToileEvent::BoardChanged)),
        "a camera move announced a change"
    );
}

/// `TOILE-009`: a member a frame takes from its group leaves that group's box.
/// The detach once kept the old union until the next rebuild.
#[test]
fn a_member_framed_out_of_its_group_shrinks_the_group() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(5.0, 5.0));
    let a = create_rect(&mut st, Point::new(0.0, 0.0), Size::new(5.0, 5.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(5.0, 5.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f,
        add: vec![b],
        remove: vec![],
    });
    assert_eq!(st.bounds_of(&g), st.bounds_of(&a));
}

/// `TOILE-056`: reading which group or frame an element belongs to walks no
/// board. The selection's data names each member's group and frame, and a
/// camera move derives the visible set, without resolving the element map once.
#[test]
fn a_parent_read_walks_no_board() {
    let mut st = ToileState::new();
    for i in 0..20 {
        create_rect(
            &mut st,
            Point::new(f64::from(i) * 30.0, 0.0),
            Size::new(10.0, 10.0),
        );
    }
    let a = create_rect(&mut st, Point::new(500.0, 500.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(600.0, 500.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    let f = create_frame(&mut st, Point::new(-500.0, -500.0), Size::new(100.0, 100.0));
    let c = create_rect(&mut st, Point::new(700.0, 500.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f,
        add: vec![c.clone()],
        remove: vec![],
    });

    let before = st.board_walks();
    let out = st.handle_command(ToileCommand::RequestSelectionData {
        ids: vec![a, b, c.clone()],
    });
    st.handle_command(ToileCommand::SetViewport(wide_viewport()));
    assert_eq!(st.board_walks(), before, "a parent read walked the board");
    match out.display.as_slice() {
        [
            ToileEvent::SelectionData {
                elements, framed, ..
            },
        ] => {
            let groups: Vec<_> = elements.iter().map(|e| e.group.clone()).collect();
            assert_eq!(groups, vec![Some(g.clone()), Some(g), None]);
            assert_eq!(framed, &vec![c]);
        }
        other => panic!("expected SelectionData, got {other:?}"),
    }
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

/// `TOILE-048`: a child drawn before its frame is clamped above it, and one
/// undo reverts the membership and the clamp together.
#[test]
fn a_clamped_membership_change_is_one_undo_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(10.0, 10.0));
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let before = st.layer_order().to_vec();
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone()],
        remove: vec![],
    });
    assert_eq!(
        st.layer_order(),
        &[NodeId::Element(f.clone()), NodeId::Element(a.clone())],
        "the clamp lifts the child above its frame"
    );
    st.handle_command(ToileCommand::Undo);
    assert!(st.membership_of(&f).is_empty());
    assert_eq!(
        st.layer_order(),
        before.as_slice(),
        "the clamp is undone too"
    );
}

// -----------------------------------------------------------------------------
// Smart Animate snapshot derivation (`toile-presentation`)
// -----------------------------------------------------------------------------

/// `TOILE-FEAT-110`: a slide's snapshot holds the frame's declared members with
/// their box, rotation, opacity and fill.
#[test]
fn frame_snapshot_captures_declared_children_props() {
    use crate::logic::animate::smart_animate;

    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(10.0, 20.0), Size::new(30.0, 40.0));
    let b = create_rect(&mut st, Point::new(60.0, 10.0), Size::new(10.0, 10.0));

    // Give `a` a distinctive style + rotation so the snapshot picks them up.
    let style = ElementStyle {
        fill: Some(Rgba::from_bits(0x1234_56FF)),
        opacity: ambre_core::Opacity::new(0.5).unwrap(),
        ..ElementStyle::default()
    };
    st.handle_command(ToileCommand::RestyleElements {
        ids: vec![a.clone()],
        style,
    });
    st.handle_command(ToileCommand::RotateElement {
        id: a.clone(),
        radians: ambre_core::Radians::new(1.0),
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
    assert_eq!(sa.rotation, ambre_core::Radians::new(1.0));
    assert_eq!(sa.opacity.get(), 0.5);
    assert_eq!(sa.fill, Some(Rgba::from_bits(0x1234_56FF)));

    // A frame with no children yields an empty snapshot; a transition between an
    // empty and the populated frame fades every child in (endpoint sanity).
    let empty = st.frame_snapshot(&b); // b is a plain rect, no children
    assert!(empty.elements.is_empty());
    let anim = smart_animate(&empty, &snap, crate::logic::animate::Phase::END);
    assert_eq!(anim.len(), 2);
    assert!(
        anim.iter()
            .any(|e| e.id == a && (e.opacity.get() - 0.5).abs() < 1e-6)
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
    assert_eq!(st.element_count(), ElementCount::new(1));
    let payload = clip_of(&copy);
    assert_eq!(payload.origin, Rect::new(10.0, 10.0, 30.0, 30.0));

    // Paste so `origin` (top-left 10,10) lands at (110, 210): +100, +200.
    let paste = st.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(110.0, 210.0),
    });
    assert!(paste.update.is_some(), "paste must broadcast a CRDT delta");
    assert_eq!(st.element_count(), ElementCount::new(2));
    let pasted = created_ids(&paste);
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], a, "pasted element must get a fresh id");
    assert_eq!(
        st.bounds_of(&pasted[0]),
        Some(Rect::new(110.0, 210.0, 130.0, 230.0))
    );
}

/// `TOILE-137` and `TOILE-138`: a pasted frame is a frame. It carries its
/// speaker notes as the shared text they are born as, never as a scalar
/// field. It takes an element like any other frame.
#[test]
fn a_pasted_frame_carries_its_notes_and_takes_an_element() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::EditNotes {
        frame: frame.clone(),
        base: String::new(),
        delta: TextDelta {
            at: CharOffset::ZERO,
            delete: CharCount::ZERO,
            insert: "open on the budget".into(),
        },
    });

    // Copied alone: no member travels with it.
    let copy = st.handle_command(ToileCommand::CopyElements {
        ids: vec![frame.clone()],
    });
    let paste = st.handle_command(ToileCommand::PasteElements {
        payload: clip_of(&copy),
        at: Point::new(500.0, 500.0),
    });
    let pasted = created_ids(&paste);
    assert_eq!(pasted.len(), 1);
    assert_eq!(
        st.notes(&pasted[0]).unwrap_or_default(),
        "open on the budget",
        "the copy lost its notes"
    );

    // A peer's first edit of the copy's notes converges rather than minting a
    // rival container, which is what a scalar field would have produced.
    let mut peer = ToileState::new();
    peer.apply_remote_snapshot(&st.export_snapshot());
    let da = st
        .handle_command(ToileCommand::EditNotes {
            frame: pasted[0].clone(),
            base: "open on the budget".into(),
            delta: TextDelta {
                at: CharOffset::ZERO,
                delete: CharCount::ZERO,
                insert: "A: ".into(),
            },
        })
        .update
        .expect("a delta");
    let db = peer
        .handle_command(ToileCommand::EditNotes {
            frame: pasted[0].clone(),
            base: "open on the budget".into(),
            delta: TextDelta {
                at: CharOffset::new(18),
                delete: CharCount::ZERO,
                insert: " (B)".into(),
            },
        })
        .update
        .expect("a delta");
    peer.apply_remote(&da);
    st.apply_remote(&db);
    let merged = st.notes(&pasted[0]).unwrap_or_default();
    assert_eq!(
        merged,
        peer.notes(&pasted[0]).unwrap_or_default(),
        "the notes diverged"
    );
    assert!(
        merged.contains("A: ") && merged.contains("(B)"),
        "a rival container dropped one side: {merged:?}"
    );

    // And the copy takes an element, as any frame can.
    let rect = create_rect(&mut st, Point::new(510.0, 510.0), Size::new(10.0, 10.0));
    let out = st.handle_command(ToileCommand::SetFrameChildren {
        frame: pasted[0].clone(),
        add: vec![rect.clone()],
        remove: Vec::new(),
    });
    assert!(
        out.update.is_some(),
        "the add was refused: {:?}",
        out.display
    );
}

/// `TOILE-165`: an arrow goes dangling when a peer deletes its endpoint
/// concurrently. The deleting peer never saw the arrow, so nothing cleaned it
/// up, and it stops being painted on every board. The announcement is what a
/// surface has to say the document still holds it.
#[test]
fn an_arrow_stranded_by_a_peer_is_announced() {
    let mut a = ToileState::new();
    let x = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let y = create_rect(&mut a, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());

    // Concurrent: B joins the two, A deletes one of them.
    let db = b
        .handle_command(ToileCommand::CreateConnector {
            from: x.clone(),
            from_anch: Anchor::Right,
            to: y.clone(),
            to_anch: Anchor::Left,
            path_type: PathType::Straight,
            arrows: EndCaps {
                start: ambre_bus::app::toile::Cap::Plain,
                end: ambre_bus::app::toile::Cap::Arrow,
            },
        })
        .update
        .expect("the arrow broadcasts a delta");
    let da = a
        .handle_command(ToileCommand::DeleteElements { ids: vec![y] })
        .update
        .expect("the delete broadcasts a delta");

    let events = b.apply_remote(&da);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ToileEvent::DanglingConnector(_)))
            .count(),
        1,
        "the stranded arrow was not announced: {events:?}"
    );

    // The peer that deleted sees it too, once the arrow reaches it.
    assert_eq!(
        a.apply_remote(&db)
            .iter()
            .filter(|e| matches!(e, ToileEvent::DanglingConnector(_)))
            .count(),
        1
    );

    // A later delta that strands nothing announces nothing, so the notice does
    // not repeat for as long as the arrow stays dangling.
    let da = a
        .handle_command(ToileCommand::MoveElements {
            ids: vec![x],
            delta: Vec2::new(5.0, 5.0),
        })
        .update
        .expect("a delta");
    assert!(
        !b.apply_remote(&da)
            .iter()
            .any(|e| matches!(e, ToileEvent::DanglingConnector(_)))
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
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
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

/// The elements a copy and a paste of `ids` create, in paint order.
fn copy_paste(st: &mut ToileState, ids: Vec<ElementId>) -> Vec<ElementId> {
    let payload = clip_of(&st.handle_command(ToileCommand::CopyElements { ids }));
    created_ids(&st.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(1000.0, 1000.0),
    }))
}

/// `TOILE-132`: a frame copied alone lands empty, and its members stay with
/// the source. A frame copied with a member binds the member's copy.
#[test]
fn a_copied_frame_carries_no_member_it_was_not_copied_with() {
    let mut st = ToileState::new();
    let frame = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let inside = create_rect(&mut st, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![inside.clone()],
        remove: Vec::new(),
    });
    let alone = copy_paste(&mut st, vec![frame.clone()]);
    assert_eq!(alone.len(), 1, "no member travels with the frame");
    assert!(st.membership_of(&alone[0]).is_empty());
    assert_eq!(st.membership_of(&frame), std::slice::from_ref(&inside));

    let both = copy_paste(&mut st, vec![frame, inside]);
    let [new_frame, new_inside] = &both[..] else {
        panic!("the frame and its member: {both:?}");
    };
    assert_eq!(
        st.membership_of(new_frame),
        std::slice::from_ref(new_inside)
    );
}

/// `TOILE-071`: a pasted group binds its own copies, and the source group
/// keeps its own members.
#[test]
fn a_pasted_group_binds_its_own_copies() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    }));
    let pasted = copy_paste(&mut st, vec![g.clone()]);
    assert_eq!(pasted.len(), 3);
    let groups: Vec<&ElementId> = pasted
        .iter()
        .filter(|id| !st.membership_of(id).is_empty())
        .collect();
    let [group] = groups[..] else {
        panic!("one pasted group: {groups:?}");
    };
    let mut members = st.membership_of(group);
    members.sort();
    let mut copies: Vec<ElementId> = pasted.iter().filter(|id| *id != group).cloned().collect();
    copies.sort();
    assert_eq!(members, copies, "bound to the copies");
    assert_eq!(st.membership_of(&g), [a, b], "the source keeps its members");
}

/// `TOILE-136`: a pasted arrow joins the copies of its two ends, never the
/// elements that were copied.
#[test]
fn a_pasted_arrow_joins_the_copies() {
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
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
        },
    });
    let before = st.connector_ids();
    let pasted = copy_paste(&mut st, vec![a, b]);
    let fresh: Vec<ConnectorId> = st
        .connector_ids()
        .into_iter()
        .filter(|c| !before.contains(c))
        .collect();
    let [arrow] = &fresh[..] else {
        panic!("one pasted arrow: {fresh:?}");
    };
    let joined = st.connector_of(arrow).expect("the arrow is stored");
    assert_eq!(
        [joined.from, joined.to],
        [pasted[0].clone(), pasted[1].clone()]
    );
}

/// `TOILE-140`: a paste of an empty payload writes nothing and reaches
/// nobody.
#[test]
fn an_empty_paste_writes_nothing() {
    let mut st = ToileState::new();
    create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let payload = clip_of(&st.handle_command(ToileCommand::CopyElements { ids: Vec::new() }));
    assert!(payload.elements.is_empty());
    let history = st.history();
    let out = st.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(50.0, 50.0),
    });
    assert!(out.update.is_none(), "no delta leaves");
    assert!(created_ids(&out).is_empty());
    assert_eq!(st.history(), history, "and nothing is written");
    assert_eq!(st.element_count(), ElementCount::new(1));
}

#[test]
fn duplicate_a_group_is_one_undo_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    let g = created_id(&st.handle_command(ToileCommand::GroupElements { ids: vec![a, b] }));
    assert_eq!(st.element_count(), ElementCount::new(3));

    // Duplicating the group re-mints the group and both members (subtree copy).
    st.handle_command(ToileCommand::DuplicateElements {
        ids: vec![g],
        delta: Vec2::new(200.0, 0.0),
    });
    assert_eq!(st.element_count(), ElementCount::new(6));

    // The whole duplicate collapses under a single undo.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), ElementCount::new(3));
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
    assert_eq!(dst.element_count(), ElementCount::ZERO);
    let out = dst.handle_command(ToileCommand::PasteElements {
        payload,
        at: Point::new(5.0, 5.0), // origin top-left is (5,5): zero offset
    });
    assert_eq!(dst.element_count(), ElementCount::new(2));
    assert_eq!(created_ids(&out).len(), 2);
}

// -----------------------------------------------------------------------------
// Undo / redo
// -----------------------------------------------------------------------------

#[test]
fn undo_reverts_create_and_redo_restores() {
    let mut st = ToileState::new();
    let id = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    assert_eq!(st.element_count(), ElementCount::new(1));

    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), ElementCount::ZERO);
    assert!(st.bounds_of(&id).is_none());

    st.handle_command(ToileCommand::Redo);
    assert_eq!(st.element_count(), ElementCount::new(1));
    assert_eq!(st.bounds_of(&id), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

/// `UNDO-002`: an undo and a redo each leave as an ordinary delta, and a
/// second peer that applies them reads the board the author reads.
#[test]
fn a_history_step_reaches_the_other_peer_as_a_delta() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let id = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    b.apply_remote_snapshot(&a.export_snapshot());
    assert_eq!(b.element_count(), ElementCount::new(1));

    let undo = a.handle_command(ToileCommand::Undo);
    b.apply_remote(&undo.update.expect("an undo writes"));
    assert_eq!(b.element_count(), ElementCount::ZERO);
    assert!(b.bounds_of(&id).is_none());

    let redo = a.handle_command(ToileCommand::Redo);
    b.apply_remote(&redo.update.expect("a redo writes"));
    assert_eq!(b.element_count(), ElementCount::new(1));
    assert_eq!(b.bounds_of(&id), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
}

/// `UNDO-003` and `UNDO-008`: an undo with nothing to revert writes nothing
/// and emits no delta. This holds on a fresh board and on a board undone to
/// its start.
#[test]
fn an_undo_with_nothing_to_revert_writes_nothing() {
    let mut st = ToileState::new();
    for round in 0..2 {
        let before = st.crdt().oplog_vv();
        let out = st.handle_command(ToileCommand::Undo);
        assert!(
            out.update.is_none(),
            "round {round}: the undo emitted a delta"
        );
        assert_eq!(
            st.crdt().oplog_vv(),
            before,
            "round {round}: the undo wrote"
        );
        create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
        st.handle_command(ToileCommand::Undo);
    }
}

/// `UNDO-007`: an undo reverts this peer's own work and leaves a remote
/// peer's move in place.
#[test]
fn an_undo_leaves_a_remote_move_alone() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let shared = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    b.apply_remote_snapshot(&a.export_snapshot());
    let own = create_rect(&mut b, Point::new(50.0, 0.0), Size::new(10.0, 10.0));
    let moved = a.handle_command(ToileCommand::MoveElements {
        ids: vec![shared.clone()],
        delta: Vec2::new(5.0, 7.0),
    });
    b.apply_remote(&moved.update.expect("a move writes"));
    b.handle_command(ToileCommand::Undo);
    assert!(
        b.bounds_of(&own).is_none(),
        "the undo takes back B's own sticky"
    );
    assert_eq!(b.bounds_of(&shared), Some(Rect::new(5.0, 7.0, 15.0, 17.0)));
    let out = b.handle_command(ToileCommand::Undo);
    assert!(
        out.update.is_none(),
        "B has nothing more of its own to revert"
    );
    assert_eq!(b.bounds_of(&shared), Some(Rect::new(5.0, 7.0, 15.0, 17.0)));
}

#[test]
fn undo_reverts_group_as_one_item() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(20.0, 0.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::GroupElements {
        ids: vec![a.clone(), b.clone()],
    });
    assert_eq!(st.element_count(), ElementCount::new(3));
    // A single undo drops the whole group, leaving the two members intact.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.element_count(), ElementCount::new(2));
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
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Arrow,
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
        radians: ambre_core::Radians::new(std::f64::consts::FRAC_PI_2),
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

/// `TOILE-285`: re-aiming an arrow at an element the board holds announces no
/// dangling arrow.
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

/// The arrows a derivation answers, by id.
fn painted_arrows(st: &ToileState) -> Vec<ConnectorId> {
    st.debug_viewport()
        .nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Connector(c) => Some(c.id.clone()),
            DisplayNode::Element(_) => None,
        })
        .collect()
}

/// `TOILE-045`, `TOILE-207`: an arrow is culled like an element. One far
/// away is not in the answer. One whose ends are both off screen but whose
/// outline crosses the frame is, and so is a curve whose bulge alone reaches
/// it. Moving an end off screen drops the arrow at the next camera.
#[test]
fn an_arrow_off_screen_is_culled() {
    let mut st = ToileState::new();
    let far_a = create_rect(&mut st, Point::new(5000.0, 5000.0), Size::new(10.0, 10.0));
    let far_b = create_rect(&mut st, Point::new(5100.0, 5000.0), Size::new(10.0, 10.0));
    let far = connect(&mut st, far_a, far_b);
    // Both ends left and right of the default frame; the line crosses it.
    let west = create_rect(&mut st, Point::new(-3000.0, 0.0), Size::new(10.0, 10.0));
    let east = create_rect(&mut st, Point::new(3000.0, 0.0), Size::new(10.0, 10.0));
    let across = connect(&mut st, west.clone(), east);
    // Both ends above the frame and wide of it, facing down: only the bulge
    // reaches into it.
    let low_a = create_rect(&mut st, Point::new(-1500.0, -700.0), Size::new(10.0, 10.0));
    let low_b = create_rect(&mut st, Point::new(1500.0, -700.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::CreateConnector {
        from: low_a,
        from_anch: Anchor::Bottom,
        to: low_b,
        to_anch: Anchor::Bottom,
        path_type: PathType::Curved,
        arrows: EndCaps {
            start: ambre_bus::app::toile::Cap::Plain,
            end: ambre_bus::app::toile::Cap::Plain,
        },
    });
    let bulge = st.connector_ids().pop().expect("the curve exists");

    let mut seen = painted_arrows(&st);
    seen.sort();
    let mut want = vec![across.clone(), bulge];
    want.sort();
    assert_eq!(
        seen, want,
        "only the arrows that meet the frame are answered"
    );
    assert!(!seen.contains(&far));

    st.handle_command(ToileCommand::MoveElements {
        ids: vec![west.clone()],
        delta: Vec2::new(0.0, 9000.0),
    });
    assert!(
        !painted_arrows(&st).contains(&across),
        "the arrow left with its end"
    );
}

// -----------------------------------------------------------------------------
// Presentation: slide sequence CRDT + playback (`toile-presentation`)
// -----------------------------------------------------------------------------

/// The playback as `(mode, slide, focus)`, with the slide as a bare rank.
fn played(st: &ToileState) -> (PresentMode, u32, bool) {
    let (mode, playback) = st.playback();
    (
        mode,
        playback.active.get(),
        playback.focus == PresentFocus::Focus,
    )
}

fn slide_changed(out: &crate::logic::command::ToileOutcome) -> Option<u32> {
    out.display.iter().find_map(|e| match e {
        ToileEvent::SlideChanged(c) => Some(c.position.index().get()),
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
    st.handle_command(ToileCommand::GotoSlide(SlideIndex::new(2)));
    assert_eq!(st.playback().1.active.get(), 2);

    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone()],
    });
    assert_eq!(
        st.playback().1.active.get(),
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
    st.handle_command(ToileCommand::GotoSlide(SlideIndex::new(2)));

    st.handle_command(ToileCommand::DeleteElements {
        ids: vec![b.clone()],
    });
    assert_eq!(
        st.slide_order(),
        vec![a.clone(), c.clone()],
        "the deck must hold only slides whose frame still exists"
    );
    assert_eq!(
        st.playback().1.active.get(),
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
        slide_changed(&st.handle_command(ToileCommand::GotoSlide(SlideIndex::new(99)))),
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

/// `TOILE-FEAT-081`: a slide change carries both slides' snapshots and the
/// camera target.
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
    assert_eq!(change.position.index().get(), 1);
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

/// `TOILE-FEAT-081`: a slide change carries the deck length, the target frame
/// and its notes.
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
        base: String::new(),
        delta: insert_notes(0, "Talk track for slide two"),
    });

    let change = slide_change(&st.handle_command(ToileCommand::NextSlide));
    assert_eq!(
        change.position.count().get(),
        2,
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

/// `TOILE-FEAT-111`: entering presentation mode starts from an empty source
/// snapshot, so the first slide fades in.
#[test]
fn enter_present_starts_from_empty_snapshot() {
    // Entering present mode has no source frame: `from` is empty so slide 0 fades
    // in rather than tweening from a stale frame.
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone()],
    });

    let change = slide_change(&st.handle_command(ToileCommand::EnterPresent {
        focus: PresentFocus::Focus,
    }));
    assert_eq!(change.position.index().get(), 0);
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

    let out = st.handle_command(ToileCommand::EnterPresent {
        focus: PresentFocus::Focus,
    });
    assert!(
        out.update.is_none(),
        "present mode is local, never a CRDT delta"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentModeChanged(PresentMode::Presenting))),
        "EnterPresent announces present mode"
    );
    assert_eq!(
        slide_changed(&out),
        Some(0),
        "EnterPresent resets to slide 0"
    );
    assert_eq!(played(&st), (PresentMode::Presenting, 0, true));

    let out = st.handle_command(ToileCommand::ExitPresent);
    assert!(out.update.is_none());
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentModeChanged(PresentMode::Editing)))
    );
    assert!(
        !st.playback().0.is_presenting(),
        "ExitPresent leaves present mode"
    );
}

/// `TOILE-FEAT-094`: leaving the presentation stops this viewer's broadcast.
/// The role is dropped, the panel hears it, and the peers get a frame that
/// names no slide.
#[test]
fn leaving_the_presentation_stops_the_broadcast() {
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    st.handle_command(ToileCommand::EnterPresent {
        focus: PresentFocus::Focus,
    });
    st.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));
    assert!(st.broadcast().is_on());

    let out = st.handle_command(ToileCommand::ExitPresent);
    assert!(
        !st.broadcast().is_on(),
        "the role outlived the presentation"
    );
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::PresentBroadcastChanged(Broadcast::Private)))
    );
    let frame = out.ephemeral.expect("the peers hear it");
    let mut rx = ToileState::new();
    let presenter = UserId::new();
    rx.apply_ephemeral(presenter, &frame);
    assert_eq!(
        rx.peer_presence(&presenter)
            .and_then(|p| p.present_slide.map(SlideIndex::get)),
        None
    );
}

/// `TOILE-FEAT-098`: each of the four paths that move the deck says so. A
/// local write, a history step, a remote delta and a deleted frame each emit
/// `DeckChanged`, which is what the open panel re-reads on.
#[test]
fn every_path_that_moves_the_deck_says_so() {
    let said = |events: &[ToileEvent]| events.iter().any(|e| matches!(e, ToileEvent::DeckChanged));
    let mut st = ToileState::new();
    let a = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let b = create_frame(&mut st, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    let mut peer = ToileState::new();
    peer.apply_remote_snapshot(&st.export_snapshot());

    let write = st.handle_command(ToileCommand::SetSlideOrder {
        order: vec![a.clone(), b.clone()],
    });
    assert!(said(&write.display), "a local write");
    let delta = write.update.expect("a deck write is a delta");
    assert!(said(&peer.apply_remote(&delta)), "a remote delta");

    assert!(
        said(&st.handle_command(ToileCommand::Undo).display),
        "a history step"
    );
    st.handle_command(ToileCommand::Redo);
    let deleted = st.handle_command(ToileCommand::DeleteElements { ids: vec![b] });
    assert!(said(&deleted.display), "a deleted frame");
    assert_eq!(st.slide_order(), [a]);
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
    b.apply_remote_snapshot(&a.export_snapshot());
    assert_eq!(a.slide_order(), b.slide_order(), "the sequence is shared");

    // A enters present mode and steps forward; neither produces a delta to relay.
    assert!(
        a.handle_command(ToileCommand::EnterPresent {
            focus: PresentFocus::WithChrome
        })
        .update
        .is_none()
    );
    assert!(a.handle_command(ToileCommand::NextSlide).update.is_none());
    assert_eq!(played(&a), (PresentMode::Presenting, 1, false));
    assert_eq!(
        played(&b),
        (PresentMode::Editing, 0, false),
        "B's playback is untouched by A's presentation"
    );
}

// -----------------------------------------------------------------------------
// Follow presenter: ephemeral slide broadcast (`toile-presentation`)
// -----------------------------------------------------------------------------

/// Decode a broadcast slide index out of an outcome's ephemeral, if any.
fn broadcast_slide(out: &crate::logic::command::ToileOutcome) -> Option<u32> {
    let bytes = out.ephemeral.as_ref()?;
    let p: PresencePayload =
        postcard::from_bytes(bytes.as_bytes()).expect("decodable presence sample");
    p.present_slide.map(SlideIndex::get)
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

/// `TOILE-029`: two peers building a deck at the same moment write into one
/// list. The order lives in a root container, whose identity is its name, so
/// neither peer creates it and neither side's slides are dropped.
#[test]
fn concurrent_first_decks_converge_on_one_list() {
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    let f1 = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let f2 = create_frame(&mut a, Point::new(300.0, 0.0), Size::new(200.0, 200.0));
    // B takes the board, frames and all, before either peer has a deck.
    b.apply_remote_snapshot(&a.export_snapshot());
    assert!(a.slide_order().is_empty() && b.slide_order().is_empty());

    // Each peer builds a deck, neither having seen the other's.
    let da = a
        .handle_command(ToileCommand::SetSlideOrder {
            order: vec![f1.clone()],
        })
        .update
        .expect("A's deck broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::SetSlideOrder {
            order: vec![f2.clone()],
        })
        .update
        .expect("B's deck broadcasts a delta");
    b.apply_remote(&da);
    a.apply_remote(&db);

    let merged = a.slide_order();
    assert_eq!(merged, b.slide_order(), "the two decks diverged");
    assert!(
        merged.contains(&f1) && merged.contains(&f2),
        "a rival list dropped one peer's slides, got {merged:?}"
    );
}

#[test]
fn broadcast_sample_carries_active_slide() {
    // While broadcasting, a navigation additionally emits an ephemeral
    // carrying the new active index -- and never a CRDT delta.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    st.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));

    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(slide_changed(&out), Some(1), "the local view still glides");
    assert_eq!(
        broadcast_slide(&out),
        Some(1),
        "the ephemeral carries the new active index"
    );
    assert!(out.update.is_none(), "playback is never a CRDT delta");
}

#[test]
fn no_broadcast_sample_when_not_broadcasting() {
    // With broadcasting off, navigation stays purely local: no ephemeral.
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(slide_changed(&out), Some(1));
    assert!(
        out.ephemeral.is_none(),
        "no broadcast frame while broadcasting is off"
    );
}

/// `TOILE-FEAT-028`: a transition lasts what the deck configures, not a
/// constant of the render thread. The length rides the slide event, because
/// the UI thread holds no board data.
#[test]
fn a_transition_lasts_what_the_deck_configures() {
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(
        slide_change(&out).transition,
        std::time::Duration::from_millis(300),
        "a board nobody configured carries the default"
    );

    st.set_transition_ms_for_test(900.0);
    let out = st.handle_command(ToileCommand::PrevSlide);
    assert_eq!(
        slide_change(&out).transition,
        std::time::Duration::from_millis(900)
    );

    // A register holding something else, or a length nothing could play, reads
    // as the default rather than freezing or skipping every slide change.
    st.set_transition_ms_for_test(-1.0);
    let out = st.handle_command(ToileCommand::NextSlide);
    assert_eq!(
        slide_change(&out).transition,
        std::time::Duration::from_millis(300)
    );
}

/// `TOILE-UI-316`: a peer writing into the note on screen reaches this
/// viewer's panel. The editor is seeded by the slide event alone, so without
/// the event the writing stays invisible until the slide changes.
#[test]
fn a_peers_note_edit_reaches_the_slide_on_screen() {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    a.handle_command(ToileCommand::SetSlideOrder {
        order: vec![frame.clone()],
    });
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    b.handle_command(ToileCommand::EnterPresent {
        focus: PresentFocus::Focus,
    });

    let delta = a
        .handle_command(ToileCommand::EditNotes {
            frame: frame.clone(),
            base: String::new(),
            delta: TextDelta {
                at: CharOffset::ZERO,
                delete: CharCount::ZERO,
                insert: "open on the budget".into(),
            },
        })
        .update
        .expect("the note broadcasts a delta");

    let events = b.apply_remote(&delta);
    let reported = events.iter().find_map(|e| match e {
        ToileEvent::NotesChanged { frame, notes } => Some((frame.clone(), notes.clone())),
        _ => None,
    });
    assert_eq!(
        reported,
        Some((frame, "open on the budget".to_owned())),
        "the note on screen moved and the panel was not told"
    );

    // A remote change that leaves the note alone says nothing.
    let other = create_rect(&mut a, Point::new(900.0, 0.0), Size::new(10.0, 10.0));
    let _ = other;
    let delta = a.export_snapshot();
    assert!(
        !b.apply_remote_snapshot(&delta)
            .iter()
            .any(|e| matches!(e, ToileEvent::NotesChanged { .. }))
    );
}

/// `TOILE-UI-315`: a viewer that stops broadcasting says so on the wire. The
/// peers read the whole live set. A sample naming no slide is what drops this
/// viewer from the presenters the panel offers to follow.
#[test]
fn stopping_a_broadcast_sends_a_sample_naming_no_slide() {
    let mut st = ToileState::new();
    deck_of_three(&mut st);
    st.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));

    let frame = st
        .handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Private))
        .ephemeral
        .expect("stopping broadcasts one last frame");
    let mut rx = ToileState::new();
    let presenter = UserId::new();
    rx.apply_ephemeral(presenter, &frame);
    assert_eq!(
        rx.peer_presence(&presenter)
            .and_then(|p| p.present_slide.map(SlideIndex::get)),
        None,
        "the peer is still offered as a presenter"
    );

    // A viewer that was not broadcasting says nothing.
    assert!(
        st.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Private))
            .ephemeral
            .is_none()
    );
}

#[test]
fn follower_adopts_presenter_slide() {
    // A follower adopts the presenter's broadcast slide (gliding via SlideChanged)
    // without touching the CRDT or its history.
    let mut presenter = ToileState::new();
    deck_of_three(&mut presenter);
    presenter.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(SlideIndex::new(2)))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote_snapshot(&presenter.export_snapshot());
    let alice = UserId::new();
    rx.handle_command(ToileCommand::FollowPresenter(Some(alice)));

    let before = rx.export_snapshot();
    let events = rx.apply_ephemeral(alice, &frame);
    assert_eq!(
        rx.playback().1.active.get(),
        2,
        "the follower adopted slide 2"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ToileEvent::SlideChanged(c) if c.position.index().get() == 2)),
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
    // Not following anyone: a broadcast sample updates presence but never moves us.
    let mut presenter = ToileState::new();
    deck_of_three(&mut presenter);
    presenter.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(SlideIndex::new(2)))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote_snapshot(&presenter.export_snapshot());
    let events = rx.apply_ephemeral(UserId::new(), &frame);
    assert_eq!(
        rx.playback().1.active.get(),
        0,
        "a non-follower does not move"
    );
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
    presenter.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));
    let frame = presenter
        .handle_command(ToileCommand::GotoSlide(SlideIndex::new(2)))
        .ephemeral
        .expect("broadcast frame");

    let mut rx = ToileState::new();
    rx.apply_remote_snapshot(&presenter.export_snapshot());
    let bob = UserId::new();
    rx.handle_command(ToileCommand::FollowPresenter(Some(bob)));
    let alice = UserId::new();
    rx.apply_ephemeral(alice, &frame);
    assert_eq!(
        rx.playback().1.active.get(),
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
        present_slide: Some(SlideIndex::new(99)),
        ..Default::default()
    })
    .unwrap();
    rx.apply_ephemeral(alice, &frame.into());
    assert_eq!(
        rx.playback().1.active.get(),
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
        present_slide: Some(SlideIndex::new(2)),
        ..Default::default()
    })
    .unwrap();
    rx.apply_ephemeral(alice, &frame.into());
    assert_eq!(
        rx.playback().1.active.get(),
        0,
        "no adoption before following"
    );

    let out = rx.handle_command(ToileCommand::FollowPresenter(Some(alice)));
    assert_eq!(
        rx.playback().1.active.get(),
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
    let out = st.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting));
    assert!(out.update.is_none(), "the broadcast toggle is never CRDT");
    assert!(
        out.display.iter().any(|e| matches!(
            e,
            ToileEvent::PresentBroadcastChanged(Broadcast::Broadcasting)
        )),
        "the toggle is announced to the UI"
    );
    assert!(st.broadcast().is_on());
}

#[test]
fn follow_state_is_not_synced() {
    // Follow / broadcast toggles never produce a CRDT delta, and one peer's
    // toggles never touch another peer's playback (mirrors playback_is_not_synced).
    let mut a = ToileState::new();
    let mut b = ToileState::new();
    deck_of_three(&mut a);
    b.apply_remote_snapshot(&a.export_snapshot());

    let alice = UserId::new();
    assert!(
        a.handle_command(ToileCommand::FollowPresenter(Some(alice)))
            .update
            .is_none()
    );
    assert!(
        a.handle_command(ToileCommand::SetPresentBroadcast(Broadcast::Broadcasting))
            .update
            .is_none()
    );
    assert_eq!(
        played(&b),
        (PresentMode::Editing, 0, false),
        "B's playback is untouched by A's follow / broadcast toggles"
    );
}

// -----------------------------------------------------------------------------
// Speaker notes (toile-presentation)
// -----------------------------------------------------------------------------

/// A single-splice insertion at `at` (no deletion), the common editor edit.
fn insert_notes(at: usize, s: &str) -> TextDelta {
    TextDelta {
        at: CharOffset::from_usize(at),
        delete: CharCount::ZERO,
        insert: s.to_owned(),
    }
}

#[test]
fn edit_notes_produces_delta_and_is_undoable() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));

    let out = st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: String::new(),
        delta: insert_notes(0, "Intro slide"),
    });
    assert!(
        out.update.is_some(),
        "speaker notes are CRDT and must broadcast a delta"
    );
    assert_eq!(st.notes(&f).unwrap_or_default(), "Intro slide");

    // A second splice appends " -- talk track" after the existing text.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: "Intro slide".into(),
        delta: insert_notes(11, " -- talk track"),
    });
    assert_eq!(
        st.notes(&f).unwrap_or_default(),
        "Intro slide -- talk track"
    );

    // Each EditNotes is one undo item: undo drops the last splice only.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.notes(&f).unwrap_or_default(),
        "Intro slide",
        "one undo reverts one splice"
    );
    st.handle_command(ToileCommand::Redo);
    assert_eq!(
        st.notes(&f).unwrap_or_default(),
        "Intro slide -- talk track",
        "redo re-applies it"
    );
}

/// `TOILE-FEAT-088`: the panel measured its keystroke against a note a peer
/// has since moved. The keystroke lands where it was typed, and the panel is
/// told the merged note, since it shows only its own.
#[test]
fn a_splice_measured_against_a_moved_note_lands_where_it_was_typed() {
    let mut a = ToileState::new();
    let f = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    a.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: String::new(),
        delta: insert_notes(0, "Hello world"),
    });
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let peer = b
        .handle_command(ToileCommand::EditNotes {
            frame: f.clone(),
            base: "Hello world".into(),
            delta: insert_notes(0, "Dear "),
        })
        .update
        .expect("a delta");
    a.apply_remote(&peer);

    // A's panel still holds "Hello world" and appends "!" to it.
    let out = a.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: "Hello world".into(),
        delta: insert_notes(11, "!"),
    });
    assert_eq!(
        a.notes(&f).unwrap_or_default(),
        "Dear Hello world!",
        "not \"Dear Hello !world\""
    );
    assert!(
        out.display.iter().any(|e| matches!(
            e,
            ToileEvent::NotesChanged { frame, notes } if frame == &f && notes == "Dear Hello world!"
        )),
        "the panel is told the merged note"
    );
    b.apply_remote(&out.update.expect("a delta"));
    assert_eq!(b.notes(&f).unwrap_or_default(), "Dear Hello world!");

    // A splice measured against the note as stored says nothing more.
    let out = a.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: "Dear Hello world!".into(),
        delta: insert_notes(17, "!"),
    });
    assert_eq!(a.notes(&f).unwrap_or_default(), "Dear Hello world!!");
    assert!(
        !out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::NotesChanged { .. })),
        "the panel already shows it"
    );
    a.handle_command(ToileCommand::Undo);
    a.handle_command(ToileCommand::Undo);
    assert_eq!(
        a.notes(&f).unwrap_or_default(),
        "Dear Hello world",
        "each keystroke is one undo"
    );
}

#[test]
fn edit_notes_clamps_out_of_range_splice() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: String::new(),
        delta: insert_notes(0, "abc"),
    });
    // The handler clamps an `at` past the end to the length, so the insert lands
    // at the tail instead of earning a refusal.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: "abc".into(),
        delta: insert_notes(999, "Z"),
    });
    assert_eq!(st.notes(&f).unwrap_or_default(), "abcZ");
    // The handler clamps an oversized deletion span to what remains.
    st.handle_command(ToileCommand::EditNotes {
        frame: f.clone(),
        base: "abcZ".into(),
        delta: TextDelta {
            at: CharOffset::ZERO,
            delete: CharCount::new(999),
            insert: String::new(),
        },
    });
    assert_eq!(st.notes(&f).unwrap_or_default(), "");
}

#[test]
fn edit_notes_rejects_non_frame() {
    let mut st = ToileState::new();
    let sticky = create_sticky(&mut st, Point::ZERO, "body");

    // A sticky carries no `children` field, so it is not a frame: rejected no-op.
    let out = st.handle_command(ToileCommand::EditNotes {
        frame: sticky.clone(),
        base: String::new(),
        delta: insert_notes(0, "nope"),
    });
    assert!(out.update.is_none(), "a rejected EditNotes emits no delta");
    assert!(
        out.display
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::ContentKindMismatch))),
        "notes on a non-frame report a ContentKindMismatch"
    );
    assert_eq!(
        st.notes(&sticky),
        None,
        "no notes were written to the sticky"
    );

    // A missing element reports NoSuchElement.
    let out = st.handle_command(ToileCommand::EditNotes {
        frame: ElementId("ghost".to_owned()),
        base: String::new(),
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
    b.apply_remote_snapshot(&a.export_snapshot());
    assert_eq!(a.notes(&f).unwrap_or_default(), "");
    assert_eq!(b.notes(&f).unwrap_or_default(), "");

    // Concurrent, non-overlapping edits, each unaware of the other.
    let da = a
        .handle_command(ToileCommand::EditNotes {
            frame: f.clone(),
            base: String::new(),
            delta: insert_notes(0, "AAA"),
        })
        .update
        .expect("A's notes edit broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::EditNotes {
            frame: f.clone(),
            base: String::new(),
            delta: insert_notes(0, "BBB"),
        })
        .update
        .expect("B's notes edit broadcasts a delta");

    // Relay each peer's delta to the other.
    b.apply_remote(&da);
    a.apply_remote(&db);

    // Char-CRDT convergence: identical final notes on both peers, and both edits
    // survived (no LWW loss of a rival container).
    assert_eq!(
        a.notes(&f).unwrap_or_default(),
        b.notes(&f).unwrap_or_default(),
        "concurrent notes edits diverged"
    );
    let merged = a.notes(&f).unwrap_or_default();
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
        gap: ambre_core::BoardLength::new(gap),
        padding: ambre_bus::app::toile::Padding::uniform(ambre_core::BoardLength::new(4.0)),
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
    b.apply_remote_snapshot(&a.export_snapshot());

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
        kind: ReactionKind::Emoji(ambre_bus::Emoji::from(s)),
        ts: ambre_bus::Timestamp(0),
    }
}

fn vote(w: i32) -> Reaction {
    Reaction {
        kind: ReactionKind::Vote(ambre_bus::app::toile::VoteWeight::new(w)),
        ts: ambre_bus::Timestamp(0),
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
    assert_eq!(
        tally.emoji,
        vec![(
            ambre_bus::Emoji::from("thumbsup"),
            ambre_bus::app::toile::ReactionCount::new(1)
        )]
    );
    assert_eq!(tally.vote_count.get(), 0);

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
    assert_eq!((tally.vote_total.get(), tally.vote_count.get()), (1, 1));
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
    assert_eq!(
        tally.emoji,
        vec![(
            ambre_bus::Emoji::from("thumbsup"),
            ambre_bus::app::toile::ReactionCount::new(1)
        )]
    );

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
        correlate: None,
    });

    let geom = only_doc_embed(&st).expect("the embed must derive as DocEmbed");
    let DisplayGeom::DocEmbed {
        document_id,
        doc_type,
        focused,
    } = geom
    else {
        panic!("filtered above")
    };
    assert_eq!(document_id, child, "the child reference did not round-trip");
    assert_eq!(doc_type, ambre_core::DocType::Trame);
    // Focus is view-local state, never CRDT: derive always says false.
    assert_eq!(
        focused,
        ambre_bus::app::toile::InputFocus::Board,
        "focus must not be derived from the document"
    );
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
        correlate: None,
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
    st.debug_set_element_field(&id, crate::logic::state::ElementKey::DocId, "not-a-uuid");

    assert!(
        only_doc_embed(&st).is_none(),
        "an unparsable reference must not derive as a live embed"
    );
    let geom = st.debug_viewport().nodes.into_iter().find_map(|n| match n {
        DisplayNode::Element(e) => Some(e.geom),
        _ => None,
    });
    assert!(
        matches!(geom, Some(DisplayGeom::Frame(_))),
        "expected a Frame placeholder, got {geom:?}"
    );
}

/// `TOILE-267`: a hostile peer writes an unsafe address into an embed. The
/// reader's board derives no address from it, so no frame ever loads it. The
/// shell builds its frames from `visible_web_embeds` alone.
#[test]
fn a_peers_unsafe_web_address_never_reaches_a_frame() {
    let hostile = [
        "javascript:alert(document.domain)",
        "JaVaScRiPt:alert(1)",
        "java\nscript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "blob:https://example.com/0f0e",
        "http://example.com",
        "about:blank",
        "",
    ];
    for value in hostile {
        let mut peer = ToileState::new();
        let out = peer.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Embed,
            at: Point::ZERO,
            size: Size::new(100.0, 100.0),
            fill: None,
            content: InitContent::Embed(
                ambre_bus::app::toile::WebAddress::parse("example.com").expect("a safe address"),
            ),
            correlate: None,
        });
        let id = created_id(&out);
        // The peer writes the field as raw text, past every check its own
        // commands would run.
        peer.debug_set_element_field(&id, crate::logic::state::ElementKey::Url, value);

        let mut reader = ToileState::new();
        reader.apply_remote_snapshot(&peer.export_snapshot());
        let cam = crate::ui::viewport::ViewportTransform::new(ambre_draw::Size::new(1000.0, 800.0));
        let frames = crate::ui::embed::visible_web_embeds(&reader.debug_viewport(), &cam, None);
        assert!(
            frames.is_empty(),
            "{value:?} reached a frame as {:?}",
            frames.iter().map(|f| &f.url).collect::<Vec<_>>()
        );
    }
}

/// `TOILE-085` and `TOILE-226`: a pasted embed is a second reference to the
/// same document. The copy and the paste ask the service plane for nothing,
/// so no second document comes into being.
#[test]
fn a_copied_embed_is_a_second_reference_to_the_same_document() {
    let mut st = ToileState::new();
    let child = ambre_core::DocumentId::new();
    let out = st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::DocEmbed,
        at: Point::ZERO,
        size: Size::new(400.0, 300.0),
        fill: None,
        content: InitContent::DocEmbed {
            document_id: child,
            doc_type: ambre_core::DocType::Prose,
        },
        correlate: None,
    });
    let source = created_ids(&out).pop().expect("the embed");

    let copy = st.handle_command(ToileCommand::CopyElements {
        ids: vec![source.clone()],
    });
    assert!(copy.update.is_none() && copy.requests.is_empty());
    let paste = st.handle_command(ToileCommand::PasteElements {
        payload: clip_of(&copy),
        // Inside the default camera, so the derived frame carries both.
        at: Point::new(420.0, 0.0),
    });
    assert!(paste.requests.is_empty(), "the paste asks for no document");
    let pasted = created_ids(&paste);
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], source, "a new element");

    let references: Vec<_> = st
        .debug_viewport()
        .nodes
        .into_iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => match e.geom {
                DisplayGeom::DocEmbed {
                    document_id,
                    doc_type,
                    ..
                } => Some((e.id, document_id, doc_type)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(references.len(), 2, "two embeds: {references:?}");
    for (_, document_id, doc_type) in &references {
        assert_eq!(*document_id, child, "both name the one document");
        assert_eq!(*doc_type, ambre_core::DocType::Prose);
    }
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
    b.apply_remote_snapshot(&a.export_snapshot());

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
        vec![(
            ambre_bus::Emoji::from("thumbsup"),
            ambre_bus::app::toile::ReactionCount::new(2)
        )],
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
    b.apply_remote_snapshot(&a.export_snapshot());

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
        ta.vote_count.get(),
        1,
        "two devices of one user is a single entry, not two votes"
    );
}

/// A board whose one target carries one reaction per entry. A distinct user
/// places each one on a peer of its own, and the board merges it back in order.
fn reacted_by(reactions: &[Reaction]) -> (ToileState, ElementId) {
    let (mut board, t) = state_with_user(UserId::new());
    for reaction in reactions {
        let mut peer = ToileState::new();
        peer.set_user_id(UserId::new());
        peer.apply_remote_snapshot(&board.export_snapshot());
        let delta = peer
            .handle_command(ToileCommand::React {
                target: t.clone(),
                reaction: Some(reaction.clone()),
            })
            .update
            .expect("each reaction writes a delta");
        board.apply_remote(&delta);
    }
    (board, t)
}

/// `TOILE-FEAT-057`: the derived frame carries the tallies of what the camera
/// sees. A reacted target scrolled out of view is left out of the frame. The
/// board still holds and counts its reaction: it is skipped, not deleted.
#[test]
fn a_reacted_target_out_of_view_is_left_out_of_the_frame() {
    let (mut st, near) = state_with_user(UserId::new());
    let far = create_sticky(&mut st, Point::new(1.0e6, 1.0e6), "far");
    for t in [&near, &far] {
        st.handle_command(ToileCommand::React {
            target: t.clone(),
            reaction: Some(emoji("heart")),
        });
    }
    st.handle_command(ToileCommand::SetViewport(crate::logic::index::Viewport {
        center: Point::ZERO,
        zoom: ambre_core::ZoomFactor::positive(1.0),
        size: Size::new(800.0, 600.0),
    }));

    let frame = st.debug_viewport();
    assert!(frame.reactions.contains_key(&near), "the target in view");
    assert!(
        !frame.reactions.contains_key(&far),
        "the target out of view costs nothing"
    );
    assert_eq!(
        st.reaction_tally(&far).emoji,
        vec![(
            ambre_bus::Emoji::from("heart"),
            ambre_bus::app::toile::ReactionCount::new(1)
        )],
        "the board still holds the reaction out of view"
    );
}

/// `TOILE-FEAT-058`: two emoji on one target read in one order, whatever
/// order the reactions arrived in, on every read and on every peer.
#[test]
fn the_chips_of_a_target_keep_one_order() {
    let (one, t) = reacted_by(&[emoji("zebra"), emoji("apple"), emoji("zebra")]);
    let (other, u) = reacted_by(&[emoji("apple"), emoji("zebra"), emoji("zebra")]);
    let expected = vec![
        (
            ambre_bus::Emoji::from("apple"),
            ambre_bus::app::toile::ReactionCount::new(1),
        ),
        (
            ambre_bus::Emoji::from("zebra"),
            ambre_bus::app::toile::ReactionCount::new(2),
        ),
    ];

    assert_eq!(one.reaction_tally(&t).emoji, expected);
    assert_eq!(one.reaction_tally(&t), one.reaction_tally(&t), "two reads");
    assert_eq!(
        other.reaction_tally(&u).emoji,
        expected,
        "another arrival order"
    );
    let frame = one.debug_viewport();
    assert_eq!(
        frame.reactions.get(&t).map(|tally| tally.emoji.clone()),
        Some(expected),
        "the frame reads the same order"
    );
}

/// `TOILE-FEAT-059`: votes of opposite signs fold into one signed total and
/// one voter count, in the single-target read and in the frame alike.
#[test]
fn votes_fold_into_one_signed_total_and_one_count() {
    let (st, t) = reacted_by(&[vote(3), vote(-1), vote(-1), emoji("heart")]);
    let tally = st.reaction_tally(&t);
    assert_eq!((tally.vote_total.get(), tally.vote_count.get()), (1, 3));
    assert_eq!(
        tally.emoji,
        vec![(
            ambre_bus::Emoji::from("heart"),
            ambre_bus::app::toile::ReactionCount::new(1)
        )],
        "a vote is no chip"
    );
    assert_eq!(st.debug_viewport().reactions.get(&t), Some(&tally));

    let (st, t) = reacted_by(&[vote(1), vote(-1)]);
    let tally = st.reaction_tally(&t);
    assert_eq!(
        (tally.vote_total.get(), tally.vote_count.get()),
        (0, 2),
        "a zero total still counts its voters"
    );
}

/// `TOILE-FEAT-063`: placing again the reaction the user already has writes
/// nothing, says nothing and costs no undo item.
#[test]
fn placing_the_same_reaction_again_does_nothing() {
    let (mut st, t) = state_with_user(UserId::new());
    st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("heart")),
    });

    let again = st.handle_command(ToileCommand::React {
        target: t.clone(),
        reaction: Some(emoji("heart")),
    });
    assert!(again.update.is_none(), "no delta");
    assert!(again.display.is_empty(), "no event: {:?}", again.display);

    // One undo removes the reaction: the second gesture pushed nothing. The
    // next undo item is the target itself.
    st.handle_command(ToileCommand::Undo);
    assert_eq!(st.my_reaction(&t), None);
    assert_eq!(
        st.element_count(),
        ElementCount::new(1),
        "the target is still there"
    );
    st.handle_command(ToileCommand::Undo);
    assert_eq!(
        st.element_count(),
        ElementCount::ZERO,
        "the next undo item is the target"
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
fn comment_on(
    st: &mut ToileState,
    anchor: CommentAnchor,
    body: &str,
    ts: ambre_core::Timestamp,
) -> ThreadId {
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

    let on_element = comment_on(
        &mut st,
        CommentAnchor::Element(e.clone()),
        "on the note",
        ambre_bus::Timestamp(0),
    );
    let on_board = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(40.0, -12.5)),
        "on the board",
        ambre_bus::Timestamp(0),
    );

    assert_eq!(
        st.comment_thread(&on_element).map(|t| t.anchor),
        Some(CommentAnchor::Element(e)),
    );
    assert_eq!(
        st.comment_thread(&on_board).map(|t| t.anchor),
        Some(CommentAnchor::Point(kurbo::Point::new(40.0, -12.5))),
    );
}

#[test]
fn a_message_author_is_the_user_id_of_the_session() {
    // TOILE-FEAT-014: the message carries the person, not the session. A board
    // with no authenticated session writes nothing at all.
    let me = UserId::new();
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        body: "too early".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    assert!(
        out.update.is_none(),
        "a comment before SessionReady writes nothing"
    );
    assert!(st.comment_thread_ids().is_empty());

    st.set_user_id(me);
    let t = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "hello",
        ambre_bus::Timestamp(7),
    );
    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].author, me);
    assert_eq!(thread.messages[0].ts, ambre_bus::Timestamp(7));
    assert_eq!(thread.resolved, ambre_core::ThreadState::Open);
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
    b.apply_remote_snapshot(&a.export_snapshot());

    let out_a = a.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Element(e.clone()),
        body: "from A".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    let out_b = b.handle_command(ToileCommand::CreateComment {
        anchor: CommentAnchor::Element(e),
        body: "from B".to_owned(),
        ts: ambre_bus::Timestamp(0),
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
    let t = comment_on(
        &mut a,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "first",
        ambre_bus::Timestamp(50),
    );

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: "from A".to_owned(),
            ts: ambre_bus::Timestamp(900),
        })
        .update
        .expect("A's reply broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: "from B".to_owned(),
            ts: ambre_bus::Timestamp(1),
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
    let t = comment_on(
        &mut a,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "typo",
        ambre_bus::Timestamp(0),
    );

    let mut b = ToileState::new();
    b.set_user_id(me);
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::EditComment {
            thread: t.clone(),
            message: ThreadMessageIndex::ZERO,
            delta: insert_notes(0, "AAA"),
        })
        .update
        .expect("device A's edit broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::EditComment {
            thread: t.clone(),
            message: ThreadMessageIndex::ZERO,
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
    let t = comment_on(
        &mut a,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "mine",
        ambre_bus::Timestamp(0),
    );

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote_snapshot(&a.export_snapshot());

    let out = b.handle_command(ToileCommand::EditComment {
        thread: t.clone(),
        message: ThreadMessageIndex::ZERO,
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
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "settled?",
        ambre_bus::Timestamp(0),
    );

    let mut b = ToileState::new();
    b.set_user_id(UserId::new());
    b.apply_remote_snapshot(&a.export_snapshot());

    let da = a
        .handle_command(ToileCommand::ResolveThread {
            thread: t.clone(),
            state: ambre_core::ThreadState::Resolved,
        })
        .update
        .expect("A's resolve broadcasts a delta");
    let db = b
        .handle_command(ToileCommand::ResolveThread {
            thread: t.clone(),
            state: ambre_core::ThreadState::Resolved,
        })
        .update
        .expect("B's resolve broadcasts a delta");

    b.apply_remote(&da);
    a.apply_remote(&db);

    let ra = a.comment_thread(&t).expect("A still holds the thread");
    let rb = b.comment_thread(&t).expect("B still holds the thread");
    assert_eq!(ra, rb, "concurrent resolves diverged");
    assert_eq!(
        ra.resolved,
        ambre_core::ThreadState::Resolved,
        "the thread reads resolved on both peers"
    );
    assert_eq!(ra.messages.len(), 1, "resolving keeps the messages");

    // Idempotent: resolving an already-resolved thread writes nothing.
    let again = a.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        state: ambre_core::ThreadState::Resolved,
    });
    assert!(again.update.is_none(), "a redundant resolve is not a write");

    // Reopening restores the thread whole.
    a.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        state: ambre_core::ThreadState::Open,
    });
    let reopened = a.comment_thread(&t).expect("the thread survives a reopen");
    assert_eq!(reopened.resolved, ambre_core::ThreadState::Open);
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
        ambre_bus::Timestamp(0),
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
    let t = comment_on(
        &mut st,
        CommentAnchor::Element(e),
        "already read",
        ambre_bus::Timestamp(0),
    );

    st.handle_command(ToileCommand::Undo);

    assert_eq!(
        st.element_count(),
        ElementCount::ZERO,
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
        ts: ambre_bus::Timestamp(0),
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
    let t = comment_on(
        &mut st,
        CommentAnchor::Element(e.clone()),
        "the point",
        ambre_bus::Timestamp(11),
    );

    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert_eq!(
        thread.resolved,
        ambre_core::ThreadState::Open,
        "a dropped thread is open"
    );
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
    let t = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "one",
        ambre_bus::Timestamp(0),
    );
    for body in ["two", "three"] {
        st.handle_command(ToileCommand::ReplyComment {
            thread: t.clone(),
            body: body.to_owned(),
            ts: ambre_bus::Timestamp(0),
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
    let t = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "first",
        ambre_bus::Timestamp(0),
    );
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "secnd".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "third".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });

    // Correct the middle message: insert the missing letter.
    st.handle_command(ToileCommand::EditComment {
        thread: t.clone(),
        message: ThreadMessageIndex::new(1),
        delta: TextDelta {
            at: CharOffset::new(3),
            delete: CharCount::ZERO,
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
    let anchor = CommentAnchor::Point(kurbo::Point::new(12.0, -4.0));
    let t = comment_on(&mut st, anchor.clone(), "open it", ambre_bus::Timestamp(0));
    st.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        state: ambre_core::ThreadState::Resolved,
    });
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "one more".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });

    let thread = st.comment_thread(&t).expect("the thread reads back");
    assert_eq!(thread.anchor, anchor, "a reply does not move the pin");
    assert_eq!(
        thread.resolved,
        ambre_core::ThreadState::Resolved,
        "a reply does not reopen the thread"
    );
}

#[test]
fn a_resolve_keeps_every_message_body() {
    // TOILE-FEAT-074: a settled point stays readable. A resolve that blanked the
    // message it kept fails here.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let t = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "why",
        ambre_bus::Timestamp(0),
    );
    st.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "because".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    st.handle_command(ToileCommand::ResolveThread {
        thread: t.clone(),
        state: ambre_core::ThreadState::Resolved,
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

    let t = comment_on(
        &mut a,
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "seed",
        ambre_bus::Timestamp(0),
    );
    b.apply_remote_snapshot(&a.export_snapshot());

    let ra = a.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "from A".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    let rb = b.handle_command(ToileCommand::ReplyComment {
        thread: t.clone(),
        body: "from B".to_owned(),
        ts: ambre_bus::Timestamp(0),
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
    let on_element = comment_on(
        &mut st,
        CommentAnchor::Element(e),
        "about it",
        ambre_bus::Timestamp(0),
    );
    st.handle_command(ToileCommand::ReplyComment {
        thread: on_element.clone(),
        body: "agreed".to_owned(),
        ts: ambre_bus::Timestamp(0),
    });
    let on_board = comment_on(
        &mut st,
        CommentAnchor::Point(kurbo::Point::new(5.0, 6.0)),
        "here",
        ambre_bus::Timestamp(0),
    );
    st.handle_command(ToileCommand::ResolveThread {
        thread: on_board.clone(),
        state: ambre_core::ThreadState::Resolved,
    });

    let pins = pins(&st);
    let element_pin = pins
        .iter()
        .find(|p| p.thread == on_element)
        .expect("the anchored thread has a pin");
    assert_eq!(element_pin.messages.get(), 2);
    assert_eq!(element_pin.resolved, ambre_core::ThreadState::Open);
    // The pin sits on the element it is about: its top-right corner.
    assert_eq!(element_pin.at, Point::new(90.0, 20.0));

    let board_pin = pins
        .iter()
        .find(|p| p.thread == on_board)
        .expect("the free-point thread has a pin");
    assert_eq!(board_pin.at, Point::new(5.0, 6.0));
    assert_eq!(board_pin.resolved, ambre_core::ThreadState::Resolved);
}

#[test]
fn a_pin_leaves_the_board_with_its_anchor_element() {
    // TOILE-025: a pin whose element is gone is not painted -- and the thread
    // itself stays, which `a_thread_survives_the_deletion_of_its_anchor_element`
    // holds.
    let mut st = ToileState::new();
    st.set_user_id(UserId::new());
    let e = create_sticky(&mut st, Point::ZERO, "note");
    let t = comment_on(
        &mut st,
        CommentAnchor::Element(e.clone()),
        "about it",
        ambre_bus::Timestamp(0),
    );
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
        CommentAnchor::Point(kurbo::Point::new(0.0, 0.0)),
        "the body",
        ambre_bus::Timestamp(0),
    );

    // The board-wide list carries the opening line and the counts, no more.
    let list = read_comments(&mut st, None);
    let Some(ToileEvent::CommentList(rows)) = list.first() else {
        panic!("expected a CommentList, got {list:?}");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, t);
    assert_eq!(rows[0].messages.get(), 1);
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
        fill: Rgba,
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
    // Frame membership -- the `children` writes the parent cache mirrors
    // (`TOILE-056`). A pick that is no frame answers no delta.
    FrameNth {
        frame: usize,
        nth: usize,
    },
    UnframeNth {
        nth: usize,
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
    // The advisory lock (`TOILE-033`): every later op on a locked element
    // must write nothing, on both peers alike.
    LockNth {
        nth: usize,
        locked: bool,
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
        (0usize..6, any::<u32>()).prop_map(|(nth, fill)| PeerOp::RestyleNth {
            nth,
            fill: Rgba::from_bits(fill)
        }),
        (0usize..6).prop_map(|nth| PeerOp::DeleteNth { nth }),
        (0usize..6).prop_map(|nth| PeerOp::ReorderNthToBack { nth }),
        (0usize..6, 0usize..6).prop_map(|(i, j)| PeerOp::GroupNth { i, j }),
        (0usize..6).prop_map(|nth| PeerOp::UngroupNth { nth }),
        (0usize..6, 0usize..6).prop_map(|(i, j)| PeerOp::ConnectNth { i, j }),
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::Stroke { x, y }),
        (-200.0f64..200.0, -200.0f64..200.0).prop_map(|(x, y)| PeerOp::CreateFrame { x, y }),
        (0usize..6).prop_map(|by| PeerOp::SetSlideOrderRot { by }),
        (0usize..6, 0usize..6).prop_map(|(frame, nth)| PeerOp::FrameNth { frame, nth }),
        (0usize..6).prop_map(|nth| PeerOp::UnframeNth { nth }),
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
        (0usize..6, any::<bool>()).prop_map(|(nth, locked)| PeerOp::LockNth { nth, locked }),
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
fn apply_local(st: &mut ToileState, op: &PeerOp) -> Option<ambre_core::DeltaBytes> {
    let ids = live_ids(st);
    let nth = |n: usize| ids.get(n % ids.len().max(1)).cloned();
    let out = match op {
        PeerOp::Create { x, y } => st.handle_command(ToileCommand::CreateElement {
            kind: ElementKind::Shape(ShapeKind::Rect),
            at: Point::new(*x, *y),
            size: Size::new(20.0, 20.0),
            content: InitContent::None,
            fill: None,
            correlate: None,
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
                radians: ambre_core::Radians::new(*radians),
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
            Some(id) => st.handle_command(ToileCommand::UngroupElements { groups: vec![id] }),
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
                    start: ambre_bus::app::toile::Cap::Plain,
                    end: ambre_bus::app::toile::Cap::Arrow,
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
            correlate: None,
        }),
        PeerOp::FrameNth { frame, nth: n } => match (nth(*frame), nth(*n)) {
            (Some(f), Some(id)) => st.handle_command(ToileCommand::SetFrameChildren {
                frame: f,
                add: vec![id],
                remove: vec![],
            }),
            _ => return None,
        },
        PeerOp::UnframeNth { nth: n } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::RemoveFromFrames { ids: vec![id] }),
            None => return None,
        },
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
        PeerOp::LockNth { nth: n, locked } => match nth(*n) {
            Some(id) => st.handle_command(ToileCommand::LockElements {
                ids: vec![id],
                lock: ambre_bus::app::toile::LockState::locked_if(*locked),
            }),
            None => return None,
        },
        PeerOp::Stroke { x, y } => st.handle_command(ToileCommand::CommitStroke {
            points: vec![
                StrokePoint::full_pressure(
                    ambre_core::StrokeCoord::new(*x as f32),
                    ambre_core::StrokeCoord::new(*y as f32),
                ),
                StrokePoint::full_pressure(
                    ambre_core::StrokeCoord::new(*x as f32 + 10.0),
                    ambre_core::StrokeCoord::new(*y as f32 + 5.0),
                ),
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
        zoom: ambre_core::ZoomFactor::positive(0.01),
        size: Size::new(2000.0, 2000.0),
    }
}

/// Two peers pinned to `peers`, which drive `ops_a` and `ops_b` in rounds. Each
/// round applies one op per peer locally, then hands each delta to the other
/// peer. Operations therefore target a partly-shared state. A final exchange of
/// snapshots settles a straggler. An op picks its target by rank among ids
/// that embed the peer. A found case therefore replays only under the same two
/// ids, so the two ids are pinned.
fn interleave(
    peers: (ambre_core::PeerId, ambre_core::PeerId),
    ops_a: &[PeerOp],
    ops_b: &[PeerOp],
) -> (ToileState, ToileState) {
    let mut a = ToileState::with_peer(peers.0);
    let mut b = ToileState::with_peer(peers.1);
    // Distinct canonical identities, as `SessionReady` would seed them. A
    // `ReactNth` on each peer then writes under its own key, and concurrent
    // reactions on one target must both survive. They are pinned too, so a
    // found case replays with the same reaction keys.
    a.set_user_id(UserId::from_bytes([0xA1; 16]));
    b.set_user_id(UserId::from_bytes([0xB2; 16]));
    let rounds = ops_a.len().max(ops_b.len());
    for i in 0..rounds {
        let da = ops_a.get(i).and_then(|op| apply_local(&mut a, op));
        let db = ops_b.get(i).and_then(|op| apply_local(&mut b, op));
        if let Some(d) = da {
            b.apply_remote(&d);
        }
        if let Some(d) = db {
            a.apply_remote(&d);
        }
    }
    for _ in 0..2 {
        let da = a.export_snapshot();
        let db = b.export_snapshot();
        b.apply_remote_snapshot(&da);
        a.apply_remote_snapshot(&db);
    }
    (a, b)
}

/// The derived caches a fresh engine rebuilds from `st`'s document.
fn rebuilt_caches(st: &ToileState) -> String {
    let mut fresh = ToileState::new();
    fresh
        .import_snapshot(&st.export_snapshot())
        .expect("a board this build wrote");
    fresh.debug_caches()
}

/// `TOILE-017`, `TOILE-271`: the history `two_peers_converge` found. Peer A frames
/// B's stroke, then deletes it while B concurrently sends it to the back. The
/// delete left the stroke on the frame's list. A's parent cache forgot the
/// link, while B and a rebuild kept it, so B still named the stroke framed. The
/// two ids are pinned in the order that put the stroke under both picks.
#[test]
fn a_delete_concurrent_with_a_reorder_leaves_no_frame_member_behind() {
    let ops_a = [
        PeerOp::FrameNth { frame: 0, nth: 3 },
        PeerOp::ReactNth {
            nth: 1,
            emoji: false,
            weight: -2,
        },
        PeerOp::CreateFrame { x: 190.8, y: -13.9 },
        PeerOp::FrameNth { frame: 3, nth: 2 },
        PeerOp::UngroupNth { nth: 4 },
        PeerOp::SetSlideOrderRot { by: 1 },
        PeerOp::DeleteNth { nth: 3 },
    ];
    let ops_b = [
        PeerOp::MoveNth {
            nth: 1,
            dx: -13.5,
            dy: 4.6,
        },
        PeerOp::MoveNth {
            nth: 1,
            dx: 16.5,
            dy: -22.5,
        },
        PeerOp::Stroke {
            x: -100.9,
            y: -177.5,
        },
        PeerOp::ConnectNth { i: 5, j: 1 },
        PeerOp::SetAutoLayoutNth {
            nth: 1,
            dir: 1,
            gap: 6.7,
        },
        PeerOp::Create { x: 34.0, y: 126.7 },
        PeerOp::ReorderNthToBack { nth: 3 },
        PeerOp::SetSlideOrderRot { by: 5 },
        PeerOp::ConnectNth { i: 0, j: 4 },
        PeerOp::CreateFrame { x: 124.9, y: 138.9 },
        PeerOp::Stroke { x: 112.0, y: 184.4 },
        PeerOp::UngroupNth { nth: 4 },
    ];
    let peers = (ambre_core::PeerId(PEER_A), ambre_core::PeerId(PEER_B));
    let (mut a, mut b) = interleave(peers, &ops_a, &ops_b);
    let live = live_ids(&a);
    assert_eq!(
        framed_of(&mut a, live.clone()),
        framed_of(&mut b, live),
        "frame membership diverged"
    );
    assert_eq!(a.debug_caches(), rebuilt_caches(&a), "peer A");
    assert_eq!(b.debug_caches(), rebuilt_caches(&b), "peer B");
}

/// Two peers that each add one rect to one frame, concurrently, leave the id
/// on the frame's list twice. A remove then takes every entry off, so the rect
/// leaves the frame, and each peer's caches equal a rebuild.
#[test]
fn a_remove_takes_every_entry_a_concurrent_add_left() {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let rect = create_rect(&mut a, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let add = |st: &mut ToileState| {
        st.handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: vec![rect.clone()],
            remove: vec![],
        })
        .update
        .expect("a delta")
    };
    let from_a = add(&mut a);
    let from_b = add(&mut b);
    a.apply_remote(&from_b);
    b.apply_remote(&from_a);
    assert_eq!(
        a.membership_of(&frame),
        vec![rect.clone(), rect.clone()],
        "the merge holds both adds"
    );
    let removed = a
        .handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: vec![],
            remove: vec![rect.clone()],
        })
        .update
        .expect("a delta");
    b.apply_remote(&removed);
    for (name, st) in [("remover", &mut a), ("receiver", &mut b)] {
        assert_eq!(st.membership_of(&frame), Vec::<ElementId>::new(), "{name}");
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// An undo of a delete puts the id back on its frame's list. A peer that
/// took the member out and in again meanwhile left one entry of its own.
/// The list then names the member twice, and each peer's caches still
/// equal a rebuild of its own document. One remove then takes both entries.
#[test]
fn an_undone_delete_after_a_concurrent_re_add_keeps_the_caches_whole() {
    let (mut a, mut b, frame, rect) = a_framed_rect_on_two_peers();
    let deleted = a
        .handle_command(ToileCommand::DeleteElements {
            ids: vec![rect.clone()],
        })
        .update
        .expect("a delta");
    let re_added = b
        .handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: vec![],
            remove: vec![rect.clone()],
        })
        .update
        .into_iter()
        .chain(
            b.handle_command(ToileCommand::SetFrameChildren {
                frame: frame.clone(),
                add: vec![rect.clone()],
                remove: vec![],
            })
            .update,
        )
        .collect::<Vec<_>>();
    for delta in &re_added {
        a.apply_remote(delta);
    }
    b.apply_remote(&deleted);
    let undone = a
        .handle_command(ToileCommand::Undo)
        .update
        .expect("a delta");
    b.apply_remote(&undone);
    for (name, st) in [("undoer", &mut a), ("peer", &mut b)] {
        assert_eq!(
            st.membership_of(&frame),
            vec![rect.clone(), rect.clone()],
            "{name}"
        );
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
    let removed = b
        .handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: vec![],
            remove: vec![rect.clone()],
        })
        .update
        .expect("a delta");
    a.apply_remote(&removed);
    for (name, st) in [("undoer", &mut a), ("peer", &mut b)] {
        assert_eq!(st.membership_of(&frame), Vec::<ElementId>::new(), "{name}");
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// Two peers holding one frame and one rect, the rect a member of the frame.
fn a_framed_rect_on_two_peers() -> (ToileState, ToileState, ElementId, ElementId) {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let rect = create_rect(&mut a, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    a.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![rect.clone()],
        remove: vec![],
    });
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    (a, b, frame, rect)
}

/// `TOILE-271`: deleting a framed element takes its id off the frame's list, on
/// the peer that deleted and on the peer that received the delta. Each peer's
/// caches then equal a rebuild of its own document.
#[test]
fn a_delete_takes_the_id_off_its_frame_list() {
    let (mut a, mut b, frame, rect) = a_framed_rect_on_two_peers();
    let delta = a
        .handle_command(ToileCommand::DeleteElements {
            ids: vec![rect.clone()],
        })
        .update
        .expect("a delta");
    b.apply_remote(&delta);
    for (name, st) in [("deleter", &mut a), ("receiver", &mut b)] {
        assert_eq!(st.membership_of(&frame), Vec::<ElementId>::new(), "{name}");
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// `TOILE-017`, found by `caches_match_a_full_rebuild`: a deleted frame keeps
/// its member on the board. The deleter's parent cache kept the link the
/// frame's list held, where the receiver and a rebuild dropped it.
#[test]
fn deleting_a_frame_drops_the_links_its_list_held() {
    let (mut a, mut b, frame, rect) = a_framed_rect_on_two_peers();
    let delta = a
        .handle_command(ToileCommand::DeleteElements {
            ids: vec![frame.clone()],
        })
        .update
        .expect("a delta");
    b.apply_remote(&delta);
    for (name, st) in [("deleter", &mut a), ("receiver", &mut b)] {
        assert!(st.bounds_of(&rect).is_some(), "{name}: the member stays");
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// `TOILE-271`, found by `two_peers_converge`: deleting both members of a group
/// a frame holds prunes the group, and the frame's list drops the group too.
#[test]
fn a_pruned_group_leaves_its_frame_list() {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let x = create_rect(&mut a, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    let y = create_rect(&mut a, Point::new(50.0, 10.0), Size::new(20.0, 20.0));
    a.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![x.clone(), y.clone()],
        remove: vec![],
    });
    let group = created_id(&a.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), y.clone()],
    }));
    assert_eq!(
        a.membership_of(&frame),
        vec![group],
        "the group took their place"
    );
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let delta = a
        .handle_command(ToileCommand::DeleteElements { ids: vec![x, y] })
        .update
        .expect("a delta");
    b.apply_remote(&delta);
    for (name, st) in [("deleter", &mut a), ("receiver", &mut b)] {
        assert_eq!(st.membership_of(&frame), Vec::<ElementId>::new(), "{name}");
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// `TOILE-157`, found by `two_peers_converge`: ungrouping a group a
/// frame holds hands its children to the frame. The frame's list kept the dead
/// group and lost the children, and the caches disagreed with a rebuild.
#[test]
fn ungrouping_a_framed_group_hands_its_children_to_the_frame() {
    let mut a = ToileState::new();
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let x = create_rect(&mut a, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    let y = create_rect(&mut a, Point::new(50.0, 10.0), Size::new(20.0, 20.0));
    a.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![x.clone(), y.clone()],
        remove: vec![],
    });
    let group = created_id(&a.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), y.clone()],
    }));
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let delta = a
        .handle_command(ToileCommand::UngroupElements {
            groups: vec![group],
        })
        .update
        .expect("a delta");
    b.apply_remote(&delta);
    for (name, st) in [("ungrouper", &mut a), ("receiver", &mut b)] {
        assert_eq!(
            st.membership_of(&frame),
            vec![x.clone(), y.clone()],
            "{name}"
        );
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// `TOILE-011`: the clamp lifts a framed group alone, so its children still
/// paint below the frame. When the group goes, the frame takes them, and they
/// must then paint above it on both peers. The clamp lands in the ungroup's
/// commit, so one undo restores the group, its frame link and the paint order.
#[test]
fn an_ungroup_into_a_frame_lifts_the_children_above_it() {
    let mut a = ToileState::new();
    let x = create_rect(&mut a, Point::new(10.0, 10.0), Size::new(20.0, 20.0));
    let y = create_rect(&mut a, Point::new(50.0, 10.0), Size::new(20.0, 20.0));
    let frame = create_frame(&mut a, Point::ZERO, Size::new(200.0, 200.0));
    let group = created_id(&a.handle_command(ToileCommand::GroupElements {
        ids: vec![x.clone(), y.clone()],
    }));
    a.handle_command(ToileCommand::SetFrameChildren {
        frame: frame.clone(),
        add: vec![group.clone()],
        remove: vec![],
    });
    let before = a.layer_order().to_vec();
    let mut b = ToileState::new();
    b.apply_remote_snapshot(&a.export_snapshot());
    let delta = a
        .handle_command(ToileCommand::UngroupElements {
            groups: vec![group.clone()],
        })
        .update
        .expect("a delta");
    b.apply_remote(&delta);
    for (name, st) in [("ungrouper", &a), ("receiver", &b)] {
        let order = st.layer_order();
        let idx = |id: &ElementId| order.iter().position(|n| n == &NodeId::Element(id.clone()));
        assert_eq!(
            st.membership_of(&frame),
            vec![x.clone(), y.clone()],
            "{name}"
        );
        assert!(
            idx(&frame) < idx(&x),
            "{name}: x under its frame in {order:?}"
        );
        assert!(
            idx(&frame) < idx(&y),
            "{name}: y under its frame in {order:?}"
        );
    }
    a.handle_command(ToileCommand::Undo);
    assert_eq!(a.membership_of(&frame), vec![group.clone()]);
    assert_eq!(a.membership_of(&group), vec![x, y]);
    assert_eq!(a.layer_order(), before);
    assert_eq!(a.debug_caches(), rebuilt_caches(&a));
}

/// `TOILE-272`: B frames an element while A deletes it, so the merged list
/// names an element neither peer holds. A board an earlier build stored holds
/// the same entry, since its delete left every frame list alone. Each peer, and
/// a fresh load of the stored bytes, names no member and keeps caches equal to
/// a rebuild.
#[test]
fn a_frame_entry_for_a_deleted_element_names_no_member() {
    let (mut a, mut b, frame, _) = a_framed_rect_on_two_peers();
    let other = create_rect(&mut a, Point::new(50.0, 50.0), Size::new(20.0, 20.0));
    b.apply_remote_snapshot(&a.export_snapshot());
    let da = a
        .handle_command(ToileCommand::DeleteElements {
            ids: vec![other.clone()],
        })
        .update
        .expect("a delta");
    let db = b
        .handle_command(ToileCommand::SetFrameChildren {
            frame: frame.clone(),
            add: vec![other.clone()],
            remove: vec![],
        })
        .update
        .expect("a delta");
    b.apply_remote(&da);
    a.apply_remote(&db);
    let mut stored = ToileState::new();
    stored
        .import_snapshot(&a.export_snapshot())
        .expect("a board with a dangling member loads");
    for (name, st) in [
        ("deleter", &mut a),
        ("framer", &mut b),
        ("stored", &mut stored),
    ] {
        assert!(
            st.membership_of(&frame).contains(&other),
            "{name}: the entry"
        );
        assert_eq!(
            framed_of(st, vec![other.clone()]),
            Vec::<ElementId>::new(),
            "{name}"
        );
        assert_eq!(st.debug_caches(), rebuilt_caches(st), "{name}");
    }
}

/// The two peer ids the regression above replays under.
const PEER_A: u64 = 2;
/// See [`PEER_A`].
const PEER_B: u64 = 1;

/// The pair of peer ids a case runs under, both orders drawn, so a found case
/// carries its order and replays.
fn peers_strategy() -> impl Strategy<Value = (ambre_core::PeerId, ambre_core::PeerId)> {
    let (a, b) = (ambre_core::PeerId(PEER_A), ambre_core::PeerId(PEER_B));
    prop_oneof![Just((a, b)), Just((b, a))]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn two_peers_converge(ops_a in proptest::collection::vec(op_strategy(), 0..20),
                          ops_b in proptest::collection::vec(op_strategy(), 0..20),
                          peers in peers_strategy()) {
        let (mut a, mut b) = interleave(peers, &ops_a, &ops_b);

        // Each peer's incremental caches equal a rebuild of its own document.
        prop_assert_eq!(a.debug_caches(), rebuilt_caches(&a), "peer A diverged from a rebuild");
        prop_assert_eq!(b.debug_caches(), rebuilt_caches(&b), "peer B diverged from a rebuild");

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
        // No write, local or merged, lists a node twice (`TOILE-060`).
        for st in [&a, &b] {
            let raw = st.debug_raw_layers();
            let distinct: std::collections::HashSet<&String> = raw.iter().collect();
            prop_assert_eq!(distinct.len(), raw.len(), "a node is listed twice: {:?}", raw);
        }

        // Identical frame membership, read from each peer's parent cache.
        prop_assert_eq!(
            framed_of(&mut a, la.clone()),
            framed_of(&mut b, la.clone()),
            "frame membership diverged"
        );

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

    /// `TOILE-092`: a peer that receives every change as a delta refreshes only
    /// what each delta touched. It still holds the caches a full rebuild of the
    /// same document gives.
    #[test]
    fn a_remote_refresh_matches_a_full_rebuild(
        ops in proptest::collection::vec(op_strategy(), 0..40)
    ) {
        let mut a = ToileState::new();
        a.set_user_id(UserId::new());
        let mut b = ToileState::new();
        b.apply_remote_snapshot(&a.export_snapshot());
        for op in &ops {
            if let Some(d) = apply_local(&mut a, op) {
                b.apply_remote(&d);
            }
        }
        let mut c = ToileState::new();
        c.import_snapshot(&b.export_snapshot()).expect("a board this build wrote");
        prop_assert_eq!(b.debug_caches(), c.debug_caches(), "the refresh diverged");
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
        b.apply_remote_snapshot(&a.export_snapshot());

        a.handle_command(ToileCommand::SetViewport(wide_viewport()));
        b.handle_command(ToileCommand::SetViewport(wide_viewport()));

        // Every cache the engine keeps, the parent cache included.
        prop_assert_eq!(a.debug_caches(), b.debug_caches(), "caches diverged from a rebuild");
        prop_assert_eq!(a.element_count(), b.element_count(), "element count diverged from rebuild");
        prop_assert_eq!(a.layer_order(), b.layer_order(), "layers diverged from rebuild");
        // The parent cache: the group each painted element names is in the
        // paint set below, and frame membership is read here.
        let live = live_ids(&a);
        prop_assert_eq!(
            framed_of(&mut a, live.clone()),
            framed_of(&mut b, live),
            "frame membership diverged from rebuild"
        );
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

/// Peer A and peer B restyle the same element concurrently, then exchange
/// their whole histories. `SCHEMA-001`: the element's style is one binary
/// register keyed by the element, so the two writes are LWW rivals. The style
/// they converge on must be one of the two written, never a field-by-field
/// blend, which is what a register per attribute would produce.
///
/// Each peer's intent is read back rather than assumed. The whole-selection
/// restyle keeps the element's own corner radius, shadow and weight, so the
/// register holds less than the command carried.
#[test]
fn concurrent_whole_element_restyles_do_not_blend_across_registers() {
    // Shared base: one element, its history known to both peers.
    let mut a = ToileState::with_peer(ambre_core::PeerId(1));
    let id = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let base = a.export_snapshot();
    let mut b = ToileState::with_peer(ambre_core::PeerId(2));
    b.import_snapshot(&base)
        .expect("a snapshot this build wrote reads back at the current schema");

    // Two whole-style writes that differ in every field a blend could mix.
    a.handle_command(ToileCommand::RestyleElements {
        ids: vec![id.clone()],
        style: ElementStyle {
            fill: Some(Rgba::from_bits(0xFF00_00FF)),
            stroke: None,
            stroke_w: ambre_bus::app::toile::StyleLength::new(4.0),
            opacity: ambre_core::Opacity::new(0.5).unwrap(),
            text_color: Some(Rgba::from_bits(0x00FF_00FF)),
            ..ElementStyle::default()
        },
    });
    b.handle_command(ToileCommand::RestyleElements {
        ids: vec![id.clone()],
        style: ElementStyle {
            fill: None,
            stroke: Some(Rgba::from_bits(0x0000_FFFF)),
            stroke_w: ambre_bus::app::toile::StyleLength::new(1.5),
            opacity: ambre_core::Opacity::new(0.25).unwrap(),
            text_color: None,
            ..ElementStyle::default()
        },
    });
    let a_intent = a.style_of(&id);
    let b_intent = b.style_of(&id);
    assert_ne!(a_intent, b_intent, "the two edits must actually differ");

    // Full bidirectional exchange.
    let sa = a.export_snapshot();
    let sb = b.export_snapshot();
    a.apply_remote_snapshot(&sb);
    b.apply_remote_snapshot(&sa);

    let conv_a = a.style_of(&id);
    let conv_b = b.style_of(&id);
    assert_eq!(conv_a, conv_b, "peers diverged after full history exchange");
    assert!(
        conv_a == a_intent || conv_a == b_intent,
        "converged style is a blend neither peer wrote:\n\
         converged: {conv_a:?}\n\
         peer A wrote: {a_intent:?}\n\
         peer B wrote: {b_intent:?}"
    );
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
    let out = st.handle_command(ToileCommand::UngroupElements { groups: vec![a] });
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

/// `TOILE-214`: a remote change that cannot be imported reports a distinct
/// import failure, never a `NoSuchElement` with an empty id.
#[test]
fn a_remote_change_that_cannot_import_reports_an_import_failure() {
    let mut st = ToileState::new();
    let events = st.apply_remote(&b"not a loro delta".to_vec().into());
    assert!(matches!(
        events.as_slice(),
        [ToileEvent::Error(ToileError::RemoteImportFailed)]
    ));
}

/// `TOILE-215`: one failed delta of a batch reports the failure while the
/// surviving deltas still merge.
#[test]
fn a_batch_with_one_failed_delta_reports_the_failure() {
    let mut src = ToileState::new();
    let out = src.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::ZERO,
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: None,
        correlate: None,
    });
    let good = out.update.expect("a create carries a delta");

    let mut st = ToileState::new();
    let events = st.apply_remote_batch(&[good, b"garbage".to_vec().into()]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ToileEvent::Error(ToileError::RemoteImportFailed)))
    );
    assert_eq!(
        st.element_count(),
        ElementCount::new(1),
        "the surviving delta still merges"
    );
}

// -----------------------------------------------------------------------------
// Connectors and paths held (`toile-connectors`, `toile-vector-paths`,
// `toile-auto-layout`, `toile-freehand`)
// -----------------------------------------------------------------------------

/// The paint position of one node, or a panic when it is not in the order.
fn order_of(st: &ToileState, node: &NodeId) -> usize {
    st.layer_order()
        .iter()
        .position(|n| n == node)
        .expect("the node is in the paint order")
}

/// `TOILE-007`: a peer deleting an endpoint while another joins it leaves
/// the arrow dangling on both boards, its record whole on each.
#[test]
fn an_arrow_whose_end_a_peer_deleted_keeps_its_record() {
    let mut a = ToileState::with_peer(ambre_core::PeerId(1));
    let from = create_rect(&mut a, Point::ZERO, Size::new(10.0, 10.0));
    let to = create_rect(&mut a, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let mut b = ToileState::with_peer(ambre_core::PeerId(2));
    b.apply_remote_snapshot(&a.export_snapshot());
    let conn = connect(&mut a, from.clone(), to.clone());
    let joined = a.connector_of(&conn).expect("the arrow is stored");
    let gone = b.handle_command(ToileCommand::DeleteElements {
        ids: vec![to.clone()],
    });
    a.apply_remote(&gone.update.expect("a delete writes"));
    b.apply_remote_snapshot(&a.export_snapshot());
    for peer in [&a, &b] {
        assert_eq!(peer.connector_ids(), vec![conn.clone()], "not destroyed");
        assert_eq!(peer.connector_of(&conn).as_ref(), Some(&joined));
        assert_eq!(connector_node_count(peer), 0, "dangling, so not painted");
    }
}

/// `TOILE-043`: an arrow sits in the one paint order, so a reorder can put
/// it behind the elements it joins.
#[test]
fn an_arrow_goes_behind_an_element() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let conn = connect(&mut st, a.clone(), b.clone());
    st.handle_command(ToileCommand::Reorder {
        node: NodeId::Connector(conn.clone()),
        to: ZTarget::Back,
    });
    assert_eq!(
        st.layer_order(),
        &[
            NodeId::Connector(conn),
            NodeId::Element(a),
            NodeId::Element(b)
        ]
    );
}

/// `TOILE-168`: a create naming an element the board does not hold is
/// refused, and writes nothing.
#[test]
fn a_create_to_a_missing_element_is_refused() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let order = st.layer_order().to_vec();
    let ghost = ElementId("ffff_0000".to_owned());
    for (from, to) in [(a.clone(), ghost.clone()), (ghost.clone(), a.clone())] {
        let out = st.handle_command(ToileCommand::CreateConnector {
            from,
            from_anch: Anchor::Right,
            to,
            to_anch: Anchor::Left,
            path_type: PathType::Straight,
            arrows: EndCaps {
                start: ambre_bus::app::toile::Cap::Plain,
                end: ambre_bus::app::toile::Cap::Arrow,
            },
        });
        assert!(out.update.is_none(), "a refused create writes nothing");
        assert!(matches!(
            out.display.as_slice(),
            [ToileEvent::Error(ToileError::NoSuchElement(id))] if id == &ghost
        ));
    }
    assert!(st.connector_ids().is_empty());
    assert_eq!(st.layer_order(), order.as_slice());
}

/// `TOILE-169`: a create puts the arrow at the top of the paint order.
#[test]
fn a_new_arrow_lands_on_top() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    create_rect(&mut st, Point::new(200.0, 0.0), Size::new(10.0, 10.0));
    let conn = connect(&mut st, a, b);
    assert_eq!(st.layer_order().last(), Some(&NodeId::Connector(conn)));
    assert_eq!(st.layer_order().len(), 4);
}

/// `TOILE-171`, `TOILE-238`, `TOILE-173`: a re-aim moves the named end and
/// nothing else. The other end, the path kind, the caps and the paint
/// position read back as they were.
#[test]
fn a_re_aim_moves_only_the_named_end() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(100.0, 0.0), Size::new(10.0, 10.0));
    let c = create_rect(&mut st, Point::new(0.0, 100.0), Size::new(10.0, 10.0));
    let conn = connect(&mut st, a, b);
    let node = NodeId::Connector(conn.clone());
    let before = st.connector_of(&conn).expect("stored");
    let at = order_of(&st, &node);
    st.handle_command(ToileCommand::RetargetConnector {
        id: conn.clone(),
        endpoint: End::To,
        to: c.clone(),
        anch: Anchor::Top,
    });
    let after = st.connector_of(&conn).expect("stored");
    assert_eq!((after.to.clone(), after.to_anch), (c, Anchor::Top));
    assert_eq!(
        (after.from.clone(), after.from_anch),
        (before.from.clone(), before.from_anch),
        "the other end stays"
    );
    assert_eq!(after.path_type, before.path_type);
    assert_eq!(after.arrows, before.arrows);
    assert_eq!(order_of(&st, &node), at, "the paint position stays");
}

/// `TOILE-174`: re-aiming an arrow nobody minted answers an error and
/// leaves the document as it was.
#[test]
fn re_aiming_an_unknown_arrow_changes_nothing() {
    let mut st = ToileState::new();
    let a = create_rect(&mut st, Point::ZERO, Size::new(10.0, 10.0));
    let before = st.crdt().oplog_vv();
    let out = st.handle_command(ToileCommand::RetargetConnector {
        id: ConnectorId("ffff_0000".to_owned()),
        endpoint: End::From,
        to: a,
        anch: Anchor::Left,
    });
    assert!(out.update.is_none());
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::NoSuchConnector(id))] if id.as_str() == "ffff_0000"
    ));
    assert_eq!(st.crdt().oplog_vv(), before, "nothing was written");
}

/// `TOILE-UI-348`: a caption write changes the caption alone. The ends, the
/// path kind, the caps and the paint position read back as they were.
#[test]
fn a_caption_changes_nothing_else_on_the_arrow() {
    let mut st = ToileState::new();
    let conn = create_connector(&mut st);
    let node = NodeId::Connector(conn.clone());
    let before = st.connector_of(&conn).expect("stored");
    let at = order_of(&st, &node);
    st.handle_command(ToileCommand::SetConnectorLabel {
        id: conn.clone(),
        base: String::new(),
        text: "yes".to_owned(),
    });
    assert_eq!(caption(&st, &conn), "yes");
    assert_eq!(st.connector_of(&conn), Some(before));
    assert_eq!(order_of(&st, &node), at);
}

/// The stored value of one element, as the document holds it.
fn stored(st: &ToileState, id: &ElementId) -> loro::LoroValue {
    st.crdt()
        .get_map(crate::logic::state::Root::Elements.as_str())
        .get_deep_value()
        .into_map()
        .ok()
        .and_then(|m| m.get(id.as_str()).cloned())
        .expect("the element is stored")
}

/// `TOILE-FEAT-030`: setting a rule writes the rule and no solved box. The
/// children's stored boxes, and every other field of the frame, read back
/// as they were, after a derivation too.
#[test]
fn a_layout_rule_writes_no_solved_box() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    let a = create_rect(&mut st, Point::new(50.0, 70.0), Size::new(10.0, 10.0));
    let b = create_rect(&mut st, Point::new(120.0, 20.0), Size::new(10.0, 10.0));
    st.handle_command(ToileCommand::SetFrameChildren {
        frame: f.clone(),
        add: vec![a.clone(), b.clone()],
        remove: vec![],
    });
    let (sa, sb) = (stored(&st, &a), stored(&st, &b));
    let frame_before = stored(&st, &f).into_map().expect("a map");
    st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: Some(a_layout(Direction::Horizontal, 8.0)),
    });
    st.debug_viewport();
    assert_eq!(stored(&st, &a), sa);
    assert_eq!(stored(&st, &b), sb);
    let mut frame_after = stored(&st, &f).into_map().expect("a map").as_ref().clone();
    assert!(
        frame_after.remove("autolayout").is_some(),
        "the rule is stored"
    );
    assert_eq!(
        &frame_after,
        frame_before.as_ref(),
        "nothing else on the frame"
    );
}

/// `TOILE-FEAT-052`: a rule identical to the stored one writes nothing and
/// sends nothing.
#[test]
fn an_unchanged_layout_rule_writes_nothing() {
    let mut st = ToileState::new();
    let f = create_frame(&mut st, Point::ZERO, Size::new(200.0, 200.0));
    st.handle_command(ToileCommand::SetAutoLayout {
        frame: f.clone(),
        layout: Some(a_layout(Direction::Vertical, 6.0)),
    });
    let before = st.crdt().oplog_vv();
    let out = st.handle_command(ToileCommand::SetAutoLayout {
        frame: f,
        layout: Some(a_layout(Direction::Vertical, 6.0)),
    });
    assert!(out.update.is_none(), "nothing reaches the other users");
    assert_eq!(st.crdt().oplog_vv(), before, "nothing was written");
}

fn filled_rect(st: &mut ToileState, x: f64, fill: Rgba) -> ElementId {
    created_id(&st.handle_command(ToileCommand::CreateElement {
        kind: ElementKind::Shape(ShapeKind::Rect),
        at: Point::new(x, 0.0),
        size: Size::new(10.0, 10.0),
        content: InitContent::None,
        fill: Some(fill),
        correlate: None,
    }))
}

/// `TOILE-FEAT-034`: the result carries the first source's style, whatever
/// order the board holds them in.
#[test]
fn a_combined_path_wears_the_first_sources_style() {
    let mut st = ToileState::new();
    let red = filled_rect(&mut st, 0.0, Rgba::from_bits(0xFF00_00FF));
    let blue = filled_rect(&mut st, 5.0, Rgba::from_bits(0x0000_FFFF));
    let out = st.handle_command(ToileCommand::BooleanOp {
        op: BoolOp::Union,
        ids: vec![blue.clone(), red],
        sources: Sources::Consume,
    });
    let result = created_id(&out);
    assert_eq!(
        st.style_of(&result).fill,
        Some(Rgba::from_bits(0x0000_FFFF))
    );
}

/// `TOILE-FEAT-035`: a locked element and a non-shape among the ids are
/// ignored, not consumed. The two shapes are.
#[test]
fn a_combine_consumes_only_its_sources() {
    let mut st = ToileState::new();
    let one = filled_rect(&mut st, 0.0, Rgba::from_bits(0xFF00_00FF));
    let two = filled_rect(&mut st, 5.0, Rgba::from_bits(0xFF00_00FF));
    let locked = filled_rect(&mut st, 3.0, Rgba::from_bits(0xFF00_00FF));
    st.handle_command(ToileCommand::LockElements {
        ids: vec![locked.clone()],
        lock: ambre_bus::app::toile::LockState::Locked,
    });
    let note = create_sticky(&mut st, Point::new(2.0, 2.0), "note");
    st.handle_command(ToileCommand::BooleanOp {
        op: BoolOp::Union,
        ids: vec![locked.clone(), note.clone(), one.clone(), two.clone()],
        sources: Sources::Consume,
    });
    assert!(st.bounds_of(&locked).is_some(), "the locked shape stays");
    assert!(st.bounds_of(&note).is_some(), "the note stays");
    assert!(st.bounds_of(&one).is_none() && st.bounds_of(&two).is_none());
    assert_eq!(st.element_count(), ElementCount::new(3));
}

/// `TOILE-FEAT-037`: an empty result is refused, and nothing is written.
#[test]
fn an_empty_combine_is_refused() {
    let mut st = ToileState::new();
    let a = filled_rect(&mut st, 0.0, Rgba::from_bits(0xFF00_00FF));
    let b = filled_rect(&mut st, 100.0, Rgba::from_bits(0xFF00_00FF));
    let before = st.crdt().oplog_vv();
    let out = st.handle_command(ToileCommand::BooleanOp {
        op: BoolOp::Intersect,
        ids: vec![a, b],
        sources: Sources::Consume,
    });
    assert!(matches!(
        out.display.as_slice(),
        [ToileEvent::Error(ToileError::BooleanEmpty)]
    ));
    assert!(out.update.is_none());
    assert_eq!(st.crdt().oplog_vv(), before);
    assert_eq!(st.element_count(), ElementCount::new(2));
}

fn sample(x: f32, y: f32) -> StrokePoint {
    StrokePoint::full_pressure(
        ambre_core::StrokeCoord::new(x),
        ambre_core::StrokeCoord::new(y),
    )
}

/// `TOILE-041`: a stroke's box runs from its lowest to its highest sample on
/// both axes.
#[test]
fn a_stroke_box_spans_its_samples() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CommitStroke {
        points: vec![
            sample(10.0, 40.0),
            sample(-20.0, 5.0),
            sample(35.0, -15.0),
            sample(0.0, 60.0),
        ],
        style: ElementStyle::default(),
    });
    let id = created_id(&out);
    assert_eq!(st.bounds_of(&id), Some(Rect::new(-20.0, -15.0, 35.0, 60.0)));
}

/// `TOILE-107`: a one-sample stroke still has a box with area, so the cull
/// and the pick can find it.
#[test]
fn a_dot_has_a_box_with_area() {
    let mut st = ToileState::new();
    let out = st.handle_command(ToileCommand::CommitStroke {
        points: vec![sample(7.0, 9.0)],
        style: ElementStyle::default(),
    });
    let b = st.bounds_of(&created_id(&out)).expect("bounds");
    assert!(b.area() > 0.0, "the dot has no area: {b:?}");
    assert!(b.contains(Point::new(7.0, 9.0)));
}
