//! A WebDriver client, as small as the belt needs (W3C WebDriver, level 2).
//!
//! It starts chromedriver, opens one headless Chrome session, and speaks the
//! handful of commands a test uses: navigate, run a script, perform input
//! actions. Each call is one HTTP request with a JSON body.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The chromedriver binary: `CHROMEDRIVER`, or `chromedriver` on the path.
fn chromedriver() -> String {
    std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into())
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

/// One running chromedriver and the one session it holds.
pub struct Browser {
    http: reqwest::Client,
    session: String,
    process: Child,
}

impl Browser {
    /// Starts chromedriver and opens a headless Chrome that downloads into
    /// `downloads`.
    pub async fn start(downloads: &Path) -> Self {
        let port = free_port();
        let process = Command::new(chromedriver())
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start chromedriver: set CHROMEDRIVER, or put it on the path");
        // The driver and the page are both local: no proxy stands between.
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("an HTTP client");
        let root = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(20);
        while http.get(format!("{root}/status")).send().await.is_err() {
            assert!(Instant::now() < deadline, "chromedriver did not answer");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut options = json!({
            "args": [
                "--headless=new",
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--window-size=1280,800",
                "--enable-unsafe-swiftshader",
                "--use-angle=swiftshader",
            ],
            "prefs": {
                "download.default_directory": downloads.display().to_string(),
                "download.prompt_for_download": false,
            },
        });
        if let Ok(binary) = std::env::var("BELT_CHROME") {
            options["binary"] = json!(binary);
        }
        let caps = json!({
            "capabilities": { "alwaysMatch": {
                "browserName": "chrome",
                "goog:chromeOptions": options,
                "goog:loggingPrefs": { "browser": "ALL" },
            }}
        });
        let resp = post(&http, &format!("{root}/session"), &caps).await;
        let id = resp["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("no session: {resp}"));
        Self {
            http,
            session: format!("{root}/session/{id}"),
            process,
        }
    }

    async fn command(&self, path: &str, body: &Value) -> Value {
        post(&self.http, &format!("{}/{path}", self.session), body).await
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
        let console = self.command("se/log", &json!({ "type": "browser" })).await;
        console
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["message"].as_str().map(str::to_owned))
            .collect()
    }

    /// Sends one DevTools Protocol command to the page's target, through the
    /// driver's Chrome extension, and answers its result.
    pub async fn cdp(&self, method: &str, params: Value) -> Value {
        self.command(
            "goog/cdp/execute",
            &json!({ "cmd": method, "params": params }),
        )
        .await
    }

    /// Sets a permission of the page's origin (WebDriver Permissions).
    pub async fn permit(&self, name: &str) {
        self.command(
            "permissions",
            &json!({ "descriptor": { "name": name }, "state": "granted" }),
        )
        .await;
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Killing the driver orphans Chrome and leaves its profile on disk. So
        // the session is closed first: the driver then quits Chrome and removes
        // the profile. `Drop` cannot await, so the request is written by hand.
        if let Some((addr, path)) = self
            .session
            .strip_prefix("http://")
            .and_then(|rest| rest.split_once('/'))
        {
            use std::io::{Read, Write};
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
    let reply: Value = serde_json::from_str(&text).unwrap_or_else(|_| panic!("{url}: {text}"));
    let value = reply.get("value").cloned().unwrap_or(Value::Null);
    if let Some(error) = value.get("error") {
        panic!("{url}: {error}: {}", value["message"]);
    }
    value
}
