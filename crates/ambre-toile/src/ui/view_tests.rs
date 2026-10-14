//! The board view driven the way the host drives it. Input events go in and
//! the commands for the Logic thread come out. The Logic thread's answers are
//! fed back through the inbound channel. No DOM and no GPU: the text pipe is
//! detached, and a frame paints into a recording surface.

use super::*;
use ambre_bus::app::toile::Anchor;
use ambre_bus::conduit::Receiver;
use kurbo::Rect;

/// One board view and the far ends of its two channels.
struct Rig {
    view: ToileView,
    to_logic: Receiver<LogicCommand>,
    /// The Logic thread's side of the view's inbound channel.
    from_logic: ambre_bus::conduit::Sender<UiEvent>,
    /// What the view asked the shell to tell the user.
    notices: NoticePublisher,
}

impl Rig {
    fn new() -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(256);
        let (from_logic, rx) = ambre_bus::conduit::local(256);
        let notices = NoticePublisher::new();
        let view = ToileView::new(
            DocumentId::new(),
            tx,
            rx,
            1200.0,
            800.0,
            TextInput::detached(),
            false,
            PerfPublisher::new(),
            BusyPublisher::new(),
            notices.clone(),
            ThemeControl::default(),
        );
        Self {
            view,
            to_logic,
            from_logic,
            notices,
        }
    }

    /// A board showing one sticky per entry, in the frame and in the
    /// selection data alike, with the commands of the boot drained.
    fn with_stickies(stickies: &[(&str, Rect)]) -> Self {
        let mut rig = Self::new();
        rig.show(
            stickies
                .iter()
                .map(|(id, bounds)| DisplayNode::Element(sticky(id, *bounds)))
                .collect(),
        );
        rig.sent();
        rig
    }

    /// A fresh frame from the Logic thread holding these nodes and no other.
    fn show(&mut self, nodes: Vec<DisplayNode>) {
        self.answer(ToileEvent::ViewportData(DisplayViewport {
            version: 0,
            pins: Vec::new(),
            reactions: Default::default(),
            viewport: Viewport::default(),
            nodes,
        }));
    }

    fn type_text(&mut self, text: &str) {
        self.input(InputEvent::Text {
            text: text.to_owned(),
        });
    }

    fn key(&mut self, key: &str, ctrl: bool, shift: bool) -> bool {
        self.view.handle_input(&InputEvent::Key {
            key: key.to_owned(),
            ctrl,
            alt: false,
            shift,
            meta: false,
        })
    }

    fn input(&mut self, ev: InputEvent) -> bool {
        self.view.handle_input(&ev)
    }

    /// The screen point over the world point (x, y).
    fn screen(&self, x: f64, y: f64) -> (f64, f64) {
        let p = self.view.cam.world_to_screen() * kurbo::Point::new(x, y);
        (p.x, p.y)
    }

    fn press(&mut self, (x, y): (f64, f64)) {
        self.input(InputEvent::PointerDown {
            x,
            y,
            button: 0,
            shift: false,
            ctrl: false,
        });
    }

    fn release(&mut self, (x, y): (f64, f64)) {
        self.input(InputEvent::PointerUp {
            x,
            y,
            button: 0,
            ctrl: false,
        });
    }

    /// A press and a release at the world point (x, y), with no move between.
    fn click(&mut self, x: f64, y: f64) {
        let at = self.screen(x, y);
        self.press(at);
        self.release(at);
    }

    /// A drag from empty board between two world points.
    fn marquee(&mut self, from: (f64, f64), to: (f64, f64)) {
        let from = self.screen(from.0, from.1);
        let to = self.screen(to.0, to.1);
        self.press(from);
        self.input(InputEvent::PointerMove { x: to.0, y: to.1 });
        self.release(to);
    }

    fn double_click(&mut self, x: f64, y: f64) {
        let (x, y) = self.screen(x, y);
        self.input(InputEvent::DoubleClick { x, y });
    }

    /// Deliver a Logic-thread answer the way the host does: queued on the
    /// inbound channel, drained by the frame's `update`.
    fn answer(&mut self, ev: ToileEvent) {
        let sent = self.from_logic.try_send(UiEvent::Viewport {
            document_id: self.view.document_id,
            event: AppEvent::Toile(ev),
        });
        assert!(sent.is_ok(), "the inbound channel has room");
        self.view.update();
    }

    /// Plan one frame, where the view sends what a changed selection asks for.
    fn frame(&mut self) {
        self.view.plan_repaint(0.0, false);
    }

    /// Run one frame's paint into a recording surface. A deferred editor
    /// close lands here, as it does in the host's frame.
    fn paint(&mut self) -> ambre_draw::testing::RecordingSurface {
        let mut surface = ambre_draw::testing::RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut cache = TextCache::new(1.0);
        self.view
            .paint_content(&mut surface, &mut font_cx, &mut cache, None);
        self.view.paint_overlay(
            &mut surface,
            &mut font_cx,
            &mut cache,
            &FrameStats::default(),
        );
        surface
    }

    /// The board commands the view sent since the last call, in order.
    fn sent(&mut self) -> Vec<ToileCommand> {
        self.view.outbox.flush();
        self.to_logic
            .try_iter()
            .filter_map(|msg| match msg {
                LogicCommand::App {
                    command: AppCommand::Toile(cmd),
                    ..
                } => Some(cmd),
                _ => None,
            })
            .collect()
    }
}

fn eid(s: &str) -> ElementId {
    ElementId(s.to_owned())
}

fn sticky(id: &str, bounds: Rect) -> DisplayElement {
    DisplayElement {
        id: eid(id),
        kind: ElementKind::Sticky,
        transform: kurbo::Affine::IDENTITY,
        bounds,
        locked: false,
        style: Default::default(),
        geom: ambre_bus::app::toile::DisplayGeom::Text("hi".to_owned()),
        link: None,
        group: None,
    }
}

fn arrow(id: &str) -> DisplayConnector {
    let mut path = kurbo::BezPath::new();
    path.move_to((0.0, 300.0));
    path.line_to((400.0, 300.0));
    DisplayConnector {
        id: ConnectorId(id.to_owned()),
        path,
        style: Default::default(),
        arrows: ambre_bus::app::toile::EndCaps {
            start: false,
            end: true,
        },
        label: "yes".to_owned(),
    }
}

fn lost(error: ambre_bus::app::toile::ToileError) -> ToileEvent {
    ToileEvent::Error(error)
}

const A: Rect = Rect::new(0.0, 0.0, 200.0, 120.0);
const B: Rect = Rect::new(300.0, 0.0, 500.0, 120.0);

#[test]
fn a_click_selects_and_a_click_on_empty_board_clears() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a")]);
    rig.click(900.0, 600.0);
    assert!(rig.view.selection.is_empty());
}

#[test]
fn a_click_on_a_selected_element_keeps_the_whole_selection() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.marquee((-20.0, -20.0), (600.0, 300.0));
    assert_eq!(rig.view.selection, [eid("a"), eid("b")]);
    let on_a = rig.screen(50.0, 50.0);
    rig.press(on_a);
    assert_eq!(rig.view.selection, [eid("a"), eid("b")], "the press");
    rig.release(on_a);
    assert_eq!(rig.view.selection, [eid("a"), eid("b")], "the release");
}

#[test]
fn a_marquee_takes_what_it_covers() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.marquee((-20.0, -20.0), (250.0, 300.0));
    assert_eq!(rig.view.selection, [eid("a")]);
}

#[test]
fn an_untouched_editor_closes_without_a_write() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    assert!(rig.view.editor.is_some(), "the editor opened");
    rig.key("Escape", false, false);
    rig.paint();
    assert!(rig.view.editor.is_none(), "the editor closed");
    assert!(rig.sent().is_empty(), "and wrote nothing");
}

#[test]
fn a_double_click_keeps_the_body_and_a_close_writes_it_once() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    let opened = rig.view.editor.as_ref().map(|e| e.opened_text().to_owned());
    assert_eq!(opened.as_deref(), Some("hi"), "the body is kept");
    rig.input(InputEvent::Text {
        text: "!".to_owned(),
    });
    rig.input(InputEvent::Text {
        text: "?".to_owned(),
    });
    rig.key("Escape", false, false);
    rig.paint();
    assert!(rig.view.editor.is_none());
    // The caret sat at the end: the keystrokes extend the body, and the two
    // of them leave as one whole-body write.
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::SetText { id, base, text }]
            if id == &eid("a") && base == "hi" && text == "hi!?"
    ));
}

#[test]
fn an_editor_on_an_element_a_peer_deleted_closes_without_a_write_and_says_so() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.answer(ToileEvent::SelectionData {
        elements: vec![sticky("a", A)],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    rig.type_text("!");
    rig.paint();
    // The peer's delete: the frame loses the element, then the refetch
    // answers that the board no longer holds it.
    rig.show(Vec::new());
    rig.answer(ToileEvent::BoardChanged);
    rig.paint();
    assert!(rig.view.editor.is_some(), "the editor waits for the answer");
    rig.answer(ToileEvent::SelectionData {
        elements: Vec::new(),
        framed: Vec::new(),
        missing: vec![eid("a")],
    });
    rig.paint();
    assert!(rig.view.editor.is_none());
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::SetText { .. })),
        "no write names the lost element"
    );
    assert_eq!(rig.notices.take().len(), 1, "the user is told once");
}

#[test]
fn a_caption_editor_on_an_arrow_a_peer_deleted_closes_without_a_write_and_says_so() {
    let mut rig = Rig::new();
    rig.show(vec![DisplayNode::Connector(arrow("c"))]);
    rig.view.open_label_editor(ConnectorId("c".to_owned()));
    assert!(rig.view.editor.is_some());
    rig.type_text("!");
    rig.paint();
    rig.answer(ToileEvent::ArrowsGone(vec![ConnectorId("c".to_owned())]));
    rig.show(Vec::new());
    rig.paint();
    assert!(rig.view.editor.is_none());
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::SetConnectorLabel { .. })),
        "no write names the lost arrow"
    );
    assert_eq!(rig.notices.take().len(), 1, "the user is told once");
}

/// `TOILE-UI-390`: a title editor fetches its frame on its own, so a pan that
/// culls the frame keeps it open and writes no rename. The board's word that
/// the frame is gone closes it with no write and one notice.
#[test]
fn a_title_editor_outlives_a_pan_and_leaves_with_the_board() {
    let mut rig = Rig::new();
    let mut frame = sticky("f", A);
    frame.kind = ElementKind::Frame;
    frame.geom = ambre_bus::app::toile::DisplayGeom::Frame("Plan".to_owned());
    rig.show(vec![DisplayNode::Element(frame.clone())]);
    rig.sent();
    rig.view.open_frame_title_editor(eid("f"));
    assert!(
        rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::RequestSelectionData { ids } if ids == &[eid("f")])),
        "the editor fetches its frame"
    );
    rig.answer(ToileEvent::SelectionData {
        elements: vec![frame],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    assert!(
        rig.view.selection_data.is_empty(),
        "the frame is not selected"
    );
    rig.type_text("!");
    rig.paint();
    rig.show(Vec::new());
    rig.paint();
    assert!(rig.view.editor.is_some(), "culled, not deleted");
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::RenameFrame { .. })),
        "a pan renames nothing"
    );
    rig.answer(ToileEvent::BoardChanged);
    assert!(
        rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::RequestSelectionData { ids } if ids == &[eid("f")])),
        "a board change fetches the frame again"
    );
    rig.answer(ToileEvent::SelectionData {
        elements: Vec::new(),
        framed: Vec::new(),
        missing: vec![eid("f")],
    });
    rig.paint();
    assert!(rig.view.editor.is_none(), "the board lost it");
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::RenameFrame { .. })),
        "no write names the lost frame"
    );
    assert_eq!(rig.notices.take().len(), 1, "the user is told once");
}

/// `TOILE-UI-386`: a selected arrow, and a caption editor open on it, outlive
/// the arrow leaving the frame. Only the board's word that it lost the arrow
/// drops them, and another arrow's loss drops neither.
#[test]
fn a_selected_arrow_outlives_the_frame_and_leaves_with_the_board() {
    let mut rig = Rig::new();
    rig.show(vec![DisplayNode::Connector(arrow("c"))]);
    rig.click(200.0, 300.0);
    assert_eq!(rig.view.arrow, Some(ConnectorId("c".to_owned())));
    rig.view.open_label_editor(ConnectorId("c".to_owned()));
    rig.show(Vec::new());
    rig.paint();
    assert_eq!(
        rig.view.arrow,
        Some(ConnectorId("c".to_owned())),
        "culled, not deleted"
    );
    assert!(rig.view.editor.is_some(), "the caption editor stays open");
    rig.answer(ToileEvent::ArrowsGone(vec![ConnectorId("d".to_owned())]));
    assert!(rig.view.arrow.is_some() && rig.view.editor.is_some());
    rig.answer(ToileEvent::ArrowsGone(vec![ConnectorId("c".to_owned())]));
    assert_eq!(rig.view.arrow, None, "the board lost it");
    assert!(rig.view.editor.is_none());
}

#[test]
fn a_write_refused_for_its_lost_target_is_told_and_another_refusal_is_not() {
    // The peer's delete lands before the first selection answer, so the
    // editor reads the target as gone and its close writes blind.
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.type_text("!");
    rig.paint();
    rig.show(Vec::new());
    rig.paint();
    assert!(rig.view.editor.is_none());
    assert!(matches!(
        rig.sent().as_slice(),
        [.., ToileCommand::SetText { id, .. }] if id == &eid("a")
    ));
    use ambre_bus::app::toile::ToileError;
    rig.answer(lost(ToileError::NoSuchElement("b".to_owned())));
    assert!(
        rig.notices.take().is_empty(),
        "another id's refusal says nothing"
    );
    rig.answer(lost(ToileError::NoSuchElement("a".to_owned())));
    assert_eq!(rig.notices.take().len(), 1, "the user is told once");
    rig.answer(lost(ToileError::NoSuchElement("a".to_owned())));
    assert!(rig.notices.take().is_empty(), "and only once");
}

#[test]
fn a_peer_deletion_reaches_the_selection_through_update() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.marquee((-20.0, -20.0), (600.0, 300.0));
    rig.answer(ToileEvent::SelectionData {
        elements: Vec::new(),
        framed: Vec::new(),
        missing: vec![eid("b")],
    });
    assert_eq!(rig.view.selection, [eid("a")]);
}

#[test]
fn a_board_change_refetches_the_selection_data_and_a_camera_move_does_not() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.frame();
    rig.sent();
    rig.answer(ToileEvent::ViewportData(DisplayViewport {
        version: 0,
        pins: Vec::new(),
        reactions: Default::default(),
        viewport: Viewport::default(),
        nodes: vec![DisplayNode::Element(sticky("a", A))],
    }));
    rig.frame();
    let asks = |sent: &[ToileCommand]| {
        sent.iter()
            .any(|c| matches!(c, ToileCommand::RequestSelectionData { ids } if ids == &[eid("a")]))
    };
    assert!(!asks(&rig.sent()), "a fresh frame alone asks nothing");
    rig.answer(ToileEvent::BoardChanged);
    rig.frame();
    assert!(asks(&rig.sent()), "a board change asks again");
    rig.frame();
    assert!(!asks(&rig.sent()), "once");
}

#[test]
fn the_undo_chords_reach_the_logic_thread() {
    let mut rig = Rig::new();
    rig.sent();
    assert!(rig.key("z", true, false));
    assert!(rig.key("Z", true, true));
    assert!(rig.key("y", true, false));
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::Undo, ToileCommand::Redo, ToileCommand::Redo]
    ));
}

#[test]
fn escape_leaves_the_scope_and_clears_the_selection_in_one_press() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.view.group_scope = Some(eid("g"));
    assert!(rig.key("Escape", false, false));
    assert!(rig.view.selection.is_empty());
    assert_eq!(rig.view.group_scope, None);
}

#[test]
fn the_ungroup_chord_sends_one_command_over_the_selection() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.marquee((-20.0, -20.0), (600.0, 300.0));
    rig.sent();
    assert!(rig.key("G", true, true));
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::UngroupElements { groups }] if groups == &[eid("a"), eid("b")]
    ));
}

/// A board holding sticky `a` and the frame `f`, with `a` selected and its
/// selection data in hand. `framed` lists the selected ids a frame holds.
fn selected_in(frame_bounds: Rect, framed: &[&str]) -> Rig {
    let frame = DisplayElement {
        kind: ElementKind::Frame,
        ..sticky("f", frame_bounds)
    };
    let mut rig = Rig::new();
    rig.show(vec![
        DisplayNode::Element(frame),
        DisplayNode::Element(sticky("a", A)),
    ]);
    rig.click(50.0, 50.0);
    rig.frame();
    rig.answer(ToileEvent::SelectionData {
        elements: vec![sticky("a", A)],
        framed: framed.iter().map(|id| eid(id)).collect(),
        missing: Vec::new(),
    });
    rig.sent();
    rig
}

const AWAY: Rect = Rect::new(1000.0, 1000.0, 1100.0, 1100.0);
const AROUND: Rect = Rect::new(-50.0, -50.0, 400.0, 400.0);

#[test]
fn add_to_frame_with_no_frame_around_says_so_and_sends_nothing() {
    let mut rig = selected_in(AWAY, &[]);
    rig.view.apply_menu_command(crate::ui::menu::ADD_TO_FRAME);
    assert!(rig.sent().is_empty());
    assert_eq!(rig.notices.take().len(), 1, "the user is told why");

    let mut rig = selected_in(AROUND, &[]);
    rig.view.apply_menu_command(crate::ui::menu::ADD_TO_FRAME);
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::SetFrameChildren { frame, add, .. }]
            if frame == &eid("f") && add == &[eid("a")]
    ));
    assert!(rig.notices.take().is_empty(), "a done entry says nothing");
}

#[test]
fn remove_from_frame_outside_any_frame_says_so_and_sends_nothing() {
    let mut rig = selected_in(AWAY, &[]);
    rig.view
        .apply_menu_command(crate::ui::menu::REMOVE_FROM_FRAME);
    assert!(rig.sent().is_empty());
    assert_eq!(rig.notices.take().len(), 1, "the user is told why");

    let mut rig = selected_in(AROUND, &["a"]);
    rig.view
        .apply_menu_command(crate::ui::menu::REMOVE_FROM_FRAME);
    assert!(!rig.sent().is_empty(), "a framed selection is removed");
    assert!(rig.notices.take().is_empty(), "a done entry says nothing");
}

#[test]
fn zoom_to_selection_with_nothing_selected_says_so() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view
        .apply_menu_command(crate::ui::menu_bar::ZOOM_TO_SELECTION);
    assert!(rig.sent().is_empty());
    assert_eq!(rig.notices.take().len(), 1, "the user is told why");

    rig.click(50.0, 50.0);
    rig.frame();
    rig.sent();
    rig.view
        .apply_menu_command(crate::ui::menu_bar::ZOOM_TO_SELECTION);
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::RequestSelectionStats { ids, .. }] if ids == &[eid("a")]
    ));
    assert!(rig.notices.take().is_empty(), "a done entry says nothing");
}

/// The bytes Toile's own copy reply leaves in a blob, standing in for a
/// payload whose `origin` names where it came from.
fn clip(origin: Rect) -> ambre_bus::ClipboardBlob {
    let payload = ToileClipboardPayload {
        elements: Vec::new(),
        connectors: Vec::new(),
        membership: Vec::new(),
        origin,
    };
    ambre_bus::ClipboardBlob {
        text: "mirror".to_owned(),
        html: None,
        ambre: postcard::to_allocvec(&payload).ok(),
        plain: false,
    }
}

/// The origin of the payload one `PasteElements` carried, which names the rung
/// that answered.
fn pasted(sent: &[ToileCommand]) -> Option<Rect> {
    match sent {
        [ToileCommand::PasteElements { payload, .. }] => Some(payload.origin),
        _ => None,
    }
}

/// Covers `UI-THREAD-045`: with the Logic thread gone, the copy is refused, so
/// the cut deletes nothing and the selection stays.
#[test]
fn a_cut_whose_copy_went_nowhere_deletes_nothing() {
    let Rig {
        mut view,
        to_logic,
        from_logic: _f,
        notices: _n,
    } = Rig::with_stickies(&[("a", A)]);
    drop(to_logic);
    view.selection = vec![eid("a")];
    assert_eq!(view.clipboard_write(true), ambre_bus::CopyOutcome::Nothing);
    assert_eq!(view.selection, [eid("a")], "the cut kept the selection");
}

/// Covers `UI-THREAD-010` and `UI-THREAD-044`: a copy of a selection that
/// reaches past the viewport names every id, and sends nothing else.
#[test]
fn a_copy_names_the_whole_selection_and_sends_nothing_else() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.answer(ToileEvent::BoardIds(vec![eid("a"), eid("far")]));
    assert!(rig.key("a", true, false));
    rig.sent();
    assert_eq!(
        rig.view.clipboard_write(false),
        ambre_bus::CopyOutcome::Deferred
    );
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::CopyElements { ids }] if ids == &[eid("a"), eid("far")]
    ));
    assert_eq!(rig.view.selection.len(), 2, "a copy leaves the selection");
}

/// Covers `UI-THREAD-022` and `UI-THREAD-046`: the kept copy answers when the
/// token is ours, and the clipboard's own bytes answer when it is not.
#[test]
fn a_paste_takes_the_highest_rung_it_can_serve() {
    let mut rig = Rig::new();
    let kept = Rect::new(1.0, 1.0, 2.0, 2.0);
    let read = Rect::new(5.0, 5.0, 6.0, 6.0);
    rig.answer(ToileEvent::Clipboard(ToileClipboardPayload {
        elements: Vec::new(),
        connectors: Vec::new(),
        membership: Vec::new(),
        origin: kept,
    }));
    rig.sent();
    rig.view.paste_clipboard(clip(read), true);
    assert_eq!(
        pasted(&rig.sent()),
        Some(kept),
        "own copy: the kept payload"
    );
    rig.view.paste_clipboard(clip(read), false);
    assert_eq!(
        pasted(&rig.sent()),
        Some(read),
        "another copy: the bytes read"
    );
}

/// Covers `UI-THREAD-046`: an open editor is the highest rung, so the text
/// goes in at the caret and no element is pasted.
#[test]
fn a_paste_into_an_open_editor_inserts_the_text() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.view.paste_clipboard(clip(B), false);
    rig.key("Escape", false, false);
    rig.paint();
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::SetText { text, .. }] if text == "himirror"
    ));
}

/// Covers `UI-THREAD-047`: a paste leaves no blob for the host to write.
#[test]
fn a_paste_writes_nothing_back() {
    let mut rig = Rig::new();
    rig.view.paste_clipboard(clip(B), false);
    assert_eq!(rig.view.poll_clipboard(), None);
}

/// Covers `CHROME-062`: in an open editor, a printable key, the space
/// included, is left to the text path.
#[test]
fn a_printable_key_in_an_editor_is_not_consumed() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    for key in ["a", "Z", "é", " ", "1"] {
        assert!(!rig.key(key, false, false), "{key:?} was consumed");
    }
}

/// Covers `CHROME-007`: in an open editor the pipe sits on the caret inside
/// the element and follows it as the text grows. A closed editor holds none.
#[test]
fn the_pipe_follows_the_caret_inside_the_element() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.paint();
    assert_eq!(rig.view.caret_drawn, None, "no editor, no caret");
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.paint();
    let first = rig.view.caret_drawn.expect("the editor drew a caret");
    let (left, top) = rig.screen(A.x0, A.y0);
    let (right, bottom) = rig.screen(A.x1, A.y1);
    let on_screen = Rect::new(left, top, right, bottom);
    assert!(
        on_screen.contains(Point::new(first.x0, first.y0)),
        "inside the element: {first:?}"
    );
    rig.input(InputEvent::Text {
        text: "!!!".to_owned(),
    });
    rig.paint();
    let later = rig.view.caret_drawn.expect("the editor drew a caret");
    assert!(later.x0 > first.x0, "the caret moved right: {later:?}");
    rig.key("Escape", false, false);
    rig.paint();
    assert_eq!(rig.view.caret_drawn, None, "the editor closed");
}

/// A find step asks for the geometry of its focus, which may sit far outside
/// the camera, and the answer brings it into view. A reply that no step asked
/// for moves nothing (`FIND-004`).
#[test]
fn the_current_match_is_brought_into_view() {
    use ambre_bus::app::toile::{FindResults, SelectionStats};
    let far = Rect::new(1.0e6, 1.0e6, 1.0e6 + 100.0, 1.0e6 + 80.0);
    let mut rig = Rig::with_stickies(&[("a", A)]);
    assert!(!rig.view.cam.visible_world().contains(far.center()));
    let stats = |kind| {
        ToileEvent::SelectionStats(SelectionStats {
            count: 1,
            bounds: Some(far),
            kind,
        })
    };

    rig.answer(stats(StatsRequestKind::FindFocus));
    assert!(
        !rig.view.cam.visible_world().contains(far.center()),
        "an unasked reply moves nothing"
    );

    rig.answer(ToileEvent::FindResults(FindResults {
        hits: vec![eid("a"), eid("far")],
        focus: Some(eid("far")),
    }));
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::RequestSelectionStats { ids, kind: StatsRequestKind::FindFocus }]
            if ids == &[eid("far")]
    ));
    rig.answer(stats(StatsRequestKind::FindFocus));
    assert!(
        rig.view.cam.visible_world().contains(far.center()),
        "the focus is in view"
    );
}

/// A press reaches the editor in its own frame, never in screen space. The
/// note sits far from the screen origin, so a press at its left edge is a
/// small local x. Read as a screen point, it would land past the end of the
/// text (`CANVAS-039`).
#[test]
fn a_press_reaches_the_editor_in_its_own_frame() {
    let far = Rect::new(600.0, 400.0, 800.0, 520.0);
    let mut rig = Rig::with_stickies(&[("b", far)]);
    rig.click(700.0, 460.0);
    rig.double_click(700.0, 460.0);
    rig.paint();
    rig.click(far.x0 + 2.0, 460.0);
    rig.type_text("X");
    rig.paint();
    let ed = rig.view.editor.as_ref().expect("still open");
    assert_eq!(ed.committed_text().as_deref(), Some("Xhi"));
}

/// The board's editor reimplements no caret motion and no selection. A gesture
/// goes through the view, and its intent goes into a bare core. After each
/// step, both hold the same text and selection (`CANVAS-037`).
#[test]
fn the_board_editor_drives_the_shared_core() {
    use ambre_canvas::TextEditor;
    let key = |key: &str, alt: bool, shift: bool| InputEvent::Key {
        key: key.to_owned(),
        ctrl: false,
        alt,
        shift,
        meta: false,
    };
    let text = |s: &str| InputEvent::Text { text: s.to_owned() };
    let left = |select, word| EditIntent::Left { select, word };
    let right = |select, word| EditIntent::Right { select, word };
    let corpus: Vec<(InputEvent, EditIntent)> = vec![
        (text(" ab cd"), EditIntent::Insert(" ab cd".to_owned())),
        (key("ArrowLeft", false, false), left(false, false)),
        (key("ArrowLeft", false, true), left(true, false)),
        (key("Backspace", false, false), EditIntent::Backspace),
        (key("ArrowLeft", true, false), left(false, true)),
        (key("ArrowRight", true, true), right(true, true)),
        (text("X"), EditIntent::Insert("X".to_owned())),
        (
            key("Home", false, false),
            EditIntent::LineStart { select: false },
        ),
        (key("Delete", false, false), EditIntent::Delete),
        (
            key("End", false, true),
            EditIntent::LineEnd { select: true },
        ),
        (key("ArrowRight", false, false), right(false, false)),
        (
            key("Home", false, true),
            EditIntent::LineStart { select: true },
        ),
        (
            InputEvent::Key {
                key: "a".to_owned(),
                ctrl: true,
                alt: false,
                shift: false,
                meta: false,
            },
            EditIntent::SelectAll,
        ),
        (text("new"), EditIntent::Insert("new".to_owned())),
    ];

    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.key("End", false, false);
    rig.paint();
    let style = rig.view.editor.as_ref().expect("open").style().clone();
    let mut core = TextEditor::new("hi", style);
    let mut font_cx = FontContext::new();
    core.flush(&mut font_cx, 1000.0, 1.0);

    for (step, (event, intent)) in corpus.into_iter().enumerate() {
        rig.input(event);
        rig.paint();
        core.push(intent);
        core.flush(&mut font_cx, 1000.0, 1.0);
        let ed = rig.view.editor.as_ref().expect("still open");
        let board_text = ed
            .committed_text()
            .unwrap_or_else(|| ed.opened_text().to_owned());
        assert_eq!(board_text, core.text(), "text after step {step}");
        assert_eq!(
            ed.selected_text(),
            core.selected_text(),
            "selection after step {step}"
        );
    }
}

/// A finished export reaches the browser as one download, and nothing else
/// happens. No command leaves the view, the selection holds and the board
/// paints the same pixels (`PDF-003`).
#[test]
fn a_finished_export_is_handed_to_the_browser_and_nothing_else() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.frame();
    let _ = rig.sent();
    let selection = rig.view.selection.clone();
    assert_eq!(selection, [eid("a")]);
    let before = rig.paint();
    DOWNLOADS.with(|d| d.borrow_mut().clear());

    rig.answer(ToileEvent::ExportComplete {
        filename: "deck.pdf".to_owned(),
        data: b"%PDF-1.7".to_vec(),
        skipped: Vec::new(),
    });
    rig.frame();

    let handed = DOWNLOADS.with(|d| d.borrow().clone());
    assert_eq!(handed, [("deck.pdf".to_owned(), b"%PDF-1.7".to_vec())]);
    assert!(rig.sent().is_empty(), "the handoff sent a command");
    assert_eq!(rig.view.selection, selection);
    assert_eq!(rig.paint().ops(), before.ops(), "the board changed");
}

/// A frame from the Logic thread at `version`, holding these nodes alone.
fn frame_at(version: u64, nodes: Vec<DisplayNode>) -> ToileEvent {
    ToileEvent::ViewportData(DisplayViewport {
        version,
        pins: Vec::new(),
        reactions: Default::default(),
        viewport: Viewport::default(),
        nodes,
    })
}

/// The ids of the elements the view paints from.
fn shown(rig: &Rig) -> Vec<ElementId> {
    rig.view
        .display
        .nodes
        .iter()
        .filter_map(|n| match n {
            DisplayNode::Element(e) => Some(e.id.clone()),
            _ => None,
        })
        .collect()
}

/// `BUS-017`: an answer older than the one the view holds is dropped, and an
/// answer at the same version still lands.
#[test]
fn a_stale_viewport_answer_is_dropped_by_its_version() {
    let mut rig = Rig::new();
    rig.answer(frame_at(5, vec![DisplayNode::Element(sticky("new", A))]));
    rig.answer(frame_at(3, vec![DisplayNode::Element(sticky("old", A))]));
    assert_eq!(shown(&rig), vec![eid("new")], "the older answer is dropped");
    rig.answer(frame_at(5, vec![DisplayNode::Element(sticky("same", A))]));
    assert_eq!(shown(&rig), vec![eid("same")], "the same version lands");
}

/// `BUS-012`: two camera moves inside one frame leave the view as one
/// `SetViewport`, for the camera the second move left.
#[test]
fn two_camera_moves_in_one_frame_send_one_viewport_request() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    for dy in [40.0, 60.0] {
        rig.input(InputEvent::Wheel {
            dx: 0.0,
            dy,
            delta_mode: 0,
            ctrl_key: false,
        });
    }
    let last = rig.view.cam.to_bus_viewport();
    rig.view.update();
    let requests: Vec<_> = rig
        .sent()
        .into_iter()
        .filter_map(|c| match c {
            ToileCommand::SetViewport(vp) => Some(vp),
            _ => None,
        })
        .collect();
    assert_eq!(requests, vec![last], "one request, for the last camera");
}

/// `ARCH-008`, `ARCH-009`: a fatal event from the Logic thread reaches the
/// page's fatal latch, and the view sends nothing after it.
#[test]
fn a_fatal_event_latches_the_page_and_silences_the_view() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    let sent = rig.from_logic.try_send(UiEvent::FatalError {
        message: "doorbell lost".to_owned(),
    });
    assert!(sent.is_ok(), "the inbound channel has room");
    rig.view.update();
    assert!(rig.view.fatal, "the view is inert");
    assert!(ambre_canvas::fatal_message().is_some(), "the page is told");
    rig.key("z", true, false);
    assert!(rig.sent().is_empty(), "an inert view sends nothing");
}

/// `ARCH-008`: a Logic thread that dropped its end without a word is as
/// fatal to the view as one that said so.
#[test]
fn a_vanished_logic_thread_latches_the_page() {
    let Rig {
        mut view,
        from_logic,
        ..
    } = Rig::with_stickies(&[("a", A)]);
    drop(from_logic);
    view.update();
    assert!(view.fatal, "the view is inert");
    assert!(ambre_canvas::fatal_message().is_some(), "the page is told");
}

// -----------------------------------------------------------------------------
// Arrows driven through the view (`toile-connectors`)
// -----------------------------------------------------------------------------

fn cid(s: &str) -> ConnectorId {
    ConnectorId(s.to_owned())
}

/// A board holding the arrow `c`, from (0, 300) to (400, 300), and these
/// stickies, with the commands of the boot drained.
fn with_arrow(stickies: &[(&str, Rect)]) -> Rig {
    let mut rig = Rig::new();
    let mut nodes = vec![DisplayNode::Connector(arrow("c"))];
    nodes.extend(
        stickies
            .iter()
            .map(|(id, bounds)| DisplayNode::Element(sticky(id, *bounds))),
    );
    rig.show(nodes);
    rig.sent();
    rig
}

/// A press at one world point, a move and a release at another.
fn drag(rig: &mut Rig, from: (f64, f64), to: (f64, f64)) {
    let from = rig.screen(from.0, from.1);
    let to = rig.screen(to.0, to.1);
    rig.press(from);
    rig.input(InputEvent::PointerMove { x: to.0, y: to.1 });
    rig.release(to);
}

fn creates_a_connector(sent: &[ToileCommand]) -> bool {
    sent.iter()
        .any(|c| matches!(c, ToileCommand::CreateConnector { .. }))
}

/// `TOILE-UI-214`: a connector drag released over the element it started on
/// sends nothing.
#[test]
fn an_arrow_to_its_own_source_sends_nothing() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.set_tool(Tool::Connector);
    drag(&mut rig, (190.0, 60.0), (60.0, 60.0));
    assert!(!creates_a_connector(&rig.sent()));
}

/// `TOILE-UI-215`: a connector drag sends the one create on release, and
/// moves, resizes or selects nothing on the way.
#[test]
fn a_connector_drag_writes_only_its_create() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.view.set_tool(Tool::Connector);
    let from = rig.screen(190.0, 60.0);
    let to = rig.screen(320.0, 60.0);
    rig.press(from);
    rig.input(InputEvent::PointerMove { x: to.0, y: to.1 });
    assert!(rig.sent().is_empty(), "the preview is local");
    rig.release(to);
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::CreateConnector { from, to, .. }] if from == &eid("a") && to == &eid("b")
    ));
    assert!(rig.view.selection.is_empty(), "the source is not selected");
    assert_eq!(shown_bounds(&rig, "a"), Some(A), "the source did not move");
}

fn shown_bounds(rig: &Rig, id: &str) -> Option<Rect> {
    rig.view.display.nodes.iter().find_map(|n| match n {
        DisplayNode::Element(e) if e.id == eid(id) => Some(e.bounds),
        _ => None,
    })
}

/// `TOILE-UI-344`: a double-click on an arrow opens the caption editor on
/// that arrow, seeded with its caption.
#[test]
fn a_double_click_on_an_arrow_opens_its_caption_editor() {
    let mut rig = with_arrow(&[]);
    rig.double_click(200.0, 300.0);
    let ed = rig.view.editor.as_ref().expect("the editor opened");
    assert_eq!(ed.target().connector(), Some(&cid("c")));
    assert_eq!(ed.opened_text(), "yes");
}

/// `TOILE-UI-345`: leaving the caption editor sends the caption once.
#[test]
fn leaving_the_caption_editor_sends_the_caption() {
    let mut rig = with_arrow(&[]);
    rig.double_click(200.0, 300.0);
    rig.type_text("!");
    rig.key("Escape", false, false);
    rig.paint();
    assert!(rig.view.editor.is_none());
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::SetConnectorLabel { id, base, text }]
            if id == &cid("c") && base == "yes" && text == "yes!"
    ));
}

/// `TOILE-UI-363`: leaving the caption editor with the caption as it opened
/// sends nothing.
#[test]
fn an_untouched_caption_editor_sends_nothing() {
    let mut rig = with_arrow(&[]);
    rig.double_click(200.0, 300.0);
    rig.key("Escape", false, false);
    rig.paint();
    assert!(rig.view.editor.is_none());
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::SetConnectorLabel { .. })),
        "an unchanged caption is not written"
    );
}

/// `TOILE-UI-350`: a click on an arrow with nothing else under it selects
/// the arrow.
#[test]
fn a_click_on_a_bare_arrow_selects_it() {
    let mut rig = with_arrow(&[]);
    rig.click(200.0, 300.0);
    assert_eq!(rig.view.arrow, Some(cid("c")));
}

/// `TOILE-UI-359`: a click on an element lying over an arrow takes the
/// element, and the arrow under it stays unselected.
#[test]
fn a_click_on_an_element_over_an_arrow_takes_the_element() {
    let mut rig = with_arrow(&[("s", Rect::new(150.0, 250.0, 250.0, 350.0))]);
    rig.click(200.0, 300.0);
    assert_eq!(rig.view.selection, [eid("s")]);
    assert_eq!(rig.view.arrow, None);
}

/// `TOILE-UI-352`: dragging the far handle of the selected arrow onto a
/// shape re-aims that end at the shape, on the side the pointer aimed at.
#[test]
fn an_endpoint_dragged_onto_a_shape_re_aims_that_end() {
    let mut rig = with_arrow(&[("b", Rect::new(500.0, 250.0, 700.0, 350.0))]);
    rig.click(200.0, 300.0);
    rig.sent();
    drag(&mut rig, (400.0, 300.0), (510.0, 300.0));
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::RetargetConnector { id, endpoint: End::To, to, anch: Anchor::Left }]
            if id == &cid("c") && to == &eid("b")
    ));
}

/// `TOILE-UI-372`: an endpoint handle released over empty board sends
/// nothing, and the arrow stays selected.
#[test]
fn an_endpoint_released_over_empty_board_leaves_the_arrow() {
    let mut rig = with_arrow(&[]);
    rig.click(200.0, 300.0);
    rig.sent();
    drag(&mut rig, (400.0, 300.0), (900.0, 600.0));
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::RetargetConnector { .. })),
        "a release over nothing re-aims nothing"
    );
    assert_eq!(rig.view.arrow, Some(cid("c")));
}

/// `TOILE-UI-354`: selecting an arrow empties the element selection, an
/// element click drops the arrow, and a marquee over both takes the
/// element alone.
#[test]
fn an_arrow_and_an_element_are_never_selected_together() {
    let mut rig = with_arrow(&[("a", A)]);
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a")]);
    rig.click(300.0, 300.0);
    assert_eq!(rig.view.arrow, Some(cid("c")));
    assert!(rig.view.selection.is_empty(), "the arrow empties it");
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.arrow, None, "the element click drops the arrow");
    rig.marquee((-20.0, -20.0), (600.0, 400.0));
    assert_eq!(rig.view.selection, [eid("a")], "a marquee takes no arrow");
    assert_eq!(rig.view.arrow, None);
}

/// The live stroke each presence sample sent since the last call carried.
fn live_strokes(sent: &[ToileCommand]) -> Vec<Option<usize>> {
    sent.iter()
        .filter_map(|c| match c {
            ToileCommand::Presence { payload, .. } => {
                Some(payload.live_stroke.as_ref().map(Vec::len))
            }
            _ => None,
        })
        .collect()
}

/// `TOILE-106`: while a user draws, each frame's presence sample carries the
/// stroke as it grows, and the sample after the release carries none.
#[test]
fn each_frame_offers_the_stroke_being_drawn_to_the_others() {
    let mut rig = Rig::new();
    rig.sent();
    rig.view.set_tool(Tool::Draw);
    let start = rig.screen(0.0, 0.0);
    rig.press(start);
    let mut t = 100.0;
    let mut seen = Vec::new();
    for i in 1..4 {
        let (x, y) = rig.screen(i as f64 * 10.0, 0.0);
        rig.input(InputEvent::PointerMove { x, y });
        rig.view.plan_repaint(t, false);
        t += 16.0;
        seen.extend(live_strokes(&rig.sent()));
    }
    assert_eq!(
        seen,
        [Some(2), Some(3), Some(4)],
        "one growing sample a frame"
    );
    let end = rig.screen(40.0, 0.0);
    rig.release(end);
    rig.view.plan_repaint(t, false);
    assert_eq!(live_strokes(&rig.sent()), [None], "the trail ends");
}

// -- The inline editor (`toile-text-editing`) --------------------------------

/// The commands that change the document. Reads, presence and the viewport
/// the view asks for are left out.
fn writes(sent: Vec<ToileCommand>) -> Vec<ToileCommand> {
    sent.into_iter()
        .filter(|c| {
            !matches!(
                c,
                ToileCommand::RequestSelectionStats { .. }
                    | ToileCommand::RequestSelectionData { .. }
                    | ToileCommand::RequestBoardIds
                    | ToileCommand::RequestMedia { .. }
                    | ToileCommand::SetViewport { .. }
                    | ToileCommand::Presence { .. }
                    | ToileCommand::ReadComments { .. }
                    | ToileCommand::ReadDeck
            )
        })
        .collect()
}

/// The element the open editor writes to.
fn editing(rig: &Rig) -> Option<ElementId> {
    rig.view
        .editor
        .as_ref()
        .and_then(|ed| ed.target().element().cloned())
}

/// A rectangle shape: an element that carries no body.
fn shape(id: &str, bounds: Rect) -> DisplayElement {
    DisplayElement {
        kind: ElementKind::Shape(ambre_bus::app::toile::ShapeKind::Rect),
        geom: ambre_bus::app::toile::DisplayGeom::Rect,
        ..sticky(id, bounds)
    }
}

/// `TOILE-UI-059`, `TOILE-UI-193`: a note placed with the tool opens its
/// editor once the board answers with it, and the tool is back on select.
/// Another create's reply claims nothing.
#[test]
fn a_placed_note_opens_its_editor_and_the_tool_returns_to_select() {
    let mut rig = Rig::new();
    rig.sent();
    rig.view.set_tool(Tool::Sticky(0xffff_eb3b));
    rig.click(600.0, 400.0);
    let token = rig.sent().iter().find_map(|c| match c {
        ToileCommand::CreateElement {
            kind: ElementKind::Sticky,
            correlate,
            ..
        } => Some(*correlate),
        _ => None,
    });
    assert_eq!(rig.view.machine.tool(), Tool::Select, "one note a click");
    let token = token.expect("the click places a note");
    rig.answer(ToileEvent::ElementCreated {
        id: eid("n"),
        correlate: token,
    });
    assert_eq!(editing(&rig), None, "no geometry yet");
    rig.show(vec![DisplayNode::Element(sticky("n", A))]);
    assert_eq!(editing(&rig), Some(eid("n")));
}

/// `TOILE-UI-198`, `TOILE-UI-196`, `TOILE-UI-201`: a second click on the
/// sole selection opens its editor. The open writes nothing and moves no
/// camera, and the body it opens on is the committed one.
#[test]
fn a_second_click_on_the_sole_selection_opens_its_editor() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    assert_eq!(editing(&rig), None, "the first click only selects");
    let cam = (rig.view.cam.zoom, rig.view.cam.world_to_screen());
    rig.sent();
    rig.click(50.0, 50.0);
    assert_eq!(editing(&rig), Some(eid("a")));
    rig.frame();
    rig.paint();
    assert!(writes(rig.sent()).is_empty(), "entering is not editing");
    assert_eq!((rig.view.cam.zoom, rig.view.cam.world_to_screen()), cam);
    let opened = rig.view.editor.as_ref().map(|e| e.opened_text().to_owned());
    assert_eq!(opened.as_deref(), Some("hi"));
    assert_eq!(shown_bounds(&rig, "a"), Some(A));
}

/// `TOILE-UI-199`: a press on the selected note that travels before its
/// release is a move, and opens nothing.
#[test]
fn a_drag_over_the_selected_note_opens_no_editor() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    drag(&mut rig, (50.0, 50.0), (90.0, 80.0));
    assert_eq!(editing(&rig), None);
}

/// `TOILE-UI-200`: a double-click inside a wider selection opens the editor
/// and narrows the selection to the element under edit.
#[test]
fn an_open_editor_is_the_whole_selection() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.marquee((-20.0, -20.0), (600.0, 300.0));
    assert_eq!(rig.view.selection, [eid("a"), eid("b")]);
    rig.double_click(50.0, 50.0);
    assert_eq!(editing(&rig), Some(eid("a")));
    assert_eq!(rig.view.selection, [eid("a")]);
}

/// `TOILE-UI-184`: a shape carries no body. A second click, a double-click
/// and a typed key each leave it without an editor.
#[test]
fn an_element_without_a_body_opens_no_editor() {
    let mut rig = Rig::new();
    rig.show(vec![DisplayNode::Element(shape("s", A))]);
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("s")]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.key("x", false, false);
    assert!(rig.view.editor.is_none());
}

/// `TOILE-UI-060`, `TOILE-UI-202`, `TOILE-UI-203`: a printable key over one
/// selected note opens its editor and is not consumed. The character then
/// replaces the body rather than extending it.
#[test]
fn a_typed_character_opens_the_editor_and_replaces_the_body() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.sent();
    assert!(!rig.key("x", false, false), "the key reaches the text pipe");
    assert_eq!(editing(&rig), Some(eid("a")));
    rig.type_text("x");
    rig.key("Escape", false, false);
    rig.paint();
    assert!(matches!(
        writes(rig.sent()).as_slice(),
        [ToileCommand::SetText { id, base, text }]
            if id == &eid("a") && base == "hi" && text == "x"
    ));
}

/// `TOILE-UI-204`: the space bar over a selected note arms the pan.
#[test]
fn the_space_bar_opens_no_editor() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    assert!(rig.key(" ", false, false), "space is the board's");
    assert!(rig.view.editor.is_none());
}

/// `TOILE-UI-205`: with no selection, two notes, or a shape selected, a
/// typed key opens nothing.
#[test]
fn a_key_over_anything_but_one_note_opens_no_editor() {
    let mut rig = Rig::new();
    rig.show(vec![
        DisplayNode::Element(sticky("a", A)),
        DisplayNode::Element(sticky("b", B)),
        DisplayNode::Element(shape("s", Rect::new(0.0, 200.0, 100.0, 260.0))),
    ]);
    rig.key("x", false, false);
    assert!(rig.view.editor.is_none(), "nothing selected");
    rig.marquee((-20.0, -20.0), (600.0, 150.0));
    assert_eq!(rig.view.selection, [eid("a"), eid("b")]);
    rig.key("x", false, false);
    assert!(rig.view.editor.is_none(), "two notes");
    rig.click(50.0, 230.0);
    assert_eq!(rig.view.selection, [eid("s")]);
    rig.key("x", false, false);
    assert!(rig.view.editor.is_none(), "a shape");
}

/// `TOILE-UI-206`, `TOILE-UI-064`, `TOILE-UI-208`: an edit of many keystrokes
/// leaves as one body write and nothing else. The element stays selected,
/// and the camera and the tool are as they were.
#[test]
fn a_close_writes_the_body_once_and_keeps_the_element_selected() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    let cam = (rig.view.cam.zoom, rig.view.cam.world_to_screen());
    rig.sent();
    for ch in ["a", "b", "c"] {
        rig.type_text(ch);
        rig.paint();
    }
    rig.key("Escape", false, false);
    rig.paint();
    rig.frame();
    assert!(rig.view.editor.is_none());
    assert!(matches!(
        writes(rig.sent()).as_slice(),
        [ToileCommand::SetText { text, .. }] if text == "hiabc"
    ));
    assert_eq!(rig.view.selection, [eid("a")]);
    assert_eq!((rig.view.cam.zoom, rig.view.cam.world_to_screen()), cam);
    assert_eq!(rig.view.machine.tool(), Tool::Select);
    assert_eq!(shown_bounds(&rig, "a"), Some(A));
}

/// Where each glyph run a paint drew lands on screen, in order. The pushed
/// transforms around a run are folded into its own, rounded to a millipixel.
fn glyph_runs(surface: &ambre_draw::testing::RecordingSurface) -> Vec<[i64; 6]> {
    use ambre_draw::testing::DrawOp;
    let mut stack = vec![kurbo::Affine::IDENTITY];
    let mut runs = Vec::new();
    for op in surface.ops() {
        let top = stack.last().copied().unwrap_or_default();
        match op {
            DrawOp::PushTransform { transform } => stack.push(top * *transform),
            DrawOp::PopTransform => {
                stack.pop();
            }
            DrawOp::Glyphs { transform, .. } => {
                runs.push(
                    (top * *transform)
                        .as_coeffs()
                        .map(|c| (c * 1000.0).round() as i64),
                );
            }
            _ => {}
        }
    }
    runs
}

/// `TOILE-UI-189`, `TOILE-UI-191`: the editor paints its body where the
/// committed body was, and the committed body is not painted under it.
#[test]
fn the_body_under_edit_replaces_the_committed_one_in_place() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    let committed = glyph_runs(&rig.paint());
    assert!(!committed.is_empty());
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    let edited = glyph_runs(&rig.paint());
    assert_eq!(edited, committed, "one body, at the same origin");
}

/// `TOILE-UI-192`: the caret shows for one half of the cycle and hides for
/// the other.
#[test]
fn the_caret_blinks() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    let ops_at = |rig: &mut Rig, t: f64| {
        rig.view.plan_repaint(t, false);
        rig.paint().ops().len()
    };
    let on = ops_at(&mut rig, 0.0);
    let off = ops_at(&mut rig, 600.0);
    assert_eq!(on, off + 1, "the caret is one op");
    assert_eq!(ops_at(&mut rig, 1100.0), on, "and back within the second");
}

/// `TOILE-UI-190`: a rotated note keeps its rotation under the editor. Each
/// run lands where the committed run did, rotation included.
#[test]
fn a_rotated_note_keeps_its_rotation_while_edited() {
    let mut note = sticky("a", A);
    let c = A.center().to_vec2();
    note.transform = kurbo::Affine::translate(c)
        * kurbo::Affine::rotate(std::f64::consts::FRAC_PI_6)
        * kurbo::Affine::translate(-c);
    let mut rig = Rig::new();
    rig.show(vec![DisplayNode::Element(note)]);
    let committed = glyph_runs(&rig.paint());
    assert!(committed.iter().all(|r| r[1] != 0), "the body is rotated");
    rig.click(100.0, 60.0);
    rig.double_click(100.0, 60.0);
    assert_eq!(editing(&rig), Some(eid("a")));
    assert_eq!(glyph_runs(&rig.paint()), committed);
}

/// `TOILE-UI-115`, `TOILE-UI-116`, `TOILE-UI-118`: a step asks for the
/// focus box and recentres on it when the answer lands. The zoom and the
/// selection are as they were before the search.
#[test]
fn a_find_step_recentres_without_a_zoom_or_a_selection() {
    use ambre_bus::app::toile::{FindResults, SelectionStats};
    let far = Rect::new(5000.0, 4000.0, 5100.0, 4080.0);
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    let zoom = rig.view.cam.zoom;
    rig.sent();
    rig.answer(ToileEvent::FindResults(FindResults {
        hits: vec![eid("a"), eid("far")],
        focus: Some(eid("far")),
    }));
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::RequestSelectionStats { ids, kind: StatsRequestKind::FindFocus }]
            if ids == &[eid("far")]
    ));
    rig.answer(ToileEvent::SelectionStats(SelectionStats {
        count: 1,
        bounds: Some(far),
        kind: StatsRequestKind::FindFocus,
    }));
    let centre = rig.view.cam.visible_world().center();
    assert!((centre - far.center()).hypot() < 1e-6, "on the match");
    assert_eq!(rig.view.cam.zoom, zoom, "the zoom is kept");
    assert_eq!(rig.view.selection, [eid("a")], "a match is not selected");
}
