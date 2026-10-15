//! The built application, served as a deployment serves it.
//!
//! One static server over the Trunk output. It sets the two headers that make
//! the page cross-origin isolated, without which the Logic thread cannot share
//! the page's memory (`Trunk.toml`, `[serve.headers]`). A path that names no
//! file is answered with the page itself. So a deep link boots the application
//! as it does behind the production server.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::Request;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderValue, Response, StatusCode, Uri, header};

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
fn media_type(path: &Path) -> &'static str {
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
fn resolve(root: &Path, uri: &Uri) -> PathBuf {
    let rel = uri.path().trim_start_matches('/');
    let climbs = rel.split('/').any(|seg| seg == "..");
    let file = root.join(rel);
    if rel.is_empty() || climbs || !file.is_file() {
        root.join("index.html")
    } else {
        file
    }
}

async fn answer(root: PathBuf, uri: Uri) -> Response<Body> {
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
    let headers = resp.headers_mut();
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-embedder-policy",
        HeaderValue::from_static("credentialless"),
    );
    resp
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
async fn forward(relay: String, peer: String, req: Request<Body>) -> Response<Body> {
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
async fn join_socket(page: WebSocket, relay: String, peer: String) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Up;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("{}/api/ws", relay.replacen("http", "ws", 1));
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

/// Serves `root` on a free local port, for as long as the test runs, and
/// answers the origin. Every `/api` path goes to the relay at `relay`, from
/// an address of its own.
pub async fn spawn(root: PathBuf, relay: String) -> String {
    assert!(
        root.join("index.html").is_file(),
        "no build in {}: run `trunk build` first (just test-browser)",
        root.display()
    );
    let peer = page_address();
    let ws_relay = relay.clone();
    let ws_peer = peer.clone();
    let app = axum::Router::new()
        .route(
            "/api/ws",
            axum::routing::get(move |ws: WebSocketUpgrade| {
                let relay = ws_relay.clone();
                let peer = ws_peer.clone();
                async move { ws.on_upgrade(move |page| join_socket(page, relay, peer)) }
            }),
        )
        .fallback(move |req: Request<Body>| {
            let root = root.clone();
            let relay = relay.clone();
            let peer = peer.clone();
            async move {
                if req.uri().path().starts_with("/api/") {
                    forward(relay, peer, req).await
                } else {
                    answer(root, req.uri().clone()).await
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
}
