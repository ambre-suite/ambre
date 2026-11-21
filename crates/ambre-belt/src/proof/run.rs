//! The run of one call proof: the services, the senders, the receiver, the phases and the
//! result. `ambre-spec-call-proofs.md` states each step and what a person does on a phone.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    Bound, Figure, Host, Plan, Receiver, Sizes, Verdict, Window, chunk_figures, date, echo_figure,
    paint_figures, seal_figures, table, threshold, video_figures,
};
use crate::murmure::tests as murmure;
use crate::page::Page;
use crate::provider::{Provider, sign_in};
use crate::relay::Relay;

/// The space and the channel the senders and the receiver share.
const SPACE: &str = "Proof";
const CHANNEL: &str = "proof";

/// Prints one line of the run at once, for the person beside the phone.
fn say(line: &str) {
    println!("proof: {line}");
}

/// The videos of `now` that decoded more pictures than in `before`.
fn growing(before: &Value, now: &Value) -> usize {
    let videos = |r: &Value| r["videosIn"].as_array().cloned().unwrap_or_default();
    videos(now)
        .iter()
        .filter(|v| {
            let id = v["id"].as_str();
            let was = videos(before)
                .iter()
                .find(|b| b["id"].as_str() == id)
                .and_then(|b| b["decoded"].as_f64())
                .unwrap_or(0.0);
            v["decoded"].as_f64().unwrap_or(0.0) > was
        })
        .count()
}

/// What the latest reports say of the receiver, in one line.
fn status(reports: &[Value]) -> String {
    let Some(now) = reports.last() else {
        return "no report from the page yet: is it open on the receiver?".to_owned();
    };
    let before = reports
        .len()
        .checked_sub(3)
        .and_then(|i| reports.get(i))
        .unwrap_or(now);
    let encoding = now["videosOut"].as_array().map_or(0, |v| {
        v.iter().filter(|e| e["encoded"].as_u64() > Some(0)).count()
    });
    format!(
        "home page: {}, page {}, peer connection {}, videos decoding {}, encodings sent {}, \
         call worker timed: {}",
        now["home"].as_bool().unwrap_or(false),
        now["visible"].as_str().unwrap_or("?"),
        now["state"].as_str().unwrap_or("none"),
        growing(before, now),
        encoding,
        now["worker"].is_object()
    )
}

/// Waits until `done` holds of the reports, printing their state every 10 s.
async fn until(host: &Host, what: &str, within: Duration, done: impl Fn(&[Value]) -> bool) {
    until_seen(host, what, within, &[], done).await;
}

/// Waits as [`until`] does. A timeout also prints the console of each page of `pages`, so a
/// page that never connected says why.
async fn until_seen(
    host: &Host,
    what: &str,
    within: Duration,
    pages: &[&Page],
    done: impl Fn(&[Value]) -> bool,
) {
    let deadline = Instant::now() + within;
    let mut spoke = Instant::now();
    loop {
        let reports = host.reports();
        if done(&reports) {
            return;
        }
        if Instant::now() >= deadline {
            let mut consoles = Vec::new();
            for (i, page) in pages.iter().enumerate() {
                let lines = page.browser.console().await;
                consoles.push(format!("page {i} said:\n{}", lines.join("\n")));
            }
            panic!(
                "within {within:?}, no {what}: {}\n{}",
                status(&reports),
                consoles.join("\n")
            );
        }
        if spoke.elapsed() >= Duration::from_secs(10) {
            say(&format!("waiting for {what}: {}", status(&reports)));
            spoke = Instant::now();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Runs one phase of `length`, the probe sealing or not, and answers the indices of its first
/// and its last report.
async fn phase(host: &Host, name: &str, length: Duration, seal: bool) -> (usize, usize) {
    host.sealing(seal);
    // The first report after the order opens the phase: the seals have started by then.
    let asked = host.len();
    until(host, "the next report", Duration::from_secs(30), |r| {
        r.len() > asked
    })
    .await;
    let start = host.len() - 1;
    say(&format!("{name}: {} s from now", length.as_secs()));
    let end_at = Instant::now() + length;
    while Instant::now() < end_at {
        let left = end_at.saturating_duration_since(Instant::now());
        tokio::time::sleep(left.min(Duration::from_secs(60))).await;
        say(&format!(
            "{name}: {} s left. {}",
            end_at.saturating_duration_since(Instant::now()).as_secs(),
            status(&host.reports())
        ));
    }
    let asked = host.len();
    until(host, "the closing report", Duration::from_secs(30), |r| {
        r.len() > asked
    })
    .await;
    (start, host.len() - 1)
}

/// The figures of `figures` with no verdict: they are printed for the reader of a phase that
/// holds no threshold of theirs.
fn read_only(figures: Vec<Figure>) -> Vec<Figure> {
    figures
        .into_iter()
        .map(|f| Figure {
            verdict: Verdict::Read,
            threshold: "--".to_owned(),
            ..f
        })
        .collect()
}

/// A sender of the run: a headless page that signs `name` in through
/// `provider`, then opens Murmure. The open reloads the page, and the session
/// the Sync worker kept survives it (`SRV-323`).
async fn sender(relay: Arc<Relay>, provider: &Provider, name: &str) -> Page {
    let page = Page::home(relay).await;
    sign_in(&page, provider, name).await;
    murmure::open_murmure(&page).await;
    page
}

/// A headed Chrome on this machine, with its GPU: the receiver of a desktop proof.
async fn desktop_receiver(relay: Arc<Relay>, origin: &str) -> Page {
    let downloads = std::env::temp_dir().join(format!("proof-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&downloads).expect("a download directory");
    let browser = crate::driver::Browser::start_with(&downloads, |caps| {
        let args = &mut caps["capabilities"]["alwaysMatch"]["goog:chromeOptions"]["args"];
        if let Some(list) = args.as_array_mut() {
            list.retain(|a| {
                !matches!(
                    a.as_str(),
                    Some(
                        "--headless=new"
                            | "--enable-unsafe-swiftshader"
                            | "--use-angle=swiftshader"
                    )
                )
            });
            list.push(json!("--window-size=1280,800"));
        }
    })
    .await;
    browser.goto(&format!("{origin}/")).await;
    Page::over(browser, origin.to_owned(), downloads, relay)
}

/// One line per sender: the widths it sends and what limits its encoder. A sender that lacks
/// the top size starves the receiver, and the figures then understate its load.
async fn senders_line(senders: &[Page]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, page) in senders.iter().enumerate() {
        // The live peer connection of the page: an earlier one may have closed.
        let stats = page
            .browser
            .script_async(
                "const done = arguments[arguments.length - 1]; \
                 const pc = (window.beltPcs || []).filter((p) => p.connectionState !== 'closed').pop(); \
                 if (!pc) { done({ sent: [] }); return; } \
                 pc.getStats().then((stats) => { const sent = []; stats.forEach((s) => { \
                   if (s.type === 'outbound-rtp' && s.kind === 'video') sent.push({ \
                     width: s.frameWidth || 0, limit: s.qualityLimitationReason || null }); }); \
                   done({ sent }); }, (e) => done({ error: String(e) }));",
                json!([]),
            )
            .await;
        let sent: Vec<String> = stats["sent"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                format!(
                    "{}x{}",
                    s["width"].as_u64().unwrap_or(0),
                    s["limit"].as_str().unwrap_or("none")
                )
            })
            .collect();
        lines.push(if sent.is_empty() {
            format!("sender {i}: no encoding read: {stats}")
        } else {
            let starved = !sent.iter().any(|s| s.starts_with("1280x"))
                || sent.iter().any(|s| s.ends_with("xcpu"));
            format!(
                "sender {i}: sends {} (width x limit){}",
                sent.join(" "),
                if starved {
                    ": short of CPU, the receiver gets less than the proof needs"
                } else {
                    ""
                }
            )
        });
    }
    lines
}

/// What one run measured: the reports of the probe, the span of each phase, and the pages
/// that stay open until the result is read.
struct Outcome {
    reports: Vec<Value>,
    paint: (usize, usize),
    worker: (usize, usize),
    decode: Option<(usize, usize)>,
    senders: Vec<Page>,
    /// Held so the receiver stays in the call until the result is printed.
    _receiver: Option<Page>,
    /// Held so the call runs until the result is printed: its drop ends the SFU.
    _sfu: crate::call::Sfu,
}

impl Outcome {
    fn window(&self, (a, b): (usize, usize)) -> Window<'_> {
        Window {
            reports: self.reports.get(a..=b).unwrap_or_default(),
        }
    }
}

/// Runs the services, the senders and the receiver of `plan`, then its phases.
async fn run(plan: &Plan) -> Outcome {
    let sfu = crate::call::Sfu::start_on(None, plan.host).await;
    let bound = Bound::bind(plan).await;
    let origin = bound.origin.clone();
    // Each participant signs in through the provider of the belt, as a
    // release build does: the build holds no development sign-in (`ZK-050`).
    let provider = Provider::spawn().await;
    let mut env: Vec<(&str, String)> = sfu.relay_env_at(&bound.sfu_url()).to_vec();
    env.extend(provider.relay_env());
    let relay = Arc::new(Relay::start_with(&env).await);
    let host = Arc::new(Host::default());
    let sfu_base = sfu
        .signal
        .replacen("ws://", "http://", 1)
        .replace("/call", "");
    bound.serve(
        plan,
        crate::serve::dist_dir(),
        relay.base.clone(),
        sfu_base,
        Arc::clone(&host),
    );
    let short = &uuid::Uuid::new_v4().simple().to_string()[..4];
    let receiver_name = format!("phone-{short}");
    say(&format!(
        "host {origin}, SFU media on {}:{}, {} senders",
        plan.host, sfu.media_port, plan.senders
    ));

    // The receiver signs in first, so the senders can invite it.
    let receiver = match plan.receiver {
        Receiver::Phone => {
            host.sign_in_phone(provider.clone(), receiver_name.clone());
            say(&format!(
                "on the phone, open {origin}/proof/sign-in, then set a passphrase of your \
                 choice and confirm the recovery words: it signs in as `{receiver_name}`"
            ));
            None
        }
        Receiver::Desktop => Some(desktop_receiver(Arc::clone(&relay), &origin).await),
        Receiver::Headless => {
            let downloads = std::env::temp_dir().join(format!("proof-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&downloads).expect("a download directory");
            let browser = crate::driver::Browser::start(&downloads).await;
            browser.goto(&format!("{origin}/")).await;
            Some(Page::over(
                browser,
                origin.clone(),
                downloads,
                Arc::clone(&relay),
            ))
        }
    };
    if let Some(page) = &receiver {
        sign_in(page, &provider, &receiver_name).await;
    }
    until(&host, "a signed-in page", Duration::from_secs(900), |r| {
        r.iter().any(|r| r["home"].as_bool() == Some(true))
    })
    .await;
    say("the receiver is signed in: the senders start");

    let names: Vec<String> = (0..plan.senders)
        .map(|i| format!("sender-{short}-{i}"))
        .collect();
    let senders: Vec<Page> = futures_util::future::join_all(
        names
            .iter()
            .map(|name| sender(Arc::clone(&relay), &provider, name)),
    )
    .await;
    let first = senders.first().expect("one sender at least");
    let others = names.get(1..).unwrap_or_default();
    match (others.first(), senders.get(1)) {
        (Some(second), Some(second_page)) => {
            murmure::space_with_channel(first, SPACE, CHANNEL, second, second_page).await
        }
        _ => {
            murmure::prompt(first, ".murmure-spaces", "murmure-new-space", SPACE).await;
            murmure::prompt(first, ".murmure-space", "murmure-new-channel", CHANNEL).await;
            murmure::open(first, CHANNEL).await;
        }
    }
    for (name, page) in others.iter().zip(senders.iter().skip(1)).skip(1) {
        let link = murmure::invite_into_space(first, name).await;
        murmure::follow(page, &link).await;
    }
    // The receiver opens Murmure from its own invite link (`SYNC-236`).
    let receiver_link = murmure::invite_into_space(first, &receiver_name).await;
    for page in &senders {
        murmure::open(page, CHANNEL).await;
        page.browser.script(murmure::RECORD, json!([])).await;
    }
    let channel = murmure::last_channel();
    let call = first
        .browser
        .script(
            "return window.wasmBindings.start_video_call(arguments[0]);",
            json!([channel]),
        )
        .await;
    let call = call.as_str().unwrap_or_default().to_owned();
    assert!(!call.is_empty(), "the first sender started no call");
    if let Some(second) = senders.get(1) {
        murmure::read_until(
            second,
            "a live call",
            |h| h.contains(murmure::LIVE),
            murmure::ROSTER_WAIT,
        )
        .await;
    }
    for page in senders.iter().skip(1) {
        page.browser
            .script(
                "window.wasmBindings.join_video_call(arguments[0], arguments[1]); return true;",
                json!([call, channel]),
            )
            .await;
    }
    // One sender speaks, so the top size stays on one video.
    for page in senders.iter().skip(1) {
        murmure::set_mute(page, "true").await;
    }

    match &receiver {
        None => say(&format!(
            "on the phone: open {origin}{receiver_link}, open the channel `{CHANNEL}` of the \
             space `{SPACE}`, press Join on its call message, then the camera button. Keep \
             the screen on."
        )),
        Some(page) => {
            murmure::follow(page, &receiver_link).await;
            murmure::open(page, CHANNEL).await;
            page.browser
                .script(
                    "window.wasmBindings.join_video_call(arguments[0], arguments[1]); \
                     return true;",
                    json!([call, channel]),
                )
                .await;
        }
    }
    let videos = plan.videos();
    let watched: Vec<&Page> = receiver.iter().chain(senders.iter()).collect();
    until_seen(
        &host,
        &format!("{videos} videos decoding and the camera sent"),
        Duration::from_secs(900),
        &watched,
        |r| {
            let Some(now) = r.last() else { return false };
            let before = r.len().checked_sub(3).and_then(|i| r.get(i)).unwrap_or(now);
            growing(before, now) >= videos
                && now["worker"].is_object()
                && now["videosOut"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|e| e["encoded"].as_u64() > Some(0)))
        },
    )
    .await;
    say("the call runs: 10 s to settle, then the phases");
    tokio::time::sleep(Duration::from_secs(10)).await;

    let paint = phase(&host, "paint phase", plan.window, false).await;
    let worker = phase(
        &host,
        "worker phase, the Sync worker sealing",
        plan.window,
        true,
    )
    .await;
    let decode = if plan.decode.is_zero() {
        None
    } else {
        say("decode phase: read the battery level of the phone now, unplugged");
        let span = phase(&host, "decode phase", plan.decode, false).await;
        say("decode phase over: read the battery level of the phone now");
        Some(span)
    };
    host.sealing(false);
    Outcome {
        reports: host.reports(),
        paint,
        worker,
        decode,
        senders,
        _receiver: receiver,
        _sfu: sfu,
    }
}

/// Prints the figures of `outcome` against their thresholds, then the `PROOF-RESULT` line.
async fn print(plan: &Plan, outcome: &Outcome) {
    let mut sections: Vec<(&str, Vec<Figure>)> = Vec::new();
    let paint = outcome.window(outcome.paint);
    let mut painted = paint_figures(&paint);
    painted.extend(video_figures(&paint, threshold::CANVAS_FPS, Sizes::Printed));
    painted.extend(read_only(chunk_figures(&paint)));
    sections.push(("paint phase: nine videos on the canvas", painted));
    let worker = outcome.window(outcome.worker);
    let mut held = chunk_figures(&worker);
    held.extend(seal_figures(&worker));
    sections.push((
        "worker phase: the call worker while the Sync worker seals",
        held,
    ));
    if let Some(decode) = outcome.decode {
        sections.push((
            "decode phase: one 720p and eight 180p",
            video_figures(
                &outcome.window(decode),
                threshold::DECODE_FPS,
                Sizes::OneTopEightLow,
            ),
        ));
    }
    let last = outcome.reports.last().cloned().unwrap_or(Value::Null);
    sections.push(("microphone", vec![echo_figure(&last)]));

    let today = date(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    );
    let agent = last["agent"].as_str().unwrap_or("?").to_owned();
    let renderer = last["renderer"].as_str().unwrap_or("?").to_owned();
    println!();
    println!("call proof, {today}, receiver {:?}: {agent}", plan.receiver);
    println!("  renderer: {renderer}");
    for line in senders_line(&outcome.senders).await {
        println!("  {line}");
    }
    let mut all = Vec::new();
    for (title, figures) in &sections {
        println!("{title}");
        println!("{}", table(figures));
        all.extend(figures.iter().cloned());
    }
    let failed = all.iter().any(|f| f.verdict == Verdict::Fail);
    println!("verdict: {}", if failed { "FAIL" } else { "pass" });
    let figures: Vec<Value> = all
        .iter()
        .map(|f| {
            json!({ "name": f.name, "value": f.value, "threshold": f.threshold,
                    "verdict": format!("{:?}", f.verdict) })
        })
        .collect();
    println!(
        "PROOF-RESULT {}",
        json!({ "date": today, "receiver": format!("{:?}", plan.receiver), "agent": agent,
                "renderer": renderer,
                "senders": plan.senders, "window_s": plan.window.as_secs(),
                "decode_s": plan.decode.as_secs(), "figures": figures })
    );
}

/// `ambre-spec-call-proofs.md`: one call proof on one receiver, by hand.
///
/// The variables of [`Plan::read`] set the run. The recipes of the `justfile` set them for
/// each proof: `call-proof-android`, `call-proof-ios` and `call-proof-desktop`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a proof on real hardware, by hand: just call-proof-android, call-proof-ios or call-proof-desktop"]
async fn a_call_proof_on_one_receiver() {
    let plan = Plan::read(|name| std::env::var(name).ok()).unwrap_or_else(|e| panic!("{e}"));
    let outcome = run(&plan).await;
    print(&plan, &outcome).await;
    for page in &outcome.senders {
        page.browser
            .script("window.wasmBindings.leave_call(); return true;", json!([]))
            .await;
    }
}

/// `PROOF-002` to `PROOF-005`, and `ZK-050`, in the browser.
///
/// The proof host serves the build to a headless receiver, which two senders call, over two
/// phases of 10 s. Each participant signs in through the provider of the belt. The probe reports the paints and the counters of both videos, and the timer
/// reports the chunks of the call worker. The receiver decodes both videos through the timer.
/// The Sync worker seals files in the worker phase alone. No figure is held against its budget:
/// a container of the belt measures no device.
#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(
    not(feature = "browser"),
    ignore = "needs a build, the SFU, a driver and the services; run just test-browser"
)]
async fn the_proof_host_reads_a_call_through_its_probe() {
    let plan = Plan {
        receiver: Receiver::Headless,
        host: std::net::Ipv4Addr::LOCALHOST.into(),
        port: 0,
        tls: None,
        senders: 2,
        window: Duration::from_secs(10),
        decode: Duration::ZERO,
    };
    // The paths of the earlier tests of the process stay out of the count.
    let before = crate::serve::asked().len();
    let outcome = run(&plan).await;
    // `ZK-050`: each sender signed in through the provider of the belt, and
    // none through the development sign-in.
    let asked = crate::serve::asked().split_off(before);
    let returns = asked
        .iter()
        .filter(|path| *path == "/api/auth/google/callback")
        .count();
    assert!(
        returns >= plan.senders,
        "{returns} provider returns: {asked:?}"
    );
    assert!(
        !asked.iter().any(|path| path == "/api/auth/dev/login"),
        "a participant signed in by a name: {asked:?}"
    );
    let paint = outcome.window(outcome.paint);
    let worker = outcome.window(outcome.worker);
    assert!(!paint.paints().is_empty(), "no paint reported");
    let videos = video_figures(&paint, 0.0, Sizes::Printed);
    assert_eq!(videos[0].value, "2", "the receiver decodes {videos:?}");
    let chunks = chunk_figures(&worker);
    assert_eq!(chunks[0].name, "chunks per second", "{chunks:?}");
    let (done, failed, _) = worker.seals();
    assert!(done >= 1 && failed == 0, "{done} sealed, {failed} failed");
    assert_eq!(paint.seals().0, 0, "a seal with no order of the host");
    for page in &outcome.senders {
        page.browser
            .script("window.wasmBindings.leave_call(); return true;", json!([]))
            .await;
    }
}
