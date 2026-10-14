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
        self.view.state.set_cell_text(row, col, raw.to_owned());
        if let Some(cell) = self.view.state.cell_cache.get_mut(&(row, col)) {
            cell.text = shown.to_owned();
        }
    }

    /// Cut the Logic thread off, so every later send is refused.
    fn lose_logic(&mut self) {
        let (_tx, rx) = ambre_bus::conduit::local(1);
        drop(std::mem::replace(&mut self.to_logic, rx));
    }

    fn active(&self) -> (u32, u32) {
        (self.view.state.active_row(), self.view.state.active_col())
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
    rig.view.state.select_rect(0, 0, 1, 1);
    assert_eq!(
        rig.view.clipboard_write(true),
        ambre_bus::CopyOutcome::Deferred
    );
    let copied = rig.sent();
    assert!(
        !writes(&copied),
        "the cut wrote before any paste: {copied:?}"
    );
    rig.view.state.select_rect(5, 0, 5, 0);
    rig.view.paste_clipboard(clip(0, "x"), false);
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [
                TrameCommand::PasteRange { target_row: 5, .. },
                TrameCommand::SetCellsBatch { cells },
            ] if cells.len() == 4 && cells.iter().all(|(r, c, t)| *r <= 1 && *c <= 1 && t.is_empty())
        ),
        "sent {sent:?}"
    );
    rig.view.paste_clipboard(clip(0, "x"), false);
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
        [TrameCommand::BeginCellEdit { row: 0, col: 0, .. }]
    ));
    assert_eq!(
        rig.view.text_input.focus_requests(),
        focus,
        "the pipe took the focus"
    );
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
    assert_eq!(rig.view.state.selection.bounds(), (0, 0, 0, 0));
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
    assert_eq!(rig.view.cell_seed(0, 0), "old");
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
    assert_eq!(rig.view.cell_seed(0, 0), "old");
    assert_eq!(rig.view.formula.text, "old");
}

/// `TRAME-UI-166`, `TRAME-UI-167` and `TRAME-UI-168`: Ctrl+Enter writes the
/// formula as typed into each selected cell and no other. The selection and
/// the active cell stay put.
#[test]
fn a_filled_formula_reaches_each_selected_cell_as_typed() {
    let mut rig = Rig::new();
    rig.frame();
    rig.view.state.select_rect(0, 0, 1, 1);
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
    let mut at: Vec<(u32, u32)> = cells.iter().map(|(r, c, _)| (*r, *c)).collect();
    at.sort_unstable();
    assert_eq!(at, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
    assert!(cells.iter().all(|(_, _, t)| t == "=A1+1"), "{cells:?}");
    assert_eq!(rig.active(), (0, 0));
    assert_eq!(rig.view.state.selection.bounds(), (0, 0, 1, 1));
}

impl Rig {
    /// Press and release the primary button at `(x, y)`.
    fn click(&mut self, x: f64, y: f64) {
        self.view.handle_input(&InputEvent::PointerDown {
            x,
            y,
            button: 0,
            shift: false,
            ctrl: false,
        });
        self.view.handle_input(&InputEvent::PointerUp {
            x,
            y,
            button: 0,
            ctrl: false,
        });
    }

    /// The right edge of column `c` on screen.
    fn col_edge(&self, c: u32) -> f64 {
        self.view.state.col_x(c) + self.view.state.col_width_of(c)
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
        rig.view.state.header_hit(x, 2.0),
        Some(crate::ui::grid::HeaderHit::ColInsert(2))
    );
    rig.click(x, 2.0);
    rig.frame();
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [
                TrameCommand::SetActiveCell { col: 2, .. },
                TrameCommand::InsertColLeft,
                ..
            ]
        ),
        "sent {sent:?}"
    );
    // The row strip: the boundary under row 3, in the left half.
    let y = rig.view.state.row_y(3) + rig.view.state.row_height_of(3);
    assert_eq!(
        rig.view.state.header_hit(2.0, y),
        Some(crate::ui::grid::HeaderHit::RowInsert(4))
    );
    rig.click(2.0, y);
    rig.frame();
    let sent_rows = rig.sent();
    assert!(
        matches!(
            sent_rows.as_slice(),
            [
                TrameCommand::SetActiveCell { row: 4, .. },
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
        assert!(rig.view.set_zoom(zoom) || zoom == 1.0);
        rig.frame();
        rig.sent();
        let edge = rig.col_edge(1);
        let y = rig.view.state.header_h() * 0.8;
        assert_eq!(
            rig.view.state.header_hit(edge, y),
            Some(crate::ui::grid::HeaderHit::ColBorder(1))
        );
        let c_before = rig.view.state.col_x(2);
        rig.view.handle_input(&InputEvent::PointerDown {
            x: edge,
            y,
            button: 0,
            shift: false,
            ctrl: false,
        });
        // 200 base pixels, drawn at this zoom.
        let target = rig.view.state.col_x(1) + 200.0 * zoom;
        rig.view
            .handle_input(&InputEvent::PointerMove { x: target, y });
        rig.view.handle_input(&InputEvent::PointerUp {
            x: target,
            y,
            button: 0,
            ctrl: false,
        });
        let sent = rig.sent();
        let widths: Vec<f32> = sent
            .iter()
            .filter_map(|cmd| match cmd {
                TrameCommand::ResizeCol {
                    col: 1, width_px, ..
                } => Some(*width_px),
                _ => None,
            })
            .collect();
        assert_eq!(widths, [200.0], "zoom {zoom}: sent {sent:?}");
        // No answer came back, and the grid already reflowed.
        assert_eq!(rig.view.state.col_widths.get(&1), Some(&200.0));
        assert_eq!(
            rig.view.state.col_x(2),
            rig.view.state.col_x(1) + 200.0 * zoom
        );
        assert_ne!(rig.view.state.col_x(2), c_before);
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
    assert!(
        rig.view
            .handle_input(&InputEvent::DoubleClick { x: edge, y })
    );
    rig.frame();
    let asked = rig.sent();
    assert!(
        matches!(
            asked.as_slice(),
            [TrameCommand::RequestColumnContent { col: 0 }]
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
    rig.answer(TrameEvent::ColumnContent { col: 0, cells });
    let sent = rig.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [TrameCommand::ResizeCol { col: 0, width_px, .. }] if *width_px == expected
        ),
        "sent {sent:?}"
    );
    assert!(expected > crate::ui::grid::COL_WIDTH as f32);
}

/// A drag from a merged source previews whole blocks: a 2-row merge grows two
/// rows at a time, whatever row the pointer stops on (`TRAME-UI-116`).
#[test]
fn a_merged_fill_previews_whole_blocks() {
    let mut rig = Rig::new();
    rig.frame();
    let merged = |row, span| crate::DisplayCell {
        row,
        col: 0,
        is_merged: true,
        merge_span: span,
        ..Default::default()
    };
    // A1:A2 is one merge of one column and two rows.
    rig.answer(TrameEvent::ViewportData {
        version: 0,
        row_start: 0,
        col_start: 0,
        col_widths: vec![],
        row_heights: vec![],
        cells: vec![merged(0, Some((1, 2))), merged(1, None)],
    });
    rig.sent();
    let (hx, hy, w, h) = rig.view.state.fill_handle_rect().expect("a handle");
    rig.view.handle_input(&InputEvent::PointerDown {
        x: hx + w / 2.0,
        y: hy + h / 2.0,
        button: 0,
        shift: false,
        ctrl: false,
    });
    let mid = |rig: &Rig, row: u32| {
        let (x, y, w, h) = rig.view.state.cell_pixel_rect(row, 0).expect("in view");
        (x + w / 2.0, y + h / 2.0)
    };
    // Row 2 is one row past the source, row 4 is three: each snaps up to the
    // block that holds it.
    for (row, last) in [(2, 3), (3, 3), (4, 5)] {
        let (x, y) = mid(&rig, row);
        rig.view.handle_input(&InputEvent::PointerMove { x, y });
        assert_eq!(
            rig.view.state.fill_preview,
            Some((0, 0, last, 0)),
            "pointer on row {row}"
        );
    }
}
