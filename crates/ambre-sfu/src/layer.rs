//! The layer choice: which layer of each video track the SFU forwards to one receiver.
//!
//! The choice reads the downlink estimate of the receiver, which str0m derives from the
//! transport-wide feedback of that receiver. It runs per receiver, and knows no other one, so a
//! drop for one receiver never moves the layer of another (`CALL-043`).

use std::time::{Duration, Instant};

use ambre_core::{Count, CountKind, Octets};

use str0m::bwe::Bitrate;
use str0m::media::Rid;

/// How many slots of one kind a receiver holds.
pub enum Slots {}

impl CountKind for Slots {
    type Repr = u32;
    const NAME: &'static str = "SlotCount";
}

/// The count of videos one receiver gets: the most recent speakers (`CALL-041`), from the call
/// row of `ambre-spec-budgets.md section 1`. Each receiver holds this many video slots.
pub const VIDEOS_PER_RECEIVER: Count<Slots> = Count::new(9);

/// The count of screen slots of one receiver: one screen per room (`CALL-104`, `CALL-106`).
pub const SCREEN_SLOTS: Count<Slots> = Count::new(1);

/// The count of audio slots of one receiver: the most recent speakers (`CALL-047`), from the
/// call row of `ambre-spec-budgets.md section 1`.
pub const AUDIO_SLOTS: Count<Slots> = Count::new(3);

/// The share of the estimate the video may take. The rest absorbs the audio, the
/// retransmissions and the error of the estimate.
const VIDEO_SHARE: f64 = 0.85;

/// The margin a layer above the current one needs before the choice moves up to it. A
/// receiver near a boundary then stays on one layer.
const UPGRADE_MARGIN: f64 = 1.25;

/// One layer of a video track: its RTP stream id, or none for a track sent without simulcast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer(Option<Rid>);

impl Layer {
    /// The layer of the RTP stream id `rid`, or of a track sent without
    /// simulcast when it is `None`.
    pub fn new(rid: Option<Rid>) -> Self {
        Self(rid)
    }

    /// The RTP stream id of the layer, `None` for a track sent without
    /// simulcast.
    pub fn get(self) -> Option<Rid> {
        self.0
    }
}

/// One layer a sender offers, with the bitrate the SFU measures on it.
#[derive(Clone, Copy, Debug)]
pub struct Offered {
    /// The layer.
    pub layer: Layer,
    /// Its measured bitrate.
    pub rate: Bitrate,
}

/// One video track a receiver may get, ranked by its sender's last speech.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The layers of the track, from the lowest bitrate up.
    pub layers: Vec<Offered>,
    /// The layer the receiver gets now, if any.
    pub current: Option<Layer>,
}

/// The layer each candidate gets, in the order of `candidates`: none for a track the
/// receiver does not get.
///
/// The most recent speakers come first. Each of the first [`VIDEOS_PER_RECEIVER`] gets its
/// lowest layer while the estimate holds it. Then the first candidate moves up to the
/// highest layer the rest of the estimate holds.
pub fn choose(estimate: Bitrate, candidates: &[Candidate]) -> Vec<Option<Layer>> {
    let mut left = estimate * VIDEO_SHARE;
    let mut chosen: Vec<Option<usize>> = vec![None; candidates.len()];
    let videos = usize::try_from(VIDEOS_PER_RECEIVER.get()).unwrap_or(usize::MAX);
    for (slot, candidate) in chosen.iter_mut().zip(candidates).take(videos) {
        if let Some(lowest) = candidate.layers.first()
            && cost(candidate, lowest).as_f64() <= left.as_f64()
        {
            left = left - lowest.rate;
            *slot = Some(0);
        }
    }
    if let (Some(first), Some(Some(0))) = (candidates.first(), chosen.first()) {
        let base = first.layers.first().map_or(Bitrate::bps(0), |l| l.rate);
        let room = left + base;
        let top = first
            .layers
            .iter()
            .enumerate()
            .rev()
            .find(|(_, offered)| cost(first, offered).as_f64() <= room.as_f64());
        if let (Some((index, _)), Some(slot)) = (top, chosen.first_mut()) {
            *slot = Some(index);
        }
    }
    chosen
        .iter()
        .zip(candidates)
        .map(|(index, candidate)| {
            index.and_then(|i| candidate.layers.get(i).map(|offered| offered.layer))
        })
        .collect()
}

/// The rates the pacer of a receiver works toward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    /// What the SFU pads up to: the layers the receiver gets, plus the next step up.
    pub minimum: Bitrate,
    /// Every layer the receiver would get on an open link.
    pub desired: Bitrate,
    /// The rate a probe pads up to, when a step up exists: the minimum then.
    pub probe: Option<Bitrate>,
}

/// The rates of the pacer of a receiver, from the layers `chosen` for `candidates`.
///
/// Each rate carries the share and the margin of the choice. Without them, the estimate stays
/// at what one layer sends, and the choice never moves up.
pub fn rates(candidates: &[Candidate], chosen: &[Option<Layer>]) -> Rates {
    let scale = UPGRADE_MARGIN / VIDEO_SHARE;
    let rate_of =
        |c: &Candidate, layer: Layer| c.layers.iter().find(|o| o.layer == layer).map(|o| o.rate);
    let mut granted = Bitrate::bps(0);
    let mut next: Option<Bitrate> = None;
    let mut wanted = Bitrate::bps(0);
    for (rank, (candidate, choice)) in candidates
        .iter()
        .zip(chosen)
        .take(usize::try_from(VIDEOS_PER_RECEIVER.get()).unwrap_or(usize::MAX))
        .enumerate()
    {
        let ideal = if rank == 0 {
            candidate.layers.last()
        } else {
            candidate.layers.first()
        };
        if let Some(ideal) = ideal {
            wanted = wanted + ideal.rate;
        }
        let current = choice.and_then(|l| rate_of(candidate, l));
        if let Some(rate) = current {
            granted = granted + rate;
        }
        // The next step up: the layer above the one chosen, or the lowest one for nothing.
        let above = match choice {
            Some(layer) => candidate
                .layers
                .iter()
                .skip_while(|o| o.layer != *layer)
                .nth(1),
            None => candidate.layers.first(),
        };
        let step = above
            .filter(|_| rank == 0 || choice.is_none())
            .map(|o| o.rate - current.unwrap_or(Bitrate::bps(0)));
        if next.is_none() {
            next = step;
        }
    }
    let desired = wanted * scale;
    let minimum = ((granted + next.unwrap_or(Bitrate::bps(0))) * scale).min(desired);
    Rates {
        minimum,
        desired,
        probe: next.is_some().then_some(minimum),
    }
}

/// What the layer `offered` of `candidate` costs against the estimate. A layer above the one
/// the receiver gets now costs its bitrate raised by the margin, any other its bitrate.
fn cost(candidate: &Candidate, offered: &Offered) -> Bitrate {
    let position = |layer: Layer| candidate.layers.iter().position(|l| l.layer == layer);
    let current = candidate.current.and_then(position);
    if current.is_none_or(|c| position(offered.layer).is_none_or(|i| i > c)) {
        offered.rate * UPGRADE_MARGIN
    } else {
        offered.rate
    }
}

/// The bitrate of one layer as it arrives, over windows of half a second.
#[derive(Debug)]
pub struct Meter {
    since: Instant,
    bytes: Count<Octets>,
    rate: Bitrate,
}

/// The length of one window of the meter.
const WINDOW: Duration = Duration::from_millis(500);

impl Meter {
    /// A meter that has seen nothing since `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            since: now,
            bytes: Count::ZERO,
            rate: Bitrate::bps(0),
        }
    }

    /// Counts a packet of a payload of `size` that arrived at `now`.
    pub fn count(&mut self, now: Instant, size: Count<Octets>) {
        let elapsed = now.saturating_duration_since(self.since);
        if elapsed >= WINDOW {
            let measured = Bitrate::from(self.bytes.get() as f64 * 8.0 / elapsed.as_secs_f64());
            // A fresh window weighs half, so one quiet window does not hide a layer.
            self.rate = if self.rate.is_zero() {
                measured
            } else {
                self.rate * 0.5 + measured * 0.5
            };
            self.since = now;
            self.bytes = Count::ZERO;
        }
        self.bytes = self.bytes.plus(size);
    }

    /// The last bitrate measured.
    pub fn rate(&self) -> Bitrate {
        self.rate
    }
}

#[cfg(test)]
mod tests;
