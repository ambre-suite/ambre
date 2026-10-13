// ambre-app/src/host/pipe.rs
// ============================================================================
// The listeners on the transparent text pipe: keys, the buffer, the input
// method and the three clipboard events (`ambre-spec-text-input.md`,
// `ambre-spec-clipboard.md`). They reach the host through `PipeHost` alone, so
// a wasm test wires them to a recording host and raises real DOM events.
// ============================================================================

use super::*;

/// What the pipe's listeners ask of the host. `FrameHost` is the one
/// production implementation. Nothing here names an application: every call
/// addresses the active view (`UI-THREAD-025`).
pub(crate) trait PipeHost {
    /// Forward an input event to the active view. `true` when the view
    /// consumed it, so the listener prevents the browser default.
    fn handle_input(&mut self, ev: &InputEvent) -> bool;
    /// Ask the active view for the outcome of a `copy`, or of a `cut`.
    fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome;
    /// Hand a decoded paste to the active view (`UI-THREAD-024`).
    fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool);
    /// The per-page-load ownership token (`UI-THREAD-007`).
    fn clip_token(&self) -> &str;
    /// Whether the last copy's asynchronous write landed (`UI-THREAD-054`).
    fn clip_published(&self) -> Rc<std::cell::Cell<bool>>;
}

impl PipeHost for FrameHost {
    fn handle_input(&mut self, ev: &InputEvent) -> bool {
        FrameHost::handle_input(self, ev)
    }

    fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
        FrameHost::clipboard_write(self, cut)
    }

    fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
        FrameHost::paste_clipboard(self, blob, own_copy);
    }

    fn clip_token(&self) -> &str {
        FrameHost::clip_token(self)
    }

    fn clip_published(&self) -> Rc<std::cell::Cell<bool>> {
        FrameHost::clip_published(self)
    }
}

/// Serve a Paste-button read the shell decoded. The token decides rung 2 as on
/// the `paste` event, and the blob reaches the view by the same call
/// (`UI-THREAD-024`).
pub(crate) fn button_paste<H: PipeHost>(host: &mut H, paste: ClipboardPaste) {
    let own_copy = paste.token.as_deref() == Some(host.clip_token());
    host.paste_clipboard(paste.blob, own_copy);
}

/// Wire every listener the pipe needs. Keys are heard on `document`, so a
/// navigation key reaches the view while the pipe holds focus. The buffer, the
/// input method and the clipboard are heard on the pipe itself.
#[expect(
    clippy::expect_used,
    reason = "the boot wires the page it was served with, which has no fallback page"
)]
pub(crate) fn wire_pipe<H: PipeHost + 'static>(
    document: &web_sys::Document,
    text_input: &TextInput,
    host: &Rc<RefCell<H>>,
) {
    // The plain-paste chord rides the browser's own `paste` event, which says
    // nothing of the chord that raised it. The keydown notes it, and the
    // `paste` listener that follows reads and clears it (`PROSE-UI-227`).
    let plain_paste = Rc::new(std::cell::Cell::new(false));

    // -- Keyboard input -> view. ----------------------------------------------
    {
        let host_c = Rc::clone(host);
        let plain_c = Rc::clone(&plain_paste);
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
                // would commit again and navigate off it (`CHROME-020`).
                if event.target().is_some_and(|t| is_other_editable(&t, &pipe)) {
                    return;
                }
                plain_c.set(
                    event.shift_key()
                        && (event.ctrl_key() || event.meta_key())
                        && event.key().eq_ignore_ascii_case("v"),
                );
                let consumed = host_c.borrow_mut().handle_input(&InputEvent::Key {
                    key: event.key(),
                    ctrl: event.ctrl_key(),
                    alt: event.alt_key(),
                    shift: event.shift_key(),
                    meta: event.meta_key(),
                });
                // A key left unconsumed keeps its default, so its character
                // reaches the buffer (`CHROME-062`).
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

    // -- Key releases -> view. ------------------------------------------------
    // A view that arms a mode on a held key needs the release to disarm it: the
    // board's space-drag is the one today (`TOILE-UI-256`). The release is never
    // consumed, so it reaches whatever else listens.
    {
        let host_c = Rc::clone(host);
        let pipe = text_input.element().clone();
        let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                if event.target().is_some_and(|t| is_other_editable(&t, &pipe)) {
                    return;
                }
                host_c
                    .borrow_mut()
                    .handle_input(&InputEvent::KeyUp { key: event.key() });
            },
        );
        document
            .add_event_listener_with_callback("keyup", cb.as_ref().unchecked_ref())
            .expect("add keyup listener");
        cb.forget();
    }

    // -- Textarea pipe: committed text + IME composition -> view. -------------
    // `input` fires for latin typing / paste / autocomplete; outside a
    // composition we read the buffer, forward it as `Text`, and clear it so the
    // textarea never accumulates. During a composition the buffer is half-formed,
    // so we skip it and let the composition events drive the preview instead.
    {
        let host_c = Rc::clone(host);
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
    {
        let ta = text_input.element().clone();
        for cut in [false, true] {
            let host_e = Rc::clone(host);
            let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
                move |event: web_sys::ClipboardEvent| {
                    // A new copy owns the clipboard until its own asynchronous
                    // write lands (`UI-THREAD-054`).
                    host_e.borrow().clip_published().set(false);
                    let outcome = host_e.borrow_mut().clipboard_write(cut);
                    match outcome {
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
                        // (`poll_clipboard` -> `async_write_clipboard`). Until
                        // that write lands, the ownership token written here is
                        // all the clipboard holds (`UI-THREAD-007`). Prevent the
                        // default so the browser does not clobber the clipboard
                        // with the empty textarea (`UI-THREAD-008`).
                        ambre_bus::CopyOutcome::Deferred => {
                            let token = host_e.borrow().clip_token().to_owned();
                            if let Some(data) = event.clipboard_data() {
                                let _ = data.set_data(AMBRE_CLIPBOARD_TOKEN_TYPE, &token);
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
        let host_c = Rc::clone(host);
        let plain_c = Rc::clone(&plain_paste);
        let cb = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
            move |event: web_sys::ClipboardEvent| {
                let plain = plain_c.replace(false);
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
                    .filter(|s| !s.is_empty() && !plain)
                    .and_then(|b64| base64_decode(&b64));
                let text = data.get_data("text/plain").unwrap_or_default();
                // The formatting another editor wrote, for a view that reads it.
                let html = data
                    .get_data("text/html")
                    .ok()
                    .filter(|s| !s.is_empty() && !plain);
                // Same-tab ownership: the synchronous token is here while no
                // asynchronous write replaced it, or in the HTML a browser
                // refusing the `web ` types received (`UI-THREAD-056`). A match
                // means paste our own in-memory copy at full fidelity.
                let own_copy = !plain && {
                    let token = host_c.borrow().clip_token().to_owned();
                    data.get_data(AMBRE_CLIPBOARD_TOKEN_TYPE)
                        .is_ok_and(|t| !t.is_empty() && t == token)
                        || html
                            .as_deref()
                            .and_then(ambre_bus::ClipboardBlob::html_token)
                            == Some(token.as_str())
                };
                // After the last copy's asynchronous write landed, the token
                // and the private bytes sit under `web ` types the event cannot
                // see. Read them back through the asynchronous API. A refused
                // read pastes what the event carried (`UI-THREAD-054`).
                let published = host_c.borrow().clip_published().get();
                if !plain
                    && !own_copy
                    && published
                    && let Some(clipboard) = crate::platform::async_clipboard()
                {
                    event.prevent_default();
                    let fallback = ambre_bus::ClipboardBlob {
                        text,
                        html,
                        ambre,
                        plain: false,
                    };
                    let read = clipboard.read();
                    let host_a = Rc::clone(&host_c);
                    wasm_bindgen_futures::spawn_local(async move {
                        let (blob, own_copy) = match crate::platform::read_clipboard_blob(read)
                            .await
                        {
                            Some((blob, token)) => {
                                let own = token.as_deref() == Some(host_a.borrow().clip_token());
                                (blob, own)
                            }
                            None => (fallback, false),
                        };
                        host_a.borrow_mut().paste_clipboard(blob, own_copy);
                    });
                    return;
                }
                if ambre.is_none() && html.is_none() && text.is_empty() && !own_copy {
                    return;
                }
                // Prevent the native insert (which would also fire `input` and
                // double-paste); route through the host's paste path.
                event.prevent_default();
                host_c.borrow_mut().paste_clipboard(
                    ambre_bus::ClipboardBlob {
                        text,
                        html,
                        ambre,
                        plain,
                    },
                    own_copy,
                );
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
        let host_c = Rc::clone(host);
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
        let host_c = Rc::clone(host);
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
}

// -- Tests (wasm tier) -------------------------------------------------------
// Real DOM events on a real pipe, under headless Chrome. The host is a recorder,
// so each test reads what the listeners asked of it and what they did to the
// event. No GPU, no view, no Logic thread.
#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// A host that answers as the test sets it and records every call.
    #[derive(Default)]
    struct Recorder {
        /// Each input event, as a short tag and its text.
        inputs: Vec<String>,
        /// Each copy asked for: `true` for a cut.
        copies: Vec<bool>,
        pastes: Vec<(ambre_bus::ClipboardBlob, bool)>,
        /// What a copy answers. `None` answers `Nothing`.
        outcome: Option<ambre_bus::CopyOutcome>,
        /// Whether the view consumes every key.
        consume: bool,
        token: String,
        published: Rc<Cell<bool>>,
    }

    impl PipeHost for Recorder {
        fn handle_input(&mut self, ev: &InputEvent) -> bool {
            self.inputs.push(match ev {
                InputEvent::Key { key, .. } => format!("key {key}"),
                InputEvent::KeyUp { key } => format!("up {key}"),
                InputEvent::Text { text } => format!("text {text}"),
                InputEvent::CompositionUpdate { text } => format!("preview {text}"),
                InputEvent::CompositionEnd { text } => format!("commit {text}"),
                _ => "other".to_owned(),
            });
            self.consume
        }

        fn clipboard_write(&mut self, cut: bool) -> ambre_bus::CopyOutcome {
            self.copies.push(cut);
            self.outcome
                .clone()
                .unwrap_or(ambre_bus::CopyOutcome::Nothing)
        }

        fn paste_clipboard(&mut self, blob: ambre_bus::ClipboardBlob, own_copy: bool) {
            self.pastes.push((blob, own_copy));
        }

        fn clip_token(&self) -> &str {
            &self.token
        }

        fn clip_published(&self) -> Rc<Cell<bool>> {
            Rc::clone(&self.published)
        }
    }

    /// One pipe under a positioned cell of the page, wired to a recorder.
    struct Rig {
        document: web_sys::Document,
        cell: web_sys::HtmlElement,
        pipe: TextInput,
        host: Rc<RefCell<Recorder>>,
    }

    impl Rig {
        fn new() -> Self {
            let document = web_sys::window()
                .and_then(|w| w.document())
                .expect("a page");
            let cell: web_sys::HtmlElement = document
                .create_element("div")
                .expect("a cell")
                .dyn_into()
                .expect("an element");
            let _ = cell.set_attribute(
                "style",
                "position: relative; width: 400px; height: 300px; margin: 7px;",
            );
            document
                .body()
                .expect("a body")
                .append_child(&cell)
                .expect("attached");
            let pipe = TextInput::create(&document, &cell);
            let host = Rc::new(RefCell::new(Recorder {
                token: "ambre-test-token".to_owned(),
                ..Recorder::default()
            }));
            wire_pipe(&document, &pipe, &host);
            Self {
                document,
                cell,
                pipe,
                host,
            }
        }

        fn ta(&self) -> &web_sys::HtmlTextAreaElement {
            self.pipe.element()
        }

        /// Type `text` the way the browser does: the buffer fills, then
        /// `input` fires.
        fn type_text(&self, text: &str) {
            self.ta().set_value(text);
            let ev = web_sys::InputEvent::new("input").expect("an input event");
            self.ta().dispatch_event(&ev).expect("dispatched");
        }

        fn compose(&self, kind: &str, data: &str) {
            let init = web_sys::CompositionEventInit::new();
            init.set_data(data);
            let ev = web_sys::CompositionEvent::new_with_event_init_dict(kind, &init)
                .expect("a composition event");
            self.ta().dispatch_event(&ev).expect("dispatched");
        }

        /// Raise a keydown on `target`, bubbling as the browser's does. Answers
        /// whether the default survived.
        fn key_on(&self, target: &web_sys::EventTarget, key: &str) -> bool {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(key);
            init.set_bubbles(true);
            init.set_cancelable(true);
            let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .expect("a key event");
            target.dispatch_event(&ev).expect("dispatched")
        }

        /// Raise a clipboard event on the pipe carrying `data`. Answers the
        /// event, to read its default and its transfer back.
        fn clip(&self, kind: &str, data: &web_sys::DataTransfer) -> web_sys::ClipboardEvent {
            let init = web_sys::ClipboardEventInit::new();
            init.set_clipboard_data(Some(data));
            init.set_bubbles(true);
            init.set_cancelable(true);
            let ev = web_sys::ClipboardEvent::new_with_event_init_dict(kind, &init)
                .expect("a clipboard event");
            self.ta().dispatch_event(&ev).expect("dispatched");
            ev
        }

        fn inputs(&self) -> Vec<String> {
            self.host.borrow().inputs.clone()
        }
    }

    fn transfer(entries: &[(&str, &str)]) -> web_sys::DataTransfer {
        let data = web_sys::DataTransfer::new().expect("a transfer");
        for (kind, value) in entries {
            data.set_data(kind, value).expect("set");
        }
        data
    }

    /// Let the tasks a listener spawned run to their end.
    async fn settle() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .expect("a window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 100);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    /// `CHROME-061` and `CHROME-017`: a typed run reaches the view whole, in one
    /// call, and the pipe is empty after it. A second run is not the first one
    /// again.
    #[wasm_bindgen_test]
    fn a_typed_run_reaches_the_view_whole_and_the_pipe_empties() {
        let rig = Rig::new();
        rig.type_text("é");
        assert_eq!(rig.ta().value(), "");
        rig.type_text("日本");
        assert_eq!(rig.ta().value(), "");
        assert_eq!(rig.inputs(), vec!["text é", "text 日本"]);
    }

    /// `CHROME-018`, `CHROME-063`, `CHROME-019` and `CHROME-064`: while a
    /// composition runs the buffer is never read. The preview comes from the
    /// update events, the end commits the final run and empties the pipe, and
    /// typing reads the buffer again afterwards.
    #[wasm_bindgen_test]
    fn a_composition_drives_the_preview_and_ends_in_one_commit() {
        let rig = Rig::new();
        rig.compose("compositionstart", "");
        rig.type_text("n");
        rig.compose("compositionupdate", "に");
        rig.type_text("に");
        rig.compose("compositionupdate", "日本");
        rig.compose("compositionend", "日本");
        assert_eq!(rig.ta().value(), "", "the end empties the pipe");
        rig.type_text("a");
        assert_eq!(
            rig.inputs(),
            vec!["preview に", "preview 日本", "commit 日本", "text a"]
        );
    }

    /// `CHROME-020`: a key aimed at a field of the page never reaches the view,
    /// and the same key aimed at the pipe does.
    #[wasm_bindgen_test]
    fn a_key_aimed_at_another_editable_element_never_reaches_the_view() {
        let rig = Rig::new();
        for tag in ["input", "textarea", "select"] {
            let field = rig.document.create_element(tag).expect("a field");
            rig.cell.append_child(&field).expect("attached");
            rig.key_on(&field, "Enter");
        }
        let editable = rig.document.create_element("div").expect("a host");
        let _ = editable.set_attribute("contenteditable", "true");
        rig.cell.append_child(&editable).expect("attached");
        rig.key_on(&editable, "Enter");
        assert!(rig.inputs().is_empty(), "{:?}", rig.inputs());
        rig.key_on(rig.ta(), "Enter");
        assert_eq!(rig.inputs(), vec!["key Enter"]);
    }

    /// `CHROME-062`, the host half: a key the view leaves keeps its default,
    /// which is what lets its character reach the buffer. Only a consumed key
    /// loses it.
    #[wasm_bindgen_test]
    fn only_a_consumed_key_loses_its_default() {
        let rig = Rig::new();
        assert!(rig.key_on(rig.ta(), "a"), "an unconsumed key keeps its default");
        rig.host.borrow_mut().consume = true;
        assert!(!rig.key_on(rig.ta(), "ArrowDown"), "a consumed key loses it");
    }

    /// `CHROME-021`: the copy, the cut and the paste the browser raises are
    /// each serviced, with no chord decoded by the host.
    #[wasm_bindgen_test]
    fn the_three_clipboard_events_reach_the_host() {
        let rig = Rig::new();
        rig.clip("copy", &transfer(&[]));
        rig.clip("cut", &transfer(&[]));
        rig.clip("paste", &transfer(&[("text/plain", "x")]));
        let host = rig.host.borrow();
        assert_eq!(host.copies, vec![false, true]);
        assert_eq!(host.pastes.len(), 1);
        assert_eq!(host.pastes[0].0.text, "x");
        assert!(rig.inputs().is_empty(), "no chord was decoded");
    }

    /// `UI-THREAD-008`: a deferred copy writes the token and nothing else, and
    /// prevents the default. A text copy writes its mirror; a copy with nothing
    /// selected leaves the browser alone.
    #[wasm_bindgen_test]
    fn a_deferred_copy_writes_the_token_alone_and_prevents_the_default() {
        let rig = Rig::new();
        rig.host.borrow_mut().outcome = Some(ambre_bus::CopyOutcome::Deferred);
        let data = transfer(&[]);
        let ev = rig.clip("copy", &data);
        assert!(ev.default_prevented());
        assert_eq!(data.get_data("text/plain").ok().as_deref(), Some(""));
        assert_eq!(
            data.get_data(AMBRE_CLIPBOARD_TOKEN_TYPE).ok().as_deref(),
            Some("ambre-test-token")
        );

        rig.host.borrow_mut().outcome = Some(ambre_bus::CopyOutcome::Text("a\tb".to_owned()));
        let data = transfer(&[]);
        assert!(rig.clip("copy", &data).default_prevented());
        assert_eq!(data.get_data("text/plain").ok().as_deref(), Some("a\tb"));

        rig.host.borrow_mut().outcome = None;
        assert!(!rig.clip("copy", &transfer(&[])).default_prevented());
    }

    /// `UI-THREAD-024` and `UI-THREAD-025`: a paste event and a Paste-button
    /// read of the same clipboard end in one call, with the same answer to
    /// rung 2. Neither names an application: the host trait has no such
    /// parameter.
    #[wasm_bindgen_test]
    fn a_paste_event_and_the_paste_button_end_in_one_call() {
        let rig = Rig::new();
        let ev = rig.clip(
            "paste",
            &transfer(&[
                ("text/plain", "x"),
                (AMBRE_CLIPBOARD_TOKEN_TYPE, "ambre-test-token"),
            ]),
        );
        assert!(ev.default_prevented(), "the native insert is prevented");
        let blob = ambre_bus::ClipboardBlob {
            text: "x".to_owned(),
            ..ambre_bus::ClipboardBlob::default()
        };
        button_paste(
            &mut *rig.host.borrow_mut(),
            ClipboardPaste {
                blob: blob.clone(),
                token: Some("ambre-test-token".to_owned()),
            },
        );
        button_paste(
            &mut *rig.host.borrow_mut(),
            ClipboardPaste {
                blob: blob.clone(),
                token: Some("another-tab".to_owned()),
            },
        );
        let host = rig.host.borrow();
        assert_eq!(
            host.pastes,
            vec![(blob.clone(), true), (blob.clone(), true), (blob, false)]
        );
    }

    /// `UI-THREAD-054`: once the last copy's write landed, a paste reads the
    /// clipboard back instead of trusting the event. The clipboard holds what
    /// that write published, so the token read back answers rung 2 and the
    /// private bytes come with it. Before the write lands the event answers at
    /// once.
    ///
    /// Headless Chrome leaves a real read pending on its permission prompt, so
    /// the page's `read` answers the item the write would have left.
    #[wasm_bindgen_test]
    async fn a_paste_after_a_landed_write_reads_the_clipboard_back() {
        let rig = Rig::new();
        let (item, _) = publish_item(
            "x",
            None,
            Some("AQID"),
            "ambre-test-token",
            clipboard_item,
        )
        .expect("an item");
        let clipboard = crate::platform::async_clipboard().expect("the clipboard API");
        let items: JsValue = js_sys::Array::of1(&item).into();
        let read = Closure::<dyn FnMut() -> js_sys::Promise>::new(move || {
            js_sys::Promise::resolve(&items)
        });
        js_sys::Reflect::set(&clipboard, &"read".into(), read.as_ref()).expect("read replaced");

        rig.host.borrow().published.set(true);
        let ev = rig.clip("paste", &transfer(&[("text/plain", "x")]));
        assert!(ev.default_prevented());
        assert!(rig.host.borrow().pastes.is_empty(), "the event is not trusted");
        settle().await;
        let _ = js_sys::Reflect::delete_property(&clipboard, &"read".into());
        {
            let host = rig.host.borrow();
            assert_eq!(host.pastes.len(), 1, "the read answers");
            assert_eq!(host.pastes[0].0.ambre.as_deref(), Some(&[1u8, 2, 3][..]));
            assert!(host.pastes[0].1, "the token read back answers rung 2");
        }

        rig.host.borrow().published.set(false);
        rig.clip("paste", &transfer(&[("text/plain", "y")]));
        let host = rig.host.borrow();
        assert_eq!(host.pastes.len(), 2, "the event answers at once");
        assert_eq!(host.pastes[1], (blob_of("y"), false));
    }

    fn blob_of(text: &str) -> ambre_bus::ClipboardBlob {
        ambre_bus::ClipboardBlob {
            text: text.to_owned(),
            ..ambre_bus::ClipboardBlob::default()
        }
    }

    /// `UI-THREAD-047`: a paste writes nothing. The host asks for no copy, and
    /// the transfer the paste read holds what it held before.
    #[wasm_bindgen_test]
    fn a_paste_writes_nothing_to_the_clipboard() {
        let rig = Rig::new();
        let entries = [
            ("text/plain", "x"),
            (AMBRE_CLIPBOARD_TYPE, "AQID"),
            (AMBRE_CLIPBOARD_TOKEN_TYPE, "ambre-test-token"),
        ];
        let data = transfer(&entries);
        rig.clip("paste", &data);
        rig.clip("paste", &data);
        let host = rig.host.borrow();
        assert!(host.copies.is_empty());
        for (kind, value) in entries {
            assert_eq!(data.get_data(kind).ok().as_deref(), Some(value), "{kind}");
        }
        assert_eq!(host.pastes.len(), 2);
        assert_eq!(host.pastes[0], host.pastes[1], "the second paste is the first");
    }

    /// The shell's rules for the pipe, as the page loads them.
    fn load_pipe_rules(document: &web_sys::Document) {
        let css = include_str!("../../styles.css");
        let start = css.find(".ambre-input {").expect("the pipe rule");
        let end = css[start..]
            .find("/* Content a11y")
            .map_or(css.len(), |end| start + end);
        let style = document.create_element("style").expect("a style");
        style.set_text_content(Some(&css[start..end]));
        document
            .head()
            .expect("a head")
            .append_child(&style)
            .expect("attached");
    }

    /// `CHROME-007` and `CHROME-016`: the pipe lands on the point it is given,
    /// one line tall, moved by a transform. No offset is written, so the page
    /// never reflows under it.
    #[wasm_bindgen_test]
    fn the_pipe_moves_by_a_transform_to_the_point_it_is_given() {
        let rig = Rig::new();
        load_pipe_rules(&rig.document);
        rig.pipe.set_caret(40.0, 25.0, 18.0);
        let style = rig.ta().style();
        assert_eq!(
            style.get_property_value("transform").ok().as_deref(),
            Some("translate(40px, 25px)")
        );
        for offset in ["top", "left", "margin-top", "margin-left"] {
            assert_eq!(style.get_property_value(offset).ok().as_deref(), Some(""));
        }
        let cell = rig.cell.get_bounding_client_rect();
        let at = rig.ta().get_bounding_client_rect();
        assert_eq!(at.left() - cell.left(), 40.0);
        assert_eq!(at.top() - cell.top(), 25.0);
        assert_eq!(at.height(), 18.0);
    }

    /// `CHROME-059`: under the shell's rules the pipe shows no text, no caret,
    /// no background and no selection highlight.
    #[wasm_bindgen_test]
    fn the_pipe_displays_nothing() {
        let rig = Rig::new();
        load_pipe_rules(&rig.document);
        let window = web_sys::window().expect("a window");
        let clear = Some("rgba(0, 0, 0, 0)");
        let shown = window
            .get_computed_style(rig.ta())
            .ok()
            .flatten()
            .expect("a computed style");
        for property in ["color", "caret-color", "background-color"] {
            assert_eq!(shown.get_property_value(property).ok().as_deref(), clear, "{property}");
        }
        let selection = window
            .get_computed_style_with_pseudo_elt(rig.ta(), "::selection")
            .ok()
            .flatten()
            .expect("the selection style");
        for property in ["color", "background-color"] {
            assert_eq!(
                selection.get_property_value(property).ok().as_deref(),
                clear,
                "::selection {property}"
            );
        }
    }
}
