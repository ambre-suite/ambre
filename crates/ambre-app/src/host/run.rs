// ambre-app/src/host/run.rs
// ============================================================================
// `run_with` wires the canvas element, the transparent textarea input pipe, and
// the resize / input / clipboard listeners. It also wires the route-independent
// rAF loop that runs the per-frame relay + dispatcher. Returns the `DocSwitcher`
// handle the route poll uses to swap the active document in place.
// ============================================================================

use super::*;

/// Run the UI thread: build the application view and drive it through a
/// [`FrameHost`] on the requestAnimationFrame loop. DOM input and resize events
/// are forwarded to it.
///
/// `make_view` receives the canvas size in logical pixels plus the transparent
/// [`TextInput`] pipe, so the view can focus it and follow it to the caret. It
/// returns the [`AppView`] to drive. It runs on this (UI) thread before the GPU
/// is ready.
///
/// `pre_frame` runs once per animation frame immediately before the view's
/// update. The launcher pumps its route-independent relay there: the Logic<->Sync
/// relay, the dispatcher, the SPA route poll. So this driver knows nothing about
/// commands or Leptos. `raf` gates only the render (a modal / landing overlay
/// freezes the canvas), never the relay.
///
/// `to_logic` carries the one command this driver sends itself. A font face
/// fetched on demand is forwarded to the Logic thread (`CANVAS-068`). That
/// thread measures Prose pages on a context of its own, and cannot fetch.
#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "the boot wires the page it was served with, which has no fallback page"
)]
pub fn run_with<F, C, P>(
    canvas_id: ambre_core::CanvasId,
    make_view: F,
    make_child_fn: C,
    pre_frame: P,
    raf: RafControl,
    paste_rx: ambre_bus::conduit::Receiver<ClipboardPaste>,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
) -> DocSwitcher
where
    F: FnOnce(f64, f64, TextInput) -> AppView,
    C: Fn(DocumentId, DocType, Size, TextInput) -> AppView + 'static,
    P: FnMut() + 'static,
{
    tracing::info!("Ambre UI thread starting");

    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");
    let canvas: HtmlCanvasElement = document
        .get_element_by_id(canvas_id.as_str())
        .unwrap_or_else(|| panic!("no #{canvas_id} element"))
        .dyn_into()
        .expect("element is not a canvas");

    let perf = web_sys::window()
        .expect("no window")
        .performance()
        .expect("no Performance API");

    // Size the backing store to the canvas cell before the first frame. The
    // chrome (ambre-chrome AppShell) creates this `<canvas>` at its CSS layout
    // size; left untouched it would default to 300x150. Reading `client_width`
    // forces the pending layout, so the dimensions are exact. Subsequent cell
    // resizes are handled by the resize listener below.
    let size = size_backing_store(&canvas).unwrap_or(SurfaceSize::UNSIZED);

    // The transparent <textarea> pipe (keyboard + IME + clipboard). Attached
    // under the canvas's positioned cell so a `transform` caret-follow is
    // measured against the same box; it renders nothing (see text_input.rs).
    let input_parent: web_sys::Element = canvas
        .parent_node()
        .expect("canvas has a parent cell")
        .dyn_into()
        .expect("canvas parent is an Element");
    let text_input = TextInput::create(&document, &input_parent);

    // The view lays out in logical pixels (`CANVAS-032`).
    let (width, height) = size.logical();
    let view = make_view(f64::from(width), f64::from(height), text_input.clone());
    // Wrap the launcher's child factory with the shared text pipe, so `FrameHost`
    // can build a child view (`toile-embeds`) knowing nothing about the pipe. The
    // child reuses the one transparent textarea, but is constructed in embedded
    // mode so it does not steal focus from the top-level document.
    let make_child: MakeChild = {
        let child_pipe = text_input.clone();
        Rc::new(move |id, doc_type, size| make_child_fn(id, doc_type, size, child_pipe.clone()))
    };
    // Per-page-load token proving a same-tab copy. It is written synchronously
    // into the clipboard on `copy`/`cut` and read back on `paste`. A match means
    // the OS clipboard still holds this tab's own copy. The async publish
    // re-writes it (`UI-THREAD-023`), and the Paste-button path compares it in
    // `FrameHost` (`UI-THREAD-007`).
    let clip_token = ambre_bus::ClipToken::new(format!(
        "ambre-{:x}",
        (js_sys::Math::random() * 1e18) as u64
    ));
    let host = Rc::new(RefCell::new(FrameHost::new(
        view,
        perf,
        Some(make_child),
        clip_token,
        paste_rx,
        to_logic,
    )));

    // -- Async GPU init, then start the rAF loop. -----------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let raf_c = raf.clone();
        // Start the rAF loop only after the GPU is ready, so Firefox sees
        // active GPU frames from frame 1. Empty early frames make it lock to 60
        // Hz.
        wasm_bindgen_futures::spawn_local(async move {
            let (settled, gpu) = bootstrap_gpu(canvas_c, size).await;
            if let Some(gpu) = gpu {
                host_c.borrow_mut().install_gpu(gpu);
            }
            // `pre_frame` is the route-independent relay: Logic<->Sync, the
            // dispatcher, the SPA route poll. It must run EVERY frame, even when
            // the canvas is frozen behind a modal or the landing overlay. So
            // `drive_frame` gates only the render on `raf_c`.
            // A hidden page gets no frame, so a timer relays for it: the
            // threads keep hearing the relay, and a notification can rise
            // (`CANVAS-075`). One relay serves both, never at once.
            let pre_frame = Rc::new(RefCell::new(pre_frame));
            let hidden_relay = Rc::clone(&pre_frame);
            ambre_canvas::host::start_hidden_relay(
                move || (hidden_relay.borrow_mut())(),
                HIDDEN_RELAY_PERIOD_MS,
            );
            start_raf_loop(
                move |now| {
                    drive_frame(
                        &raf_c,
                        || (pre_frame.borrow_mut())(),
                        || host_c.borrow_mut().render(now),
                    )
                },
                settled,
            );
        });
    }

    // -- Keys, the buffer, the input method and the clipboard -> view. --------
    pipe::wire_pipe(&document, &text_input, &host);

    // -- Canvas resize -> host (ResizeObserver). ------------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let canvas_c2 = canvas.clone();
        let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>::new(
            move |_entries, _observer| {
                if let Some(size) = size_backing_store(&canvas_c2) {
                    host_c.borrow_mut().resize(size);
                }
            },
        );
        let observer =
            web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()).expect("ResizeObserver");
        observer.observe(&canvas_c);
        cb.forget();
    }

    // -- DPI change -> host (matchMedia). --------------------------------------
    // When the window moves between displays of different density, the canvas
    // CSS size stays the same but the density changes. ResizeObserver does not
    // fire in that case, so we watch resolution changes separately.
    //
    // `DensityWatch` re-arms itself on the new density after each change.
    {
        let host = Rc::clone(&host);
        let canvas = canvas.clone();
        DensityWatch::arm(Rc::new(move || {
            if let Some(size) = size_backing_store(&canvas) {
                host.borrow_mut().resize(size);
            }
        }));
    }

    // -- GPU context loss / restore -> host. -----------------------------------
    // A WebGL/WebGPU context can be dropped by the browser (GPU reset, the tab
    // backgrounded, driver hiccup). Without handling, the canvas stays blank
    // forever. `preventDefault` on `webglcontextlost` is REQUIRED for the
    // browser to later fire `webglcontextrestored`; on restore we reconfigure
    // the surface and force a full repaint.
    {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            tracing::warn!("webglcontextlost");
            event.prevent_default();
        });
        let _ = canvas
            .add_event_listener_with_callback("webglcontextlost", cb.as_ref().unchecked_ref());
        cb.forget();
    }
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event| {
            tracing::info!("webglcontextrestored");
            if let Some(size) = size_backing_store(&canvas_c) {
                host_c.borrow_mut().notify_context_restored(size);
            }
        });
        let _ = canvas
            .add_event_listener_with_callback("webglcontextrestored", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // -- Pointer events -> view. -----------------------------------------------
    {
        let host_c = Rc::clone(&host);

        // Positions are CSS pixels, which is what a view lays out in
        // (`CANVAS-032`). A cancel carries no reliable position, so the last one stands in.
        let mut last = (0.0, 0.0);
        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(
            move |event: web_sys::PointerEvent| {
                let cancelled = event.type_() == "pointercancel";
                let (x, y) = if cancelled {
                    last
                } else {
                    (f64::from(event.offset_x()), f64::from(event.offset_y()))
                };
                last = (x, y);
                // A finger pans the view rather than selecting.
                if event.pointer_type() == "touch" && host_c.borrow().captures_pointer() {
                    let phase = match event.type_().as_str() {
                        "pointerdown" => TouchPhase::Down,
                        "pointermove" => TouchPhase::Move,
                        "pointerup" => TouchPhase::Up,
                        "pointercancel" => TouchPhase::Cancel,
                        _ => return,
                    };
                    let now = event.time_stamp();
                    if host_c.borrow_mut().touch(
                        phase,
                        x as f32,
                        y as f32,
                        ambre_bus::PageTime(now),
                    ) {
                        event.prevent_default();
                    }
                    return;
                }
                let ev = match event.type_().as_str() {
                    "pointerdown" => InputEvent::PointerDown {
                        x,
                        y,
                        button: event.button(),
                        shift: event.shift_key(),
                        // Fold Cmd (macOS) into ctrl so disjoint-select works there.
                        ctrl: event.ctrl_key() || event.meta_key(),
                    },
                    "pointermove" => InputEvent::PointerMove { x, y },
                    // The browser took a mouse gesture over: it ends where
                    // it last was, as a release (`KINETIC-010`).
                    "pointerup" | "pointercancel" => InputEvent::PointerUp {
                        x,
                        y,
                        button: if cancelled { 0 } else { event.button() },
                        ctrl: event.ctrl_key() || event.meta_key(),
                    },
                    _ => return,
                };
                let is_down = matches!(ev, InputEvent::PointerDown { .. });
                let consumed = host_c.borrow_mut().handle_input(&ev);
                if consumed {
                    event.prevent_default();
                }
                // A drag keeps its moves past the canvas edge, so the page can
                // scroll under it (`PROSE-UI-038`). A drag across read-only
                // text leaves its press to the browser, and is captured too.
                if is_down {
                    let host = host_c.borrow();
                    if (consumed || host.drags_text())
                        && host.captures_pointer()
                        && let Some(target) = event
                            .current_target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    {
                        let _ = target.set_pointer_capture(event.pointer_id());
                    }
                }
            },
        );
        for event_type in &["pointerdown", "pointermove", "pointerup", "pointercancel"] {
            canvas
                .add_event_listener_with_callback(event_type, cb.as_ref().unchecked_ref())
                .expect("add pointer listener");
        }
        cb.forget();
    }

    // -- Dropped text -> view. -----------------------------------------------
    // `dragover` must be cancelled for the canvas to accept a drop. Only plain
    // text is read; a dropped file is left to the browser (`PROSE-UI-204`).
    {
        let host_c = Rc::clone(&host);
        let cb = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |event: web_sys::DragEvent| {
            let Some(data) = event.data_transfer() else {
                return;
            };
            if event.type_() == "dragover" {
                if data.types().includes(&"text/plain".into(), 0) {
                    event.prevent_default();
                }
                return;
            }
            let text = data.get_data("text/plain").unwrap_or_default();
            if text.is_empty() {
                return;
            }
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            if host_c.borrow_mut().drop_text(x, y, text) {
                event.prevent_default();
            }
        });
        for event_type in &["dragover", "drop"] {
            canvas
                .add_event_listener_with_callback(event_type, cb.as_ref().unchecked_ref())
                .expect("add drop listener");
        }
        cb.forget();
    }

    // -- Double-click -> view. ------------------------------------------------
    // A separate `dblclick` listener so the platform's own double-click timing
    // (the OS setting) decides what counts as a double-click -- no hand-rolled
    // threshold. `MouseEvent::offset_*` is in CSS px, like the pointer path.
    {
        let host_c = Rc::clone(&host);
        let cb =
            Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
                let ev = InputEvent::DoubleClick {
                    x: f64::from(event.offset_x()),
                    y: f64::from(event.offset_y()),
                };
                if host_c.borrow_mut().handle_input(&ev) {
                    event.prevent_default();
                }
            });
        canvas
            .add_event_listener_with_callback("dblclick", cb.as_ref().unchecked_ref())
            .expect("add dblclick listener");
        cb.forget();
    }

    // -- Wheel event -> view. -------------------------------------------------
    // The browser emits `wheel` for the mouse wheel *and* for two-finger
    // touchpad scrolling, so this single listener covers both. Deltas are in
    // CSS px (pixel mode) or lines/pages. The delta keeps its unit, and the view
    // reads it as pixels (`KINETIC-015`).
    {
        let host_c = Rc::clone(&host);
        let cb =
            Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |event: web_sys::WheelEvent| {
                let ev = InputEvent::Wheel {
                    delta: ambre_canvas::WheelDelta::new(
                        event.delta_x(),
                        event.delta_y(),
                        ambre_canvas::WheelDeltaMode::from_dom(event.delta_mode()),
                    ),
                    ctrl_key: event.ctrl_key(),
                };
                let consumed = host_c.borrow_mut().handle_input(&ev);
                if consumed {
                    event.prevent_default();
                }
            });
        canvas
            .add_event_listener_with_callback("wheel", cb.as_ref().unchecked_ref())
            .expect("add wheel listener");
        cb.forget();
    }

    tracing::info!("Ambre UI thread ready");

    // Notify the page that the UI thread is up (optional JS hook).
    if let Some(window) = web_sys::window()
        && let Ok(f) = js_sys::Reflect::get(&window, &"ambreReady".into())
        && let Ok(f) = f.dyn_into::<js_sys::Function>()
    {
        let _ = f.call0(&window);
    }

    // Hand the caller a route-driven document-swap handle. It captures the host
    // (kept alive by the rAF closure regardless), so swapping the active
    // document never touches the GPU surface.
    DocSwitcher {
        set: Rc::new(move |id| host.borrow_mut().set_document(id)),
    }
}
