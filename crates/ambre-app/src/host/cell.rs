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
use ambre_chrome::commands::{Key, KeyState, ModifierFold, ModifierKeys};
use ambre_core::{Instant, PageLoad};

/// The listeners a cell takes its input from, on its own canvas. The canvas
/// takes the focus, so its keys reach its own view (the Murmure message list
/// spec).
const INPUTS: [&str; 6] = [
    "pointerdown",
    "pointermove",
    "pointerup",
    "pointercancel",
    "wheel",
    "keydown",
];

/// A DOM listener a cell holds, with the event it listens to.
type Listener = (&'static str, Closure<dyn FnMut(web_sys::Event)>);

/// A cell's claim on the text pipe. A press, a tap or an edit loaded from a
/// menu focuses its field. Whoever takes the pipe back lets that field go.
#[derive(Clone)]
struct Claim {
    id: StaticKey<Canvases>,
    typist: Rc<std::cell::Cell<Option<StaticKey<Canvases>>>>,
}

impl Claim {
    /// Claim the pipe when the field of `view` holds the keys.
    fn note(&self, view: &AppView) {
        if view.takes_text() {
            self.typist.set(Some(self.id));
        }
    }
}

/// The cells a view shows beside its content canvas, keyed by canvas id.
#[derive(Default)]
pub(super) struct Cells {
    bound: HashMap<StaticKey<Canvases>, Bound>,
    /// The id of the cell whose field last took the keys, shared with the
    /// listeners of every cell.
    typist: Rc<std::cell::Cell<Option<StaticKey<Canvases>>>>,
    /// Whether the content view's field held the keys at the last settle.
    content_held: bool,
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
        let lookup = |id: StaticKey<Canvases>| -> Option<HtmlCanvasElement> {
            document.get_element_by_id(id.as_str())?.dyn_into().ok()
        };
        self.bound.retain(|id, cell| {
            ids.contains(id) && lookup(*id).is_some_and(|canvas| canvas == cell.canvas)
        });
        for &id in ids {
            if self.bound.contains_key(&id) {
                continue;
            }
            let Some(canvas) = lookup(id) else { continue };
            let Some(size) = size_backing_store(&canvas) else {
                continue;
            };
            if let Some(cell) = view
                .cell_view(id, size.logical())
                .and_then(|region| Bound::bind(id, canvas, size, region, &self.typist))
            {
                self.bound.insert(id, cell);
            }
        }
    }

    /// The view of the cell that holds the page's text pipe, `None` when the
    /// content view holds it. A cell holds it from the moment its field took
    /// the keys until its field lets go (`CANVAS-076`).
    pub(super) fn typist(&self) -> Option<Rc<RefCell<AppView>>> {
        let cell = self.bound.get(&self.typist.get()?)?;
        cell.view
            .borrow()
            .takes_text()
            .then(|| Rc::clone(&cell.view))
    }

    /// The content view took the text pipe back: the cell that held it lets
    /// its field go.
    pub(super) fn release(&self) {
        if let Some(view) = self.typist() {
            view.borrow_mut().leave_text();
        }
        self.typist.set(None);
    }

    /// Settle who holds the text pipe before the cells render. A content
    /// view whose field just took the keys, as an edit from a menu does,
    /// takes the pipe back. A cell that holds it takes the caret from the
    /// content view (`CANVAS-076`).
    pub(super) fn settle(&mut self, content: &mut AppView) {
        if content.takes_text() && !self.content_held {
            self.release();
        }
        if self.typist().is_some() {
            content.leave_text();
        }
        self.content_held = content.takes_text();
    }

    /// Route an event of the text pipe. A press on the content canvas takes
    /// the pipe back from the cell that held it. A key, a character or a
    /// composition goes to the cell that holds it, and answers whether the
    /// cell consumed it. `None` leaves the event to the content view.
    pub(super) fn route(&self, ev: &InputEvent) -> Option<Consumed> {
        match ev {
            InputEvent::PointerDown { .. } => {
                self.release();
                None
            }
            InputEvent::Key { .. }
            | InputEvent::KeyUp { .. }
            | InputEvent::Text { .. }
            | InputEvent::CompositionUpdate { .. }
            | InputEvent::CompositionEnd { .. } => {
                Some(self.typist()?.borrow_mut().handle_input(ev))
            }
            _ => None,
        }
    }

    /// Render every bound cell, each under its own plan (`CANVAS-070`).
    pub(super) fn render(&mut self, now: Instant<PageLoad>) {
        for cell in self.bound.values_mut() {
            cell.render(now);
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
    claim: Claim,
}

impl Bound {
    /// Bind `canvas`, measured at `size`, to `view`, and bring its GPU up.
    /// A press that focuses the view's field names `id` the `typist`. `None`
    /// when the page cannot observe the canvas box.
    fn bind(
        id: StaticKey<Canvases>,
        canvas: HtmlCanvasElement,
        size: SurfaceSize,
        view: AppView,
        typist: &Rc<std::cell::Cell<Option<StaticKey<Canvases>>>>,
    ) -> Option<Self> {
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

        let mut listeners = Vec::with_capacity(INPUTS.len() + 3);
        let claim = Claim {
            id,
            typist: Rc::clone(typist),
        };
        for kind in INPUTS {
            let view = Rc::downgrade(&view);
            let claim = claim.clone();
            let on_input =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let Some(view) = view.upgrade() else { return };
                    // A finger pans the cell's view rather than pointing.
                    if let Some((phase, at)) = touch_of(&event) {
                        if view
                            .borrow_mut()
                            .touch(phase, at, Instant::<PageLoad>::new(event.time_stamp()))
                            .is_yes()
                        {
                            event.prevent_default();
                        }
                        claim.note(&view.borrow());
                        return;
                    }
                    let Some(input) = input_of(&event) else {
                        return;
                    };
                    if view.borrow_mut().handle_input(&input).is_yes() {
                        event.prevent_default();
                        // A key the cell took is not the page's: the document
                        // listener would hand it to the content view as well.
                        event.stop_propagation();
                    }
                    // A press that focused the field takes the pipe
                    // (`CANVAS-076`).
                    claim.note(&view.borrow());
                    // A drag across the text keeps its moves past the edge.
                    if matches!(input, InputEvent::PointerDown { .. })
                        && view.borrow().drags_text()
                        && let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>()
                        && let Some(target) = event
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    {
                        let _ = target.set_pointer_capture(pointer.pointer_id());
                    }
                });
            listeners.push((kind, on_input));
        }
        // The canvas holds the focus, so a copy fires on it, not on the pipe.
        {
            let view = Rc::downgrade(&view);
            listeners.push((
                "copy",
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let Some(view) = view.upgrade() else { return };
                    let outcome = view.borrow_mut().clipboard_write(ClipboardWrite::Copy);
                    if let ambre_bus::CopyOutcome::Text(text) = outcome
                        && let Some(data) = event
                            .dyn_ref::<web_sys::ClipboardEvent>()
                            .and_then(|e| e.clipboard_data())
                    {
                        let _ = data.set_data("text/plain", &text);
                        event.prevent_default();
                        event.stop_propagation();
                    }
                }),
            ));
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
            claim,
        })
    }

    /// Measure the canvas against its cell, and resize the view to it.
    fn measure(&self) {
        measure(&self.canvas, &self.view, &self.size);
    }

    /// Plan, paint and present this cell. Nothing runs before its GPU is up,
    /// so a plan is never spent on a canvas that cannot show it. Answers the
    /// plan it presented.
    fn render(&mut self, now: Instant<PageLoad>) -> Option<RepaintPlan<Px<Device>>> {
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
        let relayout = if refit || faces {
            Relayout::Again
        } else {
            Relayout::Kept
        };
        let plan = plan_frame(&mut view, size, relayout, now);
        // An edit asked from a menu loads the field as its plan runs.
        self.claim.note(&view);
        let phases = Phases::of(plan.content, plan.overlay);
        if phases.any_build() {
            gpu.text_cache.reset_frame_stats();
            gpu.quad_renderer.begin_frame(gpu.size);
            let mut surface = WgpuSurface::new(&mut gpu.quad_renderer, &gpu.device, &gpu.queue);
            if phases.content == LayerRepaint::Redraw {
                surface.begin_layer(Layer::Content);
                view.paint_content(
                    &mut surface,
                    &mut gpu.font_cx,
                    &mut gpu.text_cache,
                    plan.content_strip,
                );
            }
            if phases.overlay == LayerRepaint::Redraw {
                surface.begin_layer(Layer::Overlay);
                view.paint_overlay(
                    &mut surface,
                    &mut gpu.font_cx,
                    &mut gpu.text_cache,
                    &FrameStats::default(),
                );
            }
        }
        if phases.present == ambre_canvas::Present::Run {
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
        view.borrow_mut().resize(measured.logical());
    }
}

/// Whether a cell's view is laid out again before its frame is planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Relayout {
    Kept,
    Again,
}

/// Plan the frame of a cell's `view` at `size`, the size its GPU is fitted
/// to. [`Relayout::Again`] lays the view out again first. The plan answers in the
/// physical pixels of the canvas.
fn plan_frame(
    view: &mut AppView,
    size: SurfaceSize,
    relayout: Relayout,
    now: Instant<PageLoad>,
) -> RepaintPlan<Px<Device>> {
    if relayout == Relayout::Again {
        view.resize(size.logical());
    }
    size.on_device(view.plan_repaint(now, ambre_canvas::HudTick::Idle))
}

/// The phase and the point of a finger on a cell, in its own pixels. `None`
/// for a mouse, a pen or a wheel.
fn touch_of(event: &web_sys::Event) -> Option<(TouchPhase, ambre_core::Point<Px<View>>)> {
    let pointer = event.dyn_ref::<web_sys::PointerEvent>()?;
    if pointer.pointer_type() != "touch" {
        return None;
    }
    let phase = match event.type_().as_str() {
        "pointerdown" => TouchPhase::Down,
        "pointermove" => TouchPhase::Move,
        "pointerup" => TouchPhase::Up,
        "pointercancel" => TouchPhase::Cancel,
        _ => return None,
    };
    Some((phase, super::offset_of(pointer)))
}

/// The input event a DOM event on a cell carries, in the cell's own pixels.
fn input_of(event: &web_sys::Event) -> Option<InputEvent> {
    if let Some(key) = event.dyn_ref::<web_sys::KeyboardEvent>() {
        // A key the chord set does not model is no input of the cell.
        return Some(InputEvent::Key {
            key: Key::from_dom(&key.key())?,
            mods: ModifierKeys::NONE
                .ctrl_if(KeyState::from_bool(key.ctrl_key()))
                .alt_if(KeyState::from_bool(key.alt_key()))
                .shift_if(KeyState::from_bool(key.shift_key()))
                .meta_if(KeyState::from_bool(key.meta_key())),
        });
    }
    if let Some(wheel) = event.dyn_ref::<web_sys::WheelEvent>() {
        return Some(InputEvent::Wheel {
            delta: ambre_canvas::WheelDelta::new(
                ambre_core::Vector::new(
                    ambre_canvas::WheelAmount::new(wheel.delta_x()),
                    ambre_canvas::WheelAmount::new(wheel.delta_y()),
                ),
                ambre_canvas::WheelDeltaMode::from_dom(wheel.delta_mode()),
            ),
            mods: ModifierKeys::NONE.ctrl_if(KeyState::from_bool(wheel.ctrl_key())),
        });
    }
    let pointer = event.dyn_ref::<web_sys::PointerEvent>()?;
    let at = super::offset_of(pointer);
    let mods = ModifierKeys::NONE
        .ctrl_if(KeyState::from_bool(pointer.ctrl_key()))
        .alt_if(KeyState::from_bool(pointer.alt_key()))
        .shift_if(KeyState::from_bool(pointer.shift_key()))
        .meta_if(KeyState::from_bool(pointer.meta_key()));
    Some(match event.type_().as_str() {
        "pointerdown" => InputEvent::PointerDown {
            at,
            button: ambre_canvas::PointerButton::from_dom(pointer.button()),
            mods,
        },
        "pointermove" => InputEvent::PointerMove { at },
        "pointerup" => InputEvent::PointerUp {
            at,
            button: ambre_canvas::PointerButton::from_dom(pointer.button()),
            mods,
        },
        // The browser took the gesture over: it ends where it was, as a
        // release (`KINETIC-010`).
        "pointercancel" => InputEvent::PointerUp {
            at,
            button: ambre_canvas::PointerButton::Primary,
            mods,
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
pub(super) fn present(gpu: &mut GpuState, plan: &RepaintPlan<Px<Device>>) -> bool {
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
    use ambre_bus::{Instant, PageLoad};
    use ambre_canvas::ThemeControl;
    use ambre_chrome::commands::KeyChar;
    use ambre_core::Px;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// The size of a region, in its view pixels.
    fn view_size(width: f32, height: f32) -> ambre_core::Size<ambre_core::Px<ambre_core::View>> {
        ambre_core::Size::new(
            ambre_core::Px::<ambre_core::View>::new(width),
            ambre_core::Px::<ambre_core::View>::new(height),
        )
    }

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
        canvas.set_id(THREAD_CANVAS.as_str());
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
            ambre_canvas::BusyPublisher::new(),
            crate::host::page_reads(
                ambre_murmure::chrome::MurmureRelay::new(),
                TextInput::detached(),
                std::rc::Rc::new(|_| {}),
            ),
            ambre_core::Size::new(Px::new(300), Px::new(200)),
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

    /// A pointer at canvas `(x, y)`, as the host hands it over.
    fn pointer(x: f64, y: f64) -> ambre_core::Point<ambre_core::Px<ambre_core::View>> {
        ambre_core::Point::new(
            ambre_core::Px::<ambre_core::View>::new(x as f32),
            ambre_core::Px::<ambre_core::View>::new(y as f32),
        )
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
        let bound = &cells.bound[&THREAD_CANVAS];
        assert_eq!(murmure(&bound.view.borrow()).size(), view_size(160.0, 90.0));
        assert_ne!(
            (canvas.width(), canvas.height()),
            (300, 150),
            "the backing store fits the cell, not the default"
        );

        let first = Rc::clone(&bound.view);
        cells.sync(&page);
        assert!(
            Rc::ptr_eq(&first, &cells.bound[&THREAD_CANVAS].view),
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
        assert_eq!(murmure(&first.borrow()).size(), view_size(200.0, 120.0));
    }

    /// A finger on a cell holds its list rather than pointing at it, and a
    /// mouse still points.
    #[wasm_bindgen_test]
    fn a_finger_on_a_cell_holds_its_list_and_points_at_nothing() {
        let (_cell, canvas) = thread_cell(160, 90);
        let page = page();
        let mut cells = Cells::default();
        cells.sync(&page);
        let view = Rc::clone(&cells.bound[&THREAD_CANVAS].view);
        let finger = |kind: &str| {
            let init = web_sys::PointerEventInit::new();
            init.set_pointer_type("touch");
            web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap()
        };

        canvas.dispatch_event(&finger("pointerdown")).unwrap();
        assert!(murmure(&view.borrow()).finger_down());
        canvas.dispatch_event(&finger("pointermove")).unwrap();
        assert!(murmure(&view.borrow()).hover().is_none(), "no hover");
        canvas.dispatch_event(&finger("pointerup")).unwrap();
        assert!(!murmure(&view.borrow()).finger_down());

        canvas.dispatch_event(&finger("pointerdown")).unwrap();
        canvas.dispatch_event(&finger("pointercancel")).unwrap();
        assert!(!murmure(&view.borrow()).finger_down(), "cancelled");

        let mouse = web_sys::PointerEvent::new("pointermove").unwrap();
        canvas.dispatch_event(&mouse).unwrap();
        assert!(murmure(&view.borrow()).hover().is_some(), "a mouse points");
    }

    /// A key on the thread's canvas reaches its own view, and one it took
    /// goes no further, so the channel never hears it. A key it leaves still
    /// reaches the page (the Murmure message list spec).
    #[wasm_bindgen_test]
    fn a_key_on_a_cell_reaches_its_view_and_not_the_page() {
        let (_cell, canvas) = thread_cell(160, 90);
        let page = page();
        let mut cells = Cells::default();
        cells.sync(&page);
        // The body hears a key before the document, whose listeners other
        // tests leave behind: the probe reads the key as the cell left it.
        let body = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
            .unwrap();
        let heard = Rc::new(std::cell::Cell::new(None));
        let sink = Rc::clone(&heard);
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            sink.set(Some(event.default_prevented()));
        });
        let callback = listener.as_ref().unchecked_ref();
        body.add_event_listener_with_callback("keydown", callback)
            .unwrap();
        let press = |key: &str| {
            heard.set(None);
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(key);
            init.set_bubbles(true);
            init.set_cancelable(true);
            let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .unwrap();
            canvas.dispatch_event(&event).unwrap();
            event.default_prevented()
        };

        assert!(press("PageUp"), "the thread took the key");
        assert_eq!(heard.get(), None, "the page never heard it");
        assert!(press("ArrowUp"));
        assert_eq!(heard.get(), None, "nor this one");
        press("a");
        assert_eq!(
            heard.get(),
            Some(false),
            "a key the thread leaves reaches the page"
        );
        body.remove_event_listener_with_callback("keydown", callback)
            .unwrap();
    }

    /// One frame of `view` on a recording surface. No face is shipped to the
    /// test, so the text shapes with no width: the host tier holds the bytes.
    fn paint(view: &mut MurmureView) {
        let mut font_cx = ambre_canvas::fonts::font_context_from_bytes(Vec::new());
        let mut cache = ambre_canvas::TextCache::new();
        let mut surface = ambre_draw::testing::RecordingSurface::new();
        view.update();
        view.plan_repaint(Instant::<PageLoad>::new(0.0), ambre_canvas::HudTick::Idle);
        view.paint_content(&mut surface, &mut font_cx, &mut cache, None);
    }

    /// A press on a cell's text starts a drag the cell keeps, and a copy on
    /// its canvas writes the selected text. With nothing selected, the copy
    /// is the browser's (the Murmure message list spec).
    #[wasm_bindgen_test]
    fn a_copy_on_a_cell_writes_the_text_it_selects() {
        let (_cell, canvas) = thread_cell(300, 200);
        let (mut view, _events) = ambre_murmure::ui::testing::channel_of(
            &["hello", "world"],
            ambre_core::Size::new(Px::new(300), Px::new(200)),
        );
        paint(&mut view);
        let size = size_backing_store(&canvas).unwrap();
        let typist = Rc::default();
        let bound = Bound::bind(
            THREAD_CANVAS,
            canvas.clone(),
            size,
            AppView::Murmure(view),
            &typist,
        )
        .unwrap();
        let copy = || {
            let init = web_sys::ClipboardEventInit::new();
            init.set_cancelable(true);
            init.set_clipboard_data(Some(&web_sys::DataTransfer::new().unwrap()));
            let event = web_sys::ClipboardEvent::new_with_event_init_dict("copy", &init).unwrap();
            canvas.dispatch_event(&event).unwrap();
            let text = event.clipboard_data().unwrap().get_data("text/plain");
            (event.default_prevented(), text.unwrap_or_default())
        };
        assert_eq!(copy(), (false, String::new()), "nothing selected");

        let down = web_sys::PointerEvent::new("pointerdown").unwrap();
        canvas.dispatch_event(&down).unwrap();
        assert!(murmure(&bound.view.borrow()).drags_text(), "a drag begins");
        let up = web_sys::PointerEvent::new("pointerup").unwrap();
        canvas.dispatch_event(&up).unwrap();

        let rows = {
            let mut view = bound.view.borrow_mut();
            let AppView::Murmure(v) = &mut *view else {
                panic!("a Murmure view")
            };
            paint(v);
            v.last_painted().unwrap().rows
        };
        let [first, .., last] = rows.as_slice() else {
            panic!("two rows: {rows:?}");
        };
        let (top, bottom) = (first.bounds.top.get(), last.bounds.bottom.get());
        let mut view = bound.view.borrow_mut();
        for input in [
            InputEvent::PointerDown {
                at: pointer(0.0, f64::from(top)),
                button: ambre_canvas::PointerButton::Primary,
                mods: ModifierKeys::NONE,
            },
            InputEvent::PointerMove {
                at: pointer(299.0, f64::from(bottom - 1.0)),
            },
        ] {
            let _ = view.handle_input(&input);
        }
        let AppView::Murmure(v) = &mut *view else {
            panic!("a Murmure view")
        };
        paint(v);
        drop(view);
        let (prevented, text) = copy();
        assert!(prevented, "the cell wrote the clipboard");
        assert!(
            text.starts_with("Ada, t\n") && text.ends_with("world"),
            "{text}"
        );
        let cut = bound.view.borrow_mut().clipboard_write(ClipboardWrite::Cut);
        assert!(
            matches!(cut, ambre_bus::CopyOutcome::Nothing),
            "a cut leaves read-only text"
        );
    }

    /// A point in the field of `view`, a view `height` pixels tall.
    fn field_of(view: &MurmureView, height: f64) -> (f64, f64) {
        (40.0, height - f64::from(view.composer().band().get()) / 2.0)
    }

    /// `CANVAS-076`: the text pipe writes into the cell whose field took the
    /// keys, and the content view's field lets them go. A press on the
    /// content canvas, or its field taking the keys again, takes the pipe
    /// back.
    #[wasm_bindgen_test]
    fn the_pipe_writes_into_the_cell_whose_field_took_the_keys() {
        let (_cell, canvas) = thread_cell(300, 200);
        let (mut thread, _thread_events) = ambre_murmure::ui::testing::channel_of(
            &["hello"],
            ambre_core::Size::new(Px::new(300), Px::new(200)),
        );
        paint(&mut thread);
        let (x, y) = field_of(&thread, 200.0);
        let (mut channel, _channel_events) = ambre_murmure::ui::testing::channel_of(
            &["hi"],
            ambre_core::Size::new(Px::new(300), Px::new(200)),
        );
        paint(&mut channel);
        let mut content = AppView::Murmure(channel);
        let press_content = |content: &mut AppView| {
            let (cx, cy) = match content {
                AppView::Murmure(v) => field_of(v, 200.0),
                _ => panic!("a Murmure view"),
            };
            let _ = content.handle_input(&InputEvent::PointerDown {
                at: pointer(cx, cy),
                button: ambre_canvas::PointerButton::Primary,
                mods: ModifierKeys::NONE,
            });
        };
        let mut cells = Cells::default();
        let size = size_backing_store(&canvas).unwrap();
        let bound = Bound::bind(
            THREAD_CANVAS,
            canvas.clone(),
            size,
            AppView::Murmure(thread),
            &cells.typist,
        )
        .unwrap();
        let view = Rc::clone(&bound.view);
        cells.bound.insert(THREAD_CANVAS, bound);
        let typed = InputEvent::Text {
            text: "yo".to_owned(),
        };

        press_content(&mut content);
        cells.settle(&mut content);
        assert!(content.takes_text(), "the content's field holds the keys");
        assert_eq!(cells.route(&typed), None, "no cell holds the pipe");

        let rect = canvas.get_bounding_client_rect();
        let init = web_sys::PointerEventInit::new();
        init.set_client_x((rect.left() + x) as i32);
        init.set_client_y((rect.top() + y) as i32);
        let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
        canvas.dispatch_event(&down).unwrap();
        assert!(cells.typist().is_some(), "the cell's field took the keys");
        cells.settle(&mut content);
        assert!(!content.takes_text(), "the content's field let them go");
        assert_eq!(
            cells.route(&typed),
            Some(Consumed::Yes),
            "the cell takes the text"
        );
        let key = InputEvent::Key {
            key: Key::Char(KeyChar::A),
            mods: ModifierKeys::NONE,
        };
        assert!(cells.route(&key).is_some(), "and the keys");

        press_content(&mut content);
        cells.settle(&mut content);
        assert!(cells.typist().is_none(), "the content's field took it back");
        assert!(!view.borrow().takes_text(), "the cell's field let go");

        canvas.dispatch_event(&down).unwrap();
        assert!(cells.typist().is_some());
        let press = InputEvent::PointerDown {
            at: pointer(10.0, 10.0),
            button: ambre_canvas::PointerButton::Primary,
            mods: ModifierKeys::NONE,
        };
        assert_eq!(cells.route(&press), None, "the press is the content's");
        assert!(cells.typist().is_none(), "a press on the content takes it");
        assert!(!view.borrow().takes_text());
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
        let bound = cells.bound.get_mut(&THREAD_CANVAS).unwrap();
        assert!(
            bound.render(Instant::<PageLoad>::new(0.0)).is_none(),
            "no GPU yet"
        );

        let plan = |bound: &Bound, now: Instant<PageLoad>| {
            plan_frame(
                &mut bound.view.borrow_mut(),
                bound.size.get(),
                Relayout::Kept,
                now,
            )
            .content
        };
        assert!(matches!(
            plan(bound, Instant::<PageLoad>::new(0.0)),
            ContentRepaint::Full
        ));
        assert!(matches!(
            plan(bound, Instant::<PageLoad>::new(16.0)),
            ContentRepaint::Reuse
        ));
        assert!(
            matches!(
                page.plan_repaint(Instant::<PageLoad>::new(16.0), ambre_canvas::HudTick::Idle)
                    .content,
                ContentRepaint::Full
            ),
            "the page keeps its own plan"
        );

        resize(&cell.0, 140, 80);
        sleep(50).await;
        let bound = &cells.bound[&THREAD_CANVAS];
        assert!(matches!(
            plan(bound, Instant::<PageLoad>::new(32.0)),
            ContentRepaint::Full
        ));
        assert!(matches!(
            page.plan_repaint(Instant::<PageLoad>::new(32.0), ambre_canvas::HudTick::Idle)
                .content,
            ContentRepaint::Reuse
        ));
        assert_eq!(
            bound.size.get().logical(),
            ambre_core::Size::new(Px::new(140), Px::new(80))
        );
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
        let first = Rc::clone(&cells.bound[&THREAD_CANVAS].view);
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
        fresh.set_id(THREAD_CANVAS.as_str());
        cell.0.append_child(&fresh).unwrap();
        cells.sync(&page);
        assert!(
            !Rc::ptr_eq(&first, &cells.bound[&THREAD_CANVAS].view),
            "rebound"
        );

        drop(cell);
        cells.sync(&page);
        assert!(cells.bound.is_empty(), "the canvas left");
    }
}
