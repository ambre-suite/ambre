//! The layer choice of the SFU, between synthetic participants in simulated time.
//!
//! Each run is seeded: the losses of its links and the random numbers str0m draws. So a run
//! follows one path, and its figures are the same on every machine and under any load.

use std::time::{Duration, Instant};

use str0m::media::MediaKind;

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

/// The seeds of the runs one proof spans. Each seed is one path, so several seeds keep a
/// proof from resting on one path that happens to pass.
const SEEDS: [u64; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// Joins `count` participants to one room over `profile`, the first one speaking.
fn call(count: u64, profile: Profile) -> Net {
    seeded_call(count, profile, 1)
}

/// The same call as [`call`], on the path of `seed`.
fn seeded_call(count: u64, profile: Profile, seed: u64) -> Net {
    let mut net = Net::new(seed);
    for index in 0..count {
        net.join("r", profile, seed * 1000 + index + 1);
        net.run(Duration::from_millis(200));
    }
    net.member(0).synth.speaking = true;
    net
}

/// The cap of the layer-drop runs: above the middle layer of the speaker and the lowest layer
/// of the other sender, below its top layer.
const CAP: u64 = 800_000;

/// Runs a call of three on the path of `seed`, until the receivers hold the top layer of the
/// speaker. Then it caps the downlink of participant 1 for 5 s. Answers the network and the
/// instant of the cap.
fn capped_run(seed: u64) -> (Net, Instant) {
    let mut net = seeded_call(3, proof_link(), seed);
    net.run(Duration::from_secs(15));
    let before = net.now - Duration::from_secs(1);
    assert!(
        since(track(&net, 1, 0), before).all(|a| a.layer == 2),
        "seed {seed}: before the cap, the receiver gets the top layer of the speaker"
    );
    let capped = net.now;
    net.member(1).down.profile.cap = Some(CAP);
    net.run(Duration::from_secs(5));
    (net, capped)
}

/// `CALL-038`: the layer drop, at 150 ms of round trip and 2 % loss. The downlink of a
/// receiver falls below the top layer of the speaker. The first packet of a lower layer then
/// reaches the receiver within 3 s, on each path of [`SEEDS`].
#[test]
fn a_capped_receiver_drops_a_layer_within_the_budget() {
    for seed in SEEDS {
        let (net, capped) = capped_run(seed);
        let dropped = since(track(&net, 1, 0), capped)
            .find(|a| a.layer < 2)
            .map(|a| a.at - capped);
        // The budget row of the layer drop reads these figures.
        eprintln!("seed {seed}: the capped receiver dropped a layer after {dropped:?}");
        assert!(
            dropped.is_some_and(|d| d <= Duration::from_secs(3)),
            "seed {seed}: the capped receiver dropped a layer after {dropped:?}"
        );
    }
}

/// `CALL-043`: the drop for the capped receiver leaves the layer of the other receiver, on
/// each path of [`SEEDS`].
#[test]
fn a_layer_drop_for_one_receiver_leaves_another_receiver_on_its_layer() {
    for seed in SEEDS {
        let (net, capped) = capped_run(seed);
        assert!(
            since(track(&net, 1, 0), capped).any(|a| a.layer < 2),
            "seed {seed}: the capped receiver dropped no layer"
        );
        assert!(
            since(track(&net, 2, 0), capped).all(|a| a.layer == 2),
            "seed {seed}: the other receiver lost the top layer of the speaker"
        );
    }
}

/// The noise of a downlink: up to 40 ms of jitter, and 30 % of loss for 300 ms.
const JITTER: Duration = Duration::from_millis(40);
const BURST: Duration = Duration::from_millis(300);
const BURST_LOSS: f64 = 0.30;

/// A fast drop must not drop on noise. A call of three runs at 150 ms of round trip and 2 %
/// loss. The downlink of a receiver carries jitter from its join, then a short burst of loss,
/// and no lasting congestion. The receiver keeps the top layer of the speaker, on each path of
/// [`SEEDS`].
#[test]
fn a_receiver_keeps_its_layer_through_jitter_and_a_loss_burst() {
    for seed in SEEDS {
        let mut net = Net::new(seed);
        for index in 0..3 {
            let mut profile = proof_link();
            if index == 1 {
                profile.jitter = JITTER;
            }
            net.join("r", profile, seed * 1000 + index + 1);
            net.run(Duration::from_millis(200));
        }
        net.member(0).synth.speaking = true;
        net.run(Duration::from_secs(15));
        let from = net.now;
        net.run(Duration::from_secs(2));
        net.member(1).down.profile.loss = BURST_LOSS;
        net.run(BURST);
        net.member(1).down.profile.loss = proof_link().loss;
        net.run(Duration::from_secs(3));
        let layers: Vec<u8> = since(track(&net, 1, 0), from).map(|a| a.layer).collect();
        assert!(!layers.is_empty(), "seed {seed}: no video of the speaker");
        let lower = layers.iter().filter(|l| **l < 2).count();
        assert_eq!(lower, 0, "seed {seed}: {lower} packets below the top layer");
    }
}

/// `CALL-044`: a receiver in a call of twenty keeps the top layer of the speaker. The links
/// carry 150 ms of round trip and a random loss of 2 % or of 10 %. str0m's estimate held it on the middle
/// layer at 2 %.
#[test]
fn a_receiver_of_twenty_keeps_the_top_layer_up_to_ten_percent_loss() {
    for loss in [0.02, 0.10] {
        let mut net = call(20, Profile::half(Duration::from_millis(150), loss));
        net.run(Duration::from_secs(20));
        let from = net.now;
        net.run(Duration::from_secs(10));
        let layers: Vec<u8> = since(track(&net, 1, 0), from).map(|a| a.layer).collect();
        assert!(
            !layers.is_empty(),
            "at a loss of {loss}, the receiver got no video of the speaker"
        );
        assert!(
            layers.iter().all(|l| *l == 2),
            "at a loss of {loss}, the receiver left the top layer: {} of {} packets below it",
            layers.iter().filter(|l| **l < 2).count(),
            layers.len()
        );
    }
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

/// A call of twelve where the speaker moves from participant 0 to participant 11, who joined
/// last. Answers the network and the offers each participant had before the move. The links
/// lose nothing, and their 150 ms of round trip let several pictures pass before a keyframe.
fn speaker_joins_the_nine() -> (Net, Vec<usize>) {
    let mut net = call(12, Profile::half(Duration::from_millis(150), 0.0));
    net.run(Duration::from_secs(4));
    let offers = net.members.iter().map(|m| m.offers).collect();
    net.member(0).synth.speaking = false;
    net.member(11).synth.speaking = true;
    net.run(Duration::from_secs(4));
    (net, offers)
}

/// `CALL-045`: in a call of twelve, a receiver gets three audio tracks and nine video tracks
/// from the SFU, whatever the count of senders.
#[test]
fn a_receiver_holds_a_fixed_count_of_slots() {
    let (net, _) = speaker_joins_the_nine();
    let receiving = &net.members.get(1).expect("a receiver").synth.receiving;
    let count = |kind| receiving.iter().filter(|(_, k)| *k == kind).count();
    assert_eq!(count(MediaKind::Audio), 3, "audio slots: {receiving:?}");
    assert_eq!(count(MediaKind::Video), 9, "video slots: {receiving:?}");
}

/// `CALL-046`: a change of speaker sends no participant a new offer.
#[test]
fn a_change_of_speaker_renegotiates_nothing() {
    let (net, before) = speaker_joins_the_nine();
    let after: Vec<usize> = net.members.iter().map(|m| m.offers).collect();
    assert_eq!(after, before, "offers before and after the move");
    assert!(before.iter().all(|o| *o == 1), "offers: {before:?}");
}

/// The video slot of receiver 1 that moved from one sender to another.
fn moved_slot(net: &Net) -> &VideoRx {
    net.members
        .get(1)
        .expect("a receiver")
        .synth
        .video
        .values()
        .find(|rx| {
            rx.arrivals.iter().any(|a| a.sender == 11) && rx.arrivals.iter().any(|a| a.sender != 11)
        })
        .expect("a slot that moved to the new speaker")
}

/// `CALL-048`: the sequence numbers of a slot stay continuous when it moves to another sender.
#[test]
fn the_sequence_numbers_of_a_slot_stay_continuous_across_a_change_of_sender() {
    let (net, _) = speaker_joins_the_nine();
    let rx = moved_slot(&net);
    for pair in rx.arrivals.windows(2) {
        if let [a, b] = pair {
            assert_eq!(b.seq, a.seq.wrapping_add(1), "a gap from {a:?} to {b:?}");
        }
    }
}

/// `CALL-049`: a slot moves to another sender on the first packet of one of its keyframes.
#[test]
fn a_slot_moves_to_another_sender_only_on_a_keyframe() {
    let (net, _) = speaker_joins_the_nine();
    let rx = moved_slot(&net);
    let first = rx
        .arrivals
        .iter()
        .find(|a| a.sender == 11)
        .expect("a packet of the new speaker");
    assert!(
        first.keyframe_start && first.index == 0,
        "the slot moved on {first:?}"
    );
}

/// `CALL-047`: of five other participants, a receiver gets the audio of the three who spoke
/// last, and nothing of the two others.
#[test]
fn a_receiver_gets_the_audio_of_the_three_most_recent_speakers_alone() {
    let mut net = call(6, Profile::CLEAN);
    net.run(Duration::from_secs(3));
    for speaker in [1, 2, 3] {
        for m in &mut net.members {
            m.synth.speaking = false;
        }
        net.member(speaker).synth.speaking = true;
        net.run(Duration::from_secs(2));
    }
    let from = net.now - Duration::from_secs(1);
    let mut senders: Vec<u16> = net
        .members
        .get(5)
        .expect("a receiver")
        .synth
        .audio_from
        .iter()
        .filter(|(at, _)| *at >= from)
        .map(|(_, sender)| *sender)
        .collect();
    senders.sort_unstable();
    senders.dedup();
    assert_eq!(senders, vec![1, 2, 3], "the receiver heard {senders:?}");
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

/// The delay step of the recovery runs: a one-way delay that grows once, mid-call, and stays.
const DELAY_STEP: Duration = Duration::from_millis(40);

/// `CALL-050`: a false drop recovers. A call of three runs at 150 ms of round trip and 2 %
/// loss. The one-way delay of the downlink of a receiver then grows once by 40 ms, with no
/// cap. The receiver gets the top layer of the speaker again within 2 s of the step, and keeps
/// it for the rest of the run, on each path of [`SEEDS`].
#[test]
fn a_receiver_climbs_back_to_the_top_layer_after_a_delay_step() {
    for seed in SEEDS {
        let mut net = seeded_call(3, proof_link(), seed);
        net.run(Duration::from_secs(15));
        let before = net.now - Duration::from_secs(1);
        assert!(
            since(track(&net, 1, 0), before).all(|a| a.layer == 2),
            "seed {seed}: before the step, the receiver gets the top layer of the speaker"
        );
        let stepped = net.now;
        net.member(1).down.profile.delay += DELAY_STEP;
        net.run(Duration::from_secs(8));
        let arrivals: Vec<&Arrival> = since(track(&net, 1, 0), stepped).collect();
        let dropped = arrivals
            .iter()
            .find(|a| a.layer < 2)
            .map(|a| a.at - stepped);
        // The last packet below the top layer: the receiver holds the top layer after it.
        let back = arrivals
            .iter()
            .rposition(|a| a.layer < 2)
            .and_then(|i| arrivals.get(i + 1))
            .map(|a| a.at - stepped);
        eprintln!("seed {seed}: dropped after {dropped:?}, back on the top layer after {back:?}");
        let tail = arrivals
            .iter()
            .filter(|a| a.at >= stepped + Duration::from_secs(7))
            .count();
        assert!(
            tail > 0,
            "seed {seed}: no video of the speaker in the last second"
        );
        assert!(
            dropped.is_none() || back.is_some_and(|b| b <= Duration::from_secs(2)),
            "seed {seed}: dropped after {dropped:?}, back on the top layer after {back:?}"
        );
    }
}

/// A cap that holds the middle layer of the speaker and the lowest layer of the other sender,
/// with the margin of a move up, and not the top layer of the speaker.
const MIDDLE_CAP: u64 = 1_200_000;

/// The target the top layer of the speaker needs, beside the lowest layer of the other sender:
/// their sum under the share and the margin of the layer choice, about 2.43 Mbit/s.
const TOP_NEEDS: f64 = (1_500_000.0 + 150_000.0) * 1.25 / 0.85;

/// `CALL-051`: a probe never lifts a capped receiver to a layer its downlink cannot carry. A
/// call of three runs at 150 ms of round trip and 2 % loss. The downlink of a receiver is then
/// capped below the top layer of the speaker for 20 s. After its drop, the receiver gets no
/// packet of the top layer, and its target stays under what the top layer needs, on each path of [`SEEDS`].
#[test]
fn a_receiver_capped_below_the_top_layer_never_climbs_to_it() {
    for seed in SEEDS {
        let mut net = seeded_call(3, proof_link(), seed);
        net.run(Duration::from_secs(15));
        let capped = net.now;
        net.member(1).down.profile.cap = Some(MIDDLE_CAP);
        let id = net.member(1).id;
        let mut highest: f64 = 0.0;
        for _ in 0..2000 {
            net.run(Duration::from_millis(10));
            let target = net.sfu.estimate_of(id).expect("a receiver").as_f64();
            if net.now >= capped + Duration::from_secs(3) {
                highest = highest.max(target);
            }
        }
        let settled = capped + Duration::from_secs(3);
        let layers: Vec<u8> = since(track(&net, 1, 0), settled).map(|a| a.layer).collect();
        let top = layers.iter().filter(|l| **l == 2).count();
        let moves = layers.windows(2).filter(|w| w.first() != w.last()).count();
        eprintln!(
            "seed {seed}: {} packets after the drop, {top} on the top layer, {moves} switches, \
             highest target {highest:.0} bit/s",
            layers.len()
        );
        assert!(
            layers.len() > 500,
            "seed {seed}: {} packets of the speaker after the drop",
            layers.len()
        );
        assert_eq!(top, 0, "seed {seed}: {top} packets on the top layer");
        assert!(
            highest < TOP_NEEDS,
            "seed {seed}: the target reached {highest:.0} bit/s"
        );
    }
}
