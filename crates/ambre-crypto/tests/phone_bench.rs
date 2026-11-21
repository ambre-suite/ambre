//! The measures of the Sync worker that Murmure owes a phone, timed in the
//! browser that runs the page
//! (`5-Murmure/features/ambre-spec-murmure-phone-measures.md`).
//!
//! `just phone-measures crypto <address>` serves this file to a phone through
//! `wasm-bindgen-test-runner`. The phone runs the ignored test, then prints one
//! `PHONE-MEASURE` line of JSON. CI runs the smoke test alone, at a small size,
//! so the page cannot rot.
//!
//! On the host target the whole file is empty.
#![cfg(target_arch = "wasm32")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use ambre_core::{Aad, Confidential, EntryDigest, ExchangePublicKey, FixedBytes, Plaintext};
use ambre_crypto::ecdh::{KeyWrapper, unwrap_anchored};
use ambre_crypto::{DocumentKey, MasterSeed, envelope};
use serde_json::{Value, json};
use wasm_bindgen_test::{console_log, wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// The shortest sample the page times. A phone browser rounds the clock to
/// 100 us, and Safari to 1 ms. A sample of 20 ms then errs by 5 % at most.
const SAMPLE_MS: f64 = 20.0;

/// The most runs one sample groups, so a calibration always ends.
const SAMPLE_MAX_RUNS: usize = 1 << 16;

/// The public channels of the space leave row of `ambre-spec-budgets.md`.
const LEAVE_CHANNELS: usize = 50;

/// The members of the space leave row, the one who left included.
const LEAVE_MEMBERS: usize = 200;

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
    while timed(runs) < SAMPLE_MS && runs < SAMPLE_MAX_RUNS {
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

/// One figure of the page, in `unit`, against the row of the budget register
/// it fills. `scale` turns microseconds into the unit.
fn figure(row: &str, unit: &str, scale: f64, target: f64, spread: &Spread) -> Value {
    let median = spread.median * scale;
    let p95 = spread.p95 * scale;
    json!({
        "row": row,
        "unit": unit,
        "median": (median * 10.0).round() / 10.0,
        "p95": (p95 * 10.0).round() / 10.0,
        "target": target,
        "holds": p95 < target,
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

/// The exchange key of one member, derived from a seed of its own.
fn member(index: usize) -> (x25519_dalek::StaticSecret, FixedBytes<ExchangePublicKey>) {
    let mut seed = [0x5A; 32];
    seed[..8].copy_from_slice(&(index as u64).to_le_bytes());
    let secret = MasterSeed::from_array(seed).derive_exchange_key();
    let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
    (secret, FixedBytes::<ExchangePublicKey>::from_array(public))
}

/// Every figure of the Sync worker, with `channels` public channels and
/// `members` members for the space leave.
fn measure(channels: usize, members: usize, samples: usize) -> Value {
    let doc_key = DocumentKey::from_array([0x11; 32]);
    let signing = MasterSeed::from_array([0x22; 32]).derive_signing_key();
    let author =
        FixedBytes::<ambre_core::PublicKey>::from_array(signing.verifying_key().to_bytes());
    let aad = Aad::of(b"public-data-metadata");
    let payload = Confidential::<Plaintext>::new(vec![0x42u8; 1024]);

    let seal = spread(samples, 1, |runs| {
        let start = now();
        for _ in 0..runs {
            std::hint::black_box(envelope::seal(&payload, &aad, &doc_key, &signing).expect("seal"));
        }
        now() - start
    });
    let sealed = envelope::seal(&payload, &aad, &doc_key, &signing).expect("seal");
    let open = spread(samples, 1, |runs| {
        let start = now();
        for _ in 0..runs {
            let opened = envelope::open(&sealed, &aad, &doc_key, &author).expect("open");
            std::hint::black_box(opened);
        }
        now() - start
    });
    let opened = envelope::open(&sealed, &aad, &doc_key, &author).expect("open");
    assert_eq!(
        opened.secret().expose(),
        payload.secret().expose(),
        "the open returns another plaintext than the one sealed"
    );

    // The leave: the admin wraps the fresh space key and one fresh key per
    // public channel for each member who stays. The space key takes a
    // wrapper of its own, and every channel key shares one (`SYNC-264`), as
    // the Sync worker does. So each sample derives every pair key again.
    let (admin, admin_pub) = member(0);
    let staying: Vec<_> = (1..members - 1).map(member).collect();
    let keys: Vec<DocumentKey> = (0..=channels).map(|_| DocumentKey::generate()).collect();
    let anchor = FixedBytes::<EntryDigest>::from_array([0x33; 32]);
    let space_id = ambre_core::id::Id::<ambre_core::id::Document>::from_bytes([0x34; 16]);
    let wraps = keys.len() * (staying.len() + 1);
    let leave = spread(samples.min(5), wraps, |runs| {
        let start = now();
        for _ in 0..runs {
            let mut space = KeyWrapper::new(&admin);
            let mut channels = KeyWrapper::new(&admin);
            for (index, key) in keys.iter().enumerate() {
                let wrapper = if index == 0 {
                    &mut space
                } else {
                    &mut channels
                };
                std::hint::black_box(
                    wrapper
                        .wrap(&admin_pub, space_id, key, Some(&anchor))
                        .expect("wrap own"),
                );
                for (_, public) in &staying {
                    std::hint::black_box(
                        wrapper
                            .wrap(public, space_id, key, Some(&anchor))
                            .expect("wrap"),
                    );
                }
            }
        }
        now() - start
    });
    let (last_secret, last_public) = staying.last().expect("one member stays");
    // The member opens a wrap the shared wrapper made from a held pair key.
    let mut shared = KeyWrapper::new(&admin);
    shared
        .wrap(last_public, space_id, &keys[0], Some(&anchor))
        .expect("wrap");
    let wrapped = shared
        .wrap(last_public, space_id, &keys[channels], Some(&anchor))
        .expect("wrap");
    let (unwrapped, read_anchor) =
        unwrap_anchored(last_secret, &admin_pub, space_id, &wrapped).expect("unwrap");
    assert_eq!(
        unwrapped.id(),
        keys[channels].id(),
        "the wrap seals another key"
    );
    assert_eq!(read_anchor, Some(anchor), "the wrap loses its anchor");

    json!({
        "page": "crypto",
        "user_agent": web_sys::window().expect("a window").navigator().user_agent().expect("a user agent"),
        "clock_step_us": clock_step(),
        "leave_channels": channels,
        "leave_members": members,
        "leave_wraps": wraps,
        "figures": [
            figure("Outbound encryption 1 KB (compress+AEAD+HMAC+Ed25519)", "us", 1.0, 200.0, &seal),
            figure("Inbound decryption 1 KB (verify+HMAC+AEAD+decompress)", "us", 1.0, 350.0, &open),
            figure(
                "Murmure space leave, the rotation of the space key and of every public channel key",
                "ms",
                wraps as f64 / 1000.0,
                2000.0,
                &leave,
            ),
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

/// The run a phone makes: the space leave at the size of its row.
#[wasm_bindgen_test]
#[ignore = "a phone run, served by `just phone-measures crypto`"]
fn measure_on_the_phone() {
    let report = measure(LEAVE_CHANNELS, LEAVE_MEMBERS, 30);
    show(&format!("PHONE-MEASURE {report}"));
}

/// Each figure names its row and reads a finite time above zero.
#[wasm_bindgen_test]
fn every_figure_names_its_row_and_reads_a_time() {
    let report = measure(2, 4, 3);
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
