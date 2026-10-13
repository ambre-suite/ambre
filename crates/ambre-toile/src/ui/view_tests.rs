//! The board view driven the way the host drives it. Input events go in and
//! the commands for the Logic thread come out. The Logic thread's answers are
//! fed back through the inbound channel. No DOM and no GPU: the text pipe is
//! detached, and a frame paints into a recording surface.

use super::*;
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
