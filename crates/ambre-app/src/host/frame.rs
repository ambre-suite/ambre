// ambre-app/src/host/frame.rs
// ============================================================================
// FrameHost: the per-frame render driver. Owns the active `AppView`, the GPU
// state (installed after `bootstrap_gpu`), the fps ring buffer, and the fatal
// screen. Drives one document view per frame; the document is swapped in place
// by `DocSwitcher` (see `run`).
// ============================================================================

use super::*;

/// The container the shell mounts over the canvas for a board's web embeds
/// (`TOILE-088`). `None` before the shell renders, and for a host page that does
/// not carry one.
fn web_embed_layer() -> Option<web_sys::HtmlElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id("ambre-web-embeds")?
        .dyn_into()
        .ok()
}

/// Builds a child `AppView` for a nested `doc_embed`. Supplied by the
/// launcher, which alone can name the concrete app views and holds the leptos
/// owner their (throwaway) chrome signals need. Given the embedded document's id,
/// type and child canvas size, it wires a demultiplexed Logic->UI queue and
/// returns the view. `FrameHost` stays app-agnostic: it only calls this.
pub type MakeChild = Rc<dyn Fn(DocumentId, DocType, Size) -> AppView>;

/// Rewrite an input event for the focused child ("Input focus"). Pointer
/// coordinates are mapped from the canvas into the child's pixel space through
/// the camera and the embed's transform. Every other event forwards unchanged: a
/// wheel delta and a keystroke are frame-agnostic, since a child pixel is a
/// screen pixel at the same zoom.
///
/// `None` when a positional event cannot be mapped -- the focused embed left the
/// culled frame between the event and this call. The event is then dropped rather
/// than handed over with a coordinate from the board's own space
/// (`TOILE-UI-328`). That would land a click at whatever the raw screen point
/// happens to address inside the child.
fn remap_for_child(view: &AppView, ev: &InputEvent) -> Option<InputEvent> {
    remap_with(ev, |p| view.embed_to_child_space(p))
}

/// Ship font faces to the Logic thread, which measures on a context of its own
/// and cannot fetch. The base faces at boot leave through here, and so does
/// each script face fetched later: a late face travels the base path
/// (`CANVAS-068`). A full channel drops the faces rather than stall the frame;
/// the next fetch is a fresh delivery. Whether they were sent.
pub(crate) fn send_faces(
    to_logic: &ambre_bus::conduit::Sender<LogicCommand>,
    faces: Vec<Vec<u8>>,
) -> bool {
    let faces = faces.into_iter().map(ambre_core::FontBytes::new).collect();
    to_logic.try_send(LogicCommand::LoadFonts { faces }).is_ok()
}

/// [`remap_for_child`] with the board's mapping supplied, so the rewrite is
/// tested without a live view.
fn remap_with(ev: &InputEvent, to_child: impl Fn(Point) -> Option<Point>) -> Option<InputEvent> {
    let map = |x: f64, y: f64| to_child(Point::new(x, y)).map(|p| (p.x, p.y));
    Some(match ev {
        InputEvent::PointerDown {
            x,
            y,
            button,
            shift,
            ctrl,
        } => {
            let (x, y) = map(*x, *y)?;
            InputEvent::PointerDown {
                x,
                y,
                button: *button,
                shift: *shift,
                ctrl: *ctrl,
            }
        }
        InputEvent::PointerMove { x, y } => {
            let (x, y) = map(*x, *y)?;
            InputEvent::PointerMove { x, y }
        }
        InputEvent::PointerUp { x, y, button, ctrl } => {
            let (x, y) = map(*x, *y)?;
            InputEvent::PointerUp {
                x,
                y,
                button: *button,
                ctrl: *ctrl,
            }
        }
        InputEvent::DoubleClick { x, y } => {
            let (x, y) = map(*x, *y)?;
            InputEvent::DoubleClick { x, y }
        }
        // Non-positional events carry no coordinates to rewrite.
        other => other.clone(),
    })
}

/// Reconcile the child views against the visible `doc_embed`s, then pump each
/// one. A child is made when its embed enters the set, resized when its box
/// changes, and dropped when it leaves. Children are keyed by the board
/// element, so two embeds of one document are two children (`TOILE-UI-097`).
/// `pump` answers whether the child changed on its own. Returns whether the
/// board must recomposite: a new child, a resize, or a pumped change.
fn reconcile_children<C>(
    children: &mut HashMap<ElementId, C>,
    sizes: &mut HashMap<ElementId, (u32, u32)>,
    embeds: &[VisibleEmbed],
    mut make: impl FnMut(&VisibleEmbed) -> C,
    mut resize: impl FnMut(&mut C, u32, u32),
    mut pump: impl FnMut(&mut C) -> bool,
) -> bool {
    let live: HashSet<ElementId> = embeds.iter().map(|e| e.element.clone()).collect();
    children.retain(|id, _| live.contains(id));
    sizes.retain(|id, _| live.contains(id));

    let mut dirty = false;
    for e in embeds {
        let w = e.child_size.width.round().max(0.0) as u32;
        let h = e.child_size.height.round().max(0.0) as u32;
        if w == 0 || h == 0 {
            continue;
        }
        if sizes.get(&e.element) == Some(&(w, h)) && children.contains_key(&e.element) {
            continue;
        }
        let child = children.entry(e.element.clone()).or_insert_with(|| make(e));
        resize(child, w, h);
        sizes.insert(e.element.clone(), (w, h));
        dirty = true;
    }
    // Every child is pumped, so none skips a frame of its own events.
    for child in children.values_mut() {
        dirty |= pump(child);
    }
    dirty
}

/// Paint each visible child into its embed: clip to the embed's box, push the
/// child-to-screen transform, paint, pop both. The content pass and the
/// overlay pass both call it. So a child's two halves land in one place
/// (`TOILE-UI-320`), and a rotated embed paints its child rotated
/// (`TOILE-UI-096`). A box under one pixel paints nothing.
fn paint_children<S: DrawSurface + ?Sized, C>(
    surface: &mut S,
    embeds: &[VisibleEmbed],
    children: &mut HashMap<ElementId, C>,
    mut paint: impl FnMut(&mut C, &mut S),
) {
    for e in embeds {
        let Some(child) = children.get_mut(&e.element) else {
            continue;
        };
        let r = e.screen_rect;
        if r.width() < 1.0 || r.height() < 1.0 {
            continue;
        }
        surface.push_clip(r);
        surface.push_transform(e.to_screen);
        paint(child, surface);
        surface.pop_transform();
        surface.pop_clip();
    }
}

/// Drives one [`AppView`] across the GPU and the requestAnimationFrame loop.
///
/// App-agnostic at this level: it owns the [`GpuState`], measures frame timings,
/// decides which layers to encode from the view's [`RepaintPlan`], and presents
/// the swapchain. Everything document-specific lives behind the `AppView` match.
pub struct FrameHost {
    gpu: Option<GpuState>,
    view: AppView,
    perf: web_sys::Performance,
    /// The per-page-load clipboard ownership token (`UI-THREAD-007`). Written on
    /// `copy`/`cut`, re-published by the async write (`UI-THREAD-023`), and read
    /// back to answer rung 2 of the Paste-button path.
    clip_token: ambre_bus::ClipToken,
    /// Whether the asynchronous write of the last copy reached the clipboard.
    /// It replaces the synchronous formats, token included, with `web `
    /// variants a `paste` event cannot see (`UI-THREAD-054`).
    clip_published: Rc<std::cell::Cell<bool>>,
    /// Clipboard reads the Paste button hands over, already decoded by the shell.
    /// Drained once per frame and routed to the active view (`UI-THREAD-024`).
    paste_rx: ambre_bus::conduit::Receiver<ClipboardPaste>,

    /// The command channel to the Logic thread, held for one purpose. A font
    /// face fetched after boot has to reach the threads that measure on a
    /// context of their own (`CANVAS-068`). Only this thread can fetch, and
    /// Prose paginates on the Logic thread.
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,

    /// The live child views behind the top-level document's visible `doc_embed`s,
    /// keyed by the **board element** that carries each reference. Reconciled
    /// every frame against [`AppView::visible_embeds`]: a child appears when its
    /// embed enters the cull set and is dropped when it leaves. Empty unless the
    /// top-level view embeds.
    ///
    /// By element, not by document (`TOILE-UI-097`). The session is one per
    /// document, which is the board's own registry and stays that way. But the
    /// paint is one per embed. Keyed by document, two boxes of different sizes
    /// resized one view against each other every frame. Only one of them was ever
    /// the right size for it.
    children: HashMap<ElementId, AppView>,
    /// The last logical canvas size each child was resized to. A moved or zoomed
    /// embed only triggers a child `resize` when its box actually changes.
    child_sizes: HashMap<ElementId, (u32, u32)>,
    /// The launcher's child-view factory (see [`MakeChild`]). `None` for a host
    /// that never embeds (or before the launcher installs it).
    make_child: Option<MakeChild>,

    /// The sandboxed frames behind the board's web embeds (`TOILE-088`).
    web_frames: web_embed::WebFrames,
    /// Whether the last reconciliation left the layer empty, so a board with no
    /// web embed costs no DOM lookup per frame.
    web_frames_empty: bool,

    /// The canvases the view shows beside this one, such as a Murmure thread,
    /// each with a GPU of its own (`CANVAS-070`). Empty for a view of one
    /// canvas.
    cells: super::cell::Cells,

    render_times: VecDeque<ambre_core::PageTime>,
    render_fps: f32,
    last_frame_ms: f32,
    last_drain_ms: f32,
    last_draw_ms: f32,
    last_gpu_ms: f32,
    /// rAF timestamp of the last HUD rebuild. The overlay refreshes at most
    /// every 500 ms so idle frames never trigger a full vertex rebuild.
    last_hud_update_ms: ambre_core::PageTime,
    /// Set once the fatal terminal screen has been painted, so the static
    /// message is encoded a single time rather than every subsequent frame.
    fatal_painted: bool,
}

impl FrameHost {
    /// Build a host around `view`. The GPU is installed later via
    /// [`install_gpu`] once async bootstrap completes.
    ///
    /// [`install_gpu`]: FrameHost::install_gpu
    pub fn new(
        view: AppView,
        perf: web_sys::Performance,
        make_child: Option<MakeChild>,
        clip_token: ambre_bus::ClipToken,
        paste_rx: ambre_bus::conduit::Receiver<ClipboardPaste>,
        to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    ) -> Self {
        Self {
            gpu: None,
            view,
            perf,
            clip_token,
            clip_published: Rc::new(std::cell::Cell::new(false)),
            paste_rx,
            to_logic,
            children: HashMap::new(),
            child_sizes: HashMap::new(),
            make_child,
            web_frames: web_embed::WebFrames::default(),
            web_frames_empty: true,
            cells: super::cell::Cells::default(),
            render_times: VecDeque::with_capacity(120),
            render_fps: 0.0,
            last_frame_ms: 0.0,
            last_drain_ms: 0.0,
            last_draw_ms: 0.0,
            last_gpu_ms: 0.0,
            last_hud_update_ms: ambre_bus::PageTime(0.0),
            fatal_painted: false,
        }
    }

    /// Hand the host its GPU resources once `bootstrap_gpu` has resolved.
    pub fn install_gpu(&mut self, gpu: GpuState) {
        self.gpu = Some(gpu);
    }

    /// The per-page-load clipboard ownership token (`UI-THREAD-007`). The
    /// clipboard event listeners in `run` read it back to decide a same-tab copy.
    pub(crate) fn clip_token(&self) -> &ambre_bus::ClipToken {
        &self.clip_token
    }

    /// Whether the last copy's asynchronous write reached the clipboard
    /// (`UI-THREAD-054`). A new copy clears it; the write sets it on success.
    pub(crate) fn clip_published(&self) -> Rc<std::cell::Cell<bool>> {
        Rc::clone(&self.clip_published)
    }

    /// Forward an input event to the view. Returns `true` if the caller should
    /// `preventDefault()` on the originating DOM event.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        // A cell beside the content canvas that holds the text pipe takes
        // what the pipe hears (`CANVAS-076`).
        if let Some(consumed) = self.cells.route(ev) {
            return consumed;
        }
        // The top-level view runs first: it settles embed focus and decides
        // whether this event belongs to a focused child ("Input focus").
        let consumed = self.view.handle_input(ev);
        // If it does, forward the event into that child's own view, with pointer
        // coordinates remapped into the child's pixel space. A wheel or keystroke
        // forwards verbatim: a child pixel is a screen pixel at the same zoom.
        if let Some(child_id) = self.view.child_route()
            && let Some(child) = self.children.get_mut(&child_id)
        {
            // A positional event the board cannot map is dropped, not handed
            // over unmapped (`TOILE-UI-328`).
            if let Some(remapped) = remap_for_child(&self.view, ev) {
                child.handle_input(&remapped);
            }
        }
        consumed
    }

    /// Reconcile the child views against the top-level view's visible `doc_embed`s
    /// and pump each one. Creates a child when its embed enters the cull set,
    /// resizes it when its box changes, and drops it when it leaves. Dropping the
    /// child also drops its demultiplexed Logic->UI queue, so the relay reclaims
    /// the route on its next send. Returns whether any child needs the board
    /// recomposited this frame (a fresh child, a resize, or a self-driven content
    /// change). A no-op when the launcher installed no child factory.
    fn sync_children(&mut self, now_ms: ambre_core::PageTime) -> bool {
        let Some(make_child) = self.make_child.clone() else {
            return false;
        };
        let embeds = self.view.visible_embeds();
        reconcile_children(
            &mut self.children,
            &mut self.child_sizes,
            &embeds,
            |e| make_child(e.document_id, e.doc_type, e.child_size),
            |child, w, h| child.resize(w, h),
            // Drain the child's Logic->UI events and run its per-frame
            // planning. A non-`Reuse` content plan means it changed on its own
            // (a remote edit).
            |child| {
                child.update();
                !matches!(
                    child.plan_repaint(now_ms, false).content,
                    ContentRepaint::Reuse
                )
            },
        )
    }

    /// Reconcile the sandboxed `<iframe>`s behind the board's web embeds against
    /// the culled set (`TOILE-087`, `TOILE-088`). A no-op when the shell mounted
    /// no layer, and for every app that embeds no page.
    fn sync_web_embeds(&mut self) {
        let embeds = self.view.visible_web_embeds();
        // Never touch the DOM for a board that has no web embed and had none:
        // the overwhelming majority of frames.
        if embeds.is_empty() && self.web_frames_empty {
            return;
        }
        let Some(layer) = web_embed_layer() else {
            return;
        };
        self.web_frames.sync(&layer, &embeds);
        self.web_frames_empty = embeds.is_empty();
    }

    /// Resolve a `copy`/`cut` synchronously inside the native clipboard event.
    /// See [`AppView::clipboard_write`]: `Text` writes immediately, `Deferred`
    /// waits for the next frame to drain the payload, `Nothing` lets the
    /// browser default proceed.
    pub fn clipboard_write(&mut self, write: ClipboardWrite) -> ambre_bus::CopyOutcome {
        match self.cells.typist() {
            Some(cell) => cell.borrow_mut().clipboard_write(write),
            None => self.view.clipboard_write(write),
        }
    }

    /// Service a `paste` event with the shared clipboard transport blob.
    /// `source` says whether the clipboard still holds this tab's ownership token.
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, source: PasteSource) {
        match self.cells.typist() {
            Some(cell) => cell.borrow_mut().paste_clipboard(blob, source),
            None => self.view.paste_clipboard(blob, source),
        }
    }

    /// Text dropped on the canvas at a point, in canvas pixels. Only Prose
    /// takes it (`PROSE-UI-204`); answers whether it did.
    pub fn drop_text(&mut self, x: f32, y: f32, text: String) -> bool {
        self.view.drop_text(x, y, text)
    }

    /// A finger on the canvas, in canvas pixels. Only Prose takes a touch
    /// pointer apart (`PROSE-UI-217`); answers whether it did.
    pub fn touch(
        &mut self,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now_ms: ambre_core::PageTime,
    ) -> bool {
        self.view.touch(phase, x, y, now_ms)
    }

    /// Whether a consumed press captures the pointer, so a drag keeps its
    /// moves past the canvas edge. Only Prose asks for it (`PROSE-UI-038`).
    pub fn captures_pointer(&self) -> bool {
        self.view.captures_pointer()
    }

    /// Whether a press began a drag across read-only text, which the host
    /// captures although the view did not consume the press.
    pub fn drags_text(&self) -> bool {
        self.view.drags_text()
    }

    /// Switch the active document on the live view, keeping the GPU surface.
    pub fn set_document(&mut self, document_id: DocumentId) {
        self.view.set_document(document_id);
    }

    /// Reconfigure the GPU surface and notify the view of a canvas resize.
    /// The view hears the logical size alone (`CANVAS-044`). A density change
    /// keeps it, and still reaches the view so every layer repaints.
    pub fn resize(&mut self, size: SurfaceSize) {
        let (width, height) = size.logical();
        self.view.resize(width, height);
        // The density watch lands here, and moves no cell box either.
        self.cells.remeasure();

        let Some(gpu) = self.gpu.as_mut() else { return };
        if gpu.size == size {
            return;
        }
        tracing::info!("GPU resize: {width}x{height}");
        // Reallocate the ping-pong offscreens at the new size; previous content
        // is lost so the view's resize() must have flagged a full repaint.
        gpu.configure(size);
    }

    /// Recover after the WebGL/WebGPU context was lost and restored. The GL
    /// drawable and every GPU-side allocation were dropped. So we reconfigure
    /// the surface and rebuild the ping-pong offscreens at the current size,
    /// then force a full repaint through the view. A plain `resize` to the same
    /// dimensions early-returns and would leave a blank canvas.
    pub fn notify_context_restored(&mut self, size: SurfaceSize) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let (width, height) = size.logical();
        tracing::info!("GPU context restored, reconfiguring {width}x{height}");
        gpu.configure(size);
        // Force the view to invalidate every cached layer for a clean repaint.
        self.view.resize(width, height);
    }

    pub(crate) fn tick_fps(
        times: &mut VecDeque<ambre_core::PageTime>,
        fps: &mut f32,
        now: ambre_core::PageTime,
    ) {
        times.push_back(now);
        while times.front().is_some_and(|&t| now - t > 1000.0) {
            times.pop_front();
        }
        *fps = match times.front() {
            Some(&oldest) if times.len() >= 2 => {
                (times.len() - 1) as f32 / ((now - oldest) / 1000.0) as f32
            }
            _ => 0.0,
        };
    }

    /// Render one frame. Wired into [`start_raf_loop`] by [`run_with`].
    pub fn render(&mut self, raf_timestamp: ambre_core::PageTime) {
        // Fatal latch first: once set, the Logic thread feeding the view may be
        // aborted, so we must not keep draining it. Paint the terminal screen a
        // single time and then idle (the rAF loop keeps ticking harmlessly).
        let fatal = fatal_message();
        match Tick::of(fatal.is_some(), self.fatal_painted) {
            Tick::Render => {}
            Tick::PaintFatal => {
                self.render_fatal(fatal.as_deref().unwrap_or_default());
                self.fatal_painted = true;
                return;
            }
            Tick::Idle => return,
        }

        // The canvases beside this one, each under its own plan (`CANVAS-070`).
        // A view of one canvas names none, and costs no lookup.
        self.cells.sync(&self.view);
        self.cells.settle(&mut self.view);
        self.cells.render(raf_timestamp);

        let t_drain = self.perf.now();
        self.view.update();
        let t_after_drain = self.perf.now();

        // Service any Paste-button read the shell decoded. The shell read the
        // clipboard inside the click gesture. The host answers rung 2 by
        // comparing the token, then hands the blob to the active view
        // (`UI-THREAD-024`, `UI-THREAD-025`).
        while let Ok(paste) = self.paste_rx.try_recv() {
            super::pipe::button_paste(self, paste);
        }

        // A `Deferred` copy earlier this frame, or the previous one, had its
        // authoritative payload built on the Logic thread. `update()` above
        // drained it into the view. Mirror it to the OS clipboard now: this runs
        // ~one frame (~16 ms) after the keypress, still inside the transient
        // user-activation window `navigator.clipboard.write` requires. `text` is
        // the cross-app `text/plain` mirror; the private `web ` type carries the
        // opaque Ambre bytes for a full-fidelity internal round-trip.
        if let Some(blob) = self.view.poll_clipboard() {
            let payload_b64 = blob.ambre.as_deref().map(base64_encode);
            async_write_clipboard(
                blob.text,
                blob.html,
                payload_b64,
                self.clip_token.clone(),
                Rc::clone(&self.clip_published),
            );
        }

        // Deliver any lazily-fetched font faces (CJK/Arabic/Devanagari) that
        // arrived since the last frame. If coverage changed, the cached GPU
        // layers still hold tofu pixels for the affected strings. Force a
        // full view repaint to re-shape and redraw them. Scoped so the short
        // GPU borrow ends before we touch `self.view`.
        let new_faces = {
            let Some(gpu) = self.gpu.as_mut() else { return };
            gpu.text_cache.poll_lazy_fonts(&mut gpu.font_cx)
        };
        if !new_faces.is_empty() {
            // The same face, to the Logic thread. It measures Prose pages on a
            // sealed context and cannot fetch, so a script it never receives
            // paginates into tofu (`CANVAS-068`, `PROSE-115`).
            send_faces(&self.to_logic, new_faces);
            if let Some((w, h)) = self.gpu.as_ref().map(|g| g.size.logical()) {
                self.view.resize(w, h);
            }
        }

        Self::tick_fps(&mut self.render_times, &mut self.render_fps, raf_timestamp);

        let mut hud_tick = false;
        if raf_timestamp - self.last_hud_update_ms >= 500.0 {
            self.last_hud_update_ms = raf_timestamp;
            hud_tick = true;
        }

        let mut plan = self.view.plan_repaint(raf_timestamp, hud_tick);
        // Reconcile + pump the nested child views AFTER the parent's plan_repaint
        // has issued `OpenDocument` for any newly visible embed. A child's first
        // viewport request then never reaches Logic before its document is open. A
        // dirty child forces the content layer: children paint over the board, so
        // the board must recomposite even when otherwise idle.
        if self.sync_children(raf_timestamp) {
            plan.content = ContentRepaint::Full;
        }
        // The view shifted in logical pixels; the copy runs on the buffer.
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.size.shift_in_physical(&mut plan);
        }

        // And the sandboxed frames behind the board's web embeds (`TOILE-088`).
        // Every frame, not only a dirty one. A frame is DOM sitting over the
        // canvas, so it follows the camera even when nothing the board paints
        // has changed.
        self.sync_web_embeds();

        // rAF only starts after GPU init, so `self.gpu` is always Some here.
        let Some(gpu) = self.gpu.as_mut() else { return };

        let phases = Phases::of(plan.content, plan.overlay);
        let content_active = phases.build_content;
        let overlay_redraw = phases.build_overlay;
        let any_dirty = phases.any_build();

        let stats = FrameStats {
            fps: self.render_fps,
            frame_ms: self.last_frame_ms,
            drain_ms: self.last_drain_ms,
            draw_ms: self.last_draw_ms,
            gpu_ms: self.last_gpu_ms,
        };

        let t0 = self.perf.now();

        // -- Build CPU batches for whichever layers are being rebuilt. --------
        let t_draw_ms = if any_dirty {
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.size);
            let t_draw_start = self.perf.now();
            {
                let mut surface = WgpuSurface::new(&mut gpu.quad_renderer, &gpu.device, &gpu.queue);
                if content_active {
                    surface.begin_layer(Layer::Content);
                    self.view.paint_content(
                        &mut surface,
                        &mut gpu.font_cx,
                        &mut gpu.text_cache,
                        plan.content_strip,
                    );
                    // Paint each visible child over the board's placeholder,
                    // clipped to its embed rect ("Render"). The child draws in
                    // its own pixel space, through the embed's own transform, so
                    // a rotated embed paints its child rotated (`TOILE-UI-096`).
                    // That is one child pixel per screen pixel at the current
                    // zoom. The clip stays the AABB: `push_clip` is rect-only
                    // ("Clip limitation").
                    paint_children(
                        &mut surface,
                        &self.view.visible_embeds(),
                        &mut self.children,
                        |child, s| {
                            child.paint_content(s, &mut gpu.font_cx, &mut gpu.text_cache, None);
                        },
                    );
                }
                if overlay_redraw {
                    surface.begin_layer(Layer::Overlay);
                    self.view.paint_overlay(
                        &mut surface,
                        &mut gpu.font_cx,
                        &mut gpu.text_cache,
                        &stats,
                    );
                    // And each child's own overlay over it (`TOILE-UI-320`). A
                    // selection, a caret and a set of handles are overlay
                    // content in every app. A child painted for its content
                    // alone showed an embedded document with no visible cursor.
                    // A user typing into it could not see where they were
                    // typing. Same clip and same transform as the content pass,
                    // so the two halves of one child land in one place.
                    paint_children(
                        &mut surface,
                        &self.view.visible_embeds(),
                        &mut self.children,
                        |child, s| {
                            child.paint_overlay(s, &mut gpu.font_cx, &mut gpu.text_cache, &stats);
                        },
                    );
                }
            }
            gpu.text_cache.evict_if_oversized(5_000);
            (self.perf.now() - t_draw_start) as f32
        } else {
            0.0
        };

        // -- GPU encoding + present. ------------------------------------------
        // Every frame presents, a reused layer re-blitted (`CANVAS-010`).
        if !phases.present {
            return;
        }
        let t_gpu_start = self.perf.now();
        if !super::cell::present(gpu, &plan) {
            return;
        }

        let t2 = self.perf.now();
        self.last_frame_ms = (t2 - t0) as f32;
        self.last_drain_ms = (t_after_drain - t_drain) as f32;
        self.last_draw_ms = t_draw_ms;
        self.last_gpu_ms = (t2 - t_gpu_start) as f32;
    }

    /// Paint the unrecoverable-error terminal screen: a dark backdrop with a
    /// centered title and the panic `message` below it. Encodes one full frame
    /// over the content layer; the caller paints it once (see `render`).
    ///
    /// This is the canvas counterpart of the `window.ambreFatal` DOM overlay
    /// (index.html). It runs when the UI/render thread is still alive, after a
    /// Logic panic or a Sync-worker death. The canvas then shows a deliberate end
    /// state instead of freezing on the last good frame.
    fn render_fatal(&mut self, message: &str) {
        let Some(gpu) = self.gpu.as_mut() else { return };

        let (w, h) = gpu.size.logical();
        gpu.quad_renderer.begin_frame(gpu.size);
        {
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer, &gpu.device, &gpu.queue);
            surface.begin_layer(Layer::Content);
            paint_fatal(
                &mut surface,
                &mut gpu.font_cx,
                &mut gpu.text_cache,
                (w as f32, h as f32),
                message,
            );
        }

        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("fatal screen get_current_texture: {e:?}");
                return;
            }
        };
        let target_view = surface_tex.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.quad_renderer.flush_dirty_rect(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &target_view,
            &RepaintPlan {
                content: ContentRepaint::Full,
                content_strip: None,
                overlay: OverlayRepaint::Redraw,
                backdrop: FATAL_BACKDROP,
            },
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
    }
}

#[cfg(test)]
mod tests {
    use super::{paint_children, reconcile_children, remap_with, send_faces};
    use ambre_bus::LogicCommand;
    use ambre_bus::app::toile::ElementId;
    use ambre_bus::{DocType, DocumentId};
    use ambre_canvas::InputEvent;
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use ambre_draw::{Affine, Color, DrawSurface, Point, Rect, Size};
    use ambre_toile::ui::embed::VisibleEmbed;
    use std::collections::HashMap;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn embed(element: &str, document_id: DocumentId, to_screen: Affine) -> VisibleEmbed {
        VisibleEmbed {
            element: ElementId(element.to_owned()),
            document_id,
            doc_type: DocType::Trame,
            screen_rect: Rect::new(10.0, 20.0, 110.0, 80.0),
            child_size: Size::new(100.0, 60.0),
            to_screen,
            focused: false,
        }
    }

    fn pointers() -> Vec<InputEvent> {
        vec![
            InputEvent::PointerDown {
                x: 5.0,
                y: 6.0,
                button: 0,
                shift: false,
                ctrl: false,
            },
            InputEvent::PointerMove { x: 5.0, y: 6.0 },
            InputEvent::PointerUp {
                x: 5.0,
                y: 6.0,
                button: 0,
                ctrl: false,
            },
            InputEvent::DoubleClick { x: 5.0, y: 6.0 },
        ]
    }

    /// `TOILE-UI-328`: a pointer event the board cannot map into the child is
    /// dropped. It is never handed over with the board's own coordinate.
    #[wasm_bindgen_test]
    fn an_unmappable_pointer_event_is_dropped() {
        for ev in pointers() {
            assert!(
                remap_with(&ev, |_| None).is_none(),
                "an unmapped pointer reached the child"
            );
        }
        // And a mappable one arrives in the child's space.
        let moved = remap_with(&InputEvent::PointerMove { x: 5.0, y: 6.0 }, |p| {
            Some(Point::new(p.x - 5.0, p.y - 6.0))
        });
        assert!(matches!(moved, Some(InputEvent::PointerMove { x, y }) if x == 0.0 && y == 0.0));
    }

    /// A key, a character, a composition step and a wheel reach
    /// the child as they came. They carry no position, so even a board that
    /// cannot map a point hands them over unchanged.
    #[wasm_bindgen_test]
    fn a_key_a_character_a_composition_and_a_wheel_reach_the_child_verbatim() {
        let key = InputEvent::Key {
            key: "a".to_owned(),
            ctrl: true,
            alt: false,
            shift: true,
            meta: false,
        };
        assert!(matches!(
            remap_with(&key, |_| None),
            Some(InputEvent::Key { key, ctrl: true, alt: false, shift: true, meta: false })
                if key == "a"
        ));
        let text = InputEvent::Text {
            text: "é".to_owned(),
        };
        assert!(
            matches!(remap_with(&text, |_| None), Some(InputEvent::Text { text }) if text == "é")
        );
        let step = InputEvent::CompositionUpdate {
            text: "かな".to_owned(),
        };
        assert!(matches!(
            remap_with(&step, |_| None),
            Some(InputEvent::CompositionUpdate { text }) if text == "かな"
        ));
        let end = InputEvent::CompositionEnd {
            text: "仮名".to_owned(),
        };
        assert!(matches!(
            remap_with(&end, |_| None),
            Some(InputEvent::CompositionEnd { text }) if text == "仮名"
        ));
        let delta = ambre_canvas::WheelDelta::new(1.5, -3.0, ambre_canvas::WheelDeltaMode::Line);
        let wheel = InputEvent::Wheel {
            delta,
            ctrl_key: true,
        };
        assert!(matches!(
            remap_with(&wheel, |_| None),
            Some(InputEvent::Wheel { delta: kept, ctrl_key: true }) if kept == delta
        ));
    }

    /// `TOILE-UI-097`: two embeds of one document are two children, each made
    /// for its own element, and a third reconcile keeps them both.
    #[wasm_bindgen_test]
    fn two_embeds_of_one_document_are_two_children() {
        let doc = DocumentId::new();
        let embeds = [
            embed("a", doc, Affine::IDENTITY),
            embed("b", doc, Affine::IDENTITY),
        ];
        let mut children: HashMap<ElementId, String> = HashMap::new();
        let mut sizes = HashMap::new();
        let mut made = Vec::new();
        reconcile_children(
            &mut children,
            &mut sizes,
            &embeds,
            |e| {
                made.push(e.element.0.clone());
                e.element.0.clone()
            },
            |_, _, _| {},
            |_| false,
        );
        assert_eq!(children.len(), 2, "two embeds of one document share a view");
        assert_eq!(made, ["a", "b"]);
        assert_eq!(children[&ElementId("b".to_owned())], "b");
    }

    /// A child that changed on its own makes the board
    /// recomposite, and a quiet frame does not.
    #[wasm_bindgen_test]
    fn a_child_that_changed_on_its_own_repaints_the_board() {
        let embeds = [embed("a", DocumentId::new(), Affine::IDENTITY)];
        let mut children: HashMap<ElementId, u32> = HashMap::new();
        let mut sizes = HashMap::new();
        let mut pass = |changed: bool| {
            reconcile_children(
                &mut children,
                &mut sizes,
                &embeds,
                |_| 0,
                |_, _, _| {},
                |_| changed,
            )
        };
        assert!(pass(false), "a fresh child does not repaint the board");
        assert!(!pass(false), "a quiet frame repaints the board");
        assert!(
            pass(true),
            "a child's own change does not repaint the board"
        );
    }

    /// `TOILE-UI-096`, `TOILE-UI-320`: a child paints inside its embed's clip,
    /// through the embed's own transform, and both are popped after it. The
    /// content and the overlay pass both go through this one walk, so a
    /// rotated embed paints both halves of its child rotated.
    #[wasm_bindgen_test]
    fn a_child_paints_inside_its_embeds_clip_and_transform() {
        let turned = Affine::translate((40.0, 30.0)) * Affine::rotate(0.5);
        let embeds = [embed("a", DocumentId::new(), turned)];
        let mut children: HashMap<ElementId, ()> = HashMap::from([(ElementId("a".to_owned()), ())]);
        let mut surface = RecordingSurface::new();
        let red = Color::from_rgba8(255, 0, 0, 255);
        paint_children(&mut surface, &embeds, &mut children, |_, s| {
            s.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), red);
        });
        assert_eq!(
            surface.ops(),
            [
                DrawOp::PushClip {
                    rect: embeds[0].screen_rect
                },
                DrawOp::PushTransform { transform: turned },
                DrawOp::FillRect {
                    rect: Rect::new(0.0, 0.0, 4.0, 4.0),
                    color: red
                },
                DrawOp::PopTransform,
                DrawOp::PopClip,
            ]
        );
        assert_eq!((surface.clip_depth(), surface.transform_depth()), (0, 0));
    }

    /// `CANVAS-068`: a face fetched after boot reaches the Logic thread as the
    /// load a base face is, with its bytes intact. A full channel drops it
    /// rather than wait.
    #[wasm_bindgen_test]
    fn a_late_face_reaches_the_logic_thread_as_a_base_face_does() {
        let (to_logic, from_ui) = ambre_bus::conduit::local(1);
        assert!(send_faces(&to_logic, vec![vec![1, 2, 3]]));
        assert!(
            !send_faces(&to_logic, vec![vec![4]]),
            "a full channel took the face"
        );
        match from_ui.try_recv() {
            Ok(LogicCommand::LoadFonts { faces }) => assert_eq!(faces, [vec![1, 2, 3]]),
            other => panic!("not a font load: {other:?}"),
        }
        assert!(from_ui.try_recv().is_err(), "the dropped face arrived");
    }
}
