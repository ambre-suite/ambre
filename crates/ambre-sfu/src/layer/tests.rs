//! The layer choice alone, over made-up layer bitrates.

use std::time::{Duration, Instant};

use ambre_core::Count;
use str0m::bwe::Bitrate;
use str0m::media::Rid;

use super::{Candidate, Layer, Meter, Offered, VIDEOS_PER_RECEIVER, choose};

fn layer(rid: &str) -> Layer {
    Layer(Some(Rid::from(rid)))
}

/// A track of three layers at 150, 500 and 1500 kbit/s, which the receiver gets at `current`.
fn simulcast(current: Option<&str>) -> Candidate {
    Candidate {
        layers: [("l", 150), ("m", 500), ("h", 1500)]
            .iter()
            .map(|(rid, kbps)| Offered {
                layer: layer(rid),
                rate: Bitrate::kbps(*kbps),
            })
            .collect(),
        current: current.map(layer),
    }
}

#[test]
fn the_first_speaker_gets_the_top_layer_the_estimate_holds_and_the_others_the_lowest() {
    let candidates = vec![simulcast(None), simulcast(None), simulcast(None)];
    let chosen = choose(Bitrate::kbps(3000), &candidates);
    assert_eq!(
        chosen,
        vec![Some(layer("h")), Some(layer("l")), Some(layer("l"))]
    );
}

#[test]
fn a_lower_estimate_drops_the_first_speaker_to_a_lower_layer() {
    let candidates = vec![simulcast(Some("h")), simulcast(Some("l"))];
    let chosen = choose(Bitrate::kbps(1000), &candidates);
    assert_eq!(chosen, vec![Some(layer("m")), Some(layer("l"))]);
}

#[test]
fn an_estimate_below_every_lowest_layer_forwards_no_video() {
    let candidates = vec![simulcast(Some("l"))];
    assert_eq!(choose(Bitrate::kbps(100), &candidates), vec![None]);
}

#[test]
fn a_receiver_gets_no_more_videos_than_the_cap() {
    let cap = usize::try_from(VIDEOS_PER_RECEIVER.get()).expect("a cap that fits");
    let candidates = vec![simulcast(None); cap + 3];
    let chosen = choose(Bitrate::mbps(100), &candidates);
    assert_eq!(chosen.iter().filter(|c| c.is_some()).count(), cap);
    assert!(chosen.iter().skip(cap).all(Option::is_none));
}

#[test]
fn a_move_up_needs_a_margin_that_staying_does_not() {
    // The top layer and the other one take 1650 kbit/s of the 1700 that 2000 * 0.85 leaves.
    // That holds a layer the receiver gets, and misses the margin of a move up.
    let staying = vec![simulcast(Some("h")), simulcast(Some("l"))];
    let moving = vec![simulcast(Some("m")), simulcast(Some("l"))];
    assert_eq!(
        choose(Bitrate::kbps(2000), &staying).first(),
        Some(&Some(layer("h")))
    );
    assert_eq!(
        choose(Bitrate::kbps(2000), &moving).first(),
        Some(&Some(layer("m")))
    );
}

#[test]
fn the_meter_reads_the_rate_of_its_packets() {
    let start = Instant::now();
    let mut meter = Meter::new(start);
    // 100 packets of 1250 bytes over one second: 1 Mbit/s.
    for n in 0..=100 {
        meter.count(start + Duration::from_millis(n * 10), Count::new(1250));
    }
    let kbps = meter.rate().as_f64() / 1000.0;
    assert!((900.0..1100.0).contains(&kbps), "measured {kbps} kbit/s");
}
