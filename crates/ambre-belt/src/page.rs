//! One application page under test: the server, the browser, the downloads.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::driver::{Browser, Engine as BrowserEngine};
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

    /// A page of `browser` on `origin`, served by a host of its own in front of `relay`.
    pub fn over(browser: Browser, origin: String, downloads: PathBuf, relay: Arc<Relay>) -> Self {
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
    /// the member list shows them. The invite link the dialog shows is kept
    /// for [`Page::invite_link`].
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
                     const l = document.querySelector('.ambre-share-link-input'); \
                     if (!l || !l.value) return false; \
                     window.__beltInviteLink = l.value; \
                     document.querySelector('.ambre-share-panel .ambre-dialog-actions button').click(); \
                     return true;"
                ),
                STEP,
            )
            .await;
    }

    /// The path and the fragment of the invite link that the last
    /// [`Page::share_with`] read off the dialog. The link is the document's
    /// route, pinned to this page's user as the inviter (`SYNC-208`). An open with no link fails
    /// closed (`SYNC-210`).
    pub async fn invite_link(&self) -> String {
        self.browser
            .wait_for("the invite link", "return !!window.__beltInviteLink;", STEP)
            .await;
        let link = self
            .browser
            .script(
                "const u = new URL(window.__beltInviteLink); return u.pathname + u.hash;",
                json!([]),
            )
            .await;
        link.as_str().expect("an invite link").to_owned()
    }

    /// Opens the share dialog, presses its copy button with the pointer, as a
    /// hand does, and answers what the clipboard then holds. A click a script
    /// dispatches carries no user activation, and Firefox refuses a clipboard
    /// write without one (`SYNC-245`).
    pub async fn copy_invite_link(&self) -> String {
        let at = self
            .browser
            .wait_for(
                "the copy button of the invite link",
                "const b = document.querySelector('.ambre-share-link-copy'); \
                 if (!b || !b.checkVisibility()) { \
                   document.querySelector('[aria-label=\"Share document\"]').click(); \
                   return null; } \
                 b.scrollIntoView(); \
                 const r = b.getBoundingClientRect(); \
                 return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];",
                STEP,
            )
            .await;
        let (x, y) = (at[0].as_i64().unwrap_or(0), at[1].as_i64().unwrap_or(0));
        self.browser
            .perform(json!([{ "type": "pointer", "id": "mouse",
                "parameters": { "pointerType": "mouse" }, "actions": [
                    { "type": "pointerMove", "x": x, "y": y },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 },
                ] }]))
            .await;
        let outcome = self
            .browser
            .wait_for(
                "the outcome of the copy",
                "const o = document.querySelector('.ambre-share-link-outcome'); \
                 return o && o.textContent ? o.textContent : null;",
                STEP,
            )
            .await;
        assert_eq!(outcome, json!("Copied"), "the copy did not land");
        self.browser.permit("clipboard-read").await;
        let held = self
            .browser
            .script_async(
                "const done = arguments[0]; \
                 navigator.clipboard.readText().then(done).catch(e => done('refused: ' + e.name));",
                json!([]),
            )
            .await;
        held.as_str().expect("the text of the clipboard").to_owned()
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
        let png = match self.browser.engine() {
            BrowserEngine::Chrome => {
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
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .expect("a base64 screenshot")
            }
            // Firefox has no DevTools Protocol to clip with: the whole
            // viewport is shot, and the pixel read out of it. The headless
            // window has a device pixel ratio of 1.
            BrowserEngine::Firefox => self.browser.screenshot().await,
        };
        let shot = image::load_from_memory(&png)
            .expect("a PNG screenshot")
            .to_rgb8();
        let (x, y) = match self.browser.engine() {
            BrowserEngine::Chrome => (0, 0),
            BrowserEngine::Firefox => ((ox + at.0) as u32, (oy + at.1) as u32),
        };
        shot.get_pixel(x, y).0
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
        self.palette_row(label).await;
        self.browser
            .script(&format!("{}.click();", palette_row_js(label)), json!([]))
            .await;
    }

    /// Opens the palette with its chord, types `label` into the query, and
    /// waits for the row that reads `label`. The row is left unclicked.
    pub async fn palette_row(&self, label: &str) {
        self.focus_canvas().await;
        self.browser
            .perform(json!([{ "type": "key", "id": "keyboard", "actions": [
                { "type": "keyDown", "value": "\u{E009}" },
                { "type": "keyDown", "value": "k" },
                { "type": "keyUp", "value": "k" },
                { "type": "keyUp", "value": "\u{E009}" },
            ]}]))
            .await;
        let query = Value::String(label.to_owned());
        self.browser
            .wait_for(
                &format!("the palette row {query}"),
                &format!(
                    "const q = document.querySelector('.ambre-palette-query'); \
                     if (!q) return false; \
                     q.value = {query}; \
                     q.dispatchEvent(new Event('input', {{ bubbles: true }})); \
                     return !!{};",
                    palette_row_js(label)
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
    ///
    /// A download in progress leaves a partial file. Chrome writes a
    /// `.crdownload`, and Firefox a `.part` beside an empty file under the
    /// final name, then moves the `.part` onto that name. A file is taken
    /// once two looks in a row list the same files and sizes with no partial
    /// file. A file gone between the look and the read is not finished yet.
    pub async fn download(&self) -> (String, Vec<u8>) {
        let deadline = std::time::Instant::now() + STEP;
        let mut before = Vec::new();
        loop {
            let now = look(&self.downloads);
            if let Some(found) = settled(&before, &now).and_then(take) {
                return found;
            }
            before = now;
            assert!(std::time::Instant::now() < deadline, "no download arrived");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// One look at the download directory: each visible file and its size, in
/// name order. An entry that leaves while it is read is left out.
fn look(dir: &std::path::Path) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .expect("the download directory")
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.len())))
        .collect();
    files.sort();
    files
}

/// The file of a finished download: two looks in a row list the same files
/// with the same sizes, and no partial file.
fn settled<'a>(before: &[(PathBuf, u64)], now: &'a [(PathBuf, u64)]) -> Option<&'a PathBuf> {
    let partial = now.iter().any(|(p, _)| {
        p.extension()
            .is_some_and(|e| e == "crdownload" || e == "tmp" || e == "part")
    });
    (!partial && before == now)
        .then(|| now.first().map(|(p, _)| p))
        .flatten()
}

/// The name and the bytes of `path`, or `None` when the file left after the
/// look. The browser may still be moving its partial file onto the name.
fn take(path: &PathBuf) -> Option<(String, Vec<u8>)> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match std::fs::read(path) {
        Ok(bytes) => Some((name, bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("the downloaded file {}: {e}", path.display()),
    }
}

/// A worker the DevTools Protocol holds in its debugger: the session that
/// holds it.
pub struct Paused(Value);

impl Page {
    /// The browser's workers, every tab's: their target ids and titles.
    async fn workers(&self) -> Vec<(String, String)> {
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

    /// The target id of this page's one worker whose title holds `title`.
    ///
    /// A worker of the page before the last navigation lives on for a few
    /// seconds, under the same title. So this waits until one such worker
    /// is left, and a pause never goes to a worker that is about to end.
    pub async fn worker(&self, title: &str) -> String {
        let deadline = tokio::time::Instant::now() + STEP;
        loop {
            let workers = self.workers().await;
            let mut found = workers.iter().filter(|(_, t)| t.contains(title));
            if let (Some((id, _)), None) = (found.next(), found.next()) {
                return id.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no single {title} worker among {workers:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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

/// A script expression for the palette row that reads `label`, or undefined.
pub fn palette_row_js(label: &str) -> String {
    let label = Value::String(label.to_owned());
    format!(
        "[...document.querySelectorAll('.ambre-palette-row')] \
           .find(r => r.querySelector('.ambre-palette-label').textContent === {label})"
    )
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

#[cfg(test)]
mod download_tests {
    use super::{look, settled, take};

    /// A fresh directory under the system temporary directory.
    fn dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("belt-dl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("a directory");
        dir
    }

    /// Firefox's sequence: an empty file under the final name beside a
    /// `.part`, then the `.part` moved onto the name. No look takes the
    /// empty file, and the moved one is taken once a second look agrees.
    #[test]
    fn a_download_is_taken_once_two_looks_agree_with_no_partial_file() {
        let dir = dir();
        let (name, part) = (dir.join("a.md"), dir.join("a.md.part"));
        std::fs::write(&name, b"").expect("the empty file");
        let first = look(&dir);
        assert_eq!(settled(&[], &first), None, "one look is not enough");
        std::fs::write(&part, b"# A").expect("the partial file");
        let second = look(&dir);
        assert_eq!(settled(&first, &second), None, "a partial file is left");
        std::fs::rename(&part, &name).expect("the move");
        let third = look(&dir);
        assert_eq!(settled(&second, &third), None, "the sizes moved");
        let fourth = look(&dir);
        let path = settled(&third, &fourth).expect("two looks agree");
        assert_eq!(take(path), Some(("a.md".to_owned(), b"# A".to_vec())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file a look listed and that left before the read is no download
    /// yet: the wait goes on instead of failing.
    #[test]
    fn a_file_gone_between_the_look_and_the_read_is_not_taken() {
        let dir = dir();
        let name = dir.join("a.md");
        std::fs::write(&name, b"# A").expect("the file");
        let seen = look(&dir);
        let path = settled(&seen, &seen).expect("two looks agree").clone();
        std::fs::remove_file(&name).expect("the browser moves it away");
        assert_eq!(take(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
