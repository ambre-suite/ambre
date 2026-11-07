//! The forwarding core between str0m clients, over an in-memory link in simulated time.
//!
//! Each client is a str0m connection in RTP mode, as a browser's peer connection is to the
//! SFU. It offers one audio and one video track, and answers each offer of the SFU.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use str0m::format::Codec;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{DatagramRecv, Receive, Transmit};
use str0m::rtp::{RtpWrite, SeqNo};
use str0m::{Candidate, Event, Input, Output, Rtc};

use super::{Participant, Room, Sfu, Signal};
use ambre_core::{Seq, Validated};

/// One packet a client received: the track, the header fields the SFU keeps, the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Got {
    kind: MediaKind,
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
    /// The last sequence number written on each track, audio then video.
    last_seq: [u64; 2],
}

impl Client {
    /// Writes one packet on the track of `kind`, and answers what a receiver must read.
    fn send(&mut self, now: Instant, kind: MediaKind, payload: &[u8], marker: bool) -> Got {
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
        let (_, mid) = *self
            .sending
            .iter()
            .find(|(k, _)| *k == kind)
            .expect("a track");
        let last = self
            .last_seq
            .get_mut(usize::from(kind == MediaKind::Video))
            .expect("two tracks");
        *last += 1;
        let seq = *last;
        let time = u32::try_from(seq * 960).expect("a small time");
        let mut api = self.rtc.direct_api();
        let stream = api.stream_tx_by_mid(mid, None).expect("a send stream");
        stream.write_rtp(
            RtpWrite::new(pt, SeqNo::from(seq), time, now, payload.to_vec()).marker(marker),
        );
        Got {
            kind,
            seq: u16::try_from(seq).expect("a small sequence number"),
            time,
            marker,
            payload: payload.to_vec(),
        }
    }

    fn got(&self, kind: MediaKind) -> Vec<Got> {
        self.got
            .iter()
            .filter(|g| g.kind == kind)
            .cloned()
            .collect()
    }
}

/// The SFU and its clients, on one simulated network.
struct Net {
    now: Instant,
    sfu: Sfu,
    clients: Vec<Client>,
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
        }
    }

    /// Joins a new client to `name`, sending one audio and one video track, and answers its
    /// index.
    fn join(&mut self, name: &str) -> usize {
        let index = self.clients.len();
        let last = u8::try_from(index + 1).expect("a few clients");
        let addr: SocketAddr = (Ipv4Addr::new(10, 0, 0, last), 5000).into();
        let mut rtc = Rtc::builder()
            .set_rtp_mode(true)
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .build(self.now);
        rtc.add_local_candidate(Candidate::host(addr, "udp").expect("a candidate"));
        let mut change = rtc.sdp_api();
        let audio = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let video = change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().expect("an offer");
        let (id, answer) = self
            .sfu
            .join(self.now, room(name), offer)
            .expect("the SFU admits the client");
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
            last_seq: [0, 0],
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
            while let Some((id, Signal::Offer(offer))) = self.sfu.poll_signal() {
                let client = self
                    .clients
                    .iter_mut()
                    .find(|c| c.id == id)
                    .expect("a client");
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
                self.sfu.receive(self.now, t.source, datagram(&t));
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
                self.sfu.receive(self.now, t.source, datagram(&t));
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
                let kind = client
                    .receiving
                    .iter()
                    .find(|(m, _)| *m == mid)
                    .map(|(_, k)| *k)
                    .expect("a track offered by the SFU");
                client.got.push(Got {
                    kind,
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
        audio.push(client.send(now, MediaKind::Audio, &[0xA0, tag, n, 1, 2, 3], false));
        // A VP8 payload descriptor, then bytes the SFU must copy without reading.
        video.push(client.send(now, MediaKind::Video, &[0x10, tag, n, 9, 8, 7], n % 4 == 3));
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
