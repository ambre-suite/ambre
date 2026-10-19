//! The painter of the message list (the Murmure message list spec).
//!
//! It lays out only the rows a viewport reaches from the bottom, with the
//! text pipeline every application shares (`MURMURE-187`, `MURMURE-188`).
//! It paints them on any drawing surface: the page's canvas, or the
//! recording surface of a host test.

use std::collections::HashMap;
use std::ops::Range;

use ambre_bus::{MessageId, Timestamp, UserId};
use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
use ambre_canvas::{Palette, SpanShape, TextCache};
use ambre_draw::{
    Affine, Affinity, Alignment, Brush, Color, Cursor, DrawSurface, FontContext, Layout,
    ParleyBrush, PositionedLayoutItem, Rect, Selection,
};

use crate::list::{JUMP_TO_LATEST, MessageList, RowKey, RowWords, UNREAD_LINE, window};
use crate::look;
use crate::select::{Selecting, Spot, TextSelection};

/// How strongly a selection tints the text under it.
const SELECTION_ALPHA: f32 = 0.3;

/// What one paint of the list did.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    /// The rows laid out: the ones the viewport reaches, never the whole
    /// list.
    pub laid_out: usize,
    /// The latest row is on screen.
    pub latest_shown: bool,
    /// The oldest row the list holds is on screen (`MURMURE-191`).
    pub oldest_shown: bool,
    /// The unread line is on screen (`MURMURE-184`).
    pub line_shown: bool,
    /// The row the list rests on and its top, `None` on the latest row.
    pub rest: Option<(usize, f32)>,
    /// The first row on screen and its top, wherever the list rests.
    pub top: Option<(usize, f32)>,
    /// The last row on screen, wherever the list rests.
    pub bottom: Option<usize>,
    /// The top and the bottom of the focused row, `None` when it is off
    /// screen (`MURMURE-205`).
    pub focus: Option<(f32, f32)>,
    /// Each row on screen, top first: its index, its top and its bottom. A
    /// press finds the row under it here.
    pub rows: Vec<(usize, f32, f32)>,
    /// The selection, with the points the paint resolved into it
    /// (`MURMURE-425`).
    pub selection: Option<TextSelection>,
}

impl Painted {
    /// The index of the row under the height `y`, `None` between rows or
    /// past them.
    pub fn row_at(&self, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|&&(_, top, bottom)| top <= y && y < bottom)
            .map(|&(i, _, _)| i)
    }
}

/// What the list paints: the messages, the first unread one, which the
/// unread line sits above (`MURMURE-184`), and the row it rests on.
#[derive(Debug, Clone, Copy)]
pub struct Shown<'a> {
    pub list: &'a MessageList,
    pub unread: Option<MessageId>,
    /// The row at the top and its top, `None` to rest on the latest row.
    pub anchor: Option<(usize, f32)>,
    /// The row that holds the focus, ringed in the accent (`MURMURE-205`).
    pub focus: Option<usize>,
    /// The selection, and the points of a drag the paint resolves.
    pub selecting: Selecting,
}

/// Paint `shown` from its anchor, in a viewport of `size`.
///
/// `name_of` names a member, and `time_of` writes a time in the reader's
/// language and zone (`MURMURE-418`).
#[allow(clippy::too_many_arguments)]
pub fn paint_list<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    shown: Shown<'_>,
    size: (f32, f32),
    palette: &Palette,
    name_of: &dyn Fn(UserId) -> Option<String>,
    time_of: &dyn Fn(Timestamp) -> String,
) -> Painted {
    let (width, height) = size;
    let backdrop = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    surface.fill_rect(backdrop, palette.content_bg);
    let rows = shown.list.rows();
    let text_width = (width - 2.0 * SPACE_8).max(1.0);
    let mut words: HashMap<usize, (RowWords, bool)> = HashMap::new();
    let reached = window(rows.len(), height, shown.anchor, |i| {
        let Some(row) = rows.get(i) else {
            return 0.0;
        };
        let line = shown.unread.is_some() && shown.unread == row.message().map(|m| m.id);
        let row = row.words(name_of, time_of);
        let mut tall = measure(font_cx, cache, &row, text_width);
        if line {
            tall += line_height(font_cx, cache, text_width);
        }
        words.insert(i, (row, line));
        tall
    });
    let selecting = shown.selecting;
    let active = selecting != Selecting::default();
    let keys = if active {
        shown.list.keys()
    } else {
        Vec::new()
    };
    let mut selection = selecting.selection;
    let mut spot_at = |at| spot_at(font_cx, cache, &reached.rows, &words, &keys, at, text_width);
    if let Some(at) = selecting.press {
        selection = spot_at(at).map(|spot| TextSelection {
            anchor: spot,
            focus: spot,
        });
    }
    if let (Some(at), Some(selection)) = (selecting.drag, selection.as_mut())
        && let Some(spot) = spot_at(at)
    {
        selection.focus = spot;
    }
    let mut line_shown = false;
    let mut focus = None;
    for &(i, top, tall) in &reached.rows {
        let Some((row, line)) = words.get(&i) else {
            continue;
        };
        let band = selection.and_then(|s| s.range_in(&keys, i, row.body.len()));
        let tints: Vec<(Range<usize>, Color)> = row
            .marks
            .iter()
            .filter_map(|mark| look::tint(&mark.style, palette).map(|c| (mark.bytes.clone(), c)))
            .collect();
        if band.is_some() || !tints.is_empty() {
            let body_y = body_top(font_cx, cache, row, *line, top, text_width);
            let layout = body(font_cx, cache, row, text_width);
            // The tints go first, so a selection band stays visible over code.
            for (bytes, color) in tints {
                fill_bytes(surface, layout, bytes, body_y, color);
            }
            if let Some((start, end)) = band {
                let tint = palette.selection.with_alpha(SELECTION_ALPHA);
                fill_bytes(surface, layout, start..end, body_y, tint);
            }
        }
        let bottom = top + tall;
        let mut top = top;
        if *line {
            line_shown = true;
            top = draw_line(surface, font_cx, cache, top, width, palette);
        }
        draw_row(surface, font_cx, cache, row, top, text_width, palette);
        if shown.focus == Some(i) {
            let ring = Rect::new(
                f64::from(SPACE_2),
                f64::from(top),
                f64::from(width - SPACE_2),
                f64::from(bottom),
            );
            surface.stroke_rect(ring, palette.selection, 2.0);
            focus = Some((top, bottom));
        }
    }
    Painted {
        laid_out: words.len(),
        latest_shown: reached.latest,
        oldest_shown: reached.oldest,
        line_shown,
        rest: reached.anchor,
        top: reached.rows.first().map(|&(i, top, _)| (i, top)),
        bottom: reached.rows.last().map(|&(i, _, _)| i),
        focus,
        rows: reached
            .rows
            .iter()
            .map(|&(i, top, tall)| (i, top, top + tall))
            .collect(),
        selection,
    }
}

/// The end of a selection under `(x, y)`: the byte of the body the layout
/// puts there. A point above every row falls on the first one. A point below
/// them falls on the last, so a drag past an edge reaches it.
fn spot_at(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    rows: &[(usize, f32, f32)],
    words: &HashMap<usize, (RowWords, bool)>,
    keys: &[RowKey],
    (x, y): (f32, f32),
    width: f32,
) -> Option<Spot> {
    let &(i, top, _) = rows
        .iter()
        .find(|&&(_, top, tall)| y < top + tall)
        .or_else(|| rows.last())?;
    let (row, line) = words.get(&i)?;
    let body_y = body_top(font_cx, cache, row, *line, top, width);
    let layout = body(font_cx, cache, row, width);
    let cursor = Cursor::from_point(layout, x - SPACE_8, y - body_y);
    Some(Spot {
        row: *keys.get(i)?,
        byte: cursor.index(),
    })
}

/// Where the body of a row whose top is at `top` starts: below the unread
/// line and the header, as [`draw_row`] paints them.
fn body_top(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    line: bool,
    top: f32,
    width: f32,
) -> f32 {
    let mut y = top + SPACE_2;
    if line {
        y += line_height(font_cx, cache, width);
    }
    y + header_height(font_cx, cache, row, width)
}

/// The height of a row's header and the space above it, nothing when the
/// row opens no group.
fn header_height(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    width: f32,
) -> f32 {
    let Some((author, time)) = &row.header else {
        return 0.0;
    };
    let author = shape(font_cx, cache, author, FONT_UI, width, true).height();
    let time = shape(font_cx, cache, time, FONT_UI, width, false).height();
    SPACE_4 + author.max(time)
}

/// Paint the control that brings the list back to its latest message, at
/// the foot of a viewport of `size` (`MURMURE-505`). Returns the rectangle
/// a press on it hits.
pub fn paint_jump<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    size: (f32, f32),
    palette: &Palette,
) -> Rect {
    let (width, height) = size;
    let layout = shape(
        font_cx,
        cache,
        JUMP_TO_LATEST,
        FONT_UI,
        width.max(1.0),
        true,
    );
    let (wide, tall) = (
        2.0 * SPACE_8 + layout.width(),
        2.0 * SPACE_4 + layout.height(),
    );
    let x = ((width - wide) / 2.0).max(0.0);
    let y = (height - tall - SPACE_8).max(0.0);
    let rect = Rect::new(
        f64::from(x),
        f64::from(y),
        f64::from(x + wide),
        f64::from(y + tall),
    );
    surface.fill_rect(rect, palette.header_bg);
    surface.stroke_rect(rect, palette.header_border, 1.0);
    draw(surface, layout, x + SPACE_8, y + SPACE_4, palette.selection);
    rect
}

/// The height of the unread line: its words and the space above them.
fn line_height(font_cx: &mut FontContext, cache: &mut TextCache, width: f32) -> f32 {
    SPACE_4 + shape(font_cx, cache, UNREAD_LINE, FONT_UI, width, true).height()
}

/// Paint the unread line whose top is at `top`: a rule across the list in
/// the accent, its words over it. Returns where the row below starts.
fn draw_line<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    top: f32,
    width: f32,
    palette: &Palette,
) -> f32 {
    let text_width = (width - 2.0 * SPACE_8).max(1.0);
    let y = top + SPACE_4;
    let layout = shape(font_cx, cache, UNREAD_LINE, FONT_UI, text_width, true);
    let (wide, tall) = (layout.width(), layout.height());
    let middle = f64::from(y + tall / 2.0);
    let rule_from = f64::from(SPACE_8 + wide + SPACE_4);
    let rule = Rect::new(rule_from, middle, f64::from(width - SPACE_8), middle + 1.0);
    surface.fill_rect(rule, palette.selection);
    draw(surface, layout, SPACE_8, y, palette.selection);
    y + tall
}

/// The layout of one text of a row, shaped once and kept in the cache.
pub(crate) fn shape<'c>(
    font_cx: &mut FontContext,
    cache: &'c mut TextCache,
    text: &str,
    size: f32,
    width: f32,
    bold: bool,
) -> &'c Layout<ParleyBrush> {
    cache.get_or_build(font_cx, text, size, width, Alignment::Start, bold, false)
}

/// The layout of a row's body, each of its marks shaped in (`MURMURE-565`).
/// A body with no mark takes the same entry as its plain text.
fn body<'c>(
    font_cx: &mut FontContext,
    cache: &'c mut TextCache,
    row: &RowWords,
    width: f32,
) -> &'c Layout<ParleyBrush> {
    let spans: Vec<SpanShape> = row
        .marks
        .iter()
        .map(|mark| look::shape(&mark.style, mark.bytes.clone()))
        .collect();
    cache.get_or_build_spans(font_cx, &row.body, FONT_BODY, width, &spans)
}

/// Fill the boxes `layout` gives the bytes `bytes`, its top at `body_y`.
fn fill_bytes<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    bytes: Range<usize>,
    body_y: f32,
    color: Color,
) {
    let from = Cursor::from_byte_index(layout, bytes.start, Affinity::Downstream);
    let to = Cursor::from_byte_index(layout, bytes.end, Affinity::Upstream);
    let (x, y) = (f64::from(SPACE_8), f64::from(body_y));
    for (bb, _) in Selection::new(from, to).geometry(layout) {
        surface.fill_rect(Rect::new(bb.x0 + x, bb.y0 + y, bb.x1 + x, bb.y1 + y), color);
    }
}

/// The height of a row, from the same layouts [`draw_row`] paints.
fn measure(font_cx: &mut FontContext, cache: &mut TextCache, row: &RowWords, width: f32) -> f32 {
    let mut tall = 2.0 * SPACE_2 + header_height(font_cx, cache, row, width);
    tall += body(font_cx, cache, row, width).height();
    if let Some(replies) = &row.replies {
        tall += shape(font_cx, cache, replies, FONT_UI, width, false).height();
    }
    tall
}

/// Paint one row whose top is at `top`.
fn draw_row<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    top: f32,
    width: f32,
    palette: &Palette,
) {
    let x = SPACE_8;
    let mut y = top + SPACE_2;
    if let Some((author, time)) = &row.header {
        y += SPACE_4;
        let layout = shape(font_cx, cache, author, FONT_UI, width, true);
        let (wide, author_tall) = (layout.width(), layout.height());
        draw(surface, layout, x, y, palette.cell_text);
        let layout = shape(font_cx, cache, time, FONT_UI, width, false);
        let time_tall = layout.height();
        draw(surface, layout, x + wide + SPACE_4, y, palette.header_text);
        y += author_tall.max(time_tall);
    }
    let ink = if row.muted {
        palette.header_text
    } else {
        palette.cell_text
    };
    let layout = body(font_cx, cache, row, width);
    let body_tall = layout.height();
    draw_body(surface, layout, x, y, ink, palette);
    y += body_tall;
    if let Some(replies) = &row.replies {
        let layout = shape(font_cx, cache, replies, FONT_UI, width, false);
        draw(surface, layout, x, y, palette.selection);
    }
}

/// Emit every glyph run of `layout`, its top-left at `(x, y)`.
pub(crate) fn draw<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    x: f32,
    y: f32,
    color: Color,
) {
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let at = (f64::from(x + run.offset()), f64::from(y + run.baseline()));
            surface.draw_glyphs(&run, &brush, Affine::translate(at));
        }
    }
}

/// Emit every glyph run of a body, its top-left at `(x, y)`. An underlined
/// run is a link: it takes the link ink and a rule under its baseline.
fn draw_body<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    x: f32,
    y: f32,
    ink: Color,
    palette: &Palette,
) {
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let linked = run.style().underline.is_some();
            let color = if linked { look::link_ink(palette) } else { ink };
            let (left, baseline) = (x + run.offset(), y + run.baseline());
            let at = (f64::from(left), f64::from(baseline));
            surface.draw_glyphs(&run, &Brush::Solid(color), Affine::translate(at));
            if linked {
                let metrics = run.run().metrics();
                let top = baseline - metrics.underline_offset;
                let size = metrics.underline_size.max(1.0);
                let rule = Rect::new(
                    f64::from(left),
                    f64::from(top),
                    f64::from(left + run.advance()),
                    f64::from(top + size),
                );
                surface.fill_rect(rule, color);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ambre_bus::{
        DocumentId, Mention, MentionTarget, SegmentId, StyleMark, TextStyle, ThreadHead,
    };
    use ambre_canvas::fonts::font_context_from_bytes;
    use ambre_canvas::theme::LIGHT;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    use super::*;
    use crate::list::tests::message;

    /// The faces the page ships, read from the application's font folder.
    pub(crate) fn fonts() -> FontContext {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        let faces = ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"]
            .iter()
            .map(|file| std::fs::read(format!("{DIR}{file}")).expect("a shipped face"));
        font_context_from_bytes(faces)
    }

    fn paint(list: &MessageList, height: f32) -> (RecordingSurface, Painted) {
        paint_above(list, None, height)
    }

    fn paint_above(
        list: &MessageList,
        unread: Option<MessageId>,
        height: f32,
    ) -> (RecordingSurface, Painted) {
        paint_from(
            Shown {
                list,
                unread,
                anchor: None,
                focus: None,
                selecting: Selecting::default(),
            },
            height,
        )
    }

    fn paint_from(shown: Shown<'_>, height: f32) -> (RecordingSurface, Painted) {
        let (mut surface, mut font_cx) = (RecordingSurface::new(), fonts());
        let mut cache = TextCache::new(1.0);
        let painted = paint_list(
            &mut surface,
            &mut font_cx,
            &mut cache,
            shown,
            (400.0, height),
            &LIGHT,
            &|_| Some("Ada".to_owned()),
            &|_| "t".to_owned(),
        );
        (surface, painted)
    }

    /// Each glyph run of `color`, as its top and its glyph count, top first.
    fn runs(surface: &RecordingSurface, color: Color) -> Vec<(f64, usize)> {
        let mut runs: Vec<(f64, usize)> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::Glyphs {
                    glyph_count,
                    color: Some(c),
                    transform,
                } if *c == color => Some((transform.translation().y, *glyph_count)),
                _ => None,
            })
            .collect();
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        runs
    }

    /// The control back to the latest message paints its words in the
    /// accent, inside the rectangle it answers, at the foot of the viewport
    /// (`MURMURE-505`).
    #[test]
    fn the_jump_control_paints_its_words_inside_what_it_answers() {
        let (mut surface, mut font_cx) = (RecordingSurface::new(), fonts());
        let mut cache = TextCache::new(1.0);
        let rect = paint_jump(
            &mut surface,
            &mut font_cx,
            &mut cache,
            (400.0, 300.0),
            &LIGHT,
        );
        assert!(
            rect.x0 > 0.0 && rect.x1 < 400.0 && rect.y1 < 300.0,
            "{rect:?}"
        );
        assert!(rect.y0 > 150.0, "at the foot of the viewport: {rect:?}");
        let words = runs(&surface, LIGHT.selection);
        assert!(!words.is_empty());
        assert!(words.iter().all(|&(y, _)| y > rect.y0 && y < rect.y1));
        assert_eq!(surface.fill_colors().first(), Some(&LIGHT.header_bg));
    }

    /// The focused row is ringed in the accent, around its own words, and a
    /// focus off screen paints no ring (`MURMURE-205`).
    #[test]
    fn the_focused_row_is_ringed_and_one_off_screen_is_not() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let messages = (0..100)
            .map(|i| message(ada, i * 600_000, "hello"))
            .collect();
        list.absorb(SegmentId::new(), messages);
        let focused = |focus| Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: Some(focus),
            selecting: Selecting::default(),
        };
        let (surface, painted) = paint_from(focused(98), 300.0);
        let rings: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokeRect { rect, color, .. } if *color == LIGHT.selection => Some(*rect),
                _ => None,
            })
            .collect();
        let [ring] = rings.as_slice() else {
            panic!("one ring: {rings:?}");
        };
        let (top, bottom) = painted.focus.expect("the focused row is on screen");
        assert_eq!((ring.y0, ring.y1), (f64::from(top), f64::from(bottom)));
        assert!(top >= 0.0 && bottom <= 300.0 && bottom > top);
        assert_eq!(painted.bottom, Some(99), "the latest row is the last one");
        let body = runs(&surface, LIGHT.cell_text);
        assert!(body.iter().any(|&(y, _)| y > ring.y0 && y < ring.y1));
        let (surface, painted) = paint_from(focused(3), 300.0);
        assert_eq!(painted.focus, None);
        assert!(
            !surface.ops().iter().any(
                |op| matches!(op, DrawOp::StrokeRect { color, .. } if *color == LIGHT.selection)
            )
        );
    }

    /// Ten thousand messages lay out only the rows of one viewport, and the
    /// latest of them is on screen (`MURMURE-187`, `MURMURE-188`).
    #[test]
    fn ten_thousand_messages_lay_out_only_one_viewport() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let messages = (0..10_000).map(|i| message(ada, i, "hello")).collect();
        list.absorb(SegmentId::new(), messages);
        let (surface, painted) = paint(&list, 200.0);
        assert!(painted.latest_shown);
        assert!(painted.laid_out > 1 && painted.laid_out < 20, "{painted:?}");
        assert_eq!(surface.fill_colors().first(), Some(&LIGHT.content_bg));
    }

    /// A list resting on a row paints from that row and says so, and one
    /// at the top shows its oldest row (`MURMURE-190`, `MURMURE-191`).
    #[test]
    fn a_list_resting_on_a_row_paints_from_it() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let messages = (0..100).map(|i| message(ada, i, "hello")).collect();
        list.absorb(SegmentId::new(), messages);
        let at = |anchor| Shown {
            list: &list,
            unread: None,
            anchor: Some(anchor),
            focus: None,
            selecting: Selecting::default(),
        };
        let (_, painted) = paint_from(at((10, 0.0)), 200.0);
        assert_eq!(painted.rest, Some((10, 0.0)));
        assert!(!painted.latest_shown && !painted.oldest_shown);
        assert!(painted.laid_out < 20, "{painted:?}");
        let (_, top) = paint_from(at((0, 0.0)), 200.0);
        assert!(top.oldest_shown && !top.latest_shown);
        let (_, bottom) = paint(&list, 200.0);
        assert_eq!(bottom.rest, None);
        assert!(bottom.latest_shown && !bottom.oldest_shown);
    }

    /// The rows paint top to bottom in the order of the list, under one
    /// header for the group (`MURMURE-182`, `MURMURE-183`).
    #[test]
    fn the_rows_paint_top_to_bottom_under_one_header() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let bodies = ["a", "abcd", "abcdefgh"];
        let messages = bodies.iter().map(|b| message(ada, 0, b)).collect();
        list.absorb(SegmentId::new(), messages);
        let (surface, painted) = paint(&list, 600.0);
        assert_eq!(painted.laid_out, 3);
        let counts: Vec<usize> = runs(&surface, LIGHT.cell_text)
            .iter()
            .map(|r| r.1)
            .collect();
        assert_eq!(counts, [3, 1, 4, 8]);
        assert_eq!(runs(&surface, LIGHT.header_text).len(), 1);
    }

    /// A deleted body paints muted, and a reply count in the accent
    /// (`MURMURE-185`).
    #[test]
    fn a_deleted_body_paints_muted_and_a_reply_count_in_the_accent() {
        let mut gone = message(UserId::new(), 0, "");
        gone.deleted = true;
        gone.thread = Some(ThreadHead {
            document: DocumentId::new(),
            replies: 2,
            last_replier: None,
        });
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![gone]);
        let (surface, _) = paint(&list, 600.0);
        let author: usize = runs(&surface, LIGHT.cell_text).iter().map(|r| r.1).sum();
        assert_eq!(author, 3);
        let muted: usize = runs(&surface, LIGHT.header_text).iter().map(|r| r.1).sum();
        assert_eq!(muted, 1 + crate::list::DELETED.chars().count());
        assert!(!runs(&surface, LIGHT.selection).is_empty());
    }

    /// A sent body draws its marks: bold shapes heavier, code sits on its
    /// tint, a link takes the accent over a rule (`MURMURE-565`). A renamed
    /// mention before them does not shift them.
    #[test]
    fn a_sent_body_draws_each_of_its_marks() {
        let mut sent = message(UserId::new(), 0, "@bo hi bold c d");
        sent.mentions = vec![Mention {
            target: MentionTarget::User(UserId::new()),
            range: 0..3,
        }];
        let mark = |style, range| StyleMark { style, range };
        sent.styles = vec![
            mark(TextStyle::Link("https://ambre.dev".to_owned()), 14..15),
            mark(TextStyle::Bold, 7..11),
            mark(TextStyle::Code, 12..13),
        ];
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![sent]);
        let rows = list.rows();
        let first = rows.first().expect("one row");
        let row = first.words(|_| Some("Ada".to_owned()), |_| "t".to_owned());
        assert_eq!(row.body, "@Ada hi bold c d");
        let mut marks: Vec<(usize, usize)> = row
            .marks
            .iter()
            .map(|m| (m.bytes.start, m.bytes.end))
            .collect();
        marks.sort_unstable();
        assert_eq!(marks, [(8, 12), (13, 14), (15, 16)]);
        let (mut font_cx, mut cache) = (fonts(), TextCache::new(1.0));
        let plain = shape(&mut font_cx, &mut cache, &row.body, FONT_BODY, 384.0, false).width();
        assert!(body(&mut font_cx, &mut cache, &row, 384.0).width() > plain);

        let (surface, _) = paint(&list, 600.0);
        let ops = surface.ops();
        let fill = |want: Color| {
            ops.iter().position(
                |op| matches!(op, DrawOp::FillRect { color, rect } if *color == want && rect.height() < 3.0),
            )
        };
        let tint = ops
            .iter()
            .position(
                |op| matches!(op, DrawOp::FillRect { color, .. } if *color == LIGHT.header_hover),
            )
            .expect("the code tint");
        let first_glyph = ops
            .iter()
            .position(|op| matches!(op, DrawOp::Glyphs { .. }))
            .expect("the words");
        assert!(tint < first_glyph, "the tint lies under the words");
        let link: usize = runs(&surface, LIGHT.selection).iter().map(|r| r.1).sum();
        assert_eq!(link, 1, "only the link takes the accent");
        assert!(fill(LIGHT.selection).is_some(), "the link has its rule");
    }

    /// The unread line paints in the accent between the read message and
    /// the first unread one, and once (`MURMURE-184`).
    #[test]
    fn the_unread_line_paints_above_the_first_unread_message() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let messages: Vec<_> = ["a", "abcd"].iter().map(|b| message(ada, 0, b)).collect();
        let unread = messages.get(1).map(|m| m.id);
        list.absorb(SegmentId::new(), messages);
        let (plain, _) = paint(&list, 600.0);
        assert!(runs(&plain, LIGHT.selection).is_empty());

        let (surface, painted) = paint_above(&list, unread, 600.0);
        assert_eq!(painted.laid_out, 2);
        assert!(painted.line_shown);
        let line = runs(&surface, LIGHT.selection);
        let [(line_y, glyphs)] = line.as_slice() else {
            panic!("one run of the line, got {line:?}");
        };
        assert_eq!(*glyphs, crate::list::UNREAD_LINE.chars().count());
        let bodies = runs(&surface, LIGHT.cell_text);
        let [_, (read_y, 1), (unread_y, 4)] = bodies.as_slice() else {
            panic!("the author, then both bodies, got {bodies:?}");
        };
        assert!(read_y < line_y && line_y < unread_y);
        let rules = surface
            .fill_colors()
            .iter()
            .filter(|c| **c == LIGHT.selection)
            .count();
        assert_eq!(rules, 1);
    }

    /// A band paints as one muted row, with no header, and the message
    /// after it paints under a header of its own (`MURMURE-218`).
    #[test]
    fn a_band_paints_as_one_muted_row() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![message(ada, 30, "abcd")]);
        let band = crate::list::Band {
            cause: ambre_bus::Unreadable::Damaged,
            from: Timestamp(10),
            to: Some(Timestamp(30)),
        };
        list.unreadable(SegmentId::new(), band);
        let (surface, painted) = paint(&list, 600.0);
        assert_eq!(painted.laid_out, 2);
        let muted = runs(&surface, LIGHT.header_text);
        let body = runs(&surface, LIGHT.cell_text);
        let [(band_y, _), .., (time_y, 1)] = muted.as_slice() else {
            panic!("the band, then the time of the header, got {muted:?}");
        };
        let [(author_y, 3), (body_y, 4)] = body.as_slice() else {
            panic!("the author, then the body, got {body:?}");
        };
        assert!(band_y < author_y && author_y <= time_y && time_y < body_y);
    }

    /// `MURMURE-425`: a press and a drag resolve into a selection. It runs
    /// from the first byte of one row to the end of the next, tinted behind
    /// both bodies. A drag below every row ends on the last one.
    #[test]
    fn a_drag_selects_text_across_rows_behind_their_words() {
        let ada = UserId::new();
        let mut list = MessageList::default();
        let bodies = ["hello", "brave", "world"];
        let messages = bodies.iter().map(|b| message(ada, 0, b)).collect();
        list.absorb(SegmentId::new(), messages);
        let keys = list.keys();
        let (_, plain) = paint(&list, 600.0);
        let [(_, _, first), (_, top, second), ..] = plain.rows.as_slice() else {
            panic!("three rows: {plain:?}");
        };
        let drag = |press, drag| Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: None,
            selecting: Selecting {
                selection: None,
                press: Some(press),
                drag: Some(drag),
            },
        };
        let press = (SPACE_8, first - SPACE_2 - 2.0);
        let (surface, painted) = paint_from(drag(press, (399.0, (top + second) / 2.0)), 600.0);
        let selection = painted.selection.expect("a selection");
        assert_eq!(
            selection.anchor,
            Spot {
                row: keys[0],
                byte: 0
            }
        );
        assert_eq!(
            selection.focus,
            Spot {
                row: keys[1],
                byte: 5
            }
        );
        let tint = LIGHT.selection.with_alpha(SELECTION_ALPHA);
        let tints = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == tint))
            .count();
        assert_eq!(tints, 2, "one band behind each body");

        let (_, below) = paint_from(drag(press, (399.0, 599.0)), 600.0);
        let focus = below.selection.expect("a selection").focus;
        assert_eq!(
            focus,
            Spot {
                row: keys[2],
                byte: 5
            }
        );
        let (_, none) = paint_from(drag(press, press), 600.0);
        let collapsed = none.selection.expect("a collapsed selection");
        assert_eq!(collapsed.anchor, collapsed.focus);
    }

    /// An empty list paints its backdrop and shows no latest message.
    #[test]
    fn an_empty_list_paints_only_its_backdrop() {
        let (surface, painted) = paint(&MessageList::default(), 200.0);
        assert_eq!(
            painted,
            Painted {
                laid_out: 0,
                latest_shown: false,
                oldest_shown: false,
                line_shown: false,
                rest: None,
                top: None,
                bottom: None,
                focus: None,
                rows: Vec::new(),
                selection: None,
            }
        );
        assert!(
            surface
                .ops()
                .iter()
                .all(|op| matches!(op, DrawOp::FillRect { .. }))
        );
    }
}
