//! Real browsers in the load run, beside the synthetic participants.
//!
//! A synthetic participant sends no VP8 a browser can decode. So each browser here sends its
//! fake camera in three simulcast layers, and the browsers decode each other. A browser reaches
//! the SFU through an impairment link, as a synthetic participant does. The page rewrites the
//! candidate of the SFU to the front of its link.
//!
//! The figures come from `getStats()`, as the engine counts them. Each received video reports
//! its freezes, their total duration, its decoded pictures and its dropped pictures.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The page each browser opens. The query names the SFU, the room and the front of the link.
pub const PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>load browser</title>
<body>
<script>
const query = new URLSearchParams(location.search);
const pc = new RTCPeerConnection();
window.callError = null;
pc.ontrack = (e) => {
  const el = document.createElement(e.track.kind === 'video' ? 'video' : 'audio');
  el.autoplay = true;
  el.muted = true;
  el.srcObject = new MediaStream([e.track]);
  document.body.appendChild(el);
};
// The counters of each received video that decoded a picture.
window.videoStats = async () => {
  const out = [];
  const stats = await pc.getStats();
  stats.forEach((s) => {
    if (s.type !== 'inbound-rtp' || s.kind !== 'video' || !s.framesDecoded) return;
    out.push({ id: s.id, decoded: s.framesDecoded, dropped: s.framesDropped || 0,
      freezes: s.freezeCount === undefined ? null : s.freezeCount,
      frozen: s.totalFreezesDuration === undefined ? null : s.totalFreezesDuration,
      lost: s.packetsLost || 0, received: s.packetsReceived || 0 });
  });
  return out;
};
// The candidate of the SFU, moved to the front of the impairment link.
const relay = (sdp) => sdp.replace(/(a=candidate:\S+ \d+ udp \d+ )(\S+) (\d+)/gi,
  (m, head) => head + query.get('relay').replace(':', ' '));
(async () => {
  try {
    const media = await navigator.mediaDevices.getUserMedia(
      { audio: true, video: { width: 640, height: 480, frameRate: 30 } });
    pc.addTrack(media.getAudioTracks()[0], media);
    pc.addTransceiver(media.getVideoTracks()[0], { direction: 'sendonly', streams: [media],
      sendEncodings: [
        { rid: 'l', scaleResolutionDownBy: 4, maxBitrate: 150000 },
        { rid: 'm', scaleResolutionDownBy: 2, maxBitrate: 500000 },
        { rid: 'h', maxBitrate: 1500000 },
      ] });
    const ws = new WebSocket(query.get('sfu'));
    let chain = Promise.resolve();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      chain = chain.then(async () => {
        if (msg.answer !== undefined) {
          await pc.setRemoteDescription({ type: 'answer', sdp: relay(msg.answer) });
        } else if (msg.offer !== undefined) {
          await pc.setRemoteDescription({ type: 'offer', sdp: relay(msg.offer) });
          await pc.setLocalDescription(await pc.createAnswer());
          ws.send(JSON.stringify({ answer: pc.localDescription.sdp }));
        }
      }).catch((e) => { window.callError = String(e); });
    };
    await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
    ws.send(JSON.stringify({ admission: query.get('admission') }));
    await pc.setLocalDescription(await pc.createOffer());
    ws.send(JSON.stringify({ offer: pc.localDescription.sdp }));
  } catch (e) {
    window.callError = String(e);
  }
})();
</script>
"#;

/// The browser engines a load run can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Chrome,
    Firefox,
}

impl Engine {
    /// The engines `LOAD_BROWSERS` names, in order: `chrome` and `firefox`, comma separated.
    pub fn listed() -> Vec<Self> {
        std::env::var("LOAD_BROWSERS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|name| match name.trim() {
                "chrome" => Some(Self::Chrome),
                "firefox" => Some(Self::Firefox),
                _ => None,
            })
            .collect()
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

/// One headless browser and its WebDriver session.
pub struct Browser {
    pub engine: Engine,
    driver: std::net::SocketAddr,
    session: String,
    process: Child,
}

impl Browser {
    /// Starts the driver of `engine` and a headless browser: `CHROMEDRIVER` and `BELT_CHROME`,
    /// or `GECKODRIVER` and `BELT_FIREFOX`.
    pub async fn start(engine: Engine) -> Self {
        let port = free_port();
        let process = match engine {
            Engine::Chrome => Command::new(
                std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into()),
            )
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
            Engine::Firefox => {
                Command::new(std::env::var("GECKODRIVER").unwrap_or_else(|_| "geckodriver".into()))
                    .arg(format!("--port={port}"))
                    .arg(format!("--websocket-port={}", free_port()))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            }
        }
        .expect("start the browser driver");
        let driver: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        let deadline = Instant::now() + Duration::from_secs(20);
        while std::net::TcpStream::connect(driver).is_err() {
            assert!(Instant::now() < deadline, "the driver did not answer");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let caps = match engine {
            Engine::Chrome => {
                let mut options = json!({ "args": [
                    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
                    "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
                    "--autoplay-policy=no-user-gesture-required",
                ]});
                if let Ok(binary) = std::env::var("BELT_CHROME") {
                    options["binary"] = json!(binary);
                }
                json!({ "browserName": "chrome", "goog:chromeOptions": options })
            }
            Engine::Firefox => {
                let mut options = json!({
                    "args": ["-headless"],
                    "prefs": {
                        "media.navigator.streams.fake": true,
                        "permissions.default.camera": 1,
                        "permissions.default.microphone": 1,
                        "media.peerconnection.ice.loopback": true,
                        "media.autoplay.default": 0,
                    },
                });
                if let Ok(binary) = std::env::var("BELT_FIREFOX") {
                    options["binary"] = json!(binary);
                }
                json!({ "browserName": "firefox", "moz:firefoxOptions": options })
            }
        };
        let body = json!({ "capabilities": { "alwaysMatch": caps } });
        let resp = post(driver, "/session", &body);
        let id = resp["value"]["sessionId"]
            .as_str()
            .or_else(|| resp["sessionId"].as_str())
            .unwrap_or_else(|| panic!("no session: {resp}"))
            .to_owned();
        Self {
            engine,
            driver,
            session: format!("/session/{id}"),
            process,
        }
    }

    /// Opens `url`.
    pub async fn goto(&self, url: &str) {
        post(
            self.driver,
            &format!("{}/url", self.session),
            &json!({ "url": url }),
        );
    }

    /// The counters of each received video, or the error of the page.
    pub async fn video_stats(&self) -> Value {
        let script = "const done = arguments[arguments.length - 1]; \
            if (window.callError) { done({ error: window.callError }); return; } \
            window.videoStats().then(done, (e) => done({ error: String(e) }));";
        let resp = post(
            self.driver,
            &format!("{}/execute/async", self.session),
            &json!({ "script": script, "args": [] }),
        );
        resp["value"].clone()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// One WebDriver command: a POST of `body` to `path` on the local driver, over HTTP/1.1.
/// The driver answers with a body of a known length, or in chunks.
fn post(driver: std::net::SocketAddr, path: &str, body: &Value) -> Value {
    use std::io::{Read, Write};
    let body = body.to_string();
    let mut stream = std::net::TcpStream::connect(driver).expect("the driver");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {driver}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("a request");
    // A driver may keep the connection open: the reply ends at its length, or at its last
    // chunk, never at the close.
    let mut reply: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let ended = |reply: &[u8]| {
        let text = String::from_utf8_lossy(reply);
        let Some((head, rest)) = text.split_once("\r\n\r\n") else {
            return false;
        };
        let head = head.to_ascii_lowercase();
        if head.contains("transfer-encoding: chunked") {
            return rest.ends_with("0\r\n\r\n");
        }
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok());
        length.is_none_or(|n| rest.len() >= n)
    };
    while !ended(&reply) {
        let n = stream.read(&mut buf).expect("a reply");
        if n == 0 {
            break;
        }
        reply.extend_from_slice(buf.get(..n).unwrap_or_default());
    }
    let text = String::from_utf8_lossy(&reply);
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let content = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut out = String::new();
        let mut rest = rest;
        while let Some((size, tail)) = rest.split_once("\r\n") {
            let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            out.push_str(tail.get(..size).unwrap_or(tail));
            rest = tail.get(size + 2..).unwrap_or("");
        }
        out
    } else {
        rest.to_owned()
    };
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}: {content}"))
}

/// What one received video counted between two readings.
#[derive(Debug)]
pub struct Watched {
    pub engine: Engine,
    pub decoded: f64,
    pub dropped: f64,
    pub freezes: Option<f64>,
    pub frozen: Option<Duration>,
}

/// The difference of each video between the reading `before` and the reading `after` of one
/// browser. A video absent from `before` counts from zero.
pub fn watched(engine: Engine, before: &Value, after: &Value) -> Vec<Watched> {
    let rows = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let field = |row: &Value, name: &str| row[name].as_f64();
    rows(after)
        .iter()
        .map(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            let old = rows(before)
                .into_iter()
                .find(|r| r["id"].as_str() == Some(id))
                .unwrap_or(Value::Null);
            let delta =
                |name: &str| field(row, name).map(|now| now - field(&old, name).unwrap_or(0.0));
            Watched {
                engine,
                decoded: delta("decoded").unwrap_or(0.0),
                dropped: delta("dropped").unwrap_or(0.0),
                freezes: delta("freezes"),
                frozen: delta("frozen").map(|s| Duration::from_secs_f64(s.max(0.0))),
            }
        })
        .collect()
}
