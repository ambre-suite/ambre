// ambre-app/src/host/frame.rs
// ============================================================================
// FrameHost: the per-frame render driver. Owns the active `AppView`, the GPU
// state (installed after `bootstrap_gpu`), the fps ring buffer, and the fatal
// screen. Drives one document view per frame; the document is swapped in place
// by `DocSwitcher` (see `run`).
// ============================================================================

use super::*;

pub struct FrameHost {
    gpu: Option<GpuState>,
    view: AppView,
    perf: web_sys::Performance,

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
    pub fn new(view: AppView, perf: web_sys::Performance) -> Self {
        Self {
            gpu: None,
            view,
            perf,
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

    /// Forward an input event to the view. Returns `true` if the caller should
    /// `preventDefault()` on the originating DOM event.
    pub fn handle_input(&mut self, ev: &InputEvent) -> bool {
        self.view.handle_input(ev)
    }

    /// Pull the clipboard contents a `copy`/`cut` event should place on the OS
    /// clipboard (and, for `cut`, remove from the source). `None` means "nothing
    /// selected -- let the browser default proceed".
    pub fn clipboard_write(
        &mut self,
        cut: bool,
    ) -> Option<(String, Option<ambre_bus::ClipboardPayload>)> {
        self.view.clipboard_write(cut)
    }

    /// Service a `paste` event with the decoded Ambre payload (if any) and the
    /// plain text fallback.
    pub fn paste_clipboard(&mut self, payload: Option<ambre_bus::ClipboardPayload>, text: String) {
        self.view.paste_clipboard(payload, text);
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

        // rAF only starts after GPU init, so `self.gpu` is always Some here.
        let Some(gpu) = self.gpu.as_mut() else { return };

        Self::tick_fps(&mut self.render_times, &mut self.render_fps, raf_timestamp);

        let mut hud_tick = false;
        if raf_timestamp - self.last_hud_update_ms >= 500.0 {
            self.last_hud_update_ms = raf_timestamp;
            hud_tick = true;
        }

        let plan = self.view.plan_repaint(hud_tick);
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
                let mut surface = WgpuSurface::new(&mut gpu.quad_renderer);
                if content_active {
                    surface.begin_layer(Layer::Content);
                    self.view.paint_content(
                        &mut surface,
                        &mut gpu.font_cx,
                        &mut gpu.text_cache,
                        plan.content_strip,
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
    /// (index.html). It runs when the UI/render thread is still alive -- a Logic
    /// panic or a Sync-worker death -- so the canvas shows a deliberate end state
    /// instead of freezing on the last good frame.
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
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer);
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
