//! A WebDriver client, as small as the belt needs (W3C WebDriver, level 2).
//!
//! It starts a driver, opens one headless browser session, and speaks the
//! handful of commands a test uses: navigate, run a script, perform input
//! actions. Each call is one HTTP request with a JSON body.
//!
//! Two engines answer: Chrome through chromedriver, and Firefox through
//! geckodriver under the Cargo feature `firefox`. A feature, not a variable,
//! picks the engine. So a test that the other engine cannot drive is marked
//! ignored at compile time, and the runner reports it as ignored.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The browser engine a build of the belt drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Chrome through chromedriver: the default.
    Chrome,
    /// Firefox through geckodriver: the Cargo feature `firefox`.
    Firefox,
}

impl Engine {
    /// The engine this build of the belt drives.
    pub const fn current() -> Self {
        if cfg!(feature = "firefox") {
            Self::Firefox
        } else {
            Self::Chrome
        }
    }
}

/// The chromedriver binary: `CHROMEDRIVER`, or `chromedriver` on the path.
fn chromedriver() -> String {
    std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into())
}

/// The geckodriver binary: `GECKODRIVER`, or `geckodriver` on the path.
fn geckodriver() -> String {
    std::env::var("GECKODRIVER").unwrap_or_else(|_| "geckodriver".into())
}

/// A free local port, picked by binding to port 0.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free local port")
}

/// What a failed wait reports of the page: where it is, what the live region
/// and the toasts say, and which element holds the focus.
const SNAPSHOT: &str = "const t = s => [...document.querySelectorAll(s)].map(e => e.textContent); \
    const f = document.activeElement; \
    return { url: location.href, isolated: self.crossOriginIsolated, live: t('.ambre-a11y-live'), toasts: t('[role=status]'), \
      focus: f ? f.tagName + '.' + f.className : null, \
      body: document.body.innerText.slice(0, 400) };";

/// The capabilities of a headless Chrome that downloads into `downloads`. `BELT_CHROME_ARGS`
/// adds its words to the arguments, after the ones below: Chrome keeps the last value of a
/// switch given twice.
pub(crate) fn chrome_capabilities(downloads: &Path) -> Value {
    let mut options = json!({
        "args": [
            "--headless=new",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--window-size=1280,800",
            "--enable-unsafe-swiftshader",
            "--use-angle=swiftshader",
            "--disable-features=BackForwardCache",
            // A call test captures the fake camera and microphone, with no prompt.
            "--use-fake-device-for-media-stream",
            "--use-fake-ui-for-media-stream",
            // A call test plays a tone through an audio context, with no gesture.
            "--autoplay-policy=no-user-gesture-required",
        ],
        "prefs": {
            "download.default_directory": downloads.display().to_string(),
            "download.prompt_for_download": false,
        },
    });
    if let Ok(extra) = std::env::var("BELT_CHROME_ARGS")
        && let Some(args) = options["args"].as_array_mut()
    {
        args.extend(extra.split_whitespace().map(|word| json!(word)));
    }
    if let Ok(binary) = std::env::var("BELT_CHROME") {
        options["binary"] = json!(binary);
    }
    json!({
        "capabilities": { "alwaysMatch": {
            "browserName": "chrome",
            "goog:chromeOptions": options,
            "goog:loggingPrefs": { "browser": "ALL" },
        }}
    })
}

/// The capabilities of a headless Firefox that downloads into `downloads`.
///
/// Headless Firefox on Linux creates its WebGL2 context through the X display
/// in `DISPLAY`. Without one it has no GL driver and the canvas never boots,
/// so `just test-browser-firefox` runs under a virtual display.
fn firefox_capabilities(downloads: &Path) -> Value {
    let mut options = json!({
        "args": ["-headless"],
        "prefs": {
            // geckodriver serves no log endpoint: the console goes to the
            // standard output instead, which `collect_console` reads.
            "devtools.console.stdout.content": true,
            // A download lands in `downloads` with no dialog.
            "browser.download.folderList": 2,
            "browser.download.dir": downloads.display().to_string(),
            "browser.download.useDownloadDir": true,
            "browser.download.always_ask_before_handling_new_types": false,
            "browser.download.alwaysOpenPanel": false,
            "browser.download.manager.showWhenStarting": false,
            // Firefox has no `clipboard-read` permission to grant. This test
            // preference is its counterpart: a script reads the clipboard
            // without a paste prompt, as a granted Chrome does.
            "dom.events.testing.asyncClipboard": true,
            // The first run would otherwise open a tab of its own beside the
            // page under test.
            "browser.startup.homepage_override.mstone": "ignore",
            "datareporting.policy.dataSubmissionEnabled": false,
            // A call test captures the fake camera and microphone, with no prompt. The two
            // grants are a real permission, which `media.navigator.permission.disabled` is
            // not. A page without it gets one ICE candidate on the default route, behind an
            // mDNS name. That candidate cannot pair with the loopback address of the SFU.
            "media.navigator.streams.fake": true,
            "permissions.default.camera": 1,
            "permissions.default.microphone": 1,
            // Firefox gathers no loopback candidate by default.
            "media.peerconnection.ice.loopback": true,
            // A call test plays a tone through an audio context, with no gesture.
            "media.autoplay.default": 0,
            "media.autoplay.block-webaudio": false,
        },
    });
    if let Ok(binary) = std::env::var("BELT_FIREFOX") {
        options["binary"] = json!(binary);
    }
    json!({
        "capabilities": { "alwaysMatch": {
            "browserName": "firefox",
            "moz:firefoxOptions": options,
        }}
    })
}

/// Keeps each console line Firefox prints on `stream`, until the stream
/// closes with the driver.
fn collect_console(stream: impl Read + Send + 'static, printed: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if (line.starts_with("console.") || line.starts_with("JavaScript "))
                && let Ok(mut lines) = printed.lock()
            {
                lines.push(line);
            }
        }
    });
}

/// One running driver and the one session it holds.
pub struct Browser {
    http: reqwest::Client,
    session: String,
    process: Child,
    engine: Engine,
    /// The console lines Firefox printed since the last read.
    printed: Arc<Mutex<Vec<String>>>,
}

impl Browser {
    /// Starts the driver of [`Engine::current`], and opens a headless browser
    /// that downloads into `downloads`.
    pub async fn start(downloads: &Path) -> Self {
        Self::start_with(downloads, |_| ()).await
    }

    /// As [`Browser::start`], with the capabilities changed by `adjust` before the session
    /// opens. A proof on a desktop opens a Chrome with a window and its GPU this way.
    pub async fn start_with(downloads: &Path, adjust: impl FnOnce(&mut Value)) -> Self {
        let port = free_port();
        let engine = Engine::current();
        let printed = Arc::new(Mutex::new(Vec::new()));
        let mut process = match engine {
            Engine::Chrome => Command::new(chromedriver())
                .arg(format!("--port={port}"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start chromedriver: set CHROMEDRIVER, or put it on the path"),
            // Two pages in one test run two drivers at once. So each driver
            // takes a BiDi port of its own instead of the default 9222.
            Engine::Firefox => Command::new(geckodriver())
                .arg(format!("--port={port}"))
                .arg(format!("--websocket-port={}", free_port()))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start geckodriver: set GECKODRIVER, or put it on the path"),
        };
        if let Some(out) = process.stdout.take() {
            collect_console(out, Arc::clone(&printed));
        }
        if let Some(err) = process.stderr.take() {
            collect_console(err, Arc::clone(&printed));
        }
        // The driver and the page are both local: no proxy stands between.
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("an HTTP client");
        let root = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        while http.get(format!("{root}/status")).send().await.is_err() {
            assert!(Instant::now() < deadline, "the driver did not answer");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut caps = match engine {
            Engine::Chrome => chrome_capabilities(downloads),
            Engine::Firefox => firefox_capabilities(downloads),
        };
        adjust(&mut caps);
        let resp = post(&http, &format!("{root}/session"), &caps).await;
        let id = resp["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("no session: {resp}"));
        let browser = Self {
            http,
            session: format!("{root}/session/{id}"),
            process,
            engine,
            printed,
        };
        if engine == Engine::Firefox {
            // Chrome takes its window size as an argument. Headless Firefox
            // takes it from the window rect.
            browser
                .command("window/rect", &json!({ "width": 1280, "height": 800 }))
                .await;
        }
        browser
    }

    async fn command(&self, path: &str, body: &Value) -> Value {
        post(&self.http, &format!("{}/{path}", self.session), body).await
    }

    async fn get(&self, path: &str) -> Value {
        let url = format!("{}/{path}", self.session);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{url}: {e}"));
        reply_value(&url, &resp.text().await.expect("a WebDriver reply"))
    }

    /// The engine this browser runs.
    pub fn engine(&self) -> Engine {
        self.engine
    }

    /// Loads `url` and waits for the document to load.
    pub async fn goto(&self, url: &str) {
        self.command("url", &json!({ "url": url })).await;
    }

    /// Runs `body` as a function of `args` in the page, and answers its result.
    pub async fn script(&self, body: &str, args: Value) -> Value {
        self.command("execute/sync", &json!({ "script": body, "args": args }))
            .await
    }

    /// Runs `body` until it calls its last argument, and answers what it passed.
    pub async fn script_async(&self, body: &str, args: Value) -> Value {
        self.command("execute/async", &json!({ "script": body, "args": args }))
            .await
    }

    /// Polls `body` until it answers a value that is neither null nor false,
    /// and answers that value. Fails with `what` past `timeout`.
    pub async fn wait_for(&self, what: &str, body: &str, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let v = self.script(body, json!([])).await;
            if !v.is_null() && v != Value::Bool(false) {
                return v;
            }
            if Instant::now() >= deadline {
                let seen = self.script(SNAPSHOT, json!([])).await;
                let lines = self.console().await;
                panic!(
                    "timed out waiting for {what}; the page showed {seen}; its console said:\n{}",
                    lines.join("\n")
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Performs input `actions`, then releases every key and button they hold.
    pub async fn perform(&self, actions: Value) {
        self.command("actions", &json!({ "actions": actions }))
            .await;
    }

    /// The console lines the page logged since the last call.
    pub async fn console(&self) -> Vec<String> {
        match self.engine {
            Engine::Chrome => {
                let console = self.command("se/log", &json!({ "type": "browser" })).await;
                console
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l["message"].as_str().map(str::to_owned))
                    .collect()
            }
            Engine::Firefox => self
                .printed
                .lock()
                .map(|mut lines| std::mem::take(&mut *lines))
                .unwrap_or_default(),
        }
    }

    /// The viewport as the screen shows it, as PNG bytes (W3C Take
    /// Screenshot). Both engines serve it.
    pub async fn screenshot(&self) -> Vec<u8> {
        use base64::Engine as _;

        let shot = self.get("screenshot").await;
        let data = shot.as_str().expect("a screenshot");
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("a base64 screenshot")
    }

    /// Sends one DevTools Protocol command to the page's target, through the
    /// driver's Chrome extension, and answers its result. Chrome alone speaks
    /// it: a test that needs it is ignored under the feature `firefox`.
    pub async fn cdp(&self, method: &str, params: Value) -> Value {
        assert_eq!(
            self.engine,
            Engine::Chrome,
            "{method}: the DevTools Protocol is Chrome's; gate the test on the feature `firefox`"
        );
        self.command(
            "goog/cdp/execute",
            &json!({ "cmd": method, "params": params }),
        )
        .await
    }

    /// Sets a permission of the page's origin (WebDriver Permissions).
    ///
    /// Firefox knows no `clipboard-read` permission. Its session reads the
    /// clipboard through a preference instead (`firefox_capabilities`), so
    /// there is nothing to grant.
    pub async fn permit(&self, name: &str) {
        if self.engine == Engine::Firefox && name == "clipboard-read" {
            return;
        }
        self.command(
            "permissions",
            &json!({ "descriptor": { "name": name }, "state": "granted" }),
        )
        .await;
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Killing the driver orphans the browser and leaves its profile on
        // disk. So the session is closed first: the driver then quits the
        // browser and removes the profile. `Drop` cannot await, so the request
        // is written by hand.
        if let Some((addr, path)) = self
            .session
            .strip_prefix("http://")
            .and_then(|rest| rest.split_once('/'))
        {
            use std::io::Write;
            if let Ok(mut stream) = std::net::TcpStream::connect(addr) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let request =
                    format!("DELETE /{path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
                if stream.write_all(request.as_bytes()).is_ok() {
                    let _ = stream.read_to_end(&mut Vec::new());
                }
            }
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Posts `body` and answers the `value` of the reply. An error reply fails.
async fn post(http: &reqwest::Client, url: &str, body: &Value) -> Value {
    let resp = http
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap_or_else(|e| panic!("{url}: {e}"));
    let text = resp.text().await.expect("a WebDriver reply");
    reply_value(url, &text)
}

/// The `value` of the reply `text` to `url`. An error reply fails.
fn reply_value(url: &str, text: &str) -> Value {
    let reply: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("{url}: {text}"));
    let value = reply.get("value").cloned().unwrap_or(Value::Null);
    if let Some(error) = value.get("error") {
        panic!("{url}: {error}: {}", value["message"]);
    }
    value
}
