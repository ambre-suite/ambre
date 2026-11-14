//! The call proofs on a receiver of real hardware (`ambre-spec-call-proofs.md`).
//!
//! One run starts the relay and the SFU, and a proof host in front of them. The host serves the
//! release build of the application to the receiver under test. That receiver is a phone on the
//! network, or a Chrome with its GPU on this desktop. It injects one probe script into the page, and starts
//! the call worker through a timer script (`proof/probe.js`, `proof/call_shim.js`). The shipped
//! code runs unchanged: the scripts read it and post what they read to the host.
//!
//! Headless Chrome pages on this machine are the senders. Each is a member of one channel,
//! signed in through the relay, in one video call with a fake camera. They seal through their
//! own call worker, so the receiver opens real sealed chunks.
//!
//! The run measures in phases, then prints each figure against its threshold with a verdict.
//! No CI job runs it: `just call-proof-android`, `just call-proof-ios` and
//! `just call-proof-desktop` run it by hand.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::{HeaderValue, Response, StatusCode, Uri, header};
use serde_json::{Value, json};

use crate::serve;

/// The probe of the page, injected as its first script.
const PROBE: &str = include_str!("proof/probe.js");

/// The timer of the call worker, imported before the shipped loader.
const SHIM: &str = include_str!("proof/call_shim.js");

/// The script the probe starts as the call worker: the timer, then the shipped loader it names.
/// It is served at the root, beside the loader, so the loader finds its files where Trunk put
/// them. It imports a loader of this origin alone.
const WRAPPER: &str = "importScripts('/proof/call-shim.js');\n\
const loader = new URLSearchParams(self.location.search).get('loader') || '';\n\
if (!loader.startsWith('/') || loader.startsWith('//')) throw new Error('loader: ' + loader);\n\
importScripts(loader);\n";

/// The thresholds a proof reads its figures against. Each comes from the budget register
/// (`ambre-spec-budgets.md`) or from the roadmap row of the proof, never from this file alone.
pub mod threshold {
    /// `Call chunk, one seal or one open in the transform`: under 5 ms at the 99th percentile.
    pub const CHUNK_P99_MS: f64 = 5.0;
    /// `Call worker load`: under a quarter of one core.
    pub const WORKER_SHARE: f64 = 0.25;
    /// `Call video paint`: under 4 ms of UI thread per paint, at the 95th percentile.
    pub const PAINT_P95_MS: f64 = 4.0;
    /// `Call client proofs on Android`: nine videos reach the canvas at 30 fps.
    pub const CANVAS_FPS: f64 = 30.0;
    /// `Call client proofs on iOS`: the phone decodes the nine videos at 25 fps.
    pub const DECODE_FPS: f64 = 25.0;
    /// `Call videos forwarded to one receiver`: nine videos.
    pub const VIDEOS: usize = 9;
    /// The width of the top size of the call video row, 720p.
    pub const TOP_WIDTH: u64 = 1280;
    /// The width of the lowest size of the call video row, 180p.
    pub const LOW_WIDTH: u64 = 320;
}

/// What a figure says against its threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The figure holds its threshold.
    Pass,
    /// The figure misses its threshold, or no figure was read.
    Fail,
    /// A figure with no threshold, printed for the reader.
    Read,
}

impl Verdict {
    fn of(holds: bool) -> Self {
        if holds { Self::Pass } else { Self::Fail }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "FAIL",
            Self::Read => "--",
        }
    }
}

/// One line of the result: a figure, its threshold and its verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub name: String,
    pub value: String,
    pub threshold: String,
    pub verdict: Verdict,
}

impl Figure {
    fn new(name: &str, value: String, threshold: &str, verdict: Verdict) -> Self {
        Self {
            name: name.to_owned(),
            value,
            threshold: threshold.to_owned(),
            verdict,
        }
    }

    fn read(name: &str, value: String) -> Self {
        Self::new(name, value, "--", Verdict::Read)
    }
}

/// The value at the quantile `q` of `sorted`, by the nearest rank.
pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted.get(rank).copied().unwrap_or(f64::NAN)
}

/// The reports of one phase, from its first to its last, both included.
pub struct Window<'a> {
    pub reports: &'a [Value],
}

impl Window<'_> {
    fn first(&self) -> &Value {
        self.reports.first().unwrap_or(&Value::Null)
    }

    fn last(&self) -> &Value {
        self.reports.last().unwrap_or(&Value::Null)
    }

    /// The seconds between the first report and the last, on the clock of the page.
    pub fn seconds(&self) -> f64 {
        let at = |r: &Value| r["at"].as_f64().unwrap_or(0.0);
        (at(self.last()) - at(self.first())) / 1000.0
    }

    /// The paint measures of the phase. The first report closes the time before the phase.
    pub fn paints(&self) -> Vec<f64> {
        let mut paints: Vec<f64> = self
            .reports
            .iter()
            .skip(1)
            .flat_map(|r| r["paints"].as_array().cloned().unwrap_or_default())
            .filter_map(|v| v.as_f64())
            .collect();
        paints.sort_by(f64::total_cmp);
        paints
    }

    /// The seals of the phase that the Sync worker finished, and the time of each, in ms.
    pub fn seals(&self) -> (u64, u64, Vec<f64>) {
        let rest = self.reports.iter().skip(1);
        let sum = |field: &str| -> u64 {
            rest.clone()
                .map(|r| r["seals"][field].as_u64().unwrap_or(0))
                .sum()
        };
        let mut ms: Vec<f64> = rest
            .clone()
            .flat_map(|r| r["seals"]["ms"].as_array().cloned().unwrap_or_default())
            .filter_map(|v| v.as_f64())
            .collect();
        ms.sort_by(f64::total_cmp);
        (sum("done"), sum("failed"), ms)
    }
}

/// The figures of the call worker over `window`. They are the chunk rate, the 99th percentile
/// of the time it holds one chunk, and the share of the wall clock it holds any.
pub fn chunk_figures(window: &Window<'_>) -> Vec<Figure> {
    let (first, last) = (&window.first()["worker"], &window.last()["worker"]);
    if !first.is_object() || !last.is_object() {
        return vec![Figure::new(
            "call worker",
            "no report from the call worker".to_owned(),
            "a report",
            Verdict::Fail,
        )];
    }
    let bin = last["bin"].as_f64().unwrap_or(0.05);
    let counts = |w: &Value| -> Vec<u64> {
        w["hist"]
            .as_array()
            .map(|h| h.iter().map(|v| v.as_u64().unwrap_or(0)).collect())
            .unwrap_or_default()
    };
    let (before, after) = (counts(first), counts(last));
    let hist: Vec<u64> = after
        .iter()
        .enumerate()
        .map(|(i, n)| n.saturating_sub(before.get(i).copied().unwrap_or(0)))
        .collect();
    let over = last["over"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(first["over"].as_u64().unwrap_or(0));
    let total: u64 = hist.iter().sum::<u64>() + over;
    let elapsed = last["at"].as_f64().unwrap_or(0.0) - first["at"].as_f64().unwrap_or(0.0);
    let held = last["held"].as_f64().unwrap_or(0.0) - first["held"].as_f64().unwrap_or(0.0);
    if total == 0 || elapsed <= 0.0 {
        return vec![Figure::new(
            "call worker",
            "no chunk held in the phase".to_owned(),
            "a chunk",
            Verdict::Fail,
        )];
    }
    // The bin that holds the 99th percentile: bin `k` holds the spans in [k * bin, (k + 1) * bin).
    let rank = (total as f64 * 0.99).ceil() as u64;
    let mut seen = 0;
    let p99_bin = hist.iter().position(|n| {
        seen += n;
        seen >= rank
    });
    let limit = (threshold::CHUNK_P99_MS / bin).round() as usize;
    let (p99, p99_holds) = match p99_bin {
        Some(k) => (format!("below {:.2} ms", (k + 1) as f64 * bin), k < limit),
        None => (
            format!("at {:.0} ms or more", hist.len() as f64 * bin),
            false,
        ),
    };
    let share = held / elapsed;
    let kinds = last["counts"]
        .as_object()
        .map(|counts| {
            counts
                .iter()
                .map(|(kind, n)| {
                    let was = first["counts"][kind].as_u64().unwrap_or(0);
                    let rate =
                        n.as_u64().unwrap_or(0).saturating_sub(was) as f64 / elapsed * 1000.0;
                    format!("{kind} {rate:.0}/s")
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    vec![
        Figure::read(
            "chunks per second",
            format!("{:.0} ({kinds})", total as f64 / elapsed * 1000.0),
        ),
        Figure::new("one chunk held, p99", p99, "< 5 ms", Verdict::of(p99_holds)),
        Figure::new(
            "call worker held, share of one core",
            format!("{:.1} %", share * 100.0),
            "< 25 %",
            Verdict::of(share < threshold::WORKER_SHARE),
        ),
    ]
}

/// The figures of the paints of the videos over `window`: their rate, and the UI-thread time
/// of one paint.
pub fn paint_figures(window: &Window<'_>) -> Vec<Figure> {
    let paints = window.paints();
    let seconds = window.seconds();
    if paints.is_empty() || seconds <= 0.0 {
        return vec![Figure::new(
            "video paints",
            "no paint of a video".to_owned(),
            "a paint",
            Verdict::Fail,
        )];
    }
    let rate = paints.len() as f64 / seconds;
    let p95 = percentile(&paints, 0.95);
    vec![
        Figure::new(
            "video paints per second",
            format!("{rate:.1}"),
            ">= 30",
            Verdict::of(rate >= threshold::CANVAS_FPS),
        ),
        Figure::new(
            "UI thread per paint, p95",
            format!("{p95:.2} ms"),
            "< 4 ms",
            Verdict::of(p95 < threshold::PAINT_P95_MS),
        ),
        Figure::read(
            "UI thread per paint, median and max",
            format!(
                "{:.2} ms, {:.2} ms, over {} paints",
                percentile(&paints, 0.5),
                percentile(&paints, 1.0),
                paints.len()
            ),
        ),
    ]
}

/// Which sizes the videos of a phase must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizes {
    /// One video at the top size and the others at the lowest: the decode proof.
    OneTopEightLow,
    /// The sizes are printed, and hold nothing.
    Printed,
}

/// The figures of the received videos over `window`: how many decode, at what rate each, and
/// at which size.
pub fn video_figures(window: &Window<'_>, floor: f64, sizes: Sizes) -> Vec<Figure> {
    let (first, last) = (window.first(), window.last());
    let seconds = window.seconds();
    let videos = last["videosIn"].as_array().cloned().unwrap_or_default();
    let mut rows: Vec<(f64, u64, f64)> = videos
        .iter()
        .filter_map(|v| {
            let id = v["id"].as_str()?;
            let was = first["videosIn"]
                .as_array()
                .and_then(|a| a.iter().find(|b| b["id"].as_str() == Some(id)))
                .cloned()
                .unwrap_or(Value::Null);
            let delta = |f: &str| v[f].as_f64().unwrap_or(0.0) - was[f].as_f64().unwrap_or(0.0);
            let decoded = delta("decoded");
            (decoded > 0.0).then(|| {
                (
                    decoded / seconds.max(1e-9),
                    v["width"].as_u64().unwrap_or(0),
                    delta("frozen").max(0.0),
                )
            })
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let slowest = rows.first().map_or(0.0, |r| r.0);
    let top = rows.iter().filter(|r| r.1 == threshold::TOP_WIDTH).count();
    let low = rows.iter().filter(|r| r.1 == threshold::LOW_WIDTH).count();
    let widths: Vec<String> = rows.iter().map(|r| r.1.to_string()).collect();
    let frozen: f64 = rows.iter().map(|r| r.2).sum();
    let size_value = format!("{top} at 1280, {low} at 320 (widths {})", widths.join(" "));
    let size_figure = match sizes {
        Sizes::OneTopEightLow => Figure::new(
            "video sizes",
            size_value,
            "1 at 1280, 8 at 320",
            Verdict::of(top == 1 && low == threshold::VIDEOS - 1),
        ),
        Sizes::Printed => Figure::read("video sizes", size_value),
    };
    vec![
        Figure::new(
            "videos decoding",
            rows.len().to_string(),
            ">= 9",
            Verdict::of(rows.len() >= threshold::VIDEOS),
        ),
        Figure::new(
            "slowest video, decoded per second",
            format!("{slowest:.2}"),
            &format!(">= {floor:.0}"),
            Verdict::of(!rows.is_empty() && slowest >= floor),
        ),
        size_figure,
        Figure::read("freezes of all videos", format!("{frozen:.1} s")),
    ]
}

/// The figures of the file seals of the Sync worker over `window`, which the worker phase
/// needs while it measures.
pub fn seal_figures(window: &Window<'_>) -> Vec<Figure> {
    let (done, failed, ms) = window.seals();
    let busy: f64 = ms.iter().sum::<f64>() / 1000.0;
    let seconds = window.seconds();
    vec![Figure::new(
        "files the Sync worker sealed",
        format!(
            "{done}, {failed} failed, median {:.0} ms, busy {:.0} % of the phase",
            percentile(&ms, 0.5),
            busy / seconds.max(1e-9) * 100.0
        ),
        ">= 1, none failed",
        Verdict::of(done >= 1 && failed == 0),
    )]
}

/// The figure of the echo cancellation of the microphone, as its track reports it.
pub fn echo_figure(last: &Value) -> Figure {
    let echo = last["echo"].as_array().cloned().unwrap_or_default();
    let on = !echo.is_empty() && echo.iter().all(|e| e.as_bool() == Some(true));
    Figure::new(
        "echo cancellation of the microphone",
        Value::Array(echo).to_string(),
        "true",
        Verdict::of(on),
    )
}

/// The figures as a table a terminal prints.
pub fn table(figures: &[Figure]) -> String {
    let width = |f: fn(&Figure) -> &str| figures.iter().map(|x| f(x).len()).max().unwrap_or(0);
    let (a, b, c) = (
        width(|f| &f.name),
        width(|f| &f.value),
        width(|f| &f.threshold),
    );
    figures
        .iter()
        .map(|f| {
            format!(
                "  {:a$}  {:b$}  {:c$}  {}",
                f.name,
                f.value,
                f.threshold,
                f.verdict.word()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The day `seconds` after the epoch falls on, as `YYYY-MM-DD` in UTC.
pub fn date(seconds: u64) -> String {
    // The civil date of a day count, after Howard Hinnant's `civil_from_days`.
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The receiver a run measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// A phone on the network, which a person drives.
    Phone,
    /// A Chrome with a window and its GPU on this machine, which the run drives.
    Desktop,
    /// A headless browser of the belt on this machine, which the run drives. The browser tier
    /// holds the proof host itself this way, and measures nothing.
    Headless,
}

/// What one run does, read from its variables.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub receiver: Receiver,
    /// The address of this machine the receiver reaches: the page and the media of the SFU.
    pub host: IpAddr,
    pub port: u16,
    /// The certificate and its key, in PEM, that a phone trusts for `host`.
    pub tls: Option<(PathBuf, PathBuf)>,
    pub senders: usize,
    /// The length of the paint phase, and of the worker phase.
    pub window: Duration,
    /// The length of the decode phase, none when zero.
    pub decode: Duration,
}

impl Plan {
    /// Reads the plan from `var`: `PROOF_RECEIVER`, `PROOF_HOST`, `PROOF_PORT`, `PROOF_CERT`,
    /// `PROOF_KEY`, `PROOF_SENDERS`, `PROOF_SECONDS` and `PROOF_DECODE_MINUTES`.
    ///
    /// # Errors
    ///
    /// The variable a phone needs and that is missing or malformed, by name.
    pub fn read(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let number = |name: &str, fallback: u64| -> Result<u64, String> {
            var(name).map_or(Ok(fallback), |v| {
                v.trim()
                    .parse()
                    .map_err(|_| format!("{name}: not a whole number: {v}"))
            })
        };
        let receiver = match var("PROOF_RECEIVER").as_deref() {
            None | Some("phone") => Receiver::Phone,
            Some("desktop") => Receiver::Desktop,
            Some("headless") => Receiver::Headless,
            Some(other) => {
                return Err(format!(
                    "PROOF_RECEIVER: phone, desktop or headless, not {other}"
                ));
            }
        };
        let host = match (receiver, var("PROOF_HOST")) {
            (_, Some(host)) => host
                .trim()
                .parse()
                .map_err(|_| format!("PROOF_HOST: not an IP address: {host}"))?,
            (Receiver::Desktop | Receiver::Headless, None) => std::net::Ipv4Addr::LOCALHOST.into(),
            (Receiver::Phone, None) => {
                return Err(
                    "PROOF_HOST: the address of this machine on the network of the phone".into(),
                );
            }
        };
        let tls = match (var("PROOF_CERT"), var("PROOF_KEY")) {
            (Some(cert), Some(key)) => Some((PathBuf::from(cert), PathBuf::from(key))),
            (None, None) if receiver != Receiver::Phone => None,
            _ => {
                return Err(
                    "PROOF_CERT and PROOF_KEY: the certificate of PROOF_HOST and its key, in PEM"
                        .into(),
                );
            }
        };
        let fallback_port = if tls.is_some() { 8443 } else { 0 };
        Ok(Self {
            receiver,
            host,
            port: u16::try_from(number("PROOF_PORT", fallback_port)?)
                .map_err(|_| "PROOF_PORT: past 65535".to_owned())?,
            tls,
            senders: usize::try_from(number("PROOF_SENDERS", 9)?)
                .map_err(|_| "PROOF_SENDERS: too large".to_owned())?
                .max(1),
            window: Duration::from_secs(number("PROOF_SECONDS", 60)?.max(10)),
            decode: Duration::from_secs(number("PROOF_DECODE_MINUTES", 0)? * 60),
        })
    }

    /// The scheme of the page: a phone needs a secure context, the loopback is one already.
    fn scheme(&self) -> &'static str {
        if self.tls.is_some() { "https" } else { "http" }
    }

    /// How many videos the receiver decodes: one per sender, nine at most.
    pub fn videos(&self) -> usize {
        self.senders.min(threshold::VIDEOS)
    }
}

/// What the host keeps of one run: the reports of the probe, and whether it asks for seals.
#[derive(Default)]
pub struct Host {
    reports: Mutex<Vec<Value>>,
    seal: AtomicBool,
}

impl Host {
    /// The reports so far.
    pub fn reports(&self) -> Vec<Value> {
        self.reports.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// How many reports so far.
    pub fn len(&self) -> usize {
        self.reports.lock().map(|r| r.len()).unwrap_or(0)
    }

    /// Asks the probe to seal, or to stop.
    pub fn sealing(&self, on: bool) {
        self.seal.store(on, Ordering::SeqCst);
    }
}

/// The identifier of the seal requests of the probe: far above any the page issues.
const SEAL_ID: u64 = 1 << 52;

/// The command that seals 64 KiB under a passphrase, as the Logic thread posts it to the Sync
/// worker. Then the first bytes of its two answers. The probe posts the command, and reads an
/// answer by its first bytes.
pub fn seal_messages() -> Value {
    use ambre_bus::{LogicEvent, Request, RestCommand, RestEvent, SyncCommand};
    let id: ambre_core::Seq<Request> =
        postcard::from_bytes(&postcard::to_allocvec(&SEAL_ID).expect("an id encodes"))
            .expect("an id decodes");
    let command = SyncCommand::Rest {
        id,
        cmd: RestCommand::SealDocFile {
            plaintext: ambre_core::Confidential::<ambre_core::Plaintext>::new(vec![0x5a; 65_536]),
            passphrase: ambre_core::Confidential::<ambre_core::Passphrase>::new(
                "a passphrase of the call proof".to_owned(),
            ),
        },
    };
    let answer = |evt: RestEvent| {
        postcard::to_allocvec(&LogicEvent::Rest { id: Some(id), evt }).expect("an answer encodes")
    };
    let mut sealed = answer(RestEvent::DocFileSealed {
        data: ambre_core::Payload::new(Vec::new()),
    });
    // The empty data ends on its length, one byte of zero, which a real answer does not share.
    sealed.pop();
    json!({
        "command": postcard::to_allocvec(&command).expect("a command encodes"),
        "sealed": sealed,
        "failed": answer(RestEvent::DocFileSealFailed),
    })
}

/// A response with `body` of `media`, cross-origin isolated and never cached.
fn file(body: impl Into<Body>, media: &str) -> Response<Body> {
    let mut resp = Response::new(body.into());
    if let Ok(value) = HeaderValue::from_str(media) {
        resp.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    serve::isolate(&mut resp);
    resp
}

/// The page with the probe as the first script of its head.
pub fn inject(page: &str) -> String {
    let tag = "<script src=\"/proof/probe.js\"></script>";
    match page
        .find("<head")
        .and_then(|at| page[at..].find('>').map(|end| at + end + 1))
    {
        Some(at) => format!("{}{tag}{}", &page[..at], &page[at..]),
        None => format!("{tag}{page}"),
    }
}

/// Answers a built file under `root`, the page with its probe.
async fn built(root: PathBuf, uri: Uri) -> Response<Body> {
    let path = serve::resolve(&root, &uri);
    match tokio::fs::read(&path).await {
        Ok(bytes) if path.file_name().is_some_and(|n| n == "index.html") => file(
            inject(&String::from_utf8_lossy(&bytes)),
            serve::media_type(&path),
        ),
        Ok(bytes) => file(bytes, serve::media_type(&path)),
        Err(_) => {
            let mut resp = Response::new(Body::empty());
            *resp.status_mut() = StatusCode::NOT_FOUND;
            resp
        }
    }
}

/// The routes of the proof host: the build with its probe, the relay and the SFU behind it,
/// and the two endpoints of the probe.
fn router(root: PathBuf, relay: String, sfu: String, host: Arc<Host>) -> axum::Router {
    use axum::routing::{get, post};
    let peer = serve::page_address();
    let socket = |upstream: String, peer: String| {
        move |ws: WebSocketUpgrade, uri: Uri| {
            let (upstream, peer) = (upstream.clone(), peer.clone());
            let path = uri.path().to_owned();
            async move { ws.on_upgrade(move |page| serve::join_socket(page, upstream, peer, path)) }
        }
    };
    let sfu_socket = {
        let peer = peer.clone();
        move |ws: WebSocketUpgrade| {
            let (sfu, peer) = (sfu.clone(), peer.clone());
            async move { ws.on_upgrade(move |page| serve::join_socket(page, sfu, peer, "/call".into())) }
        }
    };
    let seal = seal_messages();
    let reported = Arc::clone(&host);
    axum::Router::new()
        .route("/api/ws", get(socket(relay.clone(), peer.clone())))
        .route(
            "/api/calls/{call}/socket",
            get(socket(relay.clone(), peer.clone())),
        )
        .route("/sfu/call", get(sfu_socket))
        .route(
            "/proof/probe.js",
            get(|| async { file(PROBE, "text/javascript") }),
        )
        .route(
            "/proof/call-shim.js",
            get(|| async { file(SHIM, "text/javascript") }),
        )
        .route(
            "/ambre-proof-call-worker.js",
            get(|| async { file(WRAPPER, "text/javascript") }),
        )
        .route(
            "/proof/seal",
            get(move || {
                let seal = seal.clone();
                async move { file(seal.to_string(), "application/json") }
            }),
        )
        .route(
            "/proof/report",
            post(move |body: axum::body::Bytes| {
                let host = Arc::clone(&reported);
                async move {
                    if let Ok(report) = serde_json::from_slice::<Value>(&body)
                        && let Ok(mut reports) = host.reports.lock()
                    {
                        reports.push(report);
                    }
                    let seal = host.seal.load(Ordering::SeqCst);
                    file(json!({ "seal": seal }).to_string(), "application/json")
                }
            }),
        )
        .fallback(move |req: Request<Body>| {
            let (root, relay, peer) = (root.clone(), relay.clone(), peer.clone());
            async move {
                if req.uri().path().starts_with("/api/") {
                    serve::forward(relay, peer, req).await
                } else {
                    built(root, req.uri().clone()).await
                }
            }
        })
}

/// A TLS listener for `axum::serve`: each handshake runs on a task of its own, so a client
/// that stalls in one holds no other.
struct TlsListener {
    accepted: tokio::sync::mpsc::Receiver<(
        tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
        SocketAddr,
    )>,
    local: SocketAddr,
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.accepted.recv().await {
            Some(accepted) => accepted,
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Wraps `listener` in TLS under the certificate `cert` and its key `key`, both in PEM.
fn tls(listener: tokio::net::TcpListener, cert: &Path, key: &Path) -> TlsListener {
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .unwrap_or_else(|e| panic!("PROOF_CERT {}: {e}", cert.display()))
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("PROOF_CERT {}: {e}", cert.display()));
    let key = PrivateKeyDer::from_pem_file(key)
        .unwrap_or_else(|e| panic!("PROOF_KEY {}: {e}", key.display()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the protocol versions of rustls")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("PROOF_CERT and PROOF_KEY: a certificate and its own key");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let local = listener.local_addr().expect("the address of the host");
    let (tx, accepted) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, addr)) = listener.accept().await else {
                continue;
            };
            let (acceptor, tx) = (acceptor.clone(), tx.clone());
            tokio::spawn(async move {
                let shake = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp));
                if let Ok(Ok(stream)) = shake.await {
                    let _ = tx.send((stream, addr)).await;
                }
            });
        }
    });
    TlsListener { accepted, local }
}

/// A host that is bound and not yet serving. The relay needs its address before it starts,
/// and the host needs the relay's before it serves.
pub struct Bound {
    listener: tokio::net::TcpListener,
    pub origin: String,
}

impl Bound {
    /// Binds the port of `plan` on its address, or on every address for a phone.
    pub async fn bind(plan: &Plan) -> Self {
        let on: IpAddr = match plan.receiver {
            Receiver::Phone => std::net::Ipv4Addr::UNSPECIFIED.into(),
            Receiver::Desktop | Receiver::Headless => plan.host,
        };
        let listener = tokio::net::TcpListener::bind((on, plan.port))
            .await
            .unwrap_or_else(|e| panic!("bind the proof host on port {}: {e}", plan.port));
        let port = listener.local_addr().expect("the host's address").port();
        let origin = format!(
            "{}://{}",
            plan.scheme(),
            SocketAddr::from((plan.host, port))
        );
        Self { listener, origin }
    }

    /// The address a page opens the signalling of the SFU at, behind this host.
    pub fn sfu_url(&self) -> String {
        format!(
            "{}/sfu/call",
            self.origin
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        )
    }

    /// Serves the build in `root` until the run ends. `relay` and `sfu` are the HTTP base of
    /// the relay and of the signalling of the SFU.
    pub fn serve(self, plan: &Plan, root: PathBuf, relay: String, sfu: String, host: Arc<Host>) {
        assert!(
            root.join("index.html").is_file(),
            "no build in {}: run `trunk build`, or name an absolute BELT_DIST",
            root.display()
        );
        let app = router(root, relay, sfu, host);
        match &plan.tls {
            Some((cert, key)) => {
                let listener = tls(self.listener, cert, key);
                tokio::spawn(async move { axum::serve(listener, app).await });
            }
            None => {
                let listener = self.listener;
                tokio::spawn(async move { axum::serve(listener, app).await });
            }
        }
    }
}

#[cfg(test)]
mod run;

#[cfg(test)]
mod tests {
    use super::*;

    /// A report of the probe at `at` ms, with the totals of the call worker `worker`.
    fn report(at: f64, worker: Value) -> Value {
        json!({ "at": at, "worker": worker, "paints": [], "videosIn": [] })
    }

    /// Totals of the call worker: `n` chunks in the bin `bin` of 0.05 ms, `held` ms held.
    fn totals(at: f64, bin: usize, n: u64, held: f64) -> Value {
        let mut hist = vec![0u64; 400];
        hist[bin] = n;
        json!({ "bin": 0.05, "hist": hist, "over": 0, "held": held, "at": at,
                "counts": { "send video": n } })
    }

    fn verdict(figures: &[Figure], name: &str) -> Verdict {
        figures
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no figure {name} in {figures:?}"))
            .verdict
    }

    #[test]
    fn a_chunk_below_five_ms_at_the_99th_percentile_passes_and_one_at_five_fails() {
        // 1 000 chunks in the bin [4.95, 5.0) over 10 s, held 1 s: a pass on both.
        let reports = [
            report(0.0, totals(0.0, 0, 0, 0.0)),
            report(10_000.0, totals(10_000.0, 99, 1_000, 1_000.0)),
        ];
        let figures = chunk_figures(&Window { reports: &reports });
        assert_eq!(verdict(&figures, "one chunk held, p99"), Verdict::Pass);
        assert_eq!(
            verdict(&figures, "call worker held, share of one core"),
            Verdict::Pass
        );
        assert_eq!(figures[0].value, "100 (send video 100/s)");
        // The bin [5.0, 5.05) is 5 ms, which the budget refuses.
        let reports = [
            report(0.0, totals(0.0, 0, 0, 0.0)),
            report(10_000.0, totals(10_000.0, 100, 1_000, 2_500.0)),
        ];
        let figures = chunk_figures(&Window { reports: &reports });
        assert_eq!(verdict(&figures, "one chunk held, p99"), Verdict::Fail);
        // 2 500 ms held over 10 000 ms is 25 %, which "under 25 %" refuses.
        assert_eq!(
            verdict(&figures, "call worker held, share of one core"),
            Verdict::Fail
        );
    }

    #[test]
    fn the_99th_percentile_reads_the_chunks_of_the_phase_alone() {
        // Before the phase, every chunk was slow; in it, 99 fast and 1 slow.
        let mut before = totals(0.0, 300, 500, 0.0);
        before["over"] = json!(7);
        let mut after = totals(5_000.0, 300, 501, 100.0);
        after["hist"][2] = json!(99);
        after["over"] = json!(7);
        let reports = [report(0.0, before), report(5_000.0, after)];
        let figures = chunk_figures(&Window { reports: &reports });
        assert_eq!(verdict(&figures, "one chunk held, p99"), Verdict::Pass);
        assert_eq!(figures[1].value, "below 0.15 ms");
    }

    #[test]
    fn a_phase_with_no_report_of_the_call_worker_fails() {
        let reports = [report(0.0, Value::Null), report(1_000.0, Value::Null)];
        let figures = chunk_figures(&Window { reports: &reports });
        assert_eq!(figures.len(), 1);
        assert_eq!(figures[0].verdict, Verdict::Fail);
    }

    #[test]
    fn paints_read_their_rate_and_their_95th_percentile() {
        let paints: Vec<f64> = (0..300).map(|i| if i < 290 { 2.0 } else { 9.0 }).collect();
        let reports = [
            json!({ "at": 0.0, "paints": [50.0] }),
            json!({ "at": 5_000.0, "paints": paints }),
        ];
        let figures = paint_figures(&Window { reports: &reports });
        // The paint of 50 ms came before the phase, so it counts for nothing.
        assert_eq!(figures[0].value, "60.0");
        assert_eq!(verdict(&figures, "video paints per second"), Verdict::Pass);
        assert_eq!(verdict(&figures, "UI thread per paint, p95"), Verdict::Pass);
        let slow = [
            json!({ "at": 0.0, "paints": [] }),
            json!({ "at": 10_000.0, "paints": vec![4.0; 290] }),
        ];
        let figures = paint_figures(&Window { reports: &slow });
        assert_eq!(verdict(&figures, "video paints per second"), Verdict::Fail);
        assert_eq!(verdict(&figures, "UI thread per paint, p95"), Verdict::Fail);
    }

    /// A report with `videos` received: (id, width, decoded frames).
    fn videos(at: f64, videos: &[(&str, u64, f64)]) -> Value {
        let list: Vec<Value> = videos
            .iter()
            .map(|(id, width, decoded)| json!({ "id": id, "width": width, "decoded": decoded, "frozen": 0.0 }))
            .collect();
        json!({ "at": at, "videosIn": list })
    }

    #[test]
    fn nine_videos_at_one_top_and_eight_low_pass_the_decode_proof() {
        let ids: Vec<String> = (0..9).map(|i| format!("v{i}")).collect();
        let before: Vec<(&str, u64, f64)> =
            ids.iter().map(|id| (id.as_str(), 320, 100.0)).collect();
        let mut after: Vec<(&str, u64, f64)> = ids
            .iter()
            .map(|id| (id.as_str(), 320, 100.0 + 1_500.0))
            .collect();
        after[0].1 = 1280;
        let reports = [videos(0.0, &before), videos(60_000.0, &after)];
        let figures = video_figures(&Window { reports: &reports }, 25.0, Sizes::OneTopEightLow);
        assert_eq!(verdict(&figures, "videos decoding"), Verdict::Pass);
        assert_eq!(
            verdict(&figures, "slowest video, decoded per second"),
            Verdict::Pass
        );
        assert_eq!(verdict(&figures, "video sizes"), Verdict::Pass);
        // The same 25 pictures a second fail the canvas floor of 30.
        let figures = video_figures(&Window { reports: &reports }, 30.0, Sizes::Printed);
        assert_eq!(
            verdict(&figures, "slowest video, decoded per second"),
            Verdict::Fail
        );
        assert_eq!(verdict(&figures, "video sizes"), Verdict::Read);
    }

    #[test]
    fn a_video_that_decodes_nothing_in_the_phase_is_not_counted() {
        let before = [("a", 320, 10.0), ("b", 320, 10.0)];
        let after = [("a", 320, 310.0), ("b", 320, 10.0)];
        let reports = [videos(0.0, &before), videos(10_000.0, &after)];
        let figures = video_figures(&Window { reports: &reports }, 25.0, Sizes::OneTopEightLow);
        assert_eq!(figures[0].value, "1");
        assert_eq!(verdict(&figures, "videos decoding"), Verdict::Fail);
        assert_eq!(verdict(&figures, "video sizes"), Verdict::Fail);
    }

    #[test]
    fn the_seals_of_the_phase_count_from_its_second_report() {
        let reports = [
            json!({ "at": 0.0, "seals": { "done": 5, "failed": 1, "ms": [1.0] } }),
            json!({ "at": 2_000.0, "seals": { "done": 1, "failed": 0, "ms": [900.0] } }),
            json!({ "at": 4_000.0, "seals": { "done": 2, "failed": 0, "ms": [800.0, 1_000.0] } }),
        ];
        let window = Window { reports: &reports };
        assert_eq!(window.seals(), (3, 0, vec![800.0, 900.0, 1_000.0]));
        assert_eq!(seal_figures(&window)[0].verdict, Verdict::Pass);
        let idle = [json!({ "at": 0.0 }), json!({ "at": 2_000.0 })];
        assert_eq!(
            seal_figures(&Window { reports: &idle })[0].verdict,
            Verdict::Fail
        );
    }

    #[test]
    fn echo_cancellation_passes_only_when_every_microphone_reports_it() {
        assert_eq!(
            echo_figure(&json!({ "echo": [true] })).verdict,
            Verdict::Pass
        );
        assert_eq!(
            echo_figure(&json!({ "echo": [true, false] })).verdict,
            Verdict::Fail
        );
        assert_eq!(echo_figure(&json!({ "echo": [] })).verdict, Verdict::Fail);
    }

    #[test]
    fn a_date_reads_as_its_civil_day() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_794_528_000), "2026-11-13");
    }

    #[test]
    fn the_probe_is_the_first_script_of_the_head() {
        let page = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\" />";
        let injected = inject(page);
        assert!(
            injected.starts_with(
                "<!DOCTYPE html>\n<html lang=\"en\">\n<head><script src=\"/proof/probe.js\"></script>\n"
            ),
            "{injected}"
        );
    }

    #[test]
    fn a_plan_names_what_a_phone_lacks() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let missing = Plan::read(vars(&[])).unwrap_err();
        assert!(missing.starts_with("PROOF_HOST"), "{missing}");
        let no_cert = Plan::read(vars(&[("PROOF_HOST", "192.168.1.20")])).unwrap_err();
        assert!(no_cert.starts_with("PROOF_CERT"), "{no_cert}");
        let phone = Plan::read(vars(&[
            ("PROOF_HOST", "192.168.1.20"),
            ("PROOF_CERT", "c.pem"),
            ("PROOF_KEY", "k.pem"),
            ("PROOF_DECODE_MINUTES", "30"),
        ]))
        .unwrap();
        assert_eq!(phone.port, 8443);
        assert_eq!(phone.senders, 9);
        assert_eq!(phone.decode, Duration::from_secs(1_800));
        assert_eq!(phone.scheme(), "https");
        let desktop = Plan::read(vars(&[
            ("PROOF_RECEIVER", "desktop"),
            ("PROOF_SENDERS", "3"),
        ]))
        .unwrap();
        assert_eq!(desktop.host, IpAddr::from(std::net::Ipv4Addr::LOCALHOST));
        assert_eq!((desktop.port, desktop.videos()), (0, 3));
        assert_eq!(desktop.scheme(), "http");
        let bad = Plan::read(vars(&[("PROOF_RECEIVER", "tablet")])).unwrap_err();
        assert!(bad.starts_with("PROOF_RECEIVER"), "{bad}");
    }

    #[test]
    fn the_seal_answer_prefix_is_a_prefix_of_a_real_answer() {
        use ambre_bus::{LogicEvent, RestEvent};
        let messages = seal_messages();
        let prefix: Vec<u8> = serde_json::from_value(messages["sealed"].clone()).unwrap();
        let id: ambre_core::Seq<ambre_bus::Request> =
            postcard::from_bytes(&postcard::to_allocvec(&SEAL_ID).unwrap()).unwrap();
        let real = postcard::to_allocvec(&LogicEvent::Rest {
            id: Some(id),
            evt: RestEvent::DocFileSealed {
                data: ambre_core::Payload::new(vec![7; 70_000]),
            },
        })
        .unwrap();
        assert!(real.starts_with(&prefix));
        let failed: Vec<u8> = serde_json::from_value(messages["failed"].clone()).unwrap();
        assert!(!real.starts_with(&failed));
        let command: Vec<u8> = serde_json::from_value(messages["command"].clone()).unwrap();
        let decoded: ambre_bus::SyncCommand = postcard::from_bytes(&command).unwrap();
        assert!(
            matches!(
                decoded,
                ambre_bus::SyncCommand::Rest {
                    cmd: ambre_bus::RestCommand::SealDocFile { .. },
                    ..
                }
            ),
            "{decoded:?}"
        );
    }
}
