//! The load run of a call, in real time. The SFU binary runs on real sockets, the synthetic
//! participants on their own threads, and the impairment relay sits between them.
//!
//! No CI job holds it: a shared runner gives no stable CPU time. It prints the figures the
//! budget register quotes (`ambre-spec-budgets.md`), and fails on none of them.
//!
//! ```text
//! cargo build -p ambre-sfu --release
//! cargo test -p ambre-sfu --release -- --ignored --nocapture load_of_one_call
//! ```
//!
//! `LOAD_PARTICIPANTS`, `LOAD_SECONDS` and `LOAD_SEED` change the run. `LOAD_SFU_CPUS` pins
//! the SFU to a set of cores, `0,1` for the 2 vCPU of the budget; run the test itself under
//! `taskset -c 2,3` then.
//!
//! `LOAD_BROWSERS`, for example `chrome,chrome,firefox,firefox`, puts that many headless
//! browsers among the participants (`sim/browsers.rs`). They join first, and the run prints the
//! freezes their decoders count beside the stall of the synthetic model.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use str0m::change::{SdpAnswer, SdpOffer};
use tokio::net::UdpSocket;
use tokio_tungstenite::tungstenite::Message;

use super::browsers::{Browser, Engine, PAGE, Watched, watched};
use super::client::Synth;
use super::impair::{Impairment, Profile};
use super::udp::{Link, link};
use crate::serve::Addresses;

fn setting(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

fn free(udp: bool) -> SocketAddr {
    if udp {
        std::net::UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr())
    } else {
        std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr())
    }
    .expect("a free port")
}

/// The CPU time of a process, from `/proc`, in clock ticks of 10 ms.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("the stat of the SFU");
    let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    let fields: Vec<&str> = after.split_whitespace().collect();
    // utime and stime are fields 14 and 15 of the line, 12 and 13 after the name.
    let field = |i: usize| {
        fields
            .get(i)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    field(11) + field(12)
}

/// The SFU binary, as a process of its own: `LOAD_SFU`, or the release build.
struct SfuProcess {
    child: std::process::Child,
    addresses: Addresses,
}

impl Drop for SfuProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the SFU binary, and waits until its signalling port accepts a connection.
fn start_sfu() -> SfuProcess {
    let addresses = Addresses {
        signal: free(false),
        media: free(true),
    };
    let binary = std::env::var("LOAD_SFU").unwrap_or_else(|_| {
        format!(
            "{}/../../target/release/ambre-sfu",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    // `LOAD_SFU_CPUS` pins the SFU to those cores, so it runs on the budget alone and the
    // participants run beside it.
    let mut command = match std::env::var("LOAD_SFU_CPUS") {
        Ok(cpus) => {
            let mut c = std::process::Command::new("taskset");
            c.args(["-c", &cpus, &binary]);
            c
        }
        Err(_) => std::process::Command::new(&binary),
    };
    let child = command
        .env("SFU_SIGNAL_ADDR", addresses.signal.to_string())
        .env("SFU_MEDIA_ADDR", addresses.media.to_string())
        .env(
            "SFU_ADMISSION_KEY",
            hex::encode(relay_key().verifying_key().as_bytes()),
        )
        .spawn()
        .expect("start the SFU: run `cargo build -p ambre-sfu --release`, or set LOAD_SFU");
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(addresses.signal).is_err() {
        assert!(Instant::now() < deadline, "the SFU did not listen");
        std::thread::sleep(Duration::from_millis(100));
    }
    SfuProcess { child, addresses }
}

/// The key the load signs its admissions with, in the place of the relay's.
fn relay_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[7; 32])
}

/// An admission to the one room of the load, for participant `index`.
fn admission(index: u64) -> String {
    let admission = ambre_protocol::signaling::Admission {
        room: ambre_core::FixedBytes::from_array([1; 16]),
        participant: ambre_core::FixedBytes::from_array(*uuid_bytes(index).as_array()),
        expires: ambre_core::Instant::new(u64::MAX / 2),
    };
    let token = crate::admit::signed(&relay_key(), &admission);
    serde_json::to_value(&token)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .expect("a token is text")
}

/// The pseudonym of participant `index`: its number, in the last bytes.
fn uuid_bytes(index: u64) -> ambre_core::FixedBytes<ambre_core::ParticipantPseudonym> {
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&index.to_be_bytes());
    ambre_core::FixedBytes::from_array(bytes)
}

/// What one participant does in the run.
struct Plan {
    index: u16,
    signal: String,
    sfu: SocketAddr,
    start: Instant,
    epoch: Instant,
    end: Instant,
    /// The turns of speech: participant `turns[k]` speaks from `turn * k`.
    turn: Duration,
    speakers: u16,
    /// The round trip of the link, which the playout model adds.
    rtt: Duration,
}

/// Runs one participant until the end of the plan, and answers it.
async fn participant(plan: Plan, link: Link) -> Synth {
    tokio::time::sleep_until(plan.start.into()).await;
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("a socket");
    let addr = socket.local_addr().expect("an address");
    let now = Instant::now();
    let (mut synth, offer) = Synth::new(now, plan.epoch, plan.index, addr);
    synth.playout_rtt = plan.rtt;
    let url = format!("{}/call", plan.signal);
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("signalling");
    let text = json!({ "admission": admission(u64::from(plan.index)) }).to_string();
    ws.send(Message::Text(text.into()))
        .await
        .expect("the admission");
    let text = json!({ "offer": offer.to_sdp_string() }).to_string();
    ws.send(Message::Text(text.into()))
        .await
        .expect("the offer");
    let mut buf = vec![0u8; 2048];
    loop {
        let now = Instant::now();
        if now >= plan.end {
            return synth;
        }
        let elapsed = now.saturating_duration_since(plan.epoch);
        let turn = (elapsed.as_millis() / plan.turn.as_millis().max(1)) as u64;
        synth.speaking = u64::from(plan.index) == turn % u64::from(plan.speakers);
        let wake = synth
            .wake
            .min(now + Duration::from_millis(5))
            .max(now + Duration::from_micros(500));
        tokio::select! {
            read = socket.recv_from(&mut buf) => {
                if let Ok((n, _)) = read
                    && let Some(bytes) = buf.get(..n)
                {
                    synth.receive(Instant::now(), plan.sfu, bytes);
                }
            }
            msg = ws.next() => {
                let Some(Ok(Message::Text(text))) = msg else { continue };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                if let Some(answer) = value["answer"].as_str() {
                    synth.accept_answer(SdpAnswer::from_sdp_string(answer).expect("an answer"));
                } else if let Some(offer) = value["offer"].as_str() {
                    let answer = synth.accept_offer(SdpOffer::from_sdp_string(offer).expect("an offer"));
                    let text = json!({ "answer": answer.to_sdp_string() }).to_string();
                    ws.send(Message::Text(text.into())).await.expect("the answer");
                }
            }
            () = tokio::time::sleep_until(wake.into()) => {}
        }
        synth.tick(Instant::now());
        for t in synth.outbox.drain(..) {
            let _ = socket.send_to(&t.contents, link.front).await;
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted.get(rank).copied().unwrap_or_default()
}

#[test]
#[ignore = "a real-time load run of minutes; run it by hand, in release, as its rustdoc says"]
fn load_of_one_call() {
    let engines = Engine::listed();
    let total = setting("LOAD_PARTICIPANTS", 50);
    let count = total.saturating_sub(engines.len() as u64);
    let seconds = setting("LOAD_SECONDS", 120);
    let seed = setting("LOAD_SEED", 1);
    let capped_index = u16::try_from(count.saturating_sub(1).min(10)).expect("a small index");
    let sfu = start_sfu();
    let addresses = sfu.addresses;
    let pid = sfu.child.id();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()
        .expect("a runtime");
    let profile = Profile::half(Duration::from_millis(150), 0.02);
    let signal = format!("ws://{}", addresses.signal);
    let browsers: Vec<Browser> = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a page port");
        let page = format!("http://{}/", listener.local_addr().expect("a page address"));
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(|| async { axum::response::Html(PAGE) }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let mut browsers = Vec::new();
        for (i, engine) in engines.iter().enumerate() {
            let i = i as u64;
            let link = link(
                addresses.media,
                Impairment::new(profile, seed * 1000 + 900 + i * 2),
                Impairment::new(profile, seed * 1000 + 901 + i * 2),
            )
            .await
            .expect("a link");
            eprintln!("load: starting {engine:?}");
            let browser = Browser::start(*engine).await;
            eprintln!("load: {engine:?} started");
            let url = format!(
                "{page}?sfu={signal}/call&admission={}&relay={}",
                admission(1000 + i),
                link.front
            );
            browser.goto(&url).await;
            eprintln!("load: {engine:?} opened {url}");
            // The link runs on the runtime as long as the run, through its tasks.
            std::mem::forget(link);
            browsers.push(browser);
        }
        browsers
    });
    let epoch = Instant::now();
    let joined = epoch + Duration::from_millis(200 * count);
    let warm = joined + Duration::from_secs(10);
    let end = warm + Duration::from_secs(seconds);
    let turn = Duration::from_secs(20);
    let cap_at = warm + Duration::from_secs(25);
    let (synths, cpu, capped, seen) = runtime.block_on(async {
        let mut handles = Vec::new();
        let mut capped = None;
        for i in 0..count {
            let index = u16::try_from(i).expect("a few participants");
            let link = link(
                addresses.media,
                Impairment::new(profile, seed * 1000 + i * 2),
                Impairment::new(profile, seed * 1000 + i * 2 + 1),
            )
            .await
            .expect("a link");
            if index == capped_index {
                capped = Some(link.down.clone());
            }
            let plan = Plan {
                index,
                signal: signal.clone(),
                sfu: addresses.media,
                start: epoch + Duration::from_millis(200 * i),
                epoch,
                end,
                turn,
                speakers: 5,
                rtt: profile.delay * 2,
            };
            handles.push(tokio::spawn(participant(plan, link)));
        }
        tokio::time::sleep_until(warm.into()).await;
        let mut first = Vec::new();
        for b in &browsers {
            first.push(b.video_stats().await);
        }
        let cpu_start = (
            Instant::now(),
            cpu_ticks(pid),
            cpu_ticks(std::process::id()),
        );
        tokio::time::sleep_until(cap_at.into()).await;
        if let Some(down) = &capped
            && let Ok(mut d) = down.lock()
        {
            d.profile.cap = Some(800_000);
        }
        tokio::time::sleep_until(end.into()).await;
        let mut seen: Vec<Watched> = Vec::new();
        for (b, before) in browsers.iter().zip(&first) {
            let after = b.video_stats().await;
            println!("browser {:?}: {after}", b.engine);
            seen.extend(watched(b.engine, before, &after));
        }
        let cpu_end = (
            Instant::now(),
            cpu_ticks(pid),
            cpu_ticks(std::process::id()),
        );
        let mut synths = Vec::new();
        for h in handles {
            synths.push(h.await.expect("a participant"));
        }
        let wall = cpu_end.0 - cpu_start.0;
        let cpu = (cpu_end.1 - cpu_start.1) as f64 / 100.0 / wall.as_secs_f64();
        let clients = (cpu_end.2 - cpu_start.2) as f64 / 100.0 / wall.as_secs_f64();
        println!("participants and relay CPU: {clients:.3} vCPU, on the same machine");
        (synths, cpu, capped.is_some(), seen)
    });
    assert!(capped, "the capped participant exists");

    let mut latency: Vec<Duration> = Vec::new();
    let mut stalled = Duration::ZERO;
    let mut watched = Duration::ZERO;
    let mut worst = (0.0, 0u16);
    let mut tracks = 0;
    for s in &synths {
        if s.index == capped_index {
            continue;
        }
        latency.extend(s.audio_latency.iter().copied());
        for rx in s.video.values() {
            let (st, wa) = rx.stalls(warm);
            if wa > Duration::from_secs(10) {
                tracks += 1;
                stalled += st;
                watched += wa;
                let per10 = st.as_secs_f64() / wa.as_secs_f64() * 600.0;
                if per10 > worst.0 {
                    worst = (per10, s.index);
                }
            }
        }
    }
    latency.sort_unstable();
    let capped_synth = synths
        .iter()
        .find(|s| s.index == capped_index)
        .expect("capped");
    // A drop is any video of the capped receiver that moves to a lower layer, or stops.
    let arrivals: Vec<_> = capped_synth
        .video
        .values()
        .flat_map(|rx| rx.arrivals.iter())
        .collect();
    let mut before: Vec<(u16, u8)> = Vec::new();
    for a in arrivals
        .iter()
        .filter(|a| a.at < cap_at && a.at + Duration::from_secs(2) >= cap_at)
    {
        match before.iter_mut().find(|(s, _)| *s == a.sender) {
            Some((_, layer)) => *layer = (*layer).max(a.layer),
            None => before.push((a.sender, a.layer)),
        }
    }
    let drop = before
        .iter()
        .filter_map(|(sender, layer)| {
            let after: Vec<_> = arrivals
                .iter()
                .filter(|a| a.sender == *sender && a.at >= cap_at)
                .collect();
            let lower = after
                .iter()
                .filter(|a| a.layer < *layer)
                .map(|a| a.at)
                .min();
            let last = after.iter().map(|a| a.at).max();
            let stopped = match last {
                None => Some(cap_at),
                Some(t) if t + Duration::from_secs(5) < end => Some(t),
                Some(_) => None,
            };
            [lower, stopped].into_iter().flatten().min()
        })
        .min()
        .map(|t| t.saturating_duration_since(cap_at));
    println!("participants {count}, measured {seconds} s after a warm-up, seed {seed}");
    println!("SFU CPU: {cpu:.3} vCPU");
    println!(
        "mouth to ear (transit, speech packets): p50 {:?} p95 {:?} p99 {:?} over {}",
        percentile(&latency, 0.5),
        percentile(&latency, 0.95),
        percentile(&latency, 0.99),
        latency.len()
    );
    println!(
        "video stall: {:.3} s per 10 minutes of one video over {tracks} tracks ({:.0} s watched), worst track {:.3} s at receiver {}",
        stalled.as_secs_f64() / watched.as_secs_f64().max(1e-9) * 600.0,
        watched.as_secs_f64(),
        worst.0,
        worst.1
    );
    println!("capped receiver: videos before the cap {before:?}, first drop after {drop:?}");
    // A browser video counts once it decoded a second of pictures in the window.
    for engine in [Engine::Chrome, Engine::Firefox] {
        let videos: Vec<&Watched> = seen
            .iter()
            .filter(|w| w.engine == engine && w.decoded >= 30.0)
            .collect();
        if videos.is_empty() {
            continue;
        }
        let span = (end - warm).as_secs_f64();
        let frozen: Option<f64> = videos
            .iter()
            .map(|w| w.frozen.map(|d| d.as_secs_f64()))
            .sum();
        let freezes: Option<f64> = videos.iter().map(|w| w.freezes).sum();
        let decoded: f64 = videos.iter().map(|w| w.decoded).sum();
        let dropped: f64 = videos.iter().map(|w| w.dropped).sum();
        println!(
            "browser video stall, {engine:?}: {:?} s frozen per 10 minutes of one video over {} videos, {freezes:?} freezes, {decoded} pictures decoded, {dropped} dropped",
            frozen.map(|f| f / (span * videos.len() as f64) * 600.0),
            videos.len()
        );
    }
    std::mem::drop(browsers);
}
