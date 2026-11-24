//! The grid view driven the way the host drives it. Input events go in and the
//! commands for the Logic thread come out. There is no DOM and no GPU. The text
//! pipe is detached, a frame paints into a recording surface, and the view
//! reads a clock the test moves.

use ambre_canvas::PointerButton;
use ambre_commands::{FunctionKey, Key, KeyChar, ModifierKeys};
use ambre_core::Flags;
use ambre_core::index::{Delta, Len, Pos};
use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::test_support::{at, rect, span};
use ambre_bus::conduit::Receiver;
use ambre_core::{Instant, PageLoad};
use ambre_draw::testing::RecordingSurface;

/// A pointer at logical `(x, y)`, as the canvas hands it over.
fn pointer(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
    ambre_core::Point::new(
        ambre_core::Px::<ambre_core::View>::new(x as f32),
        ambre_core::Px::<ambre_core::View>::new(y as f32),
    )
}

const COL_0: Pos<Col> = Pos::<Col>::new(0);
const COL_1: Pos<Col> = Pos::<Col>::new(1);
const COL_2: Pos<Col> = Pos::<Col>::new(2);
const ROW_0: Pos<Row> = Pos::<Row>::new(0);
const ROW_1: Pos<Row> = Pos::<Row>::new(1);
const ROW_4: Pos<Row> = Pos::<Row>::new(4);
const ROW_5: Pos<Row> = Pos::<Row>::new(5);

/// A clock the test sets, shared with the view it was handed to.
#[derive(Clone, Default)]
struct TestClock(Rc<Cell<f64>>);

impl TestClock {
    fn set(&self, ms: f64) {
        self.0.set(ms);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant<PageLoad> {
        Instant::<PageLoad>::new(self.0.get())
    }
}

/// One grid view, the far end of its outbound channel, and its clock.
struct Rig {
    view: TrameView,
    to_logic: Receiver<LogicCommand<TrameCommand>>,
    /// The Logic thread's end of the view's inbound channel.
    from_logic: ambre_bus::conduit::Sender<UiEvent<TrameEvent>>,
    clock: TestClock,
    /// The switch the chrome toggles, shared with the view.
    theme: ThemeControl,
}

impl Rig {
    fn new() -> Self {
        Self::with_queue(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<256>())
    }

    /// A view whose queue to the Logic thread holds `capacity` commands.
    fn with_queue(capacity: ambre_core::Bounded<ambre_bus::conduit::ChannelSlots>) -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(capacity);
        let (from_logic, rx) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<256>());
        let clock = TestClock::default();
        let theme = ThemeControl::default();
        let view = TrameView::new(
            Id::<Document>::mint(),
            ambre_core::Size::new(
                ambre_core::Px::<ambre_core::Surface>::new(1200.0),
                ambre_core::Px::<ambre_core::Surface>::new(800.0),
            ),
            TrameWiring {
                to_logic: ambre_bus::Port::direct(tx),
                from_logic: ambre_bus::Feed::direct(rx),
                text_input: TextInput::detached(),
                embedding: ambre_canvas::Embedding::Standalone,
                anchor: AnchorPublisher::new(),
                perf_pub: PerfPublisher::new(),
                busy_pub: BusyPublisher::new(),
                filter_open: FilterOpenPublisher::new(),
                theme_ctl: theme.clone(),
                clock: Box::new(clock.clone()),
            },
        );
        Self {
            view,
            to_logic,
            from_logic,
            clock,
            theme,
        }
    }

    fn key(&mut self, key: Key) -> bool {
        self.view
            .handle_input(&InputEvent::Key {
                key,
                mods: ModifierKeys::NONE,
            })
            .is_yes()
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.view
            .handle_input(&InputEvent::Text {
                text: text.to_owned(),
            })
            .is_yes()
    }

    /// Run one frame as the host does: drain, plan, then paint into a
    /// recording surface, where a deferred commit lands.
    fn frame(&mut self) {
        self.view.update();
        self.view
            .plan_repaint(self.clock.now(), ambre_canvas::HudTick::Idle);
        let mut surface = RecordingSurface::new();
        let mut font_cx = FontContext::new();
        let mut cache = TextCache::new();
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
            document_id: Id::<Document>::mint(),
            event,
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
                LogicCommand::App { command, .. } => Some(command),
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
    assert!(rig.key(Key::Enter));
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

/// `TRAME-UI-183`: a text typed after `Tab`, before the frame that commits,
/// opens the next cell's session. It never joins the buffer the `Tab` commits.
/// A fast typist, or a driven browser, types `21`, `Tab`, `=A1*2` in one frame.
#[test]
fn a_text_typed_before_the_commit_frame_reaches_the_next_cell() {
    let mut rig = Rig::new();
    rig.frame();
    rig.sent();
    rig.type_text("21");
    rig.key(Key::Tab);
    rig.type_text("=A1*2");
    rig.key(Key::Enter);
    rig.frame();
    rig.frame();
    let commits: Vec<(i64, i64, String)> = {
        let sent = rig.sent();
        let mut opened = crate::hash::HashMap::default();
        sent.iter()
            .filter_map(|c| match c {
                TrameCommand::BeginCellEdit { session, row, col } => {
                    opened.insert(*session, (row.get() as i64, col.get() as i64));
                    None
                }
                TrameCommand::CommitCellEdit { session, text } => {
                    let (r, c) = opened[session];
                    Some((r, c, text.as_str().to_owned()))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        commits,
        vec![(0, 0, "21".to_owned()), (0, 1, "=A1*2".to_owned())]
    );
    assert_eq!(rig.active(), (1, 1));
}

/// The caret blinks on the clock the view was handed, not on the browser's.
#[test]
fn the_caret_blinks_on_the_clock_the_view_is_handed() {
    let mut rig = Rig::new();
    rig.clock.set(1_000.0);
    rig.type_text("a");
    assert!(rig.view.caret_on);
    rig.clock.set(1_000.0 + BLINK_PERIOD.get() - 1.0);
    rig.frame();
    assert!(rig.view.caret_on, "half a period has not passed");
    rig.clock.set(1_000.0 + BLINK_PERIOD.get());
    rig.frame();
    assert!(!rig.view.caret_on, "half a period passed");
}

/// A blob carrying a grid payload whose `src_row` names where it came from,
/// with `text` as its `text/plain` mirror.
fn clip(src_row: u32, text: &str) -> ambre_bus::ClipboardBlob {
    let payload = crate::message::ClipboardPayload {
        src_row: Pos::<Row>::new(src_row),
        rows: Len::<Row>::new(1),
        cols: Len::<Col>::new(1),
        ..Default::default()
    };
    ambre_bus::ClipboardBlob {
        text: text.to_owned(),
        html: None,
        ambre: postcard::to_allocvec(&payload)
            .ok()
            .map(ambre_core::Payload::<ambre_core::ClipboardBytes>::new),
        form: ambre_core::PasteForm::Rich,
    }
}

/// The payload one `PasteRange` carried, or `None` for any other traffic.
fn pasted(sent: &[TrameCommand]) -> Option<&crate::message::ClipboardPayload> {
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
    for key in [
        Key::Char(KeyChar::A),
        Key::Char(KeyChar::Z),
        Key::Char(KeyChar::new('é')),
        Key::Space,
        Key::Char(KeyChar::new('1')),
        Key::Char(KeyChar::new('=')),
    ] {
        assert!(!rig.key(key), "{key:?} was consumed on the grid");
    }
    rig.type_text("x");
    for key in [
        Key::Char(KeyChar::A),
        Key::Char(KeyChar::Z),
        Key::Char(KeyChar::new('é')),
        Key::Space,
        Key::Char(KeyChar::new('1')),
        Key::Char(KeyChar::new('=')),
    ] {
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
        CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)),
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
        Some(Pos::<Row>::new(7)),
        "own copy"
    );
    rig.view
        .paste_clipboard(clip(3, "read"), PasteSource::Foreign);
    assert_eq!(
        pasted(&rig.sent()).map(|p| p.src_row),
        Some(Pos::<Row>::new(3)),
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
    assert!(rig.key(Key::Enter));
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

/// Covers `TRAME-UI-186`: the copy reply hands the host an HTML table beside
/// the mirror. The ownership token then has HTML to ride in on every browser.
#[test]
fn a_copy_hands_the_host_an_html_table() {
    let mut rig = Rig::new();
    let payload = crate::message::ClipboardPayload {
        rows: Len::<Row>::new(1),
        cols: Len::<Col>::new(2),
        cells: vec![
            crate::message::ClipboardCell {
                raw: "=A1+1".to_owned(),
                ..Default::default()
            },
            crate::message::ClipboardCell {
                dc: Len::<Col>::new(1),
                raw: "a<b".to_owned(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    rig.answer(TrameEvent::Clipboard(payload));
    let blob = rig
        .view
        .poll_clipboard()
        .expect("the copy reply yields a blob");
    assert_eq!(blob.text, "=A1+1\ta<b");
    assert_eq!(
        blob.html.as_deref(),
        Some("<table><tr><td>=A1+1</td><td>a&lt;b</td></tr></table>")
    );
    assert_eq!(
        rig.view.last_clip.as_ref(),
        Some(&blob),
        "the kept copy matches"
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
    let cell = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)))
        .expect("A1 is in view");
    assert!(
        first.x0 > cell.x0 && first.x0 < cell.x1,
        "inside the cell: {first:?}"
    );
    assert!(
        first.y0 >= cell.y0 && first.y1 <= cell.y1,
        "inside the cell: {first:?}"
    );
    rig.type_text("yyyy");
    rig.frame();
    let later = rig.view.caret_drawn.expect("the editor drew a caret");
    assert!(later.x0 > first.x0, "the caret moved right: {later:?}");
    assert!(rig.key(Key::Escape));
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
    let idle = rig
        .view
        .plan_repaint(rig.clock.now(), ambre_canvas::HudTick::Idle);
    assert!(
        matches!(idle.content, ContentRepaint::Reuse),
        "an idle frame reuses"
    );

    let next = rig.theme.toggle();
    rig.view.update();
    let plan = rig
        .view
        .plan_repaint(rig.clock.now(), ambre_canvas::HudTick::Idle);
    assert!(
        matches!(plan.content, ContentRepaint::Full),
        "{:?}",
        plan.content
    );
    let mut surface = RecordingSurface::new();
    let mut font_cx = FontContext::new();
    let mut cache = TextCache::new();
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
        && state.visible_offset(at.row) < Delta::<Row>::from_len(state.viewport_rows)
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
        active: Some(Pos::<ambre_bus::app::find::Match>::new(1)),
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
            CellPos::new(Pos::<Row>::new(2), Pos::<Col>::new(1)),
            CellPos::new(Pos::<Row>::new(4), Pos::<Col>::new(1)),
        ],
        active: Some(Pos::<ambre_bus::app::find::Match>::new(0)),
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
        rig.key(Key::ArrowRight);
        rig.key(Key::ArrowDown);
    }
    rig.type_text("hi");
    rig.frame();
    let cell = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Pos::<Row>::new(3), Pos::<Col>::new(3)))
        .expect("D4 is in view");
    let point = (cell.x0 + 1.0, cell.center().y);
    let _ = rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(point.0, point.1),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
    });
    let _ = rig.view.handle_input(&InputEvent::PointerUp {
        at: pointer(point.0, point.1),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
    });
    rig.type_text("X");
    rig.frame();
    let editor = rig.view.editor.as_ref().expect("still open");
    assert_eq!(editor.at, at(3, 3));
    assert_eq!(editor.text(), "Xhi");
}

impl Rig {
    /// Press `key` with the given modifiers.
    fn chord(&mut self, key: Key, mods: Flags<ModifierKeys>) -> bool {
        self.view
            .handle_input(&InputEvent::Key { key, mods })
            .is_yes()
    }

    /// Store `raw` in the grid's cache, shown as `shown`, as a viewport answer
    /// would leave it.
    fn store(&mut self, (row, col): (u32, u32), raw: &str, shown: &str) {
        self.view.state.set_cell_text(
            CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)),
            raw.to_owned(),
        );
        if let Some(cell) = self
            .view
            .state
            .cell_cache
            .get_mut(&CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(col)))
        {
            cell.text = shown.to_owned();
        }
    }

    /// Cut the Logic thread off, so every later send is refused.
    fn lose_logic(&mut self) {
        let (_tx, rx) = ambre_bus::conduit::local(ambre_core::Bounded::<
            ambre_bus::conduit::ChannelSlots,
        >::of::<1>());
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
        CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)),
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
        CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
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
    rig.view.formula.focused = BarFocus::Focused;
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
    rig.key(Key::ArrowRight);
    rig.key(Key::ArrowDown);
    rig.key(Key::ArrowDown);
    rig.frame();
    assert_eq!(rig.view.formula.reference, "B3");
}

/// The header of the context panel names the whole selection, such as
/// `B2:D5`, and a lone cell by its own reference.
#[test]
fn the_panel_header_names_the_selection() {
    let mut rig = Rig::new();
    rig.frame();
    assert_eq!(rig.view.selection_ref().as_str(), "A1");
    let (b2, d5) = (
        CellPos::new(Pos::new(1), Pos::new(1)),
        CellPos::new(Pos::new(4), Pos::new(3)),
    );
    let mut range = crate::ui::grid::Selection::single(b2);
    range.extend_to(d5);
    rig.view.state.selection = range;
    rig.frame();
    assert_eq!(rig.view.selection_ref().as_str(), "B2:D5");
}

/// `CHROME-246`: the range bar shows over two cells or more, centred on the
/// top edge. A lone cell and a drag hide it.
#[test]
fn the_range_bar_shows_over_a_range_alone() {
    let mut rig = Rig::new();
    rig.frame();
    assert_eq!(
        rig.view.range_bar_anchor(),
        None,
        "a lone cell shows no bar"
    );
    let (b2, d5) = (
        CellPos::new(Pos::new(1), Pos::new(1)),
        CellPos::new(Pos::new(4), Pos::new(3)),
    );
    let mut range = crate::ui::grid::Selection::single(b2);
    range.extend_to(d5);
    rig.view.state.selection = range;
    rig.frame();
    let at = rig
        .view
        .range_bar_anchor()
        .expect("a range shows the bar")
        .at;
    let (left, right) = (
        rig.view.state.cell_pixel_rect(b2).expect("B2 in view"),
        rig.view
            .state
            .cell_pixel_rect(CellPos::new(Pos::new(1), Pos::new(3)))
            .expect("D2 in view"),
    );
    assert!((f64::from(at.x.get()) - f64::midpoint(left.x0, right.x1)).abs() < 0.5);
    assert!(
        (f64::from(at.y.get()) - left.y0).abs() < 0.5,
        "on the top edge"
    );
    rig.view.selecting_drag = true;
    assert_eq!(rig.view.range_bar_anchor(), None, "a drag hides the bar");
}

/// `TRAME-UI-155` and `TRAME-UI-092`: the focus alone opens nothing. A text
/// typed in the bar opens a session, and the focus stays in the bar.
#[test]
fn only_a_text_typed_in_the_bar_opens_a_session() {
    let mut rig = Rig::new();
    rig.store((0, 0), "5", "5");
    rig.frame();
    rig.sent();
    rig.view.formula.focused = BarFocus::Focused;
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
    rig.view.formula.focused = BarFocus::Focused;
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
    rig.key(Key::F(FunctionKey::F2));
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
    rig.chord(Key::Enter, ModifierKeys::ALT);
    rig.type_text("b");
    rig.frame();
    assert!(rig.view.editor.is_some(), "Alt+Enter committed");
    assert!(rig.key(Key::Enter));
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
    let _ = rig.view.handle_input(&InputEvent::CompositionUpdate {
        text: "ka".to_owned(),
    });
    rig.frame();
    for key in [
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::Home,
        Key::End,
        Key::Backspace,
        Key::Delete,
    ] {
        rig.key(key);
    }
    rig.chord(Key::ArrowLeft, ModifierKeys::SHIFT);
    rig.chord(Key::Char(KeyChar::A), ModifierKeys::PRIMARY);
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
    let _ = rig.view.handle_input(&InputEvent::CompositionEnd {
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
    rig.key(Key::Enter);
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
    rig.key(Key::Enter);
    rig.frame();
    assert_eq!(committed(&rig.sent()), Some("1"));
    assert_eq!(rig.active(), (1, 0));
    rig.type_text("2");
    rig.key(Key::Tab);
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
    rig.key(Key::Enter);
    rig.frame();
    assert_eq!(
        rig.view
            .cell_seed(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
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
    assert!(rig.key(Key::Escape));
    rig.frame();
    let sent = rig.sent();
    assert!(
        matches!(sent.as_slice(), [TrameCommand::EndCellEdit { .. }]),
        "sent {sent:?}"
    );
    assert!(rig.view.editor.is_none());
    assert_eq!(
        rig.view
            .cell_seed(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
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
        CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)),
    ));
    rig.sent();
    rig.type_text("=A1+1");
    rig.chord(Key::Enter, ModifierKeys::PRIMARY);
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
        let _ = self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
        let _ = self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
    }

    /// The right edge of column `c` on screen.
    fn col_edge(&self, c: u32) -> f64 {
        self.view.state.col_x(Pos::<Col>::new(c)).get()
            + self.view.state.col_width_of(Pos::<Col>::new(c)).get()
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
        Some(crate::ui::grid::HeaderHit::ColInsert(Pos::<Col>::new(2)))
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
    let y = rig.view.state.row_y(Pos::<Row>::new(3)).get()
        + rig.view.state.row_height_of(Pos::<Row>::new(3)).get();
    assert_eq!(
        rig.view.state.header_hit(pointer(2.0, y)),
        Some(crate::ui::grid::HeaderHit::RowInsert(Pos::<Row>::new(4)))
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

/// A drag far past the largest size holds the guide at that size, and sends
/// that size, at every zoom (`TRAME-UI-181`). The guide then never jumps when
/// the Logic thread answers.
#[test]
fn a_drag_past_the_largest_size_holds_the_guide_there() {
    use crate::ui::grid::{HeaderHit, MAX_COL_WIDTH, MAX_ROW_HEIGHT, ResizePreview};
    for zoom in [1.0, 2.0] {
        let mut rig = Rig::new();
        assert!(
            rig.view
                .set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(zoom))
                || zoom == 1.0
        );
        rig.frame();
        rig.sent();
        let row_1 = Pos::<Row>::new(1);
        let col_y = rig.view.state.header_h().get() * 0.8;
        let row_x = rig.view.state.header_w().get() * 0.5;
        let row_edge =
            rig.view.state.row_y(row_1).get() + rig.view.state.row_height_of(row_1).get();
        let drags = [
            (pointer(rig.col_edge(1), col_y), HeaderHit::ColBorder(COL_1)),
            (pointer(row_x, row_edge), HeaderHit::RowBorder(row_1)),
        ];
        for (start, hit) in drags {
            assert_eq!(rig.view.state.header_hit(start), Some(hit));
            let _ = rig.view.handle_input(&InputEvent::PointerDown {
                at: start,
                button: PointerButton::Primary,
                mods: ModifierKeys::NONE,
            });
            let far = 10_000.0 * zoom as f32;
            let to = ambre_core::Point::new(
                start.x + ambre_core::Px::<ambre_core::View>::new(far),
                start.y + ambre_core::Px::<ambre_core::View>::new(far),
            );
            let _ = rig.view.handle_input(&InputEvent::PointerMove { at: to });
            let held = match hit {
                HeaderHit::ColBorder(c) => ResizePreview::Col(
                    rig.view.state.col_x(c) + rig.view.state.scaled(MAX_COL_WIDTH),
                ),
                _ => ResizePreview::Row(
                    rig.view.state.row_y(row_1) + rig.view.state.scaled(MAX_ROW_HEIGHT),
                ),
            };
            assert_eq!(rig.view.state.resize_preview, Some(held), "zoom {zoom}");
            let _ = rig.view.handle_input(&InputEvent::PointerUp {
                at: to,
                button: PointerButton::Primary,
                mods: ModifierKeys::NONE,
            });
        }
        let sent: Vec<ambre_core::Px<ambre_core::Doc>> = rig
            .sent()
            .iter()
            .filter_map(|cmd| match cmd {
                TrameCommand::ResizeCol { width, .. } => Some(*width),
                TrameCommand::ResizeRow { height, .. } => Some(*height),
                _ => None,
            })
            .collect();
        assert_eq!(sent, [MAX_COL_WIDTH, MAX_ROW_HEIGHT], "zoom {zoom}");
    }
}

/// A boundary dragged at any zoom stores the width in unzoomed pixels: the
/// same on-screen gesture scaled by the zoom stores the same size
/// (`TRAME-UI-040`). The grid takes the width before any answer
/// (`TRAME-UI-041`).
#[test]
fn a_resize_stores_an_unzoomed_width_and_reflows_at_once() {
    for zoom in [1.0, 2.0] {
        let mut rig = Rig::new();
        assert!(
            rig.view
                .set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(zoom))
                || zoom == 1.0
        );
        rig.frame();
        rig.sent();
        let edge = rig.col_edge(1);
        let y = rig.view.state.header_h().get() * 0.8;
        assert_eq!(
            rig.view.state.header_hit(pointer(edge, y)),
            Some(crate::ui::grid::HeaderHit::ColBorder(Pos::<Col>::new(1)))
        );
        let c_before = rig.view.state.col_x(Pos::<Col>::new(2)).get();
        let _ = rig.view.handle_input(&InputEvent::PointerDown {
            at: pointer(edge, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
        // 200 base pixels, drawn at this zoom.
        let target = rig.view.state.col_x(Pos::<Col>::new(1)).get() + 200.0 * zoom;
        let _ = rig.view.handle_input(&InputEvent::PointerMove {
            at: pointer(target, y),
        });
        let _ = rig.view.handle_input(&InputEvent::PointerUp {
            at: pointer(target, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
        let sent = rig.sent();
        let widths: Vec<ambre_core::Px<ambre_core::Doc>> = sent
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
            [ambre_core::Px::<ambre_core::Doc>::new(200.0)],
            "zoom {zoom}: sent {sent:?}"
        );
        // No answer came back, and the grid already reflowed.
        assert_eq!(
            rig.view.state.col_widths.get(&COL_1),
            Some(&ambre_core::Px::<ambre_core::Doc>::new(200.0))
        );
        assert_eq!(
            rig.view.state.col_x(Pos::<Col>::new(2)).get(),
            rig.view.state.col_x(Pos::<Col>::new(1)).get() + 200.0 * zoom
        );
        assert_ne!(rig.view.state.col_x(Pos::<Col>::new(2)).get(), c_before);
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
    let y = rig.view.state.header_h().get() * 0.8;
    assert!(
        rig.view
            .handle_input(&InputEvent::DoubleClick {
                at: pointer(edge, y)
            })
            .is_yes()
    );
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
    let expected =
        super::interact::fit_column_width(&cells, &mut FontContext::new(), &mut TextCache::new());
    rig.answer(TrameEvent::ColumnContent {
        col: Pos::<Col>::new(0),
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
        col: Pos::<Col>::new(0),
        is_merged: crate::message::Merge::Merged,
        merge_span: span,
        ..Default::default()
    };
    // A1:A2 is one merge of one column and two rows.
    rig.answer(TrameEvent::ViewportData {
        version: ambre_core::Seq::ZERO,
        row_start: Pos::<Row>::new(0),
        col_start: Pos::<Col>::new(0),
        col_widths: vec![],
        row_heights: vec![],
        frozen: crate::message::FrozenView::default(),
        cells: vec![
            merged(Pos::<Row>::new(0), Some(span(2, 1))),
            merged(Pos::<Row>::new(1), None),
        ],
    });
    rig.sent();
    let handle = rig
        .view
        .state
        .fill_handle_rect()
        .expect("a handle")
        .center();
    let _ = rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(handle.x, handle.y),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
    });
    let mid = |rig: &Rig, row: u32| {
        let centre = rig
            .view
            .state
            .cell_pixel_rect(CellPos::new(Pos::<Row>::new(row), Pos::<Col>::new(0)))
            .expect("in view")
            .center();
        (centre.x, centre.y)
    };
    // Row 2 is one row past the source, row 4 is three: each snaps up to the
    // block that holds it.
    for (row, last) in [(2, 3), (3, 3), (4, 5)] {
        let (x, y) = mid(&rig, row);
        let _ = rig
            .view
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
    fn wheel(&mut self, dy: f64, delta_mode: u32, mods: Flags<ModifierKeys>) -> bool {
        self.view
            .handle_input(&InputEvent::Wheel {
                delta: ambre_canvas::WheelDelta::new(
                    ambre_core::Vector::new(
                        ambre_canvas::WheelAmount::new(0.0),
                        ambre_canvas::WheelAmount::new(dy),
                    ),
                    ambre_canvas::WheelDeltaMode::from_dom(delta_mode),
                ),
                mods,
            })
            .is_yes()
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
    assert!(rig.wheel(3.0, 1, ModifierKeys::NONE));
    assert_eq!(rig.top(), 3, "three lines are three rows");
    let row = f64::from(ROW_HEIGHT.get());
    let page = (rig.view.state.canvas_height.get() / row) as u32;
    assert!(rig.wheel(1.0, 2, ModifierKeys::NONE));
    assert_eq!(rig.top(), 3 + page, "a page is a screen of rows");
    let at = rig.top();
    assert!(rig.wheel(row * 1.5, 0, ModifierKeys::NONE));
    assert_eq!(rig.top(), at + 1, "a row and a half moves one row");
    assert!(rig.wheel(row * 0.5, 0, ModifierKeys::NONE));
    assert_eq!(rig.top(), at + 2, "the half row was kept");
}

/// `TRAME-UI-026`: a notch moves the grid once, and the frames after it move
/// nothing.
#[test]
fn a_wheel_notch_starts_no_coast() {
    let mut rig = Rig::new();
    rig.frame();
    rig.clock.set(1_000.0);
    assert!(rig.wheel(3.0, 1, ModifierKeys::NONE));
    let landed = rig.top();
    for step in 1..=30 {
        rig.clock.set(1_000.0 + f64::from(step) * 16.0);
        rig.frame();
    }
    assert_eq!(rig.top(), landed, "the grid stays where the notch put it");
}

/// `TRAME-UI-146`, `ZOOM-003`, `ZOOM-004`: the wheel held with `Ctrl` zooms
/// the grid about the pointer. The cell under the pointer stays under it,
/// the selection stays, and the factor stays inside the grid's range.
#[test]
fn a_ctrl_wheel_zooms_about_the_cell_under_the_pointer() {
    let mut rig = Rig::new();
    rig.frame();
    assert!(rig.wheel(20.0, 1, ModifierKeys::NONE));
    rig.frame();
    rig.sent();
    let selection = rig.view.state.selection.clone();
    let at = {
        let rect = rig
            .view
            .state
            .cell_pixel_rect(CellPos::new(Pos::<Row>::new(30), Pos::<Col>::new(4)))
            .expect("in view");
        pointer(rect.center().x, rect.center().y)
    };
    let _ = rig.view.handle_input(&InputEvent::PointerMove { at });
    let under = rig.view.state.cell_at(at);
    for dy in [-3.0, -3.0, 5.0, -40.0, 60.0] {
        let zoom = rig.view.zoom();
        assert!(
            rig.wheel(dy, 1, ModifierKeys::PRIMARY),
            "the grid keeps the gesture"
        );
        let now = rig.view.zoom();
        assert!(
            if dy < 0.0 { now >= zoom } else { now <= zoom },
            "{dy}: {zoom:?} -> {now:?}"
        );
        // The grid cannot scroll left of its first column, so a zoom out at
        // the left edge keeps the row alone.
        let now_under = rig.view.state.cell_at(at).expect("on the cells");
        let under = under.expect("on the cells");
        assert_eq!(
            now_under.row, under.row,
            "{dy}: the row stays under the pointer"
        );
        assert!(
            now_under.col == under.col || rig.view.state.left_col.get() == 0,
            "{dy}: the column stays under the pointer unless the grid is at its first"
        );
        let range = crate::ui::grid::TrameUiState::ZOOM_RANGE;
        assert!(now >= range.floor() && now <= range.ceiling());
    }
    assert_eq!(rig.view.state.selection, selection, "the selection stays");
}

/// `TRAME-UI-175`: a selection drag held below the cells scrolls the grid
/// frame by frame, and the range follows to the last row on screen.
#[test]
fn a_selection_held_below_the_cells_scrolls_and_follows() {
    let mut rig = Rig::new();
    rig.frame();
    let centre = rig
        .view
        .state
        .cell_pixel_rect(CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1)))
        .expect("B2 is in view")
        .center();
    let (px, py) = (centre.x, centre.y);
    let _ = rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(px, py),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
    });
    let below = rig.view.state.canvas_height.get() + 40.0;
    let _ = rig.view.handle_input(&InputEvent::PointerMove {
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
        CellPos::new(Pos::<Row>::new(1), Pos::<Col>::new(1))
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
        let _ = self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Secondary,
            mods: ModifierKeys::NONE,
        });
        let _ = self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Secondary,
            mods: ModifierKeys::NONE,
        });
    }

    /// Press and release the primary button at `(x, y)` with Shift held.
    fn shift_click(&mut self, x: f64, y: f64) {
        let _ = self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(x, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::SHIFT,
        });
        let _ = self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(x, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
    }

    /// The centre of cell `(row, col)` on screen.
    fn cell_mid(&self, row: u32, col: u32) -> (f64, f64) {
        let s = &self.view.state;
        (
            s.col_x(Pos::<Col>::new(col)).get() + s.col_width_of(Pos::<Col>::new(col)).get() / 2.0,
            s.row_y(Pos::<Row>::new(row)).get() + s.row_height_of(Pos::<Row>::new(row)).get() / 2.0,
        )
    }

    /// The centre of column `col`'s header body.
    fn col_header_mid(&self, col: u32) -> (f64, f64) {
        (
            self.cell_mid(0, col).0,
            self.view.state.header_h().get() / 2.0,
        )
    }

    /// The centre of row `row`'s header body.
    fn row_header_mid(&self, row: u32) -> (f64, f64) {
        (
            self.view.state.header_w().get() / 2.0,
            self.cell_mid(row, 0).1,
        )
    }

    /// The centre of the corner above the row headers.
    fn corner_mid(&self) -> (f64, f64) {
        (
            self.view.state.header_w().get() / 2.0,
            self.view.state.header_h().get() / 2.0,
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
            (rig.col_edge(1), rig.view.state.header_h().get() / 2.0),
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
        Selection::single(CellPos::new(Pos::<Row>::new(4), Pos::<Col>::new(3)))
    );

    rig.view.state.selection = range.clone();
    let (x, y) = rig.col_header_mid(3);
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::axis(
            AxisPos::Col(Pos::<Col>::new(3)),
            AxisPos::Col(Pos::<Col>::new(3))
        )
    );

    rig.view.state.selection = range.clone();
    let (x, y) = rig.row_header_mid(4);
    rig.right_click(x, y);
    assert_eq!(
        rig.view.state.selection,
        Selection::axis(AxisPos::Row(Pos::<Row>::new(4)), AxisPos::Row(ROW_4))
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
                row: Pos::<Row>::new(row),
                col: Pos::<Col>::new(col),
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
        let Some(cmd) = registry::command_to_app(id) else {
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

/// A refresh the full queue refused fills the band on a later frame, with no
/// new selection change (`TRAME-UI-185`, `BUS-024`). The request leaves once,
/// and the Logic thread's answer to it is the aggregate the band shows.
#[test]
fn a_refused_stats_request_fills_the_band_on_a_later_frame() {
    let asks = |sent: &[TrameCommand]| {
        sent.iter()
            .filter(|c| matches!(c, TrameCommand::RequestSelectionStats))
            .count()
    };
    let mut rig =
        Rig::with_queue(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<1>());
    let mut logic = crate::TrameState::new();
    for row in 0..3 {
        logic.handle_command(TrameCommand::SetCellText {
            row: Pos::<Row>::new(row),
            col: Pos::<Col>::new(0),
            text: format!("{}", 10 * (row + 1)),
        });
    }
    rig.frame();
    rig.sent();
    let (x, y) = rig.cell_mid(0, 0);
    rig.click(x, y);
    for cmd in rig.sent() {
        logic.handle_command(cmd);
    }
    // The selection command takes the one slot, and the request finds the
    // queue full.
    let (x, y) = rig.cell_mid(2, 0);
    rig.shift_click(x, y);
    let first = rig.sent();
    assert_eq!(asks(&first), 0, "the full queue took the request");
    for cmd in first {
        logic.handle_command(cmd);
    }
    rig.frame();
    let later = rig.sent();
    assert_eq!(asks(&later), 1, "the refused request was not offered again");
    let mut band = None;
    for cmd in later {
        for event in logic.handle_command(cmd).display {
            if let TrameEvent::SelectionStats { count, numbers } = &event {
                band = Some((count.get(), numbers.map(|n| n.count().get())));
            }
            rig.answer(event);
        }
    }
    assert_eq!(
        band,
        Some((3, Some(3))),
        "the band shows the held selection"
    );
    rig.frame();
    assert_eq!(asks(&rig.sent()), 0, "the request was offered twice");
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
        let y = self.view.state.header_h().get() * 0.8;
        let _ = self.view.handle_input(&InputEvent::PointerDown {
            at: pointer(edge, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
        let target = self.view.state.col_x(Pos::<Col>::new(1)).get() + width;
        let _ = self.view.handle_input(&InputEvent::PointerMove {
            at: pointer(target, y),
        });
        let _ = self.view.handle_input(&InputEvent::PointerUp {
            at: pointer(target, y),
            button: PointerButton::Primary,
            mods: ModifierKeys::NONE,
        });
    }
}

/// `TRAME-UI-127` and `BUS-018`: a resize the Logic thread never receives
/// leaves the column at its width.
#[test]
fn a_refused_resize_leaves_the_width() {
    let mut rig = Rig::new();
    rig.frame();
    let before = rig.view.state.col_x(Pos::<Col>::new(2)).get();
    rig.lose_logic();
    rig.drag_col_1_to(200.0);
    assert_eq!(rig.view.state.col_widths.get(&COL_1), None);
    assert_eq!(rig.view.state.col_x(Pos::<Col>::new(2)).get(), before);
}

/// `TRAME-UI-125` and `BUS-018`: a cut whose paste never left keeps its
/// source, and stays pending for the next paste.
#[test]
fn a_refused_paste_leaves_the_cut_source() {
    let mut rig = Rig::new();
    rig.store((0, 0), "a", "a");
    rig.frame();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0)),
    ));
    rig.view.clipboard_write(ClipboardWrite::Cut);
    rig.sent();
    rig.view.state.select_rect(CellRect::spanning(
        CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
        CellPos::new(Pos::<Row>::new(5), Pos::<Col>::new(0)),
    ));
    rig.lose_logic();
    rig.view.paste_clipboard(clip(0, "a"), PasteSource::Foreign);
    assert_eq!(
        rig.view
            .cell_seed(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
        "a"
    );
    assert_eq!(rig.view.state.clipboard_anchor, Some(rect(0, 0, 0, 0)));
    assert_eq!(rig.view.state.active_row(), Pos::<Row>::new(5));
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
        rig.view
            .cell_seed(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
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
    let handle = rig
        .view
        .state
        .fill_handle_rect()
        .expect("a handle")
        .center();
    let _ = rig.view.handle_input(&InputEvent::PointerDown {
        at: pointer(handle.x, handle.y),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
    });
    let (x, y) = rig.cell_mid(3, 0);
    let _ = rig
        .view
        .handle_input(&InputEvent::PointerMove { at: pointer(x, y) });
    assert_eq!(rig.view.state.fill_preview, Some(rect(0, 0, 3, 0)));
    rig.lose_logic();
    let _ = rig.view.handle_input(&InputEvent::PointerUp {
        at: pointer(x, y),
        button: PointerButton::Primary,
        mods: ModifierKeys::NONE,
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
    assert!(rig.key(Key::Delete));
    assert_eq!(
        rig.view
            .cell_seed(CellPos::new(Pos::<Row>::new(0), Pos::<Col>::new(0))),
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
            rig.key(Key::Delete);
        }),
        ("commit", |rig| {
            rig.key(Key::Enter);
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
    let mut rig =
        Rig::with_queue(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<1>());
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
                        command: TrameCommand::ScrollViewport { .. },
                        ..
                    }
                )
            })
            .count()
    };
    let mut rig =
        Rig::with_queue(ambre_core::Bounded::<ambre_bus::conduit::ChannelSlots>::of::<1>());
    rig.frame();
    rig.to_logic.try_iter().for_each(drop);
    let cmd = rig.view.set_active_cmd();
    assert!(rig.view.send_command_lossless(cmd).is_ok());
    assert!(rig.wheel(5.0, 1, ModifierKeys::NONE));
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

/// The bounds of one viewport request, as `(row_start, row_end, col_start,
/// col_end)`.
fn bounds(cmd: &TrameCommand) -> Option<(u32, u32, u32, u32)> {
    match cmd {
        TrameCommand::ScrollViewport {
            row_start,
            row_end,
            col_start,
            col_end,
            ..
        } => Some((
            row_start.get(),
            row_end.get(),
            col_start.get(),
            col_end.get(),
        )),
        _ => None,
    }
}

/// `TRAME-UI-184`: a change the Logic thread names cell by cell re-fetches
/// those cells alone, where a whole-state change re-fetches the window.
#[test]
fn a_ranged_change_refetches_its_cells_and_a_reset_the_window() {
    let mut rig = Rig::new();
    rig.frame();
    let first = rig.sent();
    let window = first.iter().find_map(bounds).expect("the opening fetch");
    let served = first
        .iter()
        .find_map(|c| match c {
            TrameCommand::ScrollViewport { version, .. } => Some(*version),
            _ => None,
        })
        .expect("a version");
    rig.answer(TrameEvent::CellsChanged {
        version: served,
        range: CellRect::spanning(
            CellPos::new(ROW_1, Pos::<Col>::new(2)),
            CellPos::new(ROW_4, Pos::<Col>::new(3)),
        ),
    });
    let ranged: Vec<_> = rig.sent().iter().filter_map(bounds).collect();
    assert_eq!(ranged, vec![(1, 4, 2, 3)], "the named cells alone");
    rig.answer(TrameEvent::DataReset);
    let whole: Vec<_> = rig.sent().iter().filter_map(bounds).collect();
    assert_eq!(whole, vec![window], "the whole window");
}

/// A first fetch that left before the open of its document reached no live
/// document, and the Logic thread dropped it. Once the shell sends the open,
/// the view asks for its whole window again, once.
#[test]
fn an_open_sent_after_the_first_fetch_fetches_the_window_again() {
    let mut rig = Rig::new();
    rig.frame();
    let window = rig
        .sent()
        .iter()
        .find_map(bounds)
        .expect("the opening fetch");
    rig.frame();
    assert!(rig.sent().iter().all(|c| bounds(c).is_none()));
    rig.view.document_opened();
    rig.frame();
    let again: Vec<_> = rig.sent().iter().filter_map(bounds).collect();
    assert_eq!(again, vec![window], "the whole window, after the open");
    rig.frame();
    assert!(
        rig.sent().iter().all(|c| bounds(c).is_none()),
        "asked twice"
    );
}

/// The content and the overlay the view paints now, recorded.
fn painted(rig: &mut Rig) -> RecordingSurface {
    let mut surface = RecordingSurface::new();
    let mut font_cx = FontContext::new();
    let mut cache = TextCache::new();
    rig.view
        .paint_content(&mut surface, &mut font_cx, &mut cache, None);
    rig.view.paint_overlay(
        &mut surface,
        &mut font_cx,
        &mut cache,
        &FrameStats::default(),
    );
    surface
}

/// `PDF-003`: the finished file reaches the download stub once, by name and
/// bytes. The view sends no command, keeps its selection and paints the same
/// operations.
#[test]
fn a_finished_pdf_is_handed_to_the_browser_and_nothing_else() {
    let mut rig = Rig::new();
    rig.frame();
    assert!(rig.key(Key::ArrowDown));
    rig.frame();
    let _ = rig.sent();
    let selection = rig.view.state.selection.bounds();
    let active = rig.active();
    let before = painted(&mut rig);
    super::DOWNLOADS.with(|d| d.borrow_mut().clear());

    rig.answer(TrameEvent::Interop(
        ambre_bus::interop::InteropAnswer::ExportReady {
            file: ambre_bus::interop::ExportedFile {
                filename: ambre_client_core::ExportFilename::named(
                    "Sheet1",
                    ambre_client_core::ExportExtension::Pdf,
                ),
                media_type: ambre_core::MediaType::from("application/pdf"),
                data: ambre_core::Payload::<ambre_client_core::FileBytes>::new(
                    b"%PDF-1.7".to_vec(),
                ),
            },
            report: crate::message::TrameReport::new(
                ambre_bus::interop::Direction::Export,
                ambre_bus::interop::FileFormat::Pdf,
            ),
        },
    ));

    let handed = super::DOWNLOADS.with(|d| d.borrow().clone());
    assert_eq!(handed, [("Sheet1.pdf".to_owned(), b"%PDF-1.7".to_vec())]);
    assert!(rig.sent().is_empty(), "the handoff sent a command");
    assert_eq!(rig.view.state.selection.bounds(), selection);
    assert_eq!(rig.active(), active);
    assert_eq!(painted(&mut rig).ops(), before.ops(), "the grid changed");
}

impl Rig {
    /// The point the last menu key opened the context menu at, read once.
    fn menu_opened_at(&mut self) -> Option<(f64, f64)> {
        self.view
            .menu_key_at
            .take()
            .map(|at| (f64::from(at.x.get()), f64::from(at.y.get())))
    }

    /// The lower-left corner of cell `(row, col)` on screen.
    fn cell_lower_left(&self, row: u32, col: u32) -> (f64, f64) {
        let s = &self.view.state;
        (
            s.col_x(Pos::<Col>::new(col)).get(),
            s.row_y(Pos::<Row>::new(row)).get() + s.row_height_of(Pos::<Row>::new(row)).get(),
        )
    }
}

/// `TRAME-UI-188`, `TRAME-UI-189`: the context-menu key, alone or with
/// Shift, and `Shift+F10` open the cell menu at the lower-left corner of the
/// active cell. The selection stays, and nothing reaches the Logic thread.
/// A plain `F10`, a key under `Ctrl` and a key in an open editor stay the
/// browser's.
#[test]
fn the_menu_keys_open_the_menu_at_the_active_cell() {
    use crate::ui::grid::CtxScope;
    let mut rig = Rig::new();
    rig.frame();
    let range = rig.select_range((2, 1), (4, 3));
    let _ = rig.sent();
    let corner = rig.cell_lower_left(4, 3);
    rig.view.set_ctx_scope(CtxScope::Corner);

    assert!(rig.chord(Key::ContextMenu, ModifierKeys::NONE));
    assert_eq!(rig.menu_opened_at(), Some(corner), "the context-menu key");
    assert_eq!(rig.view.menu_scope, CtxScope::Cell);
    assert!(rig.chord(Key::ContextMenu, ModifierKeys::SHIFT));
    assert_eq!(rig.menu_opened_at(), Some(corner), "Shift and the menu key");
    assert!(rig.chord(Key::F(FunctionKey::F10), ModifierKeys::SHIFT));
    assert_eq!(rig.menu_opened_at(), Some(corner), "Shift+F10");

    assert!(!rig.chord(Key::F(FunctionKey::F10), ModifierKeys::NONE));
    assert!(!rig.chord(Key::ContextMenu, ModifierKeys::PRIMARY));
    assert_eq!(rig.menu_opened_at(), None, "a plain F10 is the browser's");

    assert_eq!(rig.view.state.selection, range);
    assert!(rig.sent().is_empty(), "opening the menu writes nothing");

    // An open editor keeps the keys, and the browser opens its text menu.
    assert!(rig.key(Key::F(FunctionKey::F2)));
    assert!(rig.view.is_editing());
    assert!(!rig.chord(Key::F(FunctionKey::F10), ModifierKeys::SHIFT));
    assert_eq!(rig.menu_opened_at(), None, "the editor's key");
}

/// `TRAME-UI-188`: a key names no target, so the shape of the selection
/// picks the scope. A band of columns takes the column menu, a band of rows
/// the row menu.
#[test]
fn the_menu_key_scopes_the_menu_to_the_selection_shape() {
    use crate::ui::grid::CtxScope;
    let mut rig = Rig::new();
    rig.frame();
    let (x, y) = rig.col_header_mid(2);
    rig.click(x, y);
    assert!(rig.chord(Key::ContextMenu, ModifierKeys::NONE));
    assert_eq!(rig.view.menu_scope, CtxScope::Column);
    let (x, y) = rig.row_header_mid(5);
    rig.click(x, y);
    assert!(rig.chord(Key::ContextMenu, ModifierKeys::NONE));
    assert_eq!(rig.view.menu_scope, CtxScope::Row);
    let (x, y) = rig.cell_mid(1, 1);
    rig.click(x, y);
    assert!(rig.chord(Key::ContextMenu, ModifierKeys::NONE));
    assert_eq!(rig.view.menu_scope, CtxScope::Cell);
}

/// `TRAME-UI-189`: an active cell out of the grid area holds the menu inside
/// it. The menu sits at the edge the cell lies past, never over a header.
#[test]
fn an_active_cell_past_the_grid_holds_the_menu_inside_it() {
    let mut rig = Rig::new();
    rig.frame();
    let s = &rig.view.state;
    let (header_w, header_h) = (s.header_w().get(), s.header_h().get());
    let (right, bottom) = (s.canvas_width.get(), s.grid_bottom().get());

    rig.view.state.selection = Selection::single(at(500, 200));
    assert!(rig.chord(Key::ContextMenu, ModifierKeys::NONE));
    assert_eq!(
        rig.menu_opened_at(),
        Some((right, bottom)),
        "below and right"
    );

    rig.view.state.selection = Selection::single(at(0, 0));
    rig.view.state.top_row = Pos::<Row>::new(100);
    rig.view.state.left_col = Pos::<Col>::new(20);
    assert!(rig.chord(Key::F(FunctionKey::F10), ModifierKeys::SHIFT));
    assert_eq!(
        rig.menu_opened_at(),
        Some((header_w, header_h)),
        "above and left"
    );
}

/// `ZOOM-004`, `ZOOM-005`: the zoom chords step the grid along the one ladder
/// and return it to one. The grid and an open editor behave alike. `ZOOM-001`,
/// `ZOOM-008`: the only message a zoom sends is the cull of the new view. No
/// edit and no presence sample leaves the view.
#[test]
fn a_zoom_chord_steps_the_grid_and_sends_only_a_cull() {
    let ctrl = ModifierKeys::PRIMARY;
    let mut rig = Rig::new();
    rig.frame();
    rig.type_text("x");
    rig.frame();
    rig.view.flush_outbox();
    let _ = rig.to_logic.try_iter().count();
    assert!(rig.chord(Key::Char(KeyChar::new('=')), ctrl));
    assert_eq!(rig.view.zoom().get(), 1.25);
    assert!(rig.chord(Key::Char(KeyChar::new('-')), ctrl));
    assert!(rig.chord(Key::Char(KeyChar::new('-')), ctrl));
    assert_eq!(rig.view.zoom().get(), 0.75);
    assert!(rig.chord(Key::Char(KeyChar::new('0')), ctrl));
    assert_eq!(rig.view.zoom().get(), 1.0);
    assert!(rig.view.editor.is_some(), "a zoom closed the editor");
    rig.frame();
    rig.view.flush_outbox();
    let sent: Vec<_> = rig.to_logic.try_iter().collect();
    assert!(!sent.is_empty(), "a zoom culls the new view");
    assert!(
        sent.iter().all(|msg| matches!(
            msg,
            LogicCommand::App {
                command: TrameCommand::ScrollViewport { .. },
                ..
            }
        )),
        "a zoom sent {sent:?}"
    );
}

// -- Home regions (`ambre-spec-shell.md` section 2) ---------------------------

/// The workbook with each empty map and empty list left out. A command may
/// leave behind an empty container it touched, and its undo cannot remove the
/// container. An empty container reads as an absent one.
fn settled(value: loro::LoroValue) -> loro::LoroValue {
    use loro::LoroValue;
    match value {
        LoroValue::Map(map) => LoroValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), settled(v.clone())))
                .filter(|(_, v)| !matches!(v, LoroValue::Map(m) if m.is_empty()))
                .filter(|(_, v)| !matches!(v, LoroValue::List(l) if l.is_empty()))
                .collect::<loro::LoroMapValue>(),
        ),
        LoroValue::List(list) => {
            LoroValue::List(list.iter().cloned().map(settled).collect::<Vec<_>>().into())
        }
        other => other,
    }
}

/// A grid view wired to a real Logic state. What the view sends runs against
/// the state, and what the state answers goes back to the view.
struct Wired {
    rig: Rig,
    state: crate::TrameState,
}

impl Wired {
    /// A sheet whose block A1:C3 holds values, with B2:C3 selected, settled.
    /// A value at J20 grows the sheet first. The lines a write forces into
    /// being stay out of the undo history by design. That is
    /// `CommitOrigin::Materialize`. A command run inside the grown sheet
    /// forces none.
    fn filled() -> Self {
        let mut state = crate::TrameState::new();
        let mut cells: Vec<_> = (0..3u32)
            .flat_map(|r| (0..3u32).map(move |c| (r, c)))
            .map(|(r, c)| {
                (
                    CellPos::new(Pos::<Row>::new(r), Pos::<Col>::new(c)),
                    format!("{}", 9 - r * 3 - c),
                )
            })
            .collect();
        cells.push((
            CellPos::new(Pos::<Row>::new(19), Pos::<Col>::new(9)),
            "end".to_owned(),
        ));
        state.handle_command(TrameCommand::SetCellsBatch { cells });
        state.handle_command(TrameCommand::CommitEdit);
        state.handle_command(TrameCommand::SetActiveCell {
            row: Pos::<Row>::new(2),
            col: COL_2,
            anchor_row: ROW_1,
            anchor_col: COL_1,
            extra: vec![],
            kind: crate::SelectionKind::Cells,
        });
        let mut wired = Self {
            rig: Rig::new(),
            state,
        };
        wired.rig.frame();
        wired.pump();
        wired.rig.view.state.select_rect(CellRect::spanning(
            CellPos::new(ROW_1, COL_1),
            CellPos::new(Pos::<Row>::new(2), COL_2),
        ));
        // A zoom off 100 %, so a write that reset it would show.
        let _ = wired
            .rig
            .view
            .set_zoom(ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(1.5));
        wired.rig.frame();
        wired.pump();
        wired
    }

    /// Run one command the shell sends straight to the Logic thread, then
    /// what the view sends back. True when a command emitted a CRDT delta.
    fn send(&mut self, cmd: TrameCommand) -> bool {
        let out = self.state.handle_command(cmd);
        let wrote = out.update.is_some();
        for ev in out.display {
            self.rig.answer(ev);
        }
        self.pump() || wrote
    }

    /// Run what the view sent until it sends nothing more. True when a
    /// command the state ran emitted a CRDT delta.
    fn pump(&mut self) -> bool {
        let mut wrote = false;
        for _ in 0..8 {
            self.rig.frame();
            let sent = self.rig.sent();
            if sent.is_empty() {
                break;
            }
            for cmd in sent {
                let out = self.state.handle_command(cmd);
                wrote |= out.update.is_some();
                for ev in out.display {
                    self.rig.answer(ev);
                }
            }
        }
        wrote
    }
}

/// What the shell does with `id` when no dialog, file or clipboard stands
/// between: the arms of `chrome::handler::TrameHandler::on_command` that a
/// host test can run. `None` for an id whose run waits on one of those.
fn route(wired: &mut Wired, id: ambre_core::StaticKey<ambre_core::Commands>) -> Option<bool> {
    use crate::message::StylePatch;
    use crate::ui::commands::{BOLD, ITALIC, ZOOM_TO_SELECTION};
    if id == ZOOM_TO_SELECTION {
        wired.rig.view.zoom_to_selection();
        return Some(wired.pump());
    }
    let style = |patch: StylePatch| TrameCommand::SetCellStyle { patch };
    let cmd = if id == BOLD {
        style(StylePatch {
            bold: Some(ambre_core::Weight::Bold),
            ..StylePatch::default()
        })
    } else if id == ITALIC {
        style(StylePatch {
            italic: Some(ambre_core::Slant::Italic),
            ..StylePatch::default()
        })
    } else {
        crate::ui::commands::command_to_app(id)?
    };
    Some(wired.send(cmd))
}

/// `CHROME-149`, `CHROME-172`, `CHROME-173`, `CHROME-174`: every command of
/// the table runs the way the shell runs it, against a real Logic state. A
/// command whose home is the status bar emits no delta. A command that writes
/// leaves the zoom where it was, and one undo brings the whole workbook back.
/// Undo and redo are the history itself, so the undo check skips them. The
/// ids whose run waits on a dialog, a file or the clipboard are named, so a
/// new command joins one list or the other.
#[test]
fn each_command_keeps_the_rules_of_its_home() {
    use crate::ui::commands::*;
    use ambre_commands::Home;
    let elsewhere = [
        PALETTE,
        SAVE_SEALED_FILE,
        FIND,
        REPLACE,
        SORT_RANGE_MULTI,
        FILTER_COLUMN,
        COPY,
        CUT,
        PASTE,
        IMPORT_CSV,
        IMPORT_XLSX,
        IMPORT_ODS,
    ];
    let mut writers = Vec::new();
    for command in crate::ui::commands::COMMANDS {
        let mut wired = Wired::filled();
        let before = settled(wired.state.deep_value());
        let zoom = wired.rig.view.zoom();
        let Some(wrote) = route(&mut wired, command.id) else {
            assert!(
                elsewhere.contains(&command.id),
                "{:?} has no route and no named home",
                command.id
            );
            assert_ne!(command.home, Home::StatusBar, "{:?}", command.id);
            continue;
        };
        if command.home == Home::StatusBar {
            assert!(
                !wrote,
                "{:?} is homed in the status bar and wrote",
                command.id
            );
        }
        if command.id == ZOOM_TO_SELECTION {
            assert_ne!(wired.rig.view.zoom(), zoom, "the zoom route did not run");
        }
        if !wrote {
            continue;
        }
        writers.push(command.id);
        assert_eq!(
            wired.rig.view.zoom(),
            zoom,
            "{:?} wrote and moved the zoom",
            command.id
        );
        if command.id == UNDO || command.id == REDO {
            continue;
        }
        wired.send(TrameCommand::Undo);
        assert_eq!(
            settled(wired.state.deep_value()),
            before,
            "one undo does not take {:?} back whole",
            command.id
        );
    }
    for id in [
        BOLD,
        INSERT_ROW_ABOVE,
        DELETE_COLS,
        SORT_RANGE_ASC,
        FREEZE_ROWS,
        PAPER_A3,
    ] {
        assert!(writers.contains(&id), "{id:?} wrote nothing: {writers:?}");
    }
}

/// `CHROME-173`, `CHROME-174`: the clipboard entries write through a paste.
/// A paste, and a paste behind a cut, each leave the zoom where it was and
/// go back whole on one undo.
#[test]
fn a_paste_keeps_the_zoom_and_is_one_undo_item() {
    for write in [ClipboardWrite::Copy, ClipboardWrite::Cut] {
        let mut wired = Wired::filled();
        let before = settled(wired.state.deep_value());
        let zoom = wired.rig.view.zoom();
        let _ = wired.rig.view.clipboard_write(write);
        assert!(!wired.pump(), "{write:?} wrote before any paste");
        wired.rig.view.state.select_rect(CellRect::spanning(
            CellPos::new(ROW_5, COL_0),
            CellPos::new(ROW_5, COL_0),
        ));
        let text = ambre_bus::ClipboardBlob {
            text: "x".to_owned(),
            html: None,
            ambre: None,
            form: ambre_core::PasteForm::Rich,
        };
        wired.rig.view.paste_clipboard(text, PasteSource::Foreign);
        assert!(wired.pump(), "the paste behind {write:?} wrote nothing");
        assert_eq!(wired.rig.view.zoom(), zoom);
        wired.send(TrameCommand::Undo);
        assert_eq!(
            settled(wired.state.deep_value()),
            before,
            "one undo does not take the paste behind {write:?} back whole"
        );
    }
}

/// `CHROME-173`, `CHROME-174`: the entries that wait on a dialog or a file
/// write once it resolves, through the command the dialog sends. These are
/// the multi-key sort, the column filter and a replace of every match. Each
/// leaves the zoom where it was, and one undo takes it back whole. An import is one undo item
/// too (`logic::tests::group12::an_import_is_one_undo_item`). Its lines stay out
/// of the history by design, so it is not compared value for value here.
#[test]
fn a_resolved_dialog_keeps_the_zoom_and_is_one_undo_item() {
    use crate::message::{FilterKind, FilterOp, HeaderRow, SortOrder};
    let sheet = crate::logic::schema::default_sheet();
    let writes = [
        TrameCommand::SortRange {
            sheet_id: sheet.clone(),
            rect: CellRect::spanning(
                CellPos::new(ROW_0, COL_0),
                CellPos::new(Pos::<Row>::new(2), COL_2),
            ),
            sort_col: Len::<Col>::new(0),
            order: SortOrder::Ascending,
            header: HeaderRow::Absent,
        },
        TrameCommand::ApplyFilter {
            sheet_id: sheet,
            col: COL_0,
            kind: FilterKind::Condition {
                op: FilterOp::Gt,
                value: "4".to_owned(),
            },
        },
        TrameCommand::ReplaceAll {
            query: ambre_bus::app::find::FindText::new("9".to_owned()).map(|text| {
                ambre_bus::app::find::FindQuery::new(text, crate::FindOptions::default())
            }),
            replacement: "nine".to_owned(),
        },
    ];
    for write in writes {
        let name = format!("{write:?}");
        let mut wired = Wired::filled();
        let before = settled(wired.state.deep_value());
        let zoom = wired.rig.view.zoom();
        assert!(wired.send(write), "{name} wrote nothing");
        assert_eq!(wired.rig.view.zoom(), zoom, "{name} moved the zoom");
        wired.send(TrameCommand::Undo);
        assert_eq!(
            settled(wired.state.deep_value()),
            before,
            "one undo does not take {name} back whole"
        );
    }
}

/// `CHROME-174`: a CSV import, served with the converter linked in place as
/// the Logic thread serves it, writes and leaves the zoom where it was.
#[test]
fn an_import_keeps_the_zoom() {
    let mut wired = Wired::filled();
    let zoom = wired.rig.view.zoom();
    let converter = ambre_convert::Converter::load(
        ambre_core::ConverterFamily::Tables,
        &ambre_core::Payload::new(Vec::new()),
    )
    .expect("the host links the engine");
    let out = wired.state.convert_in_place(
        TrameCommand::import_csv(crate::message::CsvImport {
            data: b"1,2\n3,4\n".to_vec().into(),
            delimiter: None,
            header: crate::message::HeaderRow::Absent,
            target_sheet_id: None,
            start_row: Pos::<ambre_bus::Row>::ZERO,
            start_col: Pos::<ambre_bus::Col>::ZERO,
        }),
        &converter,
    );
    assert!(out.update.is_some(), "the import wrote nothing");
    for ev in out.display {
        wired.rig.answer(ev);
    }
    wired.pump();
    assert_eq!(wired.rig.view.zoom(), zoom, "the import moved the zoom");
}
