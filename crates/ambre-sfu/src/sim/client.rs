//! A synthetic participant: a str0m connection that sends generated Opus and VP8 at three
//! simulcast sizes, without encoding. While it shares, it also sends a screen at one size. It
//! models what the decoder of a browser would show.
//!
//! The opaque part of each payload carries the sender, the layer, the picture number and the
//! send time. The SFU copies it unchanged, so the receiver reads it back to measure the
//! latency, to assemble each picture, and to check each layer switch.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::SimulcastLayer;
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, Mid, Rid, Simulcast};
use str0m::net::{Receive, Transmit};
use str0m::rtp::{ExtensionValues, RtpWrite, SeqNo};
use str0m::{Candidate, Event, Input, Output, Rtc};

/// The three simulcast layers: their RTP stream id and their bitrate, lowest first.
pub const LAYERS: [(&str, u64); 3] = [("l", 150_000), ("m", 500_000), ("h", 1_500_000)];

/// The bitrate of a shared screen, at one size.
pub const SCREEN_RATE: u64 = 400_000;
/// The layer byte a picture of the screen carries, past the three layers of the camera.
pub const SCREEN_LAYER: u8 = 3;
/// One picture of the screen every 200 ms: 5 fps, the call screen row of the budgets spec.
const SCREEN_PICTURE: Duration = Duration::from_millis(200);

/// One picture every 33 ms.
const PICTURE: Duration = Duration::from_micros(33_333);
/// One Opus packet every 20 ms while speaking.
const OPUS: Duration = Duration::from_millis(20);
/// One comfort-noise packet every 400 ms in silence, as Opus DTX sends.
const DTX: Duration = Duration::from_millis(400);
/// A keyframe weighs this many delta pictures.
const KEYFRAME_WEIGHT: usize = 4;
/// The largest payload of one packet.
const MAX_PAYLOAD: usize = 1100;
/// The size of a sealed audio chunk (`CALL-007`).
const AUDIO_SIZE: usize = 100;

/// A tag in the opaque part, to tell a synthetic payload from anything else.
const AUDIO_TAG: u8 = 0xA0;
const VIDEO_TAG: u8 = 0xB0;

/// The size of the VP8 payload descriptor the client writes: X, I, a 15-bit picture id.
const DESCRIPTOR: usize = 4;

/// What one received video packet carried, in arrival order.
#[derive(Clone, Copy, Debug)]
pub struct Arrival {
    pub seq: u16,
    pub sender: u16,
    pub layer: u8,
    pub picture: u32,
    pub index: u16,
    pub keyframe_start: bool,
    pub at: Instant,
    /// When the sender captured the picture.
    pub sent: Instant,
}

/// One picture being assembled.
#[derive(Debug)]
struct Partial {
    got: Vec<bool>,
    keyframe: bool,
}

/// The decoder model of one received video track.
#[derive(Debug, Default)]
pub struct VideoRx {
    pub arrivals: Vec<Arrival>,
    partial: BTreeMap<(u16, u8, u32), Partial>,
    /// Each complete picture: whether it is a keyframe, and its capture time.
    complete: BTreeMap<(u16, u8, u32), (bool, Instant)>,
    /// The least transit seen, from capture to arrival.
    least: Option<Duration>,
    /// The round trip the playout delay adds at least, as libwebrtc adds one to its jitter
    /// delay when it retransmits.
    pub playout_rtt: Duration,
    /// The lateness of each recent picture past the least transit, with its decode time.
    lateness: VecDeque<(Instant, Duration)>,
    decoded: Option<(u16, u8, u32)>,
    /// When each picture became decodable: the render time of the model.
    pub rendered: Vec<(Instant, u8)>,
    last_progress: Option<Instant>,
    asked: Option<Instant>,
}

impl VideoRx {
    fn on_packet(&mut self, arrival: Arrival, count: u16) {
        self.arrivals.push(arrival);
        let transit = arrival.at.saturating_duration_since(arrival.sent);
        self.least = Some(self.least.map_or(transit, |l| l.min(transit)));
        let key = (arrival.sender, arrival.layer, arrival.picture);
        let stale = self
            .decoded
            .is_some_and(|(s, l, p)| s == key.0 && l == key.1 && p >= key.2);
        if self.complete.contains_key(&key) || stale {
            return;
        }
        let part = self.partial.entry(key).or_insert_with(|| Partial {
            got: vec![false; usize::from(count)],
            keyframe: false,
        });
        if let Some(slot) = part.got.get_mut(usize::from(arrival.index)) {
            *slot = true;
        }
        part.keyframe |= arrival.keyframe_start;
        if part.got.iter().all(|g| *g) {
            let keyframe = part.keyframe;
            self.partial.remove(&key);
            self.complete.insert(key, (keyframe, arrival.sent));
        }
        self.decode(arrival.at);
    }

    /// Decodes every complete picture that a keyframe or its predecessor allows.
    fn decode(&mut self, now: Instant) {
        loop {
            let next = self.complete.iter().find_map(|(key, (keyframe, _))| {
                let follows = self.decoded.is_some_and(|(s, l, p)| {
                    s == key.0 && l == key.1 && p.wrapping_add(1) == key.2
                });
                (*keyframe || follows).then_some(*key)
            });
            let Some(key) = next else {
                break;
            };
            let (keyframe, sent) = self.complete.remove(&key).unwrap_or((false, now));
            self.decoded = Some(key);
            // The playout model: a picture shows at its capture time plus the least transit
            // plus a margin, or once it decodes when that comes later. The margin is the
            // larger of one round trip and the 99th percentile of the lateness over 10 s. A
            // jitter buffer grows to hold the pictures a loss delays.
            let base = sent + self.least.unwrap_or_default();
            self.lateness
                .push_back((now, now.saturating_duration_since(base)));
            while self
                .lateness
                .front()
                .is_some_and(|(t, _)| now.saturating_duration_since(*t) > Duration::from_secs(10))
            {
                self.lateness.pop_front();
            }
            let mut recent: Vec<Duration> = self.lateness.iter().map(|(_, l)| *l).collect();
            recent.sort_unstable();
            let p99 = recent
                .get(recent.len().saturating_sub(1) * 99 / 100)
                .copied()
                .unwrap_or_default();
            let due = base + self.playout_rtt.max(p99);
            let last = self.rendered.last().map_or(now, |(t, _)| *t);
            self.rendered.push((now.max(due).max(last), key.1));
            self.last_progress = Some(now);
            // A picture older than the one decoded can never decode, and a keyframe ends
            // every other layer.
            let keep = |k: &(u16, u8, u32)| {
                if k.1 == key.1 { k.2 > key.2 } else { !keyframe }
            };
            self.complete.retain(|k, _| keep(k));
            self.partial.retain(|k, _| keep(k));
        }
    }

    /// The total stall time and the time watched, over the render times of the playout model.
    /// A stall is a gap above three mean gaps and above the mean plus 150 ms, as the WebRTC
    /// statistics count a freeze. A gap of 5 s or more is a pause.
    pub fn stalls(&self, from: Instant) -> (Duration, Duration) {
        let times: Vec<Instant> = self
            .rendered
            .iter()
            .map(|(t, _)| *t)
            .filter(|t| *t >= from)
            .collect();
        let mut stalled = Duration::ZERO;
        let mut watched = Duration::ZERO;
        let mut recent: VecDeque<Duration> = VecDeque::new();
        for pair in times.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let gap = b.saturating_duration_since(*a);
            if gap >= Duration::from_secs(5) {
                recent.clear();
                continue;
            }
            watched += gap;
            if recent.len() >= 10 {
                let mean =
                    recent.iter().sum::<Duration>() / u32::try_from(recent.len()).unwrap_or(1);
                if gap > (mean * 3).max(mean + Duration::from_millis(150)) {
                    stalled += gap;
                }
            }
            recent.push_back(gap);
            if recent.len() > 30 {
                recent.pop_front();
            }
        }
        (stalled, watched)
    }
}

/// One synthetic participant.
pub struct Synth {
    pub index: u16,
    pub addr: SocketAddr,
    pub rtc: Rtc,
    epoch: Instant,
    pending: Option<SdpPendingOffer>,
    audio_mid: Mid,
    video_mid: Mid,
    screen_mid: Mid,
    pub speaking: bool,
    pub video_on: bool,
    /// Whether the participant shares its screen.
    pub sharing: bool,
    next_screen: Instant,
    screen_seq: u64,
    screen_picture: u32,
    screen_keyframe_due: bool,
    next_audio: Instant,
    next_picture: Instant,
    audio_seq: u64,
    layer_seq: [u64; 3],
    picture: [u32; 3],
    keyframe_due: [bool; 3],
    pub receiving: Vec<(Mid, MediaKind)>,
    pub video: BTreeMap<Mid, VideoRx>,
    /// The mouth-to-ear transit of every speech packet received.
    pub audio_latency: Vec<Duration>,
    /// The sender of every audio packet received, with its arrival time.
    pub audio_from: Vec<(Instant, u16)>,
    pub outbox: Vec<Transmit>,
    pub wake: Instant,
    pub keyframes_asked: usize,
    /// The round trip of the link to the SFU, which the playout model of each track adds.
    pub playout_rtt: Duration,
}

impl Synth {
    /// A participant at `addr` and its offer, as a page offers it. The offer holds one audio
    /// track, one video track at three simulcast layers, and one screen track.
    pub fn new(now: Instant, epoch: Instant, index: u16, addr: SocketAddr) -> (Self, SdpOffer) {
        let mut rtc = Rtc::builder()
            .set_rtp_mode(true)
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .build(now);
        rtc.add_local_candidate(Candidate::host(addr, "udp").expect("a candidate"));
        let mut change = rtc.sdp_api();
        let audio_mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        let mut simulcast = Simulcast::new();
        for (rid, _) in LAYERS {
            simulcast.add_send_layer(SimulcastLayer::new(rid));
        }
        let video_mid = change.add_media(
            MediaKind::Video,
            Direction::SendOnly,
            None,
            None,
            Some(simulcast),
        );
        let screen_mid = change.add_media(MediaKind::Video, Direction::SendOnly, None, None, None);
        let (offer, pending) = change.apply().expect("an offer");
        let synth = Self {
            index,
            addr,
            rtc,
            epoch,
            pending: Some(pending),
            audio_mid,
            video_mid,
            screen_mid,
            speaking: false,
            video_on: true,
            sharing: false,
            next_screen: now,
            screen_seq: 0,
            screen_picture: 0,
            screen_keyframe_due: true,
            next_audio: now,
            next_picture: now,
            audio_seq: 0,
            layer_seq: [0; 3],
            picture: [0; 3],
            keyframe_due: [true; 3],
            receiving: Vec::new(),
            video: BTreeMap::new(),
            audio_latency: Vec::new(),
            audio_from: Vec::new(),
            outbox: Vec::new(),
            wake: now,
            keyframes_asked: 0,
            playout_rtt: Duration::ZERO,
        };
        (synth, offer)
    }

    pub fn accept_answer(&mut self, answer: SdpAnswer) {
        let pending = self.pending.take().expect("an offer waits");
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .expect("the client takes the answer");
    }

    pub fn accept_offer(&mut self, offer: SdpOffer) -> SdpAnswer {
        self.rtc
            .sdp_api()
            .accept_offer(offer)
            .expect("the client takes the offer")
    }

    /// Hands one datagram to the connection, then drains it.
    pub fn receive(&mut self, now: Instant, source: SocketAddr, contents: &[u8]) {
        let Ok(contents) = contents.try_into() else {
            return;
        };
        let input = Input::Receive(
            now,
            Receive {
                proto: str0m::net::Protocol::Udp,
                source,
                destination: self.addr,
                contents,
            },
        );
        if self.rtc.handle_input(input).is_ok() {
            self.drain(now);
        }
    }

    /// Moves the clock to `now`, sends the media due, and drains the connection.
    pub fn tick(&mut self, now: Instant) {
        if self.rtc.handle_input(Input::Timeout(now)).is_err() {
            return;
        }
        self.drain(now);
        self.send_media(now);
        self.ask_keyframes(now);
        self.drain(now);
    }

    fn micros(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.epoch).as_micros()).unwrap_or(u64::MAX)
    }

    fn send_media(&mut self, now: Instant) {
        if !self.rtc.is_connected() {
            self.next_audio = now;
            self.next_picture = now;
            self.next_screen = now;
            return;
        }
        while self.next_screen <= now {
            let at = self.next_screen;
            if self.sharing {
                self.send_screen(now, at);
            }
            self.next_screen += SCREEN_PICTURE;
        }
        while self.next_audio <= now {
            let at = self.next_audio;
            self.send_audio(now, at);
            self.next_audio += if self.speaking { OPUS } else { DTX };
        }
        while self.next_picture <= now {
            let at = self.next_picture;
            if self.video_on {
                for layer in 0..LAYERS.len() {
                    self.send_picture(now, at, layer);
                }
            }
            self.next_picture += PICTURE;
        }
    }

    fn send_audio(&mut self, now: Instant, at: Instant) {
        let Some(pt) = self.pt(Codec::Opus) else {
            return;
        };
        self.audio_seq += 1;
        let mut payload = vec![0u8; AUDIO_SIZE];
        let speech = u8::from(self.speaking);
        let mut meta = vec![AUDIO_TAG, speech];
        meta.extend_from_slice(&self.index.to_le_bytes());
        meta.extend_from_slice(&self.micros(at).to_le_bytes());
        payload
            .get_mut(..meta.len())
            .expect("room for the metadata")
            .copy_from_slice(&meta);
        let ext = ExtensionValues {
            audio_level: Some(if self.speaking { -30 } else { -127 }),
            voice_activity: Some(self.speaking),
            ..ExtensionValues::default()
        };
        let time = u32::try_from((self.micros(at) * 48 / 1000) & 0xffff_ffff).unwrap_or(0);
        let mid = self.audio_mid;
        let seq = SeqNo::from(self.audio_seq);
        if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            stream.write_rtp(RtpWrite::new(pt, seq, time, now, payload).ext_vals(ext));
        }
    }

    fn send_picture(&mut self, now: Instant, at: Instant, layer: usize) {
        let Some(pt) = self.pt(Codec::Vp8) else {
            return;
        };
        let (rid, rate) = *LAYERS.get(layer).expect("a layer");
        let keyframe = std::mem::take(self.keyframe_due.get_mut(layer).expect("a layer"));
        let delta = usize::try_from(rate / 8 / 30).unwrap_or(0);
        let size = if keyframe {
            delta * KEYFRAME_WEIGHT
        } else {
            delta
        };
        let count = size.div_ceil(MAX_PAYLOAD).max(1);
        let picture = *self.picture.get(layer).expect("a layer");
        *self.picture.get_mut(layer).expect("a layer") += 1;
        let pid = u16::try_from(picture & 0x7fff).unwrap_or(0);
        let time = u32::try_from((self.micros(at) * 90 / 1000) & 0xffff_ffff).unwrap_or(0);
        let micros = self.micros(at);
        let mid = self.video_mid;
        for index in 0..count {
            let start = index == 0;
            let mut payload = vec![
                0x80 | if start { 0x10 } else { 0 },
                0x80,
                0x80 | u8::try_from(pid >> 8).unwrap_or(0),
                u8::try_from(pid & 0xff).unwrap_or(0),
                // The VP8 payload header: its P bit is clear on a keyframe.
                u8::from(!keyframe),
                VIDEO_TAG,
            ];
            payload.extend_from_slice(&self.index.to_le_bytes());
            payload.push(u8::try_from(layer).unwrap_or(0));
            payload.extend_from_slice(&picture.to_le_bytes());
            payload.extend_from_slice(&micros.to_le_bytes());
            payload.extend_from_slice(&u16::try_from(index).unwrap_or(0).to_le_bytes());
            payload.extend_from_slice(&u16::try_from(count).unwrap_or(0).to_le_bytes());
            let body = (size / count).max(payload.len());
            payload.resize(body, 0);
            let seq_slot = self.layer_seq.get_mut(layer).expect("a layer");
            *seq_slot += 1;
            let seq = SeqNo::from(*seq_slot);
            if let Some(stream) = self
                .rtc
                .direct_api()
                .stream_tx_by_mid(mid, Some(Rid::from(rid)))
            {
                stream.write_rtp(
                    RtpWrite::new(pt, seq, time, now, payload)
                        .marker(index + 1 == count)
                        .nackable(true),
                );
            }
        }
    }

    /// Sends one picture of the screen, at one size and no simulcast. A keyframe goes once at
    /// the start and on each request.
    fn send_screen(&mut self, now: Instant, at: Instant) {
        let Some(pt) = self.pt(Codec::Vp8) else {
            return;
        };
        let keyframe = std::mem::take(&mut self.screen_keyframe_due);
        let size = usize::try_from(SCREEN_RATE / 8 / 5).unwrap_or(0);
        let count = size.div_ceil(MAX_PAYLOAD).max(1);
        let picture = self.screen_picture;
        self.screen_picture += 1;
        let pid = u16::try_from(picture & 0x7fff).unwrap_or(0);
        let time = u32::try_from((self.micros(at) * 90 / 1000) & 0xffff_ffff).unwrap_or(0);
        let micros = self.micros(at);
        let mid = self.screen_mid;
        for index in 0..count {
            let start = index == 0;
            let mut payload = vec![
                0x80 | if start { 0x10 } else { 0 },
                0x80,
                0x80 | u8::try_from(pid >> 8).unwrap_or(0),
                u8::try_from(pid & 0xff).unwrap_or(0),
                u8::from(!keyframe),
                VIDEO_TAG,
            ];
            payload.extend_from_slice(&self.index.to_le_bytes());
            payload.push(SCREEN_LAYER);
            payload.extend_from_slice(&picture.to_le_bytes());
            payload.extend_from_slice(&micros.to_le_bytes());
            payload.extend_from_slice(&u16::try_from(index).unwrap_or(0).to_le_bytes());
            payload.extend_from_slice(&u16::try_from(count).unwrap_or(0).to_le_bytes());
            payload.resize((size / count).max(payload.len()), 0);
            self.screen_seq += 1;
            let seq = SeqNo::from(self.screen_seq);
            if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
                stream.write_rtp(
                    RtpWrite::new(pt, seq, time, now, payload)
                        .marker(index + 1 == count)
                        .nackable(true),
                );
            }
        }
    }

    fn pt(&mut self, codec: Codec) -> Option<str0m::media::Pt> {
        self.rtc
            .codec_config()
            .find(|p| p.spec().codec == codec)
            .map(|p| p.pt())
    }

    /// Asks for a keyframe on a track that made no progress for 500 ms while packets came,
    /// as a browser does once its retransmissions fail.
    fn ask_keyframes(&mut self, now: Instant) {
        let stuck: Vec<Mid> = self
            .video
            .iter_mut()
            .filter_map(|(mid, rx)| {
                let fresh = rx
                    .arrivals
                    .last()
                    .is_some_and(|a| now - a.at < Duration::from_millis(200));
                let stalled = rx
                    .last_progress
                    .is_none_or(|p| now.saturating_duration_since(p) > Duration::from_millis(500));
                let quiet = rx
                    .asked
                    .is_none_or(|a| now.saturating_duration_since(a) > Duration::from_secs(1));
                (fresh && stalled && quiet).then(|| {
                    rx.asked = Some(now);
                    *mid
                })
            })
            .collect();
        for mid in stuck {
            if let Some(stream) = self.rtc.direct_api().stream_rx_by_mid(mid, None) {
                stream.request_keyframe(KeyframeRequestKind::Pli);
                self.keyframes_asked += 1;
            }
        }
    }

    fn drain(&mut self, now: Instant) {
        loop {
            let Ok(output) = self.rtc.poll_output() else {
                return;
            };
            match output {
                Output::Timeout(at) => {
                    self.wake = at;
                    return;
                }
                Output::Transmit(t) => self.outbox.push(t),
                Output::Event(event) => self.on_event(now, event),
            }
        }
    }

    fn on_event(&mut self, now: Instant, event: Event) {
        match event {
            Event::MediaAdded(added) => self.receiving.push((added.mid, added.kind)),
            Event::KeyframeRequest(request) if request.mid == self.screen_mid => {
                self.screen_keyframe_due = true;
            }
            Event::KeyframeRequest(request) => {
                let layer = request
                    .rid
                    .and_then(|rid| LAYERS.iter().position(|(r, _)| Rid::from(*r) == rid));
                match layer {
                    Some(l) => {
                        if let Some(due) = self.keyframe_due.get_mut(l) {
                            *due = true;
                        }
                    }
                    None => self.keyframe_due = [true; 3],
                }
            }
            Event::RtpPacket(packet) => {
                let Some(mid) = self
                    .rtc
                    .direct_api()
                    .stream_rx(&packet.header.ssrc)
                    .map(|s| s.mid())
                else {
                    return;
                };
                let payload = &packet.payload[..];
                match payload.first() {
                    Some(&AUDIO_TAG) => {
                        let speech = payload.get(1) == Some(&1);
                        if let Some(sender) = payload
                            .get(2..4)
                            .and_then(|b| b.try_into().ok())
                            .map(u16::from_le_bytes)
                        {
                            self.audio_from.push((now, sender));
                        }
                        let sent = payload
                            .get(4..12)
                            .and_then(|b| b.try_into().ok())
                            .map(u64::from_le_bytes);
                        if let (true, Some(sent)) = (speech, sent) {
                            let at = self.epoch + Duration::from_micros(sent);
                            self.audio_latency.push(now.saturating_duration_since(at));
                        }
                    }
                    _ => {
                        let Some(meta) = payload.get(DESCRIPTOR..) else {
                            return;
                        };
                        if meta.get(1) != Some(&VIDEO_TAG) {
                            return;
                        }
                        let read = |range: std::ops::Range<usize>| meta.get(range);
                        let field16 = |at: usize| {
                            read(at..at + 2)
                                .and_then(|b| b.try_into().ok())
                                .map(u16::from_le_bytes)
                                .unwrap_or(0)
                        };
                        let sender = field16(2);
                        let layer = meta.get(4).copied().unwrap_or(0);
                        let picture = read(5..9)
                            .and_then(|b| b.try_into().ok())
                            .map(u32::from_le_bytes)
                            .unwrap_or(0);
                        let sent = read(9..17)
                            .and_then(|b| b.try_into().ok())
                            .map(u64::from_le_bytes)
                            .unwrap_or(0);
                        let index = field16(17);
                        let count = field16(19);
                        let start = payload.first().is_some_and(|b| b & 0x10 != 0);
                        let arrival = Arrival {
                            seq: packet.header.sequence_number,
                            sender,
                            layer,
                            picture,
                            index,
                            keyframe_start: start && meta.first() == Some(&0),
                            at: now,
                            sent: self.epoch + Duration::from_micros(sent),
                        };
                        let rtt = self.playout_rtt;
                        let rx = self.video.entry(mid).or_default();
                        rx.playout_rtt = rtt;
                        rx.on_packet(arrival, count);
                    }
                }
            }
            _ => {}
        }
    }
}
