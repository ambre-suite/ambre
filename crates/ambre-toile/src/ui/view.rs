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

use ambre_bus::app::toile::{
    ClipboardPayload as ToileClipboardPayload, DisplayNode, DisplayViewport, ElementId,
    ToileCommand, ToileEvent, Viewport,
};
use ambre_bus::{AppCommand, AppEvent, DocumentId, LogicCommand, UiEvent};
use ambre_canvas::{
    ContentRepaint, FrameStats, InputEvent, OverlayRepaint, PerfPublisher, PerfSnapshot,
    RepaintPlan, TextCache, TextInput, Theme, ThemeControl, WgpuSurface,
};
use ambre_draw::{FontContext, Point, Rect, Vec2};

use super::hit::hit_topmost;
use super::render;
use super::tool::{Preview, Tool, ToolMachine, ToolOutcome, is_group, resolve_group_target};
use super::viewport::ViewportTransform;

/// One notch of the mouse wheel maps to this zoom factor (ctrl+wheel / pinch).
const ZOOM_PER_WHEEL_PX: f64 = 0.0018;

/// Keyboard-nudge step in world units: a fine 1-unit tap, or a coarse 10-unit
/// jump with Shift held (`toile-ui §5`).
const NUDGE_SMALL: f64 = 1.0;
const NUDGE_LARGE: f64 = 10.0;
/// Diagonal offset applied to a `Ctrl/Cmd+D` duplicate so the copy is visibly
/// distinct from its source (§8 ter), matching the alt-drag nudge.
const DUPLICATE_NUDGE: f64 = 10.0;

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
    /// Entered group scope (§2.F bis / §7): while `Some(g)`, a click selects a
    /// member inside `g` rather than the whole group. Entered by double-clicking a
    /// grouped element (peels one nesting level per double-click), left by
    /// `Escape` or a document swap. Pure-UI state, never synced.
    group_scope: Option<ElementId>,
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

    /// A [`ClipboardBlob`] built from a `ToileEvent::Clipboard` reply, awaiting the
    /// host's per-frame [`poll_clipboard`](ToileView::poll_clipboard) which mirrors
    /// it to the OS clipboard inside the transient-activation window (§5ter).
    pending_clipboard: Option<ambre_bus::ClipboardBlob>,
    /// The last authoritative clipboard blob copied this session, kept in memory
    /// for a full-fidelity same-tab paste that never depends on the OS custom MIME
    /// type surviving the round-trip (§5ter).
    last_clip: Option<ambre_bus::ClipboardBlob>,
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
            group_scope: None,
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
            pending_clipboard: None,
            last_clip: None,
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
            // Authoritative copy reply (§5ter): the Logic thread built a
            // self-contained, id-free snapshot of the selection from the whole
            // board (no viewport limit). Stash it in memory for a full-fidelity
            // same-tab paste and mark it pending so the host mirrors it to the OS
            // clipboard next frame. Paste into a foreign app is out of MVP, so the
            // `text/plain` mirror is empty (the private Ambre bytes carry it all).
            ToileEvent::Clipboard(payload) => {
                let blob = ambre_bus::ClipboardBlob {
                    text: String::new(),
                    ambre: postcard::to_allocvec(&payload).ok(),
                };
                self.last_clip = Some(blob.clone());
                self.pending_clipboard = Some(blob);
            }
            // Phase 0 does not act on these yet (chrome / presence land later);
            // log the errors, ignore the rest.
            ToileEvent::Error(e) => tracing::warn!("Toile rejected: {e}"),
            ToileEvent::ElementCreated(_)
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
        let out = self.machine.pointer_down(
            world,
            screen,
            &self.selection,
            &self.display,
            &self.cam,
            self.group_scope.as_ref(),
        );
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
        // Select tool: a double-click on a grouped element enters its scope,
        // peeling one nesting level so the next click selects a member (§2.F bis /
        // §7). Only when the tool produced nothing of its own (e.g. pen finish).
        if matches!(out, ToolOutcome::None)
            && self.machine.tool() == Tool::Select
            && let Some(hit) = hit_topmost(&self.display.nodes, world)
        {
            let target =
                resolve_group_target(hit.clone(), &self.display, self.group_scope.as_ref());
            if is_group(&target, &self.display) {
                self.group_scope = Some(target.clone());
                // Select the member one level inside the freshly-entered scope.
                self.selection = vec![resolve_group_target(hit, &self.display, Some(&target))];
            }
        }
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

    /// Keyboard shortcuts (`toile-ui §5`). Modifiers ride the event: `ctrl || meta`
    /// is the command chord (Ctrl on Windows/Linux, Cmd on macOS). Ctrl+C/X/V are
    /// intentionally NOT decoded here -- they ride the native clipboard events
    /// (Phase A). Returns `true` when the key was consumed (host `preventDefault`).
    fn handle_key(&mut self, key: &str, ctrl: bool, _alt: bool, shift: bool, meta: bool) -> bool {
        let cmd = ctrl || meta;
        // Command chords first, so a bare-key fallthrough never shadows them.
        if cmd {
            match key {
                // Undo / redo (§8 bis). Ctrl+Shift+Z and Ctrl+Y both redo.
                "z" | "Z" if shift => self.send_edit(ToileCommand::Redo),
                "z" | "Z" => self.send_edit(ToileCommand::Undo),
                "y" | "Y" => self.send_edit(ToileCommand::Redo),
                // Duplicate the selection with a small diagonal nudge (§8 ter).
                "d" | "D" if !self.selection.is_empty() => {
                    self.send_edit(ToileCommand::DuplicateElements {
                        ids: self.selection.clone(),
                        delta: Vec2::new(DUPLICATE_NUDGE, DUPLICATE_NUDGE),
                    });
                }
                // Group / ungroup the selection (§2.F bis).
                "g" | "G" if shift => {
                    for id in self.selection.clone() {
                        self.send_edit(ToileCommand::UngroupElements { group: id });
                    }
                }
                "g" | "G" if !self.selection.is_empty() => {
                    self.send_edit(ToileCommand::GroupElements {
                        ids: self.selection.clone(),
                    });
                }
                // Local select-all over every displayed element (connectors are
                // selected with their endpoints, not directly).
                "a" | "A" => {
                    self.selection = self
                        .display
                        .nodes
                        .iter()
                        .filter_map(|n| match n {
                            DisplayNode::Element(e) => Some(e.id.clone()),
                            DisplayNode::Connector(_) => None,
                        })
                        .collect();
                }
                _ => return false,
            }
            self.needs_full = true;
            return true;
        }
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
                // Leave any entered group scope (§2.F bis): the next click selects
                // whole groups again.
                self.group_scope = None;
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
            // Nudge the selection by 1 world unit (10 with Shift), §5.
            "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" if !self.selection.is_empty() => {
                let step = if shift { NUDGE_LARGE } else { NUDGE_SMALL };
                let delta = match key {
                    "ArrowUp" => Vec2::new(0.0, -step),
                    "ArrowDown" => Vec2::new(0.0, step),
                    "ArrowLeft" => Vec2::new(-step, 0.0),
                    _ => Vec2::new(step, 0.0),
                };
                self.send_edit(ToileCommand::MoveElements {
                    ids: self.selection.clone(),
                    delta,
                });
                self.needs_full = true;
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
            InputEvent::Key {
                key,
                ctrl,
                alt,
                shift,
                meta,
            } => self.handle_key(key, *ctrl, *alt, *shift, *meta),
            InputEvent::DoubleClick { x, y } => self.handle_double_click(*x, *y),
            InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => false,
        }
    }

    /// Resolve a copy/cut synchronously inside the native clipboard event (§5ter).
    /// The clipboard is authoritative: with a non-empty selection we emit
    /// [`ToileCommand::CopyElements`] (Logic builds the id-free payload from the
    /// whole board and replies with `ToileEvent::Clipboard`) and return
    /// [`CopyOutcome::Deferred`]; the payload is mirrored to the OS clipboard next
    /// frame by [`poll_clipboard`](Self::poll_clipboard). A cut appends
    /// [`ToileCommand::DeleteElements`] on the same selection immediately after --
    /// channel order guarantees Logic captured the payload before the delete.
    /// An empty selection returns [`CopyOutcome::Nothing`].
    pub fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        if self.selection.is_empty() {
            return ambre_bus::CopyOutcome::Nothing;
        }
        let ids = self.selection.clone();
        self.send_edit(ToileCommand::CopyElements { ids: ids.clone() });
        if cut {
            self.send_edit(ToileCommand::DeleteElements { ids });
            self.selection.clear();
            self.needs_full = true;
        }
        ambre_bus::CopyOutcome::Deferred
    }

    /// Drain the clipboard blob built from the last `ToileEvent::Clipboard` reply,
    /// so the host can mirror it to the OS clipboard within the transient user
    /// activation window (§5ter). Yields the blob at most once per copy.
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.pending_clipboard.take()
    }

    /// Service a paste from the shared [`ClipboardBlob`](ambre_bus::ClipboardBlob)
    /// transport (§5ter). When `own_copy` (the synchronous ownership token was
    /// still on the clipboard) it prefers the in-memory `last_clip` at full
    /// fidelity -- immune to the async OS write having failed on Firefox; else it
    /// decodes the blob's Ambre bytes. Either way, `PasteElements` lands the id-free
    /// payload at the cursor's world position. A blob with no decodable Ambre
    /// payload is ignored (plain-text paste into a board is out of MVP).
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        let source = match &self.last_clip {
            Some(clip) if own_copy => clip,
            _ => &blob,
        };
        let Some(payload) = source
            .ambre
            .as_ref()
            .and_then(|bytes| postcard::from_bytes::<ToileClipboardPayload>(bytes).ok())
        else {
            return;
        };
        let at = self.cam.screen_to_world() * self.last_pointer;
        self.send_edit(ToileCommand::PasteElements { payload, at });
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
        self.group_scope = None;
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
