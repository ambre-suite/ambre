//! The grid view driven the way the host drives it. Input events go in and the
//! commands for the Logic thread come out. There is no DOM and no GPU. The text
//! pipe is detached, a frame paints into a recording surface, and the view
//! reads a clock the test moves.

use ambre_canvas::PointerButton;
use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::test_support::{at, rect, span};
use ambre_bus::conduit::Receiver;
use ambre_draw::testing::RecordingSurface;

/// A pointer at logical `(x, y)`, as the canvas hands it over.
fn pointer(x: f64, y: f64) -> ambre_core::ViewPoint {
    ambre_core::ViewPoint::new(
        ambre_core::ViewPx::new(x as f32),
        ambre_core::ViewPx::new(y as f32),
    )
}

const COL_0: Col = Col::new(0);
const COL_1: Col = Col::new(1);
const COL_2: Col = Col::new(2);
const ROW_0: Row = Row::new(0);
const ROW_1: Row = Row::new(1);
const ROW_4: Row = Row::new(4);
const ROW_5: Row = Row::new(5);

/// A clock the test sets, shared with the view it was handed to.
#[derive(Clone, Default)]
struct TestClock(Rc<Cell<f64>>);

impl TestClock {
    fn set(&self, ms: f64) {
        self.0.set(ms);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> ambre_core::PageTime {
        ambre_core::PageTime(self.0.get())
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
        Self::with_queue(256)
    }

    /// A view whose queue to the Logic thread holds `capacity` commands.
    fn with_queue(capacity: usize) -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(capacity);
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
            ambre_canvas::Embedding::Standalone,
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
                TrameCommand::BeginCellEdit { session: s0, row: ROW_0, col: COL_0 },
                TrameCommand::CommitCellEdit { session: s1, text },
                TrameCommand::CommitEdit,
                TrameCommand::SetActiveCell { row: ROW_1, col: COL_0, .. },
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
        src_row: Row::new(src_row),
        rows: RowCount::new(1),
        cols: ColCount::new(1),
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
    view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(0), Col::new(0)),
        CellPos::new(Row::new(1), Col::new(1)),
    ));
    assert_eq!(
        view.clipboard_write(ClipboardWrite::Cut),
        ambre_bus::CopyOutcome::Nothing
    );
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
    rig.view.paste_clipboard(clip(3, "read"), PasteSource::Own);
    assert_eq!(
        pasted(&rig.sent()).map(|p| p.src_row),
        Some(Row::new(7)),
        "own copy"
    );
    rig.view
        .paste_clipboard(clip(3, "read"), PasteSource::Foreign);
    assert_eq!(
        pasted(&rig.sent()).map(|p| p.src_row),
        Some(Row::new(3)),
        "another copy"
    );
    let foreign = ambre_bus::ClipboardBlob {
        ambre: None,
        ..clip(3, "plain")
    };
    rig.view.paste_clipboard(foreign, PasteSource::Foreign);
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
    rig.view.paste_clipboard(clip(3, "2"), PasteSource::Foreign);
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
    rig.view
        .paste_clipboard(clip(3, "read"), PasteSource::Foreign);
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
    let (x, y, w, h) = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Row::new(0), Col::new(0)))
        .expect("A1 is in view");
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
fn in_view(rig: &Rig, at: CellPos) -> bool {
    let state = &rig.view.state;
    at.row >= state.top_row
        && state.visible_offset(at.row) < RowDelta::from_count(state.viewport_rows)
        && at.col >= state.left_col
        && at.col < state.left_col + state.viewport_cols
}

/// `FIND-004`: the match the Logic thread elects is brought into view, even
/// far outside the window, and the active cell lands on it.
#[test]
fn the_current_match_is_brought_into_view() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let far = at(900, 60);
    assert!(!in_view(&rig, far), "the match starts off screen");

    rig.answer(TrameEvent::FindResult {
        matches: vec![at(0, 0), far],
        active: Some(1),
    });
    assert!(in_view(&rig, far), "the elected match is in view");
    assert_eq!(rig.view.state.active_cell(), far);
    let sent = rig.sent();
    assert!(
        sent.iter().any(
            |c| matches!(c, TrameCommand::ScrollViewport { row_start, row_end, .. }
                if (*row_start..=*row_end).contains(&far.row))
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
        matches: vec![
            CellPos::new(Row::new(2), Col::new(1)),
            CellPos::new(Row::new(4), Col::new(1)),
        ],
        active: Some(0),
    });
    assert_eq!(rig.view.state.find_matches, vec![at(2, 1), at(4, 1)]);
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
    let (x, y, _w, h) = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Row::new(3), Col::new(3)))
        .expect("D4 is in view");
    let point = (x + 1.0, y + h / 2.0);
    rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(point.0, point.1),
        button: PointerButton::Primary,
        shift: false,
        ctrl: false,
    });
    rig.view.handle_input(&InputEvent::PointerUp {
        at: pointer(point.0, point.1),
        button: PointerButton::Primary,
        ctrl: false,
    });
    rig.type_text("X");
    rig.frame();
    let editor = rig.view.editor.as_ref().expect("still open");
    assert_eq!(editor.at, at(3, 3));
    assert_eq!(editor.text(), "Xhi");
}

impl Rig {
    /// Press `key` with the given modifiers.
    fn chord(&mut self, key: &str, ctrl: bool, alt: bool, shift: bool) -> bool {
        self.view.handle_input(&InputEvent::Key {
            key: key.to_owned(),
            ctrl,
            alt,
            shift,
            meta: false,
        })
    }

    /// Store `raw` in the grid's cache, shown as `shown`, as a viewport answer
    /// would leave it.
    fn store(&mut self, (row, col): (u32, u32), raw: &str, shown: &str) {
        self.view
            .state
            .set_cell_text(CellPos::new(Row::new(row), Col::new(col)), raw.to_owned());
        if let Some(cell) = self
            .view
            .state
            .cell_cache
            .get_mut(&CellPos::new(Row::new(row), Col::new(col)))
        {
            cell.text = shown.to_owned();
        }
    }

    /// Cut the Logic thread off, so every later send is refused.
    fn lose_logic(&mut self) {
        let (_tx, rx) = ambre_bus::conduit::local(1);
        drop(std::mem::replace(&mut self.to_logic, rx));
    }

    fn active(&self) -> (u32, u32) {
        (
            (self.view.state.active_row()).get(),
            (self.view.state.active_col()).get(),
        )
    }
}

/// True when `sent` carries a command that writes a cell.
fn writes(sent: &[TrameCommand]) -> bool {
    sent.iter().any(|c| {
        matches!(
            c,
            TrameCommand::SetCellText { .. }
                | TrameCommand::CommitCellEdit { .. }
                | TrameCommand::SetCellsBatch { .. }
                | TrameCommand::PasteRange { .. }
        )
    })
}

/// The text of the one `CommitCellEdit` in `sent`.
fn committed(sent: &[TrameCommand]) -> Option<&str> {
    sent.iter().find_map(|c| match c {
        TrameCommand::CommitCellEdit { text, .. } => Some(text.as_str()),
        _ => None,
    })
}

/// `TRAME-UI-064`: a cut clears its source only behind the paste. The copy
/// clears nothing, and the paste goes out before the batch that clears.
#[test]
fn a_cut_clears_its_source_behind_the_paste() {
    let mut rig = Rig::new();
    rig.frame();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(0), Col::new(0)),
        CellPos::new(Row::new(1), Col::new(1)),
    ));
    assert_eq!(
        rig.view.clipboard_write(ClipboardWrite::Cut),
        ambre_bus::CopyOutcome::Deferred
    );
    let copied = rig.sent();
    assert!(
        !writes(&copied),
        "the cut wrote before any paste: {copied:?}"
    );
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(5), Col::new(0)),
        CellPos::new(Row::new(5), Col::new(0)),
    ));
    rig.view.paste_clipboard(clip(0, "x"), PasteSource::Foreign);
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [
                TrameCommand::PasteRange { target_row: ROW_5, .. },
                TrameCommand::SetCellsBatch { cells },
            ] if cells.len() == 4 && cells.iter().all(|(at, t)| at.row <= ROW_1 && at.col <= COL_1 && t.is_empty())
        ),
        "sent {sent:?}"
    );
    rig.view.paste_clipboard(clip(0, "x"), PasteSource::Foreign);
    assert!(
        matches!(rig.sent().as_slice(), [TrameCommand::PasteRange { .. }]),
        "a second paste clears nothing"
    );
}

/// `TRAME-UI-055`: the bar and the cell show one buffer. Without the focus the
/// bar follows the cell, and with it the cell follows the bar.
#[test]
fn the_bar_and_the_cell_show_one_buffer() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("ab");
    // The paint flush applies the typing, and the next frame's sync reads it.
    rig.frame();
    rig.frame();
    assert_eq!(rig.view.formula.text, "ab");
    rig.view.formula.focused = true;
    rig.view.formula.text = "abc".to_owned();
    rig.frame();
    assert_eq!(
        rig.view.editor.as_ref().map(|e| e.text()).as_deref(),
        Some("abc")
    );
}

/// `TRAME-UI-091`: the bar names the cell an edit will reach.
#[test]
fn the_bar_names_the_active_cell() {
    let mut rig = Rig::new();
    rig.frame();
    assert_eq!(rig.view.formula.reference, "A1");
    rig.key("ArrowRight");
    rig.key("ArrowDown");
    rig.key("ArrowDown");
    rig.frame();
    assert_eq!(rig.view.formula.reference, "B3");
}

/// `TRAME-UI-155` and `TRAME-UI-092`: the focus alone opens nothing. A text
/// typed in the bar opens a session, and the focus stays in the bar.
#[test]
fn only_a_text_typed_in_the_bar_opens_a_session() {
    let mut rig = Rig::new();
    rig.store((0, 0), "5", "5");
    rig.frame();
    rig.sent();
    rig.view.formula.focused = true;
    rig.frame();
    assert!(
        rig.view.editor.is_none(),
        "the focus alone opened a session"
    );
    assert!(rig.sent().is_empty());
    let focus = rig.view.text_input.focus_requests();
    rig.view.formula.text = "57".to_owned();
    rig.frame();
    assert_eq!(
        rig.view.editor.as_ref().map(|e| e.text()).as_deref(),
        Some("57")
    );
    assert!(matches!(
        rig.sent().as_slice(),
        [TrameCommand::BeginCellEdit {
            row: ROW_0,
            col: COL_0,
            ..
        }]
    ));
    assert_eq!(
        rig.view.text_input.focus_requests(),
        focus,
        "the pipe took the focus"
    );
}

/// `CHROME-102`: a press on a cell gives the keyboard focus to the pipe.
#[test]
fn a_press_on_a_cell_focuses_the_pipe() {
    let mut rig = Rig::new();
    rig.frame();
    let before = rig.view.text_input.focus_requests();
    rig.click(120.0, 60.0);
    assert!(rig.view.text_input.focus_requests() > before);
}

/// `TRAME-UI-169`: a commit from the bar writes the bar's text, not the buffer
/// the grid held, and the selection then moves down.
#[test]
fn a_commit_from_the_bar_writes_the_bar() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("ab");
    rig.frame();
    rig.sent();
    rig.view.formula.focused = true;
    rig.view.formula.text = "xyz".to_owned();
    rig.view.formula.action = Some(FormulaAction::Commit(CommitDirection::Down));
    rig.frame();
    let sent = rig.sent();
    assert_eq!(committed(&sent), Some("xyz"), "sent {sent:?}");
    assert_eq!(rig.active(), (1, 0));
    assert!(rig.view.editor.is_none());
}

/// `TRAME-UI-156`: a session opened on a formula starts from the formula, not
/// from its result.
#[test]
fn a_session_on_a_formula_starts_from_the_formula() {
    let mut rig = Rig::new();
    rig.store((0, 0), "=1+2", "3");
    rig.frame();
    rig.key("F2");
    assert_eq!(
        rig.view.editor.as_ref().map(|e| e.text()).as_deref(),
        Some("=1+2")
    );
    assert_eq!(rig.view.formula.text, "=1+2");
}

/// `TRAME-UI-157`: Alt+Enter puts a newline in the cell, and a bare Enter
/// commits.
#[test]
fn alt_enter_is_the_newline() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("a");
    rig.chord("Enter", false, true, false);
    rig.type_text("b");
    rig.frame();
    assert!(rig.view.editor.is_some(), "Alt+Enter committed");
    assert!(rig.key("Enter"));
    rig.frame();
    assert_eq!(committed(&rig.sent()), Some("a\nb"));
}

/// `TRAME-UI-158`, `TRAME-UI-159` and `TRAME-UI-056`: an open session writes
/// nothing before its commit. A buffer key leaves the grid selection put, and
/// a composition in progress reaches only the preview.
#[test]
fn an_open_session_writes_nothing_and_holds_the_grid() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("abc");
    rig.view.handle_input(&InputEvent::CompositionUpdate {
        text: "ka".to_owned(),
    });
    rig.frame();
    for key in [
        "ArrowLeft",
        "ArrowRight",
        "Home",
        "End",
        "Backspace",
        "Delete",
    ] {
        rig.key(key);
    }
    rig.chord("ArrowLeft", false, false, true);
    rig.chord("a", true, false, false);
    rig.frame();
    let sent = rig.sent();
    assert!(!writes(&sent), "the open session wrote: {sent:?}");
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, TrameCommand::SetActiveCell { .. })),
        "a buffer key moved the grid: {sent:?}"
    );
    assert_eq!(rig.active(), (0, 0));
    assert_eq!(rig.view.state.selection.bounds(), rect(0, 0, 0, 0));
    rig.view.handle_input(&InputEvent::CompositionEnd {
        text: "か".to_owned(),
    });
    rig.frame();
    assert!(!writes(&rig.sent()), "the composition end wrote");
}

/// `TRAME-UI-051`: a commit by key reads the buffer at the paint flush. At
/// key time it sends nothing, and the commit carries every queued insertion.
#[test]
fn a_grid_commit_reads_the_buffer_after_the_flush() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("4");
    rig.type_text("2");
    rig.key("Enter");
    let before = rig.sent();
    assert_eq!(
        committed(&before),
        None,
        "committed at key time: {before:?}"
    );
    rig.frame();
    assert_eq!(committed(&rig.sent()), Some("42"));
}

/// `TRAME-UI-161` and `TRAME-UI-171`: Enter commits and moves down, Tab
/// commits and moves right.
#[test]
fn enter_moves_down_and_tab_moves_right() {
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("1");
    rig.key("Enter");
    rig.frame();
    assert_eq!(committed(&rig.sent()), Some("1"));
    assert_eq!(rig.active(), (1, 0));
    rig.type_text("2");
    rig.key("Tab");
    rig.frame();
    assert_eq!(committed(&rig.sent()), Some("2"));
    assert_eq!(rig.active(), (1, 1));
}

/// `TRAME-UI-163`: a commit the Logic thread never receives leaves the cell
/// reading its stored value.
#[test]
fn a_refused_commit_leaves_the_stored_value() {
    let mut rig = Rig::new();
    rig.store((0, 0), "old", "old");
    rig.frame();
    rig.type_text("new");
    rig.frame();
    rig.lose_logic();
    rig.key("Enter");
    rig.frame();
    assert_eq!(
        rig.view.cell_seed(CellPos::new(Row::new(0), Col::new(0))),
        "old"
    );
}

/// `TRAME-UI-165` and `TRAME-UI-015`: Escape closes the session with no write.
/// The cell and the bar read the stored value, and only the release goes out.
#[test]
fn escape_closes_the_session_with_no_write() {
    let mut rig = Rig::new();
    rig.store((0, 0), "old", "old");
    rig.frame();
    rig.type_text("new");
    rig.frame();
    rig.sent();
    assert!(rig.key("Escape"));
    rig.frame();
    let sent = rig.sent();
    assert!(
        matches!(sent.as_slice(), [TrameCommand::EndCellEdit { .. }]),
        "sent {sent:?}"
    );
    assert!(rig.view.editor.is_none());
    assert_eq!(
        rig.view.cell_seed(CellPos::new(Row::new(0), Col::new(0))),
        "old"
    );
    assert_eq!(rig.view.formula.text, "old");
}

/// `TRAME-UI-166`, `TRAME-UI-167` and `TRAME-UI-168`: Ctrl+Enter writes the
/// formula as typed into each selected cell and no other. The selection and
/// the active cell stay put.
#[test]
fn a_filled_formula_reaches_each_selected_cell_as_typed() {
    let mut rig = Rig::new();
    rig.frame();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(0), Col::new(0)),
        CellPos::new(Row::new(1), Col::new(1)),
    ));
    rig.sent();
    rig.type_text("=A1+1");
    rig.chord("Enter", true, false, false);
    rig.frame();
    let sent = rig.sent();
    let Some(cells) = sent.iter().find_map(|c| match c {
        TrameCommand::SetCellsBatch { cells } => Some(cells),
        _ => None,
    }) else {
        panic!("no batch in {sent:?}");
    };
    let mut placed: Vec<CellPos> = cells.iter().map(|(at, _)| *at).collect();
    placed.sort_unstable();
    assert_eq!(placed, vec![at(0, 0), at(0, 1), at(1, 0), at(1, 1)]);
    assert!(cells.iter().all(|(_, t)| t == "=A1+1"), "{cells:?}");
    assert_eq!(rig.active(), (0, 0));
    assert_eq!(rig.view.state.selection.bounds(), rect(0, 0, 1, 1));
}

impl Rig {
    /// Press and release the primary button at `(x, y)`.
    fn click(&mut self, x: f64, y: f64) {
        self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        });
        self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Primary,
            ctrl: false,
        });
    }

    /// The right edge of column `c` on screen.
    fn col_edge(&self, c: u32) -> f64 {
        self.view.state.col_x(Col::new(c)) + self.view.state.col_width_of(Col::new(c))
    }
}

/// The "+" on a column boundary inserts at that boundary: the column lands
/// at the index right of it (`TRAME-UI-048`). The same pixels arm a resize in
/// their lower half, so the insert must carry no size (`TRAME-UI-123`).
#[test]
fn the_inline_insert_lands_on_its_boundary_and_sizes_nothing() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let widths = rig.view.state.col_widths.clone();
    let heights = rig.view.state.row_heights.clone();
    // The boundary between B and C, in the upper half of the strip.
    let x = rig.col_edge(1);
    assert_eq!(
        rig.view.state.header_hit(pointer(x, 2.0)),
        Some(crate::ui::grid::HeaderHit::ColInsert(Col::new(2)))
    );
    rig.click(x, 2.0);
    rig.frame();
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [
                TrameCommand::SetActiveCell { col: COL_2, .. },
                TrameCommand::InsertColLeft,
                ..
            ]
        ),
        "sent {sent:?}"
    );
    // The row strip: the boundary under row 3, in the left half.
    let y = rig.view.state.row_y(Row::new(3)) + rig.view.state.row_height_of(Row::new(3));
    assert_eq!(
        rig.view.state.header_hit(pointer(2.0, y)),
        Some(crate::ui::grid::HeaderHit::RowInsert(Row::new(4)))
    );
    rig.click(2.0, y);
    rig.frame();
    let sent_rows = rig.sent();
    assert!(
        matches!(
            sent_rows.as_slice(),
            [
                TrameCommand::SetActiveCell { row: ROW_4, .. },
                TrameCommand::InsertRowAbove,
                ..
            ]
        ),
        "sent {sent_rows:?}"
    );
    for cmd in sent.iter().chain(&sent_rows) {
        assert!(
            !matches!(
                cmd,
                TrameCommand::ResizeCol { .. } | TrameCommand::ResizeRow { .. }
            ),
            "an insert sent {cmd:?}"
        );
    }
    assert_eq!(rig.view.state.col_widths, widths);
    assert_eq!(rig.view.state.row_heights, heights);
}

/// A boundary dragged at any zoom stores the width in unzoomed pixels: the
/// same on-screen gesture scaled by the zoom stores the same size
/// (`TRAME-UI-040`). The grid takes the width before any answer
/// (`TRAME-UI-041`).
#[test]
fn a_resize_stores_an_unzoomed_width_and_reflows_at_once() {
    for zoom in [1.0, 2.0] {
        let mut rig = Rig::new();
        assert!(rig.view.set_zoom(ambre_core::ZoomFactor::positive(zoom)) || zoom == 1.0);
        rig.frame();
        rig.sent();
        let edge = rig.col_edge(1);
        let y = rig.view.state.header_h() * 0.8;
        assert_eq!(
            rig.view.state.header_hit(pointer(edge, y)),
            Some(crate::ui::grid::HeaderHit::ColBorder(Col::new(1)))
        );
        let c_before = rig.view.state.col_x(Col::new(2));
        rig.view.handle_input(&InputEvent::PointerDown {
            at: pointer(edge, y),
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        });
        // 200 base pixels, drawn at this zoom.
        let target = rig.view.state.col_x(Col::new(1)) + 200.0 * zoom;
        rig.view.handle_input(&InputEvent::PointerMove {
            at: pointer(target, y),
        });
        rig.view.handle_input(&InputEvent::PointerUp {
            at: pointer(target, y),
            button: PointerButton::Primary,
            ctrl: false,
        });
        let sent = rig.sent();
        let widths: Vec<ambre_core::DocPx> = sent
            .iter()
            .filter_map(|cmd| match cmd {
                TrameCommand::ResizeCol {
                    col: COL_1, width, ..
                } => Some(*width),
                _ => None,
            })
            .collect();
        assert_eq!(
            widths,
            [ambre_core::DocPx::new(200.0)],
            "zoom {zoom}: sent {sent:?}"
        );
        // No answer came back, and the grid already reflowed.
        assert_eq!(
            rig.view.state.col_widths.get(&COL_1),
            Some(&ambre_core::DocPx::new(200.0))
        );
        assert_eq!(
            rig.view.state.col_x(Col::new(2)),
            rig.view.state.col_x(Col::new(1)) + 200.0 * zoom
        );
        assert_ne!(rig.view.state.col_x(Col::new(2)), c_before);
    }
}

/// A double-click on a boundary asks for the whole column and sizes nothing
/// until it has it (`TRAME-UI-128`). The answer is measured, and the width
/// sent is the content's (`TRAME-UI-042`).
#[test]
fn a_double_click_fits_the_column_to_its_whole_content() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let edge = rig.col_edge(0);
    let y = rig.view.state.header_h() * 0.8;
    assert!(rig.view.handle_input(&InputEvent::DoubleClick {
        at: pointer(edge, y)
    }));
    rig.frame();
    let asked = rig.sent();
    assert!(
        matches!(
            asked.as_slice(),
            [TrameCommand::RequestColumnContent { col: COL_0 }]
        ),
        "sent {asked:?}"
    );
    let cells = vec![
        crate::DisplayCell {
            text: "short".to_owned(),
            ..Default::default()
        },
        crate::DisplayCell {
            text: "a value far wider than the default column".to_owned(),
            ..Default::default()
        },
    ];
    let expected = super::interact::fit_column_width(
        &cells,
        &mut FontContext::new(),
        &mut TextCache::new(1.0),
    );
    rig.answer(TrameEvent::ColumnContent {
        col: Col::new(0),
        cells,
    });
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [TrameCommand::ResizeCol { col: COL_0, width, .. }] if *width == expected
        ),
        "sent {sent:?}"
    );
    assert!(expected > crate::ui::grid::COL_WIDTH);
}

/// A drag from a merged source previews whole blocks: a 2-row merge grows two
/// rows at a time, whatever row the pointer stops on (`TRAME-UI-116`).
#[test]
fn a_merged_fill_previews_whole_blocks() {
    let mut rig = Rig::new();
    rig.frame();
    let merged = |row, span| crate::DisplayCell {
        row,
        col: Col::new(0),
        is_merged: true,
        merge_span: span,
        ..Default::default()
    };
    // A1:A2 is one merge of one column and two rows.
    rig.answer(TrameEvent::ViewportData {
        version: ambre_bus::ViewportVersion(0),
        row_start: Row::new(0),
        col_start: Col::new(0),
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![
            merged(Row::new(0), Some(span(2, 1))),
            merged(Row::new(1), None),
        ],
    });
    rig.sent();
    let (hx, hy, w, h) = rig.view.state.fill_handle_rect().expect("a handle");
    rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(hx + w / 2.0, hy + h / 2.0),
        button: PointerButton::Primary,
        shift: false,
        ctrl: false,
    });
    let mid = |rig: &Rig, row: u32| {
        let (x, y, w, h) = rig
            .view
            .state
            .cell_pixel_rect(CellPos::new(Row::new(row), Col::new(0)))
            .expect("in view");
        (x + w / 2.0, y + h / 2.0)
    };
    // Row 2 is one row past the source, row 4 is three: each snaps up to the
    // block that holds it.
    for (row, last) in [(2, 3), (3, 3), (4, 5)] {
        let (x, y) = mid(&rig, row);
        rig.view
            .handle_input(&InputEvent::PointerMove { at: pointer(x, y) });
        assert_eq!(
            rig.view.state.fill_preview,
            Some(rect(0, 0, last, 0)),
            "pointer on row {row}"
        );
    }
}

impl Rig {
    /// One wheel sample, as the host forwards it.
    fn wheel(&mut self, dy: f64, delta_mode: u32, ctrl_key: bool) -> bool {
        self.view.handle_input(&InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::new(
                0.0,
                dy,
                ambre_canvas::WheelDeltaMode::from_dom(delta_mode),
            ),
            ctrl_key,
        })
    }

    fn top(&self) -> u32 {
        self.view.state.top_row.get()
    }
}

/// `TRAME-UI-147`, `KINETIC-015`: a line delta and a page delta land once and in full. A
/// pixel delta keeps its sub-row remainder for the next sample.
#[test]
fn a_wheel_delta_lands_once_and_keeps_its_remainder() {
    let mut rig = Rig::new();
    rig.frame();
    assert!(rig.wheel(3.0, 1, false));
    assert_eq!(rig.top(), 3, "three lines are three rows");
    let row = f64::from(ROW_HEIGHT.get());
    let page = (rig.view.state.canvas_height / row) as u32;
    assert!(rig.wheel(1.0, 2, false));
    assert_eq!(rig.top(), 3 + page, "a page is a screen of rows");
    let at = rig.top();
    assert!(rig.wheel(row * 1.5, 0, false));
    assert_eq!(rig.top(), at + 1, "a row and a half moves one row");
    assert!(rig.wheel(row * 0.5, 0, false));
    assert_eq!(rig.top(), at + 2, "the half row was kept");
}

/// `TRAME-UI-026`: a notch moves the grid once, and the frames after it move
/// nothing.
#[test]
fn a_wheel_notch_starts_no_coast() {
    let mut rig = Rig::new();
    rig.frame();
    rig.clock.set(1_000.0);
    assert!(rig.wheel(3.0, 1, false));
    let landed = rig.top();
    for step in 1..=30 {
        rig.clock.set(1_000.0 + f64::from(step) * 16.0);
        rig.frame();
    }
    assert_eq!(rig.top(), landed, "the grid stays where the notch put it");
}

/// `TRAME-UI-146`: the wheel held with Ctrl is left to the browser and
/// scrolls nothing.
#[test]
fn a_ctrl_wheel_scrolls_nothing() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    assert!(!rig.wheel(5.0, 1, true), "the browser keeps the gesture");
    rig.frame();
    assert_eq!(rig.top(), 0);
    assert!(
        !rig.sent()
            .iter()
            .any(|c| matches!(c, TrameCommand::ScrollViewport { .. })),
        "no request for a scroll that did not happen"
    );
}

/// `TRAME-UI-175`: a selection drag held below the cells scrolls the grid
/// frame by frame, and the range follows to the last row on screen.
#[test]
fn a_selection_held_below_the_cells_scrolls_and_follows() {
    let mut rig = Rig::new();
    rig.frame();
    let (x, y, w, h) = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Row::new(1), Col::new(1)))
        .expect("B2 is in view");
    let (px, py) = (x + w / 2.0, y + h / 2.0);
    rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(px, py),
        button: PointerButton::Primary,
        shift: false,
        ctrl: false,
    });
    let below = rig.view.state.canvas_height + 40.0;
    rig.view.handle_input(&InputEvent::PointerMove {
        at: pointer(px, below),
    });
    for step in 0..=20 {
        rig.clock.set(f64::from(step) * 16.0);
        rig.frame();
    }
    assert!(rig.top() > 0, "the grid scrolled towards the pointer");
    let CellPos { row, col: _ } = rig
        .view
        .state
        .drag_cell_at(pointer(px, below))
        .expect("an edge cell");
    assert_eq!(
        rig.view.state.selection.anchor,
        CellPos::new(Row::new(1), Col::new(1))
    );
    assert_eq!(
        rig.view.state.selection.active,
        CellPos::new(row, COL_1),
        "the range reaches the edge cell"
    );
}

impl Rig {
    /// Press and release the secondary button at `(x, y)`.
    fn right_click(&mut self, x: f64, y: f64) {
        self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Secondary,
            shift: false,
            ctrl: false,
        });
        self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Secondary,
            ctrl: false,
        });
    }

    /// Press and release the primary button at `(x, y)` with Shift held.
    fn shift_click(&mut self, x: f64, y: f64) {
        self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Primary,
            shift: true,
            ctrl: false,
        });
        self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Primary,
            ctrl: false,
        });
    }

    /// The centre of cell `(row, col)` on screen.
    fn cell_mid(&self, row: u32, col: u32) -> (f64, f64) {
        let s = &self.view.state;
        (
            s.col_x(Col::new(col)) + s.col_width_of(Col::new(col)) / 2.0,
            s.row_y(Row::new(row)) + s.row_height_of(Row::new(row)) / 2.0,
        )
    }

    /// The centre of column `col`'s header body.
    fn col_header_mid(&self, col: u32) -> (f64, f64) {
        (self.cell_mid(0, col).0, self.view.state.header_h() / 2.0)
    }

    /// The centre of row `row`'s header body.
    fn row_header_mid(&self, row: u32) -> (f64, f64) {
        (self.view.state.header_w() / 2.0, self.cell_mid(row, 0).1)
    }

    /// The centre of the corner above the row headers.
    fn corner_mid(&self) -> (f64, f64) {
        (
            self.view.state.header_w() / 2.0,
            self.view.state.header_h() / 2.0,
        )
    }

    /// Select the range from cell `a` to cell `b` with a click and a
    /// Shift-click, and return the selection it made.
    fn select_range(&mut self, a: (u32, u32), b: (u32, u32)) -> Selection {
        let (x, y) = self.cell_mid(a.0, a.1);
        self.click(x, y);
        let (x, y) = self.cell_mid(b.0, b.1);
        self.shift_click(x, y);
        assert!(self.view.state.selection.is_multi());
        self.view.state.selection.clone()
    }
}

/// The menu opens scoped to what the pointer targeted (`TRAME-UI-117`): a
/// cell, a column, a row or the corner. A header boundary scopes it to its
/// axis.
#[test]
fn a_right_click_scopes_the_menu_to_its_target() {
    use crate::ui::grid::CtxScope;
    let mut rig = Rig::new();
    rig.frame();
    let targets = [
        (rig.cell_mid(3, 2), CtxScope::Cell),
        (rig.col_header_mid(2), CtxScope::Column),
        (rig.row_header_mid(3), CtxScope::Row),
        (rig.corner_mid(), CtxScope::Corner),
        (
            (rig.col_edge(1), rig.view.state.header_h() / 2.0),
            CtxScope::Column,
        ),
    ];
    for ((x, y), scope) in targets {
        rig.right_click(x, y);
        assert_eq!(rig.view.menu_scope, scope, "at ({x}, {y})");
    }
}

/// A right-click outside the selection moves it onto its target first, as a
/// left click would (`TRAME-UI-118`). A cell collapses to that cell, a header
/// body selects its axis, and the corner selects the sheet.
#[test]
fn a_right_click_outside_the_selection_reselects() {
    let mut rig = Rig::new();
    rig.frame();
    let range = rig.select_range((0, 0), (1, 1));

    let (x, y) = rig.cell_mid(4, 3);
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::single(CellPos::new(Row::new(4), Col::new(3)))
    );

    rig.view.state.selection = range.clone();
    let (x, y) = rig.col_header_mid(3);
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::axis(AxisPos::Col(Col::new(3)), AxisPos::Col(Col::new(3)))
    );

    rig.view.state.selection = range.clone();
    let (x, y) = rig.row_header_mid(4);
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::axis(AxisPos::Row(Row::new(4)), AxisPos::Row(ROW_4))
    );

    rig.view.state.selection = range;
    let (x, y) = rig.corner_mid();
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::axis(AxisPos::Col(COL_0), AxisPos::Col(crate::LAST_COL)),
        "every column, so the whole sheet"
    );
}

/// A right-click inside the selection keeps it whole (`TRAME-UI-088`), so the
/// menu acts on the range.
#[test]
fn a_right_click_inside_the_selection_keeps_it() {
    let mut rig = Rig::new();
    rig.frame();
    let range = rig.select_range((0, 0), (2, 2));
    for (row, col) in [(1, 1), (2, 0), (0, 0)] {
        let (x, y) = rig.cell_mid(row, col);
        rig.right_click(x, y);
        assert_eq!(rig.view.state.selection, range, "at ({row}, {col})");
    }
}

/// Choosing a menu entry leaves the selection it opened on (`TRAME-UI-119`).
/// Each entry reaches the Logic thread with no selection of its own, and no
/// answer it brings back moves the view's.
#[test]
fn a_menu_entry_leaves_the_selection_alone() {
    use crate::ui::commands as registry;
    let mut rig = Rig::new();
    rig.frame();
    let mut logic = crate::TrameState::new();
    for row in 0..4 {
        for col in 0..3 {
            logic.handle_command(TrameCommand::SetCellText {
                row: Row::new(row),
                col: Col::new(col),
                text: format!("{}", 10 - row - col),
            });
        }
    }
    rig.sent();
    let range = rig.select_range((1, 0), (2, 1));
    for cmd in rig.sent() {
        logic.handle_command(cmd);
    }
    for id in [
        registry::SORT_RANGE_ASC,
        registry::MERGE,
        registry::UNMERGE,
        registry::INSERT_ROW_ABOVE,
        registry::INSERT_COL_LEFT,
        registry::DELETE_ROWS,
        registry::UNDO,
    ] {
        let Some(AppCommand::Trame(cmd)) = registry::command_to_app(id) else {
            panic!("{id:?} reaches the Logic thread");
        };
        let out = logic.handle_command(cmd);
        for event in out.display {
            rig.answer(event);
        }
        assert_eq!(rig.view.state.selection, range, "after {id:?}");
    }
}

/// A lone cell asks for no aggregate, and a range asks for one
/// (`TRAME-UI-108`). With no request in flight the band is cleared.
#[test]
fn a_lone_cell_asks_for_no_aggregate() {
    let asks = |sent: &[TrameCommand]| {
        sent.iter()
            .filter(|c| matches!(c, TrameCommand::RequestSelectionStats))
            .count()
    };
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let (x, y) = rig.cell_mid(2, 2);
    rig.click(x, y);
    assert_eq!(asks(&rig.sent()), 0);
    let (x, y) = rig.cell_mid(4, 3);
    rig.shift_click(x, y);
    assert_eq!(asks(&rig.sent()), 1);
    let (x, y) = rig.cell_mid(0, 0);
    rig.click(x, y);
    assert_eq!(asks(&rig.sent()), 0);
}

/// Counts the ERROR events while it is the default subscriber.
struct ErrorCount(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl tracing::Subscriber for ErrorCount {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The ERROR events `act` emits.
fn errors_in(act: impl FnOnce()) -> usize {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    tracing::subscriber::with_default(ErrorCount(count.clone()), act);
    count.load(std::sync::atomic::Ordering::Relaxed)
}

impl Rig {
    /// Drag the boundary right of column 1 to `width` pixels.
    fn drag_col_1_to(&mut self, width: f64) {
        let edge = self.col_edge(1);
        let y = self.view.state.header_h() * 0.8;
        self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(edge, y),
            button: PointerButton::Primary,
            shift: false,
            ctrl: false,
        });
        let target = self.view.state.col_x(Col::new(1)) + width;
        self.view.handle_input(&InputEvent::PointerMove {
            at: pointer(target, y),
        });
        self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(target, y),
            button: PointerButton::Primary,
            ctrl: false,
        });
    }
}

/// `TRAME-UI-127` and `BUS-018`: a resize the Logic thread never receives
/// leaves the column at its width.
#[test]
fn a_refused_resize_leaves_the_width() {
    let mut rig = Rig::new();
    rig.frame();
    let before = rig.view.state.col_x(Col::new(2));
    rig.lose_logic();
    rig.drag_col_1_to(200.0);
    assert_eq!(rig.view.state.col_widths.get(&COL_1), None);
    assert_eq!(rig.view.state.col_x(Col::new(2)), before);
}

/// `TRAME-UI-125` and `BUS-018`: a cut whose paste never left keeps its
/// source, and stays pending for the next paste.
#[test]
fn a_refused_paste_leaves_the_cut_source() {
    let mut rig = Rig::new();
    rig.store((0, 0), "a", "a");
    rig.frame();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(0), Col::new(0)),
        CellPos::new(Row::new(0), Col::new(0)),
    ));
    rig.view.clipboard_write(ClipboardWrite::Cut);
    rig.sent();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Row::new(5), Col::new(0)),
        CellPos::new(Row::new(5), Col::new(0)),
    ));
    rig.lose_logic();
    rig.view.paste_clipboard(clip(0, "a"), PasteSource::Foreign);
    assert_eq!(
        rig.view.cell_seed(CellPos::new(Row::new(0), Col::new(0))),
        "a"
    );
    assert_eq!(rig.view.state.clipboard_anchor, Some(rect(0, 0, 0, 0)));
    assert_eq!(rig.view.state.active_row(), Row::new(5));
}

/// `TRAME-UI-125` and `TRAME-UI-142`: a source clear the Logic thread never
/// receives keeps the cut range, and the loss is logged.
#[test]
fn a_refused_source_clear_keeps_the_cut_range() {
    let mut rig = Rig::new();
    rig.store((0, 0), "a", "a");
    rig.frame();
    rig.view.cut_pending = true;
    rig.view.state.clipboard_anchor = Some(rect(0, 0, 0, 0));
    rig.lose_logic();
    let logged = errors_in(|| rig.view.finish_paste());
    assert!(logged > 0, "the refused clear left no trace");
    assert_eq!(
        rig.view.cell_seed(CellPos::new(Row::new(0), Col::new(0))),
        "a"
    );
}

/// `TRAME-UI-133`: a fill the Logic thread never receives leaves the
/// selection on its source.
#[test]
fn a_refused_fill_leaves_the_selection() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    let (hx, hy, w, h) = rig.view.state.fill_handle_rect().expect("a handle");
    rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(hx + w / 2.0, hy + h / 2.0),
        button: PointerButton::Primary,
        shift: false,
        ctrl: false,
    });
    let (x, y) = rig.cell_mid(3, 0);
    rig.view
        .handle_input(&InputEvent::PointerMove { at: pointer(x, y) });
    assert_eq!(rig.view.state.fill_preview, Some(rect(0, 0, 3, 0)));
    rig.lose_logic();
    rig.view.handle_input(&InputEvent::PointerUp {
        at: pointer(x, y),
        button: PointerButton::Primary,
        ctrl: false,
    });
    assert_eq!(rig.view.state.selection.selected_cells(), [at(0, 0)]);
}

/// `BUS-018`: a clear the Logic thread never receives leaves the cell
/// reading its value.
#[test]
fn a_refused_clear_leaves_the_cell() {
    let mut rig = Rig::new();
    rig.store((0, 0), "a", "a");
    rig.frame();
    rig.lose_logic();
    assert!(rig.key("Delete"));
    assert_eq!(
        rig.view.cell_seed(CellPos::new(Row::new(0), Col::new(0))),
        "a"
    );
}

/// `TRAME-UI-142`: every lossless send the outbox refuses is logged. The
/// resize, the paste, the clear and the commit each leave an error.
#[test]
fn a_refused_send_is_never_silent() {
    type Act = fn(&mut Rig);
    let acts: [(&str, Act); 4] = [
        ("resize", |rig| rig.drag_col_1_to(200.0)),
        ("paste", |rig| {
            rig.view.paste_clipboard(clip(0, "a"), PasteSource::Foreign)
        }),
        ("clear", |rig| {
            rig.key("Delete");
        }),
        ("commit", |rig| {
            rig.key("Enter");
            rig.frame();
        }),
    ];
    for (name, act) in acts {
        let mut rig = Rig::new();
        rig.frame();
        if name == "commit" {
            rig.type_text("x");
            rig.frame();
        }
        rig.lose_logic();
        assert!(errors_in(|| act(&mut rig)) > 0, "{name} left no trace");
    }
}

/// `BUS-022`: the grid takes no input while its outbox holds a command, and
/// takes it again once the queue drains.
#[test]
fn a_busy_grid_takes_no_input() {
    let mut rig = Rig::with_queue(1);
    rig.frame();
    rig.to_logic.try_iter().for_each(drop);
    for _ in 0..2 {
        let cmd = rig.view.set_active_cmd();
        assert!(rig.view.send_command_lossless(cmd).is_ok());
    }
    assert!(rig.view.is_busy());
    assert!(rig.type_text("x"), "the refusal consumes the event");
    assert!(rig.view.editor.is_none(), "the busy grid opened an editor");
    rig.to_logic.try_iter().for_each(drop);
    rig.view.flush_outbox();
    assert!(!rig.view.is_busy());
    rig.type_text("x");
    assert!(
        rig.view.editor.is_some(),
        "the drained grid takes input again"
    );
}

/// `BUS-024`: a viewport request the full queue dropped is offered again on
/// the next frame, once.
#[test]
fn a_dropped_scroll_request_is_offered_again() {
    let scrolls = |rig: &mut Rig| {
        rig.to_logic
            .try_iter()
            .filter(|msg| {
                matches!(
                    msg,
                    LogicCommand::App {
                        command: AppCommand::Trame(TrameCommand::ScrollViewport { .. }),
                        ..
                    }
                )
            })
            .count()
    };
    let mut rig = Rig::with_queue(1);
    rig.frame();
    rig.to_logic.try_iter().for_each(drop);
    let cmd = rig.view.set_active_cmd();
    assert!(rig.view.send_command_lossless(cmd).is_ok());
    assert!(rig.wheel(5.0, 1, false));
    rig.frame();
    assert_eq!(scrolls(&mut rig), 0, "the full queue took the request");
    rig.frame();
    assert_eq!(
        scrolls(&mut rig),
        1,
        "the dropped request was not offered again"
    );
    rig.frame();
    assert_eq!(scrolls(&mut rig), 0, "the request was offered twice");
}
