//! The layer choice of the SFU, between synthetic participants in simulated time.
//!
//! Each run is seeded, so its losses and its figures are the same on every machine.

use std::time::{Duration, Instant};

use super::client::{Arrival, VideoRx};
use super::impair::Profile;
use super::net::Net;

/// The proof setting of the roadmap: 150 ms of round trip and 2 % loss.
fn proof_link() -> Profile {
    Profile::half(Duration::from_millis(150), 0.02)
}

/// The track `receiver` gets from `sender`.
fn track(net: &Net, receiver: usize, sender: u16) -> &VideoRx {
    net.members
        .get(receiver)
        .expect("a receiver")
        .synth
        .video
        .values()
        .find(|rx| rx.arrivals.first().is_some_and(|a| a.sender == sender))
        .expect("a track from the sender")
}

/// The packets of `rx` that arrived at `from` or later.
fn since(rx: &VideoRx, from: Instant) -> impl Iterator<Item = &Arrival> {
    rx.arrivals.iter().filter(move |a| a.at >= from)
}

/// Joins `count` participants to one room over `profile`, the first one speaking.
fn call(count: u64, profile: Profile) -> Net {
    let mut net = Net::new();
    for seed in 0..count {
        net.join("r", profile, seed + 1);
        net.run(Duration::from_millis(200));
    }
    net.member(0).synth.speaking = true;
    net
}

/// The layer drop, at 150 ms of round trip and 2 % loss. The downlink of a receiver falls
/// below the top layer of the speaker, and the receiver gets a lower layer within 3 s.
#[test]
#[ignore = "moves with the load of the machine through the wall clock str0m reads"]
fn a_capped_receiver_drops_a_layer_within_the_budget() {
    let mut net = call(3, proof_link());
    net.run(Duration::from_secs(15));
    let before = net.now - Duration::from_secs(1);
    assert!(
        since(track(&net, 1, 0), before).all(|a| a.layer == 2),
        "before the cap, the receiver gets the top layer of the speaker"
    );
    let capped = net.now;
    net.member(1).down.profile.cap = Some(800_000);
    net.run(Duration::from_secs(5));
    let dropped = since(track(&net, 1, 0), capped)
        .find(|a| a.layer < 2)
        .map(|a| a.at - capped);
    assert!(
        dropped.is_some_and(|d| d <= Duration::from_secs(3)),
        "the capped receiver dropped a layer after {dropped:?}"
    );
}

/// The drop for the capped receiver leaves the layer of another receiver.
#[test]
#[ignore = "moves with the load of the machine through the wall clock str0m reads"]
fn a_layer_drop_for_one_receiver_leaves_another_receiver_on_its_layer() {
    let mut net = call(3, proof_link());
    net.run(Duration::from_secs(15));
    let capped = net.now;
    net.member(1).down.profile.cap = Some(800_000);
    net.run(Duration::from_secs(5));
    assert!(
        since(track(&net, 1, 0), capped).any(|a| a.layer < 2),
        "the capped receiver dropped no layer"
    );
    assert!(
        since(track(&net, 2, 0), capped).all(|a| a.layer == 2),
        "the other receiver lost the top layer of the speaker"
    );
}

/// Runs a call of three where the speaker moves from participant 0 to participant 1.
/// Participant 2 then sees the track of each sender switch layer. The links lose nothing, and
/// their 150 ms of round trip let several pictures pass before a keyframe comes.
fn speaker_moves() -> Net {
    let mut net = call(3, Profile::half(Duration::from_millis(150), 0.0));
    net.run(Duration::from_secs(8));
    net.member(0).synth.speaking = false;
    net.member(1).synth.speaking = true;
    net.run(Duration::from_secs(5));
    net
}

/// The index of each packet of `rx` whose layer differs from the packet before it.
fn switches(rx: &VideoRx) -> Vec<&Arrival> {
    rx.arrivals
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a.layer != b.layer => Some(b),
            _ => None,
        })
        .collect()
}

/// `CALL-039`: each layer switch the receiver sees starts on the first packet of a keyframe.
#[test]
fn a_receiver_switches_layer_only_on_a_keyframe_of_the_new_layer() {
    let net = speaker_moves();
    let mut seen = 0;
    for sender in [0, 1] {
        for packet in switches(track(&net, 2, sender)) {
            seen += 1;
            assert!(
                packet.keyframe_start && packet.index == 0,
                "a switch started on {packet:?}"
            );
        }
    }
    assert!(seen >= 2, "the moving speaker made {seen} switches");
}

/// `CALL-040`: the sequence numbers of a track stay continuous across its layer switches.
#[test]
fn the_sequence_numbers_stay_continuous_across_a_layer_switch() {
    let net = speaker_moves();
    for sender in [0, 1] {
        let rx = track(&net, 2, sender);
        assert!(
            !switches(rx).is_empty(),
            "the track of {sender} never switched"
        );
        for pair in rx.arrivals.windows(2) {
            if let [a, b] = pair {
                assert_eq!(b.seq, a.seq.wrapping_add(1), "a gap from {a:?} to {b:?}");
            }
        }
    }
}

/// `CALL-041`: of twelve senders, a receiver gets the video of nine, and the one who spoke
/// last is among them even though it joined last.
#[test]
fn a_receiver_gets_the_video_of_the_most_recent_speakers_alone() {
    let mut net = call(12, Profile::CLEAN);
    net.run(Duration::from_secs(4));
    net.member(0).synth.speaking = false;
    net.member(11).synth.speaking = true;
    net.run(Duration::from_secs(4));
    let from = net.now - Duration::from_secs(1);
    let mut senders: Vec<u16> = net
        .members
        .get(1)
        .expect("a receiver")
        .synth
        .video
        .values()
        .flat_map(|rx| since(rx, from).map(|a| a.sender))
        .collect();
    senders.sort_unstable();
    senders.dedup();
    assert_eq!(
        senders.len(),
        9,
        "the receiver got the video of {senders:?}"
    );
    assert!(
        senders.contains(&11),
        "the last speaker is missing: {senders:?}"
    );
    assert!(
        senders.contains(&0),
        "the speaker before it is missing: {senders:?}"
    );
}

#[test]
fn explore_stalls() {
    let mut net = call(4, proof_link());
    net.run(Duration::from_secs(5));
    let from = net.now;
    net.run(Duration::from_secs(30));
    for m in &net.members {
        for rx in m.synth.video.values() {
            let (st, wa) = rx.stalls(from);
            let gaps: Vec<u128> = rx
                .rendered
                .windows(2)
                .filter_map(|w| match w {
                    [a, b] if b.0 >= from && (b.0 - a.0) > Duration::from_millis(150) => {
                        Some((b.0 - a.0).as_millis())
                    }
                    _ => None,
                })
                .collect();
            eprintln!(
                "member {} from {:?}: stalled {st:?} of {wa:?}, rendered {} gaps {:?} kf {}",
                m.synth.index,
                rx.arrivals.first().map(|a| a.sender),
                rx.rendered.len(),
                gaps,
                m.synth.keyframes_asked
            );
        }
    }
}

/// A regression, found through TURN on TCP 443. The SFU wrote the audio of the room to a
/// joiner before its connection was up. str0m sent that backlog in one burst at connect, and
/// the jitter buffer of the joiner held seconds of audio.
#[test]
fn a_joiner_gets_no_backlog_of_audio_at_connect() {
    let link = Profile::half(Duration::from_millis(300), 0.0);
    let mut net = call(2, link);
    net.run(Duration::from_secs(3));
    let joiner = net.join("r", link, 99);
    net.run(Duration::from_secs(3));
    let latency = &net.member(joiner).synth.audio_latency;
    let worst = latency.iter().max().copied().unwrap_or_default();
    assert!(!latency.is_empty(), "the joiner heard nothing");
    assert!(
        worst < Duration::from_millis(400),
        "the joiner heard audio {worst:?} late"
    );
}
