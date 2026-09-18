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
/// `make_view` receives the canvas size in physical pixels plus the transparent
/// [`TextInput`] pipe, so the view can focus it and follow it to the caret. It
/// returns the [`AppView`] to drive. It runs on this (UI) thread before the GPU
/// is ready.
///
/// `pre_frame` runs once per animation frame immediately before the view's
/// update. The launcher pumps its route-independent relay there: the Logic<->Sync
/// relay, the dispatcher, the SPA route poll. So this driver knows nothing about
/// commands or Leptos. `raf` gates only the render (a modal / landing overlay
/// freezes the canvas), never the relay.
#[allow(clippy::too_many_arguments)]
pub fn run_with<F, C, P>(
    canvas_id: &str,
    make_view: F,
    make_child_fn: C,
    mut pre_frame: P,
    raf: RafControl,
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
        .get_element_by_id(canvas_id)
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
    size_backing_store(&canvas);

    // The transparent <textarea> pipe (keyboard + IME + clipboard). Attached
    // under the canvas's positioned cell so a `transform` caret-follow is
    // measured against the same box; it renders nothing (see text_input.rs).
    let input_parent: web_sys::Element = canvas
        .parent_node()
        .expect("canvas has a parent cell")
        .dyn_into()
        .expect("canvas parent is an Element");
    let text_input = TextInput::create(&document, &input_parent);

    let view = make_view(
        canvas.width() as f64,
        canvas.height() as f64,
        text_input.clone(),
    );
    // Wrap the launcher's child factory with the shared text pipe, so `FrameHost`
    // can build a child view (`toile-embeds`) knowing nothing about the pipe. The
    // child reuses the one transparent textarea, but is constructed in embedded
    // mode so it does not steal focus from the top-level document.
    let make_child: MakeChild = {
        let child_pipe = text_input.clone();
        Rc::new(move |id, doc_type, size| make_child_fn(id, doc_type, size, child_pipe.clone()))
    };
    let host = Rc::new(RefCell::new(FrameHost::new(view, perf, Some(make_child))));

    // -- Async GPU init, then start the rAF loop. -----------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let raf_c = raf.clone();
        // Start the rAF loop only after the GPU is ready, so Firefox sees
        // active GPU frames from frame 1. Empty early frames make it lock to 60
        // Hz.
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(gpu) = bootstrap_gpu(canvas_c).await {
                host_c.borrow_mut().install_gpu(gpu);
            }
            // `pre_frame` is the route-independent relay: Logic<->Sync, the
            // dispatcher, the SPA route poll. It must run EVERY frame, even when
            // the canvas is frozen behind a modal or the landing overlay. So the
            // outer loop is always-on and only the render is gated by `raf_c`.
            start_raf_loop(
                move |ts| {
                    pre_frame();
                    if raf_c.is_running() {
                        host_c.borrow_mut().render(ts);
                    }
                },
                RafControl::new(),
            );
        });
    }

    // -- Keyboard input -> view. ----------------------------------------------
    {
        let host_c = Rc::clone(&host);
        // The canvas text pipe: the one editable element whose keydowns the grid
        // owns. Navigation/edit keys ride the document listener even while it is
        // focused, since text itself arrives via the textarea `input` event.
        let pipe = text_input.element().clone();
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                // Ignore keys aimed at any OTHER editable DOM element -- the HTML
                // formula bar input and the dialog fields. Those handle their own
                // keyboard; forwarding them to the grid would double-process the
                // event. A formula-bar Enter would commit the cell, then the grid
                // would commit again and navigate off it.
                if event.target().is_some_and(|t| is_other_editable(&t, &pipe)) {
                    return;
                }
                let consumed = host_c.borrow_mut().handle_input(&InputEvent::Key {
                    key: event.key(),
                    ctrl: event.ctrl_key(),
                    alt: event.alt_key(),
                    shift: event.shift_key(),
                    meta: event.meta_key(),
                });
                if consumed {
                    event.prevent_default();
                }
            },
        );
        document
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
            .expect("add keydown listener");
        cb.forget();
    }

    // -- Textarea pipe: committed text + IME composition -> view. -------------
    // `input` fires for latin typing / paste / autocomplete; outside a
    // composition we read the buffer, forward it as `Text`, and clear it so the
    // textarea never accumulates. During a composition the buffer is half-formed,
    // so we skip it and let the composition events drive the preview instead.
    {
        let host_c = Rc::clone(&host);
        let composing = text_input.composing();
        let ta = text_input.element().clone();
        let ta_read = ta.clone();
        let cb = Closure::<dyn FnMut(web_sys::InputEvent)>::new(move |_event| {
            if composing.get() {
                return;
            }
            let value = ta_read.value();
            if !value.is_empty() {
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::Text { text: value });
                ta_read.set_value("");
            }
        });
        ta.add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
            .expect("add input listener");
        cb.forget();
    }

    // -- Clipboard: copy / cut / paste on the focused textarea pipe. -----------
    // The browser maps Ctrl+C / Ctrl+Insert -> `copy`, Ctrl+X / Shift+Del ->
    // `cut`, Ctrl+V / Shift+Insert -> `paste`, so we never decode those chords
    // ourselves: we just service the three clipboard events. `copy`/`cut` write
    // the in-cell text selection or the grid range synchronously into
    // `clipboardData`; `paste` reads it back. A grid range carries two
    // representations: tab-separated text (`text/plain`, for cross-app paste) and
    // a base64 postcard `ClipboardPayload` under a private type
    // (`ambre-spec-clipboard.md`), so a round-trip preserves styles and reindexes
    // formulas.
    // Per-page-load token proving a same-tab copy. Written synchronously into the
    // clipboard on `copy`/`cut` and read back on `paste`; a match means the OS
    // clipboard still holds this tab's own copy (see `AMBRE_CLIPBOARD_TOKEN_TYPE`).
    let clip_token: Rc<str> = Rc::from(format!(
        "ambre-{:x}",
        (js_sys::Math::random() * 1e18) as u64
    ));
    {
        let ta = text_input.element().clone();
        let host_c = Rc::clone(&host);
        let token = Rc::clone(&clip_token);
        for cut in [false, true] {
            let host_e = Rc::clone(&host_c);
            let token_e = Rc::clone(&token);
            let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
                move |event: web_sys::ClipboardEvent| {
                    match host_e.borrow_mut().clipboard_write(cut) {
                        // Nothing selected: let the browser default proceed.
                        ambre_bus::CopyOutcome::Nothing => {}
                        // Synchronous in-cell / inline text selection: write it
                        // straight into the event's clipboardData, as before.
                        ambre_bus::CopyOutcome::Text(text) => {
                            if let Some(data) = event.clipboard_data() {
                                let _ = data.set_data("text/plain", &text);
                                event.prevent_default();
                            }
                        }
                        // Authoritative copy: the full payload is built on the Logic
                        // thread and mirrored to the OS clipboard a frame later
                        // (`poll_clipboard` -> `async_write_clipboard`). That async
                        // write needs a user-activation context Firefox denies from
                        // the rAF loop, so it is best-effort. To make a same-tab
                        // paste reliable regardless, we ALSO write the ownership
                        // token synchronously here -- this always reaches the OS
                        // clipboard. Prevent the default so the browser does not
                        // clobber the clipboard with the empty textarea.
                        ambre_bus::CopyOutcome::Deferred => {
                            if let Some(data) = event.clipboard_data() {
                                let _ = data.set_data(AMBRE_CLIPBOARD_TOKEN_TYPE, &token_e);
                            }
                            event.prevent_default();
                        }
                    }
                },
            );
            ta.add_event_listener_with_callback(
                if cut { "cut" } else { "copy" },
                cb.as_ref().unchecked_ref(),
            )
            .expect("add clipboard listener");
            cb.forget();
        }
    }
    {
        let ta = text_input.element().clone();
        let host_c = Rc::clone(&host);
        let token = Rc::clone(&clip_token);
        let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
            move |event: web_sys::ClipboardEvent| {
                let Some(data) = event.clipboard_data() else {
                    return;
                };
                // Decode the shared transport blob. That is the opaque Ambre
                // bytes under the private type: app-specific, an internal
                // round-trip. It also carries the `text/plain` mirror, for
                // cross-app use or a browser that dropped the custom type. The
                // view decides which to honor.
                let ambre = data
                    .get_data(AMBRE_CLIPBOARD_TYPE)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .and_then(|b64| base64_decode(&b64));
                let text = data.get_data("text/plain").unwrap_or_default();
                // Same-tab ownership: the synchronous token survives even when the
                // async payload write failed (Firefox). So a match means paste our
                // own in-memory copy at full fidelity.
                let own_copy = data
                    .get_data(AMBRE_CLIPBOARD_TOKEN_TYPE)
                    .is_ok_and(|t| !t.is_empty() && t.as_str() == &*token);
                if ambre.is_none() && text.is_empty() && !own_copy {
                    return;
                }
                // Prevent the native insert (which would also fire `input` and
                // double-paste); route through the host's paste path.
                event.prevent_default();
                host_c
                    .borrow_mut()
                    .paste_clipboard(ambre_bus::ClipboardBlob { text, ambre }, own_copy);
            },
        );
        ta.add_event_listener_with_callback("paste", cb.as_ref().unchecked_ref())
            .expect("add paste listener");
        cb.forget();
    }

    // compositionstart: enter the composing window (gate the `input` listener).
    {
        let composing = text_input.composing();
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(move |_event| {
            composing.set(true);
        });
        text_input
            .element()
            .add_event_listener_with_callback("compositionstart", cb.as_ref().unchecked_ref())
            .expect("add compositionstart listener");
        cb.forget();
    }

    // compositionupdate: forward the running preview; canvas renders it styled.
    {
        let host_c = Rc::clone(&host);
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(
            move |event: web_sys::CompositionEvent| {
                let text = event.data().unwrap_or_default();
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::CompositionUpdate { text });
            },
        );
        text_input
            .element()
            .add_event_listener_with_callback("compositionupdate", cb.as_ref().unchecked_ref())
            .expect("add compositionupdate listener");
        cb.forget();
    }

    // compositionend: commit the final text, leave the composing window, clear.
    {
        let host_c = Rc::clone(&host);
        let composing = text_input.composing();
        let ta = text_input.element().clone();
        let cb = Closure::<dyn FnMut(web_sys::CompositionEvent)>::new(
            move |event: web_sys::CompositionEvent| {
                composing.set(false);
                let text = event.data().unwrap_or_default();
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::CompositionEnd { text });
                ta.set_value("");
            },
        );
        text_input
            .element()
            .add_event_listener_with_callback("compositionend", cb.as_ref().unchecked_ref())
            .expect("add compositionend listener");
        cb.forget();
    }

    // -- Canvas resize -> host (ResizeObserver). ------------------------------
    {
        let host_c = Rc::clone(&host);
        let canvas_c = canvas.clone();
        let canvas_c2 = canvas.clone();
        let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>::new(
            move |_entries, _observer| {
                if let Some((w, h)) = size_backing_store(&canvas_c2) {
                    host_c.borrow_mut().resize(w, h);
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
    // CSS size stays the same but devicePixelRatio changes. ResizeObserver does
    // not fire in that case, so we watch resolution changes separately.
    //
    // A `MediaQueryList` only fires `change` when it crosses ITS OWN threshold,
    // so the old `match_media("(resolution)")` -- a feature-existence query with
    // no threshold -- never fired. The correct pattern is to query the CURRENT
    // `Ndppx` and re-arm a fresh listener after each change, since the new DPR
    // needs a new query. `arm_dpi_watch` does exactly that.
    fn arm_dpi_watch(host: Rc<RefCell<FrameHost>>, canvas: HtmlCanvasElement) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let dpr = window.device_pixel_ratio();
        let query = format!("(resolution: {dpr}dppx)");
        let Ok(Some(mql)) = window.match_media(&query) else {
            return;
        };
        let cb = Closure::once_into_js(move |_event: web_sys::MediaQueryListEvent| {
            if let Some((w, h)) = size_backing_store(&canvas) {
                host.borrow_mut().resize(w, h);
            }
            // Re-arm for the new device pixel ratio: this listener has fired and
            // its query no longer matches the current DPR.
            arm_dpi_watch(host.clone(), canvas.clone());
        });
        let _ = mql.add_event_listener_with_callback("change", cb.unchecked_ref());
    }
    arm_dpi_watch(Rc::clone(&host), canvas.clone());

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
            if let Some((w, h)) = size_backing_store(&canvas_c) {
                host_c.borrow_mut().notify_context_restored(w, h);
            }
        });
        let _ = canvas
            .add_event_listener_with_callback("webglcontextrestored", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // -- Pointer events -> view. -----------------------------------------------
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0);

        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(
            move |event: web_sys::PointerEvent| {
                let x = event.offset_x() as f64 * dpr;
                let y = event.offset_y() as f64 * dpr;
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
                    "pointerup" => InputEvent::PointerUp {
                        x,
                        y,
                        button: event.button(),
                        ctrl: event.ctrl_key() || event.meta_key(),
                    },
                    _ => return,
                };
                let consumed = host_c.borrow_mut().handle_input(&ev);
                if consumed {
                    event.prevent_default();
                }
            },
        );
        for event_type in &["pointerdown", "pointermove", "pointerup"] {
            canvas
                .add_event_listener_with_callback(event_type, cb.as_ref().unchecked_ref())
                .expect("add pointer listener");
        }
        cb.forget();
    }

    // -- Double-click -> view. ------------------------------------------------
    // A separate `dblclick` listener so the platform's own double-click timing
    // (the OS setting) decides what counts as a double-click -- no hand-rolled
    // threshold. `MouseEvent::offset_*` is in CSS px; scale to physical like the
    // pointer path.
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0);
        let cb =
            Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
                let ev = InputEvent::DoubleClick {
                    x: event.offset_x() as f64 * dpr,
                    y: event.offset_y() as f64 * dpr,
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
    // CSS px (pixel mode) or lines/pages; scale to physical px like the pointer
    // path and forward `delta_mode` so the view can normalise lines/pages.
    {
        let host_c = Rc::clone(&host);
        let dpr = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0);
        let cb =
            Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |event: web_sys::WheelEvent| {
                let ev = InputEvent::Wheel {
                    dx: event.delta_x() * dpr,
                    dy: event.delta_y() * dpr,
                    delta_mode: event.delta_mode(),
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
