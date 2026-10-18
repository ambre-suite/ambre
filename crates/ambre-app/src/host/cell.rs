// ambre-app/src/host/cell.rs
// ============================================================================
// The canvas cells beside the content canvas. A view that shows several
// regions gives each one a canvas of its own (`CANVAS-070`). A Murmure channel
// and its thread are two such regions (the Murmure message list spec). The
// shell mounts the element. The host binds a
// cell to it by id while it is in the page, and drops the cell once it leaves.
//
// Each cell holds a GPU state of its own. On the WebGL2 backend a device
// presents only to the canvas whose context created it, so two canvases cannot
// share one (`ambre-spec-frame.md` section 2). Each cell has its own size, its
// own input and its own repaint plan. The one rAF callback renders them all.
// ============================================================================

use super::*;

/// The listeners a cell takes its input from, on its own canvas.
const INPUTS: [&str; 5] = [
    "pointerdown",
    "pointermove",
    "pointerup",
    "pointercancel",
    "wheel",
];

/// A DOM listener a cell holds, with the event it listens to.
type Listener = (&'static str, Closure<dyn FnMut(web_sys::Event)>);

/// The cells a view shows beside its content canvas, keyed by canvas id.
#[derive(Default)]
pub(super) struct Cells {
    bound: HashMap<&'static str, Bound>,
}

impl Cells {
    /// Bind a cell to each canvas `view` names that is in the page with a box,
    /// and drop each cell whose canvas left. A canvas with no box yet, such as
    /// a hidden panel, binds no GPU until it shows. A view that names no
    /// canvas costs no DOM lookup.
    pub(super) fn sync(&mut self, view: &AppView) {
        let ids = view.cell_ids();
        if ids.is_empty() && self.bound.is_empty() {
            return;
        }
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let lookup = |id: &str| -> Option<HtmlCanvasElement> {
            document.get_element_by_id(id)?.dyn_into().ok()
        };
        self.bound.retain(|id, cell| {
            ids.contains(id) && lookup(id).is_some_and(|canvas| canvas == cell.canvas)
        });
        for &id in ids {
            if self.bound.contains_key(id) {
                continue;
            }
            let Some(canvas) = lookup(id) else { continue };
            let Some(size) = size_backing_store(&canvas) else {
                continue;
            };
            let (width, height) = size.logical();
            if let Some(cell) = view
                .cell_view(id, f64::from(width), f64::from(height))
                .and_then(|region| Bound::bind(canvas, size, region))
            {
                self.bound.insert(id, cell);
            }
        }
    }

    /// Render every bound cell, each under its own plan (`CANVAS-070`).
    pub(super) fn render(&mut self, now_ms: f64) {
        for cell in self.bound.values_mut() {
            cell.render(now_ms);
        }
    }

    /// Measure every cell again. A density change moves no box, so no
    /// observer of a cell fires on it (`CANVAS-035`).
    pub(super) fn remeasure(&self) {
        for cell in self.bound.values() {
            cell.measure();
        }
    }
}

/// One canvas bound to the region view it paints.
struct Bound {
    canvas: HtmlCanvasElement,
    view: Rc<RefCell<AppView>>,
    /// Filled once the bootstrap of this canvas resolves; `None` before, and
    /// for a canvas the GPU refused.
    gpu: Rc<RefCell<Option<GpuState>>>,
    /// The size the canvas was last measured at. The frame fits the GPU to it.
    size: Rc<std::cell::Cell<SurfaceSize>>,
    /// Set when the browser restored a lost context. The next frame fits the
    /// surface again and repaints every layer.
    restored: Rc<std::cell::Cell<bool>>,
    observer: web_sys::ResizeObserver,
    _observe: Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
    listeners: Vec<Listener>,
}

impl Bound {
    /// Bind `canvas`, measured at `size`, to `view`, and bring its GPU up.
    /// `None` when the page cannot observe the canvas box.
    fn bind(canvas: HtmlCanvasElement, size: SurfaceSize, view: AppView) -> Option<Self> {
        let view = Rc::new(RefCell::new(view));
        let gpu: Rc<RefCell<Option<GpuState>>> = Rc::new(RefCell::new(None));
        let restored = Rc::new(std::cell::Cell::new(false));

        // The closures hold weak handles, so a cell dropped while one is
        // pending frees its state. A size measured meanwhile is fitted on the
        // first frame the GPU is up.
        let slot = Rc::downgrade(&gpu);
        let target = canvas.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (_settled, state) = bootstrap_gpu(target, size).await;
            match (slot.upgrade(), state) {
                (Some(slot), Some(state)) => *slot.borrow_mut() = Some(state),
                (Some(_), None) => tracing::error!("a canvas cell got no GPU"),
                (None, _) => {}
            }
        });
        let size = Rc::new(std::cell::Cell::new(size));

        let observe = {
            let canvas = canvas.clone();
            let view = Rc::downgrade(&view);
            let size = Rc::downgrade(&size);
            Closure::<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>::new(move |_, _| {
                if let (Some(view), Some(size)) = (view.upgrade(), size.upgrade()) {
                    measure(&canvas, &view, &size);
                }
            })
        };
        let observer = web_sys::ResizeObserver::new(observe.as_ref().unchecked_ref()).ok()?;
        observer.observe(&canvas);

        let mut listeners = Vec::with_capacity(INPUTS.len() + 2);
        for kind in INPUTS {
            let view = Rc::downgrade(&view);
            let on_input =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let Some(view) = view.upgrade() else { return };
                    if let Some(input) = input_of(&event)
                        && view.borrow_mut().handle_input(&input)
                    {
                        event.prevent_default();
                    }
                });
            listeners.push((kind, on_input));
        }
        // `preventDefault` on a lost context is what lets the browser restore it.
        listeners.push((
            "webglcontextlost",
            Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
                event.prevent_default();
            }),
        ));
        {
            let restored = Rc::downgrade(&restored);
            listeners.push((
                "webglcontextrestored",
                Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                    if let Some(restored) = restored.upgrade() {
                        restored.set(true);
                    }
                }),
            ));
        }
        for (kind, listener) in &listeners {
            let _ =
                canvas.add_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }

        Some(Self {
            canvas,
            view,
            gpu,
            size,
            restored,
            observer,
            _observe: observe,
            listeners,
        })
    }

    /// Measure the canvas against its cell, and resize the view to it.
    fn measure(&self) {
        measure(&self.canvas, &self.view, &self.size);
    }

    /// Plan, paint and present this cell. Nothing runs before its GPU is up,
    /// so a plan is never spent on a canvas that cannot show it. Answers the
    /// plan it presented.
    fn render(&mut self, now_ms: f64) -> Option<RepaintPlan> {
        let mut view = self.view.borrow_mut();
        view.update();
        let mut slot = self.gpu.borrow_mut();
        let gpu = slot.as_mut()?;
        let size = self.size.get();
        let refit = gpu.size != size || self.restored.replace(false);
        if refit {
            gpu.configure(size);
        }
        // A script face fetched for this cell repaints it, as on the content
        // canvas. The content canvas alone hands faces to the Logic thread.
        let faces = !gpu.text_cache.poll_lazy_fonts(&mut gpu.font_cx).is_empty();
        let plan = plan_frame(&mut view, size, refit || faces, now_ms);
        let phases = Phases::of(plan.content, plan.overlay);
        if phases.any_build() {
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.size);
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer, &gpu.device, &gpu.queue);
            if phases.build_content {
                surface.begin_layer(Layer::Content);
                view.paint_content(
                    &mut surface,
                    &mut gpu.font_cx,
                    &mut gpu.text_cache,
                    plan.content_strip,
                );
            }
            if phases.build_overlay {
                surface.begin_layer(Layer::Overlay);
                view.paint_overlay(
                    &mut surface,
                    &mut gpu.font_cx,
                    &mut gpu.text_cache,
                    &FrameStats::default(),
                );
            }
        }
        if phases.present {
            present(gpu, &plan);
        }
        Some(plan)
    }
}

impl Drop for Bound {
    /// Take the listeners off the canvas, stop observing it, and release its
    /// context rather than wait for the collector. A page holds few live
    /// contexts, and each thread opened would spend one.
    fn drop(&mut self) {
        for (kind, listener) in &self.listeners {
            let _ = self
                .canvas
                .remove_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
        }
        self.observer.disconnect();
        if self.gpu.borrow_mut().take().is_some() {
            lose_context(&self.canvas);
        }
    }
}

/// Size the backing store of `canvas` to its cell, record it, and resize the
/// view. A canvas that lost its box, such as a hidden panel, keeps its size.
fn measure(
    canvas: &HtmlCanvasElement,
    view: &RefCell<AppView>,
    size: &std::cell::Cell<SurfaceSize>,
) {
    if let Some(measured) = size_backing_store(canvas) {
        size.set(measured);
        let (width, height) = measured.logical();
        view.borrow_mut().resize(width, height);
    }
}

/// Plan the frame of a cell's `view` at `size`, the size its GPU is fitted
/// to. A `relayout` lays the view out again first. The plan answers in the
/// physical pixels of the canvas.
fn plan_frame(view: &mut AppView, size: SurfaceSize, relayout: bool, now_ms: f64) -> RepaintPlan {
    if relayout {
        let (width, height) = size.logical();
        view.resize(width, height);
    }
    let mut plan = view.plan_repaint(now_ms, false);
    size.shift_in_physical(&mut plan);
    plan
}

/// The input event a DOM event on a cell carries, in the cell's own pixels.
fn input_of(event: &web_sys::Event) -> Option<InputEvent> {
    if let Some(wheel) = event.dyn_ref::<web_sys::WheelEvent>() {
        return Some(InputEvent::Wheel {
            dx: wheel.delta_x(),
            dy: wheel.delta_y(),
            delta_mode: wheel.delta_mode(),
            ctrl_key: wheel.ctrl_key(),
        });
    }
    let pointer = event.dyn_ref::<web_sys::PointerEvent>()?;
    let (x, y) = (f64::from(pointer.offset_x()), f64::from(pointer.offset_y()));
    let ctrl = pointer.ctrl_key() || pointer.meta_key();
    Some(match event.type_().as_str() {
        "pointerdown" => InputEvent::PointerDown {
            x,
            y,
            button: pointer.button(),
            shift: pointer.shift_key(),
            ctrl,
        },
        "pointermove" => InputEvent::PointerMove { x, y },
        "pointerup" => InputEvent::PointerUp {
            x,
            y,
            button: pointer.button(),
            ctrl,
        },
        // The browser took the gesture over: it ends where it was, as a
        // release (`KINETIC-010`).
        "pointercancel" => InputEvent::PointerUp {
            x,
            y,
            button: 0,
            ctrl,
        },
        _ => return None,
    })
}

/// Release the WebGL context of `canvas` now. A canvas under WebGPU holds no
/// WebGL context, and is left to the collector.
fn lose_context(canvas: &HtmlCanvasElement) {
    let Ok(Some(context)) = canvas.get_context("webgl2") else {
        return;
    };
    let extension = js_sys::Reflect::get(&context, &"getExtension".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .and_then(|f| f.call1(&context, &"WEBGL_lose_context".into()).ok());
    if let Some(extension) = extension.filter(|e| !e.is_null())
        && let Ok(lose) = js_sys::Reflect::get(&extension, &"loseContext".into())
        && let Ok(lose) = lose.dyn_into::<js_sys::Function>()
    {
        let _ = lose.call0(&extension);
    }
}

/// Encode the layers of `plan` onto the swapchain of `gpu` and present them.
/// A lost or outdated swapchain is fitted again and the frame skipped; the
/// next one draws fresh. Answers whether the frame reached the screen.
pub(super) fn present(gpu: &mut GpuState, plan: &RepaintPlan) -> bool {
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
            gpu.reconfigure_surface();
            return false;
        }
        Err(e) => {
            tracing::error!("get_current_texture: {e:?}");
            return false;
        }
    };
    let target_view = surface_tex.texture.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.quad_renderer
        .flush_dirty_rect(&gpu.device, &gpu.queue, &mut encoder, &target_view, plan);
    gpu.queue.submit(std::iter::once(encoder.finish()));
    surface_tex.present();
    true
}

// -- Tests (wasm tier, real `document` and GPU) --------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{MurmureView, THREAD_CANVAS};
    use ambre_canvas::ThemeControl;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A box in the page, taken out when the test ends, even on a failed
    /// assertion, so no canvas outlives its test.
    struct Mounted(web_sys::HtmlElement);

    impl Drop for Mounted {
        fn drop(&mut self) {
            self.0.remove();
        }
    }

    /// A box of `width` x `height` holding the thread's canvas, in the page.
    fn thread_cell(width: u32, height: u32) -> (Mounted, HtmlCanvasElement) {
        let document = web_sys::window().and_then(|w| w.document()).unwrap();
        let cell: web_sys::HtmlElement =
            document.create_element("div").unwrap().dyn_into().unwrap();
        resize(&cell, width, height);
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_id(THREAD_CANVAS);
        canvas
            .set_attribute("style", "display:block;width:100%;height:100%")
            .unwrap();
        cell.append_child(&canvas).unwrap();
        document.body().unwrap().append_child(&cell).unwrap();
        (Mounted(cell), canvas)
    }

    fn resize(cell: &web_sys::HtmlElement, width: u32, height: u32) {
        let style = format!("width:{width}px;height:{height}px");
        cell.set_attribute("style", &style).unwrap();
    }

    /// The content canvas's view: a Murmure page, which names the thread.
    fn page() -> AppView {
        AppView::Murmure(MurmureView::new(
            None,
            None,
            crate::host::page_reads(ambre_murmure::chrome::MurmureRelay::new()),
            (300.0, 200.0),
            ThemeControl::default(),
        ))
    }

    fn murmure(view: &AppView) -> &MurmureView {
        match view {
            AppView::Murmure(v) => v,
            _ => panic!("a Murmure view"),
        }
    }

    async fn sleep(ms: i32) {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(wait).await;
    }

    /// The thread is a canvas of its own, sized by its own cell (the Murmure
    /// message list spec). Its input reaches its own view alone, and it follows its cell
    /// when the cell resizes.
    #[wasm_bindgen_test]
    async fn the_thread_is_a_canvas_of_its_own_sized_by_its_cell() {
        let (cell, canvas) = thread_cell(160, 90);
        let page = page();
        let mut cells = Cells::default();
        cells.sync(&page);
        let bound = &cells.bound[THREAD_CANVAS];
        assert_eq!(murmure(&bound.view.borrow()).size(), (160.0, 90.0));
        assert_ne!(
            (canvas.width(), canvas.height()),
            (300, 150),
            "the backing store fits the cell, not the default"
        );

        let first = Rc::clone(&bound.view);
        cells.sync(&page);
        assert!(
            Rc::ptr_eq(&first, &cells.bound[THREAD_CANVAS].view),
            "bound once"
        );

        let moved = web_sys::PointerEvent::new("pointermove").unwrap();
        canvas.dispatch_event(&moved).unwrap();
        assert!(
            murmure(&first.borrow()).hover().is_some(),
            "the cell's own view"
        );
        assert!(murmure(&page).hover().is_none(), "not the page's");

        resize(&cell.0, 200, 120);
        sleep(50).await;
        assert_eq!(murmure(&first.borrow()).size(), (200.0, 120.0));
    }

    /// `CANVAS-070`: a cell plans its frame apart from the page. A frame
    /// before its GPU is up spends no plan. Its first plan repaints in full,
    /// an idle one reuses its layers, and a resize of its cell repaints it
    /// alone.
    #[wasm_bindgen_test]
    async fn a_cell_plans_its_frame_apart_from_the_page() {
        let (cell, _canvas) = thread_cell(120, 80);
        let mut page = page();
        let mut cells = Cells::default();
        cells.sync(&page);
        let bound = cells.bound.get_mut(THREAD_CANVAS).unwrap();
        assert!(bound.render(0.0).is_none(), "no GPU yet");

        let plan = |bound: &Bound, now_ms: f64| {
            plan_frame(
                &mut bound.view.borrow_mut(),
                bound.size.get(),
                false,
                now_ms,
            )
            .content
        };
        assert!(matches!(plan(bound, 0.0), ContentRepaint::Full));
        assert!(matches!(plan(bound, 16.0), ContentRepaint::Reuse));
        assert!(
            matches!(page.plan_repaint(16.0, false).content, ContentRepaint::Full),
            "the page keeps its own plan"
        );

        resize(&cell.0, 140, 80);
        sleep(50).await;
        let bound = &cells.bound[THREAD_CANVAS];
        assert!(matches!(plan(bound, 32.0), ContentRepaint::Full));
        assert!(matches!(
            page.plan_repaint(32.0, false).content,
            ContentRepaint::Reuse
        ));
        assert_eq!(bound.size.get().logical(), (140, 80));
    }

    /// A canvas that left the page drops its cell, and one that replaced it
    /// binds a fresh cell. A canvas with no box binds none until it shows.
    #[wasm_bindgen_test]
    async fn a_cell_follows_its_canvas_in_and_out_of_the_page() {
        let page = page();
        let mut cells = Cells::default();
        let (hidden, _) = thread_cell(0, 0);
        cells.sync(&page);
        assert!(cells.bound.is_empty(), "a canvas with no box");
        drop(hidden);

        let (cell, canvas) = thread_cell(100, 60);
        cells.sync(&page);
        let first = Rc::clone(&cells.bound[THREAD_CANVAS].view);
        let document = web_sys::window().and_then(|w| w.document()).unwrap();
        let fresh: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        fresh
            .set_attribute("style", "display:block;width:100%;height:100%")
            .unwrap();
        canvas.remove();
        fresh.set_id(THREAD_CANVAS);
        cell.0.append_child(&fresh).unwrap();
        cells.sync(&page);
        assert!(
            !Rc::ptr_eq(&first, &cells.bound[THREAD_CANVAS].view),
            "rebound"
        );

        drop(cell);
        cells.sync(&page);
        assert!(cells.bound.is_empty(), "the canvas left");
    }
}
