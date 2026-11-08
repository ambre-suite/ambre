//! The header fields of a video track forwarded to one receiver, across layer switches.
//!
//! Each layer of a simulcast track numbers its packets and its pictures on its own. The
//! receiver sees one track, so the SFU shifts the fields of each layer to continue where the
//! last layer stopped (`CALL-040`). It reads the VP8 payload descriptor alone, which the seal of
//! a chunk leaves in clear. str0m copies the rest of the payload unchanged (`CALL-032`).

use std::time::Instant;

use str0m::rtp::{RtpPacket, SeqNo, Vp8Descriptor, Vp8Patch};

/// The 90 kHz clock of a video RTP timestamp, in ticks per millisecond.
const TICKS_PER_MS: u128 = 90;

/// The width of a 15-bit VP8 picture id.
const PICTURE_ID_MASK: u16 = 0x7fff;

/// The fields of one packet as the receiver gets them.
#[derive(Debug)]
pub struct Rewritten {
    /// The sequence number.
    pub seq: SeqNo,
    /// The RTP timestamp.
    pub time: u32,
    /// The patch of the VP8 payload descriptor, which str0m applies as it writes the packet.
    pub patch: Option<Vp8Patch>,
}

/// The last packet written to the receiver.
#[derive(Clone, Copy, Debug)]
struct Last {
    seq: SeqNo,
    time: u32,
    at: Instant,
    picture: Option<u16>,
    tl0: Option<u8>,
}

/// The shift from the fields of the current layer to the fields the receiver gets.
#[derive(Clone, Copy, Debug)]
struct Shift {
    /// The first sequence number of the layer since the switch. An earlier one is a late
    /// packet, and its shifted number would fall on a packet already written.
    from: SeqNo,
    seq: u64,
    time: u32,
    picture: u16,
    tl0: u8,
}

/// The rewrite state of one video track forwarded to one receiver.
#[derive(Debug, Default)]
pub struct Rewrite {
    last: Option<Last>,
    shift: Option<Shift>,
}

impl Rewrite {
    /// Starts a new layer at `packet`, the first packet of one of its keyframes.
    pub fn switch(&mut self, now: Instant, packet: &RtpPacket) {
        let descriptor = Vp8Descriptor::parse(&packet.payload).ok();
        let shift = match self.last {
            // The first packet keeps the 16 bits of its header: the receiver starts its
            // rollover counter at zero.
            None => Shift {
                from: packet.seq_no,
                seq: (*packet.seq_no & 0xffff).wrapping_sub(*packet.seq_no),
                time: 0,
                picture: 0,
                tl0: 0,
            },
            Some(last) => {
                let ticks = now.saturating_duration_since(last.at).as_millis() * TICKS_PER_MS;
                let step = u32::try_from(ticks).unwrap_or(u32::MAX).max(1);
                let picture = descriptor.as_ref().and_then(Vp8Descriptor::picture_id);
                let tl0 = descriptor.as_ref().and_then(Vp8Descriptor::tl0_pic_idx);
                Shift {
                    from: packet.seq_no,
                    seq: (*last.seq + 1).wrapping_sub(*packet.seq_no),
                    time: last
                        .time
                        .wrapping_add(step)
                        .wrapping_sub(packet.header.timestamp),
                    picture: match (last.picture, picture) {
                        (Some(out), Some(input)) => out.wrapping_add(1).wrapping_sub(input),
                        _ => 0,
                    },
                    tl0: match (last.tl0, tl0) {
                        (Some(out), Some(input)) => out.wrapping_add(1).wrapping_sub(input),
                        _ => 0,
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
        let seq = SeqNo::from((*packet.seq_no).wrapping_add(shift.seq));
        let time = packet.header.timestamp.wrapping_add(shift.time);
        let descriptor = Vp8Descriptor::parse(&packet.payload).ok();
        let picture = descriptor
            .as_ref()
            .and_then(Vp8Descriptor::picture_id)
            .map(|p| p.wrapping_add(shift.picture) & PICTURE_ID_MASK);
        let tl0 = descriptor
            .as_ref()
            .and_then(Vp8Descriptor::tl0_pic_idx)
            .map(|t| t.wrapping_add(shift.tl0));
        let patch = descriptor.as_ref().and_then(|d| {
            let mut patch = d.patch();
            if let Some(p) = picture {
                patch = patch.picture_id(p);
            }
            if let Some(t) = tl0 {
                patch = patch.tl0_pic_idx(t);
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
