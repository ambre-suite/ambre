//! The forwarding core: one str0m connection per participant, and the packets between them.
//!
//! The core does no I/O. A driver hands it each datagram and each tick, and sends what it
//! answers (`serve.rs` over a UDP socket, the tests over an in-memory link). It forwards in RTP
//! mode: it copies the payload of each packet, and never reassembles a frame (`CALL-032`).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Instant;

use ambre_core::{Issuer, Rule, Seq, SeqKind, Validated};
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::error::IceError;
use str0m::format::PayloadParams;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{DatagramRecv, Protocol, Receive, Transmit};
use str0m::rtp::{RtpPacket, RtpWrite, SeqNo};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcError};

use crate::participant_config;

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
    /// The rollover part of the first sequence number forwarded. The receiver starts its
    /// SRTP rollover counter at zero. So the SFU subtracts that part, and the 16 bits of the
    /// header stay as the sender wrote them.
    rollover: Option<SeqNo>,
}

/// One track a participant sends.
#[derive(Debug)]
struct TrackIn {
    mid: Mid,
    kind: MediaKind,
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
}

/// What one participant's output asks of the others.
#[expect(
    clippy::large_enum_variant,
    reason = "nearly every entry is a packet, and a box would allocate once per packet"
)]
enum Propagated {
    /// A track this participant sends opened.
    TrackOpen(Seq<Participant>, Mid, MediaKind),
    /// A packet of this participant, on the track of this mid.
    Packet(Seq<Participant>, Mid, PayloadParams, RtpPacket),
    /// A receiver asked the sender of this track for a keyframe.
    Keyframe(Seq<Participant>, Mid, KeyframeRequestKind),
}

/// The SFU of one process: every participant of every room on one UDP address.
pub struct Sfu {
    local: SocketAddr,
    issuer: Issuer<Participant>,
    members: Vec<Member>,
    signals: VecDeque<(Seq<Participant>, Signal)>,
    propagated: VecDeque<Propagated>,
    outbox: VecDeque<Transmit>,
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
            outbox: VecDeque::new(),
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
                m.tracks_in.iter().map(|t| TrackOut {
                    source: m.id,
                    source_mid: t.mid,
                    kind: t.kind,
                    state: OutState::ToOpen,
                    rollover: None,
                })
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
    /// drops it when none does.
    pub fn receive(&mut self, now: Instant, source: SocketAddr, contents: DatagramRecv<'_>) {
        let input = Input::Receive(
            now,
            Receive {
                proto: Protocol::Udp,
                source,
                destination: self.local,
                contents,
            },
        );
        if let Some(member) = self.members.iter_mut().find(|m| m.rtc.accepts(&input)) {
            member.handle(input, &mut self.outbox, &mut self.propagated);
        }
    }

    /// Moves the clock of every connection to `now`.
    pub fn tick(&mut self, now: Instant) {
        for member in &mut self.members {
            member.handle(Input::Timeout(now), &mut self.outbox, &mut self.propagated);
        }
    }

    /// Forwards what the inputs so far ask, sends every datagram through `send`, and answers
    /// when the earliest connection must be woken.
    pub fn poll(&mut self, now: Instant, mut send: impl FnMut(Transmit)) -> Instant {
        self.members.retain(|m| m.rtc.is_alive());
        let live: Vec<Seq<Participant>> = self.members.iter().map(|m| m.id).collect();
        for member in &mut self.members {
            member.tracks_out.retain(|t| live.contains(&t.source));
        }
        loop {
            for member in &mut self.members {
                if let Some(offer) = member.negotiate() {
                    self.signals.push_back((member.id, Signal::Offer(offer)));
                }
                member.drain(&mut self.outbox, &mut self.propagated);
            }
            if self.propagated.is_empty() {
                break;
            }
            while let Some(p) = self.propagated.pop_front() {
                self.propagate(p);
            }
        }
        self.outbox.drain(..).for_each(&mut send);
        let soonest = self.members.iter().map(|m| m.wake).min();
        let idle = now + std::time::Duration::from_millis(100);
        soonest.map_or(idle, |at| at.min(idle))
    }

    /// The next session description to send, and the participant it goes to.
    pub fn poll_signal(&mut self) -> Option<(Seq<Participant>, Signal)> {
        self.signals.pop_front()
    }

    /// Hands `p` to every participant it concerns.
    fn propagate(&mut self, p: Propagated) {
        let origin = match &p {
            Propagated::TrackOpen(id, ..) | Propagated::Packet(id, ..) => *id,
            Propagated::Keyframe(source, mid, kind) => {
                if let Some(member) = self.members.iter_mut().find(|m| m.id == *source)
                    && let Some(stream) = member.rtc.direct_api().stream_rx_by_mid(*mid, None)
                {
                    stream.request_keyframe(*kind);
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
                Propagated::TrackOpen(_, mid, kind) => member.tracks_out.push(TrackOut {
                    source: origin,
                    source_mid: *mid,
                    kind: *kind,
                    state: OutState::ToOpen,
                    rollover: None,
                }),
                Propagated::Packet(_, mid, params, packet) => {
                    member.forward(origin, *mid, *params, packet);
                }
                Propagated::Keyframe(..) => {}
            }
        }
    }
}

impl Member {
    /// Hands `input` to the connection, then drains it. str0m keeps one received packet until
    /// the next poll, so no second input may come first.
    fn handle(
        &mut self,
        input: Input,
        outbox: &mut VecDeque<Transmit>,
        propagated: &mut VecDeque<Propagated>,
    ) {
        if self.rtc.handle_input(input).is_err() {
            self.rtc.disconnect();
        }
        self.drain(outbox, propagated);
    }

    /// Runs the connection until it waits. Its datagrams go to the outbox. What its events ask
    /// of the others goes to the propagation queue.
    fn drain(&mut self, outbox: &mut VecDeque<Transmit>, propagated: &mut VecDeque<Propagated>) {
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
                Output::Transmit(packet) => outbox.push_back(packet),
                Output::Event(event) => {
                    if let Some(p) = self.on_event(event) {
                        propagated.push_back(p);
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
                });
                Some(Propagated::TrackOpen(self.id, added.mid, added.kind))
            }
            Event::RtpPacket(packet) => {
                let pt = packet.header.payload_type;
                let mid = self.rtc.direct_api().stream_rx(&packet.header.ssrc)?.mid();
                let params = *self.rtc.codec_config().find(|p| p.pt() == pt)?;
                Some(Propagated::Packet(self.id, mid, params, packet))
            }
            Event::KeyframeRequest(request) => {
                let track = self
                    .tracks_out
                    .iter()
                    .find(|t| t.state == OutState::Open(request.mid))?;
                Some(Propagated::Keyframe(
                    track.source,
                    track.source_mid,
                    request.kind,
                ))
            }
            _ => None,
        }
    }

    /// Writes `packet`, sent by `source` on its track `mid`, to this participant (`CALL-032`).
    /// The payload, the marker, the timestamp and the 16 bits of the sequence number stay as
    /// the sender wrote them.
    fn forward(
        &mut self,
        source: Seq<Participant>,
        mid: Mid,
        params: PayloadParams,
        packet: &RtpPacket,
    ) {
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
        let Some(pt) = self.rtc.codec_config().match_params(params).map(|p| p.pt()) else {
            return;
        };
        let rollover = *track
            .rollover
            .get_or_insert(SeqNo::from(*packet.seq_no & !0xffff));
        let mut api = self.rtc.direct_api();
        let Some(stream) = api.stream_tx_by_mid(out_mid, None) else {
            return;
        };
        let write = RtpWrite::new(
            pt,
            SeqNo::from(packet.seq_no.saturating_sub(*rollover)),
            packet.header.timestamp,
            packet.timestamp,
            packet.payload.clone(),
        )
        .marker(packet.header.marker)
        .ext_vals(packet.header.ext_vals.clone())
        // The SFU answers a receiver's NACK from its own cache, so a loss on one downlink
        // costs the sender nothing.
        .nackable(track.kind == MediaKind::Video);
        stream.write_rtp(write);
    }
}

#[cfg(test)]
mod tests;
