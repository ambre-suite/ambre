//! The built application, served as a deployment serves it.
//!
//! One static server over the Trunk output. It sets the two headers that make
//! the page cross-origin isolated, without which the Logic thread cannot share
//! the page's memory (`Trunk.toml`, `[serve.headers]`). A path that names no
//! file is answered with the page itself. So a deep link boots the application
//! as it does behind the production server. It also applies the headers file
//! the build writes beside the page, as a deployment's server does. So every
//! answer refuses a frame (`SRV-281`), and the script of each worker carries
//! its policy (`SRV-282`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderName, HeaderValue, Response, StatusCode, Uri, header};

/// The headers file the post-build hook writes beside the page
/// (`scripts/trunk_post_build.py`), in the format Netlify and Cloudflare Pages
/// read.
pub const HEADERS_FILE: &str = "_headers";

/// The paths a rule of the headers file covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Paths {
    /// This one path.
    Exact(String),
    /// Every path that starts with this prefix: the rule ended on `*`.
    Under(String),
}

impl Paths {
    fn covers(&self, path: &str) -> bool {
        match self {
            Self::Exact(exact) => path == exact,
            Self::Under(prefix) => path.starts_with(prefix.as_str()),
        }
    }
}

/// One rule of the headers file: the paths it covers, and the headers each of
/// their answers carries.
#[derive(Debug, Clone)]
pub(crate) struct Rule {
    paths: Paths,
    headers: Vec<(HeaderName, HeaderValue)>,
}

/// The rules of a headers file, in file order. A path line starts at the
/// first column; each header line under it is indented and reads
/// `Name: value`. A line that parses as neither fails the test run, so the
/// belt never serves a build under a header it dropped.
pub(crate) fn parse_rules(text: &str) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with(char::is_whitespace) {
            let (name, value) = line
                .trim()
                .split_once(':')
                .unwrap_or_else(|| panic!("a header line with no colon: {line}"));
            let rule = rules
                .last_mut()
                .unwrap_or_else(|| panic!("a header line before any path: {line}"));
            rule.headers.push((
                HeaderName::try_from(name.trim()).expect("a header name"),
                HeaderValue::try_from(value.trim()).expect("a header value"),
            ));
        } else {
            let path = line.trim();
            let paths = path.strip_suffix('*').map_or_else(
                || Paths::Exact(path.to_owned()),
                |prefix| Paths::Under(prefix.to_owned()),
            );
            rules.push(Rule {
                paths,
                headers: Vec::new(),
            });
        }
    }
    rules
}

/// The rules of the headers file of the build in `root`. A build with no such
/// file predates the hook that writes it, and fails the run.
pub(crate) fn read_rules(root: &Path) -> Vec<Rule> {
    let file = root.join(HEADERS_FILE);
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        panic!(
            "no {} in the build: {e}; run `trunk build` again (just test-browser)",
            file.display()
        )
    });
    parse_rules(&text)
}

/// Adds to `resp` the headers of each rule that covers `path`. Two rules may
/// each give a content policy. The answer carries both, and the browser
/// enforces each, as it does behind a server that joins them.
pub(crate) fn apply_rules(rules: &[Rule], path: &str, resp: &mut Response<Body>) {
    let headers = resp.headers_mut();
    for rule in rules.iter().filter(|r| r.paths.covers(path)) {
        for (name, value) in &rule.headers {
            headers.append(name.clone(), value.clone());
        }
    }
}

/// The directory Trunk builds into: `BELT_DIST`, or `dist/` at the workspace
/// root.
pub fn dist_dir() -> PathBuf {
    std::env::var_os("BELT_DIST").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist"),
        PathBuf::from,
    )
}

/// The media type of a built file, by its extension. A module script and the
/// wasm binary must carry theirs, or the browser refuses them.
pub(crate) fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// The file `uri` names under `root`, or the page when it names none. A path
/// that climbs out of `root` names none.
pub(crate) fn resolve(root: &Path, uri: &Uri) -> PathBuf {
    let rel = uri.path().trim_start_matches('/');
    let climbs = rel.split('/').any(|seg| seg == "..");
    let file = root.join(rel);
    if rel.is_empty() || climbs || !file.is_file() {
        root.join("index.html")
    } else {
        file
    }
}

async fn answer(root: PathBuf, rules: Arc<Vec<Rule>>, uri: Uri) -> Response<Body> {
    let path = resolve(&root, &uri);
    let mut resp = match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(media_type(&path)),
            );
            resp
        }
        Err(_) => {
            let mut resp = Response::new(Body::empty());
            *resp.status_mut() = StatusCode::NOT_FOUND;
            resp
        }
    };
    apply_rules(&rules, uri.path(), &mut resp);
    isolate(&mut resp);
    resp
}

/// Sets the two headers that make a page cross-origin isolated, and a worker script with it.
pub(crate) fn isolate(resp: &mut Response<Body>) {
    let headers = resp.headers_mut();
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-embedder-policy",
        HeaderValue::from_static("credentialless"),
    );
}

/// The header a reverse proxy overwrites with the address of the client.
pub const FORWARDED_FOR: &str = "x-forwarded-for";

/// An address that stands for one page, as a reverse proxy reports it. Every
/// page connects from the loopback, so the relay would put all of them in one
/// rate-limit bucket. A deployment's proxy gives each client its own address.
pub fn page_address() -> String {
    let [a, b, c, ..] = *uuid::Uuid::new_v4().as_bytes();
    format!("10.{a}.{b}.{}", c.max(1))
}

/// Forwards one `/api` request to the relay at `relay`, and its reply back.
/// It overwrites the forwarded address with `peer`, as a deployment's proxy
/// does.
pub(crate) async fn forward(relay: String, peer: String, req: Request<Body>) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    let Ok(body) = axum::body::to_bytes(body, 64 << 20).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let mut headers = parts.headers;
    headers.remove(header::HOST);
    if let Ok(value) = HeaderValue::from_str(&peer) {
        headers.insert(FORWARDED_FOR, value);
    }
    let sent = relay_client()
        .request(parts.method, format!("{relay}{path}"))
        .headers(headers)
        .body(body)
        .send()
        .await;
    let Ok(reply) = sent else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let code = reply.status();
    let headers = reply.headers().clone();
    let bytes = reply.bytes().await.unwrap_or_default();
    let mut resp = Response::new(Body::from(bytes));
    *resp.status_mut() = code;
    *resp.headers_mut() = headers;
    resp.headers_mut().remove(header::TRANSFER_ENCODING);
    resp
}

/// The client the forwarding uses: local, so no proxy stands between.
fn relay_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("an HTTP client")
        })
        .clone()
}

fn status(code: StatusCode) -> Response<Body> {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = code;
    resp
}

/// Joins the page's socket to one opened on the relay, frame for frame. The
/// socket carries no credential in its request (`SYNC-061`), so the frames are
/// all there is to carry.
pub(crate) async fn join_socket(page: WebSocket, relay: String, peer: String, path: String) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Up;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("{}{path}", relay.replacen("http", "ws", 1));
    let Ok(mut request) = url.into_client_request() else {
        return;
    };
    if let Ok(value) = peer.parse() {
        request.headers_mut().insert(FORWARDED_FOR, value);
    }
    let Ok((upstream, _)) = tokio_tungstenite::connect_async(request).await else {
        return;
    };
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut page_tx, mut page_rx) = page.split();
    let to_relay = async {
        while let Some(Ok(msg)) = page_rx.next().await {
            let msg = match msg {
                Message::Text(t) => Up::text(t.as_str()),
                Message::Binary(b) => Up::Binary(b),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
            };
            if up_tx.send(msg).await.is_err() {
                break;
            }
        }
        let _ = up_tx.close().await;
    };
    let to_page = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let msg = match msg {
                Up::Text(t) => Message::Text(t.as_str().into()),
                Up::Binary(b) => Message::Binary(b),
                Up::Close(_) => break,
                _ => continue,
            };
            if page_tx.send(msg).await.is_err() {
                break;
            }
        }
        let _ = page_tx.close().await;
    };
    tokio::join!(to_relay, to_page);
}

/// Every `/api` path a page of this process asked, in order. A test reads from them an
/// identifier the page shows no element for, such as the channel it opened.
static ASKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn record(path: &str) {
    if let Ok(mut asked) = ASKED.lock() {
        asked.push(path.to_owned());
    }
}

/// The `/api` paths the pages of this process asked so far.
pub fn asked() -> Vec<String> {
    ASKED.lock().map(|a| a.clone()).unwrap_or_default()
}

/// Serves `root` on a free local port, for as long as the test runs, and
/// answers the origin. Every `/api` path goes to the relay at `relay`, from
/// an address of its own.
pub async fn spawn(root: PathBuf, relay: String) -> String {
    assert!(
        root.join("index.html").is_file(),
        "no build in {}: run `trunk build` first (just test-browser)",
        root.display()
    );
    let rules = Arc::new(read_rules(&root));
    let peer = page_address();
    let ws_relay = relay.clone();
    let ws_peer = peer.clone();
    let (call_relay, call_peer) = (relay.clone(), peer.clone());
    let app = axum::Router::new()
        .route(
            "/api/ws",
            axum::routing::get(move |ws: WebSocketUpgrade| {
                let relay = ws_relay.clone();
                let peer = ws_peer.clone();
                async move {
                    ws.on_upgrade(move |page| join_socket(page, relay, peer, "/api/ws".into()))
                }
            }),
        )
        // The call socket of a call, a socket of its own on the relay (`SRV-049`).
        .route(
            "/api/calls/{call}/socket",
            axum::routing::get(move |ws: WebSocketUpgrade, uri: Uri| {
                let relay = call_relay.clone();
                let peer = call_peer.clone();
                let path = uri.path().to_owned();
                // A test reads that a page opened no call socket (`CALL-076`).
                record(&path);
                async move { ws.on_upgrade(move |page| join_socket(page, relay, peer, path)) }
            }),
        )
        .fallback(move |req: Request<Body>| {
            let root = root.clone();
            let rules = Arc::clone(&rules);
            let relay = relay.clone();
            let peer = peer.clone();
            async move {
                if req.uri().path().starts_with("/api/") {
                    record(req.uri().path());
                    forward(relay, peer, req).await
                } else {
                    answer(root, rules, req.uri().clone()).await
                }
            }
        });
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind the belt's server");
    let addr = listener.local_addr().expect("the server's address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deep link and a climb both get the page; a built file gets itself.
    #[test]
    fn a_path_naming_no_file_gets_the_page() {
        let root = std::env::temp_dir().join(format!("belt-serve-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<html>").unwrap();
        std::fs::write(root.join("app.wasm"), [0u8]).unwrap();
        let page = root.join("index.html");
        for uri in ["/", "/toile/0193", "/../Cargo.toml", "/missing.js"] {
            assert_eq!(resolve(&root, &uri.parse().unwrap()), page, "{uri}");
        }
        let wasm = resolve(&root, &"/app.wasm".parse().unwrap());
        assert_eq!(wasm, root.join("app.wasm"));
        assert_eq!(media_type(&wasm), "application/wasm");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The headers file as the post-build hook writes it for a build served
    /// at the root (`scripts/trunk_post_build.py`, `write_headers`).
    const HEADERS: &str = "/*\n  Cross-Origin-Opener-Policy: same-origin\n  \
        Cross-Origin-Embedder-Policy: credentialless\n  \
        Content-Security-Policy: frame-ancestors 'none'\n\
        /ambre-sync-0123abcd_loader.js\n  Content-Security-Policy: script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none'\n\
        /ambre-call-4567ef01_loader.js\n  Content-Security-Policy: script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none'\n";

    /// The content policies an answer of the belt's server carries for `uri`.
    async fn policies(root: &Path, uri: &str) -> Vec<String> {
        let rules = Arc::new(read_rules(root));
        let resp = answer(root.to_path_buf(), rules, uri.parse().unwrap()).await;
        resp.headers()
            .get_all(header::CONTENT_SECURITY_POLICY)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }

    /// `SRV-281`: the page server answers the page, a deep link and every
    /// file with a policy that refuses any frame. It answers the script of
    /// each worker with the worker policy the build names for it, and no
    /// other file with that policy.
    #[tokio::test]
    async fn every_answer_refuses_a_frame_and_a_worker_script_carries_its_policy() {
        let root = std::env::temp_dir().join(format!("belt-headers-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<html>").unwrap();
        std::fs::write(root.join(HEADERS_FILE), HEADERS).unwrap();
        for name in [
            "ambre-sync-0123abcd_loader.js",
            "ambre-call-4567ef01_loader.js",
            "ambre-app-0123abcd.js",
        ] {
            std::fs::write(root.join(name), "script").unwrap();
        }
        let frame = "frame-ancestors 'none'";
        let worker = "script-src 'self' 'wasm-unsafe-eval'; frame-ancestors 'none'";
        for uri in ["/", "/toile/0193", "/ambre-app-0123abcd.js", "/missing.js"] {
            assert_eq!(policies(&root, uri).await, [frame], "{uri}");
        }
        for uri in [
            "/ambre-sync-0123abcd_loader.js",
            "/ambre-call-4567ef01_loader.js",
        ] {
            assert_eq!(policies(&root, uri).await, [frame, worker], "{uri}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A rule ending on `*` covers the paths under its prefix alone; any
    /// other rule covers its own path alone.
    #[test]
    fn a_rule_covers_its_path_or_its_prefix() {
        let rules = parse_rules("/app/*\n  X-A: 1\n/app/w.js\n  X-B: 2\n");
        assert_eq!(rules.len(), 2);
        assert!(rules[0].paths.covers("/app/x/y"));
        assert!(!rules[0].paths.covers("/other"));
        assert!(rules[1].paths.covers("/app/w.js"));
        assert!(!rules[1].paths.covers("/app/w.js.br"));
    }
}
