//! The handles of a selected picture, and the resize a drag of one sends
//! (`features/ambre-spec-prose-images.md section 4`).
//!
//! A press on a picture selects it, and the picture wears eight handles: one
//! on each corner and one in the middle of each side. A drag of a handle
//! previews the new box and writes nothing. The release sends one
//! `ResizeImage`, so the resize is one step of the history. A corner keeps
//! the ratio, and a side moves that side alone.
//!
//! The geometry is the column's: document pixels from the page's left edge
//! and from the top of the column, as the painter draws in.

use ambre_core::{Doc, Point, Px, Size, View};
use ambre_draw::{Consumed, DrawSurface, Rect};

use ambre_core::Flag as _;
use ambre_core::{ContentHash, Validated};

use crate::events::{BlockId, ProseCommand, block_id};
use crate::message::{AspectLock, DisplaySize, PictureShown};
use crate::ui::{ProseUiState, SendResult};
use crate::view::walk::PagePalette;

/// How far from a handle a press still grabs it, in screen pixels. The reach
/// is the same at every zoom.
const HANDLE_REACH: Px<View> = Px::<View>::new(8.0);

/// The side of a drawn handle square, in screen pixels.
const HANDLE_SIZE: Px<View> = Px::<View>::new(8.0);

/// The width of the selection outline, in screen pixels.
const OUTLINE: Px<View> = Px::<View>::new(1.0);

/// The smallest side a drag leaves a picture, in document pixels. A box any
/// smaller could no longer be grabbed again.
const MIN_SIDE: Px<Doc> = Px::<Doc>::new(8.0);

/// One of the eight handles a selected picture wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

/// Which edge of the box a handle moves along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moves {
    /// The left or the top edge: the opposite edge stays.
    Start,
    /// The right or the bottom edge: the opposite edge stays.
    End,
    /// Neither: the box keeps its extent on that axis.
    Neither,
}

impl PictureHandle {
    /// Every handle, the corners first: where a corner and a side meet, the
    /// corner takes the press.
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomRight,
        Self::BottomLeft,
        Self::Top,
        Self::Right,
        Self::Bottom,
        Self::Left,
    ];

    fn across(self) -> Moves {
        match self {
            Self::TopLeft | Self::Left | Self::BottomLeft => Moves::Start,
            Self::TopRight | Self::Right | Self::BottomRight => Moves::End,
            Self::Top | Self::Bottom => Moves::Neither,
        }
    }

    fn down(self) -> Moves {
        match self {
            Self::TopLeft | Self::Top | Self::TopRight => Moves::Start,
            Self::BottomLeft | Self::Bottom | Self::BottomRight => Moves::End,
            Self::Left | Self::Right => Moves::Neither,
        }
    }

    /// A corner keeps the picture's ratio, and a side moves that side alone.
    #[must_use]
    pub fn lock(self) -> AspectLock {
        match (self.across(), self.down()) {
            (Moves::Neither, _) | (_, Moves::Neither) => AspectLock::Free,
            _ => AspectLock::Locked,
        }
    }

    /// The centre of the handle on `frame`.
    #[must_use]
    pub fn point(self, frame: PictureBox) -> Point<Px<Doc>> {
        let along = |moves: Moves, start: Px<Doc>, extent: Px<Doc>| match moves {
            Moves::Start => start,
            Moves::End => start + extent,
            Moves::Neither => start + extent / 2.0,
        };
        Point::new(
            along(self.across(), frame.origin.x, frame.size.width),
            along(self.down(), frame.origin.y, frame.size.height),
        )
    }
}

/// A picture's box on the column: its top-left corner and its size, in
/// document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureBox {
    pub origin: Point<Px<Doc>>,
    pub size: DisplaySize,
}

impl PictureBox {
    fn right(self) -> Px<Doc> {
        self.origin.x + self.size.width
    }

    fn bottom(self) -> Px<Doc> {
        self.origin.y + self.size.height
    }

    fn rect(self) -> Rect {
        Rect::new(
            f64::from(self.origin.x.get()),
            f64::from(self.origin.y.get()),
            f64::from(self.right().get()),
            f64::from(self.bottom().get()),
        )
    }
}

/// The handle of `frame` a press at `at` grabs, within `reach` of it. The
/// whole of each side grabs it, and a corner wins over the sides that meet
/// there.
#[must_use]
pub fn handle_at(frame: PictureBox, at: Point<Px<Doc>>, reach: Px<Doc>) -> Option<PictureHandle> {
    let near = |a: Px<Doc>, b: Px<Doc>| (a - b).abs() <= reach;
    let within = |value: Px<Doc>, start: Px<Doc>, end: Px<Doc>| {
        value >= start - reach && value <= end + reach
    };
    let in_x = within(at.x, frame.origin.x, frame.right());
    let in_y = within(at.y, frame.origin.y, frame.bottom());
    PictureHandle::ALL.into_iter().find(|handle| {
        let centre = handle.point(frame);
        match (handle.across(), handle.down()) {
            (Moves::Neither, _) => in_x && near(at.y, centre.y),
            (_, Moves::Neither) => in_y && near(at.x, centre.x),
            _ => near(at.x, centre.x) && near(at.y, centre.y),
        }
    })
}

/// The box a drag of `handle` from `from` leaves, the pointer having moved
/// by `moved`. The edges the handle does not move stay where they are.
///
/// A corner keeps the ratio of `from`: the side that moved most drives, as
/// the engine's own rule does (`PROSE-FEAT-214`). No side goes under
/// [`MIN_SIDE`], so a drag past the opposite edge stops there. Each side
/// is a whole pixel, as the document stores it.
#[must_use]
pub fn resized(from: PictureBox, handle: PictureHandle, moved: Size<Px<Doc>>) -> PictureBox {
    let side = |moves: Moves, extent: Px<Doc>, delta: Px<Doc>| match moves {
        Moves::Start => extent - delta,
        Moves::End => extent + delta,
        Moves::Neither => extent,
    };
    let asked = Size::new(
        side(handle.across(), from.size.width, moved.width).max(MIN_SIDE),
        side(handle.down(), from.size.height, moved.height).max(MIN_SIDE),
    );
    let locked = crate::picture::locked_size(asked, from.size, handle.lock(), None);
    // The document stores whole pixels, so the box drawn is the box stored.
    let size = Size::new(
        locked.width.max(MIN_SIDE).round(),
        locked.height.max(MIN_SIDE).round(),
    );
    let start = |moves: Moves, origin: Px<Doc>, end: Px<Doc>, extent: Px<Doc>| match moves {
        Moves::Start => end - extent,
        Moves::End | Moves::Neither => origin,
    };
    PictureBox {
        origin: Point::new(
            start(handle.across(), from.origin.x, from.right(), size.width),
            start(handle.down(), from.origin.y, from.bottom(), size.height),
        ),
        size,
    }
}

/// A drag of a handle in flight. It writes nothing until the release
/// (`PROSE-UI-244`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureResize {
    pub(crate) block: BlockId,
    pub(crate) handle: PictureHandle,
    /// The box when the press landed.
    pub(crate) from: PictureBox,
    /// Where the press landed, and where the pointer is now, on the column.
    pub(crate) grab: Point<Px<Doc>>,
    pub(crate) to: Point<Px<Doc>>,
}

impl PictureResize {
    /// The box the release would ask for.
    #[must_use]
    pub fn preview(&self) -> PictureBox {
        let moved = Size::new(self.to.x - self.grab.x, self.to.y - self.grab.y);
        resized(self.from, self.handle, moved)
    }
}

impl ProseUiState {
    /// A canvas point on the column: document pixels from the page's left
    /// edge and from the top of the column.
    fn column_point(&self, at: Point<Px<View>>) -> Point<Px<Doc>> {
        let scale = self.viewport.scale();
        Point::new(
            Px::<Doc>::new((at.x - self.column.left(&self.viewport)).get() / scale),
            Px::<Doc>::new(at.y.get() / scale) + self.viewport.scroll_y,
        )
    }

    /// The pictures the built pages draw, each with its box on the column
    /// and its address.
    fn drawn_pictures(&self) -> Vec<(BlockId, PictureBox, Option<Validated<ContentHash>>)> {
        let Some(pages) = self.column.built_pages(&self.viewport) else {
            return Vec::new();
        };
        let layout = &self.shaped.layout;
        pages
            .into_iter()
            .flat_map(|page| {
                let top = self.column.page_top_y(page) + layout.body_top();
                let (pieces, _) = self.pieces(page);
                pieces.into_iter().filter_map(move |piece| {
                    let picture = piece.picture.as_ref()?;
                    let block = self.shaped.block(piece.index)?;
                    let frame = PictureBox {
                        origin: Point::new(
                            layout.margin_left + picture.x,
                            top + piece.fragment.top,
                        ),
                        size: Size::new(picture.width, picture.height),
                    };
                    Some((block_id(block.block_id), frame, picture.address.clone()))
                })
            })
            .collect()
    }

    /// The box of `block` on the column, while a built page draws it.
    pub(crate) fn picture_box(&self, block: BlockId) -> Option<PictureBox> {
        self.drawn_pictures()
            .into_iter()
            .find_map(|(id, frame, _)| (id == block).then_some(frame))
    }

    /// The picture under a canvas point, and whether it failed for good.
    pub(crate) fn picture_at(&self, at: Point<Px<View>>) -> Option<(BlockId, PictureFailed)> {
        let at = self.column_point(at);
        self.drawn_pictures()
            .into_iter()
            .find_map(|(block, frame, address)| {
                let inside = at.x >= frame.origin.x
                    && at.x < frame.right()
                    && at.y >= frame.origin.y
                    && at.y < frame.bottom();
                let failed = matches!(
                    address
                        .as_ref()
                        .and_then(|address| self.pictures.get(address)),
                    Some(PictureShown::Failed(_))
                );
                inside.then_some((block, PictureFailed::from_bool(failed)))
            })
    }

    /// A press on a handle of the selected picture starts its resize, and
    /// moves no caret.
    pub(crate) fn press_handle(&mut self, at: Point<Px<View>>) -> Consumed {
        let Some(block) = self.picture_selected else {
            return Consumed::No;
        };
        let Some(from) = self.picture_box(block) else {
            return Consumed::No;
        };
        let grab = self.column_point(at);
        let reach = Px::<Doc>::new(HANDLE_REACH.get() / self.viewport.scale());
        let Some(handle) = handle_at(from, grab, reach) else {
            return Consumed::No;
        };
        self.picture_resize = Some(PictureResize {
            block,
            handle,
            from,
            grab,
            to: grab,
        });
        Consumed::Yes
    }

    /// The pointer of a handle drag moved: the preview follows, and nothing
    /// is sent.
    pub(crate) fn drag_handle(&mut self, at: Point<Px<View>>) -> Consumed {
        let to = self.column_point(at);
        match self.picture_resize.as_mut() {
            Some(resize) => {
                resize.to = to;
                Consumed::Yes
            }
            None => Consumed::No,
        }
    }

    /// The release of a handle drag sends one resize, unless the box did
    /// not change (`PROSE-UI-242`, `PROSE-UI-243`).
    pub(crate) fn release_handle(
        &mut self,
        send: &mut dyn FnMut(ProseCommand) -> SendResult,
    ) -> Consumed {
        let Some(resize) = self.picture_resize.take() else {
            return Consumed::No;
        };
        let size = resize.preview().size;
        if size != resize.from.size {
            let _ = send(ProseCommand::ResizeImage {
                block_id: resize.block,
                size,
                lock_aspect: resize.handle.lock(),
            });
        }
        Consumed::Yes
    }

    /// Draw the outline and the handles of the selected picture, over the
    /// pages, in the column's space (`PROSE-UI-241`). A drag draws them on
    /// the box its release would ask for (`PROSE-UI-245`).
    pub(crate) fn paint_picture_handles<S: DrawSurface>(
        &self,
        surface: &mut S,
        palette: &PagePalette,
    ) {
        let frame = match self.picture_resize {
            Some(resize) => Some(resize.preview()),
            None => self
                .picture_selected
                .and_then(|block| self.picture_box(block)),
        };
        let Some(frame) = frame else {
            return;
        };
        let scale = f64::from(self.viewport.scale());
        let outline = ambre_core::Px::<ambre_draw::Surface>::new(f64::from(OUTLINE.get()) / scale);
        surface.stroke_rect(frame.rect(), palette.accent, outline);
        let half = f64::from(HANDLE_SIZE.get()) / scale / 2.0;
        for handle in PictureHandle::ALL {
            let centre = handle.point(frame);
            let (x, y) = (f64::from(centre.x.get()), f64::from(centre.y.get()));
            surface.fill_rect(
                Rect::new(x - half, y - half, x + half, y + half),
                palette.accent,
            );
        }
    }
}

ambre_core::define_flag! {
    /// Whether a picture under a press failed for good, so the press asks
    /// for it again (`PROSE-UI-145`).
    pub(crate) enum PictureFailed {
        Shown,
        Failed,
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    fn frame() -> PictureBox {
        PictureBox {
            origin: Point::new(Px::<Doc>::new(100.0), Px::<Doc>::new(200.0)),
            size: Size::new(Px::<Doc>::new(200.0), Px::<Doc>::new(100.0)),
        }
    }

    fn at(x: f32, y: f32) -> Point<Px<Doc>> {
        Point::new(Px::<Doc>::new(x), Px::<Doc>::new(y))
    }

    fn moved(x: f32, y: f32) -> Size<Px<Doc>> {
        Size::new(Px::<Doc>::new(x), Px::<Doc>::new(y))
    }

    /// `PROSE-UI-241`: a corner wins where it meets two sides, the whole of
    /// a side grabs its handle, and a point inside the box grabs none.
    #[test]
    fn a_press_grabs_the_corner_first_then_the_side_then_nothing() {
        let reach = Px::<Doc>::new(8.0);
        let frame = frame();
        assert_eq!(
            handle_at(frame, at(104.0, 197.0), reach),
            Some(PictureHandle::TopLeft)
        );
        assert_eq!(
            handle_at(frame, at(299.0, 301.0), reach),
            Some(PictureHandle::BottomRight)
        );
        assert_eq!(
            handle_at(frame, at(150.0, 203.0), reach),
            Some(PictureHandle::Top)
        );
        assert_eq!(
            handle_at(frame, at(303.0, 230.0), reach),
            Some(PictureHandle::Right)
        );
        assert_eq!(handle_at(frame, at(200.0, 250.0), reach), None);
        assert_eq!(handle_at(frame, at(330.0, 250.0), reach), None);
    }

    /// `PROSE-UI-242`: a corner keeps the ratio, whichever side moved most,
    /// and the opposite corner stays.
    #[test]
    fn a_corner_keeps_the_ratio_and_the_opposite_corner() {
        let wider = resized(frame(), PictureHandle::BottomRight, moved(100.0, 10.0));
        assert_eq!(wider.size, moved(300.0, 150.0));
        assert_eq!(wider.origin, frame().origin);
        let taller = resized(frame(), PictureHandle::TopLeft, moved(-10.0, -50.0));
        assert_eq!(taller.size, moved(300.0, 150.0));
        assert_eq!(
            (taller.right(), taller.bottom()),
            (frame().right(), frame().bottom())
        );
        assert_eq!(PictureHandle::TopRight.lock(), AspectLock::Locked);
    }

    /// `PROSE-UI-243`: a side moves that side alone, and a drag past the
    /// opposite edge stops at the smallest side.
    #[test]
    fn a_side_moves_alone_and_stops_at_the_smallest_side() {
        let right = resized(frame(), PictureHandle::Right, moved(50.0, 80.0));
        assert_eq!(right.size, moved(250.0, 100.0));
        assert_eq!(right.origin, frame().origin);
        let top = resized(frame(), PictureHandle::Top, moved(40.0, 30.0));
        assert_eq!(top.size, moved(200.0, 70.0));
        assert_eq!(top.bottom(), frame().bottom());
        let crossed = resized(frame(), PictureHandle::Left, moved(500.0, 0.0));
        assert_eq!(crossed.size, moved(8.0, 100.0));
        assert_eq!(PictureHandle::Bottom.lock(), AspectLock::Free);
    }
}
