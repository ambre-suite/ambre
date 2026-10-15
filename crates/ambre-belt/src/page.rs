//! One application page under test: the server, the browser, the downloads.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::driver::Browser;
use crate::relay::Relay;
use crate::serve;

/// How long the application may take to boot in a headless browser. The wasm
/// compiles there with no cache, and the canvas draws on a software device.
pub const BOOT: Duration = Duration::from_secs(60);

/// How long an answer to one gesture may take.
pub const STEP: Duration = Duration::from_secs(10);

/// The application, served in front of a relay and opened in one browser.
pub struct Page {
    pub browser: Browser,
    pub origin: String,
    pub downloads: PathBuf,
    /// Held so the relay outlives every page in front of it.
    _relay: Arc<Relay>,
}

impl Page {
    /// Starts a relay, signs a new user in and opens a new document of `kind`
    /// (`toile`, `trame` or `prose`). A document is ready only once the Sync
    /// worker has opened it, so no document opens without a relay.
    pub async fn new_document(kind: &str) -> Self {
        let page = Self::home(Arc::new(Relay::start().await)).await;
        page.sign_in(&format!("belt-{}", uuid::Uuid::new_v4()))
            .await;
        page.open_new(kind).await;
        page
    }

    /// Opens a new document of `kind` in this page's session.
    pub async fn open_new(&self, kind: &str) {
        self.browser
            .goto(&format!("{}/{kind}/{}", self.origin, uuid::Uuid::now_v7()))
            .await;
        self.wait_canvas().await;
    }

    /// Serves the build in front of `relay` and opens the home screen, signed
    /// out.
    pub async fn home(relay: Arc<Relay>) -> Self {
        let downloads = std::env::temp_dir().join(format!("belt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&downloads).expect("a download directory");
        let origin = serve::spawn(serve::dist_dir(), relay.base.clone()).await;
        let browser = Browser::start(&downloads).await;
        browser.goto(&format!("{origin}/")).await;
        Self {
            browser,
            origin,
            downloads,
            _relay: relay,
        }
    }

    /// The relay this page stands in front of, so a second page can join it.
    pub fn relay(&self) -> Arc<Relay> {
        Arc::clone(&self._relay)
    }

    /// Invites the user `name` to the open document as an editor, through the
    /// share dialog, and confirms the first sight of their key. Waits until
    /// the member list shows them.
    pub async fn share_with(&self, name: &str) {
        let name = Value::String(name.to_owned());
        self.browser
            .wait_for(
                "the share dialog",
                &format!(
                    "const i = document.querySelector('.ambre-share-input'); \
                     if (!i || !i.checkVisibility()) {{ \
                       document.querySelector('[aria-label=\"Share document\"]').click(); \
                       return false; }} \
                     i.value = {name}; \
                     i.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     document.querySelector('.ambre-share-invite').requestSubmit(); \
                     return true;"
                ),
                STEP,
            )
            .await;
        self.browser
            .wait_for(
                "the key confirmation",
                "const b = document.querySelector('.ambre-tofu-confirm'); \
                 if (!b) return false; b.click(); return true;",
                STEP,
            )
            .await;
        self.browser
            .wait_for(
                &format!("{name} among the members"),
                &format!(
                    "if (![...document.querySelectorAll('.ambre-share-member-name')] \
                       .some(e => e.textContent.includes({name}))) return false; \
                     document.querySelector('.ambre-share-panel .ambre-dialog-actions button').click(); \
                     return true;"
                ),
                STEP,
            )
            .await;
    }

    /// Waits until the document's canvas is laid out.
    pub async fn wait_canvas(&self) {
        self.browser
            .wait_for(
                "the canvas",
                "const c = document.querySelector('canvas'); \
                 return c && c.getBoundingClientRect().width > 0 \
                   && !document.querySelector('.ambre-doc-loading');",
                BOOT,
            )
            .await;
    }

    /// Signs in on the home screen as `name`, through the dev sign-in field,
    /// and waits for the document list.
    pub async fn sign_in(&self, name: &str) {
        let name = Value::String(name.to_owned());
        self.browser
            .wait_for(
                "the sign-in field",
                &format!(
                    "const i = document.querySelector('.ambre-signin-input'); \
                     const b = document.querySelector('.ambre-signin-btn'); \
                     if (!i || !b) return false; \
                     i.value = {name}; \
                     i.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     b.click(); \
                     return true;"
                ),
                BOOT,
            )
            .await;
        self.browser
            .wait_for(
                "the document list",
                "return !!document.querySelector('.ambre-doc-list');",
                STEP,
            )
            .await;
    }

    /// The canvas's top-left corner in the viewport, in CSS pixels.
    pub async fn canvas_origin(&self) -> (f64, f64) {
        let r = self
            .browser
            .script(
                "const r = document.querySelector('canvas').getBoundingClientRect(); \
                 return [r.left, r.top];",
                json!([]),
            )
            .await;
        (r[0].as_f64().unwrap_or(0.0), r[1].as_f64().unwrap_or(0.0))
    }

    /// The colour the screen shows at `at`, in canvas pixels, as red, green
    /// and blue. It reads a screenshot, so it sees what the user sees.
    pub async fn color_at(&self, at: (f64, f64)) -> [u8; 3] {
        use base64::Engine;

        let (ox, oy) = self.canvas_origin().await;
        let shot = self
            .browser
            .cdp(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "clip": { "x": ox + at.0, "y": oy + at.1, "width": 1, "height": 1, "scale": 1 },
                }),
            )
            .await;
        let data = shot["data"].as_str().expect("a screenshot");
        let png = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("a base64 screenshot");
        let pixel = image::load_from_memory(&png)
            .expect("a PNG screenshot")
            .to_rgb8();
        pixel.get_pixel(0, 0).0
    }

    /// The text of the live region a screen reader hears beside the canvas.
    pub async fn announced(&self) -> String {
        self.browser
            .script(
                "const l = document.querySelector('.ambre-a11y-live'); \
                 return l ? l.textContent : '';",
                json!([]),
            )
            .await
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Waits until the live region reads `text`.
    pub async fn wait_announced(&self, text: &str) {
        self.browser
            .wait_for(
                &format!("the announcement {text:?}"),
                &format!(
                    "const l = document.querySelector('.ambre-a11y-live'); \
                     return l && l.textContent === {};",
                    Value::String(text.to_owned())
                ),
                STEP,
            )
            .await;
    }

    /// Waits until a toast reads `text`, and answers every toast's text.
    pub async fn wait_toast(&self, text: &str) -> Value {
        self.browser
            .wait_for(
                &format!("the toast {text:?}"),
                &format!(
                    "const t = [...document.querySelectorAll('[role=status]')] \
                       .map(e => e.textContent); \
                     return t.some(s => s.includes({})) ? t : null;",
                    Value::String(text.to_owned())
                ),
                STEP,
            )
            .await
    }

    /// Puts the keyboard focus on the canvas, as a click there would.
    pub async fn focus_canvas(&self) {
        self.browser
            .script("document.querySelector('canvas').focus();", json!([]))
            .await;
    }

    /// Runs the command whose palette row reads `label`: the palette chord, the
    /// label typed into the query, then the row clicked.
    pub async fn command(&self, label: &str) {
        self.focus_canvas().await;
        self.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E009}" },
                { "type": "keyDown", "value": "k" },
                { "type": "keyUp", "value": "k" },
                { "type": "keyUp", "value": "\u{E009}" },
            ]}]))
            .await;
        let label = Value::String(label.to_owned());
        self.browser
            .wait_for(
                &format!("the palette row {label}"),
                &format!(
                    "const q = document.querySelector('.ambre-palette-query'); \
                     if (!q) return false; \
                     q.value = {label}; \
                     q.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     const row = [...document.querySelectorAll('.ambre-palette-row')] \
                       .find(r => r.querySelector('.ambre-palette-label').textContent === {label}); \
                     if (!row) return false; \
                     row.click(); \
                     return true;"
                ),
                STEP,
            )
            .await;
    }

    /// Makes the next file picker the page opens wait for [`Page::pick`]
    /// instead of opening a dialog, which no driver can answer.
    pub async fn arm_picker(&self) {
        self.browser
            .script(
                "window.__beltPicker = null; \
                 const click = HTMLInputElement.prototype.click; \
                 HTMLInputElement.prototype.click = function () { \
                   if (this.type === 'file') { window.__beltPicker = this; return; } \
                   return click.call(this); \
                 };",
                json!([]),
            )
            .await;
    }

    /// Answers the armed picker with one file, as a user choosing it would.
    pub async fn pick(&self, name: &str, media_type: &str, bytes: &[u8]) {
        self.browser
            .wait_for("a file picker", "return !!window.__beltPicker;", STEP)
            .await;
        self.browser
            .script(
                "const [name, type, bytes] = arguments; \
                 const dt = new DataTransfer(); \
                 dt.items.add(new File([new Uint8Array(bytes)], name, { type })); \
                 const input = window.__beltPicker; \
                 window.__beltPicker = null; \
                 input.files = dt.files; \
                 input.dispatchEvent(new Event('change'));",
                json!([name, media_type, bytes]),
            )
            .await;
    }

    /// Drops one file on the canvas at `at`, in canvas pixels.
    pub async fn drop_file(&self, at: (f64, f64), name: &str, media_type: &str, bytes: &[u8]) {
        let (ox, oy) = self.canvas_origin().await;
        self.browser
            .script(
                "const [x, y, name, type, bytes] = arguments; \
                 const dt = new DataTransfer(); \
                 dt.items.add(new File([new Uint8Array(bytes)], name, { type })); \
                 const c = document.getElementById('ambre-canvas'); \
                 const at ={ bubbles: true, cancelable: true, clientX: x, clientY: y, dataTransfer: dt }; \
                 c.dispatchEvent(new DragEvent('dragover', at)); \
                 c.dispatchEvent(new DragEvent('drop', at));",
                json!([ox + at.0, oy + at.1, name, media_type, bytes]),
            )
            .await;
    }

    /// Waits for one finished download, and answers its name and its bytes.
    pub async fn download(&self) -> (String, Vec<u8>) {
        let deadline = std::time::Instant::now() + STEP;
        loop {
            let done = std::fs::read_dir(&self.downloads)
                .expect("the download directory")
                .filter_map(Result::ok)
                .map(|e| e.path())
                .find(|p| {
                    let hidden = p
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'));
                    !hidden
                        && p.extension()
                            .is_none_or(|e| e != "crdownload" && e != "tmp")
                });
            if let Some(path) = done {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                return (name, std::fs::read(&path).expect("the downloaded file"));
            }
            assert!(std::time::Instant::now() < deadline, "no download arrived");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// A worker the DevTools Protocol holds in its debugger: the session that
/// holds it.
pub struct Paused(Value);

impl Page {
    /// The browser's workers, every tab's: their target ids and titles.
    pub async fn workers(&self) -> Vec<(String, String)> {
        let targets = self.browser.cdp("Target.getTargets", json!({})).await;
        targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["type"] == "worker")
            .filter_map(|t| {
                Some((
                    t["targetId"].as_str()?.to_owned(),
                    t["title"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    /// Pauses the worker `target` at its next statement. It runs that
    /// statement when the next message reaches it, and stops there. No other
    /// way stops a worker from outside without a hook in the product.
    pub async fn pause_worker(&self, target: &str) -> Paused {
        let attached = self
            .browser
            .cdp(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": false }),
            )
            .await;
        let paused = Paused(attached["sessionId"].clone());
        self.to_worker(&paused, 1, "Debugger.enable").await;
        self.to_worker(&paused, 2, "Debugger.pause").await;
        paused
    }

    /// Lets a paused worker run again.
    pub async fn resume_worker(&self, paused: Paused) {
        self.to_worker(&paused, 3, "Debugger.resume").await;
    }

    /// Sends one DevTools command to the worker behind `paused`.
    async fn to_worker(&self, paused: &Paused, id: u32, method: &str) {
        self.browser
            .cdp(
                "Target.sendMessageToTarget",
                json!({
                    "sessionId": paused.0,
                    "message": json!({ "id": id, "method": method }).to_string(),
                }),
            )
            .await;
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.downloads);
    }
}

/// A mouse drag from `from` to `to` in viewport pixels, in `steps` moves.
pub fn drag(from: (f64, f64), to: (f64, f64), steps: u32) -> Value {
    drag_with(from, to, steps, None)
}

/// A drag that holds still at `to` before the release. A pan released this
/// way leaves no glide (`KINETIC-004`), so the view stops where it was let go.
pub fn drag_still(from: (f64, f64), to: (f64, f64), steps: u32) -> Value {
    drag_with(from, to, steps, Some(250))
}

fn drag_with(from: (f64, f64), to: (f64, f64), steps: u32, hold_ms: Option<u32>) -> Value {
    let mut moves = vec![
        json!({ "type": "pointerMove", "x": from.0 as i64, "y": from.1 as i64 }),
        json!({ "type": "pointerDown", "button": 0 }),
    ];
    for i in 1..=steps {
        let t = f64::from(i) / f64::from(steps);
        let x = from.0 + (to.0 - from.0) * t;
        let y = from.1 + (to.1 - from.1) * t;
        moves.push(json!({ "type": "pointerMove", "duration": 16, "x": x as i64, "y": y as i64 }));
    }
    if let Some(ms) = hold_ms {
        moves.push(json!({ "type": "pause", "duration": ms }));
    }
    moves.push(json!({ "type": "pointerUp", "button": 0 }));
    json!({ "type": "pointer", "id": "mouse", "parameters": { "pointerType": "mouse" }, "actions": moves })
}

/// One key pressed and released.
pub fn key(value: &str) -> Value {
    json!({ "type": "key", "id": "keyboard", "actions": [
        { "type": "keyDown", "value": value },
        { "type": "keyUp", "value": value },
    ]})
}
