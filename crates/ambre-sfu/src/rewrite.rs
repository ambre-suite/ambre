//! The header fields of one slot of a receiver, across switches of layer and of sender.
//!
//! Each layer of a simulcast track, and each sender, numbers its packets and its pictures on its
//! own. The receiver sees one track per slot. So the SFU shifts the fields of each new feed to
//! continue where the last one stopped (`CALL-040`, `CALL-048`). It reads the VP8 payload
//! descriptor alone, which the seal of a chunk leaves in clear. str0m copies the rest of the
//! payload unchanged (`CALL-032`).

use std::fmt;
use std::marker::PhantomData;
use std::num::Wrapping;
use std::ops::{Add, Sub};
use std::time::Instant;

use str0m::media::Frequency;
use str0m::rtp::{RtpPacket, SeqNo, Vp8Descriptor, Vp8Patch};

/// The marker of one field of the RTP header or of the VP8 payload descriptor.
///
/// RTP and VP8 fix the width of each field, and its arithmetic wraps at that width. A marker
/// per field keeps an offset of one field from the value of another: a sequence offset added to
/// a timestamp does not compile.
pub trait Field: 'static {
    /// The number the field holds, at its width.
    type Repr: Copy + fmt::Debug;
    /// The name `Debug` prints, for a log line.
    const NAME: &'static str;
}

/// The RTP timestamp, 32 bits wide.
#[derive(Debug)]
pub enum Timestamp {}

impl Field for Timestamp {
    type Repr = u32;
    const NAME: &'static str = "RtpTimestamp";
}

/// The extended sequence number a `SeqNo` of str0m holds, 64 bits wide.
#[derive(Debug)]
pub enum Sequence {}

impl Field for Sequence {
    type Repr = u64;
    const NAME: &'static str = "RtpSequence";
}

/// The VP8 picture id, as the 16 bits the descriptor of str0m reads.
#[derive(Debug)]
pub enum PictureId {}

impl Field for PictureId {
    type Repr = u16;
    const NAME: &'static str = "Vp8PictureId";
}

/// The VP8 TL0PICIDX, 8 bits wide.
#[derive(Debug)]
pub enum Tl0Index {}

impl Field for Tl0Index {
    type Repr = u8;
    const NAME: &'static str = "Vp8Tl0Index";
}

/// The value of the field `F` in one packet.
pub struct Mark<F: Field>(F::Repr, PhantomData<fn() -> F>);

/// The wrapping difference between two values of the field `F`.
pub struct Offset<F: Field>(F::Repr, PhantomData<fn() -> F>);

impl<F: Field> Mark<F> {
    /// The value `repr` a packet carries.
    pub const fn new(repr: F::Repr) -> Self {
        Self(repr, PhantomData)
    }

    /// The number, for the foreign call that writes the field.
    pub const fn get(self) -> F::Repr {
        self.0
    }
}

impl<F: Field> Offset<F> {
    /// The offset `repr`.
    pub const fn new(repr: F::Repr) -> Self {
        Self(repr, PhantomData)
    }
}

/// A value moved by an offset, wrapped at the width of the field.
impl<F: Field> Add<Offset<F>> for Mark<F>
where
    Wrapping<F::Repr>: Add<Output = Wrapping<F::Repr>>,
{
    type Output = Self;
    fn add(self, offset: Offset<F>) -> Self {
        Self::new((Wrapping(self.0) + Wrapping(offset.0)).0)
    }
}

/// The offset that moves `earlier` to `self`, wrapped at the width of the field.
impl<F: Field> Sub for Mark<F>
where
    Wrapping<F::Repr>: Sub<Output = Wrapping<F::Repr>>,
{
    type Output = Offset<F>;
    fn sub(self, earlier: Self) -> Offset<F> {
        Offset::new((Wrapping(self.0) - Wrapping(earlier.0)).0)
    }
}

// Every impl below is written by hand. A derive would bound the marker, which has no value.

impl<F: Field> Clone for Mark<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F: Field> Copy for Mark<F> {}

impl<F: Field> fmt::Debug for Mark<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(F::NAME).field(&self.0).finish()
    }
}

impl<F: Field> Clone for Offset<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F: Field> Copy for Offset<F> {}

impl<F: Field> fmt::Debug for Offset<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Offset({:?})", F::NAME, self.0)
    }
}

/// The clock of the RTP timestamps of a slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clock {
    /// The 48 kHz clock of Opus.
    Audio,
    /// The 90 kHz clock of video.
    #[default]
    Video,
}

impl Clock {
    /// The rate of the clock.
    fn rate(self) -> Frequency {
        match self {
            Self::Audio => Frequency::FORTY_EIGHT_KHZ,
            Self::Video => Frequency::NINETY_KHZ,
        }
    }

    /// The ticks of the clock in the whole milliseconds of `elapsed`, at least one, so the
    /// receiver sees time move forward.
    fn ticks(self, elapsed: std::time::Duration) -> Offset<Timestamp> {
        let per_ms = u128::from(self.rate().get() / 1000);
        let ticks = elapsed.as_millis() * per_ms;
        Offset::new(u32::try_from(ticks).unwrap_or(u32::MAX).max(1))
    }
}

/// The width of a 15-bit VP8 picture id.
const PICTURE_ID_MASK: u16 = 0x7fff;

impl Mark<PictureId> {
    /// The picture id cut to the 15 bits the receiver reads.
    fn within_15_bits(self) -> Self {
        Self::new(self.0 & PICTURE_ID_MASK)
    }
}

/// The fields of one packet as the receiver gets them.
#[derive(Debug)]
pub struct Rewritten {
    /// The sequence number.
    pub seq: SeqNo,
    /// The RTP timestamp.
    pub time: Mark<Timestamp>,
    /// The patch of the VP8 payload descriptor, which str0m applies as it writes the packet.
    pub patch: Option<Vp8Patch>,
}

/// The last packet written to the receiver.
#[derive(Clone, Copy, Debug)]
struct Last {
    seq: SeqNo,
    time: Mark<Timestamp>,
    at: Instant,
    picture: Option<Mark<PictureId>>,
    tl0: Option<Mark<Tl0Index>>,
}

/// The shift from the fields of the current layer to the fields the receiver gets.
#[derive(Clone, Copy, Debug)]
struct Shift {
    /// The first sequence number of the layer since the switch. An earlier one is a late
    /// packet, and its shifted number would fall on a packet already written.
    from: SeqNo,
    seq: Offset<Sequence>,
    time: Offset<Timestamp>,
    picture: Offset<PictureId>,
    tl0: Offset<Tl0Index>,
}

/// The rewrite state of one slot of one receiver.
#[derive(Debug, Default)]
pub struct Rewrite {
    clock: Clock,
    last: Option<Last>,
    shift: Option<Shift>,
}

impl Rewrite {
    /// The rewrite state of a slot whose timestamps count on `clock`, before its first packet.
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            last: None,
            shift: None,
        }
    }

    /// Starts a new feed at `packet`: for video, the first packet of one of its keyframes.
    pub fn switch(&mut self, now: Instant, packet: &RtpPacket) {
        let descriptor = Vp8Descriptor::parse(&packet.payload).ok();
        let seq = Mark::<Sequence>::new(*packet.seq_no);
        let shift = match self.last {
            // The first packet keeps the 16 bits of its header: the receiver starts its
            // rollover counter at zero.
            None => Shift {
                from: packet.seq_no,
                seq: Mark::new(*packet.seq_no & 0xffff) - seq,
                time: Offset::new(0),
                picture: Offset::new(0),
                tl0: Offset::new(0),
            },
            Some(last) => {
                let step = self.clock.ticks(now.saturating_duration_since(last.at));
                let picture = descriptor
                    .as_ref()
                    .and_then(Vp8Descriptor::picture_id)
                    .map(Mark::new);
                let tl0 = descriptor
                    .as_ref()
                    .and_then(Vp8Descriptor::tl0_pic_idx)
                    .map(Mark::new);
                Shift {
                    from: packet.seq_no,
                    seq: Mark::<Sequence>::new(*last.seq) + Offset::new(1) - seq,
                    time: last.time + step - Mark::new(packet.header.timestamp),
                    picture: match (last.picture, picture) {
                        (Some(out), Some(input)) => out + Offset::new(1) - input,
                        _ => Offset::new(0),
                    },
                    tl0: match (last.tl0, tl0) {
                        (Some(out), Some(input)) => out + Offset::new(1) - input,
                        _ => Offset::new(0),
                    },
                }
            }
        };
        self.shift = Some(shift);
    }

    /// The fields the receiver gets for `packet` of the current layer, or none for a packet
    /// older than the switch.
    pub fn apply(&mut self, now: Instant, packet: &RtpPacket) -> Option<Rewritten> {
        let shift = self.shift?;
        if *packet.seq_no < *shift.from {
            return None;
        }
        let seq = SeqNo::from((Mark::<Sequence>::new(*packet.seq_no) + shift.seq).get());
        let time = Mark::new(packet.header.timestamp) + shift.time;
        let descriptor = Vp8Descriptor::parse(&packet.payload).ok();
        let picture = descriptor
            .as_ref()
            .and_then(Vp8Descriptor::picture_id)
            .map(|p| (Mark::new(p) + shift.picture).within_15_bits());
        let tl0 = descriptor
            .as_ref()
            .and_then(Vp8Descriptor::tl0_pic_idx)
            .map(|t| Mark::new(t) + shift.tl0);
        let patch = descriptor.as_ref().and_then(|d| {
            let mut patch = d.patch();
            if let Some(p) = picture {
                patch = patch.picture_id(p.get());
            }
            if let Some(t) = tl0 {
                patch = patch.tl0_pic_idx(t.get());
            }
            picture.and_then(|_| patch.build().ok())
        });
        let newer = self.last.is_none_or(|last| *seq > *last.seq);
        if newer {
            self.last = Some(Last {
                seq,
                time,
                at: now,
                picture,
                tl0,
            });
        }
        Some(Rewritten { seq, time, patch })
    }
}

#[cfg(test)]
mod tests;
