//! The arithmetic of the header fields: each wraps at the width RTP or VP8 fixes for it.

use super::{Mark, Offset, PictureId, Sequence, Timestamp, Tl0Index};

#[test]
fn a_field_wraps_at_its_own_width() {
    assert_eq!((Mark::<Tl0Index>::new(u8::MAX) + Offset::new(1)).get(), 0);
    assert_eq!((Mark::<Timestamp>::new(u32::MAX) + Offset::new(2)).get(), 1);
    assert_eq!((Mark::<Sequence>::new(u64::MAX) + Offset::new(1)).get(), 0);
}

#[test]
fn the_offset_between_two_values_moves_the_earlier_to_the_later() {
    let earlier = Mark::<Timestamp>::new(u32::MAX - 9);
    let later = Mark::<Timestamp>::new(20);
    assert_eq!((earlier + (later - earlier)).get(), 20);
}

#[test]
fn a_picture_id_keeps_its_fifteen_bits() {
    let last = Mark::<PictureId>::new(0x7fff);
    assert_eq!((last + Offset::new(1)).within_15_bits().get(), 0);
}
