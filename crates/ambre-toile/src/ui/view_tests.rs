//! The board view driven the way the host drives it. Input events go in and
//! the commands for the Logic thread come out. The Logic thread's answers are
//! fed back through the inbound channel. No DOM and no GPU: the text pipe is
//! detached, and a frame paints into a recording surface.

use super::*;
use ambre_bus::UserId;
use ambre_bus::app::toile::{Anchor, ElementStyle, PresenceUpdate, PresentMode, ShapeKind};
use ambre_bus::conduit::Receiver;
use ambre_core::Argb;
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
        Self::with_queue(256)
    }

    /// A board whose queue to the Logic thread holds `capacity` commands.
    fn with_queue(capacity: usize) -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(capacity);
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
            version: ambre_bus::ViewportVersion(0),
            grid_unit: 20.0,
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
        self.view.plan_repaint(ambre_bus::PageTime(0.0), false);
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

/// `TOILE-UI-393`: the keyboard pipe of a board is typed text only while an
/// editor is open. Before and after, a bare key reaches the keymap and arms
/// its tool.
#[test]
fn the_pipe_is_typed_text_only_while_an_editor_is_open() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.plan_repaint(ambre_bus::PageTime(0.0), false);
    assert!(!rig.view.pipe_typing(), "a board with no editor");
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.view.plan_repaint(ambre_bus::PageTime(16.0), false);
    assert!(rig.view.pipe_typing(), "an open editor takes the keys");
    rig.key("Escape", false, false);
    rig.paint();
    rig.view.plan_repaint(ambre_bus::PageTime(32.0), false);
    assert!(!rig.view.pipe_typing(), "a closed editor gives them back");
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
        version: ambre_bus::ViewportVersion(0),
        grid_unit: 20.0,
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
    assert!(rig.key("a", true, false));
    rig.answer(ToileEvent::BoardIds(vec![eid("a"), eid("far")]));
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

/// `TOILE-UI-033`: each select-all asks Logic for the board's ids, so an
/// element created after the open is in it. An answer no select-all asked
/// for selects nothing.
#[test]
fn each_select_all_asks_for_the_board_as_it_is_now() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.answer(ToileEvent::BoardIds(vec![eid("a")]));
    assert!(rig.view.selection.is_empty(), "an unasked answer selected");

    for board in [vec![eid("a")], vec![eid("a"), eid("new")]] {
        assert!(rig.key("a", true, false));
        assert!(
            rig.sent()
                .iter()
                .any(|c| matches!(c, ToileCommand::RequestBoardIds)),
            "the select-all asked for nothing"
        );
        rig.answer(ToileEvent::BoardIds(board.clone()));
        assert_eq!(rig.view.selection, board);
    }
}

/// `TOILE-UI-396`: an `Escape` or a press that comes before the select-all
/// answer wins. The answer that follows selects nothing.
#[test]
fn a_selection_change_before_the_select_all_answer_wins() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    assert!(rig.key("a", true, false));
    assert!(rig.key("Escape", false, false));
    rig.answer(ToileEvent::BoardIds(vec![eid("a")]));
    assert!(rig.view.selection.is_empty(), "the answer undid the Escape");

    assert!(rig.key("a", true, false));
    rig.click(A.x1 + 400.0, A.y1 + 400.0);
    rig.answer(ToileEvent::BoardIds(vec![eid("a")]));
    assert!(rig.view.selection.is_empty(), "the answer undid the click");
}

/// `CHROME-102`: a press on the board gives the keyboard focus to the pipe,
/// on an element and on empty board alike.
#[test]
fn a_press_on_the_board_focuses_the_pipe() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    let before = rig.view.text_input.focus_requests();
    rig.click(A.center().x, A.center().y);
    rig.click(A.x1 + 400.0, A.y1 + 400.0);
    assert_eq!(rig.view.text_input.focus_requests(), before + 2);
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
/// paints the same pixels (`PDF-014`).
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
        version: ambre_bus::ViewportVersion(version),
        grid_unit: 20.0,
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
            delta: ambre_canvas::WheelDelta::pixels(0.0, dy),
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
        rig.view.plan_repaint(ambre_bus::PageTime(t), false);
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
    rig.view.plan_repaint(ambre_bus::PageTime(t), false);
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
    rig.view
        .set_tool(Tool::Sticky(Argb::from_bits(0xffff_eb3b)));
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
        rig.view.plan_repaint(ambre_bus::PageTime(t), false);
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

fn wheel(rig: &mut Rig, (dx, dy): (f64, f64), delta_mode: u32, ctrl_key: bool) {
    rig.input(InputEvent::Wheel {
        delta: ambre_canvas::WheelDelta::new(
            dx,
            dy,
            ambre_canvas::WheelDeltaMode::from_dom(delta_mode),
        ),
        ctrl_key,
    });
}

fn viewport_requests(sent: Vec<ToileCommand>) -> Vec<Viewport> {
    sent.into_iter()
        .filter_map(|c| match c {
            ToileCommand::SetViewport(vp) => Some(vp),
            _ => None,
        })
        .collect()
}

/// `TOILE-UI-282`, `TOILE-UI-283`: a wheel pan moves the centre alone. The
/// zoom and the selection stay, and the view asks for the place it landed.
#[test]
fn a_pan_keeps_the_zoom_and_the_selection_and_asks_where_it_landed() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.view.update();
    rig.sent();
    let (zoom, centre) = (rig.view.cam.zoom, rig.view.cam.center);
    wheel(&mut rig, (30.0, 40.0), 0, false);
    assert_ne!(rig.view.cam.center, centre, "the board moved");
    assert_eq!(rig.view.cam.zoom, zoom);
    assert_eq!(rig.view.selection, [eid("a")]);
    rig.view.update();
    assert_eq!(
        viewport_requests(rig.sent()),
        vec![rig.view.cam.to_bus_viewport()]
    );
}

/// `TOILE-UI-289`: in the trackpad mode the wheel pans both axes, and the
/// content moves against the delta, as a page scrolls.
#[test]
fn the_trackpad_mode_pans_both_axes_against_the_delta() {
    let mut rig = Rig::new();
    rig.view.pointer_mode = PointerMode::Trackpad;
    let before = rig.screen(0.0, 0.0);
    wheel(&mut rig, (12.0, 18.0), 0, false);
    let after = rig.screen(0.0, 0.0);
    assert_eq!(rig.view.cam.zoom, 1.0);
    assert!(
        after.0 < before.0 && after.1 < before.1,
        "{before:?} {after:?}"
    );
}

/// `TOILE-UI-288`: a mouse notch latches the mouse, and a fine glide with no
/// side delta is too weak to change it. A two-axis glide latches the
/// trackpad, and the fine glide then pans.
#[test]
fn an_ambiguous_wheel_keeps_the_latched_guess() {
    let mut rig = Rig::new();
    wheel(&mut rig, (0.0, 120.0), 0, false);
    assert!(rig.view.detected_mouse, "a notch is a mouse");
    let zoom = rig.view.cam.zoom;
    wheel(&mut rig, (0.0, 2.0), 0, false);
    assert!(rig.view.detected_mouse);
    assert_ne!(rig.view.cam.zoom, zoom, "the mouse guess still zooms");
    wheel(&mut rig, (3.0, 2.0), 0, false);
    assert!(!rig.view.detected_mouse, "a side delta is a trackpad");
    let zoom = rig.view.cam.zoom;
    wheel(&mut rig, (0.0, 2.0), 0, false);
    assert!(!rig.view.detected_mouse);
    assert_eq!(rig.view.cam.zoom, zoom, "the trackpad guess still pans");
}

/// `TOILE-UI-028`: `Ctrl` with the wheel zooms in every pointer mode, and a
/// pinch is that same event.
#[test]
fn ctrl_and_the_pinch_zoom_in_every_mode() {
    for mode in [PointerMode::Auto, PointerMode::Mouse, PointerMode::Trackpad] {
        let mut rig = Rig::new();
        rig.view.pointer_mode = mode;
        // A trackpad glide first, so the auto guess is not the mouse.
        wheel(&mut rig, (3.0, 2.0), 0, false);
        let zoom = rig.view.cam.zoom;
        wheel(&mut rig, (0.0, -30.0), 0, true);
        assert!(rig.view.cam.zoom > zoom, "a pinch out zooms in");
    }
}

/// `TOILE-UI-029`: a mode the user picked ignores the device signature. A
/// line wheel pans in the trackpad mode, and a glide zooms in the mouse
/// mode. Neither moves the auto latch.
#[test]
fn a_picked_mode_ignores_a_contrary_wheel() {
    let mut rig = Rig::new();
    rig.view.pointer_mode = PointerMode::Trackpad;
    wheel(&mut rig, (0.0, 3.0), 1, false);
    assert_eq!(rig.view.cam.zoom, 1.0, "a line wheel pans");
    assert!(!rig.view.detected_mouse, "the latch is untouched");

    let mut rig = Rig::new();
    rig.view.pointer_mode = PointerMode::Mouse;
    rig.view.detected_mouse = false;
    wheel(&mut rig, (4.0, 6.0), 0, false);
    assert_ne!(rig.view.cam.zoom, 1.0, "a glide zooms");
    assert!(!rig.view.detected_mouse, "the latch is untouched");
}

/// `TOILE-UI-378`, `TOILE-UI-379`: the view's hit test feeds the finger
/// rule. The empty board pans with the select tool; an element, a handle
/// or a drawing tool keeps the finger as the mouse.
#[test]
fn a_finger_pans_only_the_empty_board() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    let empty = rig.screen(900.0, 600.0);
    assert!(
        rig.view
            .touch_down(empty.0, empty.1, ambre_bus::PageTime(0.0))
    );
    assert_eq!(rig.view.finger_pans, Some(true));
    let centre = rig.view.cam.center;
    rig.view
        .touch_move(empty.0 - 60.0, empty.1, ambre_bus::PageTime(16.0));
    rig.view
        .touch_move(empty.0 - 120.0, empty.1, ambre_bus::PageTime(32.0));
    assert!(rig.view.cam.center.x > centre.x, "the finger pans");
    rig.view.touch_cancel();

    let on_a = rig.screen(50.0, 50.0);
    rig.view
        .touch_down(on_a.0, on_a.1, ambre_bus::PageTime(100.0));
    assert_eq!(rig.view.finger_pans, Some(false), "an element is grabbed");
    rig.view
        .touch_up(on_a.0, on_a.1, ambre_bus::PageTime(110.0));
    assert_eq!(rig.view.selection, [eid("a")]);

    let (x0, y0) = rig.screen(A.x0, A.y0);
    let (x1, y1) = rig.screen(A.x1, A.y1);
    let knob = crate::ui::tool::rotate_handle_point(Rect::new(x0, y0, x1, y1));
    rig.view
        .touch_down(knob.x, knob.y, ambre_bus::PageTime(200.0));
    assert_eq!(rig.view.finger_pans, Some(false), "a handle is grabbed");
    rig.view.touch_cancel();

    rig.view.set_tool(Tool::Pen);
    rig.view
        .touch_down(empty.0, empty.1, ambre_bus::PageTime(300.0));
    assert_eq!(rig.view.finger_pans, Some(false), "a drawing tool draws");
}

/// `TOILE-UI-380`: a drag held in the right-hand band pans the board right
/// once a frame. The screen step is the same at 1x and at 2x, and the view
/// asks for the place it reached.
#[test]
fn a_drag_held_at_an_edge_pans_the_board_each_frame() {
    let screen_step = |zoom: f64| {
        let mut rig = Rig::with_stickies(&[("a", A)]);
        rig.view.cam.zoom = zoom;
        rig.press(rig.screen(50.0, 50.0));
        rig.input(InputEvent::PointerMove {
            x: 1195.0,
            y: 400.0,
        });
        rig.view.plan_repaint(ambre_bus::PageTime(1000.0), false);
        let x = rig.view.cam.center.x;
        rig.view.plan_repaint(ambre_bus::PageTime(1016.0), false);
        let moved = rig.view.cam.center.x - x;
        rig.view.update();
        assert!(!viewport_requests(rig.sent()).is_empty(), "zoom {zoom}");
        moved * zoom
    };
    let (one, two) = (screen_step(1.0), screen_step(2.0));
    assert!(one > 0.0, "the board pans towards the edge");
    assert!((one - two).abs() < 1e-6, "{one} at 1x, {two} at 2x");
}

/// `TOILE-UI-296`, `TOILE-UI-297`, `TOILE-UI-298`: a resize keeps the centre
/// and the zoom, and asks for the new canvas. A canvas with no extent is
/// the smallest one, and the camera still inverts.
#[test]
fn a_resize_keeps_the_camera_and_asks_for_the_new_canvas() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.cam.center = kurbo::Point::new(40.0, -25.0);
    rig.view.cam.zoom = 1.5;
    rig.view.update();
    rig.sent();
    rig.view.resize(1600, 900);
    assert_eq!(rig.view.cam.center, kurbo::Point::new(40.0, -25.0));
    assert_eq!(rig.view.cam.zoom, 1.5);
    rig.view.update();
    let asked = viewport_requests(rig.sent());
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].size, kurbo::Size::new(1600.0, 900.0));

    rig.view.resize(0, 0);
    assert_eq!(rig.view.cam.size, kurbo::Size::new(1.0, 1.0));
    let back = rig.view.cam.screen_to_world() * kurbo::Point::new(0.5, 0.5);
    assert!(back.x.is_finite() && back.y.is_finite());
    // The host hands whole pixels; the camera floors a negative extent too.
    rig.view.cam.set_size(-4.0, -9.0);
    assert_eq!(rig.view.cam.size, kurbo::Size::new(1.0, 1.0));
}

fn stats_requests(sent: &[ToileCommand]) -> Vec<Vec<ElementId>> {
    sent.iter()
        .filter_map(|c| match c {
            ToileCommand::RequestSelectionStats {
                ids,
                kind: StatsRequestKind::Band,
            } => Some(ids.clone()),
            _ => None,
        })
        .collect()
}

/// `TOILE-UI-055`, `TOILE-UI-109`, `TOILE-UI-110`: a new selection asks for
/// its aggregates once. A frame with the same selection asks for nothing,
/// and an emptied selection clears the band and asks for nothing.
#[test]
fn the_band_asks_once_per_selection_change() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.click(50.0, 50.0);
    rig.frame();
    assert_eq!(stats_requests(&rig.sent()), vec![vec![eid("a")]]);
    rig.frame();
    assert!(rig.sent().is_empty(), "the same selection asks for nothing");
    rig.marquee((-20.0, -20.0), (600.0, 300.0));
    rig.frame();
    assert_eq!(stats_requests(&rig.sent()), vec![vec![eid("a"), eid("b")]]);
    rig.click(900.0, 600.0);
    rig.frame();
    assert!(
        stats_requests(&rig.sent()).is_empty(),
        "an empty one asks nothing"
    );
    assert!(
        rig.view.selection_data.is_empty(),
        "and the band data is cleared"
    );
}

fn plan(rig: &mut Rig) -> (bool, bool) {
    let p = rig.view.plan_repaint(ambre_bus::PageTime(0.0), false);
    (
        matches!(p.content, ContentRepaint::Full),
        matches!(p.overlay, OverlayRepaint::Redraw),
    )
}

/// `TOILE-UI-021`, `TOILE-UI-271`: a quiet frame plans no draw. New board
/// data repaints the content, and a selection change the overlay alone.
#[test]
fn a_frame_repaints_only_what_changed() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    plan(&mut rig);
    assert_eq!(plan(&mut rig), (false, false), "nothing changed");
    rig.show(vec![DisplayNode::Element(sticky("a", B))]);
    assert_eq!(plan(&mut rig), (true, true), "the content changed");
    assert_eq!(plan(&mut rig), (false, false));
    rig.click(400.0, 50.0);
    assert_eq!(plan(&mut rig), (false, true), "the overlay alone");
    rig.marquee((-20.0, 200.0), (100.0, 300.0));
    assert_eq!(plan(&mut rig), (false, true), "a marquee is overlay");
    // A move paints the element at its override, in the content pass.
    rig.press(rig.screen(400.0, 50.0));
    let (x, y) = rig.screen(430.0, 80.0);
    rig.input(InputEvent::PointerMove { x, y });
    assert_eq!(plan(&mut rig), (true, true), "a move is content");
}

/// `TOILE-UI-272`: an open editor and a gesture in flight keep the overlay
/// repainting each frame, with no other change.
#[test]
fn an_editor_or_a_gesture_keeps_the_overlay_repainting() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    assert!(rig.view.editor.is_some());
    for _ in 0..3 {
        assert!(plan(&mut rig).1, "the editor keeps the overlay live");
    }
    rig.key("Escape", false, false);
    rig.paint();
    plan(&mut rig);

    rig.press(rig.screen(50.0, 50.0));
    let (x, y) = rig.screen(80.0, 70.0);
    rig.input(InputEvent::PointerMove { x, y });
    for _ in 0..3 {
        assert!(plan(&mut rig).1, "the gesture keeps the overlay live");
    }
}

/// A group node over `bounds`, itself a member of `parent` when one is given.
fn group_node(id: &str, bounds: Rect, parent: Option<&str>) -> DisplayNode {
    DisplayNode::Element(DisplayElement {
        geom: ambre_bus::app::toile::DisplayGeom::Group,
        group: parent.map(eid),
        ..shape(id, bounds)
    })
}

/// `TOILE-UI-032`: a double-click enters one nesting level of the group
/// under it, never more. The first one enters the outer group and selects
/// the inner one; the second enters the inner group and selects the leaf.
#[test]
fn a_double_click_enters_one_nesting_level() {
    let mut rig = Rig::new();
    let leaf = DisplayElement {
        group: Some(eid("inner")),
        ..shape("a", A)
    };
    rig.show(vec![
        group_node("outer", A, None),
        group_node("inner", A, Some("outer")),
        DisplayNode::Element(leaf),
    ]);
    rig.double_click(50.0, 50.0);
    assert_eq!(rig.view.group_scope, Some(eid("outer")));
    assert_eq!(rig.view.selection, [eid("inner")]);
    rig.double_click(50.0, 50.0);
    assert_eq!(rig.view.group_scope, Some(eid("inner")));
    assert_eq!(rig.view.selection, [eid("a")]);
}

/// `TOILE-UI-240`: a double-click on an element in no group leaves the scope
/// as it was, entered or not.
#[test]
fn a_double_click_on_an_ungrouped_element_keeps_the_scope() {
    let mut rig = Rig::new();
    rig.show(vec![DisplayNode::Element(shape("a", A))]);
    rig.double_click(50.0, 50.0);
    assert_eq!(rig.view.group_scope, None);
    rig.view.group_scope = Some(eid("g"));
    rig.double_click(50.0, 50.0);
    assert_eq!(rig.view.group_scope, Some(eid("g")));
}

/// A board showing one sticky at `A` that links to the frame "f".
fn linked_to_a_frame() -> Rig {
    let mut rig = Rig::new();
    let mut linked = sticky("a", A);
    linked.link = Some(LinkTarget::Frame(eid("f")));
    rig.show(vec![DisplayNode::Element(linked)]);
    rig.sent();
    rig
}

fn ctrl_click(rig: &mut Rig, x: f64, y: f64) {
    let (x, y) = rig.screen(x, y);
    rig.input(InputEvent::PointerDown {
        x,
        y,
        button: 0,
        shift: false,
        ctrl: true,
    });
    rig.input(InputEvent::PointerUp {
        x,
        y,
        button: 0,
        ctrl: true,
    });
}

/// Whether `sent` is the one fit request a jump to the frame "f" makes.
fn asks_for_the_frame(sent: &[ToileCommand]) -> bool {
    matches!(
        sent,
        [ToileCommand::RequestSelectionStats { ids, kind: StatsRequestKind::Fit }]
            if ids == &[eid("f")]
    )
}

/// The reply a fit request gets: the frame's box, or none for an id the
/// board no longer holds.
fn fit_reply(bounds: Option<Rect>) -> ToileEvent {
    ToileEvent::SelectionStats(ambre_bus::app::toile::SelectionStats {
        count: u32::from(bounds.is_some()),
        bounds,
        kind: StatsRequestKind::Fit,
    })
}

/// `TOILE-112`: a plain click on a linked element selects it and follows
/// nothing. A modified click follows, and so does a plain click once editing
/// is off for a presentation.
#[test]
fn a_plain_click_selects_a_linked_element_and_a_modified_click_follows() {
    let mut rig = linked_to_a_frame();
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a")]);
    assert!(
        !rig.sent().iter().any(
            |c| matches!(c, ToileCommand::RequestSelectionStats { ids, .. } if ids == &[eid("f")])
        ),
        "a plain click follows nothing"
    );

    let mut rig = linked_to_a_frame();
    ctrl_click(&mut rig, 50.0, 50.0);
    assert!(asks_for_the_frame(&rig.sent()), "a modified click follows");
    assert!(rig.view.selection.is_empty(), "and selects nothing");

    let mut rig = linked_to_a_frame();
    rig.view.present = true;
    rig.click(50.0, 50.0);
    assert!(
        asks_for_the_frame(&rig.sent()),
        "presenting, a click follows"
    );
}

/// `TOILE-113`: the board answers the frame's box, off screen too, and the
/// camera fits to it.
#[test]
fn a_jump_fits_the_camera_to_the_frames_box() {
    let mut rig = linked_to_a_frame();
    ctrl_click(&mut rig, 50.0, 50.0);
    rig.sent();
    let far = Rect::new(5000.0, 5000.0, 5400.0, 5300.0);
    rig.answer(fit_reply(Some(far)));
    let seen = rig.view.cam.visible_world();
    assert!(
        seen.contains(far.origin()) && seen.contains(Point::new(far.x1, far.y1)),
        "{seen:?} frames {far:?}"
    );
    assert!(
        seen.width() < 2.0 * far.width(),
        "fitted, not merely panned"
    );
}

/// `TOILE-064`: a jump to a frame the board no longer holds gets no box. The
/// camera stays, nothing is sent, and the user is told nothing.
#[test]
fn a_jump_to_a_frame_nobody_holds_does_nothing_and_says_nothing() {
    let mut rig = linked_to_a_frame();
    let before = rig.view.cam;
    ctrl_click(&mut rig, 50.0, 50.0);
    assert!(asks_for_the_frame(&rig.sent()));
    rig.answer(fit_reply(None));
    rig.frame();
    assert_eq!(rig.view.cam, before, "the camera stays");
    assert!(rig.notices.take().is_empty(), "no error is reported");
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::SetViewport(_))),
        "and no frame is asked for"
    );
}

/// `TOILE-115`: a `Ctrl+Z` after a jump sends one undo for the last edit.
/// The camera stays where the link put it: the jump is not in the history.
#[test]
fn an_undo_after_a_jump_reverts_the_edit_and_not_the_jump() {
    let mut rig = linked_to_a_frame();
    ctrl_click(&mut rig, 50.0, 50.0);
    rig.sent();
    rig.answer(fit_reply(Some(Rect::new(5000.0, 5000.0, 5400.0, 5300.0))));
    rig.frame();
    rig.sent();
    let jumped = rig.view.cam;
    assert!(rig.key("z", true, false));
    assert!(matches!(rig.sent().as_slice(), [ToileCommand::Undo]));
    assert_eq!(rig.view.cam, jumped, "the camera stays on the frame");
}

/// A right press and release at the world point (x, y).
fn right_click(rig: &mut Rig, x: f64, y: f64) {
    let (x, y) = rig.screen(x, y);
    rig.input(InputEvent::PointerDown {
        x,
        y,
        button: 2,
        shift: false,
        ctrl: false,
    });
    rig.input(InputEvent::PointerUp {
        x,
        y,
        button: 2,
        ctrl: false,
    });
}

/// `TOILE-UI-048`, `TOILE-UI-135`, `TOILE-UI-136`: a right-click on a
/// selected element keeps the whole selection. It sends no board command
/// and leaves the camera where it was.
#[test]
fn a_right_click_inside_the_selection_keeps_it() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.view.selection = vec![eid("a"), eid("b")];
    let cam = rig.view.cam;
    right_click(&mut rig, 50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a"), eid("b")]);
    assert!(rig.sent().is_empty(), "a right-click writes nothing");
    assert_eq!(rig.view.cam, cam, "a right-click moves no camera");
}

/// `TOILE-UI-127`, `TOILE-UI-135`, `TOILE-UI-136`: a right-click on an
/// element outside the selection collapses the selection to it. It writes
/// nothing and moves no camera.
#[test]
fn a_right_click_outside_the_selection_collapses_it() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.view.selection = vec![eid("a")];
    let cam = rig.view.cam;
    right_click(&mut rig, 400.0, 50.0);
    assert_eq!(rig.view.selection, [eid("b")]);
    assert!(rig.sent().is_empty(), "a right-click writes nothing");
    assert_eq!(rig.view.cam, cam, "a right-click moves no camera");
}

/// `TOILE-UI-128`: a right-click in the gap between two selected elements
/// hits nothing, so it clears the selection although the point lies inside
/// the selection box.
#[test]
fn a_right_click_on_nothing_clears_the_selection() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.view.selection = vec![eid("a"), eid("b")];
    right_click(&mut rig, 250.0, 60.0);
    assert!(rig.view.selection.is_empty());
    assert!(rig.sent().is_empty(), "a right-click writes nothing");
}

/// The key event the keymap binds to `id`, as the host would deliver it.
fn chord_of(id: CommandId) -> InputEvent {
    let binding = crate::ui::commands::chord(id).expect("the id announces a chord");
    let key = match binding.key {
        ambre_commands::Key::Char(c) if binding.mods.shift => c.to_ascii_uppercase().to_string(),
        ambre_commands::Key::Char(c) => c.to_string(),
        ambre_commands::Key::Delete => "Delete".to_owned(),
        other => panic!("no board chord decodes {other:?}"),
    };
    InputEvent::Key {
        key,
        ctrl: binding.mods.primary,
        alt: binding.mods.alt,
        shift: binding.mods.shift,
        meta: false,
    }
}

/// A board of two notes and one group, with `selection` selected and its
/// data answered, the way a menu or a chord finds it.
fn chord_board(selection: &[&str]) -> Rig {
    let group = DisplayElement {
        kind: ElementKind::Group,
        geom: ambre_bus::app::toile::DisplayGeom::Group,
        ..sticky("g", AWAY)
    };
    let elements = vec![sticky("a", A), sticky("b", B), group];
    let mut rig = Rig::new();
    rig.show(elements.iter().cloned().map(DisplayNode::Element).collect());
    rig.sent();
    rig.view.selection = selection.iter().copied().map(eid).collect();
    rig.answer(ToileEvent::SelectionData {
        elements: elements
            .into_iter()
            .filter(|e| selection.contains(&e.id.0.as_str()))
            .collect(),
        framed: Vec::new(),
        missing: Vec::new(),
    });
    rig.sent();
    rig
}

/// `TOILE-UI-052`: the shortcut a menu shows beside an entry does what the
/// entry does. Each announced board chord is pressed on one board, and its
/// entry is picked on a twin. Both send the same commands and leave the
/// same selection.
#[test]
fn a_shown_shortcut_does_what_its_entry_does() {
    use crate::ui::{menu, menu_bar};
    let cases: [(CommandId, &[&str]); 7] = [
        (menu::DUPLICATE, &["a", "b"]),
        (menu::DELETE, &["a", "b"]),
        (menu::GROUP, &["a", "b"]),
        (menu::UNGROUP, &["g"]),
        (menu::SELECT_ALL, &["a"]),
        (menu_bar::UNDO, &["a"]),
        (menu_bar::REDO, &["a"]),
    ];
    for (id, selection) in cases {
        let mut by_key = chord_board(selection);
        by_key.input(chord_of(id));
        by_key.frame();
        let mut by_menu = chord_board(selection);
        by_menu.view.apply_menu_command(id);
        by_menu.frame();
        assert_eq!(
            format!("{:?}", by_key.sent()),
            format!("{:?}", by_menu.sent()),
            "{id:?} sends another command"
        );
        assert_eq!(
            by_key.view.selection, by_menu.view.selection,
            "{id:?} leaves another selection"
        );
    }
}

/// `TOILE-UI-157`: zoom in, zoom out and zoom to 100 % each move the camera
/// and queue a fresh cull for the new view.
#[test]
fn a_zoom_entry_moves_the_camera_and_queues_a_cull() {
    use crate::ui::menu_bar::{ZOOM_IN, ZOOM_OUT, ZOOM_RESET};
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.cam.zoom = 2.0;
    for id in [ZOOM_IN, ZOOM_OUT, ZOOM_RESET] {
        let before = rig.view.cam.zoom;
        rig.view.apply_menu_command(id);
        assert_ne!(rig.view.cam.zoom, before, "{id:?} moves no camera");
        rig.view.update();
        assert!(
            rig.sent()
                .iter()
                .any(|c| matches!(c, ToileCommand::SetViewport(_))),
            "{id:?} queues no cull"
        );
    }
    assert_eq!(rig.view.cam.zoom, 1.0, "reset lands on 100 %");
}

/// `TOILE-UI-158`: a fit asks the Logic thread for the extent first. The
/// camera stays put until the answer comes back.
#[test]
fn a_fit_asks_for_the_extent_before_it_moves() {
    use crate::ui::menu_bar::{ZOOM_FIT, ZOOM_TO_SELECTION};
    for id in [ZOOM_FIT, ZOOM_TO_SELECTION] {
        let mut rig = Rig::with_stickies(&[("a", A)]);
        rig.view.selection = vec![eid("a")];
        let cam = rig.view.cam;
        rig.view.apply_menu_command(id);
        assert!(matches!(
            rig.sent().as_slice(),
            [ToileCommand::RequestSelectionStats {
                kind: StatsRequestKind::Fit,
                ..
            }]
        ));
        assert_eq!(rig.view.cam, cam, "{id:?} moved before the answer");
        rig.answer(fit_reply(Some(Rect::new(5000.0, 5000.0, 5400.0, 5300.0))));
        assert_ne!(rig.view.cam, cam, "{id:?} ignored its answer");
    }
}

/// `TOILE-UI-159`: a pointer-mode entry stays in the view. It sends nothing
/// and leaves the camera where it was.
#[test]
fn a_pointer_mode_entry_stays_in_the_view() {
    use crate::ui::menu_bar::{POINTER_AUTO, POINTER_MOUSE, POINTER_TRACKPAD};
    let mut rig = Rig::with_stickies(&[("a", A)]);
    let cam = rig.view.cam;
    for (id, mode) in [
        (POINTER_MOUSE, PointerMode::Mouse),
        (POINTER_TRACKPAD, PointerMode::Trackpad),
        (POINTER_AUTO, PointerMode::Auto),
    ] {
        rig.view.apply_menu_command(id);
        assert!(rig.view.pointer_mode == mode, "{id:?} set another mode");
        rig.view.update();
        assert!(rig.sent().is_empty(), "{id:?} reached a thread");
        assert_eq!(rig.view.cam, cam, "{id:?} moved the camera");
    }
}

/// `TOILE-UI-041`, `TOILE-UI-255`: with `Space` held, a drag pans in every
/// tool, and the tool's own gesture never starts.
#[test]
fn a_space_drag_pans_in_any_tool_and_starts_no_gesture() {
    for tool in [
        Tool::Select,
        Tool::Draw,
        Tool::Shape(ShapeKind::Rect),
        Tool::Connector,
    ] {
        let mut rig = Rig::with_stickies(&[("a", A)]);
        rig.view.set_tool(tool);
        let cam = rig.view.cam;
        assert!(rig.key(" ", false, false));
        drag(&mut rig, (50.0, 50.0), (150.0, 90.0));
        assert_ne!(rig.view.cam, cam, "{tool:?}: the drag did not pan");
        assert!(rig.view.selection.is_empty(), "{tool:?}: the drag selected");
        assert!(
            !rig.sent().iter().any(|c| !matches!(
                c,
                ToileCommand::SetViewport(_) | ToileCommand::RequestSelectionData { .. }
            )),
            "{tool:?}: the drag wrote to the board"
        );
    }
}

/// `TOILE-UI-038`, `TOILE-UI-259`: a text element placed with the tool
/// returns the board to Select and opens its editor once its id comes back.
#[test]
fn a_placed_text_opens_its_editor_and_the_tool_returns_to_select() {
    let mut rig = Rig::new();
    rig.sent();
    rig.view.set_tool(Tool::Text);
    rig.click(600.0, 400.0);
    let token = rig.sent().iter().find_map(|c| match c {
        ToileCommand::CreateElement {
            kind: ElementKind::Text,
            correlate,
            ..
        } => Some(*correlate),
        _ => None,
    });
    assert_eq!(rig.view.machine.tool(), Tool::Select, "one text a click");
    let token = token.expect("the click places a text");
    rig.answer(ToileEvent::ElementCreated {
        id: eid("t"),
        correlate: token,
    });
    let text = DisplayElement {
        kind: ElementKind::Text,
        ..sticky("t", A)
    };
    rig.show(vec![DisplayNode::Element(text)]);
    assert_eq!(editing(&rig), Some(eid("t")));
}

/// `TOILE-UI-170`: the bar hides while a move is in flight and comes back
/// once the element lands.
#[test]
fn the_bar_hides_during_a_move() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.answer(ToileEvent::SelectionData {
        elements: vec![sticky("a", A)],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    assert!(rig.view.float_bar_at((0.0, 0.0)).is_some(), "the bar shows");
    let (from, to) = (rig.screen(50.0, 50.0), rig.screen(90.0, 70.0));
    rig.press(from);
    rig.input(InputEvent::PointerMove { x: to.0, y: to.1 });
    assert!(
        rig.view.float_bar_at((0.0, 0.0)).is_none(),
        "the bar follows the drag"
    );
    rig.release(to);
    assert!(
        rig.view.float_bar_at((0.0, 0.0)).is_some(),
        "the bar is back"
    );
}

/// `TOILE-UI-176`, `TOILE-UI-182`, `TOILE-UI-173`: a panel restyle sends one
/// whole style to every selected id, the locked one included. Each member
/// gets the same value, a field it held mixed too, and the selection stays.
#[test]
fn a_panel_restyle_names_every_member_and_keeps_the_selection() {
    let mut rig = Rig::new();
    let a = sticky("a", A);
    let b = DisplayElement {
        locked: true,
        style: ElementStyle {
            stroke_w: 4.0,
            ..ElementStyle::default()
        },
        ..sticky("b", B)
    };
    rig.show(vec![
        DisplayNode::Element(a.clone()),
        DisplayNode::Element(b.clone()),
    ]);
    rig.view.selection = vec![eid("a"), eid("b")];
    rig.answer(ToileEvent::SelectionData {
        elements: vec![a, b],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    rig.sent();
    let model = crate::ui::style::style_scope_for(&rig.view.selection, &rig.view.selection_data)
        .expect("a resolved selection has a model");
    assert!(model.mixed.stroke_w, "the stroke width is mixed");
    let mut style = model.base;
    crate::ui::floatbar::StyleEdit::Fill(Some(Argb::from_bits(0xff11_2233))).apply_to(&mut style);
    rig.view.apply_restyle(style.clone());
    match rig.sent().as_slice() {
        [ToileCommand::RestyleElements { ids, style: sent }] => {
            assert_eq!(ids, &[eid("a"), eid("b")], "the locked member is named");
            assert_eq!(sent, &style, "one whole value for every member");
        }
        other => panic!("one restyle expected, got {other:?}"),
    }
    assert_eq!(rig.view.selection, [eid("a"), eid("b")]);
}

/// `TOILE-UI-179`, `TOILE-UI-181`: each bar edit starts from the style the
/// selection has now, so a second edit keeps the first. Neither changes
/// what is selected.
#[test]
fn two_bar_edits_compose_and_keep_the_selection() {
    use crate::ui::floatbar::StyleEdit;
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.selection = vec![eid("a")];
    rig.answer(ToileEvent::SelectionData {
        elements: vec![sticky("a", A)],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    rig.sent();
    rig.view
        .apply_quick_style(StyleEdit::Fill(Some(Argb::from_bits(0xff11_2233))));
    let first = match rig.sent().as_slice() {
        [ToileCommand::RestyleElements { style, .. }] => style.clone(),
        other => panic!("one restyle expected, got {other:?}"),
    };
    // The Logic thread echoes the written style back with the selection.
    rig.answer(ToileEvent::SelectionData {
        elements: vec![DisplayElement {
            style: first,
            ..sticky("a", A)
        }],
        framed: Vec::new(),
        missing: Vec::new(),
    });
    rig.sent();
    rig.view.apply_quick_style(StyleEdit::Bold(true));
    match rig.sent().as_slice() {
        [ToileCommand::RestyleElements { ids, style }] => {
            assert_eq!(ids, &[eid("a")]);
            assert_eq!(
                style.fill,
                Some(Argb::from_bits(0xff11_2233)),
                "the first edit is kept"
            );
            assert_eq!(style.font.weight, 700);
        }
        other => panic!("one restyle expected, got {other:?}"),
    }
    assert_eq!(rig.view.selection, [eid("a")]);
}

// -- Comments, reactions and presence (`toile-comments`, `toile-reactions`,
// `toile-presence`) -----------------------------------------------------------

/// A peer whose sample puts their cursor at the world point (x, y) and
/// selects `selection`.
fn peer_at(user_id: UserId, x: f64, y: f64, selection: &[&str]) -> PresenceUpdate {
    PresenceUpdate {
        user_id,
        presence: PresencePayload {
            cursor: Some(kurbo::Point::new(x, y)),
            selection: selection.iter().map(|id| eid(id)).collect(),
            ..Default::default()
        },
    }
}

/// A board showing sticky `a` with a reaction under it, an open thread
/// pinned beside it, and one peer whose cursor sits on it.
fn a_shared_board(peer: UserId) -> Rig {
    let mut rig = Rig::new();
    let mut reactions = std::collections::BTreeMap::new();
    reactions.insert(
        eid("a"),
        ambre_bus::app::toile::ReactionTally {
            emoji: vec![("+1".to_owned(), 2)],
            vote_total: 0,
            vote_count: 0,
        },
    );
    rig.answer(ToileEvent::ViewportData(DisplayViewport {
        version: ambre_bus::ViewportVersion(0),
        grid_unit: 20.0,
        pins: vec![ambre_bus::app::toile::CommentPin {
            thread: eid("t"),
            at: kurbo::Point::new(50.0, 50.0),
            messages: 1,
            resolved: false,
        }],
        reactions,
        viewport: Viewport::default(),
        nodes: vec![DisplayNode::Element(sticky("a", A))],
    }));
    rig.answer(ToileEvent::PeerPresence(vec![peer_at(
        peer,
        50.0,
        50.0,
        &["a"],
    )]));
    rig.sent();
    rig
}

/// Paint the content pass and the overlay pass, each into its own surface.
fn paint_passes(
    rig: &mut Rig,
) -> (
    ambre_draw::testing::RecordingSurface,
    ambre_draw::testing::RecordingSurface,
) {
    let mut font_cx = FontContext::new();
    let mut cache = TextCache::new(1.0);
    let mut content = ambre_draw::testing::RecordingSurface::new();
    rig.view
        .paint_content(&mut content, &mut font_cx, &mut cache, None);
    let mut overlay = ambre_draw::testing::RecordingSurface::new();
    rig.view.paint_overlay(
        &mut overlay,
        &mut font_cx,
        &mut cache,
        &FrameStats::default(),
    );
    (content, overlay)
}

/// The colour an op paints in, when it has one.
fn op_color(op: &ambre_draw::testing::DrawOp) -> Option<ambre_draw::Color> {
    use ambre_draw::testing::DrawOp;
    match op {
        DrawOp::FillRect { color, .. }
        | DrawOp::StrokeRect { color, .. }
        | DrawOp::StrokeLine { color, .. } => Some(*color),
        DrawOp::Glyphs { color, .. }
        | DrawOp::FillPath { color }
        | DrawOp::StrokePath { color, .. } => *color,
        _ => None,
    }
}

/// The indices of the ops of `surface` filled in `color`, in paint order.
fn fills_in(
    surface: &ambre_draw::testing::RecordingSurface,
    color: ambre_draw::Color,
) -> Vec<usize> {
    use ambre_draw::testing::DrawOp;
    surface
        .ops()
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == color))
        .map(|(i, _)| i)
        .collect()
}

/// `TOILE-UI-219`, `TOILE-UI-299` and `TOILE-UI-306`: the board paints in
/// its own pass, under the overlay pass. In the overlay, the peers paint
/// first, the badge row next, and the pins last.
#[test]
fn the_overlays_stack_peers_then_badges_then_pins() {
    let peer = UserId::new();
    let mut rig = a_shared_board(peer);
    let (content, overlay) = paint_passes(&mut rig);
    let palette = rig.view.theme.palette();
    let ink = ambre_draw::peer_color(&peer);

    assert!(
        !content.ops().iter().any(|op| op_color(op) == Some(ink)),
        "no peer paints in the board pass"
    );
    let peers: Vec<usize> = (0..overlay.ops().len())
        .filter(|i| op_color(&overlay.ops()[*i]) == Some(ink))
        .collect();
    let badges = fills_in(&overlay, palette.header_bg);
    let pins = fills_in(&overlay, palette.selection);
    assert!(!peers.is_empty() && !badges.is_empty() && !pins.is_empty());
    assert!(
        peers.iter().max() < badges.iter().min(),
        "the badges paint above the peers"
    );
    assert!(
        badges.iter().max() < pins.iter().min(),
        "the pins paint above the badges"
    );
}

/// `TOILE-UI-299`: a presentation still paints the badge row above the
/// board, while it leaves the peers off the slide.
#[test]
fn a_presentation_keeps_the_badges() {
    let peer = UserId::new();
    let mut rig = a_shared_board(peer);
    rig.answer(ToileEvent::PresentModeChanged(PresentMode::Presenting));
    let (_, overlay) = paint_passes(&mut rig);
    let palette = rig.view.theme.palette();
    assert!(
        !fills_in(&overlay, palette.header_bg).is_empty(),
        "the badges paint in a presentation"
    );
    let ink = ambre_draw::peer_color(&peer);
    assert!(!overlay.ops().iter().any(|op| op_color(op) == Some(ink)));
}

/// `TOILE-180`: a sample arriving from a peer moves neither the camera nor
/// the selection of this viewer, even when it selects something else.
#[test]
fn an_arriving_sample_keeps_the_camera_and_the_selection() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.view.selection = vec![eid("a")];
    let camera = rig.view.cam;
    rig.answer(ToileEvent::PeerPresence(vec![peer_at(
        UserId::new(),
        900.0,
        900.0,
        &["b"],
    )]));
    rig.frame();
    assert_eq!(rig.view.selection, [eid("a")]);
    assert_eq!(rig.view.cam, camera, "the camera did not move");
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, ToileCommand::SetViewport(_))),
        "no camera change reaches the board"
    );
}

/// `TOILE-UI-220`: opening another board drops every peer of the first. The
/// overlay paints none of them over the new board.
#[test]
fn opening_another_board_drops_its_peers() {
    let peer = UserId::new();
    let mut rig = a_shared_board(peer);
    rig.view.set_document(DocumentId::new());
    rig.show(vec![DisplayNode::Element(sticky("a", A))]);
    let (_, overlay) = paint_passes(&mut rig);
    let ink = ambre_draw::peer_color(&peer);
    assert!(
        !overlay.ops().iter().any(|op| op_color(op) == Some(ink)),
        "no peer of the first board paints on the second"
    );
}

/// `TOILE-UI-073`: one gesture's sample carries the cursor, the selection
/// and the stroke in flight, together.
#[test]
fn a_gesture_sample_carries_the_cursor_the_selection_and_the_stroke() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.view.selection = vec![eid("a")];
    rig.view.set_tool(Tool::Draw);
    rig.press(rig.screen(500.0, 500.0));
    let (x, y) = rig.screen(530.0, 500.0);
    rig.input(InputEvent::PointerMove { x, y });
    rig.frame();
    let samples: Vec<PresencePayload> = rig
        .sent()
        .into_iter()
        .filter_map(|c| match c {
            ToileCommand::Presence { payload, .. } => Some(payload),
            _ => None,
        })
        .collect();
    let [sample] = samples.as_slice() else {
        panic!("one sample a frame, got {samples:?}");
    };
    let cursor = sample.cursor.expect("the cursor");
    assert!((cursor.x - 530.0).abs() < 1e-6 && (cursor.y - 500.0).abs() < 1e-6);
    assert_eq!(sample.selection, [eid("a")]);
    assert!(
        sample.live_stroke.as_ref().is_some_and(|s| s.len() >= 2),
        "the stroke in flight: {:?}",
        sample.live_stroke
    );
}

/// `TOILE-UI-309`: a thread on a point brings the camera to that point at
/// once. A thread on an element asks the board for its box, and the camera
/// moves to its centre when the answer lands.
#[test]
fn a_thread_brings_the_camera_to_its_anchor() {
    let mut rig = Rig::new();
    rig.sent();
    rig.view.focus_thread(&CommentAnchor::Point {
        x: 700.0,
        y: -300.0,
    });
    assert_eq!(rig.view.cam.center, kurbo::Point::new(700.0, -300.0));

    rig.view.focus_thread(&CommentAnchor::Element(eid("far")));
    let asked = rig.sent().iter().any(|c| {
        matches!(
            c,
            ToileCommand::RequestSelectionStats { ids, kind: StatsRequestKind::CommentFocus }
                if ids == &[eid("far")]
        )
    });
    assert!(asked, "the view asks for the anchor's box");
    rig.answer(ToileEvent::SelectionStats(
        ambre_bus::app::toile::SelectionStats {
            count: 1,
            bounds: Some(Rect::new(4000.0, 4000.0, 4100.0, 4200.0)),
            kind: StatsRequestKind::CommentFocus,
        },
    ));
    assert_eq!(rig.view.cam.center, kurbo::Point::new(4050.0, 4100.0));
}

/// `TOILE-UI-310`: a changed thread is read again while the panel is open:
/// the list always, and the thread itself when the panel shows it. A closed
/// panel reads nothing.
#[test]
fn a_changed_thread_is_read_again_while_the_panel_shows_it() {
    let reads = |rig: &mut Rig| -> Vec<Option<ThreadId>> {
        rig.sent()
            .into_iter()
            .filter_map(|c| match c {
                ToileCommand::ReadComments { thread } => Some(thread),
                _ => None,
            })
            .collect()
    };
    let mut rig = Rig::new();
    rig.sent();
    let t = eid("t");

    rig.view.reread_comments_shown(true, Some(&t), &t);
    assert_eq!(reads(&mut rig), [None, Some(t.clone())]);
    rig.view
        .reread_comments_shown(true, Some(&eid("other")), &t);
    assert_eq!(reads(&mut rig), [None], "the list alone");
    rig.view.reread_comments_shown(false, None, &t);
    assert!(reads(&mut rig).is_empty(), "a closed panel reads nothing");
}

/// `TOILE-FEAT-098`: a deck that moved is read again while the panel is open,
/// and a closed panel reads nothing.
#[test]
fn a_changed_deck_is_read_again_while_the_panel_is_open() {
    let mut rig = Rig::new();
    rig.sent();
    rig.view.reread_deck_shown(true);
    assert!(matches!(rig.sent().as_slice(), [ToileCommand::ReadDeck]));
    rig.view.reread_deck_shown(false);
    assert!(rig.sent().is_empty(), "a closed panel reads nothing");
}

/// `TOILE-UI-083`: a presentation takes no edit. A delete key, an undo chord,
/// a selecting click and an opening double click each send nothing.
#[test]
fn a_presentation_takes_no_edit() {
    let mut rig = Rig::with_stickies(&[("a", A), ("b", B)]);
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a")]);
    rig.answer(ToileEvent::PresentModeChanged(PresentMode::Presenting));
    assert!(rig.view.selection.is_empty(), "the selection is dropped");

    rig.click(350.0, 50.0);
    assert!(rig.view.selection.is_empty(), "a click selects nothing");
    rig.view.selection = vec![eid("a")];
    rig.key("Delete", false, false);
    rig.key("z", true, false);
    rig.key("d", true, false);
    rig.double_click(50.0, 50.0);
    assert!(rig.view.editor.is_none(), "no editor opens");
    assert!(rig.sent().is_empty(), "a presentation sent an edit");
    assert_eq!(rig.view.selection, [eid("a")], "the delete was refused");

    rig.answer(ToileEvent::PresentModeChanged(PresentMode::Editing));
    rig.key("Delete", false, false);
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::DeleteElements { .. }]
    ));
}

/// `TOILE-UI-083`: an editor open as a presentation begins closes and keeps
/// what was typed in it.
#[test]
fn a_presentation_closes_the_open_editor() {
    let mut rig = Rig::with_stickies(&[("a", A)]);
    rig.click(50.0, 50.0);
    rig.double_click(50.0, 50.0);
    rig.type_text("!");
    rig.answer(ToileEvent::PresentModeChanged(PresentMode::Presenting));
    rig.type_text("?");
    rig.paint();
    assert!(rig.view.editor.is_none(), "the editor closed");
    assert!(matches!(
        rig.sent().as_slice(),
        [ToileCommand::SetText { text, .. }] if text == "hi!"
    ));
}

impl Rig {
    /// The board commands on the queue, without flushing the outbox.
    fn drain(&mut self) -> Vec<ToileCommand> {
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

/// `BUS-024`: a viewport request the full queue dropped stays in its slot,
/// and the next frame offers it again.
#[test]
fn a_dropped_viewport_request_is_offered_again() {
    let mut rig = Rig::with_queue(1);
    assert!(
        rig.view.send_edit(ToileCommand::RequestBoardIds),
        "the queue is full"
    );
    rig.view.update();
    let first = rig.drain();
    assert!(
        !first
            .iter()
            .any(|c| matches!(c, ToileCommand::SetViewport(_))),
        "the full queue took the viewport: {first:?}"
    );
    assert!(rig.view.pending_viewport.is_some());
    rig.view.update();
    let again = rig.drain();
    assert!(
        matches!(again.as_slice(), [ToileCommand::SetViewport(_)]),
        "sent {again:?}"
    );
    assert!(rig.view.pending_viewport.is_none());
}

/// `BUS-024`: a media request the full queue dropped is not marked in
/// flight, and the next frame offers it again.
#[test]
fn a_dropped_media_request_is_offered_again() {
    let mut rig = Rig::with_queue(2);
    assert!(
        rig.view.send_edit(ToileCommand::RequestBoardIds),
        "the queue is full"
    );
    rig.view.update();
    let mut picture = sticky("p", Rect::new(0.0, 0.0, 100.0, 100.0));
    picture.kind = ElementKind::Image;
    picture.geom = ambre_bus::app::toile::DisplayGeom::Image("h".to_owned());
    rig.show(vec![DisplayNode::Element(picture)]);
    let asks = |sent: &[ToileCommand]| {
        sent.iter()
            .filter(|c| matches!(c, ToileCommand::RequestMedia { hashes } if hashes == &["h"]))
            .count()
    };
    assert_eq!(asks(&rig.drain()), 0, "the full queue took the request");
    assert!(rig.view.media_requested.is_empty());
    rig.view.update();
    assert_eq!(asks(&rig.drain()), 1);
    assert!(rig.view.media_requested.contains("h"));
    rig.view.update();
    assert_eq!(asks(&rig.drain()), 0, "a request in flight is asked once");
}

/// `BUS-022`: the board takes no input while its outbox holds an edit, and
/// takes it again once the queue drains.
#[test]
fn a_busy_board_takes_no_input() {
    let mut rig = Rig::with_queue(1);
    rig.show(vec![DisplayNode::Element(sticky(
        "a",
        Rect::new(0.0, 0.0, 100.0, 100.0),
    ))]);
    assert!(rig.view.send_edit(ToileCommand::RequestBoardIds));
    assert!(rig.view.is_busy());
    let at = rig.screen(50.0, 50.0);
    assert!(
        rig.input(InputEvent::PointerDown {
            x: at.0,
            y: at.1,
            button: 0,
            shift: false,
            ctrl: false,
        }),
        "the refusal consumes the event"
    );
    rig.release(at);
    assert!(rig.view.selection.is_empty(), "the busy board selected");
    rig.drain();
    rig.view.outbox.flush();
    rig.drain();
    assert!(!rig.view.is_busy());
    rig.click(50.0, 50.0);
    assert_eq!(rig.view.selection, [eid("a")]);
}

/// `UI-THREAD-001`: a canvas point reaches the element drawn there through the
/// shared hit-test, at any zoom. An empty point hits nothing.
#[test]
fn a_canvas_point_reaches_its_element_through_the_shared_hit_test() {
    use ambre_draw::HitTest;
    let mut rig = Rig::with_stickies(&[("a", A)]);
    for zoom in [0.5, 1.0, 2.0] {
        rig.view.cam.zoom = zoom;
        let (x, y) = rig.screen(50.0, 50.0);
        assert_eq!(rig.view.hit_test(x, y), Some(eid("a")), "zoom {zoom}");
        let (x, y) = rig.screen(900.0, 600.0);
        assert_eq!(rig.view.hit_test(x, y), None, "zoom {zoom}");
    }
}

/// `KINETIC-015`: a line-mode wheel moves the board by the pixels one line is
/// worth, in both modes, as Trame, Prose and Murmure read it.
#[test]
fn a_line_wheel_pans_and_zooms_as_its_pixels_do() {
    for (mode, name) in [(PointerMode::Trackpad, "pan"), (PointerMode::Mouse, "zoom")] {
        let mut by_line = Rig::new();
        by_line.view.pointer_mode = mode;
        wheel(&mut by_line, (0.0, 1.0), 1, false);
        let mut by_px = Rig::new();
        by_px.view.pointer_mode = mode;
        wheel(&mut by_px, (0.0, ambre_canvas::WHEEL_LINE_PX), 0, false);
        assert_eq!(by_line.view.cam.zoom, by_px.view.cam.zoom, "{name}");
        assert_eq!(by_line.screen(0.0, 0.0), by_px.screen(0.0, 0.0), "{name}");
    }
}
