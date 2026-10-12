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
    /// Held so the view's inbound channel stays open.
    _from_logic: ambre_bus::conduit::Sender<UiEvent>,
    clock: TestClock,
}

impl Rig {
    fn new() -> Self {
        let (tx, to_logic) = ambre_bus::conduit::local(256);
        let (from_logic, rx) = ambre_bus::conduit::local(256);
        let clock = TestClock::default();
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
            ThemeControl::default(),
            Box::new(clock.clone()),
        );
        Self {
            view,
            to_logic,
            _from_logic: from_logic,
            clock,
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
