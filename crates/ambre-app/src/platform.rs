// ambre-app/src/platform.rs
// ============================================================================
// Leaf platform helpers for the launcher. Command dispatch, Sync-worker spawn
// and postMessage, theme injection, dev-login URL and localStorage plumbing,
// the canvas context-menu and keyboard installers, and small formatting
// utilities. All pure glue with no reactive state of their own.
// ============================================================================

use super::*;
use ambre_draw::Point;

/// Convert a canvas-relative anchor (CSS px, as published by the view for the
/// active cell) into client-space coordinates, by adding the canvas's viewport
/// offset. Falls back to the top-left of the canvas when the view publishes no
/// anchor. Used
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
/// active document untracked, because this runs in an event handler and not a
/// reactive scope. Does nothing on the landing route, where no document is open.
pub(crate) fn dispatch_to_current(
    id: CommandId,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: &ambre_bus::conduit::Sender<LogicCommand>,
) {
    let Some(document_id) = current_doc.get_untracked() else {
        return;
    };
    if let Err(e) = dispatch_command(id, document_id, to_logic) {
        tracing::warn!("{id:?}: Logic channel full ({e:?})");
    }
}

/// Open a transient `<input type="file">` picker for a CSV import
/// (`trame-import-export`). On selection it reads the file bytes off the main
/// thread (`File::array_buffer`). It then sends a [`TrameCommand::ImportCsv`]
/// to the Logic thread, which converts them into a new sheet client-side.
pub(crate) fn open_csv_picker(
    document_id: DocumentId,
    tx: ambre_bus::conduit::Sender<LogicCommand>,
    toasts: Toasts,
) {
    open_file_picker(
        ".csv,text/csv",
        "Could not read the CSV file",
        toasts,
        move |data| {
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
        },
    );
}

/// Open a transient `<input type="file">` picker for a Markdown import
/// (`PROSE-IO-027`). The bytes go to the Logic thread, which converts them
/// client-side (`PROSE-IO-001`).
pub(crate) fn open_markdown_picker(
    document_id: DocumentId,
    tx: ambre_bus::conduit::Sender<LogicCommand>,
    toasts: Toasts,
) {
    open_file_picker(
        ".md,.markdown,text/markdown",
        "Could not read the Markdown file",
        toasts,
        move |data| {
            let _ = tx.try_send(LogicCommand::App {
                document_id,
                command: AppCommand::Prose(ambre_bus::app::prose::ProseCommand::ImportDocument {
                    format: ambre_bus::app::prose::ImportFormat::Markdown,
                    data,
                }),
            });
        },
    );
}

/// Open a transient file picker and hand the chosen file's bytes to `on_bytes`.
/// The picker element never reaches the DOM. The code leaks the change
/// closure, because the picker is one-shot and lives only until the user picks
/// a file.
fn open_file_picker(
    accept: &str,
    unreadable: &'static str,
    toasts: Toasts,
    on_bytes: impl Fn(Vec<u8>) + Clone + 'static,
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
    let _ = input.set_attribute("accept", accept);

    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        let Some(file) = picker.files().and_then(|fs| fs.get(0)) else {
            return;
        };
        let on_bytes = on_bytes.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buf) => on_bytes(js_sys::Uint8Array::new(&buf).to_vec()),
                Err(_) => toasts.warning(unreadable),
            }
        });
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

/// Service the toolbar/menu Paste button by reading the OS clipboard with the
/// async Clipboard API. `read()` recovers the private Ambre payload, the
/// ownership token and the plain text; the click handler started `read_promise`
/// synchronously, so the read keeps the gesture. On a browser without `read()`
/// it falls back to `readText()`. The host routes the decoded transport to the
/// active view through the same ladder as the native `paste` event
/// (`UI-THREAD-024`). Best-effort: a clipboard the browser will not surface
/// (permission denied, unsupported) earns a log line, not a failure -- Ctrl+V
/// still works. `plain` keeps the text alone, for the paste-as-plain-text
/// entry (`PROSE-UI-227`).
pub(crate) async fn paste_from_clipboard(
    read_promise: js_sys::Promise,
    clipboard: web_sys::Clipboard,
    paste_tx: ambre_bus::conduit::Sender<host::ClipboardPaste>,
    toasts: Toasts,
    plain: bool,
) {
    let (mut blob, mut token, mut found) = match read_clipboard_blob(read_promise).await {
        Some((blob, token)) => (blob, token, true),
        None => (ambre_bus::ClipboardBlob::default(), None, false),
    };

    // Fallback for browsers without structured `read()`.
    if !found
        && let Ok(val) = wasm_bindgen_futures::JsFuture::from(clipboard.read_text()).await
        && let Some(text) = val.as_string().filter(|t| !t.is_empty())
    {
        blob.text = text;
        found = true;
    }

    if !found {
        tracing::warn!("clipboard read unavailable for the Paste button (use Ctrl+V)");
        toasts.warning("Could not read the clipboard; use Ctrl+V to paste");
        return;
    }

    if plain {
        blob = ambre_bus::ClipboardBlob {
            text: blob.text,
            plain: true,
            ..ambre_bus::ClipboardBlob::default()
        };
        token = None;
    }

    // The host compares the token and pastes through the active view
    // (`UI-THREAD-024`, `UI-THREAD-025`).
    let _ = paste_tx.try_send(host::ClipboardPaste { blob, token });
}

/// The asynchronous clipboard, or `None` where it is undefined: outside a
/// secure context, such as http:// on a LAN IP.
pub(crate) fn async_clipboard() -> Option<web_sys::Clipboard> {
    let clipboard = web_sys::window()?.navigator().clipboard();
    let v: &wasm_bindgen::JsValue = clipboard.as_ref();
    (!v.is_undefined() && !v.is_null()).then_some(clipboard)
}

/// Decode a structured `read()`: the private bytes, the ownership token, the
/// HTML and the plain text, in whatever order the browser lists them. `None`
/// when the read was refused or surfaced nothing.
pub(crate) async fn read_clipboard_blob(
    read_promise: js_sys::Promise,
) -> Option<(ambre_bus::ClipboardBlob, Option<String>)> {
    let mut blob = ambre_bus::ClipboardBlob::default();
    let mut token: Option<String> = None;
    let mut found = false;
    let val = wasm_bindgen_futures::JsFuture::from(read_promise)
        .await
        .ok()?;
    let items = val.dyn_into::<js_sys::Array>().ok()?;
    for item in items.iter() {
        let Ok(item) = item.dyn_into::<web_sys::ClipboardItem>() else {
            continue;
        };
        let types = item.types();
        let has = |t: &str| types.iter().any(|v| v.as_string().as_deref() == Some(t));
        if has(host::AMBRE_CLIPBOARD_WEB_TYPE)
            && let Some(b64) = read_clipboard_item(&item, host::AMBRE_CLIPBOARD_WEB_TYPE).await
        {
            blob.ambre = host::base64_decode(&b64);
            found = true;
        }
        if has(host::AMBRE_CLIPBOARD_WEB_TOKEN_TYPE)
            && let Some(t) = read_clipboard_item(&item, host::AMBRE_CLIPBOARD_WEB_TOKEN_TYPE).await
        {
            token = Some(t);
            found = true;
        }
        if has("text/html")
            && let Some(html) = read_clipboard_item(&item, "text/html").await
        {
            blob.html = Some(html).filter(|h| !h.is_empty());
            found = true;
        }
        if has("text/plain")
            && let Some(text) = read_clipboard_item(&item, "text/plain").await
        {
            blob.text = text;
            found = true;
        }
    }
    // A browser that refused the `web ` types left the token in the HTML
    // (`UI-THREAD-056`).
    let token = token.or_else(|| {
        blob.html
            .as_deref()
            .and_then(ambre_bus::ClipboardBlob::html_token)
            .map(str::to_owned)
    });
    found.then_some((blob, token))
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
/// The engine lives for the whole session now (SPA). A single boot-time
/// `DevLogin` therefore re-derives the session keys once, and they persist
/// across client navigations. No `localStorage` round-trip remains, which was
/// the old full-page-load workaround. The landing's dev-login field dispatches
/// `DevLogin` directly thereafter. The `https://`->`{url_scheme}://` fix lives
/// in `ambre-sync`.
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

/// localStorage key holding the active dev username, so a refresh
/// re-establishes the dev session without `?user=` in the URL.
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
/// `data-type="worker"` wasm artifact, and emits this `importScripts`-based
/// loader. The loader is a CLASSIC worker: it calls `wasm_bindgen(...)`, which
/// creates the worker's own shared `WebAssembly.Memory` and runs
/// `#[wasm_bindgen(start)]`. `filehash` is off (Trunk.toml), so the path is
/// stable.
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

/// The main side of the Sync -> Logic channel. An event from the Sync worker
/// is lossless (a CRDT delta, a service answer), and the main thread may not
/// wait for room (`ARCH-021`). So what the channel cannot take waits in the
/// backlog, and the relay offers it again each frame (`BUS-025`). Shared by
/// the worker's `onmessage` and the relay, both on the main thread.
pub(crate) type SyncInbox = Rc<RefCell<ambre_bus::conduit::Backlog<LogicEvent>>>;

pub(crate) fn spawn_sync_worker(
    to_logic: SyncInbox,
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
                Ok(event) => to_logic.borrow_mut().push(event),
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
/// Returns `Ok(())` once the Logic channel takes the command. Returns an error
/// when that channel is full, because the lossless traffic class forbids
/// dropping a formatting command in silence.
pub(crate) fn dispatch_command(
    id: CommandId,
    document_id: DocumentId,
    to_logic: &ambre_bus::conduit::Sender<LogicCommand>,
) -> Result<(), ambre_bus::conduit::TrySendError<LogicCommand>> {
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

/// Attach `dragover` / `drop` to the canvas so a file dropped on the board
/// reaches the importer at the point it landed on (`TOILE-IO-020`,
/// `TOILE-IO-023`). Runs in a post-mount effect, like the context menu.
///
/// This function converts the drop point to physical pixels, the same scaling
/// the pointer path applies. That is the space the canvas view works in. The
/// read takes the bytes off the main thread and publishes them into the shared
/// signal the view drains next frame. The board decides what it can read
/// (`toile_chrome::accepts_file`), so the launcher holds no format list.
pub(crate) fn install_canvas_file_drop(
    dropped: RwSignal<Option<toile_chrome::DroppedFile>>,
    toasts: Toasts,
) {
    Effect::new(move |_| {
        let Some(canvas) = leptos::prelude::document().get_element_by_id("ambre-canvas") else {
            return;
        };
        // Without a `dragover` that prevents the default, the browser navigates
        // away to the dropped file and the drop never fires.
        let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|e: web_sys::DragEvent| {
            e.prevent_default();
        });
        let _ = canvas.add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref());
        over.forget();

        let dpr = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0);
        let cb = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
            e.prevent_default();
            let Some(file) = e
                .data_transfer()
                .and_then(|dt| dt.files())
                .and_then(|fs| fs.get(0))
            else {
                return;
            };
            let at = Point::new(e.offset_x() as f64 * dpr, e.offset_y() as f64 * dpr);
            read_file_into(file, Some(at), dropped, toasts);
        });
        let _ = canvas.add_event_listener_with_callback("drop", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}

/// Open a transient `<input type="file">` for a board import (`TOILE-IO-020`).
/// The picker offers what the board reads and nothing else. A file picked this
/// way has no drop point, so the view lands it in the middle of the viewport.
pub(crate) fn open_board_import_picker(
    dropped: RwSignal<Option<toile_chrome::DroppedFile>>,
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
    let _ = input.set_attribute("accept", toile_chrome::IMPORT_ACCEPT);
    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        if let Some(file) = picker.files().and_then(|fs| fs.get(0)) {
            read_file_into(file, None, dropped, toasts);
        }
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

/// Read `file` into memory and publish it for the board, or tell the user why
/// not (`TOILE-IO-022`). The check refuses a type no converter reads before it
/// touches the bytes, so a dropped video never costs a copy of itself.
fn read_file_into(
    file: web_sys::File,
    at: Option<Point>,
    dropped: RwSignal<Option<toile_chrome::DroppedFile>>,
    toasts: Toasts,
) {
    // The browser leaves the type empty for a format it does not know. Fall
    // back to the extension, all an `.svg` drop carries on some platforms.
    let mut mime = file.type_();
    if mime.is_empty() && file.name().to_ascii_lowercase().ends_with(".svg") {
        mime = "image/svg+xml".to_owned();
    }
    if !toile_chrome::accepts_file(&mime) {
        toasts.warning(format!(
            "Ambre cannot bring a {} file onto a board",
            label(&mime)
        ));
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
            Ok(buf) => {
                let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                dropped.set(Some(toile_chrome::DroppedFile { bytes, mime, at }));
            }
            Err(_) => toasts.warning("Could not read the file"),
        }
    });
}

/// A media type as a user reads it: the subtype alone (`png`). Failing that, a
/// word for a file whose type the browser could not name.
fn label(mime: &str) -> String {
    match mime.rsplit('/').next() {
        Some(sub) if !sub.is_empty() => sub.to_owned(),
        _ => "file of that kind".to_owned(),
    }
}

/// Whether a keyboard event targets an editable element: a text `<input>`, a
/// `<textarea>`, or a contenteditable host. A bare tool chord must not steal
/// such an event (`toile-tools`). The pure decision lives in
/// `ambre_toile::ui::palette::is_editable_tag` (host-tested); this only reads the
/// tag name and contenteditable flag off the event target.
fn target_is_editable(event: &web_sys::KeyboardEvent) -> bool {
    use wasm_bindgen::JsCast;
    let Some(target) = event.target() else {
        return false;
    };
    let tag = target
        .dyn_ref::<web_sys::Element>()
        .map(|el| el.tag_name())
        .unwrap_or_default();
    let content_editable = target
        .dyn_ref::<web_sys::HtmlElement>()
        .is_some_and(|el| el.is_content_editable());
    ambre_toile::ui::palette::is_editable_tag(&tag, content_editable)
}

/// Install the global command-keybinding listener (the keymap's DOM half).
/// This listener owns a `keydown` whose chord resolves in the keymap: default
/// prevented, command dispatched to the routed document. Unmatched keys fall through to the
/// canvas listener (content navigation), so this never steals arrow keys.
#[allow(clippy::too_many_arguments)]
pub(crate) fn install_command_keys(
    keymap: Keymap,
    current_doc: RwSignal<Option<DocumentId>>,
    to_logic: ambre_bus::conduit::Sender<LogicCommand>,
    find_state: RwSignal<Option<trame_chrome::FindReplaceState>>,
    find_state_toile: RwSignal<bool>,
    active_tool_toile: RwSignal<ambre_toile::ui::Tool>,
    boot_is_toile: bool,
    prose_format: Option<RwSignal<Option<CommandId>>>,
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
            // Toile board: the tool chords (`toile-tools`) and Find
            // (`toile-find`) are UI-only intents this listener handles.
            // `ToileView::handle_key` decodes every other Toile id
            // canvas-side, so this listener leaves it alone.
            if boot_is_toile {
                if let Some(tool) = toile_chrome::tool_for_command(id) {
                    // Don't steal a bare tool key while the user is typing (the
                    // Parley editor's hidden input, a chrome text field): let the
                    // character through untouched.
                    if target_is_editable(&event) {
                        return;
                    }
                    event.prevent_default();
                    active_tool_toile.set(tool);
                    return;
                }
                event.prevent_default();
                if id == toile_chrome::FIND {
                    find_state_toile.set(true);
                }
                return;
            }
            // Prose: a formatting chord leaves its id where the format bar's
            // controls leave theirs, and the view drains both the same way
            // (`PROSE-UI-056`). A chrome text field keeps its own chords.
            if let Some(format) = prose_format
                && ambre_prose::commands::mark_key(id).is_some()
            {
                if !target_is_editable(&event) {
                    event.prevent_default();
                    format.set(Some(id));
                }
                return;
            }
            event.prevent_default();
            // Find & Replace (`trame-find-replace`) are UI-only intents: open the
            // DOM panel in the matching mode, not dispatch to the Logic thread.
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

/// Open a transient `<input type="file">` for a native `.ambre` file, and open
/// the document it carries (`ambre-spec-docfile.md section 4`).
///
/// The launcher owns this gesture because the file owns the choice of
/// application. At the moment of picking, no document is open and no engine has
/// booted, so the header is what names the owner. That is also why this file
/// reads the container layer rather than an app. `read_ambre_plaintext`
/// migrates an older container or refuses a newer one before anything sees a
/// payload (`DOCFILE-002`, `DOCFILE-003`, `DOCFILE-013`).
///
/// * The payload reaches the engine through the local snapshot store, never
///   through a channel. The path always reloads the page (below), and a
///   channel does not survive that.
/// * The store is where a document's bytes already wait for a boot. The file
///   therefore lands exactly where a refresh would have left them, and the
///   existing restore leg picks it up, schema gate included (`SCHEMA-006`).
///
/// **The reload is unconditional**, where opening a *card* reloads only across
/// render kinds (`route::needs_reload`). Two reasons.
///
/// * The page load fixes the booted `AppView`, so half the files would need a
///   reload anyway.
/// * The restore leg runs at boot, not on a router swap, so a pushState would
///   land on an empty document.
///
/// One path, taken every time, beats two that differ by the byte at offset 5.
pub(crate) fn open_ambre_file_picker(toasts: Toasts, on_sealed: Callback<Vec<u8>>) {
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
    let _ = input.set_attribute("accept", ".ambre");
    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        if let Some(file) = picker.files().and_then(|fs| fs.get(0)) {
            read_ambre_file(file, toasts, on_sealed);
        }
    });
    input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
    onchange.forget();
    input.click();
}

/// Read `file`, attribute it, persist its payload and navigate to it.
///
/// Every refusal reaches the user by name and leaves the session untouched
/// (`DOCFILE-005`).
///
/// * A file that is not a container.
/// * One this build is too old to read.
/// * One whose type no engine renders.
/// * A snapshot store that did not take the bytes.
///
/// None of them navigates anywhere, so a refused file costs the user a message
/// and nothing else.
fn read_ambre_file(file: web_sys::File, toasts: Toasts, on_sealed: Callback<Vec<u8>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await else {
            toasts.error("Ambre could not read that file");
            return;
        };
        let bytes = js_sys::Uint8Array::new(&buf).to_vec();

        // A sealed file is a container of the other lineage, and the version
        // byte alone says which (`DOCFILE-010`). Nothing here can read it: it
        // needs a passphrase this thread must ask for, and a key the Sync
        // worker derives. So the bytes go to the caller's prompt, and the
        // plaintext path below resumes on what comes back (`DOCFILE-015`).
        if matches!(
            ambre_core::classify(&bytes),
            Ok(ambre_core::ContainerKind::Sealed(_))
        ) {
            on_sealed.run(bytes);
            return;
        }
        open_container_bytes(bytes, toasts).await;
    });
}

/// Attribute a plaintext container, persist its payload and navigate to it.
///
/// Split from [`read_ambre_file`] because a sealed file reaches exactly this
/// point too, one round trip later and by way of the Sync worker. What it holds
/// is a whole plaintext container, so the two lineages share every step after
/// the seal comes off (`DOCFILE-014`).
pub(crate) fn open_plaintext_container(bytes: Vec<u8>, toasts: Toasts) {
    wasm_bindgen_futures::spawn_local(open_container_bytes(bytes, toasts));
}

async fn open_container_bytes(bytes: Vec<u8>, toasts: Toasts) {
    let header = match ambre_core::docfile::read_ambre_plaintext(&bytes) {
        Ok(header) => header,
        Err(e) => {
            toasts.error(format!("Ambre cannot open that file: {e}"));
            return;
        }
    };
    // A type this build knows by name but cannot render draws a refusal here
    // rather than a blank page. `DocType::creatable` is the same predicate
    // the new-document picker offers, so the two cannot disagree.
    if !ambre_bus::DocType::creatable().contains(&header.doc_type) {
        toasts.error(format!(
            "Ambre cannot open a {} yet",
            header.doc_type.generic_label()
        ));
        return;
    }

    let store = match ambre_logic::persistence::PersistenceStore::open().await {
        Ok(Ok(store)) => store,
        _ => {
            toasts.error("Ambre could not open its local store for that file");
            return;
        }
    };
    // Durable, not fire-and-forget: the navigation below unloads the page,
    // and an open transaction goes with it.
    let committed = store
        .save_snapshot_committed(*header.document_id.as_bytes(), header.payload)
        .await;
    if committed != Ok(true) {
        toasts.error("Ambre could not store that file locally");
        return;
    }

    let path = crate::route::doc_path(header.doc_type, header.document_id.into());
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(&path);
    }
}
