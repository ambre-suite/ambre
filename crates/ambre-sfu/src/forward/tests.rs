//! The forwarding core between str0m clients, over an in-memory link in simulated time.
//!
//! Each client is a str0m connection in RTP mode, as a browser's peer connection is to the
//! SFU. It offers one audio and one video track, and answers each offer of the SFU.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ambre_core::Payload;
use str0m::format::Codec;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{DatagramRecv, Receive, Transmit};
use str0m::rtp::{RtpWrite, SeqNo};
use str0m::{Candidate, Event, Input, Output, Rtc};

use super::{Participant, Room, Sfu, Signal};
use ambre_core::{Seq, Track, Validated};

/// One packet a client received: the track, the header fields the SFU keeps, the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Got {
    kind: MediaKind,
    /// Whether the packet left on the screen track, or arrived on the screen slot.
    screen: bool,
    seq: u16,
    time: u32,
    marker: bool,
    payload: Vec<u8>,
}

/// One simulated browser.
struct Client {
    id: Seq<Participant>,
    addr: SocketAddr,
    rtc: Rtc,
    sending: Vec<(MediaKind, Mid)>,
    receiving: Vec<(Mid, MediaKind)>,
    got: Vec<Got>,
    keyframe_requests: usize,
    /// How many times the SFU told this client that a later share took the screen.
    taken_over: usize,
    /// The last sequence number written on each track: audio, camera, then screen.
    last_seq: [u64; 3],
}

/// The section of an offer of the SFU that holds the screen slot: after the three audio slots
/// and the nine video slots (`CALL-104`).
const SCREEN_SECTION: usize = 12;

impl Client {
    /// Writes one packet on the track of `kind`, and answers what a receiver must read.
    fn send(&mut self, now: Instant, kind: MediaKind, payload: &[u8], marker: bool) -> Got {
        let track = usize::from(kind == MediaKind::Video);
        self.send_on(now, track, payload, marker)
    }

    /// Writes one packet on the screen track, and answers what a receiver must read.
    fn send_screen(&mut self, now: Instant, payload: &[u8]) -> Got {
        self.send_on(now, 2, payload, true)
    }

    /// Writes one audio packet at the RFC 6464 `level`, in -dBov, and answers what a receiver
    /// must read.
    fn speak(&mut self, now: Instant, payload: &[u8], level: i8) -> Got {
        self.write(now, 0, payload, true, Some(level))
    }

    /// Writes one packet on the track at `track` of the offer: audio, camera or screen.
    fn send_on(&mut self, now: Instant, track: usize, payload: &[u8], marker: bool) -> Got {
        self.write(now, track, payload, marker, None)
    }

    /// Writes one packet on the track at `track`, with the audio level `level` when one is
    /// given.
    fn write(
        &mut self,
        now: Instant,
        track: usize,
        payload: &[u8],
        marker: bool,
        level: Option<i8>,
    ) -> Got {
        let (kind, mid) = *self.sending.get(track).expect("a track");
        let codec = if kind == MediaKind::Audio {
            Codec::Opus
        } else {
            Codec::Vp8
        };
        let pt = self
            .rtc
            .codec_config()
            .find(|p| p.spec().codec == codec)
            .expect("a payload type")
            .pt();
        let last = self.last_seq.get_mut(track).expect("three tracks");
        *last += 1;
        let seq = *last;
        let time = u32::try_from(seq * 960).expect("a small time");
        let mut api = self.rtc.direct_api();
        let stream = api.stream_tx_by_mid(mid, None).expect("a send stream");
        let ext = str0m::rtp::ExtensionValues {
            audio_level: level,
            // str0m writes the level of RFC 6464 only beside its voice activity bit.
            voice_activity: level.map(|_| true),
            ..str0m::rtp::ExtensionValues::default()
        };
        stream.write_rtp(
            RtpWrite::new(pt, SeqNo::from(seq), time, now, payload.to_vec())
                .marker(marker)
                .ext_vals(ext),
        );
        Got {
            kind,
            screen: track == 2,
            seq: u16::try_from(seq).expect("a small sequence number"),
            time,
            marker,
            payload: payload.to_vec(),
        }
    }

    /// The packets received on the slots of `kind`, the screen slot left out.
    fn got(&self, kind: MediaKind) -> Vec<Got> {
        self.got
            .iter()
            .filter(|g| g.kind == kind && !g.screen)
            .cloned()
            .collect()
    }

    /// The packets received on the screen slot.
    fn got_screen(&self) -> Vec<Got> {
        self.got.iter().filter(|g| g.screen).cloned().collect()
    }
}

/// The SFU and its clients, on one simulated network.
struct Net {
    now: Instant,
    sfu: Sfu,
    clients: Vec<Client>,
    /// The signals of the SFU to a client that has not taken its answer yet. The signalling
    /// socket carries them after the answer, so they wait for the client.
    held: Vec<(Seq<Participant>, Signal)>,
}

fn room(name: &str) -> Validated<Room> {
    Validated::<Room>::parse(name).expect("a room name")
}

impl Net {
    fn new() -> Self {
        let sfu_addr: SocketAddr = (Ipv4Addr::new(10, 0, 0, 254), 4000).into();
        Self {
            now: Instant::now(),
            sfu: Sfu::new(sfu_addr),
            clients: Vec::new(),
            held: Vec::new(),
        }
    }

    /// Joins a new client to `name`, which offers an audio and a camera track, as a page does,
    /// and answers its index. The screen track comes with a later offer ([`add_screen`]).
    fn join(&mut self, name: &str) -> usize {
        self.join_with(name, Rtc::builder())
    }

    /// Joins a new client to `name` as [`Net::join`] does, with the codecs of `config`.
    fn join_with(&mut self, name: &str, config: str0m::RtcConfig) -> usize {
        self.join_offering(name, config, |sdp| sdp, Duration::ZERO)
    }

    /// Joins a new client to `name` as [`Net::join_with`] does. `rewrite` changes the text of
    /// its offer on the way to the SFU, as the offer of a browser may differ. The client takes
    /// the answer of the SFU once the network ran for `answered_after`. The answer of a browser
    /// crosses the signalling and the page first.
    fn join_offering(
        &mut self,
        name: &str,
        config: str0m::RtcConfig,
        rewrite: impl FnOnce(String) -> String,
        answered_after: Duration,
    ) -> usize {
        let index = self.clients.len();
        let last = u8::try_from(index + 1).expect("a few clients");
        let addr: SocketAddr = (Ipv4Addr::new(10, 0, 0, last), 5000).into();
        let mut rtc = config
            .set_rtp_mode(true)
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .build(self.now);
        rtc.add_local_candidate(Candidate::host(addr, "udp").expect("a candidate"));
        let mut change = rtc.sdp_api();
        let audio = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let video = change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().expect("an offer");
        let offer = str0m::change::SdpOffer::from_sdp_string(&rewrite(offer.to_sdp_string()))
            .expect("a rewritten offer");
        let (id, answer) = self
            .sfu
            .join(self.now, room(name), Seq::new(index as u64), offer)
            .expect("the SFU admits the client");
        self.run(answered_after);
        rtc.sdp_api()
            .accept_answer(pending, answer)
            .expect("the client takes the answer");
        self.clients.push(Client {
            id,
            addr,
            rtc,
            sending: vec![(MediaKind::Audio, audio), (MediaKind::Video, video)],
            receiving: Vec::new(),
            got: Vec::new(),
            keyframe_requests: 0,
            taken_over: 0,
            last_seq: [0, 0, 0],
        });
        index
    }

    /// Runs the network for `span` of simulated time, in steps of 5 ms.
    fn run(&mut self, span: Duration) {
        let end = self.now + span;
        while self.now < end {
            self.step();
            self.now += Duration::from_millis(5);
            self.sfu.tick(self.now);
            for client in &mut self.clients {
                client
                    .rtc
                    .handle_input(Input::Timeout(self.now))
                    .expect("timeout");
            }
        }
    }

    /// Polls every side and delivers what each sends, until the network is quiet.
    fn step(&mut self) {
        loop {
            let mut to_clients = Vec::new();
            self.sfu.poll(self.now, |t| to_clients.push(t));
            let mut signals = std::mem::take(&mut self.held);
            while let Some(signal) = self.sfu.poll_signal() {
                signals.push(signal);
            }
            for (id, signal) in signals {
                let Some(client) = self.clients.iter_mut().find(|c| c.id == id) else {
                    self.held.push((id, signal));
                    continue;
                };
                let offer = match signal {
                    Signal::Offer(offer) => offer,
                    Signal::TakenOver => {
                        client.taken_over += 1;
                        continue;
                    }
                };
                let answer = client
                    .rtc
                    .sdp_api()
                    .accept_offer(offer)
                    .expect("the client takes the offer");
                self.sfu.accept_answer(id, answer);
            }
            let mut to_sfu = Vec::new();
            for client in &mut self.clients {
                poll_client(client, &mut to_sfu);
            }
            if to_clients.is_empty() && to_sfu.is_empty() {
                return;
            }
            for t in to_sfu {
                self.sfu
                    .receive(self.now, t.source, Payload::new(t.contents.to_vec()));
            }
            // str0m keeps one received packet until the next poll: each delivery is polled.
            let mut back = Vec::new();
            for t in to_clients {
                if let Some(client) = self.clients.iter_mut().find(|c| c.addr == t.destination) {
                    deliver(self.now, client, &t);
                    poll_client(client, &mut back);
                }
            }
            for t in back {
                self.sfu
                    .receive(self.now, t.source, Payload::new(t.contents.to_vec()));
            }
        }
    }
}

fn poll_client(client: &mut Client, out: &mut Vec<Transmit>) {
    loop {
        match client.rtc.poll_output().expect("poll") {
            Output::Timeout(_) => return,
            Output::Transmit(t) => out.push(t),
            Output::Event(Event::MediaAdded(added)) => {
                client.receiving.push((added.mid, added.kind));
            }
            Output::Event(Event::RtpPacket(packet)) => {
                let mid = client
                    .rtc
                    .direct_api()
                    .stream_rx(&packet.header.ssrc)
                    .expect("a receive stream")
                    .mid();
                let (section, kind) = client
                    .receiving
                    .iter()
                    .enumerate()
                    .find(|(_, (m, _))| *m == mid)
                    .map(|(at, (_, k))| (at, *k))
                    .expect("a track offered by the SFU");
                client.got.push(Got {
                    kind,
                    screen: section == SCREEN_SECTION,
                    seq: packet.header.sequence_number,
                    time: packet.header.timestamp,
                    marker: packet.header.marker,
                    payload: packet.payload.to_vec(),
                });
            }
            Output::Event(Event::KeyframeRequest(_)) => client.keyframe_requests += 1,
            Output::Event(_) => {}
        }
    }
}

fn datagram(t: &Transmit) -> DatagramRecv<'_> {
    (&t.contents[..]).try_into().expect("a datagram str0m sent")
}

fn deliver(now: Instant, client: &mut Client, t: &Transmit) {
    let contents = datagram(t);
    let input = Input::Receive(
        now,
        Receive {
            proto: t.proto,
            source: t.source,
            destination: t.destination,
            contents,
        },
    );
    client.rtc.handle_input(input).expect("receive");
}

/// Sends `count` audio and `count` video packets from client `from`, and answers them.
fn burst(net: &mut Net, from: usize, count: u8) -> (Vec<Got>, Vec<Got>) {
    let mut audio = Vec::new();
    let mut video = Vec::new();
    let tag = u8::try_from(from).expect("a few clients");
    for n in 0..count {
        let now = net.now;
        let client = net.clients.get_mut(from).expect("a client");
        let audio_chunk = sealed(Track::Audio, from, &[tag, n, 1, 2, 3]);
        audio.push(client.send(now, MediaKind::Audio, &audio_chunk, false));
        // A keyframe start, then bytes the SFU must copy without reading.
        let frame = keyframe(Track::Camera, from, &[tag, n, 9, 8, 7]);
        video.push(client.send(now, MediaKind::Video, &frame, n % 4 == 3));
        net.run(Duration::from_millis(20));
    }
    net.run(Duration::from_millis(500));
    (audio, video)
}

/// Joins `names`, in order, and runs the network until each connection carries its tracks.
fn connected(names: &[&str]) -> Net {
    let mut net = Net::new();
    for name in names {
        net.join(name);
        net.run(Duration::from_millis(300));
    }
    net.run(Duration::from_secs(2));
    net
}

fn client(net: &Net, index: usize) -> &Client {
    net.clients.get(index).expect("a client")
}

#[test]
fn each_packet_reaches_every_other_participant_of_the_room_unchanged() {
    let mut net = connected(&["r", "r", "r"]);
    let (audio, video) = burst(&mut net, 0, 20);
    for receiver in [1, 2] {
        assert_eq!(client(&net, receiver).got(MediaKind::Audio), audio);
        assert_eq!(client(&net, receiver).got(MediaKind::Video), video);
    }
}

#[test]
fn no_packet_goes_back_to_its_sender_or_into_another_room() {
    let mut net = connected(&["r", "r", "s"]);
    let (audio, _) = burst(&mut net, 0, 10);
    assert_eq!(client(&net, 1).got(MediaKind::Audio), audio);
    assert!(
        client(&net, 0).got.is_empty(),
        "the sender got its own packets"
    );
    assert!(
        client(&net, 2).got.is_empty(),
        "another room got the packets"
    );
    assert!(
        client(&net, 2).receiving.is_empty(),
        "another room was offered the tracks"
    );
}

#[test]
fn a_late_joiner_receives_the_tracks_already_in_the_room() {
    let mut net = connected(&["r"]);
    burst(&mut net, 0, 5);
    net.join("r");
    net.run(Duration::from_secs(2));
    let (audio, video) = burst(&mut net, 0, 10);
    assert_eq!(client(&net, 1).got(MediaKind::Audio), audio);
    assert_eq!(client(&net, 1).got(MediaKind::Video), video);
}

/// The offer of a browser holds the candidates it gathered by the time the page reads it.
/// Chrome gathers its TCP candidates first. The SFU still seats a client whose candidates pair
/// with none of its own: it learns the address of the client from its first check.
#[test]
fn an_offer_whose_candidates_pair_with_none_of_the_sfu_still_connects() {
    let mut net = connected(&["r"]);
    let late = net.join_offering(
        "r",
        Rtc::builder(),
        |sdp| {
            sdp.lines()
                .map(|line| {
                    if line.starts_with("a=candidate:") {
                        "a=candidate:1 1 tcp 1518280447 10.0.0.2 9 typ host tcptype active"
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\r\n")
                + "\r\n"
        },
        Duration::from_millis(200),
    );
    net.run(Duration::from_secs(2));
    let (audio, video) = burst(&mut net, late, 10);
    assert_eq!(client(&net, 0).got(MediaKind::Audio), audio);
    assert_eq!(client(&net, 0).got(MediaKind::Video), video);
}

#[test]
fn a_keyframe_request_reaches_the_sender_of_the_track() {
    let mut net = connected(&["r", "r"]);
    let before = client(&net, 0).keyframe_requests;
    let receiver = net.clients.get_mut(1).expect("a client");
    let (mid, _) = *receiver
        .receiving
        .iter()
        .find(|(_, k)| *k == MediaKind::Video)
        .expect("a video track from the SFU");
    // The receive stream exists once a packet arrived on it.
    burst(&mut net, 0, 2);
    let receiver = net.clients.get_mut(1).expect("a client");
    receiver
        .rtc
        .direct_api()
        .stream_rx_by_mid(mid, None)
        .expect("a receive stream")
        .request_keyframe(KeyframeRequestKind::Pli);
    net.run(Duration::from_millis(500));
    assert!(
        client(&net, 0).keyframe_requests > before,
        "the sender saw no keyframe request"
    );
}

/// The payloads client `c` read on its screen slot, past the first `from`.
fn since(net: &Net, c: usize, from: usize) -> Vec<Vec<u8>> {
    client(net, c)
        .got_screen()
        .into_iter()
        .skip(from)
        .map(|g| g.payload)
        .collect()
}

/// Sends `count` pictures of the screen of client `from`, one keyframe each, and answers them.
/// The opaque bytes carry the sharer and the number of the picture.
fn share(net: &mut Net, from: usize, count: u8) -> Vec<Got> {
    let tag = u8::try_from(from).expect("a few clients");
    let mut sent = Vec::new();
    for n in 0..count {
        let now = net.now;
        let client = net.clients.get_mut(from).expect("a client");
        sent.push(client.send_screen(now, &keyframe(Track::Screen, from, &[tag, n])));
        net.run(Duration::from_millis(40));
    }
    net.run(Duration::from_millis(500));
    sent
}

/// `CALL-104`: the SFU offers each receiver one screen slot after its nine video slots. The
/// screen of a sharer reaches every other receiver over that slot, unchanged, and no video
/// slot carries a packet of it.
#[test]
fn a_screen_reaches_each_receiver_over_its_screen_slot_alone() {
    let mut net = connected(&["r", "r", "r"]);
    for receiver in [1, 2] {
        let sections = &client(&net, receiver).receiving;
        assert_eq!(sections.len(), SCREEN_SECTION + 1, "{sections:?}");
        assert_eq!(
            sections.get(SCREEN_SECTION).map(|(_, kind)| *kind),
            Some(MediaKind::Video)
        );
    }
    let (_, camera) = burst(&mut net, 0, 4);
    start_share(&mut net, 0);
    let shown = share(&mut net, 0, 12);
    let payloads = |got: Vec<Got>| got.into_iter().map(|g| g.payload).collect::<Vec<_>>();
    for receiver in [1, 2] {
        assert_eq!(
            payloads(client(&net, receiver).got_screen()),
            payloads(shown.clone()),
            "receiver {receiver} reads the screen on its screen slot"
        );
        assert_eq!(
            client(&net, receiver).got(MediaKind::Video),
            camera,
            "receiver {receiver} reads a screen packet on a video slot"
        );
    }
    assert!(client(&net, 0).got_screen().is_empty());
}

/// `CALL-106` and `CALL-111`: one screen per room. Ada shares, then Bob shares: Carol and Ada
/// read Bob's screen from his first keyframe on, and Bob reads none. While Ada's share goes on,
/// no packet of it reaches anyone. Once Bob stops and Ada starts a share again, her share is the
/// last one, and Bob and Carol read it.
#[test]
fn the_screen_of_the_last_share_reaches_each_other_receiver_and_none_reaches_its_sharer() {
    let mut net = connected(&["r", "r", "r"]);
    start_share(&mut net, 0);
    let ada = share(&mut net, 0, 6);
    assert_eq!(client(&net, 1).got_screen().len(), ada.len());
    let bob_before = client(&net, 1).got_screen().len();
    let ada_before = client(&net, 0).got_screen().len();
    let carol_before = client(&net, 2).got_screen().len();
    // Bob starts while Ada still shares: each sends a picture in turn.
    start_share(&mut net, 1);
    let mut bob = Vec::new();
    for n in 0..10u8 {
        let now = net.now;
        bob.push(
            net.clients
                .get_mut(1)
                .expect("Bob")
                .send_screen(now, &keyframe(Track::Screen, 1, &[1, n])),
        );
        let now = net.now;
        net.clients
            .get_mut(0)
            .expect("Ada")
            .send_screen(now, &keyframe(Track::Screen, 0, &[0, 100 + n]));
        net.run(Duration::from_millis(40));
    }
    net.run(Duration::from_millis(500));
    let bob_payloads: Vec<Vec<u8>> = bob.iter().map(|g| g.payload.clone()).collect();
    // The slot moves to Bob on his first keyframe, so a picture of Ada sent at the same
    // instant may still pass before it (`CALL-049`). None passes after.
    for (who, before) in [("Carol", carol_before), ("Ada", ada_before)] {
        let read = since(&net, if who == "Ada" { 0 } else { 2 }, before);
        let first = read
            .iter()
            // The sharer byte sits past the keyframe start and the chunk header.
            .position(|p| p.get(KEYFRAME_START.len() + 9) == Some(&1))
            .unwrap_or(read.len());
        assert!(
            first <= 1,
            "{who} read {first} pictures of Ada after Bob's share"
        );
        assert_eq!(
            read.get(first..).map(<[_]>::to_vec),
            Some(bob_payloads.clone()),
            "{who} reads Bob's screen alone once it reaches her"
        );
    }
    assert!(
        since(&net, 1, bob_before).is_empty(),
        "Bob reads a screen while his share is the last one"
    );
    // Bob stops; Ada starts a share again, and her share is the last one.
    net.run(Duration::from_secs(3));
    start_share(&mut net, 0);
    let (bob_before, carol_before) = (
        client(&net, 1).got_screen().len(),
        client(&net, 2).got_screen().len(),
    );
    let again = share(&mut net, 0, 6);
    let again: Vec<Vec<u8>> = again.into_iter().map(|g| g.payload).collect();
    assert_eq!(since(&net, 1, bob_before), again);
    assert_eq!(since(&net, 2, carol_before), again);
}

/// Marks the start of a share of client `who`, as its page does when the screen sender takes
/// a track.
fn start_share(net: &mut Net, who: usize) {
    if client(net, who).sending.len() < 3 {
        add_screen(net, who);
    }
    let id = client(net, who).id;
    let now = net.now;
    net.sfu.share_started(now, id);
}

/// `CALL-106`: a still screen keeps no hold on the screen of the room. Ada shares, then Bob
/// shares. Ada's screen holds still for 3 s, past any pause of a picture, then changes. Carol
/// and Bob still read Bob's screen alone: a picture after a pause starts no share.
#[test]
fn a_still_screen_that_changes_again_takes_no_screen_back() {
    let mut net = connected(&["r", "r", "r"]);
    start_share(&mut net, 0);
    share(&mut net, 0, 4);
    start_share(&mut net, 1);
    let bob = share(&mut net, 1, 6);
    // Ada's screen held still, then changes: one picture of hers, then Bob's go on.
    net.run(Duration::from_secs(3));
    let (carol_before, bob_before) = (
        client(&net, 2).got_screen().len(),
        client(&net, 1).got_screen().len(),
    );
    let now = net.now;
    net.clients
        .get_mut(0)
        .expect("Ada")
        .send_screen(now, &keyframe(Track::Screen, 0, &[0, 200]));
    net.run(Duration::from_millis(100));
    let more = share(&mut net, 1, 6);
    let more: Vec<Vec<u8>> = more.into_iter().map(|g| g.payload).collect();
    assert!(!bob.is_empty());
    assert_eq!(
        since(&net, 2, carol_before),
        more,
        "Carol reads Ada's still screen again"
    );
    assert!(
        since(&net, 1, bob_before).is_empty(),
        "Bob reads Ada's screen while his share is the last one"
    );
}

/// Adds the screen section of client `who` by a later offer, as its page does at its first
/// share.
fn add_screen(net: &mut Net, who: usize) {
    let id = client(net, who).id;
    let c = net.clients.get_mut(who).expect("a client");
    let mut change = c.rtc.sdp_api();
    let screen = change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
    let (offer, pending) = change.apply().expect("an offer");
    let answer = net
        .sfu
        .accept_offer(id, offer)
        .expect("the SFU answers the later offer");
    c.rtc
        .sdp_api()
        .accept_answer(pending, answer)
        .expect("the client takes the answer");
    c.sending.push((MediaKind::Video, screen));
    net.run(Duration::from_millis(300));
}

/// `CALL-112`, `CALL-113`: a first share adds the screen section by one later offer, which the
/// SFU answers. The camera of the sharer reaches each receiver unchanged across it. The screen
/// then reaches each receiver over its screen slot, and no receiver gets a new offer.
#[test]
fn a_first_share_adds_the_screen_section_by_one_offer_and_the_camera_flows_across() {
    let mut net = connected(&["r", "r", "r"]);
    let sections: Vec<usize> = (1..3).map(|c| client(&net, c).receiving.len()).collect();
    let (_, before) = burst(&mut net, 0, 4);
    add_screen(&mut net, 0);
    let (_, after) = burst(&mut net, 0, 4);
    let start = net.now;
    net.sfu.share_started(start, client(&net, 0).id);
    let shown = share(&mut net, 0, 6);
    let payloads = |got: Vec<Got>| got.into_iter().map(|g| g.payload).collect::<Vec<_>>();
    for receiver in [1, 2] {
        assert_eq!(
            client(&net, receiver).got(MediaKind::Video),
            [before.clone(), after.clone()].concat(),
            "receiver {receiver} lost the camera across the later offer"
        );
        assert_eq!(
            payloads(client(&net, receiver).got_screen()),
            payloads(shown.clone())
        );
    }
    let after: Vec<usize> = (1..3).map(|c| client(&net, c).receiving.len()).collect();
    assert_eq!(after, sections, "a receiver got a new offer");
}

/// `CALL-114`: an offer of a participant that crosses an offer of the SFU still unanswered
/// gets no answer. Once the participant answers the SFU, its next offer gets one.
#[test]
fn an_offer_that_crosses_an_offer_of_the_sfu_gets_no_answer_until_it_is_answered() {
    let mut net = Net::new();
    net.join("r");
    net.run(Duration::from_millis(300));
    burst(&mut net, 0, 2);
    net.join("r");
    // The SFU offers client 0 the slots of the new track, and the answer waits.
    let mut out = Vec::new();
    let mut sfu_offer = None;
    for _ in 0..20 {
        net.sfu.poll(net.now, |t| out.push(t));
        if let Some((id, Signal::Offer(offer))) = net.sfu.poll_signal() {
            sfu_offer = Some((id, offer));
            break;
        }
        net.now += Duration::from_millis(5);
        net.sfu.tick(net.now);
        for c in &mut net.clients {
            poll_client(c, &mut Vec::new());
        }
    }
    let (id, sfu_offer) = sfu_offer.expect("an offer of the SFU");
    let who = net
        .clients
        .iter()
        .position(|c| c.id == id)
        .expect("the client of the offer");
    let c = net.clients.get_mut(who).expect("a client");
    let mut change = c.rtc.sdp_api();
    change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
    let (crossing, _) = change.apply().expect("an offer");
    assert!(
        net.sfu.accept_offer(id, crossing).is_none(),
        "a crossing offer got an answer"
    );
    let c = net.clients.get_mut(who).expect("a client");
    let answer = c
        .rtc
        .sdp_api()
        .accept_offer(sfu_offer)
        .expect("the client answers");
    net.sfu.accept_answer(id, answer);
    net.run(Duration::from_millis(300));
    let c = net.clients.get_mut(who).expect("a client");
    let mut change = c.rtc.sdp_api();
    change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
    let (again, _) = change.apply().expect("an offer");
    assert!(
        net.sfu.accept_offer(id, again).is_some(),
        "the next offer got no answer"
    );
}

/// `CALL-107`: the SFU tells the former sharer, and it alone, that a later share took the
/// screen. Ada shares, Bob takes the screen over, then Ada takes it back: Ada hears it once,
/// then Bob once. Carol, who shares nothing, hears nothing. A first share tells no one.
#[test]
fn a_later_share_tells_the_former_sharer_alone_each_time() {
    let mut net = connected(&["r", "r", "r"]);
    start_share(&mut net, 0);
    share(&mut net, 0, 4);
    let heard = |net: &Net| -> Vec<usize> { net.clients.iter().map(|c| c.taken_over).collect() };
    assert_eq!(heard(&net), vec![0, 0, 0], "a first share tells someone");
    start_share(&mut net, 1);
    share(&mut net, 1, 4);
    assert_eq!(
        heard(&net),
        vec![1, 0, 0],
        "Bob's share told the wrong client"
    );
    // Ada takes the screen back with the share she had before: Bob hears it.
    start_share(&mut net, 0);
    let back = share(&mut net, 0, 4);
    assert_eq!(
        heard(&net),
        vec![1, 1, 0],
        "Ada's return told the wrong client"
    );
    let carol: Vec<Vec<u8>> = client(&net, 2)
        .got_screen()
        .into_iter()
        .rev()
        .take(back.len())
        .rev()
        .map(|g| g.payload)
        .collect();
    let back: Vec<Vec<u8>> = back.into_iter().map(|g| g.payload).collect();
    assert_eq!(carol, back, "Carol reads Ada's screen again");
}

/// A chunk of client `sender` on `track`: its header, then the bytes `tail`.
fn sealed(track: Track, sender: usize, tail: &[u8]) -> Vec<u8> {
    let mut chunk = chunk_naming(track, u64::try_from(sender).expect("a small index"));
    chunk.truncate(9);
    chunk.extend_from_slice(tail);
    chunk
}

/// The first packet of a keyframe of client `sender` on `track`, whose chunk ends with `tail`.
fn keyframe(track: Track, sender: usize, tail: &[u8]) -> Vec<u8> {
    let mut packet = KEYFRAME_START.to_vec();
    packet.extend(sealed(track, sender, tail));
    packet
}

/// A chunk header, as the seal writes one, naming `sender` on `track`, then bytes for the
/// counter and the tag.
fn chunk_naming(track: Track, sender: u64) -> Vec<u8> {
    let kid = ambre_core::KeyIdentifier {
        track,
        sender: Seq::new(sender),
        generation: Seq::new(1),
    };
    let mut chunk = vec![ambre_core::CHUNK_CONFIG_BYTE];
    chunk.extend_from_slice(kid.to_bytes().expect("a small identifier").as_slice());
    chunk.extend_from_slice(&[0; 8 + 16]);
    chunk
}

/// `CALL-116`: Mallory, seated as sender 0, sends an audio chunk and the first packet of a
/// camera frame under the identifier of Bob, sender 1. No other participant receives either,
/// and her own chunks still pass.
#[test]
fn a_chunk_under_the_identifier_of_another_sender_never_leaves_the_sfu() {
    let mut net = connected(&["r", "r", "r"]);
    let mallory = net.clients.get_mut(0).expect("a client");
    let now = net.now;
    mallory.send(now, MediaKind::Audio, &chunk_naming(Track::Audio, 1), true);
    // A keyframe start, which would open the video slot, then the chunk header.
    let mut frame = KEYFRAME_START.to_vec();
    frame.extend(chunk_naming(Track::Camera, 1));
    mallory.send(now, MediaKind::Video, &frame, true);
    net.run(Duration::from_millis(200));
    assert!(
        client(&net, 2).got.is_empty(),
        "a forged chunk was forwarded"
    );

    let mallory = net.clients.get_mut(0).expect("a client");
    let now = net.now;
    let own = mallory.send(now, MediaKind::Audio, &chunk_naming(Track::Audio, 0), true);
    net.run(Duration::from_millis(200));
    assert_eq!(client(&net, 2).got(MediaKind::Audio), vec![own]);
}

/// Audit `N25`: Mallory, sender 0, sends loud audio packets that hold no chunk header, after
/// Bob, Carol and Dan spoke under their own headers. A packet that names no seat counts as no
/// speech. So the three audio slots of Eve carry the three speakers, and Mallory's latest
/// packet reaches nobody.
#[test]
fn a_loud_packet_that_names_no_seat_takes_no_speaker_slot() {
    let mut net = connected(&["r", "r", "r", "r", "r"]);
    let speak_all = |net: &mut Net, round: u8| {
        for speaker in 1..=3u64 {
            let now = net.now;
            let index = usize::try_from(speaker).expect("a small index");
            let client = net.clients.get_mut(index).expect("a client");
            client.speak(now, &chunk_naming(Track::Audio, speaker), -20);
        }
        let now = net.now;
        let mallory = net.clients.get_mut(0).expect("a client");
        mallory.speak(now, &[0xA0, round, 1, 2, 3], -10)
    };
    for round in 0..5 {
        speak_all(&mut net, round);
        net.run(Duration::from_millis(150));
    }
    let latest = speak_all(&mut net, 9);
    net.run(Duration::from_millis(300));
    let eve = client(&net, 4).got(MediaKind::Audio);
    assert!(
        !eve.iter().any(|g| g.payload == latest.payload),
        "a packet that names no seat took a speaker slot"
    );
    for speaker in 1..=3u64 {
        let named = chunk_naming(Track::Audio, speaker);
        assert!(
            eve.iter().any(|g| g.payload == named),
            "speaker {speaker} reached no slot"
        );
    }
}

/// The descriptor of a VP8 packet that starts a frame (S set, partition 0), then the clear
/// prefix of a keyframe. A video slot opens on a keyframe alone (`CALL-039`).
const KEYFRAME_START: [u8; 11] = [
    0x10, 0x00, 0x00, 0x00, 0x9d, 0x01, 0x2a, 0x00, 0x05, 0xd0, 0x02,
];

/// `CALL-140`, audit `P30`: Mallory, seated as sender 0, opens her camera with a keyframe of her
/// own. She then cuts the first packet of a keyframe inside its chunk header. The rest of the
/// header, under the index of Bob, sender 1, rides the next packet. A receiver joins the two and
/// reads the header whole, so the SFU drops the first.
#[test]
fn a_frame_start_cut_inside_its_chunk_header_never_leaves_the_sfu() {
    let mut net = connected(&["r", "r", "r"]);
    let mut own = KEYFRAME_START.to_vec();
    own.extend(chunk_naming(Track::Camera, 0));
    let forged = chunk_naming(Track::Camera, 1);
    let (head, tail) = forged.split_at(4);
    let mut start = KEYFRAME_START.to_vec();
    start.extend_from_slice(head);
    // The descriptor of a packet that continues the frame, then the rest of the header.
    let mut rest = vec![0x00];
    rest.extend_from_slice(tail);
    let mallory = net.clients.get_mut(0).expect("a client");
    let now = net.now;
    mallory.send(now, MediaKind::Video, &own, true);
    net.run(Duration::from_millis(100));
    let mallory = net.clients.get_mut(0).expect("a client");
    let now = net.now;
    mallory.send(now, MediaKind::Video, &start, false);
    mallory.send(now, MediaKind::Video, &rest, true);
    net.run(Duration::from_millis(200));
    let got = client(&net, 2).got(MediaKind::Video);
    assert!(
        got.iter().any(|g| g.payload == own),
        "her own keyframe did not pass"
    );
    assert!(
        !got.iter().any(|g| g.payload == start),
        "a frame start cut inside its header was forwarded"
    );
}

/// `CALL-139`, audit `P30`: Mallory offers H264 alone for her camera, a codec whose packets the SFU
/// cannot read. Her packets under the index of Bob reach nobody.
#[test]
fn a_camera_in_a_codec_other_than_vp8_never_leaves_the_sfu() {
    let mut net = Net::new();
    let h264 = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .enable_h264(true);
    net.join_with("r", h264);
    net.run(Duration::from_millis(300));
    for _ in 0..2 {
        net.join("r");
        net.run(Duration::from_millis(300));
    }
    net.run(Duration::from_secs(2));
    let mallory = net.clients.get_mut(0).expect("a client");
    let now = net.now;
    let (_, mid) = *mallory.sending.get(1).expect("a camera track");
    let pt = mallory
        .rtc
        .codec_config()
        .find(|p| p.spec().codec == Codec::H264)
        .map(|p| p.pt());
    let mut api = mallory.rtc.direct_api();
    // A camera the SFU refused has no stream to write on, so nothing leaves at all.
    if let (Some(pt), Some(stream)) = (pt, api.stream_tx_by_mid(mid, None)) {
        for seq in 1..=3u64 {
            let payload = chunk_naming(Track::Camera, 1);
            let time = u32::try_from(seq * 3000).expect("a small time");
            stream.write_rtp(RtpWrite::new(pt, SeqNo::from(seq), time, now, payload).marker(true));
        }
    }
    net.run(Duration::from_millis(300));
    for receiver in [1, 2] {
        assert!(
            client(&net, receiver).got(MediaKind::Video).is_empty(),
            "a camera packet in H264 was forwarded"
        );
    }
}

/// Whether `bytes` read as a STUN message: the two top bits clear, and the magic cookie.
fn is_stun(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 0xC0 == 0)
        && bytes.get(4..8) == Some(&[0x21, 0x12, 0xA4, 0x42][..])
}

/// The next ICE check client `c` sends the SFU, held back from the SFU. The clock of the
/// client alone moves, half a second a step, until its consent check leaves.
fn check_of(net: &mut Net, c: usize) -> Vec<u8> {
    let client = net.clients.get_mut(c).expect("a client");
    let mut at = net.now;
    for _ in 0..40 {
        at += Duration::from_millis(500);
        client
            .rtc
            .handle_input(Input::Timeout(at))
            .expect("timeout");
        loop {
            match client.rtc.poll_output().expect("poll") {
                Output::Timeout(_) => break,
                Output::Transmit(t) if is_stun(&t.contents[..]) => return t.contents.to_vec(),
                Output::Transmit(_) | Output::Event(_) => {}
            }
        }
    }
    panic!("the client sent no check within 20 s")
}

/// The datagrams that reached a connection of the SFU and wait for its next poll.
fn waiting(net: &Net) -> usize {
    net.sfu.members.iter().map(|m| m.inbox.len()).sum()
}

/// `CALL-144`, audit `N41`: a datagram that is no ICE check reaches no connection from a
/// source the SFU does not know, and leaves no route. A check of a participant replayed from a
/// new source reaches the connection its username fragment names, and that one alone.
#[test]
fn a_new_source_reaches_a_connection_by_its_check_alone() {
    let mut net = connected(&["r", "r", "r"]);
    let stranger: SocketAddr = (Ipv4Addr::new(10, 0, 1, 1), 6000).into();
    let routes = net.sfu.routes.len();
    let before = waiting(&net);
    let mut rtp = vec![0x80, 0x6F, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2];
    rtp.extend([0; 40]);
    net.sfu.receive(net.now, stranger, Payload::new(rtp));
    assert_eq!(
        waiting(&net),
        before,
        "a datagram of a new source reached a connection"
    );
    assert_eq!(
        net.sfu.routes.len(),
        routes,
        "a datagram of a new source left a route"
    );

    let check = check_of(&mut net, 1);
    let named = super::named_ufrag(&Payload::new(check.clone())).expect("a check names a fragment");
    let owner = net.sfu.ufrags.get(&named).copied();
    assert_eq!(
        owner,
        Some(client(&net, 1).id),
        "the fragment names the connection of its client"
    );
    net.sfu.receive(net.now, stranger, Payload::new(check));
    let member = net
        .sfu
        .members
        .iter()
        .find(|m| m.id == client(&net, 1).id)
        .expect("a member");
    assert_eq!(member.inbox.len(), 1, "the check missed its connection");
    assert_eq!(
        waiting(&net),
        before + 1,
        "the check reached another connection"
    );
    assert_eq!(net.sfu.routes.get(&stranger), Some(&client(&net, 1).id));
}

/// `CALL-145`, audit `N41`: a participant that checks from many sources holds no more routes
/// than the cap. The newest source routes, and the oldest ones are forgotten.
#[test]
fn a_participant_holds_no_more_routes_than_the_cap() {
    let mut net = connected(&["r", "r"]);
    let id = client(&net, 0).id;
    let check = check_of(&mut net, 0);
    let sources: Vec<SocketAddr> = (1..=20u8)
        .map(|n| (Ipv4Addr::new(10, 0, 2, n), 7000).into())
        .collect();
    for source in &sources {
        net.sfu
            .receive(net.now, *source, Payload::new(check.clone()));
    }
    assert_eq!(net.sfu.routes_of(id), super::ROUTES_PER_PARTICIPANT);
    let newest = sources.last().expect("a source");
    let oldest = sources.first().expect("a source");
    assert_eq!(
        net.sfu.routes.get(newest),
        Some(&id),
        "the newest source lost its route"
    );
    assert_eq!(
        net.sfu.routes.get(oldest),
        None,
        "the oldest source kept its route"
    );
    // The other participant keeps its own routes.
    assert!(net.sfu.routes_of(client(&net, 1).id) > ambre_core::Count::new(0));
}

/// `CALL-155`, audit `P60`: an offer that restarts ICE, as a page sends under a fresh TURN
/// credential, keeps the media of the connection flowing. The connection takes a fresh
/// fragment, and the old one names nothing. A check under the fresh fragment from a new source,
/// as a new TURN allocation sends it, reaches that connection.
#[test]
fn an_ice_restart_moves_the_connection_to_its_fresh_fragment() {
    let mut net = connected(&["r", "r"]);
    let id = client(&net, 0).id;
    let old = net
        .sfu
        .ufrags
        .iter()
        .find_map(|(ufrag, owner)| (*owner == id).then(|| ufrag.clone()))
        .expect("the fragment of the connection");

    let c = net.clients.get_mut(0).expect("a client");
    let mut change = c.rtc.sdp_api();
    change.ice_restart(true);
    let (offer, pending) = change.apply().expect("an offer that restarts ICE");
    let answer = net
        .sfu
        .accept_offer(id, offer)
        .expect("the SFU answers the restart");
    c.rtc
        .sdp_api()
        .accept_answer(pending, answer)
        .expect("the client takes the answer");
    net.run(Duration::from_secs(1));
    let before = client(&net, 1).got(MediaKind::Audio).len();
    let (audio, _) = burst(&mut net, 0, 4);
    let got = client(&net, 1).got(MediaKind::Audio);
    assert_eq!(
        got.get(before..),
        Some(&audio[..]),
        "the audio stopped across the restart"
    );

    let check = check_of(&mut net, 0);
    let fresh = super::named_ufrag(&Payload::new(check.clone())).expect("a fragment");
    assert_ne!(fresh, old, "the restart kept the fragment");
    assert_eq!(net.sfu.ufrags.get(&fresh), Some(&id));
    assert_eq!(
        net.sfu.ufrags.get(&old),
        None,
        "the old fragment still names it"
    );
    let relayed: SocketAddr = (Ipv4Addr::new(10, 0, 3, 1), 8000).into();
    net.sfu.receive(net.now, relayed, Payload::new(check));
    assert_eq!(
        net.sfu.routes.get(&relayed),
        Some(&id),
        "a check of the new allocation missed its connection"
    );
}
