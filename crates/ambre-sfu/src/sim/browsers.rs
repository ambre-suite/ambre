//! Real browsers in the load run, beside the synthetic participants.
//!
//! A synthetic participant sends no VP8 a browser can decode. So each browser here sends its
//! fake camera in three simulcast layers, and the browsers decode each other. A browser reaches
//! the SFU through an impairment link, as a synthetic participant does. The page rewrites the
//! candidate of the SFU to the front of its link.
//!
//! The figures come from `getStats()`, as the engine counts them. Each received video reports
//! its freezes, their total duration, its decoded pictures and its dropped pictures.
//!
//! The browsers can also run on a second machine, each behind a WebDriver server there
//! (`LOAD_DRIVERS`). They then reach the SFU over the network, which `tc netem` impairs on that
//! machine (`scripts/call_stall_receivers.sh`), and no relay of this process stands between.

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
// What the network between the browser and the SFU does: the round trip of the selected pair,
// in seconds. Then the browser, so a run names the machine of each receiver.
window.pathStats = async () => {
  const stats = await pc.getStats();
  let rtt = null;
  stats.forEach((s) => {
    const pair = (s.type === 'transport' && stats.get(s.selectedCandidatePairId))
      || (s.type === 'candidate-pair' && s.nominated && s);
    if (pair && pair.currentRoundTripTime !== undefined) rtt = pair.currentRoundTripTime;
  });
  return { rtt, state: pc.connectionState, agent: navigator.userAgent,
    cores: navigator.hardwareConcurrency || null };
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

/// One WebDriver server on another machine: the engine it drives, and its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remote {
    pub engine: Engine,
    pub driver: std::net::SocketAddr,
}

impl Remote {
    /// The servers `LOAD_DRIVERS` names: `chrome@192.168.1.21:9515,firefox@192.168.1.21:4444`.
    ///
    /// # Errors
    ///
    /// The first entry that names no engine or no address.
    pub fn listed(text: &str) -> Result<Vec<Self>, String> {
        text.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (engine, driver) = entry
                    .split_once('@')
                    .ok_or_else(|| format!("{entry}: engine@address:port"))?;
                let engine = match engine {
                    "chrome" => Engine::Chrome,
                    "firefox" => Engine::Firefox,
                    other => return Err(format!("{entry}: chrome or firefox, not {other}")),
                };
                let driver = driver
                    .parse()
                    .map_err(|_| format!("{entry}: not an address and a port: {driver}"))?;
                Ok(Self { engine, driver })
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
    /// The local driver this run started, none for a driver on another machine.
    process: Option<Child>,
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
        let caps = capabilities(engine, None);
        Self::open(engine, driver, &caps, Some(process))
    }

    /// Opens a headless browser through the WebDriver server `remote` on another machine. The
    /// page comes from `origin` over plain HTTP. The browser treats that origin as secure, so the
    /// page captures the fake camera.
    pub fn remote(remote: Remote, origin: &str) -> Self {
        let caps = capabilities(remote.engine, Some(origin));
        Self::open(remote.engine, remote.driver, &caps, None)
    }

    fn open(
        engine: Engine,
        driver: std::net::SocketAddr,
        caps: &Value,
        process: Option<Child>,
    ) -> Self {
        let body = json!({ "capabilities": { "alwaysMatch": caps } });
        let resp = post(driver, "/session", &body);
        let id = resp["value"]["sessionId"]
            .as_str()
            .or_else(|| resp["sessionId"].as_str())
            .unwrap_or_else(|| panic!("no session at {driver}: {resp}"))
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
        self.call("videoStats").await
    }

    /// The round trip of the path to the SFU and the browser, or the error of the page.
    pub async fn path_stats(&self) -> Value {
        self.call("pathStats").await
    }

    /// The answer of the page function `name`, or the error of the page.
    async fn call(&self, name: &str) -> Value {
        let script = format!(
            "const done = arguments[arguments.length - 1]; \
             if (window.callError) {{ done({{ error: window.callError }}); return; }} \
             window.{name}().then(done, (e) => done({{ error: String(e) }}));"
        );
        let resp = post(
            self.driver,
            &format!("{}/execute/async", self.session),
            &json!({ "script": script, "args": [] }),
        );
        resp["value"].clone()
    }
}

/// The capabilities of a headless `engine` on fake capture devices. With `insecure`, the
/// browser treats that origin as secure, so its page captures over plain HTTP.
fn capabilities(engine: Engine, insecure: Option<&str>) -> Value {
    match engine {
        Engine::Chrome => {
            let mut options = json!({ "args": [
                "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
                "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
                "--autoplay-policy=no-user-gesture-required",
            ]});
            // A browser of another machine runs the Chrome its driver finds there, or the
            // one `LOAD_REMOTE_CHROME` names on that machine.
            let binary = match insecure {
                Some(origin) => {
                    if let Some(args) = options["args"].as_array_mut() {
                        args.push(json!(format!(
                            "--unsafely-treat-insecure-origin-as-secure={origin}"
                        )));
                    }
                    std::env::var("LOAD_REMOTE_CHROME")
                }
                None => std::env::var("BELT_CHROME"),
            };
            if let Ok(binary) = binary {
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
            let binary = if insecure.is_some() {
                // The page captures over plain HTTP from the other machine.
                options["prefs"]["media.devices.insecure.enabled"] = json!(true);
                options["prefs"]["media.getusermedia.insecure.enabled"] = json!(true);
                // An insecure origin gets a prompt the grants above do not answer.
                options["prefs"]["media.navigator.permission.disabled"] = json!(true);
                // The SFU reads the address of each candidate, never an mDNS name.
                options["prefs"]["media.peerconnection.ice.obfuscate_host_addresses"] =
                    json!(false);
                std::env::var("LOAD_REMOTE_FIREFOX")
            } else {
                std::env::var("BELT_FIREFOX")
            };
            if let Ok(binary) = binary {
                options["binary"] = json!(binary);
            }
            json!({ "browserName": "firefox", "moz:firefoxOptions": options })
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // A remote session ends with the run, so its browser closes; a local driver goes too.
        delete(self.driver, &self.session);
        if let Some(process) = self.process.as_mut() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

/// Ends the WebDriver session `session` on `driver`, and ignores the answer.
fn delete(driver: std::net::SocketAddr, session: &str) {
    use std::io::{Read, Write};
    if let Ok(mut stream) = std::net::TcpStream::connect(driver) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let request =
            format!("DELETE {session} HTTP/1.1\r\nHost: {driver}\r\nConnection: close\r\n\r\n");
        if stream.write_all(request.as_bytes()).is_ok() {
            let _ = stream.read_to_end(&mut Vec::new());
        }
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

/// The budget of the stall of one received video, per 10 minutes: under 1 s of freezes
/// (`ambre-spec-budgets.md`, the call video stall row).
pub const STALL_BUDGET: Duration = Duration::from_secs(1);

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

/// One line per received video of `seen` that decoded a second of pictures over `span`, with
/// its freezes per 10 minutes. Then whether each one holds [`STALL_BUDGET`]. A video whose
/// browser counts no freeze holds nothing, and no video at all fails.
pub fn stall_verdict(seen: &[Watched], span: Duration) -> (Vec<String>, bool) {
    let span = span.as_secs_f64().max(1e-9);
    let videos: Vec<&Watched> = seen.iter().filter(|w| w.decoded >= 30.0).collect();
    let mut holds = !videos.is_empty();
    let lines = videos
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let per10 = w.frozen.map(|f| f.as_secs_f64() / span * 600.0);
            let ok = per10.is_some_and(|f| f < STALL_BUDGET.as_secs_f64());
            holds &= ok;
            format!(
                "video {i} at {:?}: {} frozen per 10 minutes, {} freezes, {:.0} decoded: {}",
                w.engine,
                per10.map_or("no counter".to_owned(), |f| format!("{f:.3} s")),
                w.freezes.map_or("no".to_owned(), |n| format!("{n:.0}")),
                w.decoded,
                if ok { "pass" } else { "FAIL" }
            )
        })
        .collect();
    (lines, holds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(engine: Engine, decoded: f64, frozen: Option<f64>) -> Watched {
        Watched {
            engine,
            decoded,
            dropped: 0.0,
            freezes: frozen.map(|_| 1.0),
            frozen: frozen.map(Duration::from_secs_f64),
        }
    }

    #[test]
    fn each_received_video_must_freeze_under_one_second_per_ten_minutes() {
        let ten = Duration::from_secs(600);
        let seen = [
            video(Engine::Chrome, 18_000.0, Some(0.4)),
            video(Engine::Firefox, 18_000.0, Some(0.999)),
        ];
        let (lines, holds) = stall_verdict(&seen, ten);
        assert!(holds, "{lines:?}");
        assert_eq!(lines.len(), 2);
        // One second exactly misses a budget of under one second.
        let seen = [
            video(Engine::Chrome, 18_000.0, Some(0.4)),
            video(Engine::Chrome, 18_000.0, Some(1.0)),
        ];
        assert!(!stall_verdict(&seen, ten).1);
        // Over 5 minutes, 0.6 s is 1.2 s per 10 minutes.
        let seen = [video(Engine::Firefox, 9_000.0, Some(0.6))];
        assert!(!stall_verdict(&seen, Duration::from_secs(300)).1);
    }

    #[test]
    fn a_video_with_no_freeze_counter_or_no_video_at_all_holds_nothing() {
        let ten = Duration::from_secs(600);
        assert!(!stall_verdict(&[video(Engine::Firefox, 18_000.0, None)], ten).1);
        assert!(!stall_verdict(&[], ten).1);
        // A video that decoded less than a second of pictures is not a received video.
        let seen = [
            video(Engine::Chrome, 18_000.0, Some(0.1)),
            video(Engine::Chrome, 10.0, Some(30.0)),
        ];
        let (lines, holds) = stall_verdict(&seen, ten);
        assert!(holds);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn the_drivers_of_another_machine_read_from_one_line() {
        let remotes =
            Remote::listed("chrome@192.168.1.21:9515, firefox@192.168.1.21:4444,").unwrap();
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[1].engine, Engine::Firefox);
        assert_eq!(remotes[1].driver, "192.168.1.21:4444".parse().unwrap());
        assert!(Remote::listed("safari@192.168.1.21:4444").is_err());
        assert!(Remote::listed("chrome@host:9515").is_err());
        assert!(Remote::listed("chrome").is_err());
        assert_eq!(Remote::listed("").unwrap(), Vec::new());
    }
}
