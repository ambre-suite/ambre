// ambre-app/src/platform.rs
// ============================================================================
// Leaf platform helpers for the launcher: command dispatch, Sync-worker spawn /
// postMessage, theme injection, dev-login URL/localStorage plumbing, the canvas
// context-menu and keyboard installers, and small formatting utilities. All
// pure glue with no reactive state of their own.
// ============================================================================

use super::*;

/// Convert a canvas-relative anchor (CSS px, as published by the view for the
/// active cell) into client-space coordinates, by adding the canvas's viewport
/// offset. Falls back to the top-left of the canvas when no anchor is set. Used
/// to position the filter popover under the active column header.
pub(crate) fn client_anchor(anchor: Option<(f64, f64)>) -> (f64, f64) {
    let (ox, oy) = leptos::prelude::document()
        .get_element_by_id("ambre-canvas")
        .map(|c| {
            let r = c.get_bounding_client_rect();
            (r.left(), r.top())
        })
        .unwrap_or((0.0, 0.0));
    let (ax, ay) = anchor.unwrap_or((0.0, 0.0));
    (ox + ax, oy + ay)
}

/// Dispatch a `CommandId` to the currently routed document, if any. Reads the
/// active document untracked (this runs in an event handler, not a reactive
/// scope) and no-ops on the landing route, where no document is open.
pub(crate) fn dispatch_to_current(
    id: CommandId,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: &flume::Sender<LogicCommand>,
) {
    let Some(document_id) = current_doc.get_untracked() else {
        return;
    };
    if let Err(e) = dispatch_command(id, document_id, to_logic) {
        tracing::warn!("{id:?}: Logic channel full ({e:?})");
    }
}

/// Service the toolbar/menu Paste button by reading the OS clipboard with the
/// async Clipboard API (`-ui.md` sec.5.E). `read()` recovers the private Ambre
/// payload (full styled / formula-reindexed paste) when present, else the plain
/// text; on a browser without `read()` it falls back to `readText()`. The chosen
/// content is handed to the Logic thread, which resolves the target from the
/// tracked active cell. Best-effort: a clipboard the browser will not surface
/// (permission denied, unsupported) is logged, not fatal -- Ctrl+V still works.
/// Open a transient `<input type="file">` picker for a CSV import
/// (`-interop.md` §4.A). On selection it reads the file bytes off the main
/// thread (`File::array_buffer`) and sends a [`TrameCommand::ImportCsv`] to the
/// Logic thread, which converts them into a new sheet client-side. The picker
/// element is never attached to the DOM; the change closure is leaked because the
/// picker is one-shot and lives only until the user picks a file.
pub(crate) fn open_csv_picker(
    document_id: DocumentId,
    tx: flume::Sender<LogicCommand>,
    toasts: Toasts,
) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(input) = document
        .create_element("input")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    input.set_type("file");
    let _ = input.set_attribute("accept", ".csv,text/csv");

    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        let Some(file) = picker.files().and_then(|fs| fs.get(0)) else {
            return;
        };
        let tx = tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buf) => {
                    let data = js_sys::Uint8Array::new(&buf).to_vec();
                    let _ = tx.try_send(LogicCommand::App {
                        document_id,
                        command: AppCommand::Trame(TrameCommand::ImportCsv {
                            data,
                            delimiter: None,
                            has_header: false,
                            target_sheet_id: None,
                            start_row: 0,
                            start_col: 0,
                        }),
                    });
                }
                Err(_) => toasts.warning("Could not read the CSV file"),
            }
        });
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

pub(crate) async fn paste_from_clipboard(
    read_promise: js_sys::Promise,
    clipboard: web_sys::Clipboard,
    document_id: DocumentId,
    tx: flume::Sender<LogicCommand>,
    toasts: Toasts,
) {
    let send = |cmd: TrameCommand| {
        let _ = tx.try_send(LogicCommand::App {
            document_id,
            command: AppCommand::Trame(cmd),
        });
    };

    // Structured read: rich Ambre type first, then plain text. `read_promise` was
    // started synchronously in the click handler so the user gesture is preserved.
    if let Ok(val) = wasm_bindgen_futures::JsFuture::from(read_promise).await
        && let Ok(items) = val.dyn_into::<js_sys::Array>()
    {
        for item in items.iter() {
            let Ok(item) = item.dyn_into::<web_sys::ClipboardItem>() else {
                continue;
            };
            let types = item.types();
            let has = |t: &str| types.iter().any(|v| v.as_string().as_deref() == Some(t));
            if has(host::AMBRE_CLIPBOARD_WEB_TYPE)
                && let Some(payload) = read_clipboard_item(&item, host::AMBRE_CLIPBOARD_WEB_TYPE)
                    .await
                    .and_then(|b64| host::base64_decode(&b64))
                    .and_then(|bytes| {
                        postcard::from_bytes::<ambre_bus::ClipboardPayload>(&bytes).ok()
                    })
            {
                send(TrameCommand::PasteClipboardAtActive { payload });
                return;
            }
            if has("text/plain")
                && let Some(text) = read_clipboard_item(&item, "text/plain").await
            {
                send(TrameCommand::PasteText { text });
                return;
            }
        }
    }

    // Fallback for browsers without structured `read()`.
    if let Ok(val) = wasm_bindgen_futures::JsFuture::from(clipboard.read_text()).await
        && let Some(text) = val.as_string().filter(|t| !t.is_empty())
    {
        send(TrameCommand::PasteText { text });
        return;
    }
    tracing::warn!("Trame: clipboard read unavailable for the Paste button (use Ctrl+V)");
    toasts.warning("Could not read the clipboard; use Ctrl+V to paste");
}

/// Read one MIME entry of a [`web_sys::ClipboardItem`] as a string (its blob
/// decoded as UTF-8). `None` on any failure.
pub(crate) async fn read_clipboard_item(
    item: &web_sys::ClipboardItem,
    mime: &str,
) -> Option<String> {
    let blob_val = wasm_bindgen_futures::JsFuture::from(item.get_type(mime))
        .await
        .ok()?;
    let blob = blob_val.dyn_into::<web_sys::Blob>().ok()?;
    let text_val = wasm_bindgen_futures::JsFuture::from(blob.text())
        .await
        .ok()?;
    text_val.as_string()
}

/// Dev-only: resolve the dev login identity from `?user=<name>` in the boot URL.
///
/// The engine lives for the whole session now (SPA), so a single boot-time
/// `DevLogin` re-derives the session keys once and they persist across client
/// navigations -- no `localStorage` round-trip is needed (the old full-page-load
/// workaround). The landing's dev-login field dispatches `DevLogin` directly
/// thereafter. The `https://`->`{url_scheme}://` fix lives in `ambre-sync`.
pub(crate) fn dev_user_from_url() -> Option<String> {
    let window = web_sys::window()?;
    window
        .location()
        .search()
        .ok()
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|p| p.get("user"))
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}

/// localStorage key under which the active dev username is remembered, so a
/// refresh re-establishes the dev session without `?user=` in the URL.
const DEV_USER_KEY: &str = "ambre.dev_user";

/// Read the dev username remembered from a prior session, if any.
pub(crate) fn stored_dev_user() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(DEV_USER_KEY)
        .ok()
        .flatten()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
}

/// Remember `username` as the active dev identity (dev-only; the derived keys are
/// never stored, only the name they are re-derived from).
pub(crate) fn remember_dev_user(username: &str) {
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        let _ = storage.set_item(DEV_USER_KEY, username);
    }
}

/// Map a `SyncWarningCode` (the closed `ambre-bus` contract) to a user-facing
/// notice. All are non-fatal and self-correcting, so the wording reassures
/// rather than alarms. Exhaustive by design: a new variant is a compile error
/// here until it gets a message.
pub(crate) fn sync_warning_message(code: SyncWarningCode) -> &'static str {
    match code {
        SyncWarningCode::QuarantineOverflow => {
            "Syncing is catching up - some changes are taking longer to arrive."
        }
        SyncWarningCode::DecryptFailed => "A remote change could not be read and was skipped.",
    }
}

/// URL of the Sync worker's loader. Trunk builds `ambre-sync` as a separate
/// `data-type="worker"` wasm artifact and emits this `importScripts`-based
/// loader (a CLASSIC worker; it calls `wasm_bindgen(...)`, which creates the
/// worker's own shared `WebAssembly.Memory` and runs `#[wasm_bindgen(start)]`).
/// `filehash` is off (Trunk.toml), so the path is stable.
const SYNC_WORKER_URL: &str = "/ambre-sync_loader.js";

/// Fetch the server's public auth config (`GET /api/auth/config`) and return
/// its `dev_mode` flag. Same-origin, unauthenticated; on any failure (network,
/// non-JSON, missing field) it defaults to `false`, so production falls back to
/// the OIDC sign-in affordance.
pub(crate) async fn fetch_dev_mode() -> bool {
    async fn inner() -> Option<bool> {
        let window = web_sys::window()?;
        let req = web_sys::Request::new_with_str("/api/auth/config").ok()?;
        let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req))
            .await
            .ok()?;
        let resp: web_sys::Response = resp_val.dyn_into().ok()?;
        let json = wasm_bindgen_futures::JsFuture::from(resp.json().ok()?)
            .await
            .ok()?;
        js_sys::Reflect::get(&json, &"dev_mode".into())
            .ok()
            .and_then(|v| v.as_bool())
    }
    inner().await.unwrap_or(false)
}

/// Up to two uppercase initials from a display name, for the header avatar.
/// Empty / missing name falls back to a neutral dash.
pub(crate) fn initials(name: Option<&str>) -> String {
    let Some(name) = name else {
        return "-".to_owned();
    };
    let letters: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if letters.is_empty() {
        "-".to_owned()
    } else {
        letters
    }
}

pub(crate) fn spawn_sync_worker(
    to_logic: flume::Sender<LogicEvent>,
    sync_ready: Rc<Cell<bool>>,
) -> web_sys::Worker {
    // Classic worker (the Trunk loader uses `importScripts`); not a module worker.
    let worker = web_sys::Worker::new(SYNC_WORKER_URL).expect("failed to spawn Sync worker");

    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let bytes = js_sys::Uint8Array::new(&e.data()).to_vec();
            // Empty payload = the worker's readiness sentinel (its inbound
            // onmessage is now installed). Flip the gate so the relay starts
            // draining buffered commands to it. A real LogicEvent is never
            // zero-length (postcard always emits at least a discriminant byte).
            if bytes.is_empty() {
                sync_ready.set(true);
                return;
            }
            match postcard::from_bytes::<LogicEvent>(&bytes) {
                Ok(event) => {
                    let _ = to_logic.send(event);
                }
                Err(err) => tracing::error!("[Sync->UI] failed to decode LogicEvent: {err}"),
            }
        });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Leak the closure: it lives for the whole session (the worker never stops).
    onmessage.forget();
    worker
}

/// Encode-free postMessage of raw `bytes` to the Sync worker, transferring the
/// backing `ArrayBuffer` (zero-copy move, no structured clone of the payload).
pub(crate) fn post_to_worker(worker: &web_sys::Worker, bytes: &[u8]) {
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let buffer = array.buffer();
    let transfer = js_sys::Array::of1(&buffer);
    if let Err(e) = worker.post_message_with_transfer(&buffer, &transfer) {
        tracing::warn!("[UI->Sync] postMessage to Sync worker failed: {e:?}");
    }
}

/// Translate a `CommandId` into its `LogicCommand` and send it to the Logic
/// thread. The single place where UI affordances become IPC payloads; every
/// trigger routes through here.
///
/// Returns `Ok(())` when the command was enqueued, or an error when the Logic
/// channel is full (lossless traffic class: formatting commands must not be
/// silently dropped).
pub(crate) fn dispatch_command(
    id: CommandId,
    document_id: DocumentId,
    to_logic: &flume::Sender<LogicCommand>,
) -> Result<(), flume::TrySendError<LogicCommand>> {
    let Some(command) = trame_chrome::command_to_app(id) else {
        tracing::warn!("no LogicCommand mapping for {id:?}");
        return Ok(());
    };
    to_logic.try_send(LogicCommand::App {
        document_id,
        command,
    })
}

/// Inject the generated token stylesheet (`theme.css`) into `<head>` and set the
/// initial `data-theme`. The chrome then reads colours/spacings as CSS custom
/// properties; the canvas reads the matching `Palette` -- one source, `tokens.ron`.
pub(crate) fn inject_theme(theme: ambre_canvas::Theme) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(head)) = document.query_selector("head") {
        if let Ok(style) = document.create_element("style") {
            style.set_id("ambre-theme");
            style.set_text_content(Some(ambre_canvas::theme::THEME_CSS));
            let _ = head.append_child(&style);
        }
    }
    apply_theme_attr(theme);
}

/// Set the document element's `data-theme`, switching the active CSS custom-property
/// set (`:root` light vs `:root[data-theme="dark"]`).
pub(crate) fn apply_theme_attr(theme: ambre_canvas::Theme) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = el.set_attribute("data-theme", theme.data_attr());
    }
}

/// Attach a `contextmenu` listener to the canvas: right-click opens the DOM
/// context menu at the click point (and suppresses the browser menu). Runs in
/// a post-mount effect so the AppShell's `<canvas>` is already in the DOM.
pub(crate) fn install_canvas_context_menu(ctx_menu: RwSignal<Option<(f64, f64)>>) {
    Effect::new(move |_| {
        let Some(canvas) = leptos::prelude::document().get_element_by_id("ambre-canvas") else {
            return;
        };
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            e.prevent_default();
            ctx_menu.set(Some((e.client_x() as f64, e.client_y() as f64)));
        });
        let _ = canvas.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}

/// Install the global command-keybinding listener (the keymap's DOM half).
/// A `keydown` whose chord resolves in the keymap is owned: default prevented,
/// command dispatched to the routed document. Unmatched keys fall through to the
/// canvas listener (content navigation), so this never steals arrow keys.
#[allow(clippy::too_many_arguments)]
pub(crate) fn install_command_keys(
    keymap: Keymap,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: flume::Sender<LogicCommand>,
    find_state: RwSignal<Option<trame_chrome::FindReplaceState>>,
    find_state_toile: RwSignal<bool>,
    boot_is_toile: bool,
) {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    let cb =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let Some(chord) = ambre_chrome::keybinding_from_event(&event) else {
                return;
            };
            let Some(id) = keymap.resolve(chord) else {
                return;
            };
            event.prevent_default();
            // Toile Find (`toile-ui §5quater`) is a UI-only intent: toggle the DOM
            // panel open rather than dispatching to Logic (the panel emits `Find`).
            if boot_is_toile {
                if id == toile_chrome::FIND {
                    find_state_toile.set(true);
                }
                return;
            }
            // Find & Replace (sec.5.G) are UI-only intents: open the DOM panel in
            // the matching mode rather than dispatching to the Logic thread.
            if id == trame_chrome::FIND {
                find_state.set(Some(trame_chrome::FindReplaceState {
                    mode: trame_chrome::FindMode::Find,
                }));
                return;
            }
            if id == trame_chrome::REPLACE {
                find_state.set(Some(trame_chrome::FindReplaceState {
                    mode: trame_chrome::FindMode::Replace,
                }));
                return;
            }
            dispatch_to_current(id, current_doc, &to_logic);
        });
    document
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .expect("add command keydown listener");
    cb.forget();
}
