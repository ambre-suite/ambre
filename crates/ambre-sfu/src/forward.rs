//! The forwarding core: one str0m connection per participant, and the packets between them.
//!
//! The core does no I/O. A driver hands it each datagram and each tick, and sends what it
//! answers (`serve.rs` over a UDP socket, the tests over an in-memory link). It forwards in RTP
//! mode: it copies the payload of each packet, and never reassembles a frame (`CALL-032`).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ambre_core::{Issuer, Rule, Seq, SeqKind, Validated};
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::error::IceError;
use str0m::format::{Codec, PayloadParams};
use str0m::media::Rid;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{DatagramRecv, Protocol, Receive, Transmit};
use str0m::rtp::{RtpPacket, RtpWrite, SeqNo, Vp8Descriptor};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcError};

use crate::layer::{Candidate as Choice, Layer, Meter, Offered, choose, rates};
use crate::participant_config;
use crate::rewrite::Rewrite;

/// The most threads that run the connections of one poll.
const WORKERS: usize = 4;

/// The count of busy connections from which a poll spreads them over threads. Below it, the
/// start of a thread costs more than it saves.
const PARALLEL_FROM: usize = 8;

/// How often the SFU chooses the layers of every receiver again.
const CHOICE_PERIOD: Duration = Duration::from_millis(100);

/// How long the SFU waits for a keyframe of a layer before it asks the sender again.
const KEYFRAME_RETRY: Duration = Duration::from_millis(1000);

/// The audio level of RFC 6464, in -dBov, above which a packet counts as speech.
const SPEECH_LEVEL: i8 = -50;

/// The downlink a receiver is assumed to hold before its first estimate.
pub const INITIAL_ESTIMATE: Bitrate = Bitrate::kbps(1000);

/// The participant sessions of one SFU process, numbered from 1 as each joins. A reload is a
/// new join, under a new number.
pub enum Participant {}

impl SeqKind for Participant {
    type IssueRight = ();
    const NAME: &'static str = "Participant";
}

/// The rule of a room name: 1 to 64 ASCII letters, digits, `-` or `_`. Packets never cross two
/// rooms (`CALL-033`).
pub enum Room {}

impl Rule for Room {
    type Error = &'static str;
    const LOGGED_AS: Option<&'static str> = Some("Room");

    fn admit(text: String) -> Result<String, Self::Error> {
        let fits = (1..=64).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if fits {
            Ok(text)
        } else {
            Err("a room name is 1 to 64 ASCII letters, digits, '-' or '_'")
        }
    }
}

/// Why a join failed.
#[derive(Debug)]
pub enum JoinError {
    /// str0m refused the offer of the browser.
    Offer(RtcError),
    /// str0m refused the address of the SFU as a candidate.
    Candidate(IceError),
}

/// A session description the SFU sends to one participant, over its signalling socket.
#[derive(Debug)]
pub enum Signal {
    /// A new offer: the room carries a track this participant does not receive yet.
    Offer(SdpOffer),
}

/// What one track forwarded to a participant has reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutState {
    /// No offer names it yet.
    ToOpen,
    /// An offer names it under this mid, and the answer has not come back.
    Negotiating(Mid),
    /// The answer came back: the track flows under this mid.
    Open(Mid),
}

/// One track of another participant, forwarded to this one.
#[derive(Debug)]
struct TrackOut {
    source: Seq<Participant>,
    source_mid: Mid,
    kind: MediaKind,
    state: OutState,
    /// The rollover part of the first sequence number forwarded, for an audio track. The
    /// receiver starts its SRTP rollover counter at zero. So the SFU subtracts that part, and
    /// the 16 bits of the header stay as the sender wrote them.
    rollover: Option<SeqNo>,
    /// For a video track: the layer the receiver gets, the one it should get, and the
    /// header fields across a switch.
    video: VideoOut,
}

/// The layer state of one video track forwarded to one receiver.
#[derive(Debug, Default)]
struct VideoOut {
    /// The layer whose packets flow to the receiver.
    current: Option<Layer>,
    /// The layer the choice wants. A switch to it waits for one of its keyframes (`CALL-039`).
    target: Option<Layer>,
    /// When the SFU last asked the sender for a keyframe of the target.
    asked: Option<Instant>,
    rewrite: Rewrite,
}

impl TrackOut {
    fn new(source: Seq<Participant>, source_mid: Mid, kind: MediaKind) -> Self {
        Self {
            source,
            source_mid,
            kind,
            state: OutState::ToOpen,
            rollover: None,
            video: VideoOut::default(),
        }
    }
}

/// One track a participant sends.
#[derive(Debug)]
struct TrackIn {
    mid: Mid,
    kind: MediaKind,
    /// The layers seen so far, each with its measured bitrate.
    layers: Vec<(Layer, Meter)>,
}

impl TrackIn {
    /// The layers of the track, from the lowest measured bitrate up.
    fn offered(&self) -> Vec<Offered> {
        let mut offered: Vec<Offered> = self
            .layers
            .iter()
            .map(|(layer, meter)| Offered {
                layer: *layer,
                rate: meter.rate(),
            })
            .collect();
        offered.sort_by(|a, b| a.rate.as_f64().total_cmp(&b.rate.as_f64()));
        offered
    }
}

/// One participant: its connection and the tracks it sends and receives.
struct Member {
    id: Seq<Participant>,
    room: Validated<Room>,
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    tracks_in: Vec<TrackIn>,
    tracks_out: Vec<TrackOut>,
    /// When the connection asks to be woken.
    wake: Instant,
    /// The clock of the last input, which stamps what the events of that input start.
    now: Instant,
    /// The last estimate of the downlink of this participant.
    estimate: Option<Bitrate>,
    /// When this participant last spoke, from the audio level of its packets.
    spoke: Option<Instant>,
    /// Whether a track of this participant showed a layer the last choice did not know.
    new_layer: bool,
    /// Whether the SFU wrote to this connection since its last drain: a packet, an answer, a
    /// new track to offer, or new rates.
    touched: bool,
    /// The datagrams that reached this connection since the last poll.
    inbox: Vec<(Instant, SocketAddr, Vec<u8>)>,
    /// The clock of a timer due since the last poll.
    due: Option<Instant>,
    /// The datagrams this connection sends.
    outbox: VecDeque<Transmit>,
    /// What the events of this connection ask of the others.
    events: VecDeque<Propagated>,
    /// An offer to send to this participant.
    offer: Option<SdpOffer>,
}

/// What one participant's output asks of the others.
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every entry is a packet, and a box would allocate once per packet"
)]
enum Propagated {
    /// A track this participant sends opened.
    TrackOpen(Seq<Participant>, Mid, MediaKind),
    /// A packet of this participant, on the layer of the track of this mid.
    Packet(Seq<Participant>, Mid, Layer, PayloadParams, RtpPacket),
    /// A receiver asked the sender of this track for a keyframe: of one layer, or of every
    /// layer for none.
    Keyframe(Seq<Participant>, Mid, Option<Layer>, KeyframeRequestKind),
}

/// The SFU of one process: every participant of every room on one UDP address.
pub struct Sfu {
    local: SocketAddr,
    issuer: Issuer<Participant>,
    members: Vec<Member>,
    signals: VecDeque<(Seq<Participant>, Signal)>,
    propagated: VecDeque<Propagated>,
    /// When the SFU next chooses the layers of every receiver.
    next_choice: Option<Instant>,
    /// The participant each source address reached last, so a datagram skips the search.
    routes: HashMap<SocketAddr, Seq<Participant>>,
}

impl Sfu {
    /// An SFU with no participant, that receives media on `local`.
    pub fn new(local: SocketAddr) -> Self {
        Self {
            local,
            issuer: Issuer::default(),
            members: Vec::new(),
            signals: VecDeque::new(),
            propagated: VecDeque::new(),
            next_choice: None,
            routes: HashMap::new(),
        }
    }

    /// Admits the browser that sent `offer` into `room`, and answers it.
    ///
    /// The joiner receives every track the room carries already (`CALL-034`). The SFU offers
    /// them once the joiner has the answer.
    ///
    /// # Errors
    ///
    /// [`JoinError`] when str0m refuses the offer or the local address.
    pub fn join(
        &mut self,
        now: Instant,
        room: Validated<Room>,
        offer: SdpOffer,
    ) -> Result<(Seq<Participant>, SdpAnswer), JoinError> {
        let mut rtc = participant_config().set_rtp_mode(true).build(now);
        let host = Candidate::host(self.local, "udp").map_err(JoinError::Candidate)?;
        rtc.add_local_candidate(host);
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(JoinError::Offer)?;
        let tracks_out = self
            .members
            .iter()
            .filter(|m| m.room == room)
            .flat_map(|m| {
                m.tracks_in
                    .iter()
                    .map(|t| TrackOut::new(m.id, t.mid, t.kind))
            })
            .collect();
        let id = self.issuer.issue();
        self.members.push(Member {
            id,
            room,
            rtc,
            pending: None,
            tracks_in: Vec::new(),
            tracks_out,
            wake: now,
            now,
            estimate: None,
            spoke: None,
            new_layer: false,
            touched: true,
            inbox: Vec::new(),
            due: None,
            outbox: VecDeque::new(),
            events: VecDeque::new(),
            offer: None,
        });
        Ok((id, answer))
    }

    /// Applies the answer of participant `id` to the last offer the SFU sent it.
    ///
    /// Each video track the answer opens asks its sender for a keyframe, so that the receiver
    /// decodes from its first picture.
    pub fn accept_answer(&mut self, id: Seq<Participant>, answer: SdpAnswer) {
        let Some(member) = self.members.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let Some(pending) = member.pending.take() else {
            return;
        };
        member.touched = true;
        if member.rtc.sdp_api().accept_answer(pending, answer).is_err() {
            member.rtc.disconnect();
            return;
        }
        for track in &mut member.tracks_out {
            if let OutState::Negotiating(mid) = track.state {
                track.state = OutState::Open(mid);
                if track.kind == MediaKind::Video {
                    self.propagated.push_back(Propagated::Keyframe(
                        track.source,
                        track.source_mid,
                        None,
                        KeyframeRequestKind::Pli,
                    ));
                }
            }
        }
    }

    /// Ends the session of participant `id`.
    pub fn leave(&mut self, id: Seq<Participant>) {
        if let Some(member) = self.members.iter_mut().find(|m| m.id == id) {
            member.rtc.disconnect();
        }
    }

    /// Hands a datagram from `source` to the participant whose connection accepts it, and
    /// drops it when none does. The connection reads it at the next poll.
    pub fn receive(&mut self, now: Instant, source: SocketAddr, contents: &[u8]) {
        let Ok(datagram) = DatagramRecv::try_from(contents) else {
            return;
        };
        let input = Input::Receive(
            now,
            Receive {
                proto: Protocol::Udp,
                source,
                destination: self.local,
                contents: datagram,
            },
        );
        let known = self.routes.get(&source).copied();
        let index = known
            .and_then(|id| self.members.iter().position(|m| m.id == id))
            .filter(|i| self.members.get(*i).is_some_and(|m| m.rtc.accepts(&input)))
            .or_else(|| self.members.iter().position(|m| m.rtc.accepts(&input)));
        if let Some(member) = index.and_then(|i| self.members.get_mut(i)) {
            if known != Some(member.id) {
                self.routes.insert(source, member.id);
            }
            member.inbox.push((now, source, contents.to_vec()));
        }
    }

    /// Moves the clock of every connection whose timer is due to `now`, at the next poll.
    pub fn tick(&mut self, now: Instant) {
        for member in self.members.iter_mut().filter(|m| m.wake <= now) {
            member.due = Some(now);
        }
    }

    /// Forwards what the inputs so far ask, sends every datagram through `send`, and answers
    /// when the earliest connection must be woken.
    pub fn poll(&mut self, now: Instant, mut send: impl FnMut(Transmit)) -> Instant {
        if self.members.iter().any(|m| !m.rtc.is_alive()) {
            self.members.retain(|m| m.rtc.is_alive());
            let live: Vec<Seq<Participant>> = self.members.iter().map(|m| m.id).collect();
            for member in &mut self.members {
                member.tracks_out.retain(|t| live.contains(&t.source));
            }
            self.routes.retain(|_, id| live.contains(id));
        }
        self.work();
        // A new layer is chosen at once, so the first packets of a track reach the receivers.
        // Every flag is cleared, so the count reads them all where `any` would stop early.
        let fresh = self
            .members
            .iter_mut()
            .map(|m| std::mem::take(&mut m.new_layer))
            .filter(|seen| *seen)
            .count()
            > 0;
        if fresh || self.next_choice.is_none_or(|at| now >= at) {
            self.next_choice = Some(now + CHOICE_PERIOD);
            self.choose_layers(now);
        }
        while !self.propagated.is_empty() {
            while let Some(p) = self.propagated.pop_front() {
                self.propagate(p);
            }
            self.work();
        }
        for member in &mut self.members {
            member.outbox.drain(..).for_each(&mut send);
        }
        let soonest = self.members.iter().map(|m| m.wake).min();
        let idle = now + std::time::Duration::from_millis(100);
        soonest.map_or(idle, |at| at.min(idle))
    }

    /// Runs every connection that has work: its datagrams, its timer, what the SFU wrote to
    /// it. The connections share nothing, so they run on several threads, and what they ask
    /// of each other is gathered after.
    fn work(&mut self) {
        let local = self.local;
        let busy: Vec<&mut Member> = self.members.iter_mut().filter(|m| m.has_work()).collect();
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(WORKERS);
        if busy.len() < PARALLEL_FROM || threads < 2 {
            busy.into_iter().for_each(|m| m.work(local));
        } else {
            let per = busy.len().div_ceil(threads);
            let mut busy = busy;
            std::thread::scope(|scope| {
                while !busy.is_empty() {
                    let rest = busy.split_off(per.min(busy.len()));
                    let chunk = std::mem::replace(&mut busy, rest);
                    scope.spawn(move || chunk.into_iter().for_each(|m| m.work(local)));
                }
            });
        }
        for member in &mut self.members {
            self.propagated.extend(member.events.drain(..));
            if let Some(offer) = member.offer.take() {
                self.signals.push_back((member.id, Signal::Offer(offer)));
            }
        }
    }

    /// The next session description to send, and the participant it goes to.
    pub fn poll_signal(&mut self) -> Option<(Seq<Participant>, Signal)> {
        self.signals.pop_front()
    }

    /// Chooses again the layer of each video track each receiver gets, from its estimate and
    /// the last speech of each sender (`CALL-038`, `CALL-041`).
    fn choose_layers(&mut self, now: Instant) {
        let spoke: Vec<(Seq<Participant>, Option<Instant>)> =
            self.members.iter().map(|m| (m.id, m.spoke)).collect();
        let offered: Vec<(Seq<Participant>, Mid, Vec<Offered>)> = self
            .members
            .iter()
            .flat_map(|m| {
                m.tracks_in
                    .iter()
                    .filter(|t| t.kind == MediaKind::Video)
                    .map(|t| (m.id, t.mid, t.offered()))
            })
            .collect();
        let last_spoke =
            |id: Seq<Participant>| spoke.iter().find(|(s, _)| *s == id).and_then(|(_, at)| *at);
        let mut asks = Vec::new();
        for member in &mut self.members {
            let mut ranked: Vec<usize> = member
                .tracks_out
                .iter()
                .enumerate()
                .filter(|(_, t)| t.kind == MediaKind::Video && matches!(t.state, OutState::Open(_)))
                .map(|(index, _)| index)
                .collect();
            // The most recent speaker first; a sender who never spoke after every speaker, in
            // the order of the joins.
            ranked.sort_by_key(|index| {
                member
                    .tracks_out
                    .get(*index)
                    .map(|t| (std::cmp::Reverse(last_spoke(t.source)), t.source))
            });
            let choices: Vec<Choice> = ranked
                .iter()
                .filter_map(|index| member.tracks_out.get(*index))
                .map(|t| Choice {
                    layers: offered
                        .iter()
                        .find(|(s, m, _)| *s == t.source && *m == t.source_mid)
                        .map(|(_, _, o)| o.clone())
                        .unwrap_or_default(),
                    current: t.video.current,
                })
                .collect();
            let estimate = member.estimate.unwrap_or(INITIAL_ESTIMATE);
            let chosen = choose(estimate, &choices);
            let (minimum, desired) = rates(&choices, &chosen);
            for (index, layer) in ranked.iter().zip(chosen) {
                let Some(track) = member.tracks_out.get_mut(*index) else {
                    continue;
                };
                let video = &mut track.video;
                video.target = layer;
                if layer.is_none() {
                    video.current = None;
                }
                let waiting = layer.is_some() && video.current != layer;
                if waiting && video.asked.is_none_or(|at| now >= at + KEYFRAME_RETRY) {
                    video.asked = Some(now);
                    asks.push((track.source, track.source_mid, layer));
                }
            }
            member.rtc.bwe().set_bitrate(minimum, desired);
            member.touched = true;
        }
        for (source, mid, layer) in asks {
            self.propagated.push_back(Propagated::Keyframe(
                source,
                mid,
                layer,
                KeyframeRequestKind::Pli,
            ));
        }
    }

    /// Hands `p` to every participant it concerns.
    fn propagate(&mut self, p: Propagated) {
        let origin = match &p {
            Propagated::TrackOpen(id, ..) | Propagated::Packet(id, ..) => *id,
            Propagated::Keyframe(source, mid, layer, kind) => {
                if let Some(member) = self.members.iter_mut().find(|m| m.id == *source) {
                    member.request_keyframe(*mid, *layer, *kind);
                    member.touched = true;
                }
                return;
            }
        };
        let Some(room) = self
            .members
            .iter()
            .find(|m| m.id == origin)
            .map(|m| m.room.clone())
        else {
            return;
        };
        // Never back to the sender, never into another room (`CALL-033`).
        for member in self
            .members
            .iter_mut()
            .filter(|m| m.id != origin && m.room == room)
        {
            match &p {
                Propagated::TrackOpen(_, mid, kind) => {
                    member.tracks_out.push(TrackOut::new(origin, *mid, *kind));
                    member.touched = true;
                }
                Propagated::Packet(_, mid, layer, params, packet) => {
                    member.forward(origin, *mid, *layer, *params, packet);
                }
                Propagated::Keyframe(..) => {}
            }
        }
    }
}

impl Member {
    /// Whether the connection has an input to read or something new to send.
    fn has_work(&self) -> bool {
        self.touched || self.due.is_some() || !self.inbox.is_empty()
    }

    /// Reads the datagrams and the timer of the connection, then sends what the SFU wrote to
    /// it. str0m keeps one received packet until the next poll, so each input is drained
    /// before the next.
    fn work(&mut self, local: SocketAddr) {
        for (at, source, bytes) in std::mem::take(&mut self.inbox) {
            let Ok(contents) = DatagramRecv::try_from(&bytes[..]) else {
                continue;
            };
            let input = Input::Receive(
                at,
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local,
                    contents,
                },
            );
            self.handle(input);
        }
        if let Some(at) = self.due.take() {
            self.handle(Input::Timeout(at));
        }
        if std::mem::take(&mut self.touched) {
            if let Some(offer) = self.negotiate() {
                self.offer = Some(offer);
            }
            self.drain();
        }
    }

    /// Hands `input` to the connection, then drains it.
    fn handle(&mut self, input: Input) {
        let (Input::Timeout(at) | Input::Receive(at, _)) = &input;
        self.now = *at;
        if self.rtc.handle_input(input).is_err() {
            self.rtc.disconnect();
        }
        self.drain();
    }

    /// Runs the connection until it waits. Its datagrams go to its outbox. What its events
    /// ask of the others goes to its event queue.
    fn drain(&mut self) {
        while self.rtc.is_alive() {
            let Ok(output) = self.rtc.poll_output() else {
                self.rtc.disconnect();
                return;
            };
            match output {
                Output::Timeout(at) => {
                    self.wake = at;
                    return;
                }
                Output::Transmit(packet) => self.outbox.push_back(packet),
                Output::Event(event) => {
                    if let Some(p) = self.on_event(event) {
                        self.events.push_back(p);
                    }
                }
            }
        }
    }

    /// An offer for the tracks this participant does not receive yet, when no other offer
    /// waits for its answer.
    fn negotiate(&mut self) -> Option<SdpOffer> {
        if self.pending.is_some() || !self.tracks_out.iter().any(|t| t.state == OutState::ToOpen) {
            return None;
        }
        let mut change = self.rtc.sdp_api();
        let mut opened = Vec::new();
        for (index, track) in self.tracks_out.iter().enumerate() {
            if track.state == OutState::ToOpen {
                // The tracks of one sender share a stream, so the browser syncs their playout.
                // An SDP token holds no parenthesis, so the name keeps the alphanumerics alone.
                let name = format!("{:?}", track.source);
                let stream = Some(name.chars().filter(char::is_ascii_alphanumeric).collect());
                opened.push((
                    index,
                    change.add_media(track.kind, Direction::SendOnly, stream, None, None),
                ));
            }
        }
        let (offer, pending) = change.apply()?;
        for (index, mid) in opened {
            if let Some(track) = self.tracks_out.get_mut(index) {
                track.state = OutState::Negotiating(mid);
            }
        }
        self.pending = Some(pending);
        Some(offer)
    }

    /// What the event `event` of this participant asks of the others.
    fn on_event(&mut self, event: Event) -> Option<Propagated> {
        match event {
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                self.rtc.disconnect();
                None
            }
            Event::MediaAdded(added) if added.direction.is_receiving() => {
                self.tracks_in.push(TrackIn {
                    mid: added.mid,
                    kind: added.kind,
                    layers: Vec::new(),
                });
                Some(Propagated::TrackOpen(self.id, added.mid, added.kind))
            }
            Event::EgressBitrateEstimate(
                BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. },
            ) => {
                self.estimate = Some(estimate);
                None
            }
            Event::RtpPacket(packet) => {
                let pt = packet.header.payload_type;
                let (mid, rid) = {
                    let mut api = self.rtc.direct_api();
                    let stream = api.stream_rx(&packet.header.ssrc)?;
                    (stream.mid(), stream.rid())
                };
                let params = *self.rtc.codec_config().find(|p| p.pt() == pt)?;
                let layer = Layer::new(rid);
                let now = self.now;
                if packet
                    .header
                    .ext_vals
                    .audio_level
                    .is_some_and(|l| l > SPEECH_LEVEL)
                {
                    self.spoke = Some(now);
                }
                if let Some(track) = self.tracks_in.iter_mut().find(|t| t.mid == mid) {
                    match track.layers.iter_mut().find(|(l, _)| *l == layer) {
                        Some((_, meter)) => meter.count(now, &packet.payload),
                        None => {
                            let mut meter = Meter::new(now);
                            meter.count(now, &packet.payload);
                            track.layers.push((layer, meter));
                            self.new_layer = true;
                        }
                    }
                }
                Some(Propagated::Packet(self.id, mid, layer, params, packet))
            }
            Event::KeyframeRequest(request) => {
                let track = self
                    .tracks_out
                    .iter()
                    .find(|t| t.state == OutState::Open(request.mid))?;
                Some(Propagated::Keyframe(
                    track.source,
                    track.source_mid,
                    track.video.current,
                    request.kind,
                ))
            }
            _ => None,
        }
    }

    /// Asks this participant for a keyframe on its track `mid`: of `layer`, or of every
    /// layer for none.
    fn request_keyframe(&mut self, mid: Mid, layer: Option<Layer>, kind: KeyframeRequestKind) {
        let rids: Vec<Option<Rid>> = match layer {
            Some(layer) => vec![layer.get()],
            None => self
                .tracks_in
                .iter()
                .find(|t| t.mid == mid)
                .map(|t| t.layers.iter().map(|(layer, _)| layer.get()).collect())
                .filter(|rids: &Vec<_>| !rids.is_empty())
                .unwrap_or_else(|| vec![None]),
        };
        let mut api = self.rtc.direct_api();
        for rid in rids {
            if let Some(stream) = api.stream_rx_by_mid(mid, rid) {
                stream.request_keyframe(kind);
            }
        }
    }

    /// Writes `packet`, sent by `source` on the layer `layer` of its track `mid`, to this
    /// participant (`CALL-032`). The payload and the marker stay as the sender wrote them. An
    /// audio track keeps the 16 bits of each sequence number and each timestamp. A video track
    /// shifts them across a layer switch (`CALL-040`).
    fn forward(
        &mut self,
        source: Seq<Participant>,
        mid: Mid,
        layer: Layer,
        params: PayloadParams,
        packet: &RtpPacket,
    ) {
        let now = self.now;
        // str0m holds a packet written before the connection is up. It sends the backlog in
        // one burst at connect, and the jitter buffer of the receiver then holds seconds.
        if !self.rtc.is_connected() {
            return;
        }
        let Some(track) = self
            .tracks_out
            .iter_mut()
            .find(|t| t.source == source && t.source_mid == mid)
        else {
            return;
        };
        let OutState::Open(out_mid) = track.state else {
            return;
        };
        // Most packets belong to a layer this receiver does not get: they stop here.
        if track.kind == MediaKind::Video
            && track.video.current != Some(layer)
            && track.video.target != Some(layer)
        {
            return;
        }
        let Some(pt) = self.rtc.codec_config().match_params(params).map(|p| p.pt()) else {
            return;
        };
        let (seq, time, patch) = if track.kind == MediaKind::Video {
            let video = &mut track.video;
            if video.target == Some(layer) && video.current != Some(layer) {
                // A layer starts on one of its keyframes, so the receiver decodes on from it
                // (`CALL-039`). A codec whose keyframe the SFU cannot read starts anywhere.
                let starts = params.spec().codec != Codec::Vp8
                    || Vp8Descriptor::parse(&packet.payload)
                        .is_ok_and(|d| d.starts_keyframe(&packet.payload));
                if starts {
                    video.current = Some(layer);
                    video.asked = None;
                    video.rewrite.switch(now, packet);
                }
            }
            if video.current != Some(layer) {
                return;
            }
            let Some(out) = video.rewrite.apply(now, packet) else {
                return;
            };
            (out.seq, out.time, out.patch)
        } else {
            let rollover = *track
                .rollover
                .get_or_insert(SeqNo::from(*packet.seq_no & !0xffff));
            (
                SeqNo::from(packet.seq_no.saturating_sub(*rollover)),
                packet.header.timestamp,
                None,
            )
        };
        let mut api = self.rtc.direct_api();
        let Some(stream) = api.stream_tx_by_mid(out_mid, None) else {
            return;
        };
        let mut write = RtpWrite::new(pt, seq, time, packet.timestamp, packet.payload.clone())
            .marker(packet.header.marker)
            .ext_vals(packet.header.ext_vals.clone())
            // The SFU answers a receiver's NACK from its own cache, so a loss on one downlink
            // costs the sender nothing.
            .nackable(track.kind == MediaKind::Video);
        if let Some(patch) = patch {
            write = write.vp8_patch(patch);
        }
        stream.write_rtp(write);
        self.touched = true;
    }
}

#[cfg(test)]
mod tests;
