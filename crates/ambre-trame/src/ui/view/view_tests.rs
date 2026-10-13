//! The grid view driven the way the host drives it. Input events go in and the
//! commands for the Logic thread come out. There is no DOM and no GPU. The text
//! pipe is detached, a frame paints into a recording surface, and the view
//! reads a clock the test moves.

use std::cell::Cell;
use std::rc::Rc;

use super::*;
use ambre_bus::conduit::Receiver;
use ambre_draw::testing::RecordingSurface;

/// A clock the test sets, shared with the view it was handed to.
#[derive(Clone, Default)]
struct TestClock(Rc<Cell<f64>>);

impl TestClock {
    fn set(&self, ms: f64) {
        self.0.set(ms);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> f64 {
        self.0.get()
    }
}

/// One grid view, the far end of its outbound channel, and its clock.
struct Rig {
    view: TrameView,
    to_logic: Receiver<LogicCommand>,
    /// The Logic thread's end of the view's inbound channel.
    from_logic: ambre_bus::conduit::Sender<UiEvent>,
    clock: TestClock,
    /// The switch the chrome toggles, shared with the view.
    theme: ThemeControl,
}

impl Rig {
    fn new() -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(256);
        let (from_logic, rx) = ambre_bus::conduit::local(256);
        let clock = TestClock::default();
        let theme = ThemeControl::default();
        let view = TrameView::new(
            DocumentId::new(),
            tx,
            rx,
            1200.0,
            800.0,
            TextInput::detached(),
            false,
            AnchorPublisher::new(),
            PerfPublisher::new(),
            BusyPublisher::new(),
            FilterOpenPublisher::new(),
            theme.clone(),
            Box::new(clock.clone()),
        );
        Self {
            view,
            to_logic,
            from_logic,
            clock,
            theme,
        }
    }

    fn key(&mut self, key: &str) -> bool {
        self.view.handle_input(&InputEvent::Key {
            key: key.to_owned(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
        })
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.view.handle_input(&InputEvent::Text {
            text: text.to_owned(),
        })
    }

    /// Run one frame as the host does: drain, plan, then paint into a
    /// recording surface, where a deferred commit lands.
    fn frame(&mut self) {
        self.view.update();
        self.view.plan_repaint(self.clock.now_ms(), false);
        let mut surface = RecordingSurface::new();
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
    }

    /// Answer as the Logic thread does, and run the frame that reads it.
    fn answer(&mut self, event: TrameEvent) {
        let sent = self.from_logic.try_send(UiEvent::Viewport {
            document_id: DocumentId::new(),
            event: AppEvent::Trame(event),
        });
        assert!(sent.is_ok(), "the view's inbound channel is open");
        self.frame();
    }

    /// The grid commands the view sent since the last call, in order.
    fn sent(&mut self) -> Vec<TrameCommand> {
        self.view.flush_outbox();
        self.to_logic
            .try_iter()
            .filter_map(|msg| match msg {
                LogicCommand::App {
                    command: AppCommand::Trame(cmd),
                    ..
                } => Some(cmd),
                _ => None,
            })
            .collect()
    }
}

/// A text typed on the grid opens an editor on the active cell. Enter commits
/// it at the next paint. That is the whole path from a key to the Logic
/// thread, with no browser under it.
#[test]
fn a_typed_value_reaches_the_logic_thread_through_the_view() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    assert!(rig.type_text("42"));
    assert!(rig.key("Enter"));
    rig.frame();
    let sent = rig.sent();
    // The session opens on A1, commits its text, closes its undo group, and
    // the cursor steps down to A2.
    assert!(
        matches!(
            sent.as_slice(),
            [
                TrameCommand::BeginCellEdit { session: s0, row: 0, col: 0 },
                TrameCommand::CommitCellEdit { session: s1, text },
                TrameCommand::CommitEdit,
                TrameCommand::SetActiveCell { row: 1, col: 0, .. },
            ] if s0 == s1 && text == "42"
        ),
        "sent {sent:?}"
    );
    assert!(rig.view.editor.is_none());
}

/// The caret blinks on the clock the view was handed, not on the browser's.
#[test]
fn the_caret_blinks_on_the_clock_the_view_is_handed() {
    let mut rig = Rig::new();
    rig.clock.set(1_000.0);
    rig.type_text("a");
    assert!(rig.view.caret_on);
    rig.clock.set(1_000.0 + BLINK_MS - 1.0);
    rig.frame();
    assert!(rig.view.caret_on, "half a period has not passed");
    rig.clock.set(1_000.0 + BLINK_MS);
    rig.frame();
    assert!(!rig.view.caret_on, "half a period passed");
}

/// A blob carrying a grid payload whose `src_row` names where it came from,
/// with `text` as its `text/plain` mirror.
fn clip(src_row: u32, text: &str) -> ambre_bus::ClipboardBlob {
    let payload = ambre_bus::ClipboardPayload {
        src_row,
        rows: 1,
        cols: 1,
        ..Default::default()
    };
    ambre_bus::ClipboardBlob {
        text: text.to_owned(),
        html: None,
        ambre: postcard::to_allocvec(&payload).ok(),
        plain: false,
    }
}

/// The payload one `PasteRange` carried, or `None` for any other traffic.
fn pasted(sent: &[TrameCommand]) -> Option<&ambre_bus::ClipboardPayload> {
    match sent {
        [TrameCommand::PasteRange { payload, .. }] => Some(payload),
        _ => None,
    }
}

/// Covers `CHROME-062`: a printable key is left to the text path, on the grid
/// and in an open editor alike.
#[test]
fn a_printable_key_is_not_consumed() {
    let mut rig = Rig::new();
    for key in ["a", "Z", "é", " ", "1", "="] {
        assert!(!rig.key(key), "{key:?} was consumed on the grid");
    }
    rig.type_text("x");
    for key in ["a", "Z", "é", " ", "1", "="] {
        assert!(!rig.key(key), "{key:?} was consumed in the editor");
    }
}

/// Covers `UI-THREAD-045`: with the Logic thread gone, the copy is refused, so
/// no cut is armed and no later paste clears the range.
#[test]
fn a_cut_whose_copy_went_nowhere_arms_nothing() {
    let Rig {
        mut view, to_logic, ..
    } = Rig::new();
    drop(to_logic);
    view.state.select_rect(0, 0, 1, 1);
    assert_eq!(view.clipboard_write(true), ambre_bus::CopyOutcome::Nothing);
    assert!(!view.cut_pending, "no cut is armed");
    assert_eq!(view.state.clipboard_anchor, None, "no range is marked");
}

/// Covers `UI-THREAD-022` and `UI-THREAD-046`: the kept copy answers on our
/// token, the clipboard's bytes on another, and its text when it has no bytes.
#[test]
fn a_paste_takes_the_highest_rung_it_can_serve() {
    let mut rig = Rig::new();
    rig.view.last_clip = Some(clip(7, "kept"));
    rig.sent();
    rig.view.paste_clipboard(clip(3, "read"), true);
    assert_eq!(pasted(&rig.sent()).map(|p| p.src_row), Some(7), "own copy");
    rig.view.paste_clipboard(clip(3, "read"), false);
    assert_eq!(
        pasted(&rig.sent()).map(|p| p.src_row),
        Some(3),
        "another copy"
    );
    let foreign = ambre_bus::ClipboardBlob {
        ambre: None,
        ..clip(3, "plain")
    };
    rig.view.paste_clipboard(foreign, false);
    let sent = rig.sent();
    let cells = pasted(&sent).map(|p| p.cells.iter().map(|c| c.raw.as_str()).collect::<Vec<_>>());
    assert_eq!(cells, Some(vec!["plain"]), "no bytes: the text, as cells");
}

/// Covers `UI-THREAD-046`: an open editor is the highest rung, so the text
/// goes in at the caret and no range is pasted.
#[test]
fn a_paste_into_an_open_editor_inserts_the_text() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("4");
    rig.view.paste_clipboard(clip(3, "2"), false);
    assert!(rig.key("Enter"));
    rig.frame();
    let sent = rig.sent();
    assert!(
        sent.iter()
            .any(|c| matches!(c, TrameCommand::CommitCellEdit { text, .. } if text == "42")),
        "sent {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, TrameCommand::PasteRange { .. }))
    );
}

/// Covers `UI-THREAD-047`: a paste leaves no blob for the host to write.
#[test]
fn a_paste_writes_nothing_back() {
    let mut rig = Rig::new();
    rig.view.paste_clipboard(clip(3, "read"), false);
    assert_eq!(rig.view.poll_clipboard(), None);
}

/// Covers `CHROME-007`: in an open editor the pipe sits on the caret inside
/// the cell and follows it as the text grows. A closed editor holds none.
#[test]
fn the_pipe_follows_the_caret_inside_the_cell() {
    let mut rig = Rig::new();
    rig.frame();
    assert_eq!(rig.view.caret_drawn, None, "no editor, no caret");
    rig.type_text("x");
    rig.frame();
    let first = rig.view.caret_drawn.expect("the editor drew a caret");
    let (x, y, w, h) = rig.view.state.cell_pixel_rect(0, 0).expect("A1 is in view");
    assert!(
        first.x0 > x && first.x0 < x + w,
        "inside the cell: {first:?}"
    );
    assert!(
        first.y0 >= y && first.y1 <= y + h,
        "inside the cell: {first:?}"
    );
    rig.type_text("yyyy");
    rig.frame();
    let later = rig.view.caret_drawn.expect("the editor drew a caret");
    assert!(later.x0 > first.x0, "the caret moved right: {later:?}");
    assert!(rig.key("Escape"));
    rig.frame();
    assert_eq!(rig.view.caret_drawn, None, "the editor closed");
}

/// Covers `CANVAS-015`: a theme switch repaints the whole grid on the next
/// frame, in the new palette.
///
/// An idle frame reuses the content layer. Once the chrome flips the theme,
/// the next plan rebuilds it and the grid fills in the new palette.
#[test]
fn a_theme_switch_repaints_the_content_in_the_new_palette() {
    let mut rig = Rig::new();
    rig.frame();
    rig.frame();
    rig.view.update();
    let idle = rig.view.plan_repaint(rig.clock.now_ms(), false);
    assert!(
        matches!(idle.content, ContentRepaint::Reuse),
        "an idle frame reuses"
    );

    let next = rig.theme.toggle();
    rig.view.update();
    let plan = rig.view.plan_repaint(rig.clock.now_ms(), false);
    assert!(
        matches!(plan.content, ContentRepaint::Full),
        "{:?}",
        plan.content
    );
    let mut surface = RecordingSurface::new();
    let mut font_cx = FontContext::new();
    let mut cache = TextCache::new(1.0);
    rig.view
        .paint_content(&mut surface, &mut font_cx, &mut cache, None);
    let fills = surface.fill_colors();
    // No viewport has landed, so the full repaint lays the pending fill.
    assert!(fills.contains(&next.palette().content_pending), "{fills:?}");
    assert!(
        !fills.contains(&next.toggled().palette().content_pending),
        "{fills:?}"
    );
}

/// Whether the cell is inside the grid's visible window.
fn in_view(rig: &Rig, (row, col): (u32, u32)) -> bool {
    let state = &rig.view.state;
    row >= state.top_row
        && state.visible_offset(row) < i64::from(state.viewport_rows)
        && col >= state.left_col
        && col < state.left_col + state.viewport_cols
}

/// `FIND-004`: the match the Logic thread elects is brought into view, even
/// far outside the window, and the active cell lands on it.
#[test]
fn the_current_match_is_brought_into_view() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let far = (900, 60);
    assert!(!in_view(&rig, far), "the match starts off screen");

    rig.answer(TrameEvent::FindResult {
        matches: vec![(0, 0), far],
        active: Some(1),
    });
    assert!(in_view(&rig, far), "the elected match is in view");
    assert_eq!(
        (rig.view.state.active_row(), rig.view.state.active_col()),
        far
    );
    let sent = rig.sent();
    assert!(
        sent.iter().any(
            |c| matches!(c, TrameCommand::ScrollViewport { row_start, row_end, .. }
                if (*row_start..=*row_end).contains(&far.0))
        ),
        "the window asks for the rows it now shows: {sent:?}"
    );
}

/// `FIND-005`: the empty answer an empty query or a closed panel gets clears
/// the highlight, and leaves the cursor where it was.
#[test]
fn an_empty_answer_clears_the_highlight() {
    let mut rig = Rig::new();
    rig.frame();
    rig.answer(TrameEvent::FindResult {
        matches: vec![(2, 1), (4, 1)],
        active: Some(0),
    });
    assert_eq!(rig.view.state.find_matches, vec![(2, 1), (4, 1)]);
    let cursor = (rig.view.state.active_row(), rig.view.state.active_col());

    rig.answer(TrameEvent::FindResult {
        matches: Vec::new(),
        active: None,
    });
    assert!(rig.view.state.find_matches.is_empty());
    assert_eq!(
        (rig.view.state.active_row(), rig.view.state.active_col()),
        cursor
    );
}

/// `CANVAS-039`: a press reaches the cell editor in its own frame. The cell
/// sits far from the screen origin, so a press at its left edge is a small
/// local x. Read as a screen point, it would land past the end of the text.
#[test]
fn a_press_reaches_the_editor_in_its_own_frame() {
    let mut rig = Rig::new();
    rig.frame();
    for _ in 0..3 {
        rig.key("ArrowRight");
        rig.key("ArrowDown");
    }
    rig.type_text("hi");
    rig.frame();
    let (x, y, _w, h) = rig.view.state.cell_pixel_rect(3, 3).expect("D4 is in view");
    let at = (x + 1.0, y + h / 2.0);
    rig.view.handle_input(&InputEvent::PointerDown {
        x: at.0,
        y: at.1,
        button: 0,
        shift: false,
        ctrl: false,
    });
    rig.view.handle_input(&InputEvent::PointerUp {
        x: at.0,
        y: at.1,
        button: 0,
        ctrl: false,
    });
    rig.type_text("X");
    rig.frame();
    let editor = rig.view.editor.as_ref().expect("still open");
    assert_eq!((editor.row, editor.col), (3, 3));
    assert_eq!(editor.text(), "Xhi");
}
