//! The forwarding core: one str0m connection per participant, and the packets between them.
//!
//! The core does no I/O. A driver hands it each datagram and each tick, and sends what it
//! answers (`serve.rs` over a UDP socket, the tests over an in-memory link). It forwards in RTP
//! mode: it copies the payload of each packet, and never reassembles a frame (`CALL-032`).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ambre_core::{Count, CountKind, Issuer, Payload, PayloadKind, Rule, Seq, SeqKind, Validated};
use str0m::bwe::Bitrate;
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::error::IceError;
use str0m::format::{Codec, PayloadParams};
use str0m::media::Rid;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid};
use str0m::net::{DatagramRecv, Protocol, Receive, Transmit};
use str0m::rtp::{RtpPacket, RtpWrite, Vp8Descriptor};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcError};

use crate::congestion::Congestion;
use crate::layer::{AUDIO_SLOTS, Candidate as Choice, Layer, Meter, Offered, choose, rates};
use crate::layer::{SCREEN_SLOTS, Slots, VIDEOS_PER_RECEIVER};
use crate::participant_config;
use crate::rewrite::{Clock, Rewrite};

/// How many threads run the connections of one poll.
enum Threads {}

impl CountKind for Threads {
    type Repr = u32;
    const NAME: &'static str = "ThreadCount";
}

/// How many connections have work in one poll.
enum Connections {}

impl CountKind for Connections {
    type Repr = u32;
    const NAME: &'static str = "ConnectionCount";
}

/// The bytes of one datagram the socket read, before str0m parses them.
pub enum Datagram {}

impl PayloadKind for Datagram {
    const NAME: &'static str = "Datagram";
}

/// The most threads that run the connections of one poll.
const WORKERS: Count<Threads> = Count::new(4);

/// The count of busy connections from which a poll spreads them over threads. Below it, the
/// start of a thread costs more than it saves.
const PARALLEL_FROM: Count<Connections> = Count::new(8);

/// How often the SFU chooses the layers of every receiver again.
const CHOICE_PERIOD: Duration = Duration::from_millis(100);

/// How long the SFU waits for a keyframe of a layer before it asks the sender again.
const KEYFRAME_RETRY: Duration = Duration::from_millis(300);

/// The audio level of RFC 6464, in -dBov, above which a packet counts as speech.
const SPEECH_LEVEL: i8 = -50;

/// The longest pause in the packets of a screen within one share, from the call screen row of
/// `ambre-spec-budgets.md section 1`. The next packet after a longer pause starts a share
/// (`CALL-106`).
const SHARE_GAP: Duration = Duration::from_secs(2);

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

/// What one slot of a receiver has reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutState {
    /// No offer names it yet.
    ToOpen,
    /// An offer names it under this mid, and the answer has not come back.
    Negotiating(Mid),
    /// The answer came back: the slot flows under this mid.
    Open(Mid),
}

/// One track of another participant, at one layer: what a slot carries. An audio track has
/// one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Feed {
    source: Seq<Participant>,
    mid: Mid,
    layer: Layer,
}

impl Feed {
    /// Whether this feed comes from the track `mid` of `source`, at any layer.
    fn is_track(&self, source: Seq<Participant>, mid: Mid) -> bool {
        self.source == source && self.mid == mid
    }
}

/// Whether the current feeds of a receiver take more than the target of its congestion control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Congested {
    No,
    Yes,
}

/// Whether a slot holds back its current feed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Hold {
    /// The current feed flows.
    #[default]
    Flowing,
    /// The target is a lower layer of the same track, and the downlink is congested. The current
    /// layer stops: its packets would queue in the pacer, ahead of the lower layer.
    Lowering,
}

/// What a track of a participant carries, from the order of the sections of its offer: its
/// audio, then its camera, then its screen (`CALL-104`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Audio,
    Camera,
    Screen,
    /// A video section past the screen, which the SFU forwards to no one.
    Unused,
}

impl Role {
    /// The role of a new section of `kind`, after the sections `before` of the same offer.
    fn of(kind: MediaKind, before: &[TrackIn]) -> Self {
        if kind == MediaKind::Audio {
            return Self::Audio;
        }
        let videos = before.iter().filter(|t| t.kind == MediaKind::Video).count();
        match videos {
            0 => Self::Camera,
            1 => Self::Screen,
            _ => Self::Unused,
        }
    }

    /// The kind of media a slot of this role carries.
    fn kind(self) -> MediaKind {
        if self == Self::Audio {
            MediaKind::Audio
        } else {
            MediaKind::Video
        }
    }
}

/// One outgoing track of a receiver, which carries the track of one other participant at a
/// time (`CALL-045`). A change of speaker remaps a slot, and never renegotiates (`CALL-046`).
#[derive(Debug)]
struct Slot {
    /// Audio, a camera or the screen. A screen travels over the screen slot alone (`CALL-104`).
    role: Role,
    kind: MediaKind,
    state: OutState,
    /// The feed whose packets flow to the receiver.
    current: Option<Feed>,
    /// The feed the choice wants. A video slot switches to it on one of its keyframes
    /// (`CALL-039`, `CALL-049`), an audio slot at its next packet.
    target: Option<Feed>,
    /// When the SFU last asked the sender of the target for a keyframe.
    asked: Option<Instant>,
    hold: Hold,
    rewrite: Rewrite,
}

impl Slot {
    fn new(role: Role) -> Self {
        let kind = role.kind();
        let clock = if kind == MediaKind::Audio {
            Clock::Audio
        } else {
            Clock::Video
        };
        Self {
            role,
            kind,
            state: OutState::ToOpen,
            current: None,
            target: None,
            asked: None,
            hold: Hold::Flowing,
            rewrite: Rewrite::new(clock),
        }
    }

    /// Whether the slot carries, or waits for, the track `mid` of `source`.
    fn carries(&self, source: Seq<Participant>, mid: Mid) -> bool {
        self.current.is_some_and(|f| f.is_track(source, mid))
            || self.target.is_some_and(|f| f.is_track(source, mid))
    }
}

/// Places the feeds `wanted` into the slots `slots` of one kind. A feed whose track a slot
/// carries already stays in that slot, so its picture or its voice does not move. A new feed
/// takes a slot whose track nobody wants. A slot left without a feed stops.
fn assign(slots: &mut [&mut Slot], wanted: &[Feed]) {
    let mut placed = vec![false; wanted.len()];
    let mut kept = vec![false; slots.len()];
    for (slot, keep) in slots.iter_mut().zip(kept.iter_mut()) {
        let found = wanted
            .iter()
            .zip(placed.iter_mut())
            .find(|(feed, done)| !**done && slot.carries(feed.source, feed.mid));
        if let Some((feed, done)) = found {
            *done = true;
            *keep = true;
            slot.target = Some(*feed);
        }
    }
    let mut free = slots
        .iter_mut()
        .zip(kept)
        .filter(|(_, keep)| !keep)
        .map(|(slot, _)| slot);
    for (feed, _) in wanted.iter().zip(placed).filter(|(_, done)| !done) {
        if let Some(slot) = free.next() {
            slot.target = Some(*feed);
        }
    }
    for slot in free {
        slot.target = None;
        slot.current = None;
    }
}

/// One track of the call, as one layer choice reads it.
struct Track {
    source: Seq<Participant>,
    room: Validated<Room>,
    /// When its sender last spoke.
    spoke: Option<Instant>,
    mid: Mid,
    role: Role,
    /// When its current share started, for a screen that has sent a packet.
    started: Option<Instant>,
    /// Its layers, from the lowest measured bitrate up.
    offered: Vec<Offered>,
}

/// One track a participant sends.
#[derive(Debug)]
struct TrackIn {
    mid: Mid,
    kind: MediaKind,
    role: Role,
    /// When the current share of a screen started: its first packet after a pause longer than
    /// [`SHARE_GAP`] (`CALL-106`).
    started: Option<Instant>,
    /// When the last packet of the track arrived.
    last: Option<Instant>,
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
    /// The fixed audio and video slots of this receiver, empty until its room carries a track
    /// of another participant.
    slots: Vec<Slot>,
    /// Whether the room carries a track of another participant, so the slots should open.
    wants_slots: bool,
    /// When the connection asks to be woken.
    wake: Instant,
    /// The clock of the last input, which stamps what the events of that input start.
    now: Instant,
    /// The congestion control of the downlink of this participant (`CALL-044`).
    congestion: Congestion,
    /// When this participant last spoke, from the audio level of its packets.
    spoke: Option<Instant>,
    /// Whether a track of this participant showed a layer the last choice did not know.
    new_layer: bool,
    /// Whether the congestion control of this participant lowered its target, or entered or
    /// left overuse, since the last choice.
    rechoose: bool,
    /// Whether the SFU wrote to this connection since its last drain: a packet, an answer, a
    /// new track to offer, or new rates.
    touched: bool,
    /// The datagrams that reached this connection since the last poll.
    inbox: Vec<(Instant, SocketAddr, Payload<Datagram>)>,
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
    TrackOpen(Seq<Participant>),
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
    /// The most threads that run the connections of one poll.
    workers: Count<Threads>,
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
            workers: WORKERS,
        }
    }

    /// An SFU that runs every connection on the thread of its poll. str0m draws its random
    /// numbers from a generator per thread, and the simulation seeds the one of its thread.
    #[cfg(test)]
    pub fn serial(local: SocketAddr) -> Self {
        Self {
            workers: Count::new(1),
            ..Self::new(local)
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
        // The congestion control of the SFU drives the pacer and the padding of the connection,
        // never str0m's estimate.
        rtc.bwe().set_estimate(Some(INITIAL_ESTIMATE));
        let host = Candidate::host(self.local, "udp").map_err(JoinError::Candidate)?;
        rtc.add_local_candidate(host);
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(JoinError::Offer)?;
        let wants_slots = self
            .members
            .iter()
            .any(|m| m.room == room && !m.tracks_in.is_empty());
        let id = self.issuer.issue();
        self.members.push(Member {
            id,
            room,
            rtc,
            pending: None,
            tracks_in: Vec::new(),
            slots: Vec::new(),
            wants_slots,
            wake: now,
            now,
            congestion: Congestion::new(INITIAL_ESTIMATE),
            spoke: None,
            new_layer: false,
            rechoose: false,
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
    /// The layer choice then fills the slots the answer opens. It asks the sender of each video
    /// for a keyframe, so that the receiver decodes from its first picture.
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
        for slot in &mut member.slots {
            if let OutState::Negotiating(mid) = slot.state {
                slot.state = OutState::Open(mid);
            }
        }
        // The slots fill at the next poll, not 100 ms later.
        self.next_choice = None;
    }

    /// Ends the session of participant `id`.
    pub fn leave(&mut self, id: Seq<Participant>) {
        if let Some(member) = self.members.iter_mut().find(|m| m.id == id) {
            member.rtc.disconnect();
        }
    }

    /// Hands a datagram from `source` to the participant whose connection accepts it, and
    /// drops it when none does. The connection reads it at the next poll.
    pub fn receive(&mut self, now: Instant, source: SocketAddr, contents: Payload<Datagram>) {
        let Ok(datagram) = DatagramRecv::try_from(contents.as_bytes()) else {
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
            member.inbox.push((now, source, contents));
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
            let gone = |f: &Option<Feed>| f.is_some_and(|f| !live.contains(&f.source));
            for slot in self.members.iter_mut().flat_map(|m| m.slots.iter_mut()) {
                if gone(&slot.current) {
                    slot.current = None;
                }
                if gone(&slot.target) {
                    slot.target = None;
                }
            }
            self.routes.retain(|_, id| live.contains(id));
        }
        self.work();
        // A new layer is chosen at once, so the first packets of a track reach the receivers.
        // Every flag is cleared, so the count reads them all where `any` would stop early.
        let fresh = self
            .members
            .iter_mut()
            .map(|m| std::mem::take(&mut m.new_layer) | std::mem::take(&mut m.rechoose))
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
        let threads = Count::<Threads>::of(
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        )
        .min(self.workers);
        if Count::<Connections>::of(busy.len()) < PARALLEL_FROM || threads < Count::new(2) {
            busy.into_iter().for_each(|m| m.work(local));
        } else {
            let per = busy
                .len()
                .div_ceil(usize::try_from(threads.get()).unwrap_or(usize::MAX));
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
            self.propagated.append(&mut member.events);
            if let Some(offer) = member.offer.take() {
                self.signals.push_back((member.id, Signal::Offer(offer)));
            }
        }
    }

    #[cfg(test)]
    pub fn estimate_of(&self, id: Seq<Participant>) -> Option<Bitrate> {
        self.members
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.congestion.target())
    }

    /// The next session description to send, and the participant it goes to.
    pub fn poll_signal(&mut self) -> Option<(Seq<Participant>, Signal)> {
        self.signals.pop_front()
    }

    /// Chooses again what each slot of each receiver carries: the most recent speakers. The
    /// target of the congestion control of the receiver sets the layer of each video
    /// (`CALL-038`, `CALL-041`, `CALL-047`).
    fn choose_layers(&mut self, now: Instant) {
        // Each track of the call, with its sender's room and last speech.
        let tracks: Vec<Track> = self
            .members
            .iter()
            .flat_map(|m| {
                m.tracks_in.iter().map(|t| Track {
                    source: m.id,
                    room: m.room.clone(),
                    spoke: m.spoke,
                    mid: t.mid,
                    role: t.role,
                    started: t.started,
                    offered: t.offered(),
                })
            })
            .collect();
        let mut asks = Vec::new();
        for member in &mut self.members {
            if member.slots.is_empty() {
                continue;
            }
            // The tracks of the others of the room, the most recent speaker first. A sender who
            // never spoke comes after every speaker, in the order of the joins.
            let mut ranked: Vec<&Track> = tracks
                .iter()
                .filter(|t| t.source != member.id && t.room == member.room)
                .collect();
            ranked.sort_by_key(|t| (std::cmp::Reverse(t.spoke), t.source));
            // The screen of the room: the share that started last, at its one encoding, whoever
            // speaks and whatever the estimate (`CALL-105`, `CALL-106`). Its sharer gets none
            // (`CALL-111`).
            let presenter = tracks
                .iter()
                .filter(|t| t.role == Role::Screen && t.room == member.room)
                .filter_map(|t| t.started.map(|at| (at, t)))
                .max_by_key(|(at, t)| (*at, std::cmp::Reverse(t.source)))
                .map(|(_, t)| t);
            let shown = presenter.filter(|t| t.source != member.id).and_then(|t| {
                t.offered.first().map(|o| {
                    (
                        Feed {
                            source: t.source,
                            mid: t.mid,
                            layer: o.layer,
                        },
                        o.rate,
                    )
                })
            });
            let screen: Vec<Feed> = shown.iter().map(|(feed, _)| *feed).collect();
            let screen_rate = shown.map_or(Bitrate::bps(0), |(_, rate)| rate);
            let audio: Vec<Feed> = ranked
                .iter()
                .filter(|t| t.role == Role::Audio)
                .take(usize::try_from(AUDIO_SLOTS.get()).unwrap_or(usize::MAX))
                .map(|t| Feed {
                    source: t.source,
                    mid: t.mid,
                    layer: Layer::new(None),
                })
                .collect();
            let videos: Vec<&Track> = ranked
                .iter()
                .copied()
                .filter(|t| t.role == Role::Camera)
                .collect();
            let choices: Vec<Choice> = videos
                .iter()
                .map(|t| Choice {
                    layers: t.offered.clone(),
                    current: member
                        .slots
                        .iter()
                        .filter_map(|s| s.current)
                        .find(|f| f.is_track(t.source, t.mid))
                        .map(|f| f.layer),
                })
                .collect();
            // The cameras share what the screen leaves of the estimate (`CALL-105`).
            let left = Bitrate::from(
                (member.congestion.target().as_f64() - screen_rate.as_f64()).max(0.0),
            );
            let chosen = choose(left, &choices);
            let rates = rates(&choices, &chosen);
            let video: Vec<Feed> = videos
                .iter()
                .zip(&chosen)
                .filter_map(|(t, layer)| {
                    layer.map(|layer| Feed {
                        source: t.source,
                        mid: t.mid,
                        layer,
                    })
                })
                .take(usize::try_from(VIDEOS_PER_RECEIVER.get()).unwrap_or(usize::MAX))
                .collect();
            for (role, wanted) in [
                (Role::Audio, &audio),
                (Role::Camera, &video),
                (Role::Screen, &screen),
            ] {
                let mut slots: Vec<&mut Slot> = member
                    .slots
                    .iter_mut()
                    .filter(|s| s.role == role && matches!(s.state, OutState::Open(_)))
                    .collect();
                assign(&mut slots, wanted);
            }
            // The rate the current feeds take. Above the target, the downlink is congested, and a
            // lower layer holds the current one back. A drop that only follows a change of
            // speaker keeps the current layer flowing until the lower keyframe.
            let current_rate: f64 = member
                .slots
                .iter()
                .filter_map(|s| s.current)
                .filter_map(|f| {
                    videos
                        .iter()
                        .find(|t| t.source == f.source && t.mid == f.mid)
                        .and_then(|t| t.offered.iter().find(|o| o.layer == f.layer))
                        .map(|o| o.rate.as_f64())
                })
                .sum::<f64>()
                + screen_rate.as_f64();
            let congested = if current_rate > member.congestion.target().as_f64() {
                Congested::Yes
            } else {
                Congested::No
            };
            for slot in member
                .slots
                .iter_mut()
                .filter(|s| s.kind == MediaKind::Video)
            {
                let Some(target) = slot.target else {
                    slot.hold = Hold::Flowing;
                    continue;
                };
                // A lower layer of the same track holds the current layer back.
                let offered = videos
                    .iter()
                    .find(|t| t.source == target.source && t.mid == target.mid)
                    .map(|t| t.offered.as_slice())
                    .unwrap_or_default();
                let rank = |feed: Option<Feed>| {
                    let feed = feed.filter(|f| f.is_track(target.source, target.mid))?;
                    offered.iter().position(|o| o.layer == feed.layer)
                };
                let lowering = match (rank(Some(target)), rank(slot.current)) {
                    (Some(to), Some(from)) => to < from,
                    _ => false,
                };
                slot.hold = match (slot.hold, lowering, congested) {
                    (_, true, Congested::Yes) | (Hold::Lowering, true, Congested::No) => {
                        Hold::Lowering
                    }
                    (Hold::Lowering, false, _) => {
                        // The held layer missed pictures, so it resumes on a keyframe alone,
                        // and its sequence numbers continue from the last packet written
                        // (`CALL-040`).
                        slot.current = None;
                        Hold::Flowing
                    }
                    (Hold::Flowing, _, _) => Hold::Flowing,
                };
                if slot.current != Some(target)
                    && slot.asked.is_none_or(|at| now >= at + KEYFRAME_RETRY)
                {
                    slot.asked = Some(now);
                    asks.push(target);
                }
            }
            // Under overuse, the padding would hold the queue of the bottleneck full, and the
            // keyframe of the lower layer would wait behind it.
            let minimum = if member.congestion.congested() {
                Bitrate::bps(0)
            } else {
                rates.minimum
            };
            // A probe pads above the next layer step for a short span (`CALL-050`).
            member.congestion.probe(rates.probe, now);
            let pacing = member.congestion.pacing(now);
            // The pacer carries the screen beside the cameras.
            let (minimum, desired) = (minimum + screen_rate, rates.desired + screen_rate);
            let (minimum, desired) = match member.congestion.padding(now) {
                Some(pace) => (minimum.max(pace), desired.max(pace)),
                None => (minimum, desired),
            };
            let mut bwe = member.rtc.bwe();
            bwe.set_estimate(Some(pacing));
            bwe.set_bitrate(minimum, desired);
            member.touched = true;
        }
        for feed in asks {
            self.propagated.push_back(Propagated::Keyframe(
                feed.source,
                feed.mid,
                Some(feed.layer),
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
                Propagated::TrackOpen(..) => {
                    if !member.wants_slots {
                        member.wants_slots = true;
                        member.touched = true;
                    }
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
            let Ok(contents) = DatagramRecv::try_from(bytes.as_bytes()) else {
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

    /// The one offer of the slots of this participant, once its room carries a track of
    /// another participant (`CALL-045`). A later change of speaker remaps the slots and
    /// renegotiates nothing (`CALL-046`).
    fn negotiate(&mut self) -> Option<SdpOffer> {
        if self.pending.is_some() || !self.wants_slots || !self.slots.is_empty() {
            return None;
        }
        let mut change = self.rtc.sdp_api();
        let slots_of = |count: Count<Slots>| usize::try_from(count.get()).unwrap_or(usize::MAX);
        // The screen slot comes last, so a page reads it after the nine video slots.
        let roles = std::iter::repeat_n(Role::Audio, slots_of(AUDIO_SLOTS))
            .chain(std::iter::repeat_n(
                Role::Camera,
                slots_of(VIDEOS_PER_RECEIVER),
            ))
            .chain(std::iter::repeat_n(Role::Screen, slots_of(SCREEN_SLOTS)));
        let mut slots = Vec::new();
        for role in roles {
            let mut slot = Slot::new(role);
            let mid = change.add_media(role.kind(), Direction::SendOnly, None, None, None);
            slot.state = OutState::Negotiating(mid);
            slots.push(slot);
        }
        let (offer, pending) = change.apply()?;
        self.slots = slots;
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
                let role = Role::of(added.kind, &self.tracks_in);
                self.tracks_in.push(TrackIn {
                    mid: added.mid,
                    kind: added.kind,
                    role,
                    started: None,
                    last: None,
                    layers: Vec::new(),
                });
                Some(Propagated::TrackOpen(self.id))
            }
            Event::EgressTwccFeedback(acks) => {
                let before = (
                    self.congestion.target(),
                    self.congestion.congested(),
                    self.congestion.probing(),
                );
                self.congestion.on_feedback(&acks);
                let pacing = self.congestion.pacing(self.now);
                self.rtc.bwe().set_estimate(Some(pacing));
                // A lower target, a change of overuse or the end of a probe takes effect at
                // once. A congested receiver then drops its layer and its padding before the
                // next period, and a probe that passed climbs.
                let (target, congested) = (self.congestion.target(), self.congestion.congested());
                if target.as_f64() < before.0.as_f64()
                    || congested != before.1
                    || self.congestion.probing() != before.2
                {
                    self.rechoose = true;
                }
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
                    // A packet after a pause longer than the gap starts a share, and the screen
                    // of the room moves to it at once (`CALL-106`).
                    if track.role == Role::Screen
                        && track
                            .last
                            .is_none_or(|last| now.saturating_duration_since(last) > SHARE_GAP)
                    {
                        track.started = Some(now);
                        self.rechoose = true;
                    }
                    track.last = Some(now);
                    match track.layers.iter_mut().find(|(l, _)| *l == layer) {
                        Some((_, meter)) => meter.count(now, Count::of(packet.payload.len())),
                        None => {
                            let mut meter = Meter::new(now);
                            meter.count(now, Count::of(packet.payload.len()));
                            track.layers.push((layer, meter));
                            self.new_layer = true;
                        }
                    }
                }
                Some(Propagated::Packet(self.id, mid, layer, params, packet))
            }
            Event::KeyframeRequest(request) => {
                let feed = self
                    .slots
                    .iter()
                    .find(|s| s.state == OutState::Open(request.mid))?
                    .current?;
                Some(Propagated::Keyframe(
                    feed.source,
                    feed.mid,
                    Some(feed.layer),
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

    /// Writes `packet`, sent by `source` on the layer `layer` of its track `mid`, to the slot
    /// of this participant that carries it, if any (`CALL-032`). The payload and the marker
    /// stay as the sender wrote them. The sequence numbers and the timestamps of a slot
    /// continue across each switch of layer or of sender (`CALL-040`, `CALL-048`).
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
        let feed = Feed { source, mid, layer };
        // Most packets belong to a feed no slot of this receiver carries: they stop here.
        let Some(index) = self
            .slots
            .iter()
            .position(|s| s.current == Some(feed) || s.target == Some(feed))
        else {
            return;
        };
        let Some(pt) = self.rtc.codec_config().match_params(params).map(|p| p.pt()) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        let OutState::Open(out_mid) = slot.state else {
            return;
        };
        if slot.target == Some(feed) && slot.current != Some(feed) {
            // A video feed starts on one of its keyframes, so the receiver decodes on from it
            // (`CALL-039`, `CALL-049`). A codec whose keyframe the SFU cannot read starts
            // anywhere, and so does audio.
            let starts = slot.kind == MediaKind::Audio
                || params.spec().codec != Codec::Vp8
                || Vp8Descriptor::parse(&packet.payload)
                    .is_ok_and(|d| d.starts_keyframe(&packet.payload));
            if starts {
                slot.current = Some(feed);
                slot.asked = None;
                slot.hold = Hold::Flowing;
                slot.rewrite.switch(now, packet);
            }
        }
        if slot.current != Some(feed) || slot.hold == Hold::Lowering {
            return;
        }
        let Some(out) = slot.rewrite.apply(now, packet) else {
            return;
        };
        let nackable = slot.kind == MediaKind::Video;
        let mut api = self.rtc.direct_api();
        let Some(stream) = api.stream_tx_by_mid(out_mid, None) else {
            return;
        };
        let payload = packet.payload.clone();
        let mut write = RtpWrite::new(pt, out.seq, out.time.get(), packet.timestamp, payload)
            .marker(packet.header.marker)
            .ext_vals(packet.header.ext_vals.clone())
            // The SFU answers a receiver's NACK from its own cache, so a loss on one downlink
            // costs the sender nothing.
            .nackable(nackable);
        if let Some(patch) = out.patch {
            write = write.vp8_patch(patch);
        }
        stream.write_rtp(write);
        self.touched = true;
    }
}

#[cfg(test)]
mod tests;
