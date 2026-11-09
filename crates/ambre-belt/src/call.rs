//! Two browsers in one call, through the SFU binary (`ambre-spec-call-media.md`).
//!
//! The belt starts the `ambre-sfu` binary on free ports, and serves one test page. The belt
//! signs each admission in the place of the relay. The page hands it to the SFU and opens the
//! fake camera and microphone of the engine. It sends both tracks to the SFU, and plays back what
//! the SFU forwards. The test reads the decoder counters through `getStats()`, as
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
window.joinedAt = Date.now();
// The peer connection exists once the SFU has answered the admission with its TURN servers.
let pc = null;
window.callState = 'starting';
window.callError = null;
window.beeps = [];
window.heard = [];
window.firstPicture = null;
window.firstAudio = null;
// Hears the onset of each tone in a remote audio track, by its peak.
const listen = (track) => {
  const ac = new AudioContext();
  const src = ac.createMediaStreamSource(new MediaStream([track]));
  const an = ac.createAnalyser();
  an.fftSize = 256;
  src.connect(an);
  const buf = new Float32Array(an.fftSize);
  let loud = false;
  setInterval(() => {
    an.getFloatTimeDomainData(buf);
    let peak = 0;
    for (const v of buf) peak = Math.max(peak, Math.abs(v));
    if (!loud && peak > 0.3) { loud = true; window.heard.push(Date.now()); }
    if (loud && peak < 0.05) loud = false;
  }, 2);
};
const onTrack = (e) => {
  const el = document.createElement(e.track.kind === 'video' ? 'video' : 'audio');
  el.autoplay = true;
  el.muted = true;
  el.srcObject = new MediaStream([e.track]);
  document.body.appendChild(el);
  if (e.track.kind === 'audio' && query.get('beep')) listen(e.track);
};
window.callStats = async () => {
  const out = { samples: null, packets: 0, video: 0, state: pc && pc.connectionState, relay: null };
  if (!pc) return out;
  const stats = await pc.getStats();
  stats.forEach((s) => {
    // The pair the transport selected. Chrome leaves `nominated` unset on the pair an ICE-lite
    // peer was nominated on, so the pair's own flag cannot name it.
    // Firefox reports no transport, and flags the pair it nominated instead.
    const pair = (s.type === 'transport' && stats.get(s.selectedCandidatePairId))
      || (s.type === 'candidate-pair' && s.nominated && s);
    if (pair && pair.state === 'succeeded') {
      const local = stats.get(pair.localCandidateId);
      if (local) out.relay = local.candidateType + '/' + (local.relayProtocol || local.protocol);
    }
    if (s.type !== 'inbound-rtp') return;
    if (s.kind === 'audio' && s.jitterBufferEmittedCount) out.audioBuffer = 1000 * s.jitterBufferDelay / s.jitterBufferEmittedCount;
    if (s.kind === 'audio') out.audioDetail = { jitter: s.jitter, lost: s.packetsLost, nack: s.nackCount, target: s.jitterBufferTargetDelay / (s.jitterBufferEmittedCount || 1) * 1000, minimum: s.jitterBufferMinimumDelay / (s.jitterBufferEmittedCount || 1) * 1000, accel: s.removedSamplesForAcceleration, decel: s.insertedSamplesForDeceleration, concealed: s.concealedSamples, received: s.totalSamplesReceived };
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
// The first decoded picture and the first audio packet, from the join on.
const watch = setInterval(async () => {
  const s = await window.callStats();
  if (window.firstPicture === null && s.video > 0) window.firstPicture = Date.now() - window.joinedAt;
  if (window.firstAudio === null && s.packets > 0) window.firstAudio = Date.now() - window.joinedAt;
  if (window.firstPicture !== null && window.firstAudio !== null) clearInterval(watch);
}, 20);
// `beep` replaces the microphone with a tone of 80 ms every second. It keeps the send time of
// each onset, so the other page measures mouth to ear.
const tone = () => {
  const ac = new AudioContext();
  const osc = ac.createOscillator();
  osc.frequency.value = 1000;
  const gain = ac.createGain();
  gain.gain.value = 0;
  osc.connect(gain);
  const dest = ac.createMediaStreamDestination();
  gain.connect(dest);
  osc.start();
  setInterval(() => {
    const at = ac.currentTime + 0.05;
    gain.gain.setValueAtTime(0.8, at);
    gain.gain.setValueAtTime(0, at + 0.08);
    window.beeps.push(Date.now() + (at - ac.currentTime) * 1000);
  }, 1000);
  return dest.stream.getAudioTracks()[0];
};
(async () => {
  try {
    const media = await navigator.mediaDevices.getUserMedia({ audio: true, video: true });
    if (query.get('beep')) {
      media.removeTrack(media.getAudioTracks()[0]);
      media.addTrack(tone());
    }
    const ws = new WebSocket(query.get('sfu'));
    // One message at a time: an offer of the SFU waits for the answer before it.
    let chain = Promise.resolve();
    let iced;
    const ice = new Promise((ok) => { iced = ok; });
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      chain = chain.then(async () => {
        if (msg.ice !== undefined) {
          iced(msg.ice);
        } else if (msg.answer !== undefined) {
          await pc.setRemoteDescription({ type: 'answer', sdp: msg.answer });
        } else if (msg.offer !== undefined) {
          await pc.setRemoteDescription({ type: 'offer', sdp: msg.offer });
          await pc.setLocalDescription(await pc.createAnswer());
          ws.send(JSON.stringify({ answer: pc.localDescription.sdp }));
        }
      }).catch((e) => { window.callError = String(e); });
    };
    await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
    // The admission the relay would issue, then the TURN servers the SFU minted a credential
    // for. `turn` makes the page gather relay candidates alone, as a network that opens TCP on
    // port 443 alone allows.
    ws.send(JSON.stringify({ admission: query.get('admission') }));
    const servers = await ice;
    const config = {};
    if (query.get('turn')) {
      config.iceServers = servers;
      config.iceTransportPolicy = 'relay';
    }
    pc = new RTCPeerConnection(config);
    pc.ontrack = onTrack;
    media.getTracks().forEach((t) => pc.addTrack(t, media));
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
    /// The WebSocket address of its signalling, on a port of its own.
    pub signal: String,
    /// The key the belt signs admissions with, in the place of the relay's. Drawn per run.
    relay: ed25519_dalek::SigningKey,
    process: Child,
}

/// 32 bytes from the generator of the OS.
fn drawn<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the generator of the OS");
    bytes
}

impl Sfu {
    /// Starts the SFU, and waits until its signalling port accepts a connection. With `turn`,
    /// the SFU mints credentials for that TURN server under its secret.
    pub async fn start(turn: Option<&Turn>) -> Self {
        let signal_port = free_port(Udp::No);
        let relay = ed25519_dalek::SigningKey::from_bytes(&drawn());
        let mut command = Command::new(binary());
        if let Some(turn) = turn {
            command
                .env("SFU_TURN_URLS", &turn.url)
                .env("SFU_TURN_SECRET", &turn.secret);
        }
        let process = command
            .env("SFU_SIGNAL_ADDR", format!("127.0.0.1:{signal_port}"))
            .env(
                "SFU_MEDIA_ADDR",
                format!("127.0.0.1:{}", free_port(Udp::Yes)),
            )
            .env(
                "SFU_ADMISSION_KEY",
                hex::encode(relay.verifying_key().as_bytes()),
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
            signal: format!("ws://127.0.0.1:{signal_port}/call"),
            relay,
            process,
        }
    }

    /// An admission to `room` for a fresh participant, signed as the relay signs one.
    pub fn admission(&self, room: [u8; 16]) -> String {
        use ed25519_dalek::Signer;
        let admission = ambre_protocol::signaling::Admission {
            room: ambre_core::FixedBytes::from_array(room),
            participant: ambre_core::FixedBytes::from_array(drawn()),
            expires: ambre_core::Instant::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("a clock after 1970")
                    .as_secs()
                    + 60,
            ),
        };
        let signed = admission.signed_bytes().expect("an admission encodes");
        let signature = self.relay.sign(signed.as_bytes()).to_bytes();
        let token = admission
            .token(&ambre_core::FixedBytes::from_array(signature))
            .expect("a token encodes");
        serde_json::to_value(&token)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .expect("a token is text")
    }

    /// The page URL that joins `room` on this SFU, with a fresh admission.
    pub fn page(&self, page: &str, room: [u8; 16]) -> String {
        format!(
            "{page}?sfu={}&admission={}",
            self.signal,
            self.admission(room)
        )
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

/// The port of the TCP front of the TURN relay: `BELT_TURN_PORT`, or 443.
fn turn_port() -> u16 {
    std::env::var("BELT_TURN_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(443)
}

/// A coturn process that serves TURN over TCP alone, behind a TCP front that delays each
/// direction by a fixed time. The front holds the port a network open on TCP 443 allows. It
/// checks the credentials the SFU mints, under the REST API of coturn (`CALL-014`).
pub struct Turn {
    /// The TURN URL the SFU hands a page.
    pub url: String,
    /// The secret coturn shares with the SFU alone. Drawn per run.
    secret: String,
    process: Child,
}

impl Turn {
    /// Starts coturn on a free port and the front on [`turn_port`], each direction delayed by
    /// `delay`. `BELT_TURNSERVER` names the coturn binary, `turnserver` by default.
    pub async fn start(delay: Duration) -> Self {
        let inner = free_port(Udp::No);
        let binary = std::env::var("BELT_TURNSERVER").unwrap_or_else(|_| "turnserver".into());
        let secret = hex::encode(drawn::<16>());
        let process = Command::new(binary)
            .args([
                "-n",
                "--no-udp",
                "--no-tls",
                "--no-dtls",
                "--no-cli",
                "--listening-ip=127.0.0.1",
                "--relay-ip=127.0.0.1",
                "--allow-loopback-peers",
                "--use-auth-secret",
                "--realm=ambre",
                "--min-port=49160",
                "--max-port=49999",
                "--log-file=stdout",
            ])
            .arg(format!("--listening-port={inner}"))
            .arg(format!("--static-auth-secret={secret}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start coturn: install it, or set BELT_TURNSERVER");
        let deadline = Instant::now() + Duration::from_secs(20);
        while tokio::net::TcpStream::connect(("127.0.0.1", inner))
            .await
            .is_err()
        {
            assert!(Instant::now() < deadline, "coturn did not listen");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let port = turn_port();
        let front = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect(
                "the TURN front binds its port: run as root, or lower ip_unprivileged_port_start",
            );
        tokio::spawn(async move {
            while let Ok((client, _)) = front.accept().await {
                if let Ok(server) = tokio::net::TcpStream::connect(("127.0.0.1", inner)).await {
                    // Nagle would hold small TURN frames back, which no TURN relay does.
                    let _ = client.set_nodelay(true);
                    let _ = server.set_nodelay(true);
                    let (cr, cw) = client.into_split();
                    let (sr, sw) = server.into_split();
                    tokio::spawn(delayed(cr, sw, delay));
                    tokio::spawn(delayed(sr, cw, delay));
                }
            }
        });
        Self {
            url: format!("turn:127.0.0.1:{port}?transport=tcp"),
            secret,
            process,
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Copies `from` to `to`, each chunk held for `delay`. The chunks keep their order.
async fn delayed(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    tokio::spawn(async move {
        while let Some((at, bytes)) = rx.recv().await {
            tokio::time::sleep_until(at.into()).await;
            if to.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let Some(bytes) = buf.get(..n) else { return };
                if tx.send((Instant::now() + delay, bytes.to_vec())).is_err() {
                    return;
                }
            }
        }
    }
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
        let sfu = Sfu::start(None).await;
        let page = serve_page().await;
        let room = drawn();
        let mut browsers = Vec::new();
        for _ in 0..2 {
            let url = sfu.page(&page, room);
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

    /// What one page decoded so far, when it first decoded each kind, and its tones.
    async fn heard(browser: &Browser) -> Value {
        browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 if (window.callError) { done({ error: window.callError }); return; } \
                 window.callStats().then((s) => done(Object.assign(s, { \
                   firstPicture: window.firstPicture, firstAudio: window.firstAudio, \
                   beeps: window.beeps, heard: window.heard })), \
                   (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await
    }

    /// The mouth-to-ear delays one page heard: each onset it heard, less the last onset the
    /// other page sent before it.
    fn mouth_to_ear(sent: &Value, heard: &Value) -> Vec<f64> {
        let sent: Vec<f64> = sent
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect();
        heard
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .filter_map(|h| {
                sent.iter()
                    .copied()
                    .filter(|s| *s <= h)
                    .fold(None, |best: Option<f64>, s| {
                        Some(best.map_or(s, |b| b.max(s)))
                    })
                    .map(|s| h - s)
                    .filter(|d| *d < 1000.0)
            })
            .collect()
    }

    /// `CALL-042`, `CALL-014`: a browser gathers TURN relay candidates over TCP alone, on port
    /// 443 at 150 ms of round trip. Its credential is the one the SFU minted for its pseudonym,
    /// under the secret the SFU and coturn alone share. It decodes the audio and the video of the
    /// call within the budget of 5 s. The test prints mouth to ear in each direction.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(
        not(feature = "browser"),
        ignore = "needs the SFU binary, coturn and a browser driver; run just test-browser"
    )]
    async fn a_browser_on_tcp_443_alone_decodes_the_call_through_turn() {
        // `BELT_TURN_DELAY_MS` changes the one-way delay of the TCP leg, 75 ms by default.
        let delay = std::env::var("BELT_TURN_DELAY_MS")
            .ok()
            .and_then(|d| d.parse().ok())
            .unwrap_or(75);
        let turn = Turn::start(Duration::from_millis(delay)).await;
        let sfu = Sfu::start(Some(&turn)).await;
        let page = serve_page().await;
        let room = drawn();
        // The relayed page uses the TURN server the SFU hands it, with the credential the SFU
        // minted for its pseudonym. coturn admits no other.
        let urls = [
            format!("{}&beep=1", sfu.page(&page, room)),
            format!("{}&beep=1&turn=1", sfu.page(&page, room)),
        ];
        let mut browsers = Vec::new();
        for url in &urls {
            let downloads = std::env::temp_dir().join(format!("belt-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&downloads).expect("a download directory");
            let browser = Browser::start(&downloads).await;
            browser.goto(url).await;
            browsers.push((browser, downloads));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let relayed = loop {
            let (relay_page, _) = browsers.get(1).expect("the relayed page");
            let s = heard(relay_page).await;
            assert!(s.get("error").is_none(), "the relayed page failed: {s}");
            if s["firstPicture"].is_number() && s["firstAudio"].is_number() {
                break s;
            }
            assert!(
                Instant::now() < deadline,
                "within 30 s, the relayed page decoded {s}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        assert_eq!(
            relayed["relay"],
            json!("relay/tcp"),
            "the page took {relayed}"
        );
        let picture = relayed["firstPicture"].as_f64().unwrap_or(f64::MAX);
        let audio = relayed["firstAudio"].as_f64().unwrap_or(f64::MAX);
        println!("through TURN: first audio {audio} ms, first picture {picture} ms after the join");
        assert!(
            picture <= 5000.0 && audio <= 5000.0,
            "the relayed page waited {audio} ms for audio and {picture} ms for a picture"
        );
        tokio::time::sleep(Duration::from_secs(15)).await;
        let direct = heard(&browsers.first().expect("the direct page").0).await;
        let relayed = heard(&browsers.get(1).expect("the relayed page").0).await;
        for (name, from, to) in [
            ("direct to relayed", &direct, &relayed),
            ("relayed to direct", &relayed, &direct),
        ] {
            let mut d = mouth_to_ear(&from["beeps"], &to["heard"]);
            d.sort_by(f64::total_cmp);
            let at = |p: f64| {
                d.get(((d.len().max(1) - 1) as f64 * p).round() as usize)
                    .copied()
            };
            println!(
                "mouth to ear, {name}: {} onsets, p50 {:?} ms, p95 {:?} ms; receiver audio buffer {} ms {}",
                d.len(),
                at(0.5),
                at(0.95),
                to["audioBuffer"],
                to["audioDetail"]
            );
        }
        drop(turn);
        for (browser, downloads) in browsers {
            drop(browser);
            let _ = std::fs::remove_dir_all(downloads);
        }
    }
}
