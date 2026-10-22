//! The look of each style of a body, one for the composer and the list.
//!
//! Bold takes the heavy face and italic the slanted one. Code, inline or a
//! block, takes the mono family on a tint, and a link the accent ink,
//! underlined. A quote draws a rule in its indent, and a list a dot before
//! each item. The field draws it as it is typed (`MURMURE-564`), the list
//! once it is sent (`MURMURE-565`). No token is new: the tint and the ink are
//! the palette's.

use ambre_bus::TextStyle;
use ambre_canvas::theme::SPACE_6;
use ambre_canvas::{Palette, SpanShape};
use ambre_core::ByteSpan;
use ambre_draw::{Affinity, Color, Cursor, Layout, ParleyBrush, Rect, Selection};

/// The family label an inline code run draws in.
pub(crate) const CODE_FAMILY: &str = "Monospace";

/// How far right a quote's and a list's lines start: the `space_6` step.
/// The margin mark stands in it, and a wrapped line keeps the block's edge
/// (`MURMURE-570`).
pub(crate) const BLOCK_INDENT: u16 = SPACE_6 as u16;

/// The left and right edges of a quote's rule, in the indent.
const QUOTE_RULE: (f64, f64) = (2.0, 4.0);

/// The side of a list's dot, centred in the indent.
const LIST_DOT: f64 = 3.0;

/// The weight of a bold run.
const BOLD_WEIGHT: u16 = 700;

/// What `style` shapes over the bytes `range`.
pub(crate) fn shape(style: &TextStyle, range: ByteSpan) -> SpanShape {
    let mut shape = SpanShape {
        range,
        ..SpanShape::default()
    };
    match style {
        TextStyle::Bold => shape.weight = Some(BOLD_WEIGHT),
        TextStyle::Italic => shape.italic = true,
        TextStyle::Code | TextStyle::CodeBlock => shape.family = Some(CODE_FAMILY.to_owned()),
        TextStyle::Link(_) => shape.underline = true,
        TextStyle::Quote | TextStyle::List => shape.indent = BLOCK_INDENT,
    }
    shape
}

/// The marks the block `style` draws in the indent left of the bytes
/// `bytes`, in the frame of `layout`. A quote takes one rule beside its
/// lines, and a list one dot before each item. The lines start past the
/// indent, so no mark covers the text (`MURMURE-570`).
pub(crate) fn margin(
    layout: &Layout<ParleyBrush>,
    text: &str,
    style: &TextStyle,
    bytes: ByteSpan,
) -> Vec<Rect> {
    let bytes = bytes.to_usize_range();
    let line = |at: usize| {
        let cursor = Cursor::from_byte_index(layout, at, Affinity::Downstream);
        let bb = cursor.geometry(layout, 1.0);
        (bb.y0, bb.y1)
    };
    match style {
        TextStyle::Quote => {
            let from = Cursor::from_byte_index(layout, bytes.start, Affinity::Downstream);
            let to = Cursor::from_byte_index(layout, bytes.end, Affinity::Upstream);
            let boxes = Selection::new(from, to).geometry(layout);
            let top = boxes.iter().map(|(bb, _)| bb.y0).reduce(f64::min);
            let bottom = boxes.iter().map(|(bb, _)| bb.y1).reduce(f64::max);
            let (top, bottom) = top.zip(bottom).unwrap_or_else(|| line(bytes.start));
            vec![Rect::new(QUOTE_RULE.0, top, QUOTE_RULE.1, bottom)]
        }
        TextStyle::List => {
            let ends = text.get(bytes.clone()).unwrap_or_default();
            let starts = std::iter::once(bytes.start).chain(
                ends.match_indices('\n')
                    .map(|(at, _)| bytes.start + at + 1)
                    .filter(|at| *at < bytes.end),
            );
            starts
                .map(|at| {
                    let (top, bottom) = line(at);
                    let middle = (top + bottom) / 2.0;
                    let half = LIST_DOT / 2.0;
                    let centre = f64::from(BLOCK_INDENT) / 2.0;
                    Rect::new(centre - half, middle - half, centre + half, middle + half)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The ink of the marks [`margin`] draws: the muted ink of the palette.
pub(crate) fn margin_ink(palette: &Palette) -> Color {
    palette.header_text
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
    matches!(style, TextStyle::Code | TextStyle::CodeBlock).then_some(palette.header_hover)
}
