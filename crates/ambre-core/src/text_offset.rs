//! Offsets into a text, one family per unit: code points, UTF-8 bytes and
//! UTF-16 code units.
//!
//! One string has three offsets for one place. Loro counts code points, a
//! Rust `String` slices at bytes, and a DOM textarea reports UTF-16 units.
//! A bare integer lets one unit stand for another, and every text past ASCII
//! then lands off by some bytes. One family per unit makes that a type error.
//! The functions below are the only crossings between the units.

use crate::index::{Axis, Count, Pos, Span};

/// The code points of a text. A [`Pos`] of it is the place before the code
/// point it names.
#[derive(PartialEq, Eq)]
pub enum Char {}

impl Axis for Char {
    const NAME: &'static str = "Char";
}

/// The UTF-8 bytes of a text, as a Rust `String` slices it.
#[derive(PartialEq, Eq)]
pub enum Byte {}

impl Axis for Byte {
    const NAME: &'static str = "Byte";
}

/// The UTF-16 code units of a text, as a DOM text field reports them.
#[derive(PartialEq, Eq)]
pub enum Utf16 {}

impl Axis for Utf16 {
    const NAME: &'static str = "Utf16";
}

/// The number of code points in `text`, saturating past `u32::MAX`.
pub fn char_len(text: &str) -> Count<Char> {
    Count::<Char>::from_len(text.chars().count())
}

/// The `Debug` form of a text a log must not carry: its length alone, spelled
/// `N chars`. Every hand-written redacted `Debug` hands its text here, so each
/// one prints the same spelling, `Variant(N chars)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RedactedChars(Count<Char>);

impl RedactedChars {
    /// The length of `text`, in code points.
    #[must_use]
    pub fn of(text: &str) -> Self {
        Self(char_len(text))
    }
}

impl std::fmt::Debug for RedactedChars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} chars", self.0.get())
    }
}

/// The byte where code point `at` of `text` starts. An offset past the last
/// code point gives the end of `text`.
pub fn char_to_byte(text: &str, at: Pos<Char>) -> Pos<Byte> {
    let skip = at.to_index().unwrap_or(usize::MAX);
    Pos::<Byte>::from_usize(
        text.char_indices()
            .nth(skip)
            .map_or(text.len(), |(byte, _)| byte),
    )
}

/// The code-point offset of byte `at` of `text`. A byte inside a code point
/// counts that code point, so it maps past it. A byte past the end gives the
/// number of code points.
pub fn byte_to_char(text: &str, at: Pos<Byte>) -> Pos<Char> {
    let before = text
        .char_indices()
        .take_while(|(byte, _)| *byte < at.to_usize())
        .count();
    Count::<Char>::from_len(before).as_end()
}

/// The bytes that the code points of `span` cover in `text`.
pub fn char_span_to_bytes(text: &str, span: Span<Char>) -> Span<Byte> {
    Span::<Byte>::new(char_to_byte(text, span.start), char_to_byte(text, span.end))
}

/// The code points that the bytes of `span` cover in `text`, per
/// [`byte_to_char`].
pub fn byte_span_to_chars(text: &str, span: Span<Byte>) -> Span<Char> {
    Span::<Char>::new(byte_to_char(text, span.start), byte_to_char(text, span.end))
}

/// The code-point offset of UTF-16 unit `at` of `text`. A unit between the
/// two halves of a surrogate pair maps before that code point. A unit past
/// the end gives the number of code points.
pub fn utf16_to_char(text: &str, at: Pos<Utf16>) -> Pos<Char> {
    let mut units = 0usize;
    let before = text
        .chars()
        .take_while(|c| {
            units = units.saturating_add(c.len_utf16());
            at.to_index().is_some_and(|at| units <= at)
        })
        .count();
    Count::<Char>::from_len(before).as_end()
}

/// The UTF-16 offset of code point `at` of `text`. An offset past the last
/// code point gives the UTF-16 length of `text`.
pub fn char_to_utf16(text: &str, at: Pos<Char>) -> Pos<Utf16> {
    let take = at.to_index().unwrap_or(usize::MAX);
    let units: usize = text.chars().take(take).map(char::len_utf16).sum();
    Count::<Utf16>::from_len(units).as_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One code point of each UTF-8 width: 1, 2, 3 and 4 bytes.
    const MIXED: &str = "a\u{e9}\u{20ac}\u{1f600}b";

    fn chars(at: u32) -> Pos<Char> {
        Pos::<Char>::new(at)
    }

    fn bytes(at: usize) -> Pos<Byte> {
        Pos::<Byte>::from_usize(at)
    }

    /// A redacted text prints its code-point count, spelled `N chars`.
    #[test]
    fn a_redacted_text_prints_its_length_in_chars() {
        assert_eq!(format!("{:?}", RedactedChars::of(MIXED)), "5 chars");
    }

    #[test]
    fn a_text_counts_its_code_points_not_its_bytes() {
        assert_eq!(char_len(""), Count::<Char>::ZERO);
        assert_eq!(char_len("abc"), Count::<Char>::new(3));
        assert_eq!(char_len(MIXED), Count::<Char>::new(5));
    }

    #[test]
    fn a_code_point_maps_to_the_byte_it_starts_at() {
        assert_eq!(char_to_byte("abc", chars(2)), bytes(2));
        let starts: Vec<usize> = (0..=5)
            .map(|at| char_to_byte(MIXED, chars(at)).to_usize())
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
        let span = Span::<Char>::new(chars(1), chars(4));
        let mapped = char_span_to_bytes(MIXED, span);
        assert_eq!(mapped, Span::<Byte>::new(bytes(1), bytes(10)));
        assert_eq!(byte_span_to_chars(MIXED, mapped), span);
    }

    #[test]
    fn a_surrogate_pair_is_two_utf16_units_and_one_code_point() {
        let text = "\u{1f600}ab";
        assert_eq!(char_to_utf16(text, chars(1)), Pos::<Utf16>::new(2));
        assert_eq!(char_to_utf16(text, chars(3)), Pos::<Utf16>::new(4));
        assert_eq!(char_to_utf16(text, chars(7)), Pos::<Utf16>::new(4));
        assert_eq!(utf16_to_char(text, Pos::<Utf16>::new(2)), chars(1));
        // Between the two halves of the pair, the offset stays before it.
        assert_eq!(utf16_to_char(text, Pos::<Utf16>::new(1)), chars(0));
        assert_eq!(utf16_to_char(text, Pos::<Utf16>::new(9)), chars(3));
        assert_eq!(
            utf16_to_char(MIXED, char_to_utf16(MIXED, chars(4))),
            chars(4)
        );
    }
}
