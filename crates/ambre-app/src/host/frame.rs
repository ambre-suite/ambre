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
/// CSS pixels per physical pixel. The board publishes screen geometry in
/// physical pixels; the DOM lays out in CSS pixels.
fn device_pixel_ratio() -> f64 {
    web_sys::window().map_or(1.0, |w| w.device_pixel_ratio())
}

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
/// type and child canvas size, it wires a demultiplexed Logic->UI channel and
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
    let map = |x: f64, y: f64| {
        view.embed_to_child_space(Point::new(x, y))
            .map(|p| (p.x, p.y))
    };
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

pub struct FrameHost {
    gpu: Option<GpuState>,
    view: AppView,
    perf: web_sys::Performance,
    /// The per-page-load clipboard ownership token (`UI-THREAD-007`). Written on
    /// `copy`/`cut`, re-published by the async write (`UI-THREAD-023`), and read
    /// back to answer rung 2 of the Paste-button path.
    clip_token: String,
    /// Clipboard reads the Paste button hands over, already decoded by the shell.
    /// Drained once per frame and routed to the active view (`UI-THREAD-024`).
    paste_rx: flume::Receiver<ClipboardPaste>,

    /// The command channel to the Logic thread, held for one purpose. A font
    /// face fetched after boot has to reach the threads that measure on a
    /// context of their own (`CANVAS-068`). Only this thread can fetch, and
    /// Prose paginates on the Logic thread.
    to_logic: flume::Sender<LogicCommand>,

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
    /// The last physical canvas size each child was resized to. A moved or zoomed
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

    render_times: VecDeque<f64>,
    render_fps: f32,
    last_frame_ms: f32,
    last_drain_ms: f32,
    last_draw_ms: f32,
    last_gpu_ms: f32,
    /// rAF timestamp of the last HUD rebuild. The overlay refreshes at most
    /// every 500 ms so idle frames never trigger a full vertex rebuild.
    last_hud_update_ms: f64,
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
        clip_token: String,
        paste_rx: flume::Receiver<ClipboardPaste>,
        to_logic: flume::Sender<LogicCommand>,
    ) -> Self {
        Self {
            gpu: None,
            view,
            perf,
            clip_token,
            paste_rx,
            to_logic,
            children: HashMap::new(),
            child_sizes: HashMap::new(),
            make_child,
            web_frames: web_embed::WebFrames::default(),
            web_frames_empty: true,
            render_times: VecDeque::with_capacity(120),
            render_fps: 0.0,
            last_frame_ms: 0.0,
            last_drain_ms: 0.0,
            last_draw_ms: 0.0,
            last_gpu_ms: 0.0,
            last_hud_update_ms: 0.0,
            fatal_painted: false,
        }
    }

    /// Hand the host its GPU resources once `bootstrap_gpu` has resolved.
    pub fn install_gpu(&mut self, gpu: GpuState) {
        self.gpu = Some(gpu);
    }

    /// The per-page-load clipboard ownership token (`UI-THREAD-007`). The
    /// clipboard event listeners in `run` read it back to decide a same-tab copy.
    pub(crate) fn clip_token(&self) -> &str {
        &self.clip_token
    }

    /// Forward an input event to the view. Returns `true` if the caller should
    /// `preventDefault()` on the originating DOM event.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
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
    /// child also drops its demultiplexed Logic->UI channel, so the relay reclaims
    /// the route on its next send. Returns whether any child needs the board
    /// recomposited this frame (a fresh child, a resize, or a self-driven content
    /// change). A no-op when the launcher installed no child factory.
    fn sync_children(&mut self, now_ms: f64) -> bool {
        let Some(make_child) = self.make_child.clone() else {
            return false;
        };
        let embeds = self.view.visible_embeds();
        let live: HashSet<ElementId> = embeds.iter().map(|e| e.element.clone()).collect();
        self.children.retain(|id, _| live.contains(id));
        self.child_sizes.retain(|id, _| live.contains(id));

        let mut dirty = false;
        for e in &embeds {
            let w = e.child_size.width.round().max(0.0) as u32;
            let h = e.child_size.height.round().max(0.0) as u32;
            if w == 0 || h == 0 {
                continue;
            }
            if !self.children.contains_key(&e.element) {
                let mut child = make_child(e.document_id, e.doc_type, e.child_size);
                child.resize(w, h);
                self.children.insert(e.element.clone(), child);
                self.child_sizes.insert(e.element.clone(), (w, h));
                dirty = true;
            } else if self.child_sizes.get(&e.element) != Some(&(w, h)) {
                if let Some(child) = self.children.get_mut(&e.element) {
                    child.resize(w, h);
                }
                self.child_sizes.insert(e.element.clone(), (w, h));
                dirty = true;
            }
        }

        // Pump every live child: drain its Logic->UI events, run its per-frame
        // planning. A non-`Reuse` content plan means it changed on its own (a
        // remote edit), so the board must recomposite even if otherwise idle.
        for child in self.children.values_mut() {
            child.update();
            let plan = child.plan_repaint(now_ms, false);
            if !matches!(plan.content, ContentRepaint::Reuse) {
                dirty = true;
            }
        }
        dirty
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
        self.web_frames.sync(&layer, &embeds, device_pixel_ratio());
        self.web_frames_empty = embeds.is_empty();
    }

    /// Resolve a `copy`/`cut` synchronously inside the native clipboard event.
    /// See [`AppView::clipboard_write`]: `Text` writes immediately, `Deferred`
    /// waits for [`poll_clipboard`](Self::poll_clipboard), `Nothing` lets the
    /// browser default proceed.
    pub fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        self.view.clipboard_write(cut)
    }

    /// Drain a pending authoritative clipboard blob for the OS clipboard write.
    /// Called per frame right after [`update`](Self::update).
    pub fn poll_clipboard(&mut self) -> Option<ambre_bus::ClipboardBlob> {
        self.view.poll_clipboard()
    }

    /// Service a `paste` event with the shared clipboard transport blob.
    /// `own_copy` signals the clipboard still holds this tab's ownership token.
    pub fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        self.view.paste_clipboard(blob, own_copy);
    }

    /// Switch the active document on the live view, keeping the GPU surface.
    pub fn set_document(&mut self, document_id: DocumentId) {
        self.view.set_document(document_id);
    }

    /// Reconfigure the GPU surface and notify the view of a canvas resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.view.resize(width, height);

        let Some(gpu) = self.gpu.as_mut() else { return };
        if gpu.width == width && gpu.height == height {
            return;
        }
        tracing::info!("GPU resize: {width}x{height}");

        gpu.surface.configure(
            &gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: gpu.surface_format,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoNoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
            },
        );
        // Reallocate the ping-pong offscreens at the new size; previous content
        // is lost so the view's resize() must have flagged a full repaint.
        gpu.quad_renderer
            .resize_offscreens(&gpu.device, width, height);

        gpu.width = width;
        gpu.height = height;
    }

    /// Recover after the WebGL/WebGPU context was lost and restored. The GL
    /// drawable and every GPU-side allocation were dropped. So we reconfigure
    /// the surface and rebuild the ping-pong offscreens at the current size,
    /// then force a full repaint through the view. A plain `resize` to the same
    /// dimensions early-returns and would leave a blank canvas.
    pub fn notify_context_restored(&mut self, width: u32, height: u32) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        tracing::info!("GPU context restored, reconfiguring {width}x{height}");
        gpu.surface.configure(
            &gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: gpu.surface_format,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoNoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
            },
        );
        gpu.quad_renderer
            .resize_offscreens(&gpu.device, width, height);
        gpu.width = width;
        gpu.height = height;
        // Force the view to invalidate every cached layer for a clean repaint.
        self.view.resize(width, height);
    }

    pub(crate) fn tick_fps(times: &mut VecDeque<f64>, fps: &mut f32, now: f64) {
        times.push_back(now);
        while times.front().is_some_and(|&t| now - t > 1000.0) {
            times.pop_front();
        }
        *fps = if times.len() >= 2 {
            (times.len() - 1) as f32 / ((now - times.front().unwrap()) / 1000.0) as f32
        } else {
            0.0
        };
    }

    /// Render one frame. Wired into [`start_raf_loop`] by [`run_with`].
    pub fn render(&mut self, raf_timestamp: f64) {
        // Fatal latch first: once set, the Logic thread feeding the view may be
        // aborted, so we must not keep draining it. Paint the terminal screen a
        // single time and then idle (the rAF loop keeps ticking harmlessly).
        if let Some(message) = fatal_message() {
            if !self.fatal_painted {
                self.render_fatal(&message);
                self.fatal_painted = true;
            }
            return;
        }

        let t_drain = self.perf.now();
        self.view.update();
        let t_after_drain = self.perf.now();

        // Service any Paste-button read the shell decoded. The shell read the
        // clipboard inside the click gesture. The host answers rung 2 by
        // comparing the token, then hands the blob to the active view
        // (`UI-THREAD-024`, `UI-THREAD-025`).
        while let Ok(paste) = self.paste_rx.try_recv() {
            let own_copy = paste.token.as_deref() == Some(&self.clip_token);
            self.view.paste_clipboard(paste.blob, own_copy);
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
            async_write_clipboard(blob.text, blob.html, payload_b64, self.clip_token.clone());
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
            // paginates into tofu (`CANVAS-068`, `PROSE-115`). A full channel
            // drops the face rather than stall the frame; the next lazy fetch
            // is a fresh delivery.
            let _ = self
                .to_logic
                .try_send(LogicCommand::LoadFonts { faces: new_faces });
            if let Some((w, h)) = self.gpu.as_ref().map(|g| (g.width, g.height)) {
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

        // And the sandboxed frames behind the board's web embeds (`TOILE-088`).
        // Every frame, not only a dirty one. A frame is DOM sitting over the
        // canvas, so it follows the camera even when nothing the board paints
        // has changed.
        self.sync_web_embeds();

        // rAF only starts after GPU init, so `self.gpu` is always Some here.
        let Some(gpu) = self.gpu.as_mut() else { return };

        let content_active = !matches!(plan.content, ContentRepaint::Reuse);
        let overlay_redraw = matches!(plan.overlay, OverlayRepaint::Redraw);
        let any_dirty = content_active || overlay_redraw;

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
            gpu.quad_renderer
                .begin_frame(gpu.width as f32, gpu.height as f32);
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
                    for e in self.view.visible_embeds() {
                        let Some(child) = self.children.get_mut(&e.element) else {
                            continue;
                        };
                        let r = e.screen_rect;
                        if r.width() < 1.0 || r.height() < 1.0 {
                            continue;
                        }
                        surface.push_clip(r);
                        surface.push_transform(e.to_screen);
                        child.paint_content(
                            &mut surface,
                            &mut gpu.font_cx,
                            &mut gpu.text_cache,
                            None,
                        );
                        surface.pop_transform();
                        surface.pop_clip();
                    }
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
                    for e in self.view.visible_embeds() {
                        let Some(child) = self.children.get_mut(&e.element) else {
                            continue;
                        };
                        let r = e.screen_rect;
                        if r.width() < 1.0 || r.height() < 1.0 {
                            continue;
                        }
                        surface.push_clip(r);
                        surface.push_transform(e.to_screen);
                        child.paint_overlay(
                            &mut surface,
                            &mut gpu.font_cx,
                            &mut gpu.text_cache,
                            &stats,
                        );
                        surface.pop_transform();
                        surface.pop_clip();
                    }
                }
            }
            gpu.text_cache.evict_if_oversized(5_000);
            (self.perf.now() - t_draw_start) as f32
        } else {
            0.0
        };

        // -- GPU encoding + present. ------------------------------------------
        let t_gpu_start = self.perf.now();
        let surface_tex = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            // A lost/outdated surface is recoverable: the GL context was reset
            // (tab backgrounded, GPU reset, `webglcontextrestored`) or the
            // drawable went stale after a resize. Reconfigure with the current
            // size and skip this frame; the next rAF tick draws fresh. Only a
            // genuinely unrecoverable error (OOM, removed device) is logged and
            // dropped.
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                tracing::warn!("surface lost/outdated, reconfiguring");
                gpu.surface.configure(
                    &gpu.device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: gpu.surface_format,
                        width: gpu.width,
                        height: gpu.height,
                        present_mode: wgpu::PresentMode::AutoNoVsync,
                        desired_maximum_frame_latency: 2,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                    },
                );
                return;
            }
            Err(e) => {
                tracing::error!("get_current_texture: {e:?}");
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
            plan.content,
            plan.overlay,
        );

        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();

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

        let w = gpu.width as f32;
        let h = gpu.height as f32;

        // Fixed colours (no theme dependency on the fatal path): a near-black
        // backdrop, a red title, and a dim grey body. ASCII strings only.
        const BACKDROP: Color = Color::from_rgba8(0x1a, 0x1a, 0x2e, 255);
        const TITLE: Color = Color::from_rgba8(0xe0, 0x5a, 0x5a, 255);
        const BODY: Color = Color::from_rgba8(0x9a, 0xa0, 0xa6, 255);
        const TITLE_TEXT: &str = "Ambre encountered a fatal error and must reload.";

        // Wrap the title/message to a centered column at most 640 logical px wide.
        let col_w = (w * 0.8).min(640.0);
        let origin_x = ((w as f64) - col_w as f64) / 2.0;

        gpu.quad_renderer.begin_frame(w, h);
        {
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer, &gpu.device, &gpu.queue);
            surface.begin_layer(Layer::Content);
            surface.fill_rect(Rect::new(0.0, 0.0, w as f64, h as f64), BACKDROP);

            // Lay out both blocks first so we can vertically center the stack.
            let title = gpu.text_cache.get_or_build(
                &mut gpu.font_cx,
                TITLE_TEXT,
                FONT_HEADING,
                col_w,
                Alignment::Center,
                false,
                false,
            );
            let title_h = title.height() as f64;
            let body = gpu.text_cache.get_or_build(
                &mut gpu.font_cx,
                message,
                FONT_BODY,
                col_w,
                Alignment::Center,
                false,
                false,
            );
            let body_h = body.height() as f64;

            let gap = 16.0;
            let total_h = title_h + gap + body_h;
            let title_y = ((h as f64) - total_h) / 2.0;
            let body_y = title_y + title_h + gap;

            draw_centered_text(
                &mut surface,
                &mut gpu.font_cx,
                &mut gpu.text_cache,
                TITLE_TEXT,
                FONT_HEADING,
                col_w,
                origin_x,
                title_y,
                TITLE,
            );
            draw_centered_text(
                &mut surface,
                &mut gpu.font_cx,
                &mut gpu.text_cache,
                message,
                FONT_BODY,
                col_w,
                origin_x,
                body_y,
                BODY,
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
            ContentRepaint::Full,
            OverlayRepaint::Redraw,
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));
        surface_tex.present();
    }
}
