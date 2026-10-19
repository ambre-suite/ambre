//! The painter of the message list (the Murmure message list spec).
//!
//! It lays out only the rows a viewport reaches from the bottom, with the
//! text pipeline every application shares (`MURMURE-187`, `MURMURE-188`).
//! It paints them on any drawing surface: the page's canvas, or the
//! recording surface of a host test.

use std::collections::HashMap;
use std::ops::Range;

use ambre_bus::{Attachment, MediaRef, MessageId, TextStyle, Timestamp, UserId};
use ambre_canvas::theme::{FONT_BODY, FONT_UI, SPACE_2, SPACE_4, SPACE_8};
use ambre_canvas::{Palette, SpanShape, TextCache};
use ambre_draw::{
    Affine, Affinity, Alignment, Brush, Color, Cursor, DrawSurface, FontContext, ImageSource,
    Layout, ParleyBrush, PositionedLayoutItem, Rect, Selection,
};

use crate::list::{
    JUMP_TO_LATEST, MessageList, RowKey, RowWords, UNREAD_LINE, chip_label, file_label, window,
};
use crate::look;
use crate::pictures::{Pictures, frame_at, frame_key, previews};
use crate::select::{Selecting, Spot, TextSelection};

/// How strongly a selection tints the text under it.
const SELECTION_ALPHA: f32 = 0.3;
/// The height of the box an image previews in. A picture not decoded yet
/// takes the same, so its row keeps its height when it lands.
pub const PREVIEW_TALL: f32 = 160.0;
/// The aspect of the box a picture not decoded yet holds.
const PLACEHOLDER_ASPECT: f32 = 4.0 / 3.0;

/// The pictures a paint previews, and the clock that plays them.
#[derive(Clone, Copy)]
pub struct Pictured<'a> {
    pub shelf: &'a Pictures,
    pub now_ms: f64,
    /// The user asks for reduced motion: each picture shows its first frame
    /// (`MURMURE-130`).
    pub still: bool,
}

impl std::fmt::Debug for Pictured<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pictured")
            .field("now_ms", &self.now_ms)
            .field("still", &self.still)
            .finish_non_exhaustive()
    }
}

/// What the previews of one paint drew: the files they show, and when the
/// next frame of one is due.
#[derive(Debug, Default)]
struct Played {
    files: Vec<MediaRef>,
    next: Option<f64>,
}

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
    /// The address of the link a click landed on (`MURMURE-567`).
    pub link: Option<String>,
    /// The reaction chip a click landed on: the index of its row, and its
    /// emoji.
    pub chip: Option<(usize, String)>,
    /// The file chip a click landed on: the file it fetches.
    pub file: Option<Attachment>,
    /// The files whose preview this paint drew, decoded or not. Only these
    /// are on screen, so only these play (`MURMURE-131`).
    pub pictures: Vec<MediaRef>,
    /// When the next frame of a picture on screen is due, `None` when none
    /// plays.
    pub next_frame: Option<f64>,
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
    /// The signed-in member, whose own reactions take the accent.
    pub me: Option<UserId>,
    /// The pictures to preview. `None` previews nothing: each file shows
    /// as its chip alone.
    pub pictures: Option<Pictured<'a>>,
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
        let mut tall = measure(font_cx, cache, &row, shown.pictures, text_width);
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
    let link = selecting
        .click
        .and_then(|at| link_at(font_cx, cache, &reached.rows, &words, at, text_width));
    let pics = shown.pictures;
    let chip = selecting
        .click
        .and_then(|at| chip_at(font_cx, cache, &reached.rows, &words, at, text_width, pics));
    let file = selecting
        .click
        .and_then(|at| file_at(font_cx, cache, &reached.rows, &words, at, text_width, pics));
    let mut played = Played::default();
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
        let blocks = row
            .marks
            .iter()
            .any(|mark| matches!(mark.style, TextStyle::Quote | TextStyle::List));
        if band.is_some() || !tints.is_empty() || blocks {
            let body_y = body_top(font_cx, cache, row, *line, top, text_width);
            let layout = body(font_cx, cache, row, text_width);
            // The tints go first, so a selection band stays visible over code.
            for (bytes, color) in tints {
                fill_bytes(surface, layout, bytes, body_y, color);
            }
            let (x, y) = (f64::from(SPACE_8), f64::from(body_y));
            for mark in &row.marks {
                for at in look::margin(layout, &row.body, &mark.style, mark.bytes.clone()) {
                    let at = Rect::new(at.x0 + x, at.y0 + y, at.x1 + x, at.y1 + y);
                    surface.fill_rect(at, look::margin_ink(palette));
                }
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
        draw_row(
            surface,
            font_cx,
            cache,
            row,
            top,
            text_width,
            palette,
            shown.me,
            pics,
            &mut played,
        );
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
        link,
        chip,
        file,
        pictures: played.files,
        next_frame: played.next,
    }
}

/// The reaction chip drawn under `(x, y)`: the index of its row and its
/// emoji. `None` off every chip.
fn chip_at(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    rows: &[(usize, f32, f32)],
    words: &HashMap<usize, (RowWords, bool)>,
    (x, y): (f32, f32),
    width: f32,
    pics: Option<Pictured<'_>>,
) -> Option<(usize, String)> {
    let &(i, top, _) = rows
        .iter()
        .find(|&&(_, top, tall)| top <= y && y < top + tall)?;
    let (row, line) = words.get(&i)?;
    if row.reactions.is_empty() {
        return None;
    }
    // The reactions sit under the previews and the files.
    let chips_y = body_top(font_cx, cache, row, *line, top, width)
        + body(font_cx, cache, row, width).height()
        + previews_height(row, pics)
        + file_chips(font_cx, cache, row, width).1;
    let (boxes, _) = chips(font_cx, cache, row, width);
    let (x, y) = (f64::from(x - SPACE_8), f64::from(y - chips_y));
    boxes
        .iter()
        .zip(&row.reactions)
        .find(|(b, _)| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1)
        .map(|(_, reaction)| (i, reaction.emoji.clone()))
}

/// The box of each reaction chip of a row, from the top-left of the chips,
/// and the height they take with the space above them. The chips run left
/// to right and wrap past `width`.
fn chips(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    width: f32,
) -> (Vec<Rect>, f32) {
    let labels: Vec<String> = row.reactions.iter().map(chip_label).collect();
    chip_boxes(font_cx, cache, &labels, width)
}

/// The box of each file chip of a row, as [`chips`] gives the reactions.
fn file_chips(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    width: f32,
) -> (Vec<Rect>, f32) {
    let labels: Vec<String> = row.files.iter().map(file_label).collect();
    chip_boxes(font_cx, cache, &labels, width)
}

/// The box of a chip for each of `labels`, from the top-left of the first,
/// and the height they take with the space above them.
fn chip_boxes(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    labels: &[String],
    width: f32,
) -> (Vec<Rect>, f32) {
    if labels.is_empty() {
        return (Vec::new(), 0.0);
    }
    let mut boxes = Vec::with_capacity(labels.len());
    let (mut x, mut y, mut line) = (0.0_f32, SPACE_2, 0.0_f32);
    for label in labels {
        let layout = shape(font_cx, cache, label, FONT_UI, width, false);
        let (wide, tall) = (
            layout.width() + 2.0 * SPACE_4,
            layout.height() + 2.0 * SPACE_2,
        );
        if x > 0.0 && x + wide > width {
            (x, y) = (0.0, y + line + SPACE_2);
            line = 0.0;
        }
        boxes.push(Rect::new(
            f64::from(x),
            f64::from(y),
            f64::from(x + wide),
            f64::from(y + tall),
        ));
        x += wide + SPACE_4;
        line = line.max(tall);
    }
    (boxes, y + line)
}

/// Paint the reaction chips of a row, their top-left at `(x, y)`. A chip
/// that holds `me` takes the accent (`MURMURE-137`).
#[allow(clippy::too_many_arguments)]
fn draw_chips<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    (x, y): (f32, f32),
    width: f32,
    palette: &Palette,
    me: Option<UserId>,
) {
    let (boxes, _) = chips(font_cx, cache, row, width);
    let (dx, dy) = (f64::from(x), f64::from(y));
    for (reaction, b) in row.reactions.iter().zip(boxes) {
        let at = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        let mine = me.is_some_and(|me| reaction.users.contains(&me));
        if mine {
            surface.fill_rect(at, palette.selection.with_alpha(SELECTION_ALPHA));
            surface.stroke_rect(at, palette.selection, 1.0);
        } else {
            surface.fill_rect(at, palette.header_bg);
            surface.stroke_rect(at, palette.header_border, 1.0);
        }
        let layout = shape(font_cx, cache, &chip_label(reaction), FONT_UI, width, false);
        #[allow(clippy::cast_possible_truncation)]
        let (left, top) = (at.x0 as f32 + SPACE_4, at.y0 as f32 + SPACE_2);
        draw(surface, layout, left, top, palette.cell_text);
    }
}

/// Paint the file chips of a row, their top-left at `(x, y)`, in the ink of
/// a link: a click fetches the file.
fn draw_files<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    (x, y): (f32, f32),
    width: f32,
    palette: &Palette,
) {
    let (boxes, _) = file_chips(font_cx, cache, row, width);
    let (dx, dy) = (f64::from(x), f64::from(y));
    for (file, b) in row.files.iter().zip(boxes) {
        let at = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        surface.fill_rect(at, palette.header_bg);
        surface.stroke_rect(at, palette.header_border, 1.0);
        let layout = shape(font_cx, cache, &file_label(file), FONT_UI, width, false);
        #[allow(clippy::cast_possible_truncation)]
        let (left, top) = (at.x0 as f32 + SPACE_4, at.y0 as f32 + SPACE_2);
        draw(surface, layout, left, top, palette.selection);
    }
}

/// The preview or the file chip drawn under `(x, y)`: the file a click
/// fetches. `None` off every one.
fn file_at(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    rows: &[(usize, f32, f32)],
    words: &HashMap<usize, (RowWords, bool)>,
    (x, y): (f32, f32),
    width: f32,
    pics: Option<Pictured<'_>>,
) -> Option<Attachment> {
    let &(i, top, _) = rows
        .iter()
        .find(|&&(_, top, tall)| top <= y && y < top + tall)?;
    let (row, line) = words.get(&i)?;
    if row.files.is_empty() {
        return None;
    }
    let previews_y = body_top(font_cx, cache, row, *line, top, width)
        + body(font_cx, cache, row, width).height();
    let (x, y) = (f64::from(x - SPACE_8), f64::from(y - previews_y));
    let hit = |b: &Rect| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1;
    if let Some((file, _)) = preview_boxes(row, pics, width)
        .into_iter()
        .find(|(_, b)| hit(b))
    {
        return Some(file.clone());
    }
    let y = y - f64::from(previews_height(row, pics));
    let (boxes, _) = file_chips(font_cx, cache, row, width);
    boxes
        .iter()
        .zip(&row.files)
        .find(|(b, _)| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1)
        .map(|(_, file)| file.clone())
}

/// The address of the link drawn under `(x, y)`, `None` off any link. An
/// address the write rule would refuse opens nothing (`MURMURE-561`).
fn link_at(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    rows: &[(usize, f32, f32)],
    words: &HashMap<usize, (RowWords, bool)>,
    (x, y): (f32, f32),
    width: f32,
) -> Option<String> {
    let &(i, top, _) = rows
        .iter()
        .find(|&&(_, top, tall)| top <= y && y < top + tall)?;
    let (row, line) = words.get(&i)?;
    let body_y = body_top(font_cx, cache, row, *line, top, width);
    let layout = body(font_cx, cache, row, width);
    let (x, y) = (f64::from(x - SPACE_8), f64::from(y - body_y));
    row.marks.iter().find_map(|mark| {
        let TextStyle::Link(address) = &mark.style else {
            return None;
        };
        let from = Cursor::from_byte_index(layout, mark.bytes.start, Affinity::Downstream);
        let to = Cursor::from_byte_index(layout, mark.bytes.end, Affinity::Upstream);
        let hit = Selection::new(from, to)
            .geometry(layout)
            .iter()
            .any(|(bb, _)| bb.x0 <= x && x < bb.x1 && bb.y0 <= y && y < bb.y1);
        (hit && crate::style::linkable(address)).then(|| address.clone())
    })
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

/// The files of a row that preview: a type the Logic thread decodes, and
/// not failed. None when the paint previews nothing.
fn previewed<'r>(row: &'r RowWords, pics: Option<Pictured<'_>>) -> Vec<&'r Attachment> {
    let Some(pics) = pics else {
        return Vec::new();
    };
    row.files
        .iter()
        .filter(|file| previews(&file.media_type) && !pics.shelf.failed(&file.media_ref))
        .collect()
}

/// The box of each preview of a row, from the top-left of the first, one
/// under the other. A picture keeps its aspect within `width`. One not
/// decoded yet takes [`PLACEHOLDER_ASPECT`].
fn preview_boxes<'r>(
    row: &'r RowWords,
    pics: Option<Pictured<'_>>,
    width: f32,
) -> Vec<(&'r Attachment, Rect)> {
    let mut y = 0.0_f32;
    let mut boxes = Vec::new();
    for file in previewed(row, pics) {
        let aspect = pics
            .and_then(|pics| pics.shelf.get(&file.media_ref))
            .filter(|picture| picture.width > 0 && picture.height > 0)
            .map_or(PLACEHOLDER_ASPECT, |picture| {
                #[allow(clippy::cast_precision_loss)]
                let aspect = picture.width as f32 / picture.height as f32;
                aspect
            });
        y += SPACE_2;
        let mut wide = PREVIEW_TALL * aspect;
        let mut tall = PREVIEW_TALL;
        if wide > width {
            (wide, tall) = (width, width / aspect);
        }
        boxes.push((
            file,
            Rect::new(0.0, f64::from(y), f64::from(wide), f64::from(y + tall)),
        ));
        y += PREVIEW_TALL;
    }
    boxes
}

/// The height the previews of a row take, the space above each included.
/// Each takes [`PREVIEW_TALL`], so a row keeps its height when one lands.
fn previews_height(row: &RowWords, pics: Option<Pictured<'_>>) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let count = previewed(row, pics).len() as f32;
    count * (SPACE_2 + PREVIEW_TALL)
}

/// Paint the previews of a row, their top-left at `(x, y)`. A decoded
/// picture draws the frame its clock reaches. One not decoded yet draws
/// its box empty. `played` records each file and the next frame due.
fn draw_previews<S: DrawSurface>(
    surface: &mut S,
    row: &RowWords,
    pics: Option<Pictured<'_>>,
    (x, y): (f32, f32),
    width: f32,
    palette: &Palette,
    played: &mut Played,
) {
    let Some(pictured) = pics else {
        return;
    };
    let (dx, dy) = (f64::from(x), f64::from(y));
    for (file, b) in preview_boxes(row, pics, width) {
        let at = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        played.files.push(file.media_ref.clone());
        let Some(picture) = pictured.shelf.get(&file.media_ref) else {
            surface.fill_rect(at, palette.header_bg);
            surface.stroke_rect(at, palette.header_border, 1.0);
            continue;
        };
        let (index, next) = frame_at(picture, pictured.now_ms, pictured.still);
        let Some(frame) = picture.frames.get(index) else {
            continue;
        };
        let image = ImageSource {
            key: frame_key(&file.media_ref, index),
            pixels: &frame.pixels,
            width: picture.width,
            height: picture.height,
        };
        surface.draw_image(&image, at);
        played.next = match (played.next, next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

/// The height of a row, from the same layouts [`draw_row`] paints.
fn measure(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    pics: Option<Pictured<'_>>,
    width: f32,
) -> f32 {
    let mut tall = 2.0 * SPACE_2 + header_height(font_cx, cache, row, width);
    tall += body(font_cx, cache, row, width).height();
    tall += previews_height(row, pics);
    tall += file_chips(font_cx, cache, row, width).1;
    tall += chips(font_cx, cache, row, width).1;
    if let Some(replies) = &row.replies {
        tall += shape(font_cx, cache, replies, FONT_UI, width, false).height();
    }
    tall
}

/// Paint one row whose top is at `top`.
#[allow(clippy::too_many_arguments)]
fn draw_row<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    top: f32,
    width: f32,
    palette: &Palette,
    me: Option<UserId>,
    pics: Option<Pictured<'_>>,
    played: &mut Played,
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
    draw_previews(surface, row, pics, (x, y), width, palette, played);
    y += previews_height(row, pics);
    draw_files(surface, font_cx, cache, row, (x, y), width, palette);
    y += file_chips(font_cx, cache, row, width).1;
    draw_chips(surface, font_cx, cache, row, (x, y), width, palette, me);
    y += chips(font_cx, cache, row, width).1;
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
        // The code face too, as the app ships it, so code shapes in it.
        let faces = [
            "NotoSans-Regular.ttf",
            "NotoSans-Bold.ttf",
            "DejaVuSansMono.ttf",
        ]
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
                me: None,
                pictures: None,
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
            me: None,
            pictures: None,
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
            me: None,
            pictures: None,
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

    /// The reactions of a message paint as one chip each under its body,
    /// the member's own in the accent. The row grows to hold them. A click
    /// on a chip names it; a deleted message shows none.
    #[test]
    fn the_reactions_paint_as_chips_under_the_body() {
        use ambre_bus::Reaction;
        let (me, ada) = (UserId::new(), UserId::new());
        let mut reacted = message(ada, 0, "hello");
        reacted.reactions = vec![
            Reaction {
                emoji: "\u{1F44D}".to_owned(),
                users: vec![ada],
            },
            Reaction {
                emoji: "\u{1F389}".to_owned(),
                users: vec![ada, me],
            },
        ];
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![reacted.clone()]);
        let mut bare = MessageList::default();
        bare.absorb(SegmentId::new(), vec![message(ada, 0, "hello")]);
        let tall = |painted: &Painted| painted.rows[0].2 - painted.rows[0].1;
        let (_, plain) = paint(&bare, 600.0);

        let (_, at) = paint(&list, 600.0);
        let bottom = at.rows[0].2;
        let click = (SPACE_8 + 2.0, bottom - 2.0 * SPACE_2 - 2.0);
        let selecting = Selecting {
            click: Some(click),
            ..Selecting::default()
        };
        let shown = Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: None,
            selecting,
            me: Some(me),
            pictures: None,
        };
        let (surface, painted) = paint_from(shown, 600.0);
        assert!(tall(&painted) > tall(&plain), "the chips take room");
        assert_eq!(painted.chip, Some((0, "\u{1F44D}".to_owned())));
        let fills = |color: Color| -> Vec<Rect> {
            surface
                .ops()
                .iter()
                .filter_map(|op| match op {
                    DrawOp::FillRect { rect, color: c } if *c == color => Some(*rect),
                    _ => None,
                })
                .collect()
        };
        let theirs = fills(LIGHT.header_bg);
        let mine = fills(LIGHT.selection.with_alpha(SELECTION_ALPHA));
        let ([theirs], [mine]) = (theirs.as_slice(), mine.as_slice()) else {
            panic!("one chip each: {theirs:?} {mine:?}");
        };
        assert!(theirs.x1 < mine.x0, "left to right");
        assert_eq!(theirs.y0, mine.y0, "on one line");
        assert!(mine.y1 <= f64::from(bottom));

        reacted.deleted = true;
        let mut gone = MessageList::default();
        gone.absorb(SegmentId::new(), vec![reacted]);
        let (surface, _) = paint(&gone, 600.0);
        let chips = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == LIGHT.header_bg))
            .count();
        assert_eq!(chips, 0, "a deleted message shows no chip");
    }

    /// The files of a message paint as one chip each under its body, in the
    /// ink of a link. A click on one names its file.
    #[test]
    fn the_files_paint_as_chips_a_click_names() {
        use ambre_bus::{Attachment, ByteSize, MediaRef, MediaType};
        let ada = UserId::new();
        let file = Attachment {
            media_ref: MediaRef::parse(&"0".repeat(64)).expect("an address"),
            name: "a.png".to_owned(),
            media_type: MediaType("image/png".to_owned()),
            size: ByteSize(2048),
        };
        let mut carrying = message(ada, 0, "hello");
        carrying.attachments = vec![file.clone()];
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![carrying]);
        let mut bare = MessageList::default();
        bare.absorb(SegmentId::new(), vec![message(ada, 0, "hello")]);
        let tall = |painted: &Painted| painted.rows[0].2 - painted.rows[0].1;
        let (_, plain) = paint(&bare, 600.0);
        let (_, at) = paint(&list, 600.0);
        assert!(tall(&at) > tall(&plain), "the chip takes room");

        let bottom = at.rows[0].2;
        let selecting = Selecting {
            click: Some((SPACE_8 + 2.0, bottom - 2.0 * SPACE_2 - 2.0)),
            ..Selecting::default()
        };
        let shown = Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: None,
            selecting,
            me: None,
            pictures: None,
        };
        let (surface, painted) = paint_from(shown, 600.0);
        assert_eq!(painted.file, Some(file));
        assert_eq!(painted.chip, None);
        assert!(!runs(&surface, LIGHT.selection).is_empty(), "link ink");
    }

    /// `MURMURE-136`, `MURMURE-131`: an image previews in a box of its own
    /// above its chip. Before it lands the box is empty, and the row keeps
    /// its height when it does. A paint draws the frame its clock reaches
    /// and names the next. Under reduced motion none is due (`MURMURE-130`).
    #[test]
    fn an_image_previews_in_a_box_that_keeps_its_height_and_plays() {
        use ambre_bus::{Attachment, ByteSize, MediaType, Picture, PictureFrame};
        let ada = UserId::new();
        let file = Attachment {
            media_ref: MediaRef::parse(&"a".repeat(64)).expect("an address"),
            name: "a.gif".to_owned(),
            media_type: MediaType("image/gif".to_owned()),
            size: ByteSize(2048),
        };
        let mut carrying = message(ada, 0, "hello");
        carrying.attachments = vec![file.clone()];
        let mut list = MessageList::default();
        list.absorb(SegmentId::new(), vec![carrying]);
        let mut shelf = Pictures::default();
        let at = |shelf: &Pictures, now_ms, still| {
            let shown = Shown {
                list: &list,
                unread: None,
                anchor: None,
                focus: None,
                selecting: Selecting::default(),
                me: None,
                pictures: Some(Pictured {
                    shelf,
                    now_ms,
                    still,
                }),
            };
            paint_from(shown, 600.0)
        };
        let images = |surface: &RecordingSurface| {
            surface
                .ops()
                .iter()
                .filter(|op| matches!(op, DrawOp::DrawImage { .. }))
                .count()
        };
        let tall = |painted: &Painted| painted.rows[0].2 - painted.rows[0].1;
        let (_, chip_only) = paint(&list, 600.0);
        let (surface, waiting) = at(&shelf, 0.0, false);
        assert_eq!(waiting.pictures, vec![file.media_ref.clone()]);
        assert_eq!(images(&surface), 0, "not decoded yet");
        assert!(tall(&waiting) >= tall(&chip_only) + PREVIEW_TALL);

        shelf.begin_paint(crate::pictures::Screen::Channel);
        assert!(shelf.want(&file.media_ref));
        let frame = |delay_ms| PictureFrame {
            pixels: vec![0; 2 * 4],
            delay_ms,
        };
        let gif = Picture {
            width: 2,
            height: 1,
            frames: vec![frame(100), frame(100)],
        };
        assert!(shelf.land(file.media_ref.clone(), Some(gif)));
        let (surface, playing) = at(&shelf, 30.0, false);
        assert_eq!(images(&surface), 1);
        assert_eq!(tall(&playing), tall(&waiting), "the row keeps its height");
        assert_eq!(playing.next_frame, Some(100.0));
        let (surface, still) = at(&shelf, 30.0, true);
        assert_eq!((images(&surface), still.next_frame), (1, None));
    }

    /// The colour emoji face is fetched the first time a painted row holds
    /// an emoji, in its body or on a chip. A list with none fetches nothing
    /// (`MURMURE-138`). The count is a host hook of the cache.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_emoji_face_is_asked_for_on_first_use_only() {
        let requested = |list: &MessageList| {
            let (mut surface, mut font_cx) = (RecordingSurface::new(), fonts());
            let mut cache = TextCache::new(1.0);
            let shown = Shown {
                list,
                unread: None,
                anchor: None,
                focus: None,
                selecting: Selecting::default(),
                me: None,
                pictures: None,
            };
            let palette = &LIGHT;
            paint_list(
                &mut surface,
                &mut font_cx,
                &mut cache,
                shown,
                (400.0, 600.0),
                palette,
                &|_| Some("Ada".to_owned()),
                &|_| "t".to_owned(),
            );
            cache.lazy_fonts_requested()
        };
        let ada = UserId::new();
        let mut plain = MessageList::default();
        plain.absorb(SegmentId::new(), vec![message(ada, 0, "no picture here")]);
        assert_eq!(requested(&plain), 0, "nothing fetched at start");
        let mut said = MessageList::default();
        said.absorb(SegmentId::new(), vec![message(ada, 0, "ship it \u{1F680}")]);
        assert_eq!(requested(&said), 1);
        let mut reacted = message(ada, 0, "ship it");
        reacted.reactions = vec![ambre_bus::Reaction {
            emoji: "\u{1F389}".to_owned(),
            users: vec![ada],
        }];
        let mut chipped = MessageList::default();
        chipped.absorb(SegmentId::new(), vec![reacted]);
        assert_eq!(requested(&chipped), 1, "a chip asks for it too");
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

    /// A sent quote draws one rule in the margin, and a list one dot per
    /// item, in the muted ink (`MURMURE-565`). A code block draws its tint
    /// and no margin mark.
    #[test]
    fn a_sent_block_draws_its_margin_marks() {
        let margin = |body: &str, styles: Vec<StyleMark>| {
            let mut sent = message(UserId::new(), 0, body);
            sent.styles = styles;
            let mut list = MessageList::default();
            list.absorb(SegmentId::new(), vec![sent]);
            let (surface, _) = paint(&list, 600.0);
            let marks = surface.count(|op| {
                matches!(op, DrawOp::FillRect { color, rect }
                    if *color == LIGHT.header_text && rect.x1 <= f64::from(SPACE_8))
            });
            let tints = surface.count(
                |op| matches!(op, DrawOp::FillRect { color, .. } if *color == LIGHT.header_hover),
            );
            (marks, tints)
        };
        let mark = |style, range| StyleMark { style, range };
        let blocks = vec![mark(TextStyle::Quote, 0..5), mark(TextStyle::List, 5..12)];
        assert_eq!(margin("said\none\ntwo", blocks), (3, 0), "a rule, two dots");
        let code = vec![mark(TextStyle::CodeBlock, 0..4)];
        assert_eq!(margin("fn a", code), (0, 1), "a tint, no margin mark");
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
                click: None,
            },
            me: None,
            pictures: None,
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
                link: None,
                chip: None,
                file: None,
                pictures: Vec::new(),
                next_frame: None,
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
