//! Two browsers in one call, through the SFU binary (`ambre-spec-call-media.md`).
//!
//! The belt starts the `ambre-sfu` binary on free ports, and serves one test page. The page
//! opens the fake camera and microphone of the engine, sends both tracks to the SFU, and plays
//! back what the SFU forwards. The test reads the decoder counters through `getStats()`, as
//! the engine reports them, and reaches no internal state of the SFU.
//!
//! The page is a test fixture, not the call client: the client seals each chunk in a worker,
//! and this page seals nothing.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use axum::Router;
use axum::response::Html;
use axum::routing::get;

/// The test page. It joins the room the query names on the SFU the query names, then keeps
/// its counters in `window.callStats()`.
const PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>call belt</title>
<body>
<script>
const query = new URLSearchParams(location.search);
const pc = new RTCPeerConnection();
window.callState = 'starting';
window.callError = null;
pc.ontrack = (e) => {
  const el = document.createElement(e.track.kind === 'video' ? 'video' : 'audio');
  el.autoplay = true;
  el.muted = true;
  el.srcObject = new MediaStream([e.track]);
  document.body.appendChild(el);
};
window.callStats = async () => {
  const out = { samples: null, packets: 0, video: 0, state: pc.connectionState };
  (await pc.getStats()).forEach((s) => {
    if (s.type !== 'inbound-rtp') return;
    if (s.kind === 'video') out.video += s.framesDecoded || 0;
    if (s.kind === 'audio') {
      out.packets += s.packetsReceived || 0;
      // Samples the decoder produced, without the ones it concealed.
      if (s.totalSamplesReceived !== undefined) {
        out.samples = (out.samples || 0) + s.totalSamplesReceived - (s.concealedSamples || 0);
      }
    }
  });
  return out;
};
(async () => {
  try {
    const media = await navigator.mediaDevices.getUserMedia({ audio: true, video: true });
    media.getTracks().forEach((t) => pc.addTrack(t, media));
    const ws = new WebSocket(query.get('sfu'));
    // One description at a time: an offer of the SFU waits for the answer before it.
    let chain = Promise.resolve();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      chain = chain.then(async () => {
        if (msg.answer !== undefined) {
          await pc.setRemoteDescription({ type: 'answer', sdp: msg.answer });
        } else if (msg.offer !== undefined) {
          await pc.setRemoteDescription({ type: 'offer', sdp: msg.offer });
          await pc.setLocalDescription(await pc.createAnswer());
          ws.send(JSON.stringify({ answer: pc.localDescription.sdp }));
        }
      }).catch((e) => { window.callError = String(e); });
    };
    await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
    await pc.setLocalDescription(await pc.createOffer());
    ws.send(JSON.stringify({ offer: pc.localDescription.sdp }));
    window.callState = 'offered';
  } catch (e) {
    window.callError = String(e);
  }
})();
</script>
"#;

/// A free local port, of TCP or of UDP.
fn free_port(udp: Udp) -> u16 {
    match udp {
        Udp::No => std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()),
        Udp::Yes => std::net::UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr()),
    }
    .map(|a| a.port())
    .expect("a free local port")
}

/// Which transport a free port is for.
#[derive(Clone, Copy)]
enum Udp {
    No,
    Yes,
}

/// The SFU binary: `BELT_SFU`, or the debug build of the workspace.
fn binary() -> String {
    std::env::var("BELT_SFU").unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug/ambre-sfu")
            .display()
            .to_string()
    })
}

/// One running SFU process, killed when the test ends.
pub struct Sfu {
    /// The WebSocket root of its signalling, on a port of its own.
    pub signal: String,
    process: Child,
}

impl Sfu {
    /// Starts the SFU, and waits until its signalling port accepts a connection.
    pub async fn start() -> Self {
        let signal_port = free_port(Udp::No);
        let process = Command::new(binary())
            .env("SFU_SIGNAL_ADDR", format!("127.0.0.1:{signal_port}"))
            .env(
                "SFU_MEDIA_ADDR",
                format!("127.0.0.1:{}", free_port(Udp::Yes)),
            )
            .stdout(Stdio::null())
            .stderr(if std::env::var_os("BELT_VERBOSE").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .spawn()
            .expect("start the SFU: run `cargo build -p ambre-sfu`, or set BELT_SFU");
        let deadline = Instant::now() + Duration::from_secs(20);
        while tokio::net::TcpStream::connect(("127.0.0.1", signal_port))
            .await
            .is_err()
        {
            assert!(Instant::now() < deadline, "the SFU did not listen");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Self {
            signal: format!("ws://127.0.0.1:{signal_port}"),
            process,
        }
    }
}

impl Drop for Sfu {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Serves the test page on a free port, and answers its root URL.
pub async fn serve_page() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a page port");
    let addr = listener.local_addr().expect("a page address");
    let app = Router::new().route("/", get(|| async { Html(PAGE) }));
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}/")
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::driver::Browser;

    /// What one page decoded so far: audio samples and video pictures.
    async fn decoded(browser: &Browser) -> Value {
        browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 if (window.callError) { done({ error: window.callError }); return; } \
                 window.callStats().then(done, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// `CALL-037`, `CALL-036`: two browsers join one room on the signalling port of the SFU.
    /// No relay runs, and each browser decodes the audio and the video of the other.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs the SFU binary and a browser driver; run just test-browser"
    )]
    async fn two_browsers_decode_each_other_through_the_sfu() {
        let sfu = Sfu::start().await;
        let page = serve_page().await;
        let room = format!("{}/room/belt-{}", sfu.signal, uuid::Uuid::new_v4().simple());
        let url = format!("{page}?sfu={room}");
        let mut browsers = Vec::new();
        for _ in 0..2 {
            let downloads = std::env::temp_dir().join(format!("belt-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&downloads).expect("a download directory");
            let browser = Browser::start(&downloads).await;
            browser.goto(&url).await;
            browsers.push((browser, downloads));
        }
        // A frame decoded every 33 ms for 1 s, and 1 s of audio at 48 kHz, on each side.
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        loop {
            seen.clear();
            for (browser, _) in &browsers {
                seen.push(decoded(browser).await);
            }
            assert!(
                seen.iter().all(|s| s.get("error").is_none()),
                "a page failed: {seen:?}"
            );
            let done = seen.iter().all(|s| {
                // An engine that reports no sample counter falls back to the packets it
                // received: 50 packets of 20 ms.
                let audio = s["samples"].as_f64().map_or_else(
                    || s["packets"].as_f64().unwrap_or(0.0) >= 50.0,
                    |samples| samples >= 48_000.0,
                );
                audio && s["video"].as_f64().unwrap_or(0.0) >= 30.0
            });
            if done {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "within 30 s, the two pages decoded {seen:?}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        for (browser, downloads) in browsers {
            drop(browser);
            let _ = std::fs::remove_dir_all(downloads);
        }
    }
}
