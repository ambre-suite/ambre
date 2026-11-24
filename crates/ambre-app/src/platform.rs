// ambre-app/src/platform.rs
// ============================================================================
// Leaf platform helpers for the launcher. Command dispatch, Sync-worker spawn
// and postMessage, theme injection, dev-login URL and localStorage plumbing,
// the canvas context-menu and keyboard installers, and small formatting
// utilities. All pure glue with no reactive state of their own.
// ============================================================================

use super::*;
use ambre_bus::start::WorkerPost;
use ambre_chrome::commands::PasteForm;
use ambre_core::Payload;

/// Open a transient file picker and hand the chosen file's bytes to `on_bytes`,
/// read off the main thread (`File::array_buffer`). An application asks for
/// it through its handler, and names what it reads (`ARCH-018`).
///
/// The picker element never reaches the DOM. The code leaks the change
/// closure, because the picker is one-shot and lives only until the user picks
/// a file.
pub(crate) fn open_file_picker(
    accept: ambre_chrome::FileAccept,
    unreadable: &'static str,
    toasts: Toasts,
    on_bytes: impl Fn(ambre_core::Payload<ambre_client_core::FileBytes>) + Clone + 'static,
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
    let _ = input.set_attribute("accept", accept.as_attr());

    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        let Some(file) = picker.files().and_then(|fs| fs.get(0)) else {
            return;
        };
        let on_bytes = on_bytes.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buf) => on_bytes(ambre_core::Payload::<ambre_client_core::FileBytes>::new(
                    js_sys::Uint8Array::new(&buf).to_vec(),
                )),
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
/// still works. A `Plain` form keeps the text alone, for the paste-as-plain-text
/// entry (`PROSE-UI-227`).
pub(crate) async fn paste_from_clipboard(
    read_promise: js_sys::Promise,
    clipboard: web_sys::Clipboard,
    paste_tx: ambre_bus::conduit::Sender<host::ClipboardPaste>,
    toasts: Toasts,
    form: PasteForm,
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

    if form == PasteForm::Plain {
        blob = ambre_bus::ClipboardBlob {
            text: blob.text,
            form: PasteForm::Plain,
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
) -> Option<(ambre_bus::ClipboardBlob, Option<ambre_bus::ClipToken>)> {
    let mut blob = ambre_bus::ClipboardBlob::default();
    let mut token: Option<ambre_bus::ClipToken> = None;
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
        let has = |format: host::ClipboardFormat| {
            types
                .iter()
                .any(|v| v.as_string().as_deref() == Some(format.mime()))
        };
        if has(host::ClipboardFormat::WebAmbre)
            && let Some(b64) = read_clipboard_item(&item, host::ClipboardFormat::WebAmbre).await
        {
            blob.ambre = host::base64_decode(&b64);
            found = true;
        }
        if has(host::ClipboardFormat::WebAmbreToken)
            && let Some(t) = read_clipboard_item(&item, host::ClipboardFormat::WebAmbreToken).await
        {
            token = ambre_bus::ClipToken::new(t);
            found = true;
        }
        if has(host::ClipboardFormat::Html)
            && let Some(html) = read_clipboard_item(&item, host::ClipboardFormat::Html).await
        {
            blob.html = Some(html).filter(|h| !h.is_empty());
            found = true;
        }
        if has(host::ClipboardFormat::PlainText)
            && let Some(text) = read_clipboard_item(&item, host::ClipboardFormat::PlainText).await
        {
            blob.text = text;
            found = true;
        }
    }
    // A browser that refused the `web ` types left the token in the HTML
    // (`UI-THREAD-057`).
    let token = token.or_else(|| {
        blob.html
            .as_deref()
            .and_then(ambre_bus::ClipboardBlob::html_token)
    });
    found.then_some((blob, token))
}

/// Read one type of a [`web_sys::ClipboardItem`] as a string (its blob
/// decoded as UTF-8). `None` on any failure.
pub(crate) async fn read_clipboard_item(
    item: &web_sys::ClipboardItem,
    format: host::ClipboardFormat,
) -> Option<String> {
    let blob_val = wasm_bindgen_futures::JsFuture::from(item.get_type(format.mime()))
        .await
        .ok()?;
    let blob = blob_val.dyn_into::<web_sys::Blob>().ok()?;
    let text_val = wasm_bindgen_futures::JsFuture::from(blob.text())
        .await
        .ok()?;
    text_val.as_string()
}

/// localStorage key holding the reader's own locale tag, empty for none
/// (`ambre-spec-locale.md` section 3). It lives on the device and in no
/// document.
const READER_LOCALE_KEY: ambre_core::StaticKey<ambre_core::StorageKeys> =
    ambre_core::StaticKey::new("ambre.reader_locale");

/// The reader's own locale and convention. The first run reads the browser's
/// language once and stores it, so a later boot reads the device's setting
/// and never the machine again (`LOCALE-012`). A store that cannot be read
/// gives the first-run reading, unstored.
pub(crate) fn reader_locale() -> ambre_client_core::ReaderLocale {
    let Some(window) = web_sys::window() else {
        return ambre_client_core::ReaderLocale::default();
    };
    let storage = window.local_storage().ok().flatten();
    let stored = storage
        .as_ref()
        .and_then(|s| s.get_item(READER_LOCALE_KEY.as_str()).ok().flatten());
    if let Some(tag) = stored {
        return ambre_client_core::ReaderLocale {
            own: ambre_client_core::DocumentLocale::from_register(Some(tag.as_str())),
            convention: ambre_client_core::NumberConvention::default(),
        };
    }
    // The browser's language, read once; a malformed one is no locale.
    let reader = ambre_client_core::ReaderLocale {
        own: ambre_client_core::DocumentLocale::from_register(
            window.navigator().language().as_deref(),
        ),
        convention: ambre_client_core::NumberConvention::default(),
    };
    if let Some(storage) = storage {
        let _ = storage.set_item(
            READER_LOCALE_KEY.as_str(),
            reader.own.as_str().unwrap_or_default(),
        );
    }
    reader
}

/// localStorage key of the boot hint: the theme and the interface language
/// of the account, in the clear, read before the unlock (`SETTINGS-028`).
const BOOT_HINT_KEY: ambre_core::StaticKey<ambre_core::StorageKeys> =
    ambre_core::StaticKey::new("ambre.boot_hint");

/// The boot hint this device keeps, or `None` for a fresh device, a
/// signed-out one or a hint of another version. Read with no wait
/// (`SETTINGS-034`).
pub(crate) fn boot_hint() -> Option<ambre_client_core::BootHint> {
    let text = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(BOOT_HINT_KEY.as_str()).ok().flatten())?;
    ambre_client_core::BootHint::decode(&ambre_client_core::BootHintText::new(text))
}

/// Keeps `hint` on this device, and nowhere else (`SETTINGS-029`).
pub(crate) fn keep_boot_hint(hint: &ambre_client_core::BootHint) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(BOOT_HINT_KEY.as_str(), hint.encode().as_str());
    }
}

/// Deletes the boot hint: the device signs out (`SETTINGS-039`).
pub(crate) fn forget_boot_hint() {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.remove_item(BOOT_HINT_KEY.as_str());
    }
}

/// Moves both halves of the theme to `theme`, as a switch does.
pub(crate) fn set_theme(control: &ambre_canvas::ThemeControl, theme: ambre_canvas::Theme) {
    control.set(theme);
    apply_theme_attr(theme);
}

/// The word this browser keeps under `key`, or `None` when the store refuses
/// or holds nothing. A view setting kept on the device, such as the side of
/// the context panel (`ambre-spec-shell.md` section 5), reads through here.
pub(crate) fn stored_word(key: ambre_core::StaticKey<ambre_core::StorageKeys>) -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key.as_str()).ok().flatten())
}

/// Keep `word` under `key` on this browser. A refused write keeps nothing.
pub(crate) fn keep_word(key: ambre_core::StaticKey<ambre_core::StorageKeys>, word: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(key.as_str(), word);
    }
}

/// The device of `kind` that the member chose for a call on this browser. The choice lives on
/// the device and in no document, since a list of devices nearly names a machine (`ZK-026`).
pub(crate) fn device_choice(
    kind: ambre_murmure::ui::call_devices::DeviceKind,
) -> ambre_murmure::ui::call_devices::DeviceChoice {
    let id = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(kind.storage_key().as_str()).ok().flatten())
        .map(ambre_core::Opaque::<ambre_murmure::ui::call_devices::DeviceIds>::new);
    ambre_murmure::ui::call_devices::DeviceChoice::from_stored(id)
}

/// Keeps `choice` of `kind` on this browser. The default device removes the entry.
pub(crate) fn keep_device_choice(
    kind: ambre_murmure::ui::call_devices::DeviceKind,
    choice: &ambre_murmure::ui::call_devices::DeviceChoice,
) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let key = kind.storage_key();
        let _ = match choice.stored() {
            Some(id) => storage.set_item(key.as_str(), id.as_str()),
            None => storage.remove_item(key.as_str()),
        };
    }
}

/// The volumes of a call the local storage of this browser holds (the Murmure calls spec,
/// section 3, the volumes).
pub(crate) fn stored_volumes() -> ambre_murmure::ui::call_volumes::Volumes {
    let key = ambre_murmure::ui::call_volumes::Volumes::storage_key();
    let text = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key.as_str()).ok().flatten());
    ambre_murmure::ui::call_volumes::Volumes::from_stored(text.as_deref())
}

/// Keeps `volumes` on this browser, and nowhere else.
pub(crate) fn keep_volumes(volumes: &ambre_murmure::ui::call_volumes::Volumes) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let key = ambre_murmure::ui::call_volumes::Volumes::storage_key();
        let _ = storage.set_item(key.as_str(), &volumes.to_stored());
    }
}

/// The gain of the microphone the local storage of this browser holds.
pub(crate) fn stored_gain() -> ambre_murmure::ui::call_volumes::Gain {
    let key = ambre_murmure::ui::call_volumes::Gain::storage_key();
    let text = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key.as_str()).ok().flatten());
    ambre_murmure::ui::call_volumes::Gain::from_stored(text.as_deref())
}

/// Keeps `gain` on this browser, and nowhere else. The automatic gain removes the entry.
pub(crate) fn keep_gain(gain: ambre_murmure::ui::call_volumes::Gain) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let key = ambre_murmure::ui::call_volumes::Gain::storage_key();
        let _ = match gain.to_stored() {
            Some(text) => storage.set_item(key.as_str(), &text),
            None => storage.remove_item(key.as_str()),
        };
    }
}

// The storage half of the development sign-in (`dev_login.rs`, `ZK-045`):
// `UI-THREAD-004` names this file as its persistence path.

/// localStorage key holding the active dev username, so a refresh
/// re-establishes the dev session without `?user=` in the URL. The
/// post-build hook reads these bytes in a release wasm as the mark of the
/// development sign-in, so a rename moves `scripts/trunk_post_build.py` too.
#[cfg(feature = "dev-login")]
const DEV_USER_KEY: ambre_core::StaticKey<ambre_core::StorageKeys> =
    ambre_core::StaticKey::new("ambre.dev_user");

/// Read the dev username remembered from a prior session, if any. `None`
/// when no name is stored, or when the stored text breaks the rule of a
/// display name.
#[cfg(feature = "dev-login")]
pub(crate) fn stored_dev_user() -> Option<ambre_core::Validated<ambre_core::DisplayName>> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(DEV_USER_KEY.as_str())
        .ok()
        .flatten()
        .and_then(|u| ambre_core::Validated::<ambre_core::DisplayName>::parse(u.trim()).ok())
}

/// Remember `username` as the active dev identity. The derived keys are
/// never stored, only the name they are derived from again.
#[cfg(feature = "dev-login")]
pub(crate) fn remember_dev_user(username: &ambre_core::Validated<ambre_core::DisplayName>) {
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        let _ = storage.set_item(DEV_USER_KEY.as_str(), username.as_str());
    }
}

/// Forget the remembered dev identity, so the next boot signs no one in.
#[cfg(feature = "dev-login")]
pub(crate) fn forget_dev_user() {
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        let _ = storage.remove_item(DEV_USER_KEY.as_str());
    }
}

/// A script the page ships for a worker, whose address the post-build hook
/// writes into a meta of the page (`scripts/trunk_post_build.py`). `filehash`
/// hashes each name (Trunk.toml), so no code can spell one.
#[derive(Clone, Copy)]
enum WorkerScript {
    /// The Sync worker's loader. Trunk builds `ambre-sync` as a separate
    /// `data-type="worker"` wasm artifact, and emits this `importScripts`-based
    /// loader. The loader is a CLASSIC worker: it calls `wasm_bindgen(...)`,
    /// which creates the worker's own shared `WebAssembly.Memory` and runs
    /// `#[wasm_bindgen(start)]`.
    Sync,
    /// The call worker's loader, a third instance spawned for one call.
    Call,
    /// The module script of each thread of the Main instance. It imports the
    /// Main glue, so a thread shares the page's memory. It is a file of the
    /// build, so the content policy admits no `blob:` worker (`SRV-260`).
    Thread,
}

/// The address of `script`, read off its meta. `None` when the page carries
/// no address.
fn worker_url(script: WorkerScript) -> Option<String> {
    let selector = match script {
        WorkerScript::Sync => r#"meta[name="ambre-sync-worker"]"#,
        WorkerScript::Call => r#"meta[name="ambre-call-worker"]"#,
        WorkerScript::Thread => r#"meta[name="ambre-thread-worker"]"#,
    };
    web_sys::window()?
        .document()?
        .query_selector(selector)
        .ok()??
        .get_attribute("content")
        .filter(|url| !url.is_empty())
}

/// Names the shipped thread script to every thread the Main instance spawns
/// after this call. Without it `wasm_thread` builds the script as a `blob:`
/// address, which the content policy of the page refuses (`SRV-260`). So a
/// page that names no script logs the cause before its first spawn fails.
pub(crate) fn name_thread_script() {
    match worker_url(WorkerScript::Thread) {
        Some(url) => wasm_thread::Builder::new()
            .worker_script_url(url)
            .set_default(),
        None => tracing::error!("the page carries no thread script address"),
    }
}

/// Spawns the call worker of one call: a third instance, with its own memory, which holds
/// the keys of that call alone (`ARCH-001`, `CALL-004`). The call client terminates it once
/// it erased them.
#[expect(
    clippy::expect_used,
    reason = "a page that names no call worker was built without it, and the failure is loud"
)]
pub(crate) fn spawn_call_worker() -> web_sys::Worker {
    let url = worker_url(WorkerScript::Call).expect("the page carries no call worker address");
    web_sys::Worker::new(&url).expect("failed to spawn the call worker")
}

/// As [`post_to_worker`], with `port` moved to the worker beside the bytes. The join of a
/// call hands the Sync worker its end of the channel this way (`CALL-064`).
pub(crate) fn post_to_worker_with_port(
    worker: &web_sys::Worker,
    post: &Payload<WorkerPost>,
    port: Option<&web_sys::MessagePort>,
) {
    let bytes = post.as_bytes();
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let buffer = array.buffer();
    let transfer = js_sys::Array::of1(&buffer);
    if let Some(port) = port {
        transfer.push(port);
    }
    if let Err(e) = worker.post_message_with_transfer(&buffer, &transfer) {
        tracing::warn!("[UI->Sync] postMessage to Sync worker failed: {e:?}");
    }
}

/// Fetch the server's public auth config (`GET /api/auth/config`) and return
/// its `dev_mode` flag. Same-origin, unauthenticated; on any failure (network,
/// non-JSON, missing field) it defaults to `false`, so production falls back to
/// the OIDC sign-in affordance.
pub(crate) async fn fetch_dev_mode() -> bool {
    async fn inner() -> Option<wasm_bindgen::JsValue> {
        let window = web_sys::window()?;
        let req = web_sys::Request::new_with_str("/api/auth/config").ok()?;
        let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req))
            .await
            .ok()?;
        let resp: web_sys::Response = resp_val.dyn_into().ok()?;
        let json = wasm_bindgen_futures::JsFuture::from(resp.json().ok()?)
            .await
            .ok()?;
        js_sys::Reflect::get(&json, &"dev_mode".into()).ok()
    }
    inner().await.and_then(|flag| flag.as_bool()) == Some(true)
}

/// The main side of the Sync -> Logic channel. An event from the Sync worker
/// is lossless (a CRDT delta, a service answer), and the main thread may not
/// wait for room (`ARCH-021`). So what the channel cannot take waits in the
/// backlog, and the relay offers it again each frame (`BUS-025`). Shared by
/// the worker's `onmessage` and the relay, both on the main thread.
pub(crate) type SyncInbox = Rc<RefCell<ambre_bus::conduit::Backlog<LogicEvent>>>;

#[expect(
    clippy::expect_used,
    reason = "a client without its Sync worker cannot reach the relay at all"
)]
pub(crate) fn spawn_sync_worker(
    to_logic: SyncInbox,
    start: Rc<RefCell<ambre_bus::start::StartGate>>,
) -> web_sys::Worker {
    // Classic worker (the Trunk loader uses `importScripts`); not a module worker.
    let url = worker_url(WorkerScript::Sync).expect("the page carries no Sync worker address");
    let worker = web_sys::Worker::new(&url).expect("failed to spawn Sync worker");
    let to_worker = worker.clone();

    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let post = Payload::<WorkerPost>::new(js_sys::Uint8Array::new(&e.data()).to_vec());
            // The worker's announcement opens the gate, so the relay starts
            // draining the commands held for it (`SYNC-057`).
            if start.borrow_mut().read(&post).is_none() {
                return;
            }
            match postcard::from_bytes::<LogicEvent>(post.as_bytes()) {
                Ok(event) => {
                    // A signal of a call goes to the call client, never to Logic.
                    let Some(event) = crate::call::heard(event) else {
                        return;
                    };
                    to_logic.borrow_mut().push(event);
                    // A hidden page draws no frame, so the hold is asked here
                    // too, not only by the relay.
                    relay_pressure(&to_logic, &to_worker);
                }
                Err(err) => tracing::error!("[Sync->UI] failed to decode LogicEvent: {err}"),
            }
        });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // Leak the closure: it lives for the whole session (the worker never stops).
    onmessage.forget();
    // A worker that aborts closes no queue: its trap reaches this side as an
    // error event alone, and the page latches its terminal screen (`ARCH-014`).
    let onerror = Closure::<dyn FnMut(web_sys::Event)>::new(|_: web_sys::Event| {
        let reason = "the Sync worker failed";
        tracing::error!("{reason}");
        ambre_canvas::signal_fatal(reason);
    });
    worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();
    worker
}

/// Tells the Sync worker to stop or resume reading the relay, once per crossing
/// of a mark of the inbox (`SYNC-059`).
pub(crate) fn relay_pressure(inbox: &SyncInbox, worker: &web_sys::Worker) {
    let cmd = match inbox.borrow_mut().pressure() {
        Some(ambre_bus::conduit::Pressure::Hold) => SyncCommand::HoldInbound,
        Some(ambre_bus::conduit::Pressure::Release) => SyncCommand::ReleaseInbound,
        None => return,
    };
    match postcard::to_allocvec(&cmd) {
        Ok(bytes) => post_to_worker(worker, &Payload::<WorkerPost>::new(bytes)),
        Err(e) => tracing::error!("[UI->Sync] failed to encode {cmd:?}: {e}"),
    }
}

/// Encode-free postMessage of `post` to the Sync worker, transferring the
/// backing `ArrayBuffer` (zero-copy move, no structured clone of the payload).
pub(crate) fn post_to_worker(worker: &web_sys::Worker, post: &Payload<WorkerPost>) {
    post_to_worker_with_port(worker, post, None);
}

/// Inject the generated token stylesheet (`theme.css`) into `<head>` and set the
/// initial `data-theme`. The chrome then reads colours/spacings as CSS custom
/// properties; the canvas reads the matching `Palette` -- one source, `tokens.ron`.
pub(crate) fn inject_theme(theme: ambre_canvas::Theme) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(head)) = document.query_selector("head")
        && let Ok(style) = document.create_element("style")
    {
        style.set_id("ambre-theme");
        style.set_text_content(Some(ambre_canvas::theme::THEME_CSS));
        let _ = head.append_child(&style);
    }
    apply_theme_attr(theme);
}

ambre_core::define_flag! {
    /// The colour scheme the system asks of a page (`prefers-color-scheme`).
    pub(crate) enum SystemScheme {
        /// The system asks for light pages, or answers nothing.
        Light,
        /// The system asks for dark pages.
        Dark,
    }
}

/// The theme a page boots in: the one the system asks for (`UI-THREAD-065`).
/// The scheme is the one argument, so no theme a session stored reaches the
/// boot (`UI-THREAD-066`).
pub(crate) fn boot_theme(scheme: SystemScheme) -> ambre_canvas::Theme {
    match scheme {
        SystemScheme::Light => ambre_canvas::Theme::Light,
        SystemScheme::Dark => ambre_canvas::Theme::Dark,
    }
}

/// The colour scheme the browser reports for the system. A browser that
/// answers nothing reads as light, the unmarked set of the stylesheet.
pub(crate) fn system_scheme() -> SystemScheme {
    let dark = web_sys::window()
        .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
        .is_some_and(|query| query.matches());
    if dark {
        SystemScheme::Dark
    } else {
        SystemScheme::Light
    }
}

/// Flip the theme for both halves at once, and return the new one.
///
/// The control is what the canvas reads on its next frame; the attribute is
/// what the stylesheet keys on. One act sets both (`UI-THREAD-032`).
pub(crate) fn switch_theme(control: &ambre_canvas::ThemeControl) -> ambre_canvas::Theme {
    let next = control.toggle();
    apply_theme_attr(next);
    next
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

/// The canvases a right-click opens the DOM menu on: the content's, and the
/// thread's of a Murmure page.
pub(crate) const MENU_CANVASES: [ambre_core::StaticKey<ambre_core::Canvases>; 2] = [
    ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT,
    crate::host::THREAD_CANVAS,
];

/// Listen for `contextmenu` on the document. A right-click on one of the
/// `MENU_CANVASES` opens the DOM context menu at the click point, and the
/// browser menu stays shut. The document holds the listener, since a panel
/// mounts its canvas later. While `silent` holds, a right-click opens nothing:
/// a presenting board shows no editing surface.
///
/// While `placed` holds, the page already placed the menu at a point of its
/// own, from a long press or a key. The event then only keeps the browser
/// menu shut, since its point is not the page's. A key's event lands on
/// whatever holds the focus, the text pipe or the body as well as a canvas.
pub(crate) fn install_canvas_context_menu(
    ctx_menu: RwSignal<Option<ambre_core::Point<ambre_core::Px<ambre_core::Css>>>>,
    silent: Signal<bool>,
    placed: Signal<bool>,
) {
    let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
        // The document keeps the listener past the owner of the signals.
        // Once they are gone, the browser keeps its menu.
        let (Some(placed), Some(silent)) = (placed.try_get_untracked(), silent.try_get_untracked())
        else {
            return;
        };
        let on_canvas = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| {
                let id = el.id();
                MENU_CANVASES.iter().any(|canvas| canvas.as_str() == id)
            });
        if !on_canvas && !placed {
            return;
        }
        e.prevent_default();
        if silent || placed {
            return;
        }
        ctx_menu.set(Some(ambre_core::Point::new(
            ambre_core::Px::<ambre_core::Css>::new(e.client_x() as f32),
            ambre_core::Px::<ambre_core::Css>::new(e.client_y() as f32),
        )));
    });
    let _ = leptos::prelude::document()
        .add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref());
    cb.forget();
}

/// Attach the file drop to a board's canvas: a file dropped on it reaches the
/// board's importer at the point it landed. The canvas pointer is read here
/// and in the host alone (`UI-THREAD-001`).
pub(crate) fn install_board_file_drop(
    dropped: RwSignal<Option<toile_chrome::DroppedFile>>,
    toasts: Toasts,
) {
    Effect::new(move |_| {
        let Some(canvas) = leptos::prelude::document()
            .get_element_by_id(ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str())
        else {
            return;
        };
        // Without a `dragover` that prevents the default, the browser navigates
        // away to the dropped file and the drop never fires.
        let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|e: web_sys::DragEvent| {
            e.prevent_default();
        });
        let _ = canvas.add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref());
        over.forget();

        let cb = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
            e.prevent_default();
            let Some(file) = e
                .data_transfer()
                .and_then(|dt| dt.files())
                .and_then(|fs| fs.get(0))
            else {
                return;
            };
            let at = ambre_draw::Point::new(f64::from(e.offset_x()), f64::from(e.offset_y()));
            toile_chrome::import_dropped_file(file, at, dropped, toasts);
        });
        let _ = canvas.add_event_listener_with_callback("drop", cb.as_ref().unchecked_ref());
        cb.forget();
    });
}

/// Attach the file drop to a channel's canvas: each file dropped on it is
/// read, and its region uploads it for the draft
/// (`ambre-spec-murmure-attachments.md`). A thread
/// takes no file, so its canvas takes no drop.
pub(crate) fn install_murmure_file_drop(relay: ambre_murmure::chrome::MurmureRelay) {
    Effect::new(move |_| {
        let Some(canvas) = leptos::prelude::document()
            .get_element_by_id(ambre_core::StaticKey::<ambre_core::Canvases>::CONTENT.as_str())
        else {
            return;
        };
        // Without a `dragover` that prevents the default, the browser navigates
        // away to the dropped file and the drop never fires.
        let over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|e: web_sys::DragEvent| {
            e.prevent_default();
        });
        let _ = canvas.add_event_listener_with_callback("dragover", over.as_ref().unchecked_ref());
        over.forget();

        let cb = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
            e.prevent_default();
            let Some(files) = e.data_transfer().and_then(|dt| dt.files()) else {
                return;
            };
            for file in (0..files.length()).filter_map(|i| files.get(i)) {
                wasm_bindgen_futures::spawn_local(async move {
                    let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await
                    else {
                        tracing::warn!("Murmure: a dropped file could not be read");
                        return;
                    };
                    relay.drop_file(ambre_murmure::ui::uploads::PickedFile {
                        name: file.name(),
                        media_type: ambre_bus::MediaType::new(file.type_()),
                        data: js_sys::Uint8Array::new(&buf).to_vec().into(),
                    });
                });
            }
        });
        let _ = canvas.add_event_listener_with_callback("drop", cb.as_ref().unchecked_ref());
        cb.forget();
    });
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
    // A board's keyboard pipe is a textarea. While no editor is open it is not
    // typed text, so a bare tool key still arms its tool.
    let typing = target
        .dyn_ref::<web_sys::Element>()
        .and_then(|el| el.get_attribute(ambre_canvas::TYPING_ATTR.as_str()));
    if typing.as_deref() == Some("false") {
        return false;
    }
    // A content pipe types no chord that holds the platform modifier, so such
    // a chord is the page's. A page is typed text whenever it has a caret,
    // and a chrome field, which carries no such attribute, keeps its chords.
    if typing.is_some() && (event.ctrl_key() || event.meta_key()) {
        return false;
    }
    let tag = target
        .dyn_ref::<web_sys::Element>()
        .map(|el| el.tag_name())
        .unwrap_or_default();
    let host = ambre_toile::ui::palette::Host::content_editable_if(
        target
            .dyn_ref::<web_sys::HtmlElement>()
            .is_some_and(|el| el.is_content_editable()),
    );
    ambre_toile::ui::palette::is_editable_tag(&tag, host)
}

/// Install the global command-keybinding listener (the keymap's DOM half).
/// This listener owns a `keydown` whose chord resolves in the keymap. The
/// booted application's handler says what the chord does, and whether a text
/// field keeps it (`ARCH-018`). Unmatched keys fall through to the canvas
/// listener (content navigation), so this never steals arrow keys.
#[expect(
    clippy::expect_used,
    reason = "the boot wires the page it was served with, which has no fallback page"
)]
pub(crate) fn install_command_keys(keymap: Keymap, commands: shell::CommandPath) {
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
            // A field the user types into (the editor's hidden input, a chrome
            // text field) may keep the key: the handler decides which.
            let target = if target_is_editable(&event) {
                ambre_chrome::KeyTarget::TextField
            } else {
                ambre_chrome::KeyTarget::Canvas
            };
            if commands.key(id, target) {
                event.prevent_default();
            }
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
///   through a queue. The path always reloads the page (below), and a
///   queue does not survive that.
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
pub(crate) fn open_ambre_file_picker(
    toasts: Toasts,
    on_sealed: Callback<ambre_core::Payload<ambre_core::SealedContainer>>,
    opened: RwSignal<Option<OpenedFile>>,
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
    let _ = input.set_attribute("accept", ambre_chrome::FileAccept::Ambre.as_attr());
    let picker = input.clone();
    let onchange = Closure::<dyn FnMut()>::new(move || {
        if let Some(file) = picker.files().and_then(|fs| fs.get(0)) {
            read_ambre_file(file, toasts, on_sealed, opened);
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
fn read_ambre_file(
    file: web_sys::File,
    toasts: Toasts,
    on_sealed: Callback<ambre_core::Payload<ambre_core::SealedContainer>>,
    opened: RwSignal<Option<OpenedFile>>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await else {
            toasts.error("Ambre could not read that file");
            return;
        };
        let picked = ambre_core::Payload::<ambre_core::PickedContainer>::new(
            js_sys::Uint8Array::new(&buf).to_vec(),
        );

        // A sealed file is a container of the other lineage, and the version
        // byte alone says which (`DOCFILE-010`). Nothing here can read it: it
        // needs a passphrase this thread must ask for, and a key the Sync
        // worker derives. So the bytes go to the caller's prompt, and the
        // plaintext path below resumes on what comes back (`DOCFILE-015`).
        match ambre_client_core::into_lineage(picked) {
            ambre_core::PickedFile::Sealed(file) => on_sealed.run(file),
            // The switchboard holds the store's seal, and resumes from
            // here (`RESID-025`).
            ambre_core::PickedFile::Plaintext(file) => opened.set(Some(OpenedFile(file))),
        }
    });
}

/// A plaintext container the picker read, on its way to the store's seal.
#[derive(Clone)]
pub(crate) struct OpenedFile(ambre_core::Payload<ambre_core::PlaintextContainer>);

impl OpenedFile {
    /// The container the picker read, handed on to its seal.
    pub(crate) fn into_payload(self) -> ambre_core::Payload<ambre_core::PlaintextContainer> {
        self.0
    }
}

/// Attribute a plaintext container, persist its payload sealed and navigate
/// to it.
///
/// Split from [`read_ambre_file`] because a sealed file reaches exactly this
/// point too, one round trip later and by way of the Sync worker. What it holds
/// is a whole plaintext container, so the two lineages share every step after
/// the seal comes off (`DOCFILE-014`).
pub(crate) fn open_plaintext_container(
    container: ambre_core::Payload<ambre_core::PlaintextContainer>,
    toasts: Toasts,
    vault: crate::vault::Vault,
) {
    let Some(Landing {
        document_id,
        doc_type,
        payload,
    }) = read_container(&container, toasts)
    else {
        return;
    };
    // The payload lands as every copy does, sealed under the session's key.
    vault.seal(
        ambre_bus::RowPlace::Document(document_id),
        payload.into_secret(),
        move |sealed| match sealed {
            Some(sealed) => {
                wasm_bindgen_futures::spawn_local(store_container(
                    document_id,
                    doc_type,
                    sealed,
                    toasts,
                ));
            }
            None => toasts.error("Sign in to open that file"),
        },
    );
}

/// Where an opened file lands: a document of its own, of the type the header
/// names, holding the payload.
#[derive(Debug)]
struct Landing {
    /// A fresh identifier, never the one the header names (`DOCFILE-021`).
    document_id: Id<Document>,
    doc_type: ambre_bus::DocType,
    payload: ambre_core::Payload<ambre_core::SnapshotBytes>,
}

/// The document a parsed file lands as.
///
/// A plaintext header is signed by nobody, and a sealed file proves only
/// that its opener knew the passphrase. So the identifier the header names
/// is never trusted. A file naming a document this user holds would replace
/// its local copy and drop its unsent deltas (audit `P50`). The file always
/// lands as a new document, a copy the user can read and share apart
/// (`DOCFILE-021`).
fn landing_of(header: ambre_client_core::docfile::AmbreHeader) -> Landing {
    Landing {
        document_id: Id::<Document>::mint(),
        doc_type: header.doc_type,
        payload: header.payload,
    }
}

/// Where the container lands, once this build can render it. A refusal
/// reaches the user by name.
fn read_container(
    container: &ambre_core::Payload<ambre_core::PlaintextContainer>,
    toasts: Toasts,
) -> Option<Landing> {
    let header = match ambre_client_core::docfile::read_ambre_plaintext(container) {
        Ok(header) => header,
        Err(e) => {
            toasts.error(format!("Ambre cannot open that file: {e}"));
            return None;
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
        return None;
    }
    Some(landing_of(header))
}

/// Store a container's sealed payload as the document's copy, and navigate
/// to it.
async fn store_container(
    document_id: Id<Document>,
    doc_type: ambre_bus::DocType,
    sealed: ambre_core::Payload<ambre_core::SealedRowBytes>,
    toasts: Toasts,
) {
    let store = match ambre_logic::persistence::PersistenceStore::open().await {
        Ok(Ok(store)) => store,
        _ => {
            toasts.error("Ambre could not open its local store for that file");
            return;
        }
    };
    // Durable, not fire-and-forget: the navigation below unloads the page,
    // and an open transaction goes with it.
    let committed = store.save_snapshot_committed(document_id, sealed).await;
    if committed != Ok(ambre_logic::persistence::Commit::Landed) {
        toasts.error("Ambre could not store that file locally");
        return;
    }

    let path = crate::route::doc_path(doc_type, document_id);
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A parsed container naming `document_id`, of a Prose document.
    fn header_naming(document_id: Id<Document>) -> ambre_client_core::docfile::AmbreHeader {
        let payload = ambre_core::Payload::<ambre_core::SnapshotBytes>::new(vec![1, 2, 3]);
        let file = ambre_client_core::docfile::write_ambre_plaintext(
            ambre_bus::DocType::Prose,
            document_id,
            &payload,
        );
        ambre_client_core::docfile::read_ambre_plaintext(&file).expect("the file reads")
    }

    /// `SETTINGS-029`, `SETTINGS-037`, `SETTINGS-039`: the boot hint the page
    /// keeps reads back whole from the storage, and a sign-out deletes it.
    #[wasm_bindgen_test]
    fn the_boot_hint_reads_back_from_the_storage_and_a_sign_out_deletes_it() {
        let hint = ambre_client_core::BootHint {
            theme: ambre_client_core::ThemeChoice::Light,
            interface_language: ambre_client_core::DocumentLocale::parse("fr").expect("a tag"),
        };
        keep_boot_hint(&hint);
        assert_eq!(boot_hint(), Some(hint.clone()));
        let dark = ambre_client_core::BootHint {
            theme: ambre_client_core::ThemeChoice::Dark,
            ..hint
        };
        keep_boot_hint(&dark);
        assert_eq!(boot_hint(), Some(dark), "a later change rewrites it");
        forget_boot_hint();
        assert_eq!(boot_hint(), None);
    }

    /// `SETTINGS-032`: a page boots in the theme of the hint the storage
    /// holds, whatever the system asks for.
    #[wasm_bindgen_test]
    fn a_page_boots_in_the_theme_of_the_stored_boot_hint() {
        keep_boot_hint(&ambre_client_core::BootHint {
            theme: ambre_client_core::ThemeChoice::Light,
            interface_language: ambre_client_core::DocumentLocale::NONE,
        });
        for system in [ambre_canvas::Theme::Light, ambre_canvas::Theme::Dark] {
            assert_eq!(
                crate::settings_view::boot_theme(boot_hint().as_ref(), system),
                ambre_canvas::Theme::Light
            );
        }
        forget_boot_hint();
        assert_eq!(
            crate::settings_view::boot_theme(boot_hint().as_ref(), ambre_canvas::Theme::Dark),
            ambre_canvas::Theme::Dark,
            "with no hint the system decides"
        );
    }

    /// An opened file never lands on the document its header names, so it
    /// replaces no local copy and drops no local delta (`DOCFILE-021`,
    /// audit `P50`). The type and the payload stay the file's.
    #[wasm_bindgen_test]
    fn an_opened_file_never_lands_on_the_document_its_header_names() {
        let held = Id::<Document>::mint();
        let landing = landing_of(header_naming(held));
        assert_ne!(
            landing.document_id, held,
            "the header names a held document"
        );
        assert_eq!(landing.doc_type, ambre_bus::DocType::Prose);
        assert_eq!(landing.payload.as_bytes(), &[1, 2, 3]);
    }

    /// Two opens of one file land as two documents, so the second open
    /// cannot replace what the first one landed either (`DOCFILE-021`).
    #[wasm_bindgen_test]
    fn two_opens_of_one_file_land_as_two_documents() {
        let named = Id::<Document>::mint();
        let first = landing_of(header_naming(named));
        let second = landing_of(header_naming(named));
        assert_ne!(first.document_id, second.document_id);
    }

    /// The page point at `(x, y)`, in CSS px.
    fn css_point(x: f32, y: f32) -> ambre_core::Point<ambre_core::Px<ambre_core::Css>> {
        ambre_core::Point::new(
            ambre_core::Px::<ambre_core::Css>::new(x),
            ambre_core::Px::<ambre_core::Css>::new(y),
        )
    }

    /// The document's `data-theme`, as the stylesheet reads it.
    fn document_theme() -> Option<String> {
        web_sys::window()?
            .document()?
            .document_element()?
            .get_attribute("data-theme")
    }

    /// One switch reaches the canvas control and the document attribute
    /// together (`UI-THREAD-032`).
    ///
    /// A switch that set one half alone would leave the chrome and the
    /// canvas in different themes.
    #[wasm_bindgen_test]
    fn a_theme_switch_reaches_both_halves() {
        let control = ambre_canvas::ThemeControl::new(ambre_canvas::Theme::Dark);
        apply_theme_attr(control.get());
        assert_eq!(document_theme().as_deref(), Some("dark"));

        let next = switch_theme(&control);
        assert_eq!(next, ambre_canvas::Theme::Light);
        assert_eq!(control.get(), ambre_canvas::Theme::Light, "the canvas half");
        assert_eq!(
            document_theme().as_deref(),
            Some("light"),
            "the browser half"
        );

        switch_theme(&control);
        assert_eq!(control.get(), ambre_canvas::Theme::Dark);
        assert_eq!(document_theme().as_deref(), Some("dark"));
    }

    /// A page boots in the theme the system asks for, light or dark, and in
    /// the one the browser reports here (`UI-THREAD-065`).
    #[wasm_bindgen_test]
    fn a_page_boots_in_the_theme_the_system_asks_for() {
        assert_eq!(boot_theme(SystemScheme::Light), ambre_canvas::Theme::Light);
        assert_eq!(boot_theme(SystemScheme::Dark), ambre_canvas::Theme::Dark);
        let asked = web_sys::window()
            .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
            .expect("a media query");
        let expected = if asked.matches() {
            SystemScheme::Dark
        } else {
            SystemScheme::Light
        };
        assert_eq!(system_scheme(), expected);
    }

    /// What `target_is_editable` reads off a real `keydown` dispatched on an
    /// element of this tag, with this `contenteditable` value.
    fn keydown_reads_editable(tag: &str, content_editable: Option<&str>) -> bool {
        use std::cell::Cell;
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a document");
        let el = document.create_element(tag).expect("an element");
        if let Some(value) = content_editable {
            el.set_attribute("contenteditable", value)
                .expect("an attribute");
        }
        document
            .body()
            .expect("a body")
            .append_child(&el)
            .expect("attached");
        let read = Rc::new(Cell::new(None));
        let sink = read.clone();
        let listener = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev| {
            sink.set(Some(target_is_editable(&ev)));
        });
        el.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
            .expect("a listener");
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("v");
        let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
            .expect("an event");
        el.dispatch_event(&ev).expect("dispatched");
        el.remove();
        read.get().expect("the listener ran")
    }

    /// A key typed into a text field or a contenteditable host reads as typing,
    /// so a bare tool chord leaves it alone. A key on any other element does
    /// not. The decision is read off the event target, not passed in.
    #[wasm_bindgen_test]
    fn a_key_typed_into_a_field_reads_as_typing() {
        assert!(keydown_reads_editable("input", None));
        assert!(keydown_reads_editable("textarea", None));
        assert!(keydown_reads_editable("div", Some("true")));
        assert!(!keydown_reads_editable("div", None));
        assert!(!keydown_reads_editable("button", None));
    }

    /// A right-click on `id` at (12, 34): whether the browser menu stayed
    /// shut. The element lives in the body for the click alone.
    fn right_click(id: &str) -> bool {
        let el = leptos::prelude::document()
            .create_element("canvas")
            .expect("a canvas");
        el.set_id(id);
        menu_event_on(el)
    }

    /// The menu event a key raises on the text pipe that holds the focus:
    /// whether the browser menu stayed shut.
    fn menu_key_on_the_pipe() -> bool {
        let el = leptos::prelude::document()
            .create_element("textarea")
            .expect("a textarea");
        el.set_class_name("ambre-input");
        menu_event_on(el)
    }

    /// A `contextmenu` at (12, 34) on `el`, in the body for the event alone.
    fn menu_event_on(el: web_sys::Element) -> bool {
        let document = leptos::prelude::document();
        document
            .body()
            .expect("a body")
            .append_child(&el)
            .expect("in the body");
        let init = web_sys::PointerEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_client_x(12);
        init.set_client_y(34);
        let ev = web_sys::PointerEvent::new_with_event_init_dict("contextmenu", &init)
            .expect("an event");
        el.dispatch_event(&ev).expect("dispatched");
        el.remove();
        ev.default_prevented()
    }

    /// A right-click on the content's canvas or on the thread's, mounted
    /// after the listener, opens the menu at its point. While `silent`
    /// holds it opens nothing, and a right-click elsewhere keeps the
    /// browser menu (the Murmure message list spec).
    #[wasm_bindgen_test]
    fn a_right_click_on_either_canvas_opens_the_menu_at_its_point() {
        // A scoped owner: one left current would dispose the signals of the
        // tests that follow.
        Owner::new().with(right_clicks_open_the_menu);
    }

    fn right_clicks_open_the_menu() {
        let ctx_menu = RwSignal::new(None);
        let silent = RwSignal::new(false);
        install_canvas_context_menu(ctx_menu, silent.into(), Signal::stored(false));
        for id in MENU_CANVASES {
            ctx_menu.set(None);
            assert!(
                right_click(id.as_str()),
                "{id}: the browser menu stays shut"
            );
            assert_eq!(
                ctx_menu.get_untracked(),
                Some(css_point(12.0, 34.0)),
                "{id}"
            );
        }
        ctx_menu.set(None);
        silent.set(true);
        assert!(right_click(crate::host::THREAD_CANVAS.as_str()));
        assert_eq!(ctx_menu.get_untracked(), None, "silent opens nothing");
        silent.set(false);
        assert!(!right_click("elsewhere"), "the browser menu of the page");
        assert_eq!(ctx_menu.get_untracked(), None);
    }

    /// A menu the page placed from a key keeps its point. The event the key
    /// raises on a canvas or on the text pipe only keeps the browser menu
    /// shut. With nothing placed, the pipe keeps its browser menu (the
    /// Murmure message list spec).
    ///
    /// The pipe stands for any element outside the canvases.
    #[wasm_bindgen_test]
    fn a_menu_the_page_placed_keeps_its_point_and_the_browsers_stays_shut() {
        Owner::new().with(a_placed_menu_keeps_its_point);
    }

    fn a_placed_menu_keeps_its_point() {
        let ctx_menu = RwSignal::new(None);
        let placed = RwSignal::new(false);
        install_canvas_context_menu(ctx_menu, Signal::stored(false), placed.into());
        assert!(!menu_key_on_the_pipe(), "nothing placed: the browser's");
        assert_eq!(ctx_menu.get_untracked(), None);

        placed.set(true);
        ctx_menu.set(Some(css_point(1.0, 2.0)));
        assert!(menu_key_on_the_pipe(), "the pipe's browser menu stays shut");
        assert!(
            right_click(crate::host::THREAD_CANVAS.as_str()),
            "and the canvas's"
        );
        assert_eq!(
            ctx_menu.get_untracked(),
            Some(css_point(1.0, 2.0)),
            "the point stays"
        );
    }

    /// `CHROME-103`: a pipe that is not typed text gives a bare key to the
    /// keymap. Once it is typed text again, the key stays on the pipe.
    #[wasm_bindgen_test]
    fn a_pipe_that_is_not_typing_gives_its_keys_to_the_keymap() {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a document");
        let body = document.body().expect("a body");
        let pipe = ambre_canvas::TextInput::create(&document, &body);
        let reads = || {
            use std::cell::Cell;
            use std::rc::Rc;
            use wasm_bindgen::JsCast;
            use wasm_bindgen::closure::Closure;

            let read = Rc::new(Cell::new(None));
            let sink = read.clone();
            let listener = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev| {
                sink.set(Some(target_is_editable(&ev)));
            });
            let el = pipe.element();
            el.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
                .expect("a listener");
            let init = web_sys::KeyboardEventInit::new();
            init.set_key("v");
            let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .expect("an event");
            el.dispatch_event(&ev).expect("dispatched");
            el.remove_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
                .expect("removed");
            read.get().expect("the listener ran")
        };
        assert!(reads(), "a pipe is typed text by default");
        pipe.set_typing(ambre_canvas::Typing::Commands);
        assert!(!reads(), "a board with no editor open");
        pipe.set_typing(ambre_canvas::Typing::Text);
        assert!(reads(), "a board with an editor open");
    }

    /// Whether `target_is_editable` reads `el` as a text field, for a key
    /// `key` with the primary modifier held or not.
    fn reads_editable(el: &web_sys::Element, key: &str, ctrl: bool) -> bool {
        use std::cell::Cell;
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;

        let read = Rc::new(Cell::new(None));
        let sink = read.clone();
        let listener = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev| {
            sink.set(Some(target_is_editable(&ev)));
        });
        el.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
            .expect("a listener");
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_ctrl_key(ctrl);
        let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
            .expect("an event");
        el.dispatch_event(&ev).expect("dispatched");
        el.remove_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
            .expect("removed");
        read.get().expect("the listener ran")
    }

    /// A page is typed text, and its pipe once yielded `Ctrl+B` as a chrome
    /// field does, so the bold chord never reached a Prose page. A chord with
    /// the modifier on a content pipe is the page's. A chrome field keeps it,
    /// and a bare key stays typed text.
    #[wasm_bindgen_test]
    fn a_modifier_chord_on_a_typing_pipe_is_the_pages() {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("a document");
        let body = document.body().expect("a body");
        // A page never sets the pipe's typing: it is typed text from the start.
        let pipe = ambre_canvas::TextInput::create(&document, &body);
        assert!(
            !reads_editable(pipe.element(), "b", true),
            "Ctrl+B on the pipe"
        );
        assert!(reads_editable(pipe.element(), "b", false), "b on the pipe");

        let field = document.create_element("input").expect("an input");
        body.append_child(&field).expect("attached");
        assert!(
            reads_editable(&field, "b", true),
            "Ctrl+B in a chrome field"
        );
        field.remove();
    }
}
