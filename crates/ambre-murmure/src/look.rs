//! The look of each style of a body, one for the composer and the list.
//!
//! Bold takes the heavy face and italic the slanted one. Code takes the
//! mono family on a tint, and a link the accent ink, underlined. The field
//! draws it as it is typed (`MURMURE-564`), the list once it is sent
//! (`MURMURE-565`). No token is new: the tint and the ink are the palette's.

use std::ops::Range;

use ambre_bus::TextStyle;
use ambre_canvas::{Palette, SpanShape};
use ambre_draw::Color;

/// The family label an inline code run draws in.
pub(crate) const CODE_FAMILY: &str = "Monospace";

/// The weight of a bold run.
const BOLD_WEIGHT: u16 = 700;

/// What `style` shapes over the bytes `range`.
pub(crate) fn shape(style: &TextStyle, range: Range<usize>) -> SpanShape {
    let mut shape = SpanShape {
        range,
        ..SpanShape::default()
    };
    match style {
        TextStyle::Bold => shape.weight = Some(BOLD_WEIGHT),
        TextStyle::Italic => shape.italic = true,
        TextStyle::Code => shape.family = Some(CODE_FAMILY.to_owned()),
        TextStyle::Link(_) => shape.underline = true,
    }
    shape
}

/// The ink `style` draws its run in, `None` for the body's own.
pub(crate) fn ink(style: &TextStyle, palette: &Palette) -> Option<Color> {
    matches!(style, TextStyle::Link(_)).then(|| link_ink(palette))
}

/// The ink of a link: the accent.
pub(crate) fn link_ink(palette: &Palette) -> Color {
    palette.selection
}

/// The tint `style` draws behind its run, `None` for none.
pub(crate) fn tint(style: &TextStyle, palette: &Palette) -> Option<Color> {
    matches!(style, TextStyle::Code).then_some(palette.header_hover)
}
