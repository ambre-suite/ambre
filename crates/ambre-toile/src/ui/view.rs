// ambre-toile/src/ui/view.rs
// ============================================================================
// ToileView: the Toile app's `AppView` variant. Owns the local view state
// (camera, selection, active tool, last DisplayViewport) and the UI<->Logic
// channels, and exposes the inherent driver methods the `FrameHost` dispatches
// every frame (`ambre-app/src/host`): update / plan_repaint / paint_content /
// paint_overlay / resize / handle_input / clipboard / set_document.
//
// Mirrors `ambre_trame::ui::TrameView` but with a true world<->screen camera
// (`ViewportTransform`) instead of a virtual-scroll grid. Holds NO document data
// -- every mutation leaves as a `ToileCommand`; the board lives in `LoroDoc` on
// the Logic thread (§11bis).
// ============================================================================

use ambre_bus::app::toile::{DisplayViewport, ElementId, ToileCommand, ToileEvent, Viewport};
use ambre_bus::{AppCommand, AppEvent, DocumentId, LogicCommand, UiEvent};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, PerfPublisher, PerfSnapshot,
    RepaintPlan, TextCache, TextInput, Theme, ThemeControl, WgpuSurface,
};
use ambre_draw::{FontContext, Point, Rect, Vec2};

use super::render;
use super::tool::{Preview, Tool, ToolMachine, ToolOutcome};
use super::viewport::ViewportTransform;

/// One notch of the mouse wheel maps to this zoom factor (ctrl+wheel / pinch).
const ZOOM_PER_WHEEL_PX: f64 = 0.0018;

pub struct ToileView {
    document_id: DocumentId,
    to_logic: flume::Sender<LogicCommand>,
    from_logic: flume::Receiver<UiEvent>,

    /// The render-thread camera (world <-> screen), updated locally on pan/zoom.
    cam: ViewportTransform,
    /// The last culled frame received from Logic (what to paint).
    display: DisplayViewport,

    /// Active tool + the gesture state machine (Select + creation tools).
    machine: ToolMachine,
    /// Local selection (never persisted; a peer's selection is presence, §7).
    selection: Vec<ElementId>,
    /// Live gesture preview (marquee / rubber-band / pen / stroke), world space.
    preview: Option<Preview>,

    /// Last pointer position in screen (physical) pixels -- the zoom anchor.
    last_pointer: Point,
    /// While panning (space-drag / Hand tool): the previous screen position.
    panning: Option<Point>,
    /// Space held: a drag pans in any tool (§5).
    space_down: bool,

    text_input: TextInput,
    perf_pub: PerfPublisher,
    theme_ctl: ThemeControl,
    theme: Theme,

    // Dirty-flag bookkeeping (§9).
    needs_full: bool,
    viewport_changed: bool,
    data_dirty: bool,
    /// Coalesced viewport request: at most one `SetViewport` per frame.
    pending_viewport: Option<Viewport>,
    initial_viewport_sent: bool,
    fatal: bool,
}

impl ToileView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_id: DocumentId,
        to_logic: flume::Sender<LogicCommand>,
        from_logic: flume::Receiver<UiEvent>,
        canvas_width: f64,
        canvas_height: f64,
        text_input: TextInput,
        perf_pub: PerfPublisher,
        theme_ctl: ThemeControl,
    ) -> Self {
        text_input.focus();
        let theme = theme_ctl.get();
        Self {
            document_id,
            to_logic,
            from_logic,
            cam: ViewportTransform::new(canvas_width, canvas_height),
            display: DisplayViewport {
                viewport: Viewport::default(),
                nodes: Vec::new(),
            },
            machine: ToolMachine::new(),
            selection: Vec::new(),
            preview: None,
            last_pointer: Point::new(canvas_width / 2.0, canvas_height / 2.0),
            panning: None,
            space_down: false,
            text_input,
            perf_pub,
            theme_ctl,
            theme,
            needs_full: true,
            viewport_changed: false,
            data_dirty: false,
            pending_viewport: None,
            initial_viewport_sent: false,
            fatal: false,
        }
    }

    // -- Command emission ------------------------------------------------------

    /// Send a lossy command (viewport requests): dropped when the channel is full
    /// (the next coalesced request supersedes it). Never blocks the UI thread.
    fn send_lossy(&self, cmd: ToileCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        if let Err(flume::TrySendError::Disconnected(_)) = self.to_logic.try_send(msg) {
            tracing::error!("Toile: UI->Logic channel disconnected");
        }
    }

    /// Send a mutating command (edits). Warns (does not panic) on backpressure.
    fn send_edit(&self, cmd: ToileCommand) {
        if self.fatal {
            return;
        }
        let msg = LogicCommand::App {
            document_id: self.document_id,
            command: AppCommand::Toile(cmd),
        };
        match self.to_logic.try_send(msg) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(_)) => {
                tracing::warn!("Toile: UI->Logic channel full, edit dropped");
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                tracing::error!("Toile: UI->Logic channel disconnected");
            }
        }
    }

    /// Queue a coalesced `SetViewport` for the current camera (flushed in `update`).
    fn queue_viewport(&mut self) {
        self.pending_viewport = Some(self.cam.to_bus_viewport());
        self.viewport_changed = true;
    }

    fn signal_fatal(&mut self, message: &str) {
        if !self.fatal {
            self.fatal = true;
            ambre_canvas::signal_fatal(message);
        }
    }

    // -- Event intake ----------------------------------------------------------

    /// Canvas-side reaction to one `ToileEvent`. Exhaustive: a new variant must
    /// fail the build here rather than be dropped silently.
    fn dispatch(&mut self, ev: ToileEvent) {
        match ev {
            ToileEvent::ViewportData(dv) => {
                self.display = dv;
                self.data_dirty = true;
            }
            // Phase 0 does not act on these yet (chrome / presence / clipboard
            // land in Phase 1); log the errors, ignore the rest.
            ToileEvent::Error(e) => tracing::warn!("Toile rejected: {e}"),
            ToileEvent::ElementCreated(_)
            | ToileEvent::Clipboard(_)
            | ToileEvent::SelectionStats(_)
            | ToileEvent::FindResults(_)
            | ToileEvent::DanglingConnector(_)
            | ToileEvent::PeerPresence(_)
            | ToileEvent::UndoState { .. } => {}
        }
    }

    // -- Input handling --------------------------------------------------------

    fn handle_wheel(&mut self, dx: f64, dy: f64, ctrl_key: bool) -> bool {
        if ctrl_key {
            // Pinch / ctrl-wheel zoom about the cursor.
            let factor = (-dy * ZOOM_PER_WHEEL_PX).exp();
            self.cam.zoom_about(self.last_pointer, factor);
        } else {
            // Two-finger / wheel scroll pans.
            self.cam.pan_by_screen(Vec2::new(dx, dy));
        }
        self.queue_viewport();
        true
    }

    fn apply_outcome(&mut self, out: ToolOutcome) {
        match out {
            ToolOutcome::None => {}
            ToolOutcome::SelectionChanged(ids) => {
                self.selection = ids;
                self.needs_full = true;
            }
            ToolOutcome::Command(cmd) => self.send_edit(cmd),
        }
        // The overlay preview is derived from live machine state each event.
        self.preview = self.machine.preview();
    }

    fn handle_pointer_down(&mut self, x: f64, y: f64, button: i16) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        // Space-drag or Hand tool or middle button: begin a pan.
        if self.space_down || self.machine.tool() == Tool::Hand || button == 1 {
            self.panning = Some(screen);
            return true;
        }
        let world = self.cam.screen_to_world() * screen;
        let out =
            self.machine
                .pointer_down(world, screen, &self.selection, &self.display, &self.cam);
        self.apply_outcome(out);
        self.needs_full = true;
        true
    }

    fn handle_pointer_move(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        if let Some(prev) = self.panning {
            self.cam.pan_by_screen(screen - prev);
            self.panning = Some(screen);
            self.queue_viewport();
            return true;
        }
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_move(world);
            self.apply_outcome(out);
            self.needs_full = true;
            return true;
        }
        false
    }

    fn handle_pointer_up(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        if self.panning.take().is_some() {
            return true;
        }
        if self.machine.is_active() {
            let world = self.cam.screen_to_world() * screen;
            let out = self.machine.pointer_up(world, &self.display);
            self.apply_outcome(out);
            self.needs_full = true;
            return true;
        }
        false
    }

    fn handle_double_click(&mut self, x: f64, y: f64) -> bool {
        let screen = Point::new(x, y);
        self.last_pointer = screen;
        let world = self.cam.screen_to_world() * screen;
        let out = self.machine.double_click(world);
        self.apply_outcome(out);
        self.needs_full = true;
        true
    }

    /// Switch the active tool (driven by the chrome tool palette, §1bis).
    pub fn set_tool(&mut self, tool: Tool) {
        self.machine.set_tool(tool);
        self.preview = None;
        self.needs_full = true;
    }

    fn handle_key(&mut self, key: &str) -> bool {
        match key {
            " " | "Spacebar" => {
                self.space_down = true;
                true
            }
            "Escape" => {
                self.machine.cancel();
                self.preview = None;
                if !self.selection.is_empty() {
                    self.selection.clear();
                }
                self.needs_full = true;
                true
            }
            "Delete" | "Backspace" => {
                if !self.selection.is_empty() {
                    let ids = std::mem::take(&mut self.selection);
                    self.send_edit(ToileCommand::DeleteElements { ids });
                    self.needs_full = true;
                }
                true
            }
            _ => false,
        }
    }
}

// -----------------------------------------------------------------------------
// Inherent driver methods dispatched by `AppView` in `ambre-app`.
// -----------------------------------------------------------------------------

impl ToileView {
    pub fn update(&mut self) {
        loop {
            match self.from_logic.try_recv() {
                Ok(UiEvent::Viewport {
                    event: AppEvent::Toile(te),
                    ..
                }) => self.dispatch(te),
                Ok(UiEvent::FatalError { message }) => self.signal_fatal(&message),
                // Other UiEvent variants (session, CRUD, connection) are not yet
                // consumed by the Toile view; ignore them for now.
                Ok(_) => {}
                Err(flume::TryRecvError::Empty) => break,
                Err(flume::TryRecvError::Disconnected) => {
                    self.signal_fatal("Logic thread disconnected");
                    break;
                }
            }
        }

        // First frame: request the initial culled viewport for the boot camera.
        if !self.initial_viewport_sent {
            self.initial_viewport_sent = true;
            self.needs_full = true;
            self.pending_viewport = Some(self.cam.to_bus_viewport());
        }

        // Flush the coalesced viewport request (at most one per frame).
        if let Some(vp) = self.pending_viewport.take() {
            self.send_lossy(ToileCommand::SetViewport(vp));
        }

        // Pick up a theme switch from the chrome toggle.
        let theme = self.theme_ctl.get();
        if theme != self.theme {
            self.theme = theme;
            self.needs_full = true;
        }
    }

    pub fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan {
        let any_dirty = self.needs_full
            || self.viewport_changed
            || self.data_dirty
            || self.machine.is_active()
            || hud_tick;

        // Phase 0 has no blit-scroll optimization: any content change is a full
        // repaint (the board is transformed on the GPU for pan/zoom, so this is
        // cheap for the common idle/gesture case where nothing repaints at all).
        let content = if self.needs_full || self.data_dirty || self.viewport_changed {
            ContentRepaint::Full
        } else {
            ContentRepaint::Reuse
        };
        let overlay = if any_dirty {
            OverlayRepaint::Redraw
        } else {
            OverlayRepaint::Reuse
        };

        self.needs_full = false;
        self.viewport_changed = false;
        self.data_dirty = false;

        RepaintPlan {
            content,
            content_strip: None,
            overlay,
        }
    }

    pub fn paint_content(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        _strip: Option<Rect>,
    ) {
        render::paint_content(
            surface,
            &self.display,
            &self.cam,
            self.theme.palette(),
            font_cx,
            text_cache,
        );
    }

    pub fn paint_overlay(
        &mut self,
        surface: &mut WgpuSurface<'_>,
        _font_cx: &mut FontContext,
        _text_cache: &mut TextCache,
        stats: &FrameStats,
    ) {
        render::paint_overlay(
            surface,
            &self.selection,
            &self.display,
            &self.cam,
            self.theme.palette(),
            self.preview.as_ref(),
        );

        self.perf_pub.publish(PerfSnapshot {
            fps: stats.fps,
            frame_ms: stats.frame_ms,
            drain_ms: stats.drain_ms,
            draw_ms: stats.draw_ms,
            gpu_ms: stats.gpu_ms,
            ..Default::default()
        });
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.cam.set_size(width as f64, height as f64);
        self.needs_full = true;
        self.queue_viewport();
    }

    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::Wheel {
                dx, dy, ctrl_key, ..
            } => self.handle_wheel(*dx, *dy, *ctrl_key),
            InputEvent::PointerDown { x, y, button, .. } => {
                self.handle_pointer_down(*x, *y, *button)
            }
            InputEvent::PointerMove { x, y } => self.handle_pointer_move(*x, *y),
            InputEvent::PointerUp { x, y, .. } => self.handle_pointer_up(*x, *y),
            InputEvent::Key { key, .. } => self.handle_key(key),
            InputEvent::DoubleClick { x, y } => self.handle_double_click(*x, *y),
            InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => false,
        }
    }

    /// Clipboard copy/cut. Phase 0 has no in-canvas clipboard round-trip yet
    /// (§5ter is Phase 1); let the browser default run.
    /// The clipboard payload type is the shared `AppView` contract type (the
    /// same concrete type Trame uses); Toile does not produce one in Phase 0.
    pub fn clipboard_write(
        &mut self,
        _cut: bool,
    ) -> Option<(String, Option<ambre_bus::ClipboardPayload>)> {
        None
    }

    /// Clipboard paste. Phase 0 no-op (Phase 1 wires `PasteElements`).
    pub fn paste_clipboard(
        &mut self,
        _payload: Option<ambre_bus::ClipboardPayload>,
        _text: String,
    ) {
    }

    /// Swap the active document in place (SPA route change): reset document-scoped
    /// state, keep the plumbing and canvas size, and re-fetch the viewport.
    pub fn set_document(&mut self, document_id: DocumentId) {
        if self.document_id == document_id {
            return;
        }
        self.document_id = document_id;
        self.display = DisplayViewport {
            viewport: Viewport::default(),
            nodes: Vec::new(),
        };
        self.selection.clear();
        self.preview = None;
        self.machine.cancel();
        self.initial_viewport_sent = false;
        self.pending_viewport = None;
        self.needs_full = true;
        // Keep the transparent text pipe focused so the keyboard works on the
        // freshly-routed board without a click.
        self.text_input.focus();
    }
}
