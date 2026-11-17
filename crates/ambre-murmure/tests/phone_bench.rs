//! The Murmure measures of the Logic thread, timed in the browser that runs
//! the page (`features/ambre-spec-murmure-phone-measures.md`).
//!
//! `just phone-measures murmure <address>` serves this file to a phone through
//! `wasm-bindgen-test-runner`. The phone runs the ignored test at the size of a
//! closed segment, then prints one `PHONE-MEASURE` line of JSON. CI runs the
//! smoke test alone, on a small segment, so the page cannot rot.
//!
//! On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

#[path = "../benches/common/segment.rs"]
mod corpus;

use loro::{ExportMode, JsonSchema, LoroDoc};
use serde_json::{Value, json};
use wasm_bindgen_test::{console_log, wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// The shortest sample the page times. A phone browser rounds the clock to
/// 100 us, and Safari to 1 ms. A sample of 20 ms then errs by 5 % at most.
const SAMPLE_MS: f64 = 20.0;

/// The most operations one sample groups, so a calibration always ends.
const SAMPLE_MAX_OPS: usize = 1 << 16;

/// The messages of a segment at its close (`ambre-spec-budgets.md section 6`).
const CLOSED_SEGMENT: usize = 5000;

/// The clock of the page, in milliseconds.
fn now() -> f64 {
    web_sys::window()
        .expect("a window")
        .performance()
        .expect("a performance clock")
        .now()
}

/// The median and the 95th percentile of one operation, in microseconds.
struct Spread {
    median: f64,
    p95: f64,
    ops_per_sample: usize,
    samples: usize,
}

/// The value of rank `share` in `sorted`, by the nearest rank.
fn rank(sorted: &[f64], share: f64) -> f64 {
    let at = ((sorted.len() as f64) * share).ceil() as usize;
    sorted[at.clamp(1, sorted.len()) - 1]
}

/// Times `timed(n)`, which runs `n` runs of `per_run` operations each and
/// returns the milliseconds it timed. The count doubles until one sample lasts
/// `SAMPLE_MS`, then `samples` samples follow. Each sample gives one figure per
/// operation.
fn spread(samples: usize, per_run: usize, mut timed: impl FnMut(usize) -> f64) -> Spread {
    let mut runs = 1;
    while timed(runs) < SAMPLE_MS && runs < SAMPLE_MAX_OPS {
        runs *= 2;
    }
    let ops = runs * per_run;
    let mut each: Vec<f64> = (0..samples)
        .map(|_| timed(runs) * 1000.0 / ops as f64)
        .collect();
    each.sort_by(f64::total_cmp);
    Spread {
        median: rank(&each, 0.5),
        p95: rank(&each, 0.95),
        ops_per_sample: ops,
        samples,
    }
}

/// One figure of the page, against the row of the budget register it fills.
fn figure(row: &str, target_us: f64, spread: &Spread) -> Value {
    json!({
        "row": row,
        "unit": "us",
        "median": (spread.median * 10.0).round() / 10.0,
        "p95": (spread.p95 * 10.0).round() / 10.0,
        "target": target_us,
        "holds": spread.p95 < target_us,
        "ops_per_sample": spread.ops_per_sample,
        "samples": spread.samples,
    })
}

/// The smallest step of the clock the page reads, in microseconds.
fn clock_step() -> f64 {
    let start = now();
    let mut next = now();
    while next == start {
        next = now();
    }
    ((next - start) * 1000.0 * 10.0).round() / 10.0
}

/// Every figure of the Logic thread on a segment of `count` messages.
fn measure(count: usize, samples: usize) -> Value {
    let built = now();
    let segment = corpus::build(count);
    let build_ms = now() - built;
    let deltas = segment.deltas.len();

    // A cold open with no snapshot: each delta imported alone, as the replay
    // pages deliver them. One sample is one whole replay.
    let replay = spread(samples, deltas, |runs| {
        let start = now();
        for _ in 0..runs {
            let doc = LoroDoc::new();
            for delta in &segment.deltas {
                doc.import(delta).expect("import");
            }
            std::hint::black_box(doc);
        }
        now() - start
    });

    let snapshot = segment.doc.export(ExportMode::Snapshot).expect("snapshot");
    let import = spread(samples, 1, |runs| {
        let start = now();
        for _ in 0..runs {
            let doc = LoroDoc::new();
            doc.import(&snapshot).expect("import snapshot");
            std::hint::black_box(doc);
        }
        now() - start
    });

    // The authorship check: the last delta read as a Loro JSON update, with
    // no document. The criterion bench `decode_and_scan` times the same work.
    let (_, head) = segment.deltas.split_last().expect("one delta at least");
    let before_last = LoroDoc::new();
    before_last.import_batch(head).expect("import head");
    let before = before_last.oplog_vv();
    let json = serde_json::to_vec(
        &segment
            .doc
            .export_json_updates(&before, &segment.doc.oplog_vv()),
    )
    .expect("json");
    let check = spread(samples, 1, |runs| {
        let start = now();
        for _ in 0..runs {
            let schema: JsonSchema = serde_json::from_slice(&json).expect("decode");
            let ops: usize = schema.changes.iter().map(|change| change.ops.len()).sum();
            assert!(ops > 0, "the last delta reads as no operation");
            std::hint::black_box(schema);
        }
        now() - start
    });
    json!({
        "page": "murmure",
        "user_agent": web_sys::window().expect("a window").navigator().user_agent().expect("a user agent"),
        "clock_step_us": clock_step(),
        "messages": count,
        "deltas": deltas,
        "snapshot_bytes": snapshot.len(),
        "json_update_bytes": json.len(),
        "build_ms": build_ms.round(),
        "figures": [
            figure("Murmure segment, Loro replay of one delta past its snapshot", 250.0, &replay),
            figure("Murmure segment, Loro import of its snapshot, 5000 messages", 10_000.0, &import),
            figure("Murmure authorship check, one received delta read as a Loro JSON update", 50.0, &check),
        ],
    })
}

/// Writes the line on the page, where the phone selects it, and on the
/// console, where the runner reads it. The interactive runner shows no console.
fn show(line: &str) {
    console_log!("{line}");
    let document = web_sys::window()
        .expect("a window")
        .document()
        .expect("a document");
    let area = document.create_element("textarea").expect("a text area");
    area.set_attribute("readonly", "").expect("read only");
    area.set_attribute("style", "width: 100%; height: 40vh; font: 12px monospace")
        .expect("a style");
    area.set_text_content(Some(line));
    document
        .body()
        .expect("a body")
        .append_child(&area)
        .expect("the text area shows");
}

/// The run a phone makes: a segment at its close, 30 samples a figure.
#[wasm_bindgen_test]
#[ignore = "a phone run, served by `just phone-measures murmure`"]
fn measure_on_the_phone() {
    let report = measure(CLOSED_SEGMENT, 30);
    show(&format!("PHONE-MEASURE {report}"));
}

/// Each figure names its row and reads a finite time above zero.
#[wasm_bindgen_test]
fn every_figure_names_its_row_and_reads_a_time() {
    let report = measure(50, 3);
    let figures = report["figures"].as_array().expect("figures");
    assert_eq!(figures.len(), 3);
    for figure in figures {
        assert!(figure["row"].as_str().is_some_and(|row| !row.is_empty()));
        let median = figure["median"].as_f64().expect("a median");
        let p95 = figure["p95"].as_f64().expect("a 95th percentile");
        assert!(median.is_finite() && median >= 0.0, "{figure}");
        assert!(p95 >= median, "{figure}");
    }
    show(&format!("PHONE-MEASURE {report}"));
}

/// A segment that lost a delta stops the page before any figure: a replay of
/// fewer deltas would read as a faster phone.
#[wasm_bindgen_test]
#[should_panic(expected = "the deltas")]
fn a_segment_missing_a_delta_stops_the_page() {
    let mut segment = corpus::build(20);
    segment.deltas.pop();
    corpus::assert_replays(&segment, 20);
}
