//! The video region of a call in the channel region (the Murmure calls spec, section 3, the
//! video region).
//!
//! While the page is in a call, the channel region gives a band to the call, above its composer.
//! Each other participant takes a tile of a grid of three by three at most. A tile shows the
//! picture of its member, or its initials while no video of it arrives, with its name below
//! (`MURMURE-651`). Past nine, the grid shows pages of nine, which an arrow at each side turns
//! (`MURMURE-654`). A pinned participant takes a larger tile beside the grid, on every page
//! (`MURMURE-668`). The page asks the SFU for the video of each tile it shows (`MURMURE-667`).
//!
//! The tile of a participant who speaks is outlined (`MURMURE-653`). The member's own tile sits
//! apart, small, and marks its mute (`MURMURE-652`).
//!
//! Out of a call, the band takes no height (`MURMURE-621`). A received screen takes a taller
//! band, and a tile above the grid, taller than each of its tiles (`MURMURE-629`). This module
//! decides the band, the tiles and their paint. The render runtime draws the picture of each slot
//! inside its tile (`CALL-015`).

use ambre_canvas::TextCache;
use ambre_canvas::theme::{FONT_DISPLAY, FONT_UI};
use ambre_canvas::video::Facing;
use ambre_core::id::{Id, User};
use ambre_core::index::{Axis, Len, Numbered, Pos, Shown};
use ambre_core::{
    Bounded, Count, DisplayName, FracMs, Instant, MeterLevel, Ordinal, OrdinalKind, PageLoad, Px,
    Size, Surface, TimeSpan, Validated, VideoSlots, View,
};
use ambre_draw::{DrawSurface, FontContext, Point, Rect};

use crate::ui::call_controls::Microphone;
use crate::ui::paint::{draw, painter_px, shape, surface_px, text_height, text_width, view_rect};

/// The height the band takes in a tall region.
pub const VIDEO_BAND: Px<View> = Px::<View>::new(180.0);

/// The share of the height of the region the band takes at most, so a short window keeps a list.
const BAND_SHARE: f32 = 0.4;

/// The height the band takes in a tall region while it shows a screen: text needs it.
pub const SCREEN_BAND: Px<View> = Px::<View>::new(480.0);

/// The share of the height of the region a band with a screen takes at most.
const SCREEN_BAND_SHARE: f32 = 0.6;

/// The share of a band with a screen that the grid of the participants takes, below the screen.
const CAMERA_ROW_SHARE: f32 = 0.3;

/// The gap around each tile.
const TILE_GAP: Px<View> = Px::<View>::new(8.0);

/// The width of a tile over its height: the aspect of a camera.
const TILE_ASPECT: f32 = 16.0 / 9.0;

/// A count of tiles of the grid: `Count<Tiles>`.
pub enum Tiles {}

impl ambre_core::NarrowCount for Tiles {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Tiles");
}

/// The most tiles the grid holds: three by three, the videos the SFU forwards
/// (`ambre-spec-budgets.md section 1`, the call videos row).
const GRID_CELLS: Count<Tiles> = Count::<Tiles>::new(9);

/// The share of the grid's area a pinned tile takes at most, at its left (`MURMURE-668`).
const PINNED_SHARE: f32 = 0.5;

/// The largest share of the width of a pinned tile that a tile of the grid beside it takes. So
/// the pinned one stays the larger (`MURMURE-668`).
const BESIDE_PINNED: f32 = 0.5;

/// The width of the arrow at each side of a grid of more than one page (`MURMURE-654`).
const ARROW_WIDTH: Px<View> = Px::<View>::new(28.0);

/// The widest the member's own tile grows, so it stays small beside the grid.
const OWN_WIDTH: Px<View> = Px::<View>::new(160.0);

/// The share of the width of the band the member's own tile takes at most.
const OWN_SHARE: f32 = 0.25;

/// The height of the strip under a picture that names its member.
const NAME_STRIP: Px<View> = Px::<View>::new(18.0);

/// The share of a tile its name strip takes at most, so a small tile keeps a picture.
const NAME_SHARE: f32 = 0.3;

/// The width of the outline of a tile whose member speaks.
const SPEAKER_OUTLINE: Px<View> = Px::<View>::new(3.0);

/// The height of a picture past which the initials take the display size.
const LARGE_INITIALS: Px<View> = Px::<View>::new(60.0);

/// How long a member stays outlined after its last speech. A pause between two words keeps the
/// outline (`ambre-spec-budgets.md section 1`, call speaker).
const SPEECH_HOLD: TimeSpan<FracMs> = TimeSpan::<FracMs>::new(600.0);

/// Whether a participant speaks now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// No speech of the participant arrived lately.
    Quiet,
    /// The participant speaks: its tile is outlined (`MURMURE-653`).
    Speaks,
}

/// A page of the grid of a call, counted from zero: `Pos<GridPages>` (`MURMURE-654`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GridPages {}

impl Axis for GridPages {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("GridPage");
}

/// A page of the grid as the header names it: counted from one.
impl OrdinalKind for GridPages {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("GridPageNumber");
}

/// The number the header shows for a page: its index plus one.
impl Numbered for GridPages {
    type Number = Ordinal<GridPages>;

    fn number_of(pos: Pos<GridPages>) -> Ordinal<GridPages> {
        Ordinal::<GridPages>::of_index(pos)
    }
}

/// The header shows the count of the pages.
impl Shown for GridPages {}

/// Which way a press turns the page of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// To the page before.
    Previous,
    /// To the page after.
    Next,
}

impl Turn {
    /// The accessible name of the page button of the header that turns this way.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Previous => "Previous page of the call",
            Self::Next => "Next page of the call",
        }
    }

    /// The text of the arrow of the grid that turns this way.
    const fn arrow(self) -> &'static str {
        match self {
            Self::Previous => "<",
            Self::Next => ">",
        }
    }
}

/// What a press on the video region asks of the page. Neither writes into the channel
/// (`MURMURE-670`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPress {
    /// An arrow of the grid: show the page before or after (`MURMURE-654`).
    Turn(Turn),
    /// The tile of a participant: pin it, or unpin it when it is pinned (`MURMURE-668`).
    Pin(Id<User>),
}

/// Which page of the grid shows, and how many the grid holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paging {
    /// The page the grid shows, within its pages.
    pub page: Pos<GridPages>,
    /// How many pages the grid holds: one at least.
    pub pages: Len<GridPages>,
}

impl Paging {
    /// The page a turn shows: the turns stop at the first and at the last page.
    #[must_use]
    pub fn turned(self, turn: Turn) -> Pos<GridPages> {
        match turn {
            Turn::Previous => self.page.prev().unwrap_or(self.page),
            Turn::Next => self.page.next().within(self.pages),
        }
    }
}

/// The members one page of the grid shows: the pinned one apart, then the tiles of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    /// The pinned member, while it is in the call.
    pub pinned: Option<Id<User>>,
    /// The members of the tiles of the page, in the order the call admitted them.
    pub shown: Vec<Id<User>>,
    /// The page shown, and the count of pages.
    pub paging: Paging,
}

/// The members whose video the page asks of the SFU: exactly those whose tile the grid shows,
/// the pinned one first and large (`MURMURE-667`, `MURMURE-668`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wanted {
    /// The members, in the order of the grid.
    pub members: Vec<Id<User>>,
    /// The member the page shows large: the pinned one.
    pub large: Option<Id<User>>,
}

/// The page `page` of `others`, the other participants in the order the call admitted them, with
/// `pinned` apart. A page holds nine tiles, or eight beside a pinned one. So a choice of videos
/// names nine senders at most (`ambre-spec-budgets.md section 1`, the call videos row). A page
/// past the last shows the last. A pin of a member out of the call pins nobody.
#[must_use]
pub fn grid(others: &[Id<User>], pinned: Option<Id<User>>, page: Pos<GridPages>) -> Grid {
    let pinned = pinned.filter(|member| others.contains(member));
    let rest: Vec<Id<User>> = others
        .iter()
        .copied()
        .filter(|member| Some(*member) != pinned)
        .collect();
    let cells =
        usize::try_from(GRID_CELLS.get()).unwrap_or_default() - usize::from(pinned.is_some());
    let pages = Len::<GridPages>::of(rest.len().div_ceil(cells).max(1));
    let page = page.within(pages);
    let shown = rest
        .iter()
        .skip(page.to_usize() * cells)
        .take(cells)
        .copied()
        .collect();
    Grid {
        pinned,
        shown,
        paging: Paging { page, pages },
    }
}

impl Grid {
    /// The members whose video the page asks for: the pinned one first and large, then each tile
    /// of the page (`MURMURE-667`).
    #[must_use]
    pub fn wanted(&self) -> Wanted {
        Wanted {
            members: self.pinned.iter().chain(&self.shown).copied().collect(),
            large: self.pinned,
        }
    }
}

/// One other participant of the call of the page, as the video region shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Participant {
    /// The member, whose name and initials the tile shows.
    pub member: Id<User>,
    /// The video slot whose picture the tile draws, while one carries the member.
    pub video: Option<Pos<VideoSlots>>,
    /// Whether the member speaks now.
    pub voice: Voice,
}

/// The member's own tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Own {
    /// The member, whose initials the tile shows while its camera is off.
    pub member: Option<Id<User>>,
    /// The slot of the member's own camera, while it goes into the call: the tile draws it
    /// mirrored (`MURMURE-656`).
    pub video: Option<Pos<VideoSlots>>,
    /// Whether the microphone of the member sends: a muted one marks the tile.
    pub microphone: Microphone,
}

/// How much of the channel region the call takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fill {
    /// A band above the composer, under the message list.
    #[default]
    Band,
    /// The whole height above the composer: the list gives all of it up (`MURMURE-657`).
    Region,
}

impl Fill {
    /// The other way: the press of the fill button.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Band => Self::Region,
            Self::Region => Self::Band,
        }
    }

    /// The accessible name of the fill button, which says what a press does.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Band => "Fill the channel region with the call",
            Self::Region => "Shrink the call to a band",
        }
    }
}

/// What the video region shows of the call of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallStage {
    /// The other participants, in the order the call admitted them.
    pub others: Vec<Participant>,
    /// The page of the grid the member turned to (`MURMURE-654`).
    pub page: Pos<GridPages>,
    /// The participant the member pinned (`MURMURE-668`).
    pub pinned: Option<Id<User>>,
    /// The screen slot, while a received screen holds a picture.
    pub screen: Option<Pos<VideoSlots>>,
    /// The member's own tile.
    pub own: Own,
    /// How much of the channel region the call takes.
    pub fill: Fill,
}

impl CallStage {
    /// The page of the grid the region shows.
    #[must_use]
    pub fn grid(&self) -> Grid {
        let others: Vec<Id<User>> = self.others.iter().map(|p| p.member).collect();
        grid(&others, self.pinned, self.page)
    }

    /// The participant `member`, while it is in the call.
    fn participant(&self, member: Id<User>) -> Option<Participant> {
        self.others.iter().copied().find(|p| p.member == member)
    }
}

/// Whose tile a tile is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// The tile of another participant.
    Member(Id<User>),
    /// The member's own tile.
    Own,
}

/// One tile of the region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    /// Whose tile it is.
    pub who: Who,
    /// The whole tile: its picture and its name strip.
    pub frame: Rect,
    /// The box the picture or the initials take, above the name strip.
    pub picture: Rect,
    /// The video slot whose picture the tile draws.
    pub video: Option<Pos<VideoSlots>>,
    /// Whether its member speaks now.
    pub voice: Voice,
}

/// The arrows of a grid of more than one page: each one while it turns to another page
/// (`MURMURE-654`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pager {
    /// The arrow to the page before.
    pub previous: Option<Rect>,
    /// The arrow to the page after.
    pub next: Option<Rect>,
}

impl Pager {
    /// Each arrow, and the way it turns.
    fn arrows(self) -> impl Iterator<Item = (Turn, Rect)> {
        let previous = self.previous.map(|r| (Turn::Previous, r));
        let next = self.next.map(|r| (Turn::Next, r));
        previous.into_iter().chain(next)
    }

    /// The turn of the arrow at `at`, if any.
    #[must_use]
    pub fn at(self, at: Point) -> Option<Turn> {
        self.arrows()
            .find(|(_, frame)| frame.contains(at))
            .map(|(turn, _)| turn)
    }
}

/// Where each thing of the region lies.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Placed {
    /// The tile of the received screen.
    pub screen: Option<(Pos<VideoSlots>, Rect)>,
    /// The tiles of the participants: the pinned one first, the member's own last.
    pub tiles: Vec<Tile>,
    /// The arrows that turn the page, when the grid holds more than one.
    pub pager: Pager,
}

impl Placed {
    /// What a press at `at` asks of the page: an arrow turns the page, and the tile of another
    /// participant pins it (`MURMURE-654`, `MURMURE-668`).
    #[must_use]
    pub fn press_at(&self, at: Point) -> Option<CallPress> {
        if let Some(turn) = self.pager.at(at) {
            return Some(CallPress::Turn(turn));
        }
        self.tiles
            .iter()
            .find(|t| t.frame.contains(at))
            .and_then(|t| match t.who {
                Who::Member(member) => Some(CallPress::Pin(member)),
                Who::Own => None,
            })
    }
}

/// Where the band lies in its region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// The top of the band.
    pub top: Px<View>,
    /// The height of the band.
    pub height: Px<View>,
    /// The width of the region.
    pub width: Px<View>,
}

/// A box of the band, in the painter's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Area {
    left: Px<View>,
    top: Px<View>,
    width: Px<View>,
    height: Px<View>,
}

/// The height of the band in a region of `size` that shows `stage`: none out of a call
/// (`MURMURE-621`).
pub fn band_height(size: Size<Px<View>>, stage: Option<&CallStage>) -> Px<View> {
    let Some(stage) = stage else {
        return Px::<View>::ZERO;
    };
    if stage.screen.is_some() {
        return SCREEN_BAND.min(size.height * SCREEN_BAND_SHARE);
    }
    VIDEO_BAND.min(size.height * BAND_SHARE)
}

/// The height of the video region in a region of `size`, whose composer takes `composer`. A
/// filled call takes the whole height above the composer (`MURMURE-657`), and a band its own.
pub fn region_height(
    size: Size<Px<View>>,
    composer: Px<View>,
    stage: Option<&CallStage>,
) -> Px<View> {
    match stage.map(|s| s.fill) {
        Some(Fill::Region) => (size.height - composer).max(Px::<View>::ZERO),
        Some(Fill::Band) | None => band_height(size, stage),
    }
}

/// Where each thing of `stage` lies in the band `at`. A screen takes the top of the band, above
/// the grid (`MURMURE-629`). The member's own tile takes the right end of the grid's area. A
/// pinned participant takes a larger tile at its left (`MURMURE-668`). Past one page, an arrow
/// takes each side of the grid, which shows the page of the stage (`MURMURE-654`).
pub fn place(at: Band, stage: &CallStage) -> Placed {
    if at.height <= Px::<View>::ZERO {
        return Placed::default();
    }
    let band = Area {
        left: Px::<View>::ZERO,
        top: at.top,
        width: at.width,
        height: at.height,
    };
    let (screen, people) = match stage.screen {
        Some(slot) => {
            let row = at.height * CAMERA_ROW_SHARE;
            let screen_area = Area {
                height: at.height - row,
                ..band
            };
            let screen = cells(screen_area, Count::<Tiles>::new(1), None)
                .first()
                .copied()
                .map(|r| (slot, r));
            let people = Area {
                top: at.top + at.height - row,
                height: row,
                ..band
            };
            (screen, people)
        }
        None => (None, band),
    };
    let (own_frame, grid_area) = own_corner(people);
    let grid = stage.grid();
    let mut tiles = Vec::new();
    let (grid_area, widest) = match grid.pinned.and_then(|m| stage.participant(m)) {
        Some(pinned) => {
            let (frame, rest) = pinned_side(grid_area);
            let widest = frame.map(|f| painter_px(Px::<Surface>::new(f.width())) * BESIDE_PINNED);
            if let Some(frame) = frame {
                tiles.push(tile(
                    Who::Member(pinned.member),
                    frame,
                    pinned.video,
                    pinned.voice,
                ));
            }
            (rest, widest)
        }
        None => (grid_area, None),
    };
    let (pager, grid_area) = pager(grid_area, grid.paging);
    let shown: Vec<Participant> = grid
        .shown
        .iter()
        .filter_map(|m| stage.participant(*m))
        .collect();
    let frames = cells(grid_area, Count::<Tiles>::of(shown.len()), widest);
    tiles.extend(
        shown
            .iter()
            .zip(&frames)
            .map(|(p, frame)| tile(Who::Member(p.member), *frame, p.video, p.voice)),
    );
    if let Some(frame) = own_frame {
        tiles.push(tile(Who::Own, frame, stage.own.video, Voice::Quiet));
    }
    Placed {
        screen,
        tiles,
        pager,
    }
}

/// The tile of a pinned participant at the left of `area`, and the area the grid keeps beside
/// it (`MURMURE-668`).
fn pinned_side(area: Area) -> (Option<Rect>, Area) {
    let side = Area {
        width: area.width * PINNED_SHARE,
        ..area
    };
    let frame = cells(side, Count::<Tiles>::new(1), None).first().copied();
    let taken = frame.map_or(Px::<View>::ZERO, |f| {
        painter_px(Px::<Surface>::new(f.x1)) - area.left
    });
    let rest = Area {
        left: area.left + taken,
        width: (area.width - taken).max(Px::<View>::ZERO),
        ..area
    };
    (frame, rest)
}

/// The arrows of a grid of more than one page, each at one side of `area`, and the area the
/// grid keeps between them. An arrow shows while it turns to another page (`MURMURE-654`).
fn pager(area: Area, paging: Paging) -> (Pager, Area) {
    if paging.pages <= Len::<GridPages>::ONE || area.width <= ARROW_WIDTH * 2.0 {
        return (Pager::default(), area);
    }
    let gap = TILE_GAP;
    let (top, bottom) = (area.top + gap, area.top + area.height - gap);
    let left = area.left;
    let right = area.left + area.width;
    let previous = view_rect(left + gap, top, left + ARROW_WIDTH, bottom);
    let next = view_rect(right - ARROW_WIDTH, top, right - gap, bottom);
    let pager = Pager {
        previous: (paging.turned(Turn::Previous) != paging.page).then_some(previous),
        next: (paging.turned(Turn::Next) != paging.page).then_some(next),
    };
    let rest = Area {
        left: left + ARROW_WIDTH,
        width: area.width - ARROW_WIDTH * 2.0,
        ..area
    };
    (pager, rest)
}

/// The tile of `who` in `frame`: the picture above, the name strip below.
fn tile(who: Who, frame: Rect, video: Option<Pos<VideoSlots>>, voice: Voice) -> Tile {
    let tall = frame.height();
    let strip = surface_px(NAME_STRIP)
        .get()
        .min(tall * f64::from(NAME_SHARE));
    Tile {
        who,
        frame,
        picture: Rect::new(frame.x0, frame.y0, frame.x1, frame.y1 - strip),
        video,
        voice,
    }
}

/// The member's own tile, at the bottom right of `area`, and the area the grid keeps beside it.
fn own_corner(area: Area) -> (Option<Rect>, Area) {
    let gap = TILE_GAP;
    let width = OWN_WIDTH
        .min(area.width * OWN_SHARE)
        .min((area.height - gap * 2.0) * TILE_ASPECT);
    let height = width / TILE_ASPECT;
    if width <= Px::<View>::ZERO || height <= Px::<View>::ZERO {
        return (None, area);
    }
    let right = area.left + area.width - gap;
    let bottom = area.top + area.height - gap;
    let frame = view_rect(right - width, bottom - height, right, bottom);
    let grid = Area {
        width: (area.width - width - gap).max(Px::<View>::ZERO),
        ..area
    };
    (Some(frame), grid)
}

/// `count` cells of the aspect of a camera, in a grid of three by three at most inside `area`,
/// row by row. The grid takes the count of columns that gives the largest cells, and sits
/// centred; its last row too. A count past nine takes nine. A cell is `widest` wide at most, when
/// one is given.
fn cells(area: Area, count: Count<Tiles>, widest: Option<Px<View>>) -> Vec<Rect> {
    let Ok(count) = u16::try_from(count.get().min(GRID_CELLS.get())) else {
        return Vec::new();
    };
    if count == 0 {
        return Vec::new();
    }
    let gap = TILE_GAP.get();
    let fit = |columns: u16| {
        let rows = count.div_ceil(columns);
        let (c, r) = (f32::from(columns), f32::from(rows));
        let across = (area.width.get() - (c + 1.0) * gap) / c;
        let down = (area.height.get() - (r + 1.0) * gap) / r;
        let largest = across.min(down * TILE_ASPECT);
        let largest = widest.map_or(largest, |w| largest.min(w.get()));
        (largest, columns, rows)
    };
    // Three columns at most, and three rows at most: the nine tiles of `GRID_CELLS`.
    let Some((width, columns, rows)) = (1..=3)
        .filter(|columns| count.div_ceil(*columns) <= 3)
        .map(fit)
        .max_by(|a, b| a.0.total_cmp(&b.0))
    else {
        return Vec::new();
    };
    let height = width / TILE_ASPECT;
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let block = f32::from(rows) * height + f32::from(rows - 1) * gap;
    let top = area.top.get() + (area.height.get() - block) / 2.0;
    (0..count)
        .map(|i| {
            let (row, column) = (i / columns, i % columns);
            let in_row = (count - row * columns).min(columns);
            let line = f32::from(in_row) * width + f32::from(in_row - 1) * gap;
            let left = area.left.get() + (area.width.get() - line) / 2.0;
            let x = left + f32::from(column) * (width + gap);
            let y = top + f32::from(row) * (height + gap);
            view_rect(
                Px::<View>::new(x),
                Px::<View>::new(y),
                Px::<View>::new(x + width),
                Px::<View>::new(y + height),
            )
        })
        .collect()
}

/// One picture the render runtime draws: its slot, its box, and the way it faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Picture {
    /// The slot whose video it is.
    pub slot: Pos<VideoSlots>,
    /// The box it takes, its aspect kept inside.
    pub dest: Rect,
    /// Mirrored for the member's own camera, as sent for every other (`MURMURE-656`).
    pub facing: Facing,
}

/// The video of each slot the region shows, in its box, for the render runtime. The screen
/// comes first, then the picture of each tile that has one.
pub fn videos(placed: &Placed) -> Vec<Picture> {
    let screen = placed.screen.map(|(slot, dest)| Picture {
        slot,
        dest,
        facing: Facing::AsSent,
    });
    let tiles = placed.tiles.iter().filter_map(|t| {
        let facing = match t.who {
            Who::Own => Facing::Mirrored,
            Who::Member(_) => Facing::AsSent,
        };
        t.video.map(|slot| Picture {
            slot,
            dest: t.picture,
            facing,
        })
    });
    screen.into_iter().chain(tiles).collect()
}

/// The initials of `name`: the first letter of its first two words, in capitals. A name of no
/// letter gives none.
pub fn initials(name: Option<&Validated<DisplayName>>) -> String {
    name.map(Validated::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The words under a tile: the name of its member, or "You" on the member's own tile, which says
/// a mute (`MURMURE-652`). A member the page cannot name yet reads as a participant.
pub fn label(who: Who, name: Option<&Validated<DisplayName>>, microphone: Microphone) -> String {
    match (who, microphone) {
        (Who::Member(_), _) => name.map_or("A participant", Validated::as_str).to_owned(),
        (Who::Own, Microphone::Live) => "You".to_owned(),
        (Who::Own, Microphone::Muted) => "You, muted".to_owned(),
    }
}

/// Reads of `name_of` for each member of the region: the page's directory of the spaces.
pub type NameOf<'n> = &'n dyn Fn(Id<User>) -> Option<Validated<DisplayName>>;

/// The paint context of the region: the shaped text and the palette.
pub struct Pen<'p> {
    /// The fonts the text shapes with.
    pub font_cx: &'p mut FontContext,
    /// The shaped texts, kept across paints.
    pub cache: &'p mut TextCache,
    /// The colours of the theme.
    pub palette: &'p ambre_canvas::Palette,
}

/// One text the region draws, centred in its box.
#[derive(Debug, Clone, PartialEq)]
pub struct Words {
    /// What the text says.
    pub text: String,
    /// The box the text sits centred in.
    pub inside: Rect,
    /// The size of its font.
    pub size: Px<View>,
}

/// The texts of the region, in the order the paint draws them. Each tile with no picture gives
/// its initials, then each tile its name. The arrows that turn the page come last
/// (`MURMURE-651`, `MURMURE-652`, `MURMURE-654`).
pub fn words(stage: &CallStage, placed: &Placed, name_of: NameOf<'_>) -> Vec<Words> {
    let mut out = Vec::new();
    for tile in &placed.tiles {
        let member = match tile.who {
            Who::Member(member) => Some(member),
            Who::Own => stage.own.member,
        };
        let name = member.and_then(name_of);
        let name = name.as_ref();
        if tile.video.is_none() {
            let size = if painter_px(Px::<Surface>::new(tile.picture.height())) >= LARGE_INITIALS {
                FONT_DISPLAY
            } else {
                FONT_UI
            };
            out.push(Words {
                text: initials(name),
                inside: tile.picture,
                size,
            });
        }
        out.push(Words {
            text: label(tile.who, name, stage.own.microphone),
            inside: Rect::new(tile.frame.x0, tile.picture.y1, tile.frame.x1, tile.frame.y1),
            size: FONT_UI,
        });
    }
    for (turn, frame) in placed.pager.arrows() {
        out.push(Words {
            text: turn.arrow().to_owned(),
            inside: frame,
            size: FONT_DISPLAY,
        });
    }
    out.retain(|w| !w.text.is_empty());
    out
}

/// Paints the band `at` of `stage`: its backdrop, the backdrop of each tile, its words, and the
/// outline of a member who speaks. The render runtime draws each picture over its tile
/// (`CALL-015`). A video is screen state, which no primitive of the drawing surface takes
/// (`CANVAS-056`).
pub fn paint<S: DrawSurface>(
    surface: &mut S,
    pen: &mut Pen<'_>,
    stage: Option<&CallStage>,
    at: Band,
    name_of: NameOf<'_>,
) {
    let Some(stage) = stage else {
        return;
    };
    if at.height <= Px::<View>::ZERO {
        return;
    }
    let palette = pen.palette;
    surface.fill_rect(
        view_rect(Px::<View>::ZERO, at.top, at.width, at.top + at.height),
        palette.header_bg,
    );
    let placed = place(at, stage);
    // A tile shows its backdrop until its picture arrives.
    let backdrops = placed
        .screen
        .map(|(_, screen)| screen)
        .into_iter()
        .chain(placed.tiles.iter().map(|t| t.frame))
        .chain(placed.pager.arrows().map(|(_, frame)| frame));
    for backdrop in backdrops {
        surface.fill_rect(backdrop, palette.content_pending);
    }
    for words in words(stage, &placed, name_of) {
        surface.push_clip(words.inside);
        centred(surface, pen, &words);
        surface.pop_clip();
    }
    for tile in placed.tiles.iter().filter(|t| t.voice == Voice::Speaks) {
        surface.stroke_rect(tile.frame, palette.selection, surface_px(SPEAKER_OUTLINE));
    }
}

/// Draws `words` centred in their box, in the text colour of the theme.
fn centred<S: DrawSurface>(surface: &mut S, pen: &mut Pen<'_>, words: &Words) {
    let inside = words.inside;
    let width = painter_px(Px::<Surface>::new(inside.width())).max(Px::<View>::new(1.0));
    let layout = shape(
        pen.font_cx,
        pen.cache,
        &words.text,
        words.size,
        width,
        ambre_canvas::Weight::Bold,
    );
    let (wide, tall) = (text_width(layout), text_height(layout));
    let x =
        painter_px(Px::<Surface>::new(inside.x0)) + ((width - wide) / 2.0).max(Px::<View>::ZERO);
    let y = painter_px(Px::<Surface>::new(inside.y0))
        + (painter_px(Px::<Surface>::new(inside.height())) - tall) / 2.0;
    draw(
        surface,
        layout,
        &words.text,
        ambre_core::Point::new(x, y),
        pen.palette.cell_text,
    );
}

/// Who spoke last, by member. The page reads the level of each received audio slot. A member
/// whose level passed the level of speech stays outlined for the hold (`MURMURE-653`). Nothing
/// of it leaves the page (`ZK-025`).
#[derive(Debug, Default)]
pub struct Voices {
    /// The instant each member last spoke, once per member.
    heard: Vec<(Id<User>, Instant<PageLoad>)>,
}

impl Voices {
    /// Reads `level`, the level of the audio slot that carries `member`, at `now`.
    pub fn hear(&mut self, member: Id<User>, level: Bounded<MeterLevel>, now: Instant<PageLoad>) {
        if level < Bounded::<MeterLevel>::SPEECH {
            return;
        }
        self.heard.retain(|(m, _)| *m != member);
        self.heard.push((member, now));
    }

    /// Whether `member` speaks at `now`.
    #[must_use]
    pub fn of(&self, member: Id<User>, now: Instant<PageLoad>) -> Voice {
        let speaks = self
            .heard
            .iter()
            .any(|(m, at)| *m == member && (now - *at) < SPEECH_HOLD);
        if speaks { Voice::Speaks } else { Voice::Quiet }
    }

    /// Forgets every voice: the call ended.
    pub fn clear(&mut self) {
        self.heard.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::SCREEN_SLOT;
    use ambre_draw::testing::{DrawOp, RecordingSurface};

    fn px(v: f32) -> Px<View> {
        Px::<View>::new(v)
    }

    /// A band at `top`, `height` tall, in a region `width` wide.
    fn band(top: f32, height: f32, width: f32) -> Band {
        Band {
            top: px(top),
            height: px(height),
            width: px(width),
        }
    }

    /// `count` other participants, the first `videos` of them with a video, in slot order.
    fn stage(count: u32, videos: u32) -> CallStage {
        CallStage {
            others: (0..count)
                .map(|i| Participant {
                    member: Id::<User>::mint(),
                    video: (i < videos).then(|| Pos::<VideoSlots>::new(i)),
                    voice: Voice::Quiet,
                })
                .collect(),
            page: Pos::<GridPages>::new(0),
            pinned: None,
            screen: None,
            own: Own {
                member: None,
                video: None,
                microphone: Microphone::Live,
            },
            fill: Fill::Band,
        }
    }

    /// `MURMURE-657`: a filled call takes the whole height above the composer, its grid grows
    /// with it, and a band takes its own height again. Out of a call, neither takes any.
    #[test]
    fn a_filled_call_takes_the_whole_height_above_the_composer() {
        let size = Size::new(px(1200.0), px(900.0));
        let composer = px(60.0);
        let mut shown = stage(4, 0);
        let band = region_height(size, composer, Some(&shown));
        assert_eq!(band, VIDEO_BAND);
        shown.fill = shown.fill.toggled();
        assert_eq!(shown.fill, Fill::Region);
        let filled = region_height(size, composer, Some(&shown));
        assert_eq!(filled, px(840.0));
        let small = members(&place(
            Band {
                top: px(0.0),
                height: band,
                width: size.width,
            },
            &shown,
        ));
        let large = members(&place(
            Band {
                top: px(0.0),
                height: filled,
                width: size.width,
            },
            &shown,
        ));
        assert!(large[0].frame.height() > small[0].frame.height() * 2.0);
        assert_eq!(shown.fill.toggled(), Fill::Band);
        assert_eq!(region_height(size, composer, None), Px::<View>::ZERO);
        assert_ne!(Fill::Band.label(), Fill::Region.label());
    }

    fn members(placed: &Placed) -> Vec<Tile> {
        placed
            .tiles
            .iter()
            .copied()
            .filter(|t| t.who != Who::Own)
            .collect()
    }

    /// The member of each tile of another participant, in the order of the tiles.
    fn member_ids(placed: &Placed) -> Vec<Id<User>> {
        members(placed)
            .iter()
            .filter_map(|t| match t.who {
                Who::Member(member) => Some(member),
                Who::Own => None,
            })
            .collect()
    }

    fn own(placed: &Placed) -> Option<Tile> {
        placed.tiles.iter().copied().find(|t| t.who == Who::Own)
    }

    /// `MURMURE-621`: out of a call the band takes no height, and in one it never takes more
    /// than its share of a short region. A call with no camera on still takes the band.
    #[test]
    fn no_call_takes_no_band_and_a_short_region_keeps_its_list() {
        let tall = Size::new(px(800.0), px(900.0));
        assert_eq!(band_height(tall, None), Px::<View>::ZERO);
        assert_eq!(band_height(tall, Some(&stage(1, 0))), VIDEO_BAND);
        assert_eq!(band_height(tall, Some(&stage(0, 0))), VIDEO_BAND);
        let short = Size::new(px(800.0), px(300.0));
        assert_eq!(band_height(short, Some(&stage(2, 2))), px(120.0));
        assert_eq!(
            place(band(0.0, 0.0, 800.0), &stage(2, 0)),
            Placed::default()
        );
    }

    /// `MURMURE-651`: each participant takes a tile of a grid of three by three at most. The
    /// tiles keep the order of the admission, with a video or not. The tiles keep the aspect of a camera,
    /// overlap none, and stay inside the band and the width, the member's own tile included.
    #[test]
    fn each_participant_takes_a_tile_of_a_grid_of_three_by_three_at_most() {
        for at in [band(500.0, 180.0, 800.0), band(0.0, 820.0, 1200.0)] {
            for count in 0..=9 {
                let shown = stage(count, count / 2);
                let placed = place(at, &shown);
                let tiles = members(&placed);
                assert_eq!(tiles.len(), shown.others.len());
                assert_eq!(placed.pager, Pager::default(), "one page shows no arrow");
                let rows: Vec<i64> = {
                    let mut tops: Vec<i64> = tiles.iter().map(|t| t.frame.y0 as i64).collect();
                    tops.dedup();
                    tops
                };
                assert!(rows.len() <= 3, "{} rows", rows.len());
                for t in &tiles {
                    let in_row = tiles
                        .iter()
                        .filter(|o| (o.frame.y0 - t.frame.y0).abs() < 1e-6)
                        .count();
                    assert!(in_row <= 3, "{in_row} in a row");
                    let aspect = t.frame.width() / t.frame.height();
                    assert!((aspect - f64::from(TILE_ASPECT)).abs() < 1e-3, "{aspect}");
                }
                let all = &placed.tiles;
                for (i, a) in all.iter().enumerate() {
                    let f = a.frame;
                    assert!(f.x0 >= 0.0 && f.x1 <= f64::from(at.width.get()), "{f:?}");
                    let (top, bottom) = (at.top.get(), at.top.get() + at.height.get());
                    assert!(f.y0 >= f64::from(top) && f.y1 <= f64::from(bottom), "{f:?}");
                    for b in all.iter().skip(i + 1) {
                        assert!(
                            f.intersect(b.frame).area() < 1e-6,
                            "overlap {f:?} {:?}",
                            b.frame
                        );
                    }
                }
                // A video moves no tile: the order is that of the admission.
                let admitted: Vec<Id<User>> = shown.others.iter().map(|p| p.member).collect();
                assert_eq!(member_ids(&placed), admitted);
                assert!(own(&placed).is_some(), "the member's own tile");
            }
        }
        // Four participants in a tall region take two rows of two.
        let placed = place(band(0.0, 820.0, 1200.0), &stage(4, 0));
        let tops: Vec<f64> = members(&placed).iter().map(|t| t.frame.y0).collect();
        assert!(
            (tops[0] - tops[1]).abs() < 1e-6 && tops[2] > tops[0],
            "{tops:?}"
        );
    }

    /// `MURMURE-651`: a participant with no video shows its initials in its tile, and every tile
    /// names its member in a strip under the picture.
    #[test]
    fn a_tile_without_video_shows_the_initials_and_each_tile_its_name() {
        let named = |text: &str| Validated::<DisplayName>::parse(text).ok();
        assert_eq!(initials(named("Ada Lovelace").as_ref()), "AL");
        assert_eq!(initials(named("ada").as_ref()), "A");
        assert_eq!(
            initials(named("jean-luc   picard de la barre").as_ref()),
            "JP"
        );
        assert_eq!(initials(named("(Bob) [Smith]").as_ref()), "BS");
        assert_eq!(initials(None), "");
        assert_eq!(
            label(
                Who::Member(Id::mint()),
                named("Ada").as_ref(),
                Microphone::Muted
            ),
            "Ada"
        );
        assert_eq!(
            label(Who::Member(Id::mint()), None, Microphone::Live),
            "A participant"
        );
        let shown = stage(2, 1);
        let placed = place(band(0.0, 180.0, 800.0), &shown);
        for tile in &placed.tiles {
            assert!(
                tile.picture.y1 < tile.frame.y1,
                "a name strip under {tile:?}"
            );
            assert_eq!(tile.picture.y0, tile.frame.y0);
        }
        let names: HashMapNames = shown
            .others
            .iter()
            .enumerate()
            .map(|(i, p)| (p.member, ["Ada Lovelace", "Bob Kahn"][i]))
            .collect();
        let texts = painted_texts(&shown, &names);
        // Ada sends a video: her tile shows her name alone. Bob shows his initials and his name.
        assert!(texts.contains(&"Ada Lovelace".to_owned()), "{texts:?}");
        assert!(texts.contains(&"BK".to_owned()), "{texts:?}");
        assert!(texts.contains(&"Bob Kahn".to_owned()), "{texts:?}");
        assert!(!texts.contains(&"AL".to_owned()), "{texts:?}");
    }

    type HashMapNames = std::collections::HashMap<Id<User>, &'static str>;

    /// The text of each word the region of `stage` draws, in order.
    fn painted_texts(stage: &CallStage, names: &HashMapNames) -> Vec<String> {
        let name_of = |member: Id<User>| {
            names
                .get(&member)
                .and_then(|n| Validated::<DisplayName>::parse(*n).ok())
        };
        let placed = place(band(0.0, 180.0, 800.0), stage);
        words(stage, &placed, &name_of)
            .into_iter()
            .map(|w| w.text)
            .collect()
    }

    /// A pen over the fonts the app ships, so each text shapes into glyphs.
    fn paint_of(stage: &CallStage, at: Band) -> RecordingSurface {
        let mut surface = RecordingSurface::new();
        let mut font_cx = crate::ui::paint::tests::fonts();
        let mut cache = TextCache::new();
        let mut pen = Pen {
            font_cx: &mut font_cx,
            cache: &mut cache,
            palette: ambre_canvas::Theme::Light.palette(),
        };
        paint(&mut surface, &mut pen, Some(stage), at, &|_| None);
        surface
    }

    /// `MURMURE-652`: the member's own tile is small and apart from the grid, says "You", and
    /// marks a mute.
    #[test]
    fn the_own_tile_is_small_and_marks_a_mute() {
        let mut shown = stage(3, 3);
        let at = band(0.0, 180.0, 800.0);
        let placed = place(at, &shown);
        let mine = own(&placed).unwrap_or_else(|| panic!("an own tile"));
        for t in members(&placed) {
            assert!(mine.frame.height() < t.frame.height(), "{mine:?} {t:?}");
        }
        assert!(mine.frame.width() <= f64::from(OWN_WIDTH.get()) + 1e-6);
        let names = HashMapNames::new();
        assert!(painted_texts(&shown, &names).contains(&"You".to_owned()));
        shown.own.microphone = Microphone::Muted;
        let texts = painted_texts(&shown, &names);
        assert!(texts.contains(&"You, muted".to_owned()), "{texts:?}");
    }

    /// `MURMURE-656`: while the member's camera goes into the call, its own tile draws it
    /// mirrored, and no initials. Every received video faces as sent. With the camera off, the
    /// tile draws no picture and shows the initials.
    #[test]
    fn the_own_tile_draws_the_camera_mirrored_while_it_is_on() {
        let mut shown = stage(2, 2);
        shown.own.member = Some(Id::<User>::mint());
        let at = band(0.0, 180.0, 800.0);
        let off = videos(&place(at, &shown));
        assert!(off.iter().all(|p| p.facing == Facing::AsSent), "{off:?}");
        assert!(!off.iter().any(|p| p.slot == ambre_core::OWN_SLOT));
        shown.own.video = Some(ambre_core::OWN_SLOT);
        let placed = place(at, &shown);
        let mine = own(&placed).unwrap_or_else(|| panic!("an own tile"));
        let on = videos(&placed);
        let mirrored: Vec<&Picture> = on.iter().filter(|p| p.facing == Facing::Mirrored).collect();
        assert_eq!(
            mirrored,
            vec![&Picture {
                slot: ambre_core::OWN_SLOT,
                dest: mine.picture,
                facing: Facing::Mirrored,
            }]
        );
        let name = Validated::<DisplayName>::parse("Ada Lovelace").ok();
        let name_of = |_: Id<User>| name.clone();
        let texts: Vec<String> = words(&shown, &placed, &name_of)
            .into_iter()
            .filter(|w| w.inside == mine.picture)
            .map(|w| w.text)
            .collect();
        assert!(texts.is_empty(), "no initials over the camera: {texts:?}");
        shown.own.video = None;
        let placed = place(at, &shown);
        let initials: Vec<String> = words(&shown, &placed, &name_of)
            .into_iter()
            .map(|w| w.text)
            .collect();
        assert!(initials.contains(&"AL".to_owned()), "{initials:?}");
    }

    /// `MURMURE-654`: past nine participants, the grid shows pages of nine, in the order the call
    /// admitted them, with an arrow to each other page. The turns stop at the first and at the
    /// last page, and a page past the last shows the last.
    #[test]
    fn past_nine_participants_the_grid_shows_pages_of_nine() {
        let at = band(0.0, 820.0, 1200.0);
        let nine = place(at, &stage(9, 9));
        assert_eq!(members(&nine).len(), 9);
        assert_eq!(nine.pager, Pager::default(), "one page turns nowhere");
        let mut twenty = stage(20, 3);
        let order: Vec<Id<User>> = twenty.others.iter().map(|p| p.member).collect();
        let first = place(at, &twenty);
        assert_eq!(member_ids(&first), order[..9]);
        assert!(first.pager.previous.is_none(), "no page before the first");
        let next = first
            .pager
            .next
            .unwrap_or_else(|| panic!("an arrow to the next page"));
        assert_eq!(
            first.press_at(next.center()),
            Some(CallPress::Turn(Turn::Next))
        );
        let paging = twenty.grid().paging;
        assert_eq!(paging.pages, Len::<GridPages>::new(3));
        assert_eq!(paging.turned(Turn::Previous), Pos::<GridPages>::new(0));
        twenty.page = paging.turned(Turn::Next);
        let second = place(at, &twenty);
        assert_eq!(member_ids(&second), order[9..18]);
        let previous = second
            .pager
            .previous
            .unwrap_or_else(|| panic!("an arrow to the page before"));
        assert_eq!(
            second.press_at(previous.center()),
            Some(CallPress::Turn(Turn::Previous))
        );
        for t in &second.tiles {
            for (_, arrow) in second.pager.arrows() {
                assert!(t.frame.intersect(arrow).area() < 1e-6, "{t:?} {arrow:?}");
            }
        }
        twenty.page = Pos::<GridPages>::new(2);
        let last = place(at, &twenty);
        assert_eq!(member_ids(&last), order[18..]);
        assert!(last.pager.next.is_none(), "no page after the last");
        assert_eq!(
            twenty.grid().paging.turned(Turn::Next),
            Pos::<GridPages>::new(2)
        );
        twenty.page = Pos::<GridPages>::new(7);
        assert_eq!(
            twenty.grid().paging.page,
            Pos::<GridPages>::new(2),
            "a page past the last shows the last"
        );
        let texts = painted_texts(&twenty, &HashMapNames::new());
        assert!(texts.contains(&"<".to_owned()), "{texts:?}");
        assert!(!texts.contains(&">".to_owned()), "{texts:?}");
        assert_ne!(Turn::Previous.label(), Turn::Next.label());
    }

    /// `MURMURE-667`: the page asks for exactly the members whose tile the grid shows. It names
    /// them in the order of its tiles, the pinned one first and large, nine at most. A pin of a member out of
    /// the call pins nobody.
    #[test]
    fn the_choice_names_exactly_the_tiles_the_grid_shows() {
        let at = band(0.0, 180.0, 800.0);
        for count in [0, 1, 5, 9, 10, 17, 30] {
            let mut shown = stage(count, count / 3);
            let inside = shown.others.get(3).map(|p| p.member);
            for page in 0..4 {
                for pin in [None, inside, Some(Id::<User>::mint())] {
                    shown.page = Pos::<GridPages>::new(page);
                    shown.pinned = pin;
                    let wanted = shown.grid().wanted();
                    assert_eq!(wanted.members, member_ids(&place(at, &shown)));
                    assert!(wanted.members.len() <= 9, "{count}: {wanted:?}");
                    let pinned = pin.filter(|m| shown.others.iter().any(|p| p.member == *m));
                    assert_eq!(wanted.large, pinned);
                    if pinned.is_some() {
                        assert_eq!(wanted.members.first().copied(), pinned);
                    }
                }
            }
            // The pages together name each participant once.
            shown.pinned = None;
            let pages = shown.grid().paging.pages.get();
            let mut all = Vec::new();
            for page in 0..pages {
                shown.page = Pos::<GridPages>::new(page);
                all.extend(shown.grid().wanted().members);
            }
            let admitted: Vec<Id<User>> = shown.others.iter().map(|p| p.member).collect();
            assert_eq!(all, admitted);
        }
    }

    /// `MURMURE-668`: a pinned participant takes a tile at the left of the grid, on every page.
    /// That tile is larger than each tile of the grid, and the grid beside it holds eight. A press on its tile asks
    /// for the pin again, which unpins it, and a press on the member's own tile asks nothing.
    #[test]
    fn a_pinned_participant_stays_large_on_every_page() {
        for at in [band(0.0, 180.0, 800.0), band(0.0, 820.0, 1200.0)] {
            let mut shown = stage(20, 20);
            let ada = shown.others[12].member;
            shown.pinned = Some(ada);
            assert_eq!(shown.grid().paging.pages, Len::<GridPages>::new(3));
            for page in 0..3 {
                shown.page = Pos::<GridPages>::new(page);
                let placed = place(at, &shown);
                let tiles = members(&placed);
                let pinned = tiles[0];
                assert_eq!(pinned.who, Who::Member(ada));
                assert!(tiles.len() <= 9, "{} tiles", tiles.len());
                for t in &tiles[1..] {
                    assert!(t.frame.height() < pinned.frame.height(), "{t:?} {pinned:?}");
                    assert!(t.frame.x0 >= pinned.frame.x1, "{t:?} left of {pinned:?}");
                    assert_ne!(t.who, Who::Member(ada));
                }
                assert_eq!(
                    placed.press_at(pinned.frame.center()),
                    Some(CallPress::Pin(ada))
                );
                let mine = own(&placed).unwrap_or_else(|| panic!("an own tile"));
                assert_eq!(placed.press_at(mine.frame.center()), None);
            }
        }
        // A sole other participant, pinned, shows large, and the grid beside it holds nothing.
        let mut one = stage(1, 1);
        one.pinned = one.others.first().map(|p| p.member);
        let placed = place(band(0.0, 180.0, 800.0), &one);
        assert_eq!(members(&placed).len(), 1);
        assert_eq!(one.grid().paging.pages, Len::<GridPages>::ONE);
    }

    /// `MURMURE-653`: a member stays outlined while its level passed the level of speech within
    /// the hold, and the paint outlines its tile alone.
    #[test]
    fn a_member_who_speaks_is_outlined_for_the_hold() {
        let (ada, bob) = (Id::<User>::mint(), Id::<User>::mint());
        let level = |v: f32| Bounded::<MeterLevel>::new(v).unwrap_or_else(|| panic!("{v}"));
        let at = |ms: f64| Instant::<PageLoad>::new(ms);
        let mut voices = Voices::default();
        voices.hear(ada, level(0.2), at(1_000.0));
        voices.hear(bob, level(0.001), at(1_000.0));
        assert_eq!(voices.of(ada, at(1_100.0)), Voice::Speaks);
        assert_eq!(voices.of(bob, at(1_100.0)), Voice::Quiet);
        assert_eq!(voices.of(ada, at(1_700.0)), Voice::Quiet);
        voices.hear(ada, level(0.05), at(1_650.0));
        assert_eq!(voices.of(ada, at(1_700.0)), Voice::Speaks);
        voices.clear();
        assert_eq!(voices.of(ada, at(1_700.0)), Voice::Quiet);

        let mut shown = stage(2, 0);
        if let Some(first) = shown.others.first_mut() {
            first.voice = Voice::Speaks;
        }
        let at = band(0.0, 180.0, 800.0);
        let placed = place(at, &shown);
        let surface = paint_of(&shown, at);
        let outlined: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::StrokeRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        let speaker = placed
            .tiles
            .iter()
            .find(|t| t.voice == Voice::Speaks)
            .map(|t| t.frame);
        assert_eq!(outlined, speaker.into_iter().collect::<Vec<_>>());
    }

    /// `MURMURE-629`: a received screen takes a taller band, and a tile above the grid, taller
    /// than each tile of a participant. That holds for every count, in a tall and a short region.
    #[test]
    fn a_screen_takes_a_tile_taller_than_each_camera_above_their_row() {
        for region in [
            Size::new(px(800.0), px(900.0)),
            Size::new(px(800.0), px(300.0)),
        ] {
            for count in 0..=9 {
                let mut shown = stage(count, count);
                let plain = band_height(region, Some(&shown));
                shown.screen = Some(SCREEN_SLOT);
                let height = band_height(region, Some(&shown));
                assert!(height > plain, "{height:?}");
                let at = band(100.0, height.get(), region.width.get());
                let placed = place(at, &shown);
                let (slot, screen) = placed
                    .screen
                    .unwrap_or_else(|| panic!("a tile for the screen"));
                assert_eq!(slot, SCREEN_SLOT);
                assert!(screen.x0 >= 0.0 && screen.x1 <= 800.0, "{screen:?}");
                assert!(screen.y0 >= 100.0, "{screen:?}");
                for t in &placed.tiles {
                    assert!(screen.height() > t.frame.height(), "{screen:?} {t:?}");
                    assert!(screen.y1 <= t.frame.y0, "the screen sits above {t:?}");
                    assert!(t.frame.y1 <= 100.0 + f64::from(height.get()) + 1e-6);
                }
                assert_eq!(videos(&placed).first().map(|v| v.slot), Some(SCREEN_SLOT));
            }
        }
    }

    /// `MURMURE-620`: the paint draws the backdrop of the band, then one of each tile. The render
    /// runtime gets the slot of each picture in the box above its name. Out of a call, it
    /// paints nothing.
    #[test]
    fn the_band_places_each_slot_in_its_tile() {
        let shown = stage(2, 2);
        let at = band(400.0, 180.0, 800.0);
        let placed = place(at, &shown);
        let tiles = members(&placed);
        let picture = |slot: u32, dest: Rect| Picture {
            slot: Pos::new(slot),
            dest,
            facing: Facing::AsSent,
        };
        assert_eq!(
            videos(&placed),
            vec![picture(0, tiles[0].picture), picture(1, tiles[1].picture)]
        );
        let surface = paint_of(&shown, at);
        let fills: Vec<Rect> = surface
            .ops()
            .iter()
            .filter_map(|op| match op {
                DrawOp::FillRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        let mut expected = vec![Rect::new(0.0, 400.0, 800.0, 580.0)];
        expected.extend(placed.tiles.iter().map(|t| t.frame));
        assert_eq!(fills, expected);
        // Each word shapes into glyphs: two names, each tile with a video.
        let runs = surface
            .ops()
            .iter()
            .filter(|op| matches!(op, DrawOp::Glyphs { .. }))
            .count();
        assert!(
            runs >= words(&shown, &placed, &|_| None).len(),
            "{runs} runs"
        );
        let mut none = RecordingSurface::new();
        let mut font_cx = crate::ui::paint::tests::fonts();
        let mut cache = TextCache::new();
        let mut pen = Pen {
            font_cx: &mut font_cx,
            cache: &mut cache,
            palette: ambre_canvas::Theme::Light.palette(),
        };
        paint(&mut none, &mut pen, None, at, &|_| None);
        assert!(none.ops().is_empty(), "no call paints no band");
    }
}
