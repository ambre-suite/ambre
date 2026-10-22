//! Offsets into a text, one family per unit: code points, UTF-8 bytes and
//! UTF-16 code units.
//!
//! One string has three offsets for one place. Loro counts code points, a
//! Rust `String` slices at bytes, and a DOM textarea reports UTF-16 units.
//! A bare integer lets one unit stand for another, and every text past ASCII
//! then lands off by some bytes. One family per unit makes that a type error.
//! The functions below are the only crossings between the units.

crate::define_index! {
    /// A code-point offset into a text: the place before the code point it
    /// names.
    pub struct CharOffset(u32);
    /// A number of code points.
    pub struct CharCount;
    /// The signed gap between two code-point offsets.
    pub struct CharDelta(i64);
    /// A run of code points.
    pub struct CharSpan;
    /// A list indexed by code-point offset.
    pub struct CharVec;
}

crate::define_index! {
    /// A UTF-8 byte offset into a text, as a Rust `String` slices it.
    pub struct ByteOffset(usize);
    /// A number of UTF-8 bytes.
    pub struct ByteCount;
    /// The signed gap between two byte offsets.
    pub struct ByteDelta(isize);
    /// A run of UTF-8 bytes.
    pub struct ByteSpan;
    /// A list indexed by byte offset.
    pub struct ByteVec;
}

crate::define_index! {
    /// A UTF-16 code-unit offset into a text, as a DOM text field reports it.
    pub struct Utf16Offset(u32);
    /// A number of UTF-16 code units.
    pub struct Utf16Count;
    /// The signed gap between two UTF-16 offsets.
    pub struct Utf16Delta(i64);
    /// A run of UTF-16 code units.
    pub struct Utf16Span;
    /// A list indexed by UTF-16 offset.
    pub struct Utf16Vec;
}

/// The number of code points in `text`, saturating past `u32::MAX`.
pub fn char_len(text: &str) -> CharCount {
    CharCount::from_len(text.chars().count())
}

/// The byte where code point `at` of `text` starts. An offset past the last
/// code point gives the end of `text`.
pub fn char_to_byte(text: &str, at: CharOffset) -> ByteOffset {
    let skip = at.to_index().unwrap_or(usize::MAX);
    ByteOffset::new(
        text.char_indices()
            .nth(skip)
            .map_or(text.len(), |(byte, _)| byte),
    )
}

/// The code-point offset of byte `at` of `text`. A byte inside a code point
/// counts that code point, so it maps past it. A byte past the end gives the
/// number of code points.
pub fn byte_to_char(text: &str, at: ByteOffset) -> CharOffset {
    let before = text
        .char_indices()
        .take_while(|(byte, _)| *byte < at.get())
        .count();
    CharCount::from_len(before).as_end()
}

/// The bytes that the code points of `span` cover in `text`.
pub fn char_span_to_bytes(text: &str, span: CharSpan) -> ByteSpan {
    ByteSpan::new(char_to_byte(text, span.start), char_to_byte(text, span.end))
}

/// The code points that the bytes of `span` cover in `text`, per
/// [`byte_to_char`].
pub fn byte_span_to_chars(text: &str, span: ByteSpan) -> CharSpan {
    CharSpan::new(byte_to_char(text, span.start), byte_to_char(text, span.end))
}

/// The code-point offset of UTF-16 unit `at` of `text`. A unit between the
/// two halves of a surrogate pair maps before that code point. A unit past
/// the end gives the number of code points.
pub fn utf16_to_char(text: &str, at: Utf16Offset) -> CharOffset {
    let mut units = 0usize;
    let before = text
        .chars()
        .take_while(|c| {
            units = units.saturating_add(c.len_utf16());
            at.to_index().is_some_and(|at| units <= at)
        })
        .count();
    CharCount::from_len(before).as_end()
}

/// The UTF-16 offset of code point `at` of `text`. An offset past the last
/// code point gives the UTF-16 length of `text`.
pub fn char_to_utf16(text: &str, at: CharOffset) -> Utf16Offset {
    let take = at.to_index().unwrap_or(usize::MAX);
    let units: usize = text.chars().take(take).map(char::len_utf16).sum();
    Utf16Count::from_len(units).as_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One code point of each UTF-8 width: 1, 2, 3 and 4 bytes.
    const MIXED: &str = "a\u{e9}\u{20ac}\u{1f600}b";

    fn chars(at: u32) -> CharOffset {
        CharOffset::new(at)
    }

    fn bytes(at: usize) -> ByteOffset {
        ByteOffset::new(at)
    }

    #[test]
    fn a_text_counts_its_code_points_not_its_bytes() {
        assert_eq!(char_len(""), CharCount::ZERO);
        assert_eq!(char_len("abc"), CharCount::new(3));
        assert_eq!(char_len(MIXED), CharCount::new(5));
    }

    #[test]
    fn a_code_point_maps_to_the_byte_it_starts_at() {
        assert_eq!(char_to_byte("abc", chars(2)), bytes(2));
        let starts: Vec<usize> = (0..=5)
            .map(|at| char_to_byte(MIXED, chars(at)).get())
            .collect();
        assert_eq!(starts, vec![0, 1, 3, 6, 10, 11]);
        assert_eq!(char_to_byte(MIXED, chars(9)), bytes(MIXED.len()));
    }

    #[test]
    fn a_byte_maps_back_to_its_code_point() {
        for (at, byte) in [0, 1, 3, 6, 10, 11].into_iter().enumerate() {
            assert_eq!(byte_to_char(MIXED, bytes(byte)), chars(at as u32));
        }
        // A byte inside the 3-byte euro sign counts that code point.
        assert_eq!(byte_to_char(MIXED, bytes(4)), chars(3));
        assert_eq!(byte_to_char(MIXED, bytes(99)), chars(5));
    }

    #[test]
    fn a_span_maps_its_two_ends() {
        let span = CharSpan::new(chars(1), chars(4));
        let mapped = char_span_to_bytes(MIXED, span);
        assert_eq!(mapped, ByteSpan::new(bytes(1), bytes(10)));
        assert_eq!(byte_span_to_chars(MIXED, mapped), span);
    }

    #[test]
    fn a_surrogate_pair_is_two_utf16_units_and_one_code_point() {
        let text = "\u{1f600}ab";
        assert_eq!(char_to_utf16(text, chars(1)), Utf16Offset::new(2));
        assert_eq!(char_to_utf16(text, chars(3)), Utf16Offset::new(4));
        assert_eq!(char_to_utf16(text, chars(7)), Utf16Offset::new(4));
        assert_eq!(utf16_to_char(text, Utf16Offset::new(2)), chars(1));
        // Between the two halves of the pair, the offset stays before it.
        assert_eq!(utf16_to_char(text, Utf16Offset::new(1)), chars(0));
        assert_eq!(utf16_to_char(text, Utf16Offset::new(9)), chars(3));
        assert_eq!(
            utf16_to_char(MIXED, char_to_utf16(MIXED, chars(4))),
            chars(4)
        );
    }
}
