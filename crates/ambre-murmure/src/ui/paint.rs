//! The painter of the message list (the Murmure message list spec).
//!
//! It lays out only the rows a viewport reaches from the bottom, with the
//! text pipeline every application shares (`MURMURE-187`, `MURMURE-188`).
//! It paints them on any drawing surface: the page's canvas, or the
//! recording surface of a host test.

use ambre_core::Validated;
use ambre_core::index::{IdxVec, Len, Pos, Span};
use std::collections::HashMap;

use crate::message::{Attachment, TextStyle};
use ambre_bus::{Authored, ContentHash, Emoji, Instant};
use ambre_canvas::Theme;
use ambre_canvas::theme::{
    FONT_BODY, FONT_SMALL, FONT_UI, FONT_XS, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6,
    SPACE_8, SPACE_12,
};
use ambre_canvas::{Palette, SpanShape, TextCache};
use ambre_core::id::{Id, Message, User};
use ambre_core::{Byte, Px, Size, Surface, View};
use ambre_draw::{
    Affine, Affinity, BezPath, Brush, Color, Cursor, DrawSurface, FontContext, ImageSource, Layout,
    ParleyBrush, Point, PositionedLayoutItem, Rect, Selection, peer_color,
};

use crate::ui::list::{
    BodyInk, CALL_JOIN, CallCard, CallState, Face, HeaderWords, JUMP_TO_LATEST, ListRow,
    MessageList, OnScreen, RowKey, RowWords, ThreadWords, UNREAD_LINE, chip_label, file_label,
    window,
};
use crate::ui::look;
use crate::ui::pictures::{Pictures, frame_at, frame_key, previews};
use crate::ui::select::{Selecting, Spot, TextSelection};
use ambre_core::PageLoad;

/// How strongly a selection tints the text under it: faint, so the text
/// stays legible.
const SELECTION_OPACITY: ambre_core::Bounded<ambre_core::Opacity> =
    ambre_core::Bounded::<ambre_core::Opacity>::clamped(0.3);

/// The ink `selection` tints the text under a selection with.
fn selection_tint(selection: Color) -> Color {
    selection.with_alpha(SELECTION_OPACITY.get())
}

/// The height of the box an image previews in. A picture not decoded yet
/// takes the same, so its row keeps its height when it lands.
pub const PREVIEW_TALL: Px<View> = Px::<View>::new(160.0);
/// The aspect of the box a picture not decoded yet holds.
const PLACEHOLDER_ASPECT: f32 = 4.0 / 3.0;

/// The pictures a paint previews, and the clock that plays them.
#[derive(Clone, Copy)]
pub struct Pictured<'a> {
    pub shelf: &'a Pictures,
    pub now: Instant<PageLoad>,
    /// The motion the user asks for: under reduced motion each picture
    /// shows its first frame (`MURMURE-130`).
    pub motion: crate::ui::pictures::Motion,
}

impl std::fmt::Debug for Pictured<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pictured")
            .field("now", &self.now)
            .field("motion", &self.motion)
            .finish_non_exhaustive()
    }
}

/// What the previews of one paint drew: the files they show, and when the
/// next frame of one is due.
#[derive(Debug, Default)]
struct Played {
    files: Vec<Validated<ContentHash>>,
    next: Option<Instant<PageLoad>>,
}

/// The top and the bottom of a row on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowBounds {
    pub top: Px<View>,
    pub bottom: Px<View>,
}

impl RowBounds {
    /// Whether the height `y` falls on the row, its bottom left out.
    pub fn holds(self, y: Px<View>) -> bool {
        self.top <= y && y < self.bottom
    }
}

/// A row on screen: its index in the list, and where it lies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintedRow {
    pub index: Pos<ListRow>,
    pub bounds: RowBounds,
}

/// A row the paint laid out: its index in the list, its top and its height.
#[derive(Debug, Clone, Copy)]
struct LaidRow {
    index: Pos<ListRow>,
    top: Px<View>,
    tall: Px<View>,
}

impl LaidRow {
    /// Whether the height `y` falls on the row, its bottom left out.
    fn holds(self, y: Px<View>) -> bool {
        self.top <= y && y < self.top + self.tall
    }
}

/// A length of the painter, on the surface. The painter draws from the top
/// left of the surface, so the two spaces share their origin; only the
/// float widens.
pub(crate) fn surface_px(length: Px<View>) -> Px<Surface> {
    length.on_surface(Px::<Surface>::ZERO)
}

/// A coordinate of the surface, as a length of the painter: the inverse of
/// [`surface_px`]. Only the float narrows.
pub(crate) fn painter_px(at: Px<Surface>) -> Px<View> {
    at.in_view(Px::<Surface>::ZERO)
}

/// The rectangle of the surface between the edges `left`, `top`, `right`
/// and `bottom` of the painter.
pub(crate) fn view_rect(left: Px<View>, top: Px<View>, right: Px<View>, bottom: Px<View>) -> Rect {
    Rect::new(
        surface_px(left).get(),
        surface_px(top).get(),
        surface_px(right).get(),
        surface_px(bottom).get(),
    )
}

/// The width a shaped text takes, in the painter's pixels.
pub(crate) fn text_width(layout: &Layout<ParleyBrush>) -> Px<View> {
    Px::<View>::new(layout.width())
}

/// How far below the top of a shaped text its first baseline lies.
fn first_baseline(layout: &Layout<ParleyBrush>) -> Px<View> {
    Px::<View>::new(
        layout
            .lines()
            .next()
            .map_or(0.0, |line| line.metrics().baseline),
    )
}

/// The height a shaped text takes, in the painter's pixels.
pub(crate) fn text_height(layout: &Layout<ParleyBrush>) -> Px<View> {
    Px::<View>::new(layout.height())
}

/// The paint context of a Murmure painter: the surface it draws on, the
/// shaping context, the shaped-text cache and the palette. The four travel
/// together through every painter of the crate.
pub struct Pen<'p, S> {
    /// The surface the painter draws on.
    pub surface: &'p mut S,
    /// The shaping context.
    pub font_cx: &'p mut FontContext,
    /// The shaped-text cache.
    pub cache: &'p mut TextCache,
    /// The colours of the theme.
    pub palette: &'p Palette,
}

/// What one paint of the list did.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    /// The rows laid out: the ones the viewport reaches, never the whole
    /// list.
    pub laid_out: Len<ListRow>,
    /// Whether the latest row is on screen.
    pub latest_shown: OnScreen,
    /// Whether the oldest row the list holds is on screen (`MURMURE-191`).
    pub oldest_shown: OnScreen,
    /// Whether the unread line is on screen (`MURMURE-184`).
    pub line_shown: OnScreen,
    /// The row the list rests on and its top, `None` on the latest row.
    pub rest: Option<(Pos<ListRow>, Px<View>)>,
    /// The first row on screen and its top, wherever the list rests.
    pub top: Option<(Pos<ListRow>, Px<View>)>,
    /// The last row on screen, wherever the list rests.
    pub bottom: Option<Pos<ListRow>>,
    /// The top and the bottom of the focused row, `None` when it is off
    /// screen (`MURMURE-205`).
    pub focus: Option<RowBounds>,
    /// Each row on screen, top first: its index, its top and its bottom. A
    /// press finds the row under it here.
    pub rows: Vec<PaintedRow>,
    /// The selection, with the points the paint resolved into it
    /// (`MURMURE-425`).
    pub selection: Option<TextSelection>,
    /// The address of the link a click landed on (`MURMURE-567`).
    pub link: Option<ambre_core::LinkUrl>,
    /// The reaction chip a click landed on: the index of its row, and its
    /// emoji.
    pub chip: Option<(Pos<ListRow>, Emoji)>,
    /// The file chip a click landed on: the file it fetches.
    pub file: Option<Attachment>,
    /// The `Join` button of a call card a click landed on: the index of its
    /// row and its call (`MURMURE-746`).
    pub join: Option<(Pos<ListRow>, Id<ambre_core::id::Call>)>,
    /// The files whose preview this paint drew, decoded or not. Only these
    /// are on screen, so only these play (`MURMURE-131`).
    pub pictures: Vec<Validated<ContentHash>>,
    /// When the next frame of a picture on screen is due, `None` when none
    /// plays.
    pub next_frame: Option<Instant<PageLoad>>,
}

impl Painted {
    /// The index of the row under the height `y`, `None` between rows or
    /// past them.
    pub fn row_at(&self, y: Px<View>) -> Option<Pos<ListRow>> {
        self.rows
            .iter()
            .find(|row| row.bounds.holds(y))
            .map(|row| row.index)
    }
}

/// The day of a message in the reader's zone, as the page names it
/// (`MURMURE-739`).
///
/// `Debug` prints the start alone.
#[derive(Clone, PartialEq, Eq)]
pub struct Day {
    /// The start of the day, in the reader's zone: two messages of one day
    /// share it.
    pub start: Instant<Authored>,
    /// The date the divider writes when the day is neither today nor
    /// yesterday.
    pub written: String,
}

impl std::fmt::Debug for Day {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Day")
            .field("start", &self.start)
            .finish_non_exhaustive()
    }
}

/// The reader's clock and calendar, which the day dividers and the age of a
/// last reply read (`MURMURE-739`, `MURMURE-742`).
#[derive(Clone, Copy)]
pub struct Calendar<'a> {
    /// The wall clock at this paint.
    pub now: Instant<Authored>,
    /// The day of an instant, in the reader's zone.
    pub day_of: &'a dyn Fn(Instant<Authored>) -> Day,
}

impl std::fmt::Debug for Calendar<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Calendar")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// What the list paints: the messages, the first unread one, which the
/// unread line sits above (`MURMURE-184`), and the row it rests on.
#[derive(Debug, Clone, Copy)]
pub struct Shown<'a> {
    pub list: &'a MessageList,
    pub unread: Option<Id<Message>>,
    /// The row at the top and its top, `None` to rest on the latest row.
    pub anchor: Option<(Pos<ListRow>, Px<View>)>,
    /// The row that holds the focus, ringed in the accent (`MURMURE-205`).
    pub focus: Option<Pos<ListRow>>,
    /// The selection, and the points of a drag the paint resolves.
    pub selecting: Selecting,
    /// The signed-in member, whose own reactions take the accent.
    pub me: Option<Id<User>>,
    /// The pictures to preview. `None` previews nothing: each file shows
    /// as its chip alone.
    pub pictures: Option<Pictured<'a>>,
    /// The theme, whose chrome colours the decorations of a row share.
    pub theme: Theme,
    /// The reader's calendar. `None` draws no day divider, and a last reply
    /// with no age.
    pub calendar: Option<Calendar<'a>>,
}

/// The left edge of the avatars and of the dividers: the mockup's gutter.
const EDGE: Px<View> = SPACE_8;
/// The side of an avatar beside a header (`MURMURE-738`).
const AVATAR: Px<View> = Px::<View>::new(36.0);
/// The corner of an avatar beside a header.
const AVATAR_CORNER: Px<Surface> = Px::<Surface>::new(10.0);
/// The size of the initials in an avatar beside a header.
const AVATAR_FONT: Px<View> = FONT_UI;
/// The left edge of the text of a row: past the avatar and its gap.
pub(crate) const TEXT_LEFT: Px<View> = Px::<View>::new(EDGE.get() + AVATAR.get() + SPACE_5.get());
/// The space of the list above its oldest row.
const LIST_TOP: Px<View> = SPACE_4;
/// The space above a row that opens a group, and below every row.
pub(crate) const ROW_PAD: Px<View> = Px::<View>::new(5.0);
/// The space above a row that joins the group above it.
const JOINED_PAD: Px<View> = Px::<View>::new(1.0);
/// The space between a body and what hangs under it: the pills, the thread
/// summary, the call card.
const UNDER: Px<View> = Px::<View>::new(5.0);
/// The space above and below the label of a day divider.
const DAY_MARGIN: Px<View> = SPACE_5;
/// The space above and below the unread separator.
const NEW_MARGIN: Px<View> = SPACE_3;
/// The space between two pills.
pub(crate) const PILL_GAP: Px<View> = Px::<View>::new(5.0);
/// The side of a face in a thread summary.
const THREAD_FACE: Px<View> = Px::<View>::new(20.0);
/// The corner of a face in a thread summary.
const THREAD_FACE_CORNER: Px<Surface> = Px::<Surface>::new(6.0);
/// The size of the initials of a small face.
const FACE_FONT: Px<View> = Px::<View>::new(8.5);
/// The space around the faces of a thread summary.
const THREAD_PAD: Px<View> = Px::<View>::new(3.0);
/// The widest a call card grows.
const CARD_MAX: Px<View> = Px::<View>::new(520.0);
/// The space inside a call card.
const CARD_PAD_X: Px<View> = SPACE_6;
/// The space above and below the content of a call card.
const CARD_PAD_Y: Px<View> = SPACE_5;
/// The corner of a call card.
const CARD_CORNER: Px<Surface> = Px::<Surface>::new(12.0);
/// The side of the icon of a call card.
const CARD_ICON: Px<View> = Px::<View>::new(34.0);
/// The side of a face in a call card.
const CARD_FACE: Px<View> = SPACE_12;
/// How far a face in a call card slides over the one before it.
const CARD_FACE_OVERLAP: Px<View> = Px::<View>::new(7.0);
/// The size of the initials of a face in a call card.
const CARD_FACE_FONT: Px<View> = Px::<View>::new(9.5);
/// The height of the `Join` button of a call card.
const JOIN_TALL: Px<View> = Px::<View>::new(30.0);
/// A card narrower than this drops its faces, as the phone mockup does.
const CARD_FACES_FROM: Px<View> = Px::<View>::new(420.0);

/// The phone of the mockup's icon set, on a 24 px box.
const PHONE_PATH: &str =
    "M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2";

/// The chrome colours of the decorations, mixed as the mockup mixes them.
struct Inks {
    text: Color,
    faint: Color,
    dim: Color,
    line: Color,
    sunken: Color,
    surface: Color,
    accent: Color,
    accent_soft: Color,
    mention: Color,
    ok: Color,
}

impl Inks {
    fn of(theme: Theme) -> Self {
        let chrome = theme.chrome_palette();
        Self {
            text: chrome.text,
            faint: chrome.text_faint,
            dim: chrome.text_dim,
            line: chrome.border,
            sunken: chrome.surface_2,
            surface: chrome.surface,
            accent: chrome.accent,
            accent_soft: chrome.accent_soft,
            mention: chrome.mention,
            ok: chrome.success,
        }
    }
}

/// `ink` over `ground`, as CSS mixes `color-mix(ink share, ground)`.
fn mix(ink: Color, ground: Color, share: ambre_core::Bounded<ambre_core::Opacity>) -> Color {
    let (a, b) = (ink.to_rgba8(), ground.to_rgba8());
    let t = share.get();
    let channel = |x: u8, y: u8| (f32::from(x) * t + f32::from(y) * (1.0 - t)).round() as u8;
    Color::from_rgba8(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b), 255)
}

/// The share of the success ink in the ground of a live call card.
const CARD_GROUND: ambre_core::Bounded<ambre_core::Opacity> =
    ambre_core::Bounded::<ambre_core::Opacity>::clamped(0.1);
/// The share of the success ink in the border of a live call card.
const CARD_BORDER: ambre_core::Bounded<ambre_core::Opacity> =
    ambre_core::Bounded::<ambre_core::Opacity>::clamped(0.4);
/// The share of the mention ink in the ground of a mention chip.
const MENTION_GROUND: ambre_core::Bounded<ambre_core::Opacity> =
    ambre_core::Bounded::<ambre_core::Opacity>::clamped(0.12);

/// The outline of `rect` with corners of `corner`, as four arcs and four
/// sides. A corner wider than half a side is clamped to it, so a square
/// with half its side for a corner is a circle.
fn rounded(rect: Rect, corner: Px<Surface>) -> BezPath {
    let r = corner
        .get()
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    // The cubic that draws a quarter circle best.
    let k = r * 0.552_284_8;
    let (x0, y0, x1, y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    let mut path = BezPath::new();
    path.move_to((x0 + r, y0));
    path.line_to((x1 - r, y0));
    path.curve_to((x1 - r + k, y0), (x1, y0 + r - k), (x1, y0 + r));
    path.line_to((x1, y1 - r));
    path.curve_to((x1, y1 - r + k), (x1 - r + k, y1), (x1 - r, y1));
    path.line_to((x0 + r, y1));
    path.curve_to((x0 + r - k, y1), (x0, y1 - r + k), (x0, y1 - r));
    path.line_to((x0, y0 + r));
    path.curve_to((x0, y0 + r - k), (x0 + r - k, y0), (x0 + r, y0));
    path.close_path();
    path
}

/// Fill `rect` with corners of `corner` in `ground`, and draw its border in
/// `border` when given.
fn fill_rounded<S: DrawSurface>(
    surface: &mut S,
    rect: Rect,
    corner: Px<Surface>,
    ground: Color,
    border: Option<Color>,
) {
    let path = rounded(rect, corner);
    surface.fill_path(&path, &Brush::Solid(ground));
    if let Some(border) = border {
        surface.stroke_path(&path, &Brush::Solid(border), &ambre_draw::Stroke::new(1.0));
    }
}

/// Paint the face of `face` in `rect`: its initials on its peer colour
/// (`MURMURE-738`).
fn draw_face<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    face: &Face,
    rect: Rect,
    (corner, size): (Px<Surface>, Px<View>),
) {
    fill_rounded(surface, rect, corner, peer_color(&face.who), None);
    let layout = shape(
        font_cx,
        cache,
        &face.initials,
        size,
        Px::<View>::new(rect.width() as f32),
        ambre_canvas::Weight::Bold,
    );
    let (wide, tall) = (text_width(layout), text_height(layout));
    let left = painter_px(Px::<Surface>::new(rect.x0))
        + (Px::<View>::new(rect.width() as f32) - wide) / 2.0;
    let top = painter_px(Px::<Surface>::new(rect.y0))
        + (Px::<View>::new(rect.height() as f32) - tall) / 2.0;
    let white = Color::from_rgba8(255, 255, 255, 255);
    draw(
        surface,
        layout,
        &face.initials,
        ambre_core::Point::new(left, top),
        white,
    );
}

/// What a row draws above its header: a day divider, the unread separator,
/// or both.
#[derive(Debug, Clone, PartialEq, Default)]
struct Above {
    /// The label of the day divider, `None` inside a day (`MURMURE-739`).
    day: Option<String>,
    /// The unread separator: the first unread message follows
    /// (`MURMURE-184`).
    unread: Unread,
    /// The space above the row: the padding of the list above its oldest
    /// row, nothing above any other.
    lead: Px<View>,
}

ambre_core::define_flag! {
    /// Whether the unread separator stands above a row.
    enum Unread {
        /// It does not.
        No,
        /// The first unread message follows.
        Above,
    }
}

/// The words of the divider of `day`: today, yesterday or its date
/// (`MURMURE-739`).
fn day_label(day: &Day, today: &Day, yesterday: &Day) -> String {
    if day.start == today.start {
        ambre_commands::tr!("Today", "Aujourd'hui").to_owned()
    } else if day.start == yesterday.start {
        ambre_commands::tr!("Yesterday", "Hier").to_owned()
    } else {
        day.written.clone()
    }
}

/// The days a paint compares each message with.
struct Days<'a> {
    calendar: Calendar<'a>,
    today: Day,
    yesterday: Day,
}

impl<'a> Days<'a> {
    fn of(calendar: Calendar<'a>) -> Self {
        let today = (calendar.day_of)(calendar.now);
        let eve = Instant::<Authored>::new(today.start.get().saturating_sub(1));
        let yesterday = (calendar.day_of)(eve);
        Self {
            calendar,
            today,
            yesterday,
        }
    }

    /// The divider above `message`, given the message above it: one at the
    /// first message of a day.
    fn divider(
        &self,
        message: Instant<Authored>,
        above: Option<Instant<Authored>>,
    ) -> Option<String> {
        let day = (self.calendar.day_of)(message);
        let same = above.is_some_and(|above| (self.calendar.day_of)(above).start == day.start);
        (!same).then(|| day_label(&day, &self.today, &self.yesterday))
    }
}

/// Where each part of a row lies. The measure, the paint and each hit test
/// read this one walk of its layouts, so the three never drift.
#[derive(Debug, Clone, Copy)]
struct Placed {
    /// The top of the day divider's label.
    day: Option<Px<View>>,
    /// The top of the unread separator's label.
    unread: Option<Px<View>>,
    /// The top of the header and of the avatar beside it.
    header: Option<Px<View>>,
    /// The top of the body.
    body: Px<View>,
    /// The top of the previews.
    previews: Px<View>,
    /// The top of the file chips, the space above them included.
    files: Px<View>,
    /// The top of the pills.
    pills: Option<Px<View>>,
    /// The top of the thread summary.
    thread: Option<Px<View>>,
    /// The top of the call card.
    card: Option<Px<View>>,
    /// The top of the line that names the thread of a reply also sent.
    replies: Option<Px<View>>,
    /// The bottom of the row.
    bottom: Px<View>,
}

/// The heights of the two labels a row may draw above its header.
fn label_tall(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    size: Px<View>,
) -> Px<View> {
    text_height(shape(
        font_cx,
        cache,
        text,
        size,
        Px::<View>::new(1000.0),
        ambre_canvas::Weight::Bold,
    ))
}

/// Lay `row` out from `top`, in a list `width` wide.
fn place(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    above: &Above,
    top: Px<View>,
    width: Px<View>,
    pics: Option<Pictured<'_>>,
) -> Placed {
    let text = text_column(width);
    let mut y = top + above.lead;
    let day = above.day.as_ref().map(|label| {
        let at = y + DAY_MARGIN;
        y = at + label_tall(font_cx, cache, label, FONT_SMALL) + DAY_MARGIN;
        at
    });
    let unread = (above.unread == Unread::Above).then(|| {
        let at = y + NEW_MARGIN;
        let label = ambre_commands::shown(&UNREAD_LINE);
        y = at + label_tall(font_cx, cache, label, FONT_XS) + NEW_MARGIN;
        at
    });
    let opens = row.header.is_some() || row.ink == BodyInk::Muted;
    y += if opens { ROW_PAD } else { JOINED_PAD };
    let header = row.header.as_ref().map(|words| {
        let at = y;
        y += header_tall(font_cx, cache, words, text);
        at
    });
    let body = y;
    if row.call.is_none() {
        y += text_height(self::body(font_cx, cache, row, text));
    }
    let previews = y;
    y += previews_height(row, pics);
    let files = y;
    y += file_chips(font_cx, cache, row, text).1;
    let pills = (!row.reactions.is_empty()).then(|| {
        let at = y + UNDER;
        y = at + pill_boxes(font_cx, cache, row, text).1;
        at
    });
    let thread = row.thread.as_ref().map(|_| {
        let at = y + UNDER;
        y = at + THREAD_FACE + THREAD_PAD * 2.0;
        at
    });
    let card = row.call.as_ref().map(|card| {
        let at = y + if row.header.is_some() {
            UNDER
        } else {
            Px::<View>::ZERO
        };
        y = at + card_tall(font_cx, cache, card);
        at
    });
    let replies = match (&row.thread, &row.replies) {
        (None, Some(line)) => {
            let at = y;
            y += text_height(shape(
                font_cx,
                cache,
                line,
                FONT_UI,
                text,
                ambre_canvas::Weight::Regular,
            ));
            Some(at)
        }
        _ => None,
    };
    let mut bottom = y + ROW_PAD;
    if let Some(at) = header {
        bottom = bottom.max(at + AVATAR + ROW_PAD);
    }
    Placed {
        day,
        unread,
        header,
        body,
        previews,
        files,
        pills,
        thread,
        card,
        replies,
        bottom,
    }
}

/// The width the text of a row wraps to, in a list `width` wide.
fn text_column(width: Px<View>) -> Px<View> {
    (width - TEXT_LEFT - EDGE).max(Px::<View>::new(1.0))
}

/// The height of a header line: the name and the time beside it.
fn header_tall(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    words: &HeaderWords,
    width: Px<View>,
) -> Px<View> {
    let author = text_height(shape(
        font_cx,
        cache,
        &words.author,
        FONT_BODY,
        width,
        ambre_canvas::Weight::Bold,
    ));
    let time = text_height(shape(
        font_cx,
        cache,
        &words.time,
        FONT_SMALL,
        width,
        ambre_canvas::Weight::Regular,
    ));
    author.max(time)
}

/// Paint `shown` from its anchor, in a viewport of `size`.
///
/// `name_of` names a member, and `time_of` writes a time in the reader's
/// language and zone (`MURMURE-418`).
pub fn paint_list<S: DrawSurface>(
    pen: Pen<'_, S>,
    shown: Shown<'_>,
    size: Size<Px<View>>,
    name_of: &dyn Fn(Id<User>) -> Option<ambre_core::Validated<ambre_core::DisplayName>>,
    time_of: &dyn Fn(Instant<Authored>) -> String,
) -> Painted {
    let Pen {
        surface,
        font_cx,
        cache,
        palette,
    } = pen;
    let inks = Inks::of(shown.theme);
    let (width, height) = (size.width, size.height);
    let backdrop = view_rect(Px::<View>::ZERO, Px::<View>::ZERO, width, height);
    surface.fill_rect(backdrop, palette.content_bg);
    let rows = shown.list.rows();
    let text_width = text_column(width);
    let days = shown.calendar.map(Days::of);
    let pics = shown.pictures;
    let mut words: HashMap<Pos<ListRow>, (RowWords, Above)> = HashMap::new();
    let reached = window(rows.len(), height, shown.anchor, |i| {
        let Some(row) = rows.get(i) else {
            return Px::<View>::ZERO;
        };
        let unread = if shown.unread.is_some() && shown.unread == row.message().map(|m| m.id) {
            Unread::Above
        } else {
            Unread::No
        };
        let day = days
            .as_ref()
            .zip(row.message())
            .and_then(|(days, message)| {
                let previous = i.prev().and_then(|p| rows.get(p)).and_then(|r| r.message());
                days.divider(message.ts, previous.map(|m| m.ts))
            });
        let lead = if i == Pos::<ListRow>::ZERO {
            LIST_TOP
        } else {
            Px::<View>::ZERO
        };
        let above = Above { day, unread, lead };
        let row = row.words(name_of, time_of);
        let placed = place(font_cx, cache, &row, &above, Px::<View>::ZERO, width, pics);
        words.insert(i, (row, above));
        placed.bottom
    });
    // The rows the paint draws and hit-tests.
    let laid: Vec<LaidRow> = reached
        .rows
        .iter()
        .map(|&(index, top, tall)| LaidRow { index, top, tall })
        .collect();
    let selecting = shown.selecting;
    let active = selecting != Selecting::default();
    let keys = if active {
        shown.list.keys()
    } else {
        IdxVec::<ListRow, _>::new()
    };
    let mut selection = selecting.selection;
    let hit = Hit {
        rows: &laid,
        words: &words,
        width,
        pics,
    };
    if let Some(at) = selecting.press {
        selection = hit
            .spot(font_cx, cache, &keys, at)
            .map(|spot| TextSelection {
                anchor: spot,
                focus: spot,
            });
    }
    if let (Some(at), Some(selection)) = (selecting.drag, selection.as_mut())
        && let Some(spot) = hit.spot(font_cx, cache, &keys, at)
    {
        selection.focus = spot;
    }
    let link = selecting.click.and_then(|at| hit.link(font_cx, cache, at));
    let chip = selecting.click.and_then(|at| hit.chip(font_cx, cache, at));
    let file = selecting.click.and_then(|at| hit.file(font_cx, cache, at));
    let join = selecting
        .click
        .and_then(|at| hit.join(font_cx, cache, at, shown.me));
    let mut played = Played::default();
    let mut line_shown = OnScreen::Hidden;
    let mut focus = None;
    for &LaidRow {
        index: i,
        top,
        tall,
    } in &laid
    {
        let Some((row, above)) = words.get(&i) else {
            continue;
        };
        let placed = place(font_cx, cache, row, above, top, width, pics);
        let band = selection
            .zip(keys.get(i))
            .and_then(|(s, &key)| s.range_in(&keys, key, Len::<Byte>::of(row.body.len())));
        if row.call.is_none() {
            draw_body_grounds(
                Pen {
                    surface,
                    font_cx,
                    cache,
                    palette,
                },
                row,
                placed.body,
                text_width,
                &inks,
                band.map(|band| (band, selection_tint(palette.selection))),
            );
        }
        if placed.unread.is_some() {
            line_shown = OnScreen::Shown;
        }
        let pen = Pen {
            surface,
            font_cx,
            cache,
            palette,
        };
        draw_row(
            pen,
            row,
            above,
            &placed,
            width,
            shown.me,
            pics,
            &mut played,
            &inks,
            days.as_ref(),
        );
        if shown.focus == Some(i) {
            let ring = view_rect(SPACE_2, top, width - SPACE_2, top + tall);
            surface.stroke_rect(
                ring,
                palette.selection,
                ambre_core::Px::<ambre_core::Surface>::new(2.0),
            );
            focus = Some(RowBounds {
                top,
                bottom: top + tall,
            });
        }
    }
    Painted {
        laid_out: Len::<ListRow>::of(words.len()),
        latest_shown: reached.latest,
        oldest_shown: reached.oldest,
        line_shown,
        rest: reached.anchor,
        top: reached.rows.first().map(|&(i, top, _)| (i, top)),
        bottom: reached.rows.last().map(|&(i, _, _)| i),
        focus,
        rows: laid
            .iter()
            .map(|row| PaintedRow {
                index: row.index,
                bounds: RowBounds {
                    top: row.top,
                    bottom: row.top + row.tall,
                },
            })
            .collect(),
        selection,
        link,
        chip,
        file,
        join,
        pictures: played.files,
        next_frame: played.next,
    }
}

/// Paint what lies behind the words of a body. That is the chip of each
/// inline code and each mention, the margin marks of a block, and the
/// selection.
fn draw_body_grounds<S: DrawSurface>(
    pen: Pen<'_, S>,
    row: &RowWords,
    body_y: Px<View>,
    width: Px<View>,
    inks: &Inks,
    band: Option<(Span<Byte>, Color)>,
) {
    let Pen {
        surface,
        font_cx,
        cache,
        palette,
    } = pen;
    let codes: Vec<Span<Byte>> = row
        .marks
        .iter()
        .filter(|mark| look::chip(&mark.style))
        .map(|mark| mark.bytes)
        .collect();
    let blocks = row
        .marks
        .iter()
        .any(|mark| matches!(mark.style, TextStyle::Quote | TextStyle::List));
    let tints: Vec<Span<Byte>> = row
        .marks
        .iter()
        .filter(|mark| mark.style == TextStyle::CodeBlock)
        .map(|mark| mark.bytes)
        .collect();
    if band.is_none() && codes.is_empty() && row.mentions.is_empty() && !blocks && tints.is_empty()
    {
        return;
    }
    let layout = body(font_cx, cache, row, width);
    for bytes in tints {
        fill_bytes(surface, layout, bytes, body_y, inks.sunken);
    }
    // A chip of code: the sunken ground framed by the line (`MURMURE-744`).
    for bytes in codes {
        for b in byte_boxes(layout, bytes, body_y) {
            let chip = Rect::new(b.x0 - 3.0, b.y0 + 1.0, b.x1 + 3.0, b.y1 - 1.0);
            fill_rounded(
                surface,
                chip,
                Px::<Surface>::new(4.0),
                inks.sunken,
                Some(inks.line),
            );
        }
    }
    // A chip of a mention: a faint ground of the mention ink (`MURMURE-743`).
    for &bytes in &row.mentions {
        for b in byte_boxes(layout, bytes, body_y) {
            let chip = Rect::new(b.x0 - 2.0, b.y0 + 1.0, b.x1 + 2.0, b.y1 - 1.0);
            let ground = mix(inks.mention, palette.content_bg, MENTION_GROUND);
            fill_rounded(surface, chip, Px::<Surface>::new(4.0), ground, None);
        }
    }
    let (x, y) = (surface_px(TEXT_LEFT).get(), surface_px(body_y).get());
    for mark in &row.marks {
        for at in look::margin(layout, &row.body, &mark.style, mark.bytes) {
            let at = Rect::new(at.x0 + x, at.y0 + y, at.x1 + x, at.y1 + y);
            surface.fill_rect(at, look::margin_ink(palette));
        }
    }
    if let Some((bytes, tint)) = band {
        fill_bytes(surface, layout, bytes, body_y, tint);
    }
}

/// The boxes `layout` gives the bytes `bytes`, on the surface, its top at
/// `body_y`.
fn byte_boxes(layout: &Layout<ParleyBrush>, bytes: Span<Byte>, body_y: Px<View>) -> Vec<Rect> {
    let from = Cursor::from_byte_index(layout, bytes.start.to_usize(), Affinity::Downstream);
    let to = Cursor::from_byte_index(layout, bytes.end.to_usize(), Affinity::Upstream);
    let (x, y) = (surface_px(TEXT_LEFT).get(), surface_px(body_y).get());
    Selection::new(from, to)
        .geometry(layout)
        .into_iter()
        .map(|(bb, _)| Rect::new(bb.x0 + x, bb.y0 + y, bb.x1 + x, bb.y1 + y))
        .collect()
}

/// What a hit test reads: the rows laid out and their words.
struct Hit<'h, 'p> {
    rows: &'h [LaidRow],
    words: &'h HashMap<Pos<ListRow>, (RowWords, Above)>,
    width: Px<View>,
    pics: Option<Pictured<'p>>,
}

impl Hit<'_, '_> {
    /// The row under the height `y`, its words and where its parts lie.
    fn row_at(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        y: Px<View>,
    ) -> Option<(Pos<ListRow>, &RowWords, Placed)> {
        let &LaidRow { index, top, .. } = self.rows.iter().find(|row| row.holds(y))?;
        let (row, above) = self.words.get(&index)?;
        Some((
            index,
            row,
            place(font_cx, cache, row, above, top, self.width, self.pics),
        ))
    }

    /// The reaction pill drawn under `at`: the index of its row and its
    /// emoji. `None` off every pill.
    fn chip(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        at: ambre_core::Point<Px<View>>,
    ) -> Option<(Pos<ListRow>, Emoji)> {
        let (i, row, placed) = self.row_at(font_cx, cache, at.y)?;
        let pills_y = placed.pills?;
        let (boxes, _) = pill_boxes(font_cx, cache, row, text_column(self.width));
        let (x, y) = (
            surface_px(at.x - TEXT_LEFT).get(),
            surface_px(at.y - pills_y).get(),
        );
        boxes
            .iter()
            .zip(&row.reactions)
            .find(|(b, _)| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1)
            .map(|(_, reaction)| (i, reaction.emoji.clone()))
    }

    /// The `Join` button of a call card drawn under `at`: the index of its
    /// row and its call (`MURMURE-746`). `None` off every button.
    fn join(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        at: ambre_core::Point<Px<View>>,
        me: Option<Id<User>>,
    ) -> Option<(Pos<ListRow>, Id<ambre_core::id::Call>)> {
        let (i, row, placed) = self.row_at(font_cx, cache, at.y)?;
        let card = row.call.as_ref()?;
        let parts = card_parts(font_cx, cache, card, placed.card?, self.width, me);
        let button = parts.join?;
        let (x, y) = (surface_px(at.x).get(), surface_px(at.y).get());
        (button.x0 <= x && x < button.x1 && button.y0 <= y && y < button.y1)
            .then_some((i, card.call))
    }

    /// The preview or the file chip drawn under `at`: the file a click
    /// fetches. `None` off every one.
    fn file(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        at: ambre_core::Point<Px<View>>,
    ) -> Option<Attachment> {
        let (_, row, placed) = self.row_at(font_cx, cache, at.y)?;
        if row.files.is_empty() {
            return None;
        }
        let text = text_column(self.width);
        let (x, y) = (
            surface_px(at.x - TEXT_LEFT).get(),
            surface_px(at.y - placed.previews).get(),
        );
        let hit = |b: &Rect| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1;
        if let Some((file, _)) = preview_boxes(row, self.pics, text)
            .into_iter()
            .find(|(_, b)| hit(b))
        {
            return Some(file.clone());
        }
        let y = surface_px(at.y - placed.files).get();
        let (boxes, _) = file_chips(font_cx, cache, row, text);
        boxes
            .iter()
            .zip(&row.files)
            .find(|(b, _)| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1)
            .map(|(_, file)| file.clone())
    }

    /// The address of the link drawn under `at`, `None` off any link. An
    /// address the write rule would refuse opens nothing (`MURMURE-561`).
    fn link(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        at: ambre_core::Point<Px<View>>,
    ) -> Option<ambre_core::LinkUrl> {
        let (_, row, placed) = self.row_at(font_cx, cache, at.y)?;
        let layout = body(font_cx, cache, row, text_column(self.width));
        let (x, y) = (
            surface_px(at.x - TEXT_LEFT).get(),
            surface_px(at.y - placed.body).get(),
        );
        row.marks.iter().find_map(|mark| {
            let TextStyle::Link(address) = &mark.style else {
                return None;
            };
            let from =
                Cursor::from_byte_index(layout, mark.bytes.start.to_usize(), Affinity::Downstream);
            let to = Cursor::from_byte_index(layout, mark.bytes.end.to_usize(), Affinity::Upstream);
            let hit = Selection::new(from, to)
                .geometry(layout)
                .iter()
                .any(|(bb, _)| bb.x0 <= x && x < bb.x1 && bb.y0 <= y && y < bb.y1);
            (hit && crate::logic::style::linkable(address)).then(|| address.clone())
        })
    }

    /// The end of a selection under `at`: the byte of the body the layout
    /// puts there. A point above every row falls on the first one. A point
    /// below them falls on the last, so a drag past an edge reaches it.
    fn spot(
        &self,
        font_cx: &mut FontContext,
        cache: &mut TextCache,
        keys: &IdxVec<ListRow, RowKey>,
        at: ambre_core::Point<Px<View>>,
    ) -> Option<Spot> {
        let &LaidRow { index: i, top, .. } = self
            .rows
            .iter()
            .find(|row| at.y < row.top + row.tall)
            .or_else(|| self.rows.last())?;
        let (row, above) = self.words.get(&i)?;
        let placed = place(font_cx, cache, row, above, top, self.width, self.pics);
        let layout = body(font_cx, cache, row, text_column(self.width));
        let cursor =
            Cursor::from_point(layout, (at.x - TEXT_LEFT).get(), (at.y - placed.body).get());
        Some(Spot {
            row: *keys.get(i)?,
            byte: Pos::<Byte>::from_usize(cursor.index()),
        })
    }
}

/// The box of each reaction pill of a row, from the top-left of the first,
/// and the height they take. The pills run left to right and wrap past
/// `width` (`MURMURE-741`).
fn pill_boxes(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    width: Px<View>,
) -> (Vec<Rect>, Px<View>) {
    let mut boxes = Vec::with_capacity(row.reactions.len());
    let (mut x, mut y, mut line) = (Px::<View>::ZERO, Px::<View>::ZERO, Px::<View>::ZERO);
    for reaction in &row.reactions {
        let label = chip_label(reaction);
        let layout = shape(
            font_cx,
            cache,
            &label,
            FONT_UI,
            width,
            ambre_canvas::Weight::Bold,
        );
        let (wide, tall) = (
            text_width(layout) + SPACE_4 * 2.0,
            text_height(layout) + SPACE_1 * 2.0,
        );
        if x > Px::<View>::ZERO && x + wide > width {
            (x, y) = (Px::<View>::ZERO, y + line + PILL_GAP);
            line = Px::<View>::ZERO;
        }
        boxes.push(view_rect(x, y, x + wide, y + tall));
        x += wide + PILL_GAP;
        line = line.max(tall);
    }
    (boxes, y + line)
}

/// Paint the reaction pills of a row, their top-left at `at`. A pill that
/// holds `me` takes the accent (`MURMURE-741`).
fn draw_pills<S: DrawSurface>(
    pen: Pen<'_, S>,
    row: &RowWords,
    at: ambre_core::Point<Px<View>>,
    width: Px<View>,
    me: Option<Id<User>>,
    inks: &Inks,
) {
    let Pen {
        surface,
        font_cx,
        cache,
        ..
    } = pen;
    let (boxes, _) = pill_boxes(font_cx, cache, row, width);
    let (dx, dy) = (surface_px(at.x).get(), surface_px(at.y).get());
    for (reaction, b) in row.reactions.iter().zip(boxes) {
        let rect = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        let mine = me.is_some_and(|me| reaction.users.contains(&me));
        let (ground, border, ink, weight) = if mine {
            (
                inks.accent_soft,
                inks.accent,
                inks.accent,
                ambre_canvas::Weight::Bold,
            )
        } else {
            (
                inks.surface,
                inks.line,
                inks.dim,
                ambre_canvas::Weight::Regular,
            )
        };
        let corner = Px::<Surface>::new(rect.height() / 2.0);
        fill_rounded(surface, rect, corner, ground, Some(border));
        let label = chip_label(reaction);
        // Both weights shape the box the bold one takes, so a pill keeps
        // its width when it becomes the member's own.
        let layout = shape(font_cx, cache, &label, FONT_UI, width, weight);
        let left = painter_px(Px::<Surface>::new(rect.x0)) + SPACE_4;
        let top = painter_px(Px::<Surface>::new(rect.y0)) + SPACE_1;
        draw(
            surface,
            layout,
            &label,
            ambre_core::Point::new(left, top),
            ink,
        );
    }
}

/// The box of each file chip of a row, as the pills give the reactions.
fn file_chips(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    width: Px<View>,
) -> (Vec<Rect>, Px<View>) {
    let labels: Vec<String> = row.files.iter().map(file_label).collect();
    chip_boxes(font_cx, cache, &labels, width)
}

/// The box of a chip for each of `labels`, from the top-left of the first,
/// and the height they take with the space above them.
fn chip_boxes(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    labels: &[String],
    width: Px<View>,
) -> (Vec<Rect>, Px<View>) {
    if labels.is_empty() {
        return (Vec::new(), Px::<View>::ZERO);
    }
    let mut boxes = Vec::with_capacity(labels.len());
    let (mut x, mut y, mut line) = (Px::<View>::ZERO, SPACE_2, Px::<View>::ZERO);
    for label in labels {
        let layout = shape(
            font_cx,
            cache,
            label,
            FONT_UI,
            width,
            ambre_canvas::Weight::Regular,
        );
        let (wide, tall) = (
            text_width(layout) + SPACE_4 * 2.0,
            text_height(layout) + SPACE_2 * 2.0,
        );
        if x > Px::<View>::ZERO && x + wide > width {
            (x, y) = (Px::<View>::ZERO, y + line + SPACE_2);
            line = Px::<View>::ZERO;
        }
        boxes.push(view_rect(x, y, x + wide, y + tall));
        x += wide + SPACE_4;
        line = line.max(tall);
    }
    (boxes, y + line)
}

/// Paint the file chips of a row, their top-left at `at`, in the ink of
/// a link: a click fetches the file.
fn draw_files<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    row: &RowWords,
    at: Point,
    width: Px<View>,
    palette: &Palette,
) {
    let (boxes, _) = file_chips(font_cx, cache, row, width);
    let (dx, dy) = (at.x, at.y);
    for (file, b) in row.files.iter().zip(boxes) {
        let at = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        surface.fill_rect(at, palette.header_bg);
        surface.stroke_rect(
            at,
            palette.header_border,
            ambre_core::Px::<ambre_core::Surface>::new(1.0),
        );
        let label = file_label(file);
        let layout = shape(
            font_cx,
            cache,
            &label,
            FONT_UI,
            width,
            ambre_canvas::Weight::Regular,
        );
        let (left, top) = (
            painter_px(Px::<Surface>::new(at.x0)) + SPACE_4,
            painter_px(Px::<Surface>::new(at.y0)) + SPACE_2,
        );
        draw(
            surface,
            layout,
            &label,
            ambre_core::Point::new(left, top),
            palette.selection,
        );
    }
}

/// The words of a thread summary after its count: who replied last, and
/// how long ago when the paint knows the clock (`MURMURE-742`).
fn last_reply_words(words: &ThreadWords, days: Option<&Days<'_>>) -> Option<String> {
    let (name, at) = words.last.as_ref()?;
    let name = name.split_whitespace().next().unwrap_or(name);
    Some(match days {
        Some(days) => ambre_commands::trf!(
            "Last reply from {} {}",
            "Derni\u{e8}re r\u{e9}ponse de {} {}",
            name,
            ago(days.calendar.now.since(*at))
        ),
        None => ambre_commands::trf!(
            "Last reply from {}",
            "Derni\u{e8}re r\u{e9}ponse de {}",
            name
        ),
    })
}

/// How long ago a span reads, coarse: a minute, an hour or a day at most.
fn ago(span: ambre_core::TimeSpan<ambre_core::Ms>) -> String {
    let minutes = span.get() / 60_000;
    match minutes {
        0 => ambre_commands::tr!("just now", "\u{e0} l'instant").to_owned(),
        1..60 => ambre_commands::trf!("{} min ago", "il y a {}\u{a0}min", minutes),
        60..1440 => ambre_commands::trf!("{} h ago", "il y a {}\u{a0}h", minutes / 60),
        _ => ambre_commands::trf!("{} d ago", "il y a {}\u{a0}j", minutes / 1440),
    }
}

/// Paint the thread summary of a root, its top at `top` (`MURMURE-742`). It
/// shows the faces of the latest repliers but `me`, then the count in the
/// accent, then the last reply.
#[expect(
    clippy::too_many_arguments,
    reason = "the pen, the row and the inks of one summary"
)]
fn draw_thread<S: DrawSurface>(
    pen: Pen<'_, S>,
    row: &RowWords,
    words: &ThreadWords,
    top: Px<View>,
    width: Px<View>,
    me: Option<Id<User>>,
    inks: &Inks,
    days: Option<&Days<'_>>,
) {
    let Pen {
        surface,
        font_cx,
        cache,
        ..
    } = pen;
    let mut x = TEXT_LEFT + SPACE_2;
    let face_top = top + THREAD_PAD;
    let faces = words.faces.iter().filter(|f| Some(f.who) != me).take(3);
    for face in faces {
        let rect = view_rect(x, face_top, x + THREAD_FACE, face_top + THREAD_FACE);
        draw_face(
            surface,
            font_cx,
            cache,
            face,
            rect,
            (THREAD_FACE_CORNER, FACE_FONT),
        );
        x += THREAD_FACE + SPACE_4;
    }
    let middle = face_top + THREAD_FACE / 2.0;
    if let Some(count) = &row.replies {
        let layout = shape(
            font_cx,
            cache,
            count,
            FONT_UI,
            width,
            ambre_canvas::Weight::Bold,
        );
        let (wide, tall) = (text_width(layout), text_height(layout));
        draw(
            surface,
            layout,
            count,
            ambre_core::Point::new(x, middle - tall / 2.0),
            inks.accent,
        );
        x += wide + SPACE_4;
    }
    if let Some(last) = last_reply_words(words, days) {
        let room = (TEXT_LEFT + width - x).max(Px::<View>::new(1.0));
        let layout = shape(
            font_cx,
            cache,
            &last,
            FONT_UI,
            room,
            ambre_canvas::Weight::Regular,
        );
        let tall = text_height(layout).min(THREAD_FACE);
        draw(
            surface,
            layout,
            &last,
            ambre_core::Point::new(x, middle - tall / 2.0),
            inks.faint,
        );
    }
}

/// The height of a call card: its content between its paddings.
fn card_tall(font_cx: &mut FontContext, cache: &mut TextCache, card: &CallCard) -> Px<View> {
    let (title, started) = card_texts(font_cx, cache, card);
    CARD_PAD_Y * 2.0 + CARD_ICON.max(title + started)
}

/// The heights of the two lines of a call card.
fn card_texts(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    card: &CallCard,
) -> (Px<View>, Px<View>) {
    let wide = Px::<View>::new(1000.0);
    let title = text_height(shape(
        font_cx,
        cache,
        card.title(),
        FONT_BODY,
        wide,
        ambre_canvas::Weight::Bold,
    ));
    let started = text_height(shape(
        font_cx,
        cache,
        &card.started,
        FONT_SMALL,
        wide,
        ambre_canvas::Weight::Regular,
    ));
    (title, started)
}

/// Where the parts of a call card lie, on the surface.
#[derive(Debug, Clone, Copy)]
struct CardParts {
    frame: Rect,
    icon: Rect,
    /// The left edge of the faces, when the card is wide enough for them.
    faces: Option<Px<View>>,
    /// The `Join` button, while the member is out of a live call.
    join: Option<Rect>,
}

/// Lay a call card out, its top at `top`, in a list `width` wide.
fn card_parts(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    card: &CallCard,
    top: Px<View>,
    width: Px<View>,
    me: Option<Id<User>>,
) -> CardParts {
    let wide = text_column(width).min(CARD_MAX);
    let tall = card_tall(font_cx, cache, card);
    let (left, right) = (TEXT_LEFT, TEXT_LEFT + wide);
    let frame = view_rect(left, top, right, top + tall);
    let icon_top = top + (tall - CARD_ICON) / 2.0;
    let icon = view_rect(
        left + CARD_PAD_X,
        icon_top,
        left + CARD_PAD_X + CARD_ICON,
        icon_top + CARD_ICON,
    );
    let mut edge = right - CARD_PAD_X;
    let join = card.joinable(me).then(|| {
        let label = ambre_commands::shown(&CALL_JOIN);
        let layout = shape(
            font_cx,
            cache,
            label,
            FONT_BODY,
            wide,
            ambre_canvas::Weight::Bold,
        );
        let button = text_width(layout) + SPACE_6 * 2.0;
        let y = top + (tall - JOIN_TALL) / 2.0;
        let rect = view_rect(edge - button, y, edge, y + JOIN_TALL);
        // The mockup's flex gap and the button's own margin.
        edge = edge - button - SPACE_6 - SPACE_3;
        rect
    });
    let faces = (!card.faces.is_empty() && wide >= CARD_FACES_FROM).then(|| {
        let count = card.faces.len() as f32;
        edge - CARD_FACE * count + CARD_FACE_OVERLAP * (count - 1.0)
    });
    CardParts {
        frame,
        icon,
        faces,
        join,
    }
}

/// Paint a call card, its top at `top` (`MURMURE-745`): the phone, the
/// title and who started it, the faces in the call and `Join`.
fn draw_card<S: DrawSurface>(
    pen: Pen<'_, S>,
    card: &CallCard,
    top: Px<View>,
    width: Px<View>,
    me: Option<Id<User>>,
    inks: &Inks,
    ground: Color,
) {
    let Pen {
        surface,
        font_cx,
        cache,
        ..
    } = pen;
    let parts = card_parts(font_cx, cache, card, top, width, me);
    let live = card.state == CallState::Live;
    let (fill, border, badge) = if live {
        (
            mix(inks.ok, ground, CARD_GROUND),
            mix(inks.ok, inks.line, CARD_BORDER),
            inks.ok,
        )
    } else {
        (inks.sunken, inks.line, inks.faint)
    };
    fill_rounded(surface, parts.frame, CARD_CORNER, fill, Some(border));
    fill_rounded(surface, parts.icon, Px::<Surface>::new(10.0), badge, None);
    if let Ok(mut phone) = BezPath::from_svg(PHONE_PATH) {
        // The 24 px box of the icon set, drawn 16 px wide at the icon's centre.
        let (cx, cy) = (parts.icon.center().x, parts.icon.center().y);
        phone.apply_affine(Affine::translate((cx - 8.0, cy - 8.0)) * Affine::scale(16.0 / 24.0));
        // The icon set strokes 1.9 units of its 24 px box.
        let stroke = ambre_draw::Stroke::new(1.9 * 16.0 / 24.0);
        let white = Color::from_rgba8(255, 255, 255, 255);
        surface.stroke_path(&phone, &Brush::Solid(white), &stroke);
    }
    let (title_tall, started_tall) = card_texts(font_cx, cache, card);
    let text_left = painter_px(Px::<Surface>::new(parts.icon.x1)) + SPACE_6;
    let text_top = painter_px(Px::<Surface>::new(parts.frame.y0))
        + (painter_px(Px::<Surface>::new(parts.frame.y1))
            - painter_px(Px::<Surface>::new(parts.frame.y0))
            - title_tall
            - started_tall)
            / 2.0;
    let wide = Px::<View>::new(1000.0);
    let layout = shape(
        font_cx,
        cache,
        card.title(),
        FONT_BODY,
        wide,
        ambre_canvas::Weight::Bold,
    );
    draw(
        surface,
        layout,
        card.title(),
        ambre_core::Point::new(text_left, text_top),
        inks.text,
    );
    let layout = shape(
        font_cx,
        cache,
        &card.started,
        FONT_SMALL,
        wide,
        ambre_canvas::Weight::Regular,
    );
    let at = ambre_core::Point::new(text_left, text_top + title_tall);
    draw(surface, layout, &card.started, at, inks.dim);
    if let Some(mut x) = parts.faces {
        let face_top = painter_px(Px::<Surface>::new(parts.icon.center().y)) - CARD_FACE / 2.0;
        for face in &card.faces {
            let rect = view_rect(x, face_top, x + CARD_FACE, face_top + CARD_FACE);
            let ring = Rect::new(rect.x0 - 2.0, rect.y0 - 2.0, rect.x1 + 2.0, rect.y1 + 2.0);
            fill_rounded(surface, ring, Px::<Surface>::new(99.0), inks.surface, None);
            draw_face(
                surface,
                font_cx,
                cache,
                face,
                rect,
                (Px::<Surface>::new(99.0), CARD_FACE_FONT),
            );
            x += CARD_FACE - CARD_FACE_OVERLAP;
        }
    }
    if let Some(button) = parts.join {
        fill_rounded(surface, button, Px::<Surface>::new(8.0), inks.ok, None);
        let label = ambre_commands::shown(&CALL_JOIN);
        let layout = shape(
            font_cx,
            cache,
            label,
            FONT_BODY,
            wide,
            ambre_canvas::Weight::Bold,
        );
        let (lw, lt) = (text_width(layout), text_height(layout));
        let left = painter_px(Px::<Surface>::new(button.x0))
            + (painter_px(Px::<Surface>::new(button.x1))
                - painter_px(Px::<Surface>::new(button.x0))
                - lw)
                / 2.0;
        let top = painter_px(Px::<Surface>::new(button.y0)) + (JOIN_TALL - lt) / 2.0;
        let white = Color::from_rgba8(255, 255, 255, 255);
        draw(
            surface,
            layout,
            label,
            ambre_core::Point::new(left, top),
            white,
        );
    }
}

/// Paint the day divider whose label tops at `y`: the label in the middle,
/// a rule on each side of it (`MURMURE-739`).
fn draw_day<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    label: &str,
    y: Px<View>,
    width: Px<View>,
    inks: &Inks,
) {
    let layout = shape(
        font_cx,
        cache,
        label,
        FONT_SMALL,
        width,
        ambre_canvas::Weight::Bold,
    );
    let (wide, tall) = (text_width(layout), text_height(layout));
    let left = ((width - wide) / 2.0).max(EDGE);
    let middle = surface_px(y + tall / 2.0).get().round();
    let rules = [
        (EDGE, left - SPACE_5),
        (left + wide + SPACE_5, width - EDGE),
    ];
    for (from, to) in rules {
        if to > from {
            let rule = Rect::new(
                surface_px(from).get(),
                middle,
                surface_px(to).get(),
                middle + 1.0,
            );
            surface.fill_rect(rule, inks.line);
        }
    }
    draw(
        surface,
        layout,
        label,
        ambre_core::Point::new(left, y),
        inks.faint,
    );
}

/// Paint the unread separator whose label tops at `y`: a rule across the
/// list in the mention ink, `New` at its right end (`MURMURE-740`).
fn draw_new<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    y: Px<View>,
    width: Px<View>,
    inks: &Inks,
) {
    let label = ambre_commands::shown(&UNREAD_LINE);
    let layout = shape(
        font_cx,
        cache,
        label,
        FONT_XS,
        width,
        ambre_canvas::Weight::Bold,
    );
    let (wide, tall) = (text_width(layout), text_height(layout));
    let left = width - EDGE - wide;
    let middle = surface_px(y + tall / 2.0).get().round();
    let rule = Rect::new(
        surface_px(EDGE).get(),
        middle,
        surface_px(left - SPACE_4).get(),
        middle + 1.0,
    );
    surface.fill_rect(rule, inks.mention);
    draw(
        surface,
        layout,
        label,
        ambre_core::Point::new(left, y),
        inks.mention,
    );
}

/// The layout of one text of a row, shaped once and kept in the cache.
pub(crate) fn shape<'c>(
    font_cx: &mut FontContext,
    cache: &'c mut TextCache,
    text: &str,
    size: Px<View>,
    width: Px<View>,
    weight: ambre_canvas::Weight,
) -> &'c Layout<ParleyBrush> {
    cache.get_or_build(
        font_cx,
        text,
        ambre_canvas::Look {
            weight,
            ..ambre_canvas::Look::body(size, width)
        },
    )
}

/// The layout of a row's body, each of its marks shaped in (`MURMURE-565`).
/// A mention takes the heavy face (`MURMURE-743`). A body with no mark takes
/// the same entry as its plain text.
fn body<'c>(
    font_cx: &mut FontContext,
    cache: &'c mut TextCache,
    row: &RowWords,
    width: Px<View>,
) -> &'c Layout<ParleyBrush> {
    let spans: Vec<SpanShape> = row
        .marks
        .iter()
        .map(|mark| look::shape(&mark.style, mark.bytes))
        .chain(row.mentions.iter().map(|&range| SpanShape {
            range,
            weight: Some(ambre_core::Bounded::<ambre_core::FontWeight>::BOLD),
            ..SpanShape::default()
        }))
        .collect();
    cache.get_or_build_spans(font_cx, &row.body, FONT_BODY, width, &spans)
}

/// Fill the boxes `layout` gives the bytes `bytes`, its top at `body_y`.
fn fill_bytes<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    bytes: Span<Byte>,
    body_y: Px<View>,
    color: Color,
) {
    for rect in byte_boxes(layout, bytes, body_y) {
        surface.fill_rect(rect, color);
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
    width: Px<View>,
) -> Vec<(&'r Attachment, Rect)> {
    let mut y = Px::<View>::ZERO;
    let mut boxes = Vec::new();
    for file in previewed(row, pics) {
        let aspect = pics
            .and_then(|pics| pics.shelf.get(&file.media_ref))
            .filter(|picture| picture.size.width > Px::ZERO && picture.size.height > Px::ZERO)
            .map_or(PLACEHOLDER_ASPECT, |picture| {
                picture.size.width.get() as f32 / picture.size.height.get() as f32
            });
        y += SPACE_2;
        let mut wide = PREVIEW_TALL * aspect;
        let mut tall = PREVIEW_TALL;
        if wide > width {
            (wide, tall) = (width, width / aspect);
        }
        boxes.push((file, view_rect(Px::<View>::ZERO, y, wide, y + tall)));
        y += PREVIEW_TALL;
    }
    boxes
}

/// The height the previews of a row take, the space above each included.
/// Each takes [`PREVIEW_TALL`], so a row keeps its height when one lands.
fn previews_height(row: &RowWords, pics: Option<Pictured<'_>>) -> Px<View> {
    let count = previewed(row, pics).len() as f32;
    (SPACE_2 + PREVIEW_TALL) * count
}

/// Paint the previews of a row, their top-left at `at`. A decoded
/// picture draws the frame its clock reaches. One not decoded yet draws
/// its box empty. `played` records each file and the next frame due.
fn draw_previews<S: DrawSurface>(
    surface: &mut S,
    row: &RowWords,
    pics: Option<Pictured<'_>>,
    at: Point,
    width: Px<View>,
    palette: &Palette,
    played: &mut Played,
) {
    let Some(pictured) = pics else {
        return;
    };
    let (dx, dy) = (at.x, at.y);
    for (file, b) in preview_boxes(row, pics, width) {
        let at = Rect::new(b.x0 + dx, b.y0 + dy, b.x1 + dx, b.y1 + dy);
        played.files.push(file.media_ref.clone());
        let Some(picture) = pictured.shelf.get(&file.media_ref) else {
            surface.fill_rect(at, palette.header_bg);
            surface.stroke_rect(
                at,
                palette.header_border,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            continue;
        };
        let (index, next) = frame_at(picture, pictured.now, pictured.motion);
        let Some(frame) = crate::ui::pictures::frame(picture, index) else {
            continue;
        };
        let image = ImageSource {
            key: frame_key(&file.media_ref, index),
            pixels: &frame.pixels,
            size: picture.size,
        };
        surface.draw_image(&image, at);
        played.next = match (played.next, next) {
            (Some(a), Some(b)) => Some(if b < a { b } else { a }),
            (a, b) => a.or(b),
        };
    }
}

/// Paint one row where `placed` lays it.
#[expect(
    clippy::too_many_arguments,
    reason = "the pen, the row, its place and what decorates it"
)]
fn draw_row<S: DrawSurface>(
    pen: Pen<'_, S>,
    row: &RowWords,
    above: &Above,
    placed: &Placed,
    width: Px<View>,
    me: Option<Id<User>>,
    pics: Option<Pictured<'_>>,
    played: &mut Played,
    inks: &Inks,
    days: Option<&Days<'_>>,
) {
    let Pen {
        surface,
        font_cx,
        cache,
        palette,
    } = pen;
    let text = text_column(width);
    if let (Some(label), Some(y)) = (&above.day, placed.day) {
        draw_day(surface, font_cx, cache, label, y, width, inks);
    }
    if let Some(y) = placed.unread {
        draw_new(surface, font_cx, cache, y, width, inks);
    }
    if let (Some(words), Some(y)) = (&row.header, placed.header) {
        let avatar = view_rect(EDGE, y, EDGE + AVATAR, y + AVATAR);
        let face = Face {
            who: words.who,
            initials: words.initials.clone(),
        };
        draw_face(
            surface,
            font_cx,
            cache,
            &face,
            avatar,
            (AVATAR_CORNER, AVATAR_FONT),
        );
        let layout = shape(
            font_cx,
            cache,
            &words.author,
            FONT_BODY,
            text,
            ambre_canvas::Weight::Bold,
        );
        let (wide, author_line) = (text_width(layout), first_baseline(layout));
        draw(
            surface,
            layout,
            &words.author,
            ambre_core::Point::new(TEXT_LEFT, y),
            palette.cell_text,
        );
        let layout = shape(
            font_cx,
            cache,
            &words.time,
            FONT_SMALL,
            text,
            ambre_canvas::Weight::Regular,
        );
        // The time shares the baseline of the name, as inline text does.
        let time_top = y + author_line - first_baseline(layout);
        let at = ambre_core::Point::new(TEXT_LEFT + wide + SPACE_3, time_top);
        draw(surface, layout, &words.time, at, inks.faint);
    }
    if let Some(card) = &row.call {
        if let Some(y) = placed.card {
            let pen = Pen {
                surface,
                font_cx,
                cache,
                palette,
            };
            draw_card(pen, card, y, width, me, inks, palette.content_bg);
        }
    } else {
        let ink = match row.ink {
            BodyInk::Muted => palette.header_text,
            BodyInk::Plain => palette.cell_text,
        };
        let layout = body(font_cx, cache, row, text);
        let at = ambre_core::Point::new(TEXT_LEFT, placed.body);
        draw_body(
            surface,
            layout,
            &row.body,
            at,
            ink,
            palette,
            &row.mentions,
            inks.mention,
        );
    }
    let at = Point::new(
        surface_px(TEXT_LEFT).get(),
        surface_px(placed.previews).get(),
    );
    draw_previews(surface, row, pics, at, text, palette, played);
    let at = Point::new(surface_px(TEXT_LEFT).get(), surface_px(placed.files).get());
    draw_files(surface, font_cx, cache, row, at, text, palette);
    if let Some(y) = placed.pills {
        let pen = Pen {
            surface,
            font_cx,
            cache,
            palette,
        };
        draw_pills(
            pen,
            row,
            ambre_core::Point::new(TEXT_LEFT, y),
            text,
            me,
            inks,
        );
    }
    if let (Some(words), Some(y)) = (&row.thread, placed.thread) {
        let pen = Pen {
            surface,
            font_cx,
            cache,
            palette,
        };
        draw_thread(pen, row, words, y, text, me, inks, days);
    }
    if let (Some(line), Some(y)) = (&row.replies, placed.replies) {
        let layout = shape(
            font_cx,
            cache,
            line,
            FONT_UI,
            text,
            ambre_canvas::Weight::Regular,
        );
        draw(
            surface,
            layout,
            line,
            ambre_core::Point::new(TEXT_LEFT, y),
            inks.accent,
        );
    }
}

/// Paint the control that brings the list back to its latest message, at
/// the foot of a viewport of `size` (`MURMURE-505`). Returns the rectangle
/// a press on it hits.
pub fn paint_jump<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    size: Size<Px<View>>,
    palette: &Palette,
) -> Rect {
    let (width, height) = (size.width, size.height);
    let layout = shape(
        font_cx,
        cache,
        JUMP_TO_LATEST,
        FONT_UI,
        width.max(Px::<View>::new(1.0)),
        ambre_canvas::Weight::Bold,
    );
    let (wide, tall) = (
        SPACE_8 * 2.0 + text_width(layout),
        SPACE_4 * 2.0 + text_height(layout),
    );
    let x = ((width - wide) / 2.0).max(Px::<View>::ZERO);
    let y = (height - tall - SPACE_8).max(Px::<View>::ZERO);
    let rect = view_rect(x, y, x + wide, y + tall);
    surface.fill_rect(rect, palette.header_bg);
    surface.stroke_rect(
        rect,
        palette.header_border,
        ambre_core::Px::<ambre_core::Surface>::new(1.0),
    );
    draw(
        surface,
        layout,
        JUMP_TO_LATEST,
        ambre_core::Point::new(x + SPACE_8, y + SPACE_4),
        palette.selection,
    );
    rect
}

/// Emit every glyph run of `layout`, its top-left at `(x, y)`. `text` is
/// the string `layout` was built from (`PDF-005`).
pub(crate) fn draw<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    text: &str,
    at: ambre_core::Point<Px<View>>,
    color: Color,
) {
    let (x, y) = (at.x.get(), at.y.get());
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let at = (f64::from(x + run.offset()), f64::from(y + run.baseline()));
            surface.draw_glyphs(
                &run,
                ambre_draw::LayoutText::whole(text),
                &brush,
                Affine::translate(at),
            );
        }
    }
}

/// Emit every glyph run of a body, its top-left at `(x, y)`. An underlined
/// run is a link: it takes the link ink and a rule under its baseline. A
/// run inside a mention takes the mention ink (`MURMURE-743`).
#[expect(
    clippy::too_many_arguments,
    reason = "the layout, its place and its three inks"
)]
fn draw_body<S: DrawSurface>(
    surface: &mut S,
    layout: &Layout<ParleyBrush>,
    text: &str,
    at: ambre_core::Point<Px<View>>,
    ink: Color,
    palette: &Palette,
    mentions: &[Span<Byte>],
    mention: Color,
) {
    let (x, y) = (at.x.get(), at.y.get());
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let linked = run.style().underline.is_some();
            let range = run.run().text_range();
            let named = mentions
                .iter()
                .any(|m| m.start.to_usize() <= range.start && range.end <= m.end.to_usize());
            let color = if linked {
                look::link_ink(palette)
            } else if named {
                mention
            } else {
                ink
            };
            let (left, baseline) = (x + run.offset(), y + run.baseline());
            let at = (f64::from(left), f64::from(baseline));
            surface.draw_glyphs(
                &run,
                ambre_draw::LayoutText::whole(text),
                &Brush::Solid(color),
                Affine::translate(at),
            );
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
    use crate::message::{Mention, MentionTarget, StyleMark, TextStyle, ThreadHead};
    use ambre_bus::Char;
    use ambre_canvas::fonts::font_context_from_bytes;
    use ambre_canvas::theme::LIGHT;
    use ambre_core::id::{Document, Segment};
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    use super::*;
    use crate::ui::list::tests::message;
    use ambre_bus::{Authored, Instant, PageLoad};
    use ambre_core::{Ms, TimeSpan};

    /// The point of the list at `(x, y)`.
    fn view_at((x, y): (f32, f32)) -> ambre_core::Point<Px<View>> {
        ambre_core::Point::new(Px::<View>::new(x), Px::<View>::new(y))
    }

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
        .map(|file| std::fs::read(format!("{DIR}{file}")).expect("a shipped face"))
        .map(ambre_core::Payload::<ambre_core::FontBytes>::new);
        font_context_from_bytes(faces)
    }

    fn paint(list: &MessageList, height: f32) -> (RecordingSurface, Painted) {
        paint_above(list, None, height)
    }

    fn paint_above(
        list: &MessageList,
        unread: Option<Id<Message>>,
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
                theme: Theme::Light,
                calendar: None,
            },
            height,
        )
    }

    fn paint_from(shown: Shown<'_>, height: f32) -> (RecordingSurface, Painted) {
        let (mut surface, mut font_cx) = (RecordingSurface::new(), fonts());
        let mut cache = TextCache::new();
        let painted = paint_list(
            Pen {
                surface: &mut surface,
                font_cx: &mut font_cx,
                cache: &mut cache,
                palette: &LIGHT,
            },
            shown,
            Size::new(Px::<View>::new(400.0), Px::<View>::new(height)),
            &|_| ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").ok(),
            &|_| "t".to_owned(),
        );
        (surface, painted)
    }

    /// Each glyph run of `color`, as its top and its glyph count, top first.
    /// The chrome inks of the light theme, which the decorations of a row
    /// take.
    fn inks() -> Inks {
        Inks::of(Theme::Light)
    }

    fn runs(surface: &RecordingSurface, color: Color) -> Vec<(f64, usize)> {
        let mut runs: Vec<(f64, usize)> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::Glyphs {
                    glyph_count,
                    color: Some(c),
                    transform,
                } if *c == color => Some((transform.translation().y, glyph_count.get() as usize)),
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
        let mut cache = TextCache::new();
        let rect = paint_jump(
            &mut surface,
            &mut font_cx,
            &mut cache,
            Size::new(Px::<View>::new(400.0), Px::<View>::new(300.0)),
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
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let messages = (0..100)
            .map(|i| message(ada, Instant::<Authored>::new(i * 600_000), "hello"))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        let focused = |focus| Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: Some(focus),
            selecting: Selecting::default(),
            me: None,
            pictures: None,
            theme: Theme::Light,
            calendar: None,
        };
        let (surface, painted) = paint_from(focused(Pos::<ListRow>::new(98)), 300.0);
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
        let RowBounds { top, bottom } = painted.focus.expect("the focused row is on screen");
        assert_eq!(
            (ring.y0, ring.y1),
            (surface_px(top).get(), surface_px(bottom).get())
        );
        assert!(top >= Px::<View>::ZERO && bottom <= Px::<View>::new(300.0) && bottom > top);
        assert_eq!(
            painted.bottom,
            Some(Pos::<ListRow>::new(99)),
            "the latest row is the last one"
        );
        let body = runs(&surface, LIGHT.cell_text);
        assert!(body.iter().any(|&(y, _)| y > ring.y0 && y < ring.y1));
        let (surface, painted) = paint_from(focused(Pos::<ListRow>::new(3)), 300.0);
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
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let messages = (0..10_000)
            .map(|i| message(ada, Instant::<Authored>::new(i), "hello"))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        let (surface, painted) = paint(&list, 200.0);
        assert_eq!(painted.latest_shown, OnScreen::Shown);
        assert!(
            painted.laid_out.get() > 1 && painted.laid_out.get() < 20,
            "{painted:?}"
        );
        assert_eq!(surface.fill_colors().first(), Some(&LIGHT.content_bg));
    }

    /// A list resting on a row paints from that row and says so, and one
    /// at the top shows its oldest row (`MURMURE-190`, `MURMURE-191`).
    #[test]
    fn a_list_resting_on_a_row_paints_from_it() {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let messages = (0..100)
            .map(|i| message(ada, Instant::<Authored>::new(i), "hello"))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        let at = |anchor| Shown {
            list: &list,
            unread: None,
            anchor: Some(anchor),
            focus: None,
            selecting: Selecting::default(),
            me: None,
            pictures: None,
            theme: Theme::Light,
            calendar: None,
        };
        let (_, painted) = paint_from(at((Pos::<ListRow>::new(10), Px::<View>::ZERO)), 200.0);
        assert_eq!(
            painted.rest,
            Some((Pos::<ListRow>::new(10), Px::<View>::ZERO))
        );
        let shown = |painted: &Painted| (painted.oldest_shown, painted.latest_shown);
        assert_eq!(shown(&painted), (OnScreen::Hidden, OnScreen::Hidden));
        assert!(painted.laid_out.get() < 20, "{painted:?}");
        let (_, top) = paint_from(at((Pos::<ListRow>::ZERO, Px::<View>::ZERO)), 200.0);
        assert_eq!(shown(&top), (OnScreen::Shown, OnScreen::Hidden));
        let (_, bottom) = paint(&list, 200.0);
        assert_eq!(bottom.rest, None);
        assert_eq!(shown(&bottom), (OnScreen::Hidden, OnScreen::Shown));
    }

    /// The rows paint top to bottom in the order of the list, under one
    /// header for the group (`MURMURE-182`, `MURMURE-183`).
    #[test]
    fn the_rows_paint_top_to_bottom_under_one_header() {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let bodies = ["a", "abcd", "abcdefgh"];
        let messages = bodies
            .iter()
            .map(|b| message(ada, Instant::<Authored>::new(0), b))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        let (surface, painted) = paint(&list, 600.0);
        assert_eq!(painted.laid_out.get(), 3);
        let counts: Vec<usize> = runs(&surface, LIGHT.cell_text)
            .iter()
            .map(|r| r.1)
            .collect();
        assert_eq!(counts, [3, 1, 4, 8]);
        assert_eq!(runs(&surface, inks().faint).len(), 1, "one time");
    }

    /// A deleted body paints muted, and a reply count in the accent
    /// (`MURMURE-185`).
    #[test]
    fn a_deleted_body_paints_muted_and_a_reply_count_in_the_accent() {
        let mut gone = message(Id::<User>::mint(), Instant::<Authored>::new(0), "");
        gone.deleted = crate::message::Deletion::Deleted;
        gone.thread = Some(ThreadHead {
            document: Id::<Document>::mint(),
            replies: ambre_core::Count::<crate::message::Replies>::new(2),
            last_replier: None,
            repliers: Vec::new(),
            last_reply: None,
        });
        let mut list = MessageList::default();
        list.absorb(Id::<Segment>::mint(), vec![gone]);
        let (surface, _) = paint(&list, 600.0);
        let author: usize = runs(&surface, LIGHT.cell_text).iter().map(|r| r.1).sum();
        assert_eq!(author, 3);
        let muted: usize = runs(&surface, LIGHT.header_text).iter().map(|r| r.1).sum();
        assert_eq!(
            muted,
            ambre_commands::shown(&crate::ui::list::DELETED)
                .chars()
                .count()
        );
        assert!(!runs(&surface, inks().accent).is_empty());
    }

    /// The reactions of a message paint as one pill each under its body,
    /// the most chosen first, the member's own in the accent. The row grows
    /// to hold them. A click on a pill names it; a deleted message shows
    /// none (`MURMURE-741`).
    #[test]
    fn the_reactions_paint_as_chips_under_the_body() {
        use crate::message::Reaction;
        let (me, ada) = (Id::<User>::mint(), Id::<User>::mint());
        let mut reacted = message(ada, Instant::<Authored>::new(0), "hello");
        reacted.reactions = vec![
            Reaction {
                emoji: ambre_bus::Emoji::from("\u{1F44D}"),
                users: vec![ada],
            },
            Reaction {
                emoji: ambre_bus::Emoji::from("\u{1F389}"),
                users: vec![ada, me],
            },
        ];
        let mut list = MessageList::default();
        list.absorb(Id::<Segment>::mint(), vec![reacted.clone()]);
        let mut bare = MessageList::default();
        bare.absorb(
            Id::<Segment>::mint(),
            vec![message(ada, Instant::<Authored>::new(0), "hello")],
        );
        let tall = |painted: &Painted| painted.rows[0].bounds.bottom - painted.rows[0].bounds.top;
        let (_, plain) = paint(&bare, 600.0);

        let (_, at) = paint(&list, 600.0);
        let bottom = at.rows[0].bounds.bottom.get();
        let click = (TEXT_LEFT.get() + 2.0, bottom - ROW_PAD.get() - 2.0);
        let selecting = Selecting {
            click: Some(view_at(click)),
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
            theme: Theme::Light,
            calendar: None,
        };
        let (surface, painted) = paint_from(shown, 600.0);
        assert!(tall(&painted) > tall(&plain), "the chips take room");
        assert_eq!(
            painted.chip,
            Some((Pos::<ListRow>::ZERO, Emoji::from("\u{1F389}"))),
            "the pill two members chose comes first"
        );
        let borders = |color: Color| -> Vec<Rect> {
            surface
                .ops()
                .iter()
                .filter_map(|op| match op {
                    DrawOp::StrokePath {
                        bounds,
                        color: Some(c),
                        ..
                    } if *c == color => Some(*bounds),
                    _ => None,
                })
                .collect()
        };
        let theirs = borders(inks().line);
        let mine = borders(inks().accent);
        let ([theirs], [mine]) = (theirs.as_slice(), mine.as_slice()) else {
            panic!("one pill each: {theirs:?} {mine:?}");
        };
        assert!(mine.x1 < theirs.x0, "left to right, the most chosen first");
        assert_eq!(theirs.y0, mine.y0, "on one line");
        assert!(mine.y1 <= f64::from(bottom));
        let label: usize = runs(&surface, inks().accent).iter().map(|r| r.1).sum();
        assert!(label > 0, "the member's own pill writes in the accent");

        reacted.deleted = crate::message::Deletion::Deleted;
        let mut gone = MessageList::default();
        gone.absorb(Id::<Segment>::mint(), vec![reacted]);
        let (surface, _) = paint(&gone, 600.0);
        let chips = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::StrokePath { .. }))
            .count();
        assert_eq!(chips, 0, "a deleted message shows no pill");
    }

    /// The files of a message paint as one chip each under its body, in the
    /// ink of a link. A click on one names its file.
    #[test]
    fn the_files_paint_as_chips_a_click_names() {
        use crate::message::Attachment;
        use ambre_bus::{ContentHash, MediaType, Octets};
        use ambre_core::Count;
        let ada = Id::<User>::mint();
        let file = Attachment {
            media_ref: Validated::<ContentHash>::parse("0".repeat(64)).expect("an address"),
            name: "a.png".to_owned(),
            media_type: MediaType::new("image/png".to_owned()),
            size: Count::<Octets>::new(2048),
        };
        let mut carrying = message(ada, Instant::<Authored>::new(0), "hello");
        carrying.attachments = vec![file.clone()];
        let mut list = MessageList::default();
        list.absorb(Id::<Segment>::mint(), vec![carrying]);
        let mut bare = MessageList::default();
        bare.absorb(
            Id::<Segment>::mint(),
            vec![message(ada, Instant::<Authored>::new(0), "hello")],
        );
        let tall = |painted: &Painted| painted.rows[0].bounds.bottom - painted.rows[0].bounds.top;
        let (_, plain) = paint(&bare, 600.0);
        let (_, at) = paint(&list, 600.0);
        assert!(tall(&at) > tall(&plain), "the chip takes room");

        let bottom = at.rows[0].bounds.bottom.get();
        let selecting = Selecting {
            click: Some(view_at((
                TEXT_LEFT.get() + 2.0,
                bottom - ROW_PAD.get() - 2.0,
            ))),
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
            theme: Theme::Light,
            calendar: None,
        };
        let (surface, painted) = paint_from(shown, 600.0);
        assert_eq!(painted.file, Some(file));
        assert_eq!(painted.chip, None);
        assert!(!runs(&surface, LIGHT.selection).is_empty(), "link ink");
    }

    /// An image previews in a box of its own above its chip. Before it lands
    /// the box is empty, and the row keeps its height when it does. A paint
    /// draws the frame its clock reaches and names the next, so only a
    /// preview painted plays (`MURMURE-131`). Under reduced motion none is
    /// due.
    #[test]
    fn an_image_previews_in_a_box_that_keeps_its_height_and_plays() {
        use crate::message::{Attachment, Picture, PictureFrame};
        use ambre_bus::{MediaType, Octets};
        use ambre_core::Count;
        let ada = Id::<User>::mint();
        let file = Attachment {
            media_ref: Validated::<ContentHash>::parse("a".repeat(64)).expect("an address"),
            name: "a.gif".to_owned(),
            media_type: MediaType::new("image/gif".to_owned()),
            size: Count::<Octets>::new(2048),
        };
        let mut carrying = message(ada, Instant::<Authored>::new(0), "hello");
        carrying.attachments = vec![file.clone()];
        let mut list = MessageList::default();
        list.absorb(Id::<Segment>::mint(), vec![carrying]);
        let mut shelf = Pictures::default();
        let at = |shelf: &Pictures, now, motion| {
            let shown = Shown {
                list: &list,
                unread: None,
                anchor: None,
                focus: None,
                selecting: Selecting::default(),
                me: None,
                pictures: Some(Pictured { shelf, now, motion }),
                theme: Theme::Light,
                calendar: None,
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
        let tall = |painted: &Painted| painted.rows[0].bounds.bottom - painted.rows[0].bounds.top;
        let (_, chip_only) = paint(&list, 600.0);
        let (surface, waiting) = at(
            &shelf,
            Instant::<PageLoad>::new(0.0),
            crate::ui::pictures::Motion::Full,
        );
        assert_eq!(waiting.pictures, vec![file.media_ref.clone()]);
        assert_eq!(images(&surface), 0, "not decoded yet");
        assert!(tall(&waiting) >= tall(&chip_only) + PREVIEW_TALL);

        shelf.begin_paint(crate::Region::Channel);
        assert!(shelf.want(&file.media_ref));
        let frame = |delay| PictureFrame {
            pixels: vec![0; 2 * 4].into(),
            delay,
        };
        let gif = Picture {
            size: ambre_core::Size::new(Px::new(2), Px::new(1)),
            frames: vec![
                frame(TimeSpan::<Ms>::new(100)),
                frame(TimeSpan::<Ms>::new(100)),
            ],
        };
        assert!(shelf.land(file.media_ref.clone(), Some(gif)));
        let (surface, playing) = at(
            &shelf,
            Instant::<PageLoad>::new(30.0),
            crate::ui::pictures::Motion::Full,
        );
        assert_eq!(images(&surface), 1);
        assert_eq!(tall(&playing), tall(&waiting), "the row keeps its height");
        assert_eq!(playing.next_frame, Some(Instant::<PageLoad>::new(100.0)));
        let (surface, still) = at(
            &shelf,
            Instant::<PageLoad>::new(30.0),
            crate::ui::pictures::Motion::Reduced,
        );
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
            let mut cache = TextCache::new();
            let shown = Shown {
                list,
                unread: None,
                anchor: None,
                focus: None,
                selecting: Selecting::default(),
                me: None,
                pictures: None,
                theme: Theme::Light,
                calendar: None,
            };
            let palette = &LIGHT;
            paint_list(
                Pen {
                    surface: &mut surface,
                    font_cx: &mut font_cx,
                    cache: &mut cache,
                    palette,
                },
                shown,
                Size::new(Px::<View>::new(400.0), Px::<View>::new(600.0)),
                &|_| ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").ok(),
                &|_| "t".to_owned(),
            );
            cache.lazy_fonts_requested().get()
        };
        let ada = Id::<User>::mint();
        let mut plain = MessageList::default();
        plain.absorb(
            Id::<Segment>::mint(),
            vec![message(ada, Instant::<Authored>::new(0), "no picture here")],
        );
        assert_eq!(requested(&plain), 0, "nothing fetched at start");
        let mut said = MessageList::default();
        said.absorb(
            Id::<Segment>::mint(),
            vec![message(
                ada,
                Instant::<Authored>::new(0),
                "ship it \u{1F680}",
            )],
        );
        assert_eq!(requested(&said), 1);
        let mut reacted = message(ada, Instant::<Authored>::new(0), "ship it");
        reacted.reactions = vec![crate::message::Reaction {
            emoji: ambre_bus::Emoji::from("\u{1F389}"),
            users: vec![ada],
        }];
        let mut chipped = MessageList::default();
        chipped.absorb(Id::<Segment>::mint(), vec![reacted]);
        assert_eq!(requested(&chipped), 1, "a chip asks for it too");
    }

    /// A sent body draws its marks: bold shapes heavier, code sits on its
    /// tint, a link takes the accent over a rule (`MURMURE-565`). A renamed
    /// mention before them does not shift them.
    #[test]
    fn a_sent_body_draws_each_of_its_marks() {
        let mut sent = message(
            Id::<User>::mint(),
            Instant::<Authored>::new(0),
            "@bo hi bold c d",
        );
        sent.mentions = vec![Mention {
            target: MentionTarget::User(Id::<User>::mint()),
            range: Span::<Char>::from_usize_range(0..3),
        }];
        let mark = |style, range| StyleMark { style, range };
        sent.styles = vec![
            mark(
                TextStyle::Link("https://ambre.dev".into()),
                Span::<Char>::from_usize_range(14..15),
            ),
            mark(TextStyle::Bold, Span::<Char>::from_usize_range(7..11)),
            mark(TextStyle::Code, Span::<Char>::from_usize_range(12..13)),
        ];
        let mut list = MessageList::default();
        list.absorb(Id::<Segment>::mint(), vec![sent]);
        let rows = list.rows();
        let first = rows.iter().next().expect("one row");
        let row = first.words(
            |_| ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").ok(),
            |_| "t".to_owned(),
        );
        assert_eq!(row.body, "@Ada hi bold c d");
        let mut marks: Vec<(usize, usize)> = row
            .marks
            .iter()
            .map(|m| (m.bytes.start.to_usize(), m.bytes.end.to_usize()))
            .collect();
        marks.sort_unstable();
        assert_eq!(marks, [(8, 12), (13, 14), (15, 16)]);
        let (mut font_cx, mut cache) = (fonts(), TextCache::new());
        let plain = shape(
            &mut font_cx,
            &mut cache,
            &row.body,
            FONT_BODY,
            Px::<View>::new(384.0),
            ambre_canvas::Weight::Regular,
        )
        .width();
        assert!(body(&mut font_cx, &mut cache, &row, Px::<View>::new(384.0)).width() > plain);

        let (surface, _) = paint(&list, 600.0);
        let ops = surface.ops();
        let fill = |want: Color| {
            ops.iter().position(
                |op| matches!(op, DrawOp::FillRect { color, rect } if *color == want && rect.height() < 3.0),
            )
        };
        // Inline code sits on a chip framed by the line (`MURMURE-744`).
        let tint = ops
            .iter()
            .position(
                |op| matches!(op, DrawOp::StrokePath { color: Some(c), .. } if *c == inks().line),
            )
            .expect("the code chip");
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
            let mut sent = message(Id::<User>::mint(), Instant::<Authored>::new(0), body);
            sent.styles = styles;
            let mut list = MessageList::default();
            list.absorb(Id::<Segment>::mint(), vec![sent]);
            let (surface, _) = paint(&list, 600.0);
            let indent = surface_px(TEXT_LEFT).get() + f64::from(look::BLOCK_INDENT.get());
            let marks = surface.count(|op| {
                matches!(op, DrawOp::FillRect { color, rect }
                    if *color == LIGHT.header_text
                        && rect.x0 >= surface_px(TEXT_LEFT).get() && rect.x1 <= indent)
            });
            let tints = surface.count(
                |op| matches!(op, DrawOp::FillRect { color, .. } if *color == inks().sunken),
            );
            (marks.get(), tints.get())
        };
        let mark = |style, range| StyleMark { style, range };
        let blocks = vec![
            mark(TextStyle::Quote, Span::<Char>::from_usize_range(0..5)),
            mark(TextStyle::List, Span::<Char>::from_usize_range(5..12)),
        ];
        assert_eq!(margin("said\none\ntwo", blocks), (3, 0), "a rule, two dots");
        let code = vec![mark(
            TextStyle::CodeBlock,
            Span::<Char>::from_usize_range(0..4),
        )];
        assert_eq!(margin("fn a", code), (0, 1), "a tint, no margin mark");
    }

    /// A quote and a list draw each of their lines past their margin mark,
    /// a wrapped one too. The line after the block starts at the body's edge
    /// again (`MURMURE-570`).
    #[test]
    fn a_sent_block_draws_its_lines_past_its_margin_mark() {
        let long = "word ".repeat(30);
        for style in [TextStyle::Quote, TextStyle::List] {
            let mut sent = message(
                Id::<User>::mint(),
                Instant::<Authored>::new(0),
                &format!("{long}\nplain"),
            );
            sent.styles = vec![StyleMark {
                style,
                range: Span::<Char>::from_usize_range(0..long.len() + 1),
            }];
            let mut list = MessageList::default();
            list.absorb(Id::<Segment>::mint(), vec![sent]);
            let (surface, _) = paint(&list, 600.0);
            let mut lines: Vec<(f64, f64)> = surface
                .ops()
                .iter()
                .filter_map(|op| match op {
                    DrawOp::Glyphs {
                        color: Some(c),
                        transform,
                        ..
                    } if *c == LIGHT.cell_text => {
                        let at = transform.translation();
                        Some((at.y, at.x))
                    }
                    _ => None,
                })
                .collect();
            lines.sort_by(|a, b| a.0.total_cmp(&b.0));
            // The author's name comes first, then the body's lines.
            let [_, block @ .., (_, after)] = lines.as_slice() else {
                panic!("the author and the body, got {lines:?}");
            };
            let edge = surface_px(TEXT_LEFT).get();
            let indent = edge + f64::from(look::BLOCK_INDENT.get());
            assert!(block.len() >= 2, "the block wraps, got {block:?}");
            assert!(
                block.iter().all(|(_, x)| (x - indent).abs() < 0.5),
                "each line of the block starts past its mark, got {block:?}"
            );
            assert!((after - edge).abs() < 0.5, "the next line at the edge");
        }
    }

    /// The unread line paints in the accent between the read message and
    /// the first unread one, and once (`MURMURE-184`).
    #[test]
    fn the_unread_line_paints_above_the_first_unread_message() {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let messages: Vec<_> = ["a", "abcd"]
            .iter()
            .map(|b| message(ada, Instant::<Authored>::new(0), b))
            .collect();
        let unread = messages.get(1).map(|m| m.id);
        list.absorb(Id::<Segment>::mint(), messages);
        let (plain, _) = paint(&list, 600.0);
        assert!(runs(&plain, inks().mention).is_empty());

        let (surface, painted) = paint_above(&list, unread, 600.0);
        assert_eq!(painted.laid_out.get(), 2);
        assert_eq!(painted.line_shown, OnScreen::Shown);
        // The separator reads `New` in the mention ink (`MURMURE-740`).
        let line = runs(&surface, inks().mention);
        let [(line_y, glyphs)] = line.as_slice() else {
            panic!("one run of the line, got {line:?}");
        };
        assert_eq!(
            *glyphs,
            ambre_commands::shown(&crate::ui::list::UNREAD_LINE)
                .chars()
                .count()
        );
        let bodies = runs(&surface, LIGHT.cell_text);
        let [_, (read_y, 1), (unread_y, 4)] = bodies.as_slice() else {
            panic!("the author, then both bodies, got {bodies:?}");
        };
        assert!(read_y < line_y && line_y < unread_y);
        let rules = surface
            .fill_colors()
            .iter()
            .filter(|c| **c == inks().mention)
            .count();
        assert_eq!(rules, 1);
    }

    /// A band paints as one muted row, with no header, and the message
    /// after it paints under a header of its own (`MURMURE-218`).
    #[test]
    fn a_band_paints_as_one_muted_row() {
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        list.absorb(
            Id::<Segment>::mint(),
            vec![message(ada, Instant::<Authored>::new(30), "abcd")],
        );
        let band = crate::ui::list::Band {
            cause: crate::message::Unreadable::Damaged,
            from: Instant::<Authored>::new(10),
            to: Some(Instant::<Authored>::new(30)),
        };
        list.unreadable(Id::<Segment>::mint(), band);
        let (surface, painted) = paint(&list, 600.0);
        assert_eq!(painted.laid_out.get(), 2);
        let muted = runs(&surface, LIGHT.header_text);
        let body = runs(&surface, LIGHT.cell_text);
        let [(band_y, _), ..] = muted.as_slice() else {
            panic!("the band, got {muted:?}");
        };
        let time = runs(&surface, inks().faint);
        let [(time_y, 1)] = time.as_slice() else {
            panic!("the time of the header, got {time:?}");
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
        let ada = Id::<User>::mint();
        let mut list = MessageList::default();
        let bodies = ["hello", "brave", "world"];
        let messages = bodies
            .iter()
            .map(|b| message(ada, Instant::<Authored>::new(0), b))
            .collect();
        list.absorb(Id::<Segment>::mint(), messages);
        let keys = list.keys();
        let (_, plain) = paint(&list, 600.0);
        let [one, two, ..] = plain.rows.as_slice() else {
            panic!("three rows: {plain:?}");
        };
        let (first, top, second) = (
            one.bounds.bottom.get(),
            two.bounds.top.get(),
            two.bounds.bottom.get(),
        );
        let drag = |press, drag| Shown {
            list: &list,
            unread: None,
            anchor: None,
            focus: None,
            selecting: Selecting {
                selection: None,
                press: Some(view_at(press)),
                drag: Some(view_at(drag)),
                click: None,
            },
            me: None,
            pictures: None,
            theme: Theme::Light,
            calendar: None,
        };
        let press = (SPACE_8.get(), first - SPACE_2.get() - 2.0);
        let (surface, painted) = paint_from(drag(press, (399.0, (top + second) / 2.0)), 600.0);
        let selection = painted.selection.expect("a selection");
        assert_eq!(
            selection.anchor,
            Spot {
                row: keys[Pos::<ListRow>::ZERO],
                byte: Pos::<Byte>::ZERO
            }
        );
        assert_eq!(
            selection.focus,
            Spot {
                row: keys[Pos::<ListRow>::new(1)],
                byte: Pos::<Byte>::new(5)
            }
        );
        let tint = selection_tint(LIGHT.selection);
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
                row: keys[Pos::<ListRow>::new(2)],
                byte: Pos::<Byte>::new(5)
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
                laid_out: Len::<ListRow>::ZERO,
                latest_shown: OnScreen::Hidden,
                oldest_shown: OnScreen::Hidden,
                line_shown: OnScreen::Hidden,
                rest: None,
                top: None,
                bottom: None,
                focus: None,
                rows: Vec::new(),
                selection: None,
                link: None,
                chip: None,
                file: None,
                join: None,
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

/// The look of a row, beside the tests of the painter.
#[cfg(test)]
#[path = "paint_look_tests.rs"]
mod look_tests;
